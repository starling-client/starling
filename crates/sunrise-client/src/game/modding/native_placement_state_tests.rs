//! The server placement-state query through the public handle on prepared resources.
use super::{
    Dim, Game,
    native_entity_refs::tests::{run_fresh_cases, with_game_at},
};
use mod_api::{
    Block, BlockPart, BlockState, BlocksEvent, FenceGateInWallEvent, Hooks, LevelReadError,
    LevelRef, LevelSide, ModEntry, Mods, PlacementContext, Registrar, RegistrationContext,
    StateForPlacementEvent, WorldEpoch,
};
use std::sync::Arc;
use sunrise_client::chunk::Chunk;
use sunrise_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
};

const BLOCK: &str = "placement_read:camouflage";
const POS: BlockPos = BlockPos::new(8, 70, 8);

struct PlacementPart;
#[mod_api::sunrise_part]
impl BlockPart for PlacementPart {
    fn state_for_placement(
        &self,
        level: LevelSide<'_>,
        event: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        let LevelSide::Server(level) = level else {
            panic!("the query should ask the server part");
        };
        let default = level.block_state_default(event.state.block()).unwrap();
        assert_eq!(event.state, default);
        if event.context.sneaking {
            return Some(None);
        }
        if event.context.inside {
            return Some(Some(mod_api::__private::block_state_from_ordinal(
                default.block(),
                u16::MAX,
            )));
        }
        // A real non-replaceable block is clicked, but the query must use the
        // independent destination and replacement flag supplied by the caller.
        assert_eq!(event.context.pos, POS);
        assert_eq!(event.context.hit_pos, POS.offset(2, 0, 0));
        assert!(event.context.replace_clicked);
        assert_eq!(event.context.face, Direction::East);
        assert_eq!(event.context.hit, [10.0, 70.75, 8.0]);
        assert_eq!(event.context.yaw, -90.0);
        assert_eq!(event.context.pitch, 35.0);
        assert_eq!(
            event.context.item,
            mod_api::__private::item_key("minecraft:oak_log")
        );
        assert_eq!(level.has_neighbor_signal([POS.x, POS.y, POS.z]), Some(true));
        Some(level.block_state_id("minecraft:oak_log[axis=z]"))
    }

    fn fence_gate_in_wall(&self, _: LevelSide<'_>, _: &mut FenceGateInWallEvent) -> Option<bool> {
        Some(true)
    }
}

struct PlacementReadMod;
#[mod_api::__subscriptions]
impl Hooks for PlacementReadMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let block = r.block_key(BLOCK);
        r.block_part(block, PlacementPart);
    }

    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        event
            .register(Block::new(BLOCK, "minecraft:stone"))
            .unwrap();
    }
}

fn with_game(name: &str, test: impl FnOnce(&mut Game<'_>)) {
    if run_fresh_cases(
        &format!("game::native_placement_state_tests::{name}"),
        "SUNRISE_PLACEMENT_STATE_CHILD",
        &["run"],
    ) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("placement_read", Box::new(PlacementReadMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        super::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = super::mods::content_root(&base, &Default::default()).unwrap();
        with_game_at(root.prepared().to_path_buf(), |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            test(game);
        });
    });
}

