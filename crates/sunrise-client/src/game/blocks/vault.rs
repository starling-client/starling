//! Vaults in the game: `VaultBlockEntity.Server.tick` for every vault
//! (`vault`), the block's state as it changes, the rewards it ejects, the
//! keys players insert (`VaultBlock.useItemOn`) and the display item spinning
//! inside (`VaultRenderer`).
use super::{
    Game,
    item_draw::{BlockItemDraw, block_entity_pose},
};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{
    Result,
    block_entities::BlockEntityLease,
    block_entity_render::vertex_light,
    camera::rotate_degrees,
    containers::BlockEntity,
    item_render,
    items::ItemStack,
    mesher::Vertex,
    vault::{Vault, VaultEvent, VaultLevel, VaultPlayer, VaultState},
};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_core::random::{LegacyRandom, LevelRandom};

type Pos = (i32, i32, i32);

pub(super) struct TakenVault {
    pub(super) lease: BlockEntityLease,
    pub(super) vault: Box<Vault>,
    pub(super) current: VaultState,
    pub(super) ominous: bool,
}

/// What a key insertion supplies to the reward roll
/// (`VaultBlockEntity.resolveItemsToEject`): the player, the inserted stack and
/// the player's luck.
struct VaultEject {
    player: sunrise_world::loot::LootEntity,
    tool: sunrise_world::loot::LootStack,
    luck: f32,
}

/// What a vault reads: the game time and the players. The players are listed
/// at the first ask: a vault looks for them only when its state may change
/// (`stateUpdatingResumesAt`, once a second), not at every tick.
struct Players<'g, 'a> {
    time: i64,
    game: &'g Game<'a>,
    players: std::cell::OnceCell<Vec<VaultPlayer>>,
}
impl VaultLevel for Players<'_, '_> {
    fn game_time(&self) -> i64 {
        self.time
    }
    fn players(&self) -> Vec<VaultPlayer> {
        self.players
            .get_or_init(|| self.game.vault_players())
            .clone()
    }
}

