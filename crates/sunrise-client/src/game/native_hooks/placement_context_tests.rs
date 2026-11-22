#![cfg(test)]

use crate::game::native_entity_refs::tests::{bind_local_player, with_game};
use crate::game::{Game, GuestUse, Hit};
use mod_api::{
    BlockPart, BlockState, Hooks, ItemPart, LevelSide, MustSurviveEvent, PlacementContextEvent,
    Registrar, StateForPlacementEvent,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use sunrise_client::{items::ItemStack, motion::PlayerMotion};
use sunrise_core::{
    geometry::{Direction, Vec3},
    nbt::Tag,
    position::BlockPos,
};
use sunrise_world::level::PlaceContext;

struct Move;
#[mod_api::sunrise_part]
impl ItemPart for Move {
    fn placement_context(
        &self,
        _: LevelSide<'_>,
        event: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        let mut context = event.context.clone();
        match event.stack.count() {
            2 => return None,
            3 => {
                let placer = event.placer.expect("the local predicting player");
                assert_eq!(placer.player().entity().id(), 7);
                assert_eq!(placer.player().uuid(), 42);
                assert_eq!(
                    placer.eye_position(),
                    [12.5, 64.0 + f64::from(1.62_f32), 12.5]
                );
                assert_eq!(
                    event
                        .stack
                        .components()
                        .unwrap()
                        .as_nbt()
                        .get(&"minecraft:custom_name".into()),
                    Some(&Tag::String("Marker".into()))
                );
            }
            _ => {}
        }
        context.pos.y -= i32::try_from(event.stack.count()).unwrap() + 1;
        context.face = Direction::Down;
        Some(Some(context))
    }
}
struct Refuse;
#[mod_api::sunrise_part]
impl ItemPart for Refuse {
    fn placement_context(
        &self,
        _: LevelSide<'_>,
        _: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        Some(None)
    }
}
enum Invalid {
    Nonfinite,
    Item,
    Bounds,
}
#[mod_api::sunrise_part]
impl ItemPart for Invalid {
    fn placement_context(
        &self,
        _: LevelSide<'_>,
        event: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        let mut context = event.context.clone();
        match self {
            Self::Nonfinite => context.hit[0] = f64::NAN,
            Self::Item => {
                context.item = Some(mod_api::__private::item_key("minecraft:glass").unwrap())
            }
            Self::Bounds => context.pos.y = 10_000,
        }
        Some(Some(context))
    }
}
enum Survival {
    Skip,
    RequireWhen(Arc<AtomicBool>),
}
#[mod_api::sunrise_part]
impl ItemPart for Survival {
    fn must_survive(&self, _: LevelSide<'_>, _: &mut MustSurviveEvent<'_>) -> Option<bool> {
        match self {
            Self::Skip => Some(false),
            Self::RequireWhen(required) => required.load(Ordering::Relaxed).then_some(true),
        }
    }
}
struct Parts(Arc<AtomicBool>);
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = r.item_key("minecraft:stone");
        r.item_part(stone, Move);
        let dandelion = r.item_key("minecraft:dandelion");
        r.item_part(dandelion, Move);
        let dirt = r.item_key("minecraft:dirt");
        r.item_part(dirt, Refuse);
        let sand = r.item_key("minecraft:sand");
        r.item_part(sand, Invalid::Nonfinite);
        let red_sand = r.item_key("minecraft:red_sand");
        r.item_part(red_sand, Invalid::Item);
        let gravel = r.item_key("minecraft:gravel");
        r.item_part(gravel, Invalid::Bounds);
        let poppy = r.item_key("minecraft:poppy");
        r.item_part(poppy, Survival::Skip);
        let torch = r.item_key("minecraft:torch");
        r.item_part(torch, Survival::Skip);
        let scaffolding = r.item_key("minecraft:scaffolding");
        r.item_part(scaffolding, Survival::RequireWhen(self.0.clone()));
    }
}

