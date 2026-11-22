use super::block_entity_client_tick::ClientBlockEntityAccess;
use super::block_entity_tick::ResidentBlockEntityAccess;
use super::*;
use mod_api::{BlockEntityClientScope, BlockEntityScope, BlockEntitySideScope};

impl Game<'_> {
    pub(in crate::game) fn authoritative_world_for_mods(
        resources: &crate::game::Resources,
        seed: i64,
    ) -> sunrise_client::world::World {
        let mut world = Self::world_for_mods(resources.facts().air, seed);
        world.authoritative_group_lifetime(resources.rest().behaviours.clone());
        world
    }

    /// Configure notice collection before this level can receive any block entities.
    pub(in crate::game) fn world_for_mods(
        air: BlockStateId,
        seed: i64,
    ) -> sunrise_client::world::World {
        let mut world = sunrise_client::world::World::new(air, seed);
        world
            .block_entities
            .set_on_load_enabled(mod_api::__private::has_part_hook(
                mod_api::HookId::BlockEntityOnLoad,
            ));
        world
            .block_entities
            .set_data_load_enabled(mod_api::__private::has_part_hook(
                mod_api::HookId::BlockEntityLoadAdditional,
            ));
        world
    }

    /// A replacement can inherit a ticker slot later in the current pass.
    /// Deliver its attachment notice before lending its resident row to that tick.
    /// Returns whether the resident ticker was dispatched.
    pub(in crate::game) fn tick_mod_block_entity(&mut self, pos: Pos) -> bool {
        if mod_api::__private::any_mod_disabled()
            && let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get(&pos)
            && mod_api::__private::content_owner_disabled(entity.id())
        {
            return false;
        }
        if self.world.block_entities.has_pending_on_load()
            || self.world.block_entities.has_pending_data_load()
        {
            let generation = self.world.block_entities.generation(&pos);
            self.fire_pending_block_entity_loads();
            if self.world.block_entities.generation(&pos) != generation {
                return false;
            }
        }
        self.tick_mod_block_entity_loaded(pos)
    }

    /// Drain attachment notices before either logical side runs block entity tickers.
    pub(in crate::game) fn fire_pending_block_entity_loads(&mut self) {
        self.fire_pending_block_entity_data_loads();
        if !self.world.block_entities.has_pending_on_load() {
            return;
        }
        let server = self.is_logical_server();
        let level = if server {
            self.level(self.dim.id())
        } else {
            self.native_client_level()
        };
        let Some(level) = level else { return };
        let mut remaining = self.world.block_entities.pending_on_load_len();
        while let Some((pos, generation)) = self
            .world
            .block_entities
            .next_on_load_in_batch(&mut remaining)
        {
            if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityOnLoad) {
                continue;
            }
            let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get(&pos) else {
                continue;
            };
            let Some(id) = mod_api::__private::block_entity_type_key(entity.id()) else {
                continue;
            };
            let subject = mod_api::__private::game::block_entity_ref(
                level,
                [pos.0, pos.1, pos.2],
                generation,
            );
            if server {
                let mut access = ResidentBlockEntityAccess::on_load(self, subject);
                Self::native_block_entity_on_load(mod_api::__private::BlockEntityOnLoadCall {
                    id,
                    subject: BlockEntitySideScope::Server(BlockEntityScope::new(&mut access)),
                });
            } else {
                let mut access = ClientBlockEntityAccess {
                    game: self,
                    subject,
                };
                Self::native_block_entity_on_load(mod_api::__private::BlockEntityOnLoadCall {
                    id,
                    subject: BlockEntitySideScope::Client(BlockEntityClientScope::new(&mut access)),
                });
            }
        }
    }

    /// Notify subscribers after this instance is attached and its saved data is applied.
    pub(in crate::game) fn native_block_entity_on_load(
        call: mod_api::__private::BlockEntityOnLoadCall<'_>,
    ) {
        Self::fire_block_entity_on_load(call);
    }
}
