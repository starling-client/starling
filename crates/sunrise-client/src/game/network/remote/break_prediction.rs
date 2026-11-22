//! Optimistic block writes and reconciliation on the multiplayer client.
use super::*;
use crate::game::native_connection_groups::{GroupUpdates, ReceivedGroup, check_group_updates};
use sunrise_core::position::BlockPos;

/// `BlockStatePredictionHandler.ServerVerifiedState`: the most recent sequence,
/// the server's latest state, and the player's position at the first prediction.
pub(super) struct Prediction {
    sequence: i32,
    state: BlockStateId,
    player_pos: [f64; 3],
    removed_entity: Option<Box<RemovedEntity>>,
    group: Option<ReceivedGroup>,
}

/// A predicted write displaced this entity. The server need not resend its payload
/// or its synced attachments when a denial puts the same block back.
struct RemovedEntity {
    state: BlockStateId,
    entity: BlockEntity,
    attachments: Option<ResidentAttachments>,
}

impl Game<'_> {
    /// MultiPlayerGameMode.destroyBlock: the predicted break.
    pub(super) fn predict_break(&mut self, pos: [i32; 3], state: BlockStateId, sequence: i32) {
        self.touch_vibrate(crate::game::touch::BREAK_BUZZ);
        self.tutorial_destroy(state, 1.0);
        // Block.playerWillDestroy's destroy event (particles and the break
        // sound), which BaseFireBlock skips: the server's extinguish sound
        // is all a player hears.
        if !self.res.facts().get(state).has_class("BaseFireBlock") {
            self.play_break_sound(pos, state);
            self.destroy_particles(pos, state);
        }
        // Run the local shape updates over the removal, including the fluid left behind.
        // Keep every resulting write under the break's sequence, so packet reconciliation
        // can correct them without running the local rules again.
        let [x, y, z] = pos;
        let pos = BlockPos::new(x, y, z);
        let (mut changes, order) = self.with_level(|b, base, ticks| {
            let mut level = DryLevel::new(base);
            level.reads = Some(ticks);
            level.notice_order = Some(Default::default());
            b.remove_block(
                &mut level,
                &mut sunrise_world::level::Ticks::default(),
                pos,
                false,
            );
            (
                level.changed,
                level.notice_order.unwrap_or_default().into_inner(),
            )
        });
        // Apply every final state before publishing anything. Emit one notice per net
        // change, in the simulation's after-shape order (the last notice for each position).
        let mut applied = Vec::with_capacity(changes.len());
        for at in order.into_iter().rev() {
            if let Some(state) = changes.remove(&at) {
                let pos = [at.x, at.y, at.z];
                if let Some(previous) = self.predict_block(pos, state, sequence) {
                    applied.push((pos, previous, state));
                }
            }
        }
        for (pos, previous, state) in applied.into_iter().rev() {
            self.finish_client_block_write(pos, Some(previous), state, true);
        }
        self.mining = None;
    }

    /// The blocks a use changed, kept under its sequence until acknowledgement.
    pub(in crate::game) fn predict_blocks(
        &mut self,
        changes: &[(BlockPos, BlockStateId)],
        seq: i32,
    ) {
        for &(p, state) in changes {
            let pos = [p.x, p.y, p.z];
            let previous = self.predict_block(pos, state, seq);
            self.finish_client_block_write(pos, previous, state, true);
        }
    }

    fn predict_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        seq: i32,
    ) -> Option<BlockStateId> {
        let [x, y, z] = pos;
        let before = self.world.block(x, y, z);
        let here = self.player_position();
        if let Some(r) = self.remote.as_mut() {
            r.predictions
                .entry(pos)
                .and_modify(|e| e.sequence = seq)
                .or_insert(Prediction {
                    sequence: seq,
                    state: before,
                    player_pos: here,
                    removed_entity: None,
                    group: None,
                });
        }
        self.write_client_block(pos, state, None)
    }

    /// Retire the synced copy from the live holder; retain it only with a removed
    /// prediction entity from the block the server still believes is here.
    pub(super) fn retire_client_block_entity(
        &mut self,
        pos: [i32; 3],
        previous: BlockStateId,
        removed: Option<BlockEntity>,
    ) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let mut attachments = r.attachments.remove(&Holder::BlockEntity(pos));
        if let (Some(p), Some(entity)) = (r.predictions.get_mut(&pos), removed)
            && p.removed_entity.is_none()
            && self.res.facts().get(p.state).name == self.res.facts().get(previous).name
        {
            if let Some(attachments) = &mut attachments {
                attachments.take_index_binding();
                attachments.retire();
            }
            p.removed_entity = Some(Box::new(RemovedEntity {
                state: previous,
                entity,
                attachments,
            }));
        }
    }

    /// ClientLevel.setServerVerifiedBlockState: a predicted block keeps its on-screen
    /// state until acknowledgement, but remembers the server's update for then.
    pub(in crate::game) fn server_block(
        &mut self,
        pos: [i32; 3],
        state: i32,
        group: Option<ReceivedGroup>,
    ) {
        // An unknown state (a server with mods we lack) reads as air, as in its chunks.
        let state = state.max(0) as u32;
        let state = BlockStateId(if sunrise_client::chunk::known_block_state(state) {
            state
        } else {
            0
        });
        if let Some(p) = self
            .remote
            .as_mut()
            .and_then(|r| r.predictions.get_mut(&pos))
        {
            p.state = state;
            if group.is_some() {
                p.group = group;
            }
            if let Some(removed) = &p.removed_entity
                && self.res.facts().get(removed.state).name != self.res.facts().get(state).name
            {
                p.removed_entity = None;
            }
            return;
        }
        let previous = self.write_client_block(pos, state, None);
        if let Some(group) = group {
            group.apply(&mut self.world, pos);
        }
        self.finish_client_block_write(pos, previous, state, false);
    }

    pub(in crate::game) fn check_active_group_updates(
        &self,
        groups: &GroupUpdates,
        limit: usize,
    ) -> sunrise_protocol::Result<()> {
        check_group_updates(
            &self.world,
            groups.iter().filter_map(|(&pos, &group)| {
                (!self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.predictions.contains_key(&pos)))
                .then_some((pos, group))
            }),
            limit,
        )
    }

    /// `handleLevelChunkWithLight` writes the column directly, unlike verified block
    /// updates. When a pending correction carries group data, that newer authoritative
    /// snapshot supersedes the correction (including absence). Predictions without a
    /// group correction retain vanilla's acknowledgement behaviour.
    pub(in crate::game) fn supersede_group_predictions(
        &mut self,
        chunk: &sunrise_client::chunk::Chunk,
    ) {
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        for (&[x, y, z], prediction) in &mut r.predictions {
            if prediction.group.is_some()
                && (x >> 4, z >> 4) == (chunk.x, chunk.z)
                && chunk.section_index(y).is_some()
            {
                prediction.state = chunk.block(x & 15, y, z & 15);
                prediction.group = Some(ReceivedGroup(chunk.connection_group(x & 15, y, z & 15)));
                prediction.removed_entity = None;
            }
        }
    }

    /// BlockStatePredictionHandler.endPredictionsUpTo: settle the server's known
    /// state and restore data before the correction notice. Confirmations are silent.
    pub(super) fn end_predictions(&mut self, sequence: i32) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let last_teleport = r.last_teleport_sequence;
        let settled: Vec<_> = r
            .predictions
            .extract_if(|_, p| p.sequence <= sequence)
            .collect();
        let updates = settled.iter().filter_map(|&([x, y, z], ref prediction)| {
            prediction
                .group
                .filter(|_| self.world.is_loaded_at(x, y, z))
                .map(|group| ([x, y, z], group))
        });
        if let Err(error) = check_group_updates(&self.world, updates, self.connection_group_limit())
        {
            return self.refuse_play_payload(error);
        }
        for (pos, prediction) in settled {
            let [x, y, z] = pos;
            let state = prediction.state;
            let changed = self.world.block(x, y, z) != state;
            let previous = changed
                .then(|| self.write_client_block(pos, state, prediction.removed_entity))
                .flatten();
            if let Some(group) = prediction.group {
                group.apply(&mut self.world, pos);
            }
            if !changed {
                continue;
            }
            self.finish_client_block_write(pos, previous, state, false);
            // ClientLevel.syncBlockState: the returning block may now hold the player.
            let meets = self.block_meets_player(pos, state);
            if let Some(at) = snap_target(prediction.player_pos, last_teleport, sequence, meets) {
                self.snap_player_to(at);
            }
        }
        Ok(())
    }

    fn write_client_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        restored: Option<Box<RemovedEntity>>,
    ) -> Option<BlockStateId> {
        let [x, y, z] = pos;
        let previous = self.world.block(x, y, z);
        if !self.world.loaded(x, z) || self.world.set(x, y, z, state).is_err() {
            return None;
        }
        if let Some(restored) = restored {
            let RemovedEntity {
                entity,
                attachments,
                ..
            } = *restored;
            self.world.block_entities.replace((x, y, z), entity);
            if let Some(remote) = self.remote.as_mut() {
                let holder = Holder::BlockEntity(pos);
                if let Some(attachments) = attachments {
                    // A fresh sync received from the server takes precedence over saved data.
                    remote.attachments.entry(holder).or_insert(attachments);
                }
                self.bind_remote_attachment(holder);
            }
        } else {
            self.sync_block_entity((x, y, z), Some(previous), state);
        }
        Some(previous)
    }

    fn finish_client_block_write(
        &mut self,
        pos: [i32; 3],
        previous: Option<BlockStateId>,
        state: BlockStateId,
        predicted: bool,
    ) {
        if let Some(previous) = previous {
            self.client_block_written(pos, previous, state, predicted);
        }
    }
}