#[test]
#[ignore = "daily"]
fn item_parts_rewrite_or_refuse_context_and_control_survival_on_both_sides() {
    let require_scaffolding_survival = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "placement",
        Box::new(Parts(require_scaffolding_survival.clone())),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let air = registry.default_state("minecraft:air").unwrap();
            let scaffolding = registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:scaffolding".into(),
                    properties: [
                        ("bottom", "true"),
                        ("distance", "6"),
                        ("waterlogged", "false"),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                })
                .unwrap();
            for authoritative in [true, false] {
                game.with_level(|b, level, ticks| {
                    level.native_authoritative = authoritative;
                    level.client_side = !authoritative;
                    let context = |item: &str| PlaceContext {
                        clicked_pos: BlockPos::new(4, 64, 4),
                        face: Direction::Up,
                        hit: Vec3::new(4.5, 64.0, 4.5),
                        inside: false,
                        yaw: 0.0,
                        pitch: 0.0,
                        sneaking: false,
                        item: item.into(),
                        can_use_game_master_blocks: false,
                    };
                    let preview = |level: &mut crate::game::GameLevel<'_>,
                                   ticks: &mut _,
                                   item: &str| {
                        let stack = ItemStack::new(item, 1);
                        b.decide_placement(
                            level,
                            ticks,
                            &context(item),
                            |s| (s, s),
                            |level, context| level.rewrite_placement_context(&stack, context, None),
                        )
                        .map(|(_, preview)| preview)
                    };
                    let placed = preview(level, ticks, "minecraft:stone").unwrap();
                    assert_eq!(placed.pos, BlockPos::new(4, 62, 4));
                    let fallback = preview(level, ticks, "minecraft:glass").unwrap();
                    assert_eq!(fallback.pos, BlockPos::new(4, 64, 4));
                    assert!(preview(level, ticks, "minecraft:dirt").is_none());
                    for item in ["minecraft:sand", "minecraft:red_sand", "minecraft:gravel"] {
                        assert!(preview(level, ticks, item).is_none(), "{item}");
                    }
                    assert!(
                        b.placement_preview(level, ticks, &context("minecraft:poppy"))
                            .is_some(),
                        "survival can be skipped, even with no soil"
                    );
                    assert!(
                        b.placement_preview(level, ticks, &context("minecraft:torch"))
                            .is_none(),
                        "standing/wall candidates must survive even when final survival is skipped"
                    );
                    // Unlike the vanilla rewrite, a part cannot target a nonreplaceable cell.
                    b.set_block(level, ticks, BlockPos::new(4, 62, 4), fallback.state, 3);
                    assert!(preview(level, ticks, "minecraft:stone").is_none());
                    b.set_block(level, ticks, BlockPos::new(4, 62, 4), air, 3);
                });
            }

            // Click the end of a scaffold extending six blocks from support.
            // Its horizontal walk reaches distance seven, where only vanilla's
            // skipped survival check lets the item place.
            game.world.set(4, 64, 4, scaffolding).unwrap();
            game.with_level(|b, level, ticks| {
                let context = PlaceContext {
                    clicked_pos: BlockPos::new(4, 64, 4),
                    face: Direction::Up,
                    hit: Vec3::new(4.5, 65.0, 4.5),
                    inside: false,
                    yaw: 0.0,
                    pitch: 0.0,
                    sneaking: false,
                    item: "minecraft:scaffolding".into(),
                    can_use_game_master_blocks: false,
                };
                assert_eq!(
                    b.placement_preview(level, ticks, &context).unwrap().pos,
                    BlockPos::new(4, 64, 5),
                );
                require_scaffolding_survival.store(true, Ordering::Relaxed);
                assert!(b.placement_preview(level, ticks, &context).is_none());
            });

            // The connected client actually predicts through DryLevel.
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "PlacementPrediction").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player =
                Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            game.sync_tick_chunks();
            game.set_mode_id(0);
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(4, 64, 4, stone).unwrap();
            game.world.set(4, 65, 4, stone).unwrap();
            let hit = crate::game::Hit {
                pos: [4, 64, 4],
                face: [0, 1, 0],
                point: [4.5, 65.0, 4.5],
                inside: false,
            };
            let stack = |item| ItemStack::new(item, 1);
            assert!(!Game::can_hint_block_placement(&stack("minecraft:stone")));
            assert!(!Game::can_hint_block_placement(&stack("minecraft:poppy")));
            assert!(Game::can_hint_block_placement(&stack("minecraft:glass")));
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:glass"))
                    .is_none()
            );
            assert!(!game.remote_would_place(&hit, &stack("minecraft:stone")));
            let decline = ItemStack::new("minecraft:stone", 2);
            assert!(game.remote_can_place(&hit, &decline).is_none());
            let mut redirect = ItemStack::new("minecraft:stone", 3);
            redirect.data_mut().custom_name = Some(Tag::String("Marker".into()));
            let placed = game
                .remote_can_place(&hit, &redirect)
                .expect("redirect from the occupied original destination");
            let pos = placed.pos;
            assert_eq!(pos, BlockPos::new(4, 61, 4));
            assert!(placed.changes.contains(&(pos, stone)));
            assert_eq!(game.world.block(pos.x, pos.y, pos.z), air);

            game.player.as_mut().unwrap().position = Vec3::new(4.5, 63.0, 4.5);
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:stone"))
                    .is_none(),
                "the player obstructs the final destination"
            );
            game.player.as_mut().unwrap().position = Vec3::new(12.5, 64.0, 12.5);
            game.world.set(4, 65, 4, air).unwrap();
            assert_eq!(
                game.remote_can_place(&hit, &decline).unwrap().pos,
                BlockPos::new(4, 65, 4)
            );
            let soil = registry.default_state("minecraft:dirt").unwrap();
            game.world.set(4, 64, 4, soil).unwrap();
            assert!(game.remote_would_place(&hit, &stack("minecraft:dandelion")));
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:dandelion"))
                    .is_none(),
                "the final destination lacks support"
            );
            game.world.set(4, 64, 4, air).unwrap();
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:dirt"))
                    .is_none()
            );
            assert!(
                game.remote_can_place(&hit, &stack("minecraft:poppy"))
                    .is_some()
            );
        })
    });
}

