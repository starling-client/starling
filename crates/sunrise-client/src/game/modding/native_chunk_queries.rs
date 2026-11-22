//! Read-only chunk facts that never request residency.
use super::Game;
use mod_api::{BlockKey, ChunkRef};
use sunrise_core::random::{RandomSource, slime_chunk_random};

impl Game<'_> {
    pub(super) fn chunk_is_slime_chunk(&self, chunk: ChunkRef) -> Option<bool> {
        let seed = self.level_seed(chunk.level())?;
        let [x, z] = chunk.pos();
        // Reuse the same world-bound validation as handles for unloaded chunks.
        self.level_chunk_ref(chunk.level(), [x, z])?;
        Some(
            slime_chunk_random(x, z, seed, 987_234_911)
                .next_int_bound(10)
                .unwrap_or(1)
                == 0,
        )
    }

    /// The sections of a loaded chunk whose palette may hold a state of `block` (`maybeHas`),
    /// by section Y, bottom up.
    pub(super) fn chunk_sections_with_block(
        &self,
        chunk: ChunkRef,
        block: BlockKey,
    ) -> Option<Vec<i32>> {
        if !self.is_logical_server() {
            return None;
        }
        let [x, z] = chunk.pos();
        let column = self
            .native_level_world(chunk.level())?
            .chunks
            .get(&(x, z))?;
        let states = super::api_values::block_state::current().ok()?;
        let of_block = |id: u32| {
            states
                .to_api(sunrise_world::registry::BlockStateId(id))
                .is_ok_and(|state| state.block() == block)
        };
        let bottom = column.min_y() >> 4;
        Some(
            column
                .sections
                .iter()
                .zip(bottom..)
                .filter(|(section, _)| section.blocks.maybe_has(of_block))
                .map(|(_, y)| y)
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{Authority, ChunkRef, DimensionKey, LevelRef};

    /// #2906: the sections whose palette may hold a block, by section Y from the bottom, with a
    /// section whose only such block was replaced still named (`maybeHas` keeps the entry),
    /// none for a block no palette holds, and None where no chunk is loaded or on a client.
    #[test]
    #[ignore = "daily"]
    fn sections_with_a_block_follow_the_palettes() {
        with_game(|game| {
            let key = |name: &str| mod_api::__private::block_key(name).unwrap();
            let (ore, emerald) = (key("minecraft:diamond_ore"), key("minecraft:emerald_ore"));
            let id = |block| super::super::api_values::default_state(block).unwrap().0;
            let mut column = sunrise_client::chunk::Chunk::empty(3, 4);
            // The vanilla column starts at Y -64: section index 2 is section Y -2, 5 is 1.
            std::sync::Arc::make_mut(&mut column.sections[2]).set(0, id(ore));
            let replaced = std::sync::Arc::make_mut(&mut column.sections[5]);
            replaced.set(100, id(ore));
            replaced.set(100, 0);
            assert_eq!(replaced.get(100), 0);
            game.world
                .chunks
                .insert((3, 4), std::sync::Arc::new(column));
            let level = game.level("minecraft:overworld").unwrap();
            let chunk = level.chunk_ref(game, [3, 4]).unwrap();
            assert_eq!(chunk.sections_with_block(game, ore), Some(vec![-2, 1]));
            assert_eq!(chunk.sections_with_block(game, emerald), Some(vec![]));
            let unloaded = level.chunk_ref(game, [90_000, 90_000]).unwrap();
            assert_eq!(unloaded.sections_with_block(game, ore), None);
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            assert_eq!(chunk.sections_with_block(game, ore), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn slime_queries_work_without_loading_and_refuse_stale_handles() {
        with_game(|game| {
            game.level.seed = 0;
            let level = game.level("minecraft:overworld").unwrap();
            let before = game.world.chunks.len();
            let far = (100_000, -100_000);
            assert!(!game.world.chunks.contains_key(&far));
            // Independent Java LCG calculation for seed 0: the first nextInt(10)
            // draws are 7, 1 and 0 respectively. Int terms wrap before widening;
            // the 48-bit LCG uses multiplier 0x5deece66d and addend 11.
            for (pos, expected) in [
                ([0, 0], false),
                ([-17, 29], false),
                ([100_000, -100_000], true),
            ] {
                let chunk = level.chunk_ref(game, pos).unwrap();
                assert_eq!(chunk.is_slime_chunk(game), Some(expected));
                assert_eq!(chunk.is_slime_chunk(game), Some(expected));
            }
            assert_eq!(game.world.chunks.len(), before);
            assert!(!game.world.chunks.contains_key(&far));
            let invalid = ChunkRef::new(level, [i32::MAX, 0]);
            assert_eq!(invalid.is_slime_chunk(game), None);
            let unopened = LevelRef::new(level.epoch(), DimensionKey::NETHER);
            assert_eq!(ChunkRef::new(unopened, [0, 0]).is_slime_chunk(game), None);
            game.enter_level(crate::game::Dim::Nether).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            game.level.seed = 0;
            let parked = ChunkRef::new(level, [-17, 29]);
            assert_eq!(parked.is_slime_chunk(game), Some(false));
            let active = ChunkRef::new(nether, [100_000, -100_000]);
            assert!(!game.world.chunks.contains_key(&far));
            assert_eq!(active.is_slime_chunk(game), Some(true));
            assert!(!game.world.chunks.contains_key(&far));
            // These handles are still current: refusal must be due to client role.
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            assert_eq!(active.is_slime_chunk(game), None);
            assert_eq!(parked.is_slime_chunk(game), None);
            game.options.execution_role =
                crate::game::execution_role::ExecutionRole::IntegratedServer;
            game.native_lifecycle
                .begin_open(Authority::IntegratedServer);
            assert_eq!(ChunkRef::new(level, [0, 0]).is_slime_chunk(game), None);
            assert_eq!(active.is_slime_chunk(game), None);
        });
    }
}