#[test]
#[ignore = "daily"]
fn vanilla_candidates_use_the_resolved_context_without_placing() {
    with_game(
        "vanilla_candidates_use_the_resolved_context_without_placing",
        |game| {
            use super::api_values::TryIntoEngine;
            let level = game.level(game.dim.id()).unwrap();
            let mut context = PlacementContext::new(POS, Direction::East, None);
            context.yaw = -90.0;
            context.hit[1] += 0.75;
            let registry = game.pack.assets.registry;
            // A read must not change the destination or its neighbours, or enqueue the dry coral's
            // death tick and spend the random draw its placement rule would make.
            let occupied = POS.offset(1, 0, 0);
            game.world
                .set(
                    occupied.x,
                    occupied.y,
                    occupied.z,
                    registry.default_state("minecraft:stone").unwrap(),
                )
                .unwrap();
            let blocks = |game: &Game<'_>| {
                std::iter::once(POS)
                    .chain(Direction::ALL.map(|face| {
                        let [x, y, z] = face.normal();
                        POS.offset(x, y, z)
                    }))
                    .map(|p| game.world.block(p.x, p.y, p.z))
                    .collect::<Vec<_>>()
            };
            let before = blocks(game);
            let rng = game.world.random.state();
            let ticks = game.block_ticks.blocks.count();
            for (block, key, expected) in [
                ("minecraft:oak_stairs", "facing", "east"),
                ("minecraft:oak_log", "axis", "x"),
                ("minecraft:oak_slab", "type", "top"),
                // `BlockItem.place` would refuse this unsupported door: this read does not.
                ("minecraft:oak_door", "half", "lower"),
            ] {
                let block_key = game.block_key(block).unwrap();
                let state = level
                    .state_for_placement(game, block_key, &context)
                    .unwrap()
                    .unwrap();
                let state = registry.state(state.try_into_engine(&()).unwrap()).unwrap();
                assert_eq!(state.name, block);
                assert_eq!(state.properties[key], expected, "{block}");
            }
            let coral = game.block_key("minecraft:tube_coral").unwrap();
            assert!(matches!(
                level.state_for_placement(game, coral, &context),
                Ok(Some(_))
            ));
            assert_eq!(blocks(game), before);
            assert_eq!(game.world.random.state(), rng);
            assert_eq!(game.block_ticks.blocks.count(), ticks);
        },
    );
}