struct Substitute(&'static str);
#[mod_api::sunrise_part]
impl BlockPart for Substitute {
    fn state_for_placement(
        &self,
        level: LevelSide<'_>,
        _: &mut StateForPlacementEvent<'_>,
    ) -> Option<Option<BlockState>> {
        let (LevelSide::Server(level) | LevelSide::Client(level)) = level;
        Some(level.block_state_id(self.0))
    }
}
struct PlacementSubstitution;
#[mod_api::__subscriptions]
impl Hooks for PlacementSubstitution {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let door = r.block_key("minecraft:oak_door");
        r.block_part(door, Substitute("minecraft:stone"));
        let furnace = r.block_key("minecraft:furnace");
        r.block_part(furnace, Substitute("minecraft:oak_door"));
    }

    fn entity_place(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityPlaceEvent,
    ) -> mod_api::Flow {
        if event.pos.z != 9 {
            return mod_api::Flow::Continue;
        }
        let mod_api::Placement::Multi(writes) = &event.placement else {
            panic!("the held door pre-clears above the chosen stone");
        };
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].pos, event.pos);
        assert_eq!(
            Some(writes[0].state),
            host.block_state_id("minecraft:stone")
        );
        assert_eq!(writes[1].pos, event.pos.offset(0, 1, 0));
        assert_eq!(Some(writes[1].state), host.block_state_id("minecraft:air"));
        mod_api::Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn substituted_placement_uses_the_held_item_preclear_and_selected_placed_by() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "placement_substitution",
        Box::new(PlacementSubstitution),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            game.player =
                Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
            bind_local_player(game, 20, 123);
            game.lan_acting_name = Some("Placer".into());
            game.lan_acting_permission = Some(0);
            game.set_mode_id(0);

            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            let roof = registry.default_state("minecraft:obsidian").unwrap();
            let door = registry.default_state("minecraft:oak_door").unwrap();
            let mut upper = registry.state(door).unwrap().clone();
            upper.properties.insert("half".into(), "upper".into());
            let upper = registry.id(&upper).unwrap();
            for (item, z, remaining, primary, above) in [
                ("minecraft:furnace", 5, 1, door, upper),
                ("minecraft:oak_door", 7, 1, stone, air),
                ("minecraft:oak_door", 9, 2, air, roof),
            ] {
                game.world.set(4, 63, z, stone).unwrap();
                game.world.set(4, 65, z, roof).unwrap();
                let hit = Hit {
                    pos: [4, 63, z],
                    face: [0, 1, 0],
                    point: [4.5, 64.0, f64::from(z) + 0.5],
                    inside: false,
                };
                *game.held_mut() = Some(ItemStack::new(item, 2));
                game.guest_use = Some(GuestUse::on_block(mod_api::Hand::Main));
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_eq!(game.held().unwrap().count, remaining);
                assert_eq!(game.world.block(4, 64, z), primary);
                assert_eq!(game.world.block(4, 65, z), above);
            }
        });
    });
}