impl Game<'_> {
    /// `MultiPlayerGameMode.stopDestroyBlock`: aborts the break and starts the attack strength
    /// again, if one is running (`if (!isDestroying) return;`), and says whether it did. Only the
    /// progress goes: `destroyBlockPos` and `destroyingItem` keep the target, so a dig resumed on
    /// the same block sends no START and the server's clock runs on (#4124).
    pub(super) fn stop_remote_mining(&mut self) -> bool {
        let kept = self.remote.as_mut().and_then(|r| {
            if !std::mem::take(&mut r.is_destroying) {
                return None;
            }
            let (pos, face, _) = r.destroying?;
            r.destroying = Some((pos, face, 0.0));
            r.destroying
        });
        let Some((pos, _, _)) = kept else {
            return false;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.tutorial_destroy(state, -1.0);
        self.send(S::PlayerAction(play::PlayerAction {
            action: 2,
            pos,
            direction: 0,
            sequence: 0,
        }));
        self.reset_attack_strength();
        true
    }

    pub(super) fn veto_mining_prediction(
        &self,
        pos: [i32; 3],
        face: [i32; 3],
        starting: bool,
        progress: f32,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientMiningPredict) {
            return false;
        }
        let mut canceled = false;
        self.native_client_mining_predict(
            mod_api::ClientMiningPrediction {
                pos,
                face,
                starting,
                progress: progress.clamp(0.0, 1.0),
            },
            &mut canceled,
        );
        canceled
    }

    /// `Player.blockActionRestricted` for the block at `pos`: a spectator
    /// never breaks, an adventure player only with a main-hand item whose
    /// `can_break` matches the block (`ItemStack.canBreakBlockInAdventureMode`).
    /// A predicate this client could not decode (exact components) only
    /// tells that the server gave one, and the server sends the block back
    /// if it does not match.
    pub(super) fn remote_break_restricted(&self, pos: [i32; 3]) -> bool {
        use sunrise_protocol::item::{WirePatch, component::CAN_BREAK};
        if self
            .held()
            .and_then(|s| s.data())
            .is_some_and(|d| d.other("minecraft:can_break").is_some())
        {
            return self.block_action_restricted(pos);
        }
        self.spectator()
            || (!self.may_build()
                && !self
                    .held()
                    .and_then(|s| s.wire_patch())
                    .is_some_and(|p| WirePatch::adds(&p, CAN_BREAK)))
    }

    pub(in crate::game) fn request_remote_mining_stop(&mut self) {
        if let Some(r) = self.remote.as_mut() {
            r.stop_mining = true;
        }
    }
}