impl Game<'_> {
    fn vault_players(&self) -> Vec<VaultPlayer> {
        let host = self.player.as_ref().map(|p| VaultPlayer {
            uuid: self.acting_uuid(),
            block: [p.position.x, p.position.y, p.position.z].map(|v| v.floor() as i32),
            spectator: self.spectator(),
        });
        let guests = self.lan_present().into_iter().map(|g| VaultPlayer {
            uuid: g.uuid,
            block: g.position.map(|v| v.floor() as i32),
            spectator: g.spectator,
        });
        host.into_iter().chain(guests).collect()
    }
    /// VaultBlock.useItemOn on a client level: something in the hand on an ACTIVE vault is
    /// SUCCESS_SERVER, so the click ends there (no UseItem, no other hand, no swing); only the
    /// server tries the key.
    pub(super) fn vault_takes_click(&self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        held.is_some()
            && self.pack.assets.registry.state(state).is_some_and(|block| {
                sunrise_client::block_entity_types::factory_block(&block.name) == "minecraft:vault"
                    && block.properties.get("vault_state").map(String::as_str) == Some("active")
            })
    }
    /// The vault at `key` with its block's state and ominous flag.
    pub(super) fn take_vault(&mut self, key: Pos) -> Option<TakenVault> {
        let state = self.world.block(key.0, key.1, key.2);
        let block = self.pack.assets.registry.state(state)?;
        if sunrise_client::block_entity_types::factory_block(&block.name) != "minecraft:vault" {
            return None;
        }
        let current = block
            .properties
            .get("vault_state")
            .and_then(|s| VaultState::from_name(s))
            .unwrap_or_default();
        let ominous = block.properties.get("ominous").is_some_and(|v| v == "true");
        let Some((lease, BlockEntity::Vault(vault))) = self.world.block_entities.take(&key) else {
            return None;
        };
        Some(TakenVault {
            lease,
            vault,
            current,
            ominous,
        })
    }
    /// Runs `f` on the vault at `key` with the level random and a loot roll,
    /// then applies its events.
    fn with_vault<T>(
        &mut self,
        key: Pos,
        eject: Option<&VaultEject>,
        f: impl FnOnce(
            &mut Vault,
            &Players<'_, '_>,
            &mut LegacyRandom,
            VaultState,
            bool,
            &mut sunrise_client::vault::Roll<LegacyRandom>,
            &mut Vec<VaultEvent>,
        ) -> T,
    ) -> Option<Result<T>> {
        let TakenVault {
            lease,
            mut vault,
            current,
            ominous,
        } = self.take_vault(key)?;
        let mut random = std::mem::replace(
            &mut self.world.random,
            LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let mut sequences = std::mem::take(&mut self.rand_sequences);
        let loot = self.server_loot.get();
        let players = Players {
            time: self.world.game_time,
            game: self,
            players: std::cell::OnceCell::new(),
        };
        let seed = self.level.seed;
        let mut events = Vec::new();
        let out = {
            // LootTable.getRandomItems, stacks split; the reward rolls with the
            // table's random sequence.
            let mut roll = |table: &str, random: &mut LegacyRandom, sequence: bool| {
                let Some(loot) = &loot else {
                    return Vec::new();
                };
                // `LootContextParamSets.VAULT`: the vault's centre is the ORIGIN of
                // every roll; the reward adds THIS_ENTITY, TOOL and luck.
                let reward = eject.filter(|_| sequence);
                let context = sunrise_world::loot::ContainerLootContext {
                    luck: reward.map_or(0.0, |e| e.luck),
                    entity: reward.map(|e| &e.player),
                    ..Default::default()
                };
                let caller = players.game.caller_loot(
                    Some([key.0, key.1, key.2].map(|v| f64::from(v) + 0.5)),
                    reward.map(|e| e.tool.clone()),
                );
                let drops = caller
                    .with(|| match loot.random_sequence(table).filter(|_| sequence) {
                        Some(s) => loot.roll_container_with(table, sequences.get(s, seed), context),
                        None => loot.roll_container_with(table, random, context),
                    })
                    .unwrap_or_default();
                drops
                    .iter()
                    .map(sunrise_client::structure_blocks::loot_item_with_components)
                    .collect()
            };
            f(
                &mut vault,
                &players,
                &mut random,
                current,
                ominous,
                &mut roll,
                &mut events,
            )
        };
        drop(players);
        self.world.random = random;
        self.rand_sequences = sequences;
        self.world
            .block_entities
            .put(lease, BlockEntity::Vault(vault));
        Some(self.vault_events(key, events).map(|()| out))
    }
    /// The server tick of the vault at `key`, then its client tick when there
    /// is a window.
    pub(super) fn tick_vault(&mut self, key: Pos) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let ticked = self.with_vault(key, None, |v, l, random, state, ominous, roll, events| {
            v.server_tick(l, random, pos, state, ominous, roll, events);
        });
        if let Some(result) = ticked {
            result?;
        }
        if self.options.dedicated.is_none() {
            self.client_tick_vault(key);
        }
        Ok(())
    }
    /// `VaultBlock.useItemOn` with `held` in the hand that acts: true when the
    /// vault took the click (an active vault, with anything in the hand).
    pub(super) fn use_vault(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> Result<bool> {
        let Some(stack) = held.cloned() else {
            return Ok(false);
        };
        let player = self.acting_uuid();
        let eject = VaultEject {
            player: self.killer_loot_entity(None),
            tool: super::creatures::to_loot(&stack),
            luck: self.acting_luck(),
        };
        let used = self.with_vault(
            (pos[0], pos[1], pos[2]),
            Some(&eject),
            |v, l, random, state, ominous, roll, events| {
                v.use_item_on(
                    l,
                    random,
                    pos,
                    state,
                    ominous,
                    player,
                    Some(&stack),
                    roll,
                    events,
                )
            },
        );
        let Some(taken) = used.transpose()?.flatten() else {
            return Ok(false);
        };
        if taken > 0 {
            // awardStat(ITEM_USED), then stackToInsert.consume(count, player).
            self.award_stat("used", &stack.item, 1);
            for _ in 0..taken {
                self.consume_held();
            }
        }
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, &stack);
        Ok(true)
    }
    fn vault_events(&mut self, key: Pos, events: Vec<VaultEvent>) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let center = pos.map(|v| f64::from(v) + 0.5);
        for event in events {
            match event {
                VaultEvent::State(state) => self.set_vault_state(key, state)?,
                VaultEvent::BlockUpdated => self.mark_block_entity_updated(key),
                VaultEvent::LevelEvent(id, data) => self.level_event_at(id, pos, data),
                VaultEvent::Sound(name, volume, pitch) => {
                    self.play_audio(&format!("minecraft:{name}"), Some(center), volume, pitch)
                }
                // Vec3.atBottomCenterOf(pos).relative(UP, 1.2), lowered by spawnItem.
                VaultEvent::Eject(stack, v) => {
                    self.native_vault_loot(pos, &stack.item);
                    self.spawn_item_thrown(
                        stack,
                        [center[0], center[1] + 0.7 - 0.125, center[2]],
                        v,
                    )
                }
            }
        }
        Ok(())
    }
    /// `setBlockAndUpdate` with the new `vault_state`.
    fn set_vault_state(&mut self, key: Pos, state: VaultState) -> Result<()> {
        let current = self.world.block(key.0, key.1, key.2);
        let Some(mut block) = self.pack.assets.registry.state(current).cloned() else {
            return Ok(());
        };
        block
            .properties
            .insert("vault_state".into(), state.name().into());
        let id = self.pack.assets.registry.resolve(&block)?;
        self.level_set([key.0, key.1, key.2], id)?;
        self.native_vault_change_state(self.dim.id(), [key.0, key.1, key.2], state.name());
        Ok(())
    }
    /// `VaultRenderer`: the display item of every vault near the camera that
    /// shows one, turning above its floor.
    pub(super) fn vault_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        self.vault_items(camera, alpha, |d| {
            self.submit_item_world(d.state, d.pose, d.light, out);
        });
    }

    /// The items [`Self::vault_geometry`] draws: each copy of a display item's
    /// cluster (`ItemEntityRenderer.renderMultipleFromCount`).
    pub(super) fn vault_items(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        mut draw: impl FnMut(BlockItemDraw),
    ) {
        let _items = sunrise_client::spans::raw_flight_guard("block_entity_items");
        let reach = Reach::Default;
        sunrise_client::spans::note_block_entity_item_visits(|| {
            self.world.block_entities.len() as u64
        });
        for (key, entity) in &self.world.block_entities {
            let BlockEntity::Vault(vault) = entity else {
                continue;
            };
            let Some(stack) = &vault.display_item else {
                continue;
            };
            if !reach.should_render([key.0, key.1, key.2], camera, 0) {
                continue;
            }
            let state = self.resolve_item(stack, DisplayContext::Ground, None, 0);
            if state.is_empty() {
                continue;
            }
            let mut pose = block_entity_pose(*key, camera);
            pose.translate(0.5, 0.4, 0.5);
            rotate_degrees(&mut pose, 1, vault.display_spin(alpha as f32));
            let light = vertex_light(self.world.light(key.0, key.1, key.2));
            let seed =
                i64::from(self.item_number(&stack.item).unwrap_or(0)) + i64::from(stack.damage);
            for copy in item_render::cluster_poses(
                &state,
                pose.to_glam(),
                item_render::rendered_amount(stack.count),
                seed,
            ) {
                draw(BlockItemDraw {
                    #[cfg(test)]
                    stack,
                    #[cfg(test)]
                    display: DisplayContext::Ground,
                    #[cfg(test)]
                    seed: 0,
                    state: &state,
                    pose: copy,
                    light,
                });
            }
        }
    }
}
