//! Placement-state reads, placement writes and the `Block.setPlacedBy` step.
use super::{Game, api_values::block_state::block_listens, native_server::PlayerRead};
use mod_api::{
    BlockKey, BlockUpdateFlags, HookId, LevelReadError, LevelRef, LevelWriteError, PlayerRef,
    PlayerWorldRefusal as Refusal,
};
use sunrise_client::{containers::BlockEntity, items::ItemStack};
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_world::{
    level::{PlacementContext, classes::Class},
    registry::BlockStateId,
};

/// A placement's write flags: neighbours and clients told, shapes updated unless known.
fn placement_flags(update_neighbour_shapes: bool) -> BlockUpdateFlags {
    let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
    if update_neighbour_shapes {
        flags
    } else {
        flags | BlockUpdateFlags::KNOWN_SHAPE
    }
}

impl Game<'_> {
    /// The one block write (`level_set_block`, its block entity included) with a placement's
    /// flags, then `Block.setPlacedBy` in the player's turn; nothing else of `BlockItem.place`.
    pub(super) fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        update_neighbour_shapes: bool,
    ) -> Result<(), Refusal> {
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        let dead = match self.player_handle_read(player).ok_or(Refusal::Stale)? {
            PlayerRead::Acting { .. } => self
                .player
                .as_ref()
                .is_none_or(|p| p.health.health() <= 0.0),
            PlayerRead::Guest(g) => g.dead,
        };
        if dead {
            return Err(Refusal::Dead);
        }
        if self.pack.assets.registry.state(state).is_none() {
            return Err(Refusal::UnknownState);
        }
        if self.native_level_world(level).is_none() {
            return Err(Refusal::StaleLevel);
        }
        if level.dimension() != self.dim.key() {
            return Err(Refusal::OtherLevel);
        }
        let [x, y, z] = pos;
        if !self.world.shape.contains_y(y)
            || !(-30_000_000..30_000_000).contains(&x)
            || !(-30_000_000..30_000_000).contains(&z)
        {
            return Err(Refusal::OutOfWorld { pos });
        }
        if !self.world.loaded(x, z) {
            return Err(Refusal::NotLoaded { pos });
        }
        if self.world.block(x, y, z) == state {
            return Err(Refusal::Unchanged);
        }
        let flags = placement_flags(update_neighbour_shapes);
        if !self
            .placement_allowed(pos, state, Some(flags))
            .map_err(|_| Refusal::InTransaction)?
        {
            return Err(Refusal::Fenced);
        }
        // The write is `level_set_block`'s, on the level as it stands (a guest's turn is not
        // the authoritative level); only `setPlacedBy` runs in the player's turn.
        match self.level_set_block(level, pos, state, flags) {
            Ok(true) => {}
            Ok(false) => return Err(Refusal::Unchanged),
            Err(_) => return Err(Refusal::Stale),
        }
        let ran = self.native_as_player_handle(player, |game| {
            game.run_mod_placement_callback(pos, state, None);
        });
        if ran { Ok(()) } else { Err(Refusal::Stale) }
    }
    pub(super) fn level_state_for_placement(
        &mut self,
        level: LevelRef,
        block: BlockKey,
        context: &mod_api::PlacementContext,
    ) -> Result<Option<BlockStateId>, LevelReadError> {
        let world = self.native_level_world_result(level)?;
        if level.dimension() != self.dim.key() {
            return Err(LevelReadError::Unavailable);
        }
        let BlockPos { x, y, z } = context.pos;
        if !context.hit.iter().all(|n| n.is_finite())
            || !context.yaw.is_finite()
            || !context.pitch.is_finite()
            || !(-30_000_000..30_000_000).contains(&x)
            || !(-30_000_000..30_000_000).contains(&z)
            || !world.is_loaded_at(x, y, z)
        {
            return Ok(None);
        }
        let Some(block) = super::api_values::behavior_block(block, self.res.behaviours()) else {
            return Ok(None);
        };
        let item = match context.item {
            Some(key) => {
                let Some(item) = mod_api::__private::item_key_id(key) else {
                    return Ok(None);
                };
                item
            }
            None => "",
        };
        Ok(self.preview_state_for_placement(
            block,
            &PlacementContext {
                pos: context.pos,
                hit_pos: context.hit_pos,
                face: context.face,
                hit: Vec3::new(context.hit[0], context.hit[1], context.hit[2]),
                inside: context.inside,
                yaw: context.yaw,
                pitch: context.pitch,
                sneaking: context.sneaking,
                item: item.to_owned(),
                replace_clicked: context.replace_clicked,
            },
        ))
    }

    /// A chosen state written as a placement writes it, then `Block.setPlacedBy`, with no
    /// player: the write and the extra halves are those of `player_place_block`. `Ok(false)`
    /// when the write was refused or changed nothing.
    pub(super) fn level_place_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        update_neighbour_shapes: bool,
    ) -> Result<bool, LevelWriteError> {
        let flags = placement_flags(update_neighbour_shapes);
        if !self.native_level_write_position(level, pos)?
            || self.world.block(pos[0], pos[1], pos[2]) == state
            || !self.placement_allowed(pos, state, Some(flags))?
        {
            return Ok(false);
        }
        if !self.level_set_block(level, pos, state, flags)? {
            return Ok(false);
        }
        self.run_mod_placement_callback(pos, state, None);
        Ok(true)
    }
    /// `Block.setPlacedBy` of a block already standing at `pos`, with no placer. Its stack
    /// is consulted, not placed: the caller has already loaded any block entity data.
    pub(super) fn level_set_placed_by(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: Option<&ItemStack>,
    ) -> Result<bool, LevelWriteError> {
        if !self.native_level_write_position(level, pos)? {
            return Ok(false);
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !self.placement_allowed(pos, state, None)? {
            return Ok(false);
        }
        self.run_mod_placement_callback(pos, state, stack);
        Ok(true)
    }

    fn placement_allowed(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        write_flags: Option<BlockUpdateFlags>,
    ) -> Result<bool, LevelWriteError> {
        if self.world.carrier_fences.has_locked_cells() {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            if self.world.carrier_fences.locks_cell(&pos)
                || self
                    .preview_set_placed_by(at, state, write_flags.map(|flags| flags.bits()))
                    .keys()
                    .any(|p| self.world.carrier_fences.locks_cell(&[p.x, p.y, p.z]))
            {
                return Ok(false);
            }
        }
        if !self.transactions.is_empty() && self.wither_spawn_pattern(pos, state).is_some() {
            // The pattern's flag-2 writes, destroy events and neighbour pass are not journalled.
            return Err(LevelWriteError::InTransaction);
        }
        Ok(true)
    }

    /// Chosen-state mod placements did not run the simulation's block-item placement.
    /// Apply its state-only callbacks before the same game-level step used by an item.
    fn run_mod_placement_callback(
        &mut self,
        pos: [i32; 3],
        expected: BlockStateId,
        stack: Option<&ItemStack>,
    ) {
        let [x, y, z] = pos;
        let placed = self.world.block(x, y, z);
        let behaviours = self.res.behaviours();
        if behaviours.block(placed) != behaviours.block(expected) {
            return;
        }
        self.with_level(|b, level, ticks| {
            b.set_placed_by_state(level, ticks, BlockPos::new(x, y, z), placed);
        });
        let _ = self.set_placed_by(pos, None, stack);
        let _ = self.process_level_events();
    }

    /// The game-level part of `Block.setPlacedBy`, once the state-only part has run.
    /// A supplied player is already the acting player. A supplied stack is read only.
    pub(super) fn set_placed_by(
        &mut self,
        pos: [i32; 3],
        placer: Option<PlayerRef>,
        stack: Option<&ItemStack>,
    ) -> crate::Result<()> {
        let [x, y, z] = pos;
        let key = (x, y, z);
        let placed = self.world.block(x, y, z);
        if self.res.behaviours().data(placed).is(Class::Jukebox) {
            if let Some(stack) = stack {
                self.jukebox_placed(pos, stack);
            }
        } else {
            match self.world.block_entities.get(&key) {
                Some(BlockEntity::CommandBlock(_)) => self.command_block_placed(key, stack),
                // StructureBlock.createdBy accepts any living placer; this route's placers are players.
                Some(BlockEntity::StructureBlock(_)) if placer.is_some() => {
                    self.structure_block_placed(key)
                }
                // The sign editor is a ServerPlayer side effect, not a block-state or NBT write.
                Some(BlockEntity::Sign(sign))
                    if placer.is_some()
                        && !sign.waxed
                        && sign.front.has_editable_text()
                        && !self.fake_blocks(mod_api::FakeEffect::Menu) =>
                {
                    self.open_text_edit(key, true);
                }
                _ => {}
            }
        }
        self.check_wither_spawn(pos)?;
        if self.is_logical_server()
            && block_listens(HookId::SetPlacedBy, placed)
            && let Some(level) = self.level(self.dim.id())
        {
            self.native_set_placed_by(level, pos, placed, placer.map(PlayerRef::entity), stack);
        }
        Ok(())
    }
}