#[derive(Debug, PartialEq)]
struct Notice {
    pos: [i32; 3],
    face: mod_api::BlockFace,
    click: [f64; 3],
}

#[derive(Default)]
struct VetoTrace {
    occupy_target: bool,
    decisions: Vec<BlockPos>,
    vetoes: Vec<(BlockPos, mod_api::BlockState)>,
    notices: Vec<Notice>,
}

#[derive(Clone)]
struct VetoProbe(Arc<Mutex<VetoTrace>>);

#[mod_api::sunrise_part]
impl ItemPart for VetoProbe {
    fn placement_context(
        &self,
        level: LevelSide<'_>,
        event: &mut PlacementContextEvent<'_>,
    ) -> Option<Option<mod_api::PlacementContext>> {
        let (LevelSide::Server(level) | LevelSide::Client(level)) = level;
        let mut context = event.context.clone();
        assert_eq!(event.stack.count(), 2);
        if context.pos.x == 4 {
            let placer = event.placer.expect("the acting server player");
            assert_eq!(placer.player().entity().id(), 20);
            assert_eq!(placer.player().uuid(), 123);
            assert_eq!(
                placer.eye_position(),
                [12.5, 64.0 + f64::from(1.62_f32), 12.5]
            );
        } else {
            assert!(event.placer.is_none(), "a dispenser has no player");
        }
        context.pos.z += 2;
        let BlockPos { x, y, z } = context.pos;
        if level.block_state([x, y, z]) != level.block_state_id("minecraft:air") {
            context.pos.z += 1;
        }
        context.face = Direction::Down;
        context.hit = [
            f64::from(x) + 0.125,
            f64::from(y) + 0.75,
            f64::from(context.pos.z) + 0.875,
        ];
        self.0.lock().unwrap().decisions.push(context.pos);
        Some(Some(context))
    }

    fn on_block_placed_by(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::OnBlockPlacedByEvent<'_>,
    ) {
        self.0.lock().unwrap().notices.push(Notice {
            pos: event.pos,
            face: event.face,
            click: event.click,
        });
    }
}