#[test]
#[ignore = "daily"]
fn placement_read_leaves_queued_spill_untouched() {
    with_game("placement_read_leaves_queued_spill_untouched", |game| {
        let level = game.level(game.dim.id()).unwrap();
        let block = game.block_key("minecraft:oak_log").unwrap();
        let context = PlacementContext::new(POS, Direction::Up, None);
        let stack = sunrise_client::items::ItemStack::new("minecraft:stick", 3);
        let pos = [8.5, 70.5, 8.5];
        let motion = Some([0.0, 0.25, 0.0]);
        let delay = 10;
        game.spill.spawns.push(crate::game::spill::Spawn {
            random: None,
            stack: stack.clone(),
            pos,
            motion,
            delay,
        });
        let dropped = game.dropped.len();
        assert!(matches!(
            level.state_for_placement(game, block, &context),
            Ok(Some(_))
        ));
        assert_eq!(game.dropped.len(), dropped);
        let [queued] = game.spill.spawns.as_slice() else {
            panic!("the spill must still contain the original item");
        };
        assert_eq!(queued.stack, stack);
        assert_eq!(queued.pos, pos);
        assert_eq!(queued.motion, motion);
        assert_eq!(queued.delay, delay);
        assert!(queued.random.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn placement_refusals_are_typed() {
    with_game("placement_refusals_are_typed", |game| {
        let level = game.level(game.dim.id()).unwrap();
        let mut context = PlacementContext::new(POS, Direction::Up, None);
        // No surface supports a vine in this empty column.
        let vine = game.block_key("minecraft:vine").unwrap();
        assert_eq!(level.state_for_placement(game, vine, &context), Ok(None));
        let mod_block = game.block_key(BLOCK).unwrap();
        context.sneaking = true;
        assert_eq!(
            level.state_for_placement(game, mod_block, &context),
            Ok(None)
        );
        context.sneaking = false;
        context.inside = true;
        assert_eq!(
            level.state_for_placement(game, mod_block, &context),
            Ok(game
                .block_state_default(mod_block)
                .map(|state| super::api_values::TryIntoApi::try_into_api(&state).unwrap()))
        );

        for (hit_y, yaw, pitch) in [
            (f64::NAN, 0.0, 0.0),
            (70.0, f32::INFINITY, 0.0),
            (70.0, 0.0, f32::NEG_INFINITY),
        ] {
            let mut invalid = context.clone();
            invalid.hit[1] = hit_y;
            invalid.yaw = yaw;
            invalid.pitch = pitch;
            assert_eq!(
                level.state_for_placement(game, mod_block, &invalid),
                Ok(None)
            );
        }

        let stone = game.block_key("minecraft:stone").unwrap();
        let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
        let closed = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
        for invalid in [stale, closed] {
            assert_eq!(
                invalid.state_for_placement(game, stone, &context),
                Err(LevelReadError::Stale)
            );
        }
        let unknown = mod_api::__private::parts_installation::block_key(u32::MAX);
        assert_eq!(level.state_for_placement(game, unknown, &context), Ok(None));
        context.item = Some(mod_api::__private::parts_installation::item_key(u32::MAX));
        assert_eq!(level.state_for_placement(game, stone, &context), Ok(None));
        context.item = None;
        for pos in [POS.offset(0, 1_000, 0), POS.offset(16, 0, 0)] {
            context.pos = pos;
            assert_eq!(level.state_for_placement(game, stone, &context), Ok(None));
        }
        assert_eq!(game.world.chunks.len(), 1);
        context.pos = POS;
        assert!(matches!(
            level.state_for_placement(game, stone, &context),
            Ok(Some(_))
        ));
    });
}

#[test]
#[ignore = "daily"]
fn another_blocks_state_is_returned_only_from_the_level_in_play() {
    with_game(
        "another_blocks_state_is_returned_only_from_the_level_in_play",
        |game| {
            use super::api_values::TryIntoApi;
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let log = game
                .block_state_id("minecraft:oak_log[axis=z]")
                .unwrap()
                .try_into_api()
                .unwrap();
            let mod_block = game.block_key(BLOCK).unwrap();
            let mut context = PlacementContext::new(
                POS,
                Direction::East,
                mod_api::__private::item_key("minecraft:oak_log"),
            );
            context.hit_pos = POS.offset(2, 0, 0);
            context.replace_clicked = true;
            context.hit = [10.0, 70.75, 8.0];
            context.yaw = -90.0;
            context.pitch = 35.0;
            game.world
                .set(
                    10,
                    70,
                    8,
                    registry.default_state("minecraft:stone").unwrap(),
                )
                .unwrap();
            let comparator = game
                .block_state_id("minecraft:comparator[facing=west,powered=true]")
                .unwrap();
            game.world.set(7, 70, 8, comparator).unwrap();
            game.block_ticks
                .comparator_outputs
                .insert(POS.offset(-1, 0, 0), 8);
            assert_eq!(
                level.state_for_placement(game, mod_block, &context),
                Ok(Some(log))
            );
            let door = game.block_key("minecraft:oak_door").unwrap();
            let state = level
                .state_for_placement(game, door, &context)
                .unwrap()
                .unwrap();
            use super::api_values::TryIntoEngine;
            let state = registry.state(state.try_into_engine(&()).unwrap()).unwrap();
            assert_eq!(state.properties["powered"], "true");

            // The vanilla candidate can also depend on a neighbour's Part.
            let neighbour = game.block_state_default(mod_block).unwrap();
            game.world.set(8, 70, 9, neighbour).unwrap();
            let gate = game.block_key("minecraft:oak_fence_gate").unwrap();
            let state = level
                .state_for_placement(game, gate, &context)
                .unwrap()
                .unwrap();
            let state = registry.state(state.try_into_engine(&()).unwrap()).unwrap();
            assert_eq!(state.properties["in_wall"], "true");

            assert!(game.enter_command_level(Dim::Nether).is_some());
            assert_eq!(
                level.block(game, [10, 70, 8]),
                game.block_key("minecraft:stone")
            );
            assert_eq!(
                level.state_for_placement(game, mod_block, &context),
                Err(LevelReadError::Unavailable)
            );
            assert_eq!(game.dim, Dim::Nether, "the query never swaps the level in");

            // The column layout has padding above the Nether's last buildable Y = 255.
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let nether = game.level(Dim::Nether.id()).unwrap();
            let log = game.block_key("minecraft:oak_log").unwrap();
            let almost_top = PlacementContext::new(BlockPos::new(8, 254, 8), Direction::Up, None);
            let top = PlacementContext::new(BlockPos::new(8, 255, 8), Direction::Up, None);
            let above_top = PlacementContext::new(BlockPos::new(8, 256, 8), Direction::Up, None);
            assert!(matches!(
                nether.state_for_placement(game, door, &almost_top),
                Ok(Some(_))
            ));
            assert_eq!(nether.state_for_placement(game, door, &top), Ok(None));
            assert!(matches!(
                nether.state_for_placement(game, log, &top),
                Ok(Some(_))
            ));
            assert_eq!(nether.state_for_placement(game, log, &above_top), Ok(None));
        },
    );
}