#[mod_api::__subscriptions]
impl Hooks for VetoProbe {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for item in ["minecraft:stone", "minecraft:shulker_box"] {
            let key = r.item_key(item);
            r.item_part(key, self.clone());
        }
    }
    fn entity_place(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityPlaceEvent,
    ) -> mod_api::Flow {
        let occupy_target = {
            let mut trace = self.0.lock().unwrap();
            trace.vetoes.push((event.pos, event.placed_against));
            trace.occupy_target
        };
        if occupy_target {
            let level = host.level("minecraft:overworld").unwrap();
            let marker = host.block_state_id("minecraft:obsidian").unwrap();
            let BlockPos { x, y, z } = event.pos;
            assert_eq!(
                level.set_block(host, [x, y, z], marker, mod_api::BlockUpdateFlags::NONE),
                Ok(true)
            );
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn player_and_dispenser_apply_only_the_context_seen_by_the_veto() {
    use crate::game::api_values::TryIntoApi;

    let trace = Arc::new(Mutex::new(VetoTrace::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "placement_veto",
        Box::new(VetoProbe(trace.clone())),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let state = |name| registry.default_state(name).unwrap();
            let stone = state("minecraft:stone");
            let glass = state("minecraft:glass");
            let air = state("minecraft:air");
            let obsidian = state("minecraft:obsidian");
            for x in [4, 10] {
                for z in [2, 4, 7, 9] {
                    game.world.set(x, 63, z, stone).unwrap();
                }
                for z in [4, 9] {
                    game.world.set(x, 65, z, glass).unwrap();
                }
            }
            game.player =
                Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
            bind_local_player(game, 20, 123);
            game.lan_acting_name = Some("Placer".into());
            game.lan_acting_permission = Some(0);
            game.set_mode_id(0);

            for (occupy_target, z) in [(false, 2), (true, 7)] {
                trace.lock().unwrap().occupy_target = occupy_target;
                game.world.set(4, 64, z, stone).unwrap();
                *game.held_mut() = Some(ItemStack::new("minecraft:stone", 2));
                game.guest_use = Some(GuestUse::on_block(mod_api::Hand::Main));
                let hit = Hit {
                    pos: [4, 63, z],
                    face: [0, 1, 0],
                    point: [4.5, 64.0, f64::from(z) + 0.5],
                    inside: false,
                };
                // Mark the swapped-in body as a server guest's active turn for the click.
                game.lan_acting = Some(1);
                assert!(game.use_hand(Some(&hit)).unwrap());
                game.lan_acting = None;
                let left = game
                    .dispense_item(
                        (9, 64, z),
                        Direction::East,
                        ItemStack::new("minecraft:shulker_box", 2),
                    )
                    .unwrap()
                    .unwrap();
                let count = if occupy_target { 2 } else { 1 };
                assert_eq!(game.held().unwrap().count, count);
                assert_eq!(left.count, count);
                assert_eq!(
                    game.world.block(4, 64, z + 2),
                    if occupy_target { obsidian } else { stone }
                );
                let dispenser_state = game.world.block(10, 64, z + 2);
                if occupy_target {
                    assert_eq!(dispenser_state, obsidian);
                } else {
                    assert_eq!(
                        registry.state(dispenser_state).unwrap().name.as_str(),
                        "minecraft:shulker_box"
                    );
                }
                assert_eq!(
                    game.world.block(4, 64, z),
                    stone,
                    "occupied original destination"
                );
                assert_eq!(game.world.block(10, 64, z), air, "dispenser's front cell");
                for x in [4, 10] {
                    assert_eq!(game.world.block(x, 64, z + 3), air, "second candidate");
                }
            }
            game.world.set(10, 64, 12, stone).unwrap();
            let left = game
                .dispense_item(
                    (9, 64, 12),
                    Direction::East,
                    ItemStack::new("minecraft:shulker_box", 2),
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                left.count, 2,
                "a blocked dispenser still fails before the hook"
            );
            let trace = trace.lock().unwrap();
            let targets = [
                BlockPos::new(4, 64, 4),
                BlockPos::new(10, 64, 4),
                BlockPos::new(4, 64, 9),
                BlockPos::new(10, 64, 9),
            ];
            let against = glass.try_into_api().unwrap();
            assert_eq!(trace.decisions, targets);
            assert_eq!(trace.vetoes, targets.map(|pos| (pos, against)));
            assert_eq!(
                trace.notices,
                [
                    Notice {
                        pos: [4, 64, 4],
                        face: mod_api::BlockFace::Down,
                        click: [4.125, 64.75, 4.875]
                    },
                    Notice {
                        pos: [10, 64, 4],
                        face: mod_api::BlockFace::Down,
                        click: [10.125, 64.75, 4.875]
                    },
                ]
            );
        });
    });
}
