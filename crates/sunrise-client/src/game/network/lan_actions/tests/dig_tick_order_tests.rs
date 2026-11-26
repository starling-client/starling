//! ServerPlayer.tick precedes the connection phase, including across levels.
use super::*;
use crate::game::levels::{ServerTickPhase, server_tick_phases};
use crate::game::native_entity_refs::tests::with_game;
use crate::game::native_item_places::tests::{guest, open_to_lan};
use mod_api::{HostSide, ServerHost};
use std::sync::{Arc, Mutex};
use sunrise_client::chunk::Chunk;
use sunrise_world::registry::BlockRegistry;

/// A stub for two levels, each with one connection mining one block. Drive
/// the production dispatcher without a Game or the prepared resource tables.
#[test]
fn every_level_ticks_before_the_connection_phase() {
    let blocks = BlockRegistry::from_facts_json(
        &br#"{"defaults":{"minecraft:air":0,"minecraft:stone":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:stone"]]}"#[..],
    )
    .unwrap();
    let stone = blocks.default_state("minecraft:stone").unwrap();
    let air = blocks.default_state("minecraft:air").unwrap();
    let dims = [Dim::Overworld, Dim::Nether];
    let mut dig = [0; 2];
    let mut heard = Vec::new();
    for phase in server_tick_phases(&dims, &dims) {
        match phase {
            ServerTickPhase::Level(i, dim) => {
                assert_eq!(dim, dims[i]);
                if stone != air {
                    dig[i] += 1;
                    heard.push(("dig", dim, dig[i]));
                }
            }
            ServerTickPhase::IdleLevel(dim) => panic!("unexpected idle level {dim:?}"),
            ServerTickPhase::Connections => {
                for (dim, progress) in dims.into_iter().zip(dig) {
                    heard.push(("pre", dim, progress));
                    heard.push(("post", dim, progress));
                }
            }
        }
    }
    assert_eq!(
        heard,
        [
            ("dig", Dim::Overworld, 1),
            ("dig", Dim::Nether, 1),
            ("pre", Dim::Overworld, 1),
            ("post", Dim::Overworld, 1),
            ("pre", Dim::Nether, 1),
            ("post", Dim::Nether, 1),
        ],
    );
}

#[derive(Debug, PartialEq, Eq)]
enum Event {
    EntityPre(u32, Option<i32>),
    EntityPost(u32),
    Dig(u128, i32),
    LevelPost(&'static str),
    PlayerPre(u128, &'static str, Option<i32>, Option<i32>),
    PlayerPost(u128),
    Criterion(u128, String),
}

struct Observe {
    events: Arc<Mutex<Vec<Event>>>,
    cancel: u32,
    check_delayed_destroy: bool,
}

const DELAYED: [i32; 3] = [9, 63, 9];

impl Observe {
    fn assert_delayed_block_is_air(&self, host: &dyn ServerHost, uuid: u128) {
        if self.check_delayed_destroy && uuid == 123 {
            let level = host.level("minecraft:overworld").unwrap();
            let air = host.block_state_id("minecraft:air").unwrap();
            assert_eq!(
                level.block_state(host, DELAYED),
                Some(air),
                "delayed destroy finished before the player body"
            );
        }
    }
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for Observe {
    fn entity_tick_pre(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::EntityTickPreEvent<'_>,
    ) -> mod_api::Flow {
        let HostSide::Server(host) = host else {
            return mod_api::Flow::Continue;
        };
        let id = event.entity;
        let immunity = host
            .entity(id)
            .and_then(|entity| entity.invulnerable_time(host));
        self.events
            .lock()
            .unwrap()
            .push(Event::EntityPre(id, immunity));
        if id == self.cancel {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
    fn entity_tick_post(&self, _: HostSide<'_>, event: &mut mod_api::EntityTickPostEvent<'_>) {
        self.events
            .lock()
            .unwrap()
            .push(Event::EntityPost(event.entity));
    }
    fn block_break_progress_update(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::BlockBreakProgressUpdateEvent<'_>,
    ) {
        self.events
            .lock()
            .unwrap()
            .push(Event::Dig(event.player, event.progress));
    }
    fn player_advancement_progress(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::PlayerAdvancementProgressEvent<'_>,
    ) {
        if matches!(event.advancement, "test:phase" | "test:staged") {
            self.events
                .lock()
                .unwrap()
                .push(Event::Criterion(event.player, event.criterion.into()));
        }
    }
    fn level_tick_post(&self, host: HostSide<'_>, _: &mut mod_api::LevelTickPostEvent) {
        if let HostSide::Server(host) = host {
            self.events
                .lock()
                .unwrap()
                .push(Event::LevelPost(host.dimension()));
        }
    }
    fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
        let HostSide::Server(host) = host else { return };
        self.assert_delayed_block_is_air(host, event.player.uuid());
        let entity = event.player.entity();
        self.events.lock().unwrap().push(Event::PlayerPre(
            event.player.uuid(),
            host.dimension(),
            entity.invulnerable_time(host),
            entity.damage_cooldown_time(host),
        ));
    }
    fn player_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPostEvent) {
        let HostSide::Server(host) = host else { return };
        self.assert_delayed_block_is_air(host, event.player.uuid());
        self.events
            .lock()
            .unwrap()
            .push(Event::PlayerPost(event.player.uuid()));
    }
}

#[test]
#[ignore = "daily"]
fn advancement_criteria_straddle_the_player_body_even_with_entity_cancel() {
    use sunrise_client::advancements::Tree;
    use sunrise_client::effects::EffectInstance;
    use sunrise_client::lan::{ServerAdvancements, data::SyncData};
    use sunrise_world::loot::PackOverlay;

    let _publication = sunrise_client::datapack_net::isolate_server_publication();
    let heard = Arc::default();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "observe",
            Box::new(Observe {
                events: Arc::clone(&heard),
                cancel: 23,
                check_delayed_destroy: false,
            }),
        )]
        .into(),
        || {
            with_game(|game| {
                game.world_root = game.options.game_dir.clone().unwrap();
                // Load the world's tables before installing the test trees. Otherwise
                // the first tick's scheduled functions replace the game's tree.
                game.open_world_datapacks(&[], None).unwrap();
                open_to_lan(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));

                let pack = tempfile::tempdir().unwrap();
                let data = pack.path().join("data");
                let advancements = data.join("test/advancement");
                std::fs::create_dir_all(&advancements).unwrap();
                std::fs::write(
                    advancements.join("phase.json"),
                    br#"{
                "criteria": {
                    "tick": {"trigger": "minecraft:tick"},
                    "levitation": {"trigger": "minecraft:levitation"},
                    "location": {"trigger": "minecraft:location"}
                },
                "requirements": [["tick"], ["levitation"], ["location"]]
            }"#,
                )
                .unwrap();
                game.adv.tree = Arc::new(Tree::load(&data));
                // This unconditional criterion is only in the hub's tree, so it
                // must go through the staged native first-tick grant path.
                std::fs::write(
                    advancements.join("staged.json"),
                    br#"{
                "criteria": {"staged": {"trigger": "minecraft:tick"}},
                "requirements": [["staged"]]
            }"#,
                )
                .unwrap();
                let mut sync = SyncData::empty();
                sync.advancements =
                    ServerAdvancements::load(pack.path(), &PackOverlay::NONE, Default::default());
                game.with_advancement_hub(|hub, owners| {
                    hub.reload_resources(Arc::new(sync), owners)
                });

                let levitation = sunrise_client::brewing::effect("levitation").unwrap().id;
                for (conn, ticks) in [(1, 19), (4, 20)] {
                    let mut player = guest(conn);
                    player.dimension = game.dim.id().into();
                    player.loaded = true;
                    player.position = [5.5 + f64::from(conn), 64.0, 8.5];
                    game.lan.as_mut().unwrap().hub.guests.push(player);
                    game.as_guest(conn, |g| {
                        g.player.as_mut().unwrap().health.tick_count = ticks;
                        g.potions
                            .effects
                            .add(EffectInstance::new(levitation, 100, 0));
                    })
                    .unwrap();
                }
                game.tick_server(false).unwrap();

                let host = game.lan.as_ref().unwrap();
                assert!(host.hub.guest(1).unwrap().advancements_sent);
                assert!(!host.hub.guest(4).unwrap().advancements_sent);
                assert!(host.hub.native_advancement_grants.is_empty());
                use Event::*;
                let events = heard.lock().unwrap();
                let position = |event: &Event| {
                    events
                        .iter()
                        .position(|e| e == event)
                        .unwrap_or_else(|| panic!("missing {event:?}; saw {events:?}"))
                };
                let post_entity = position(&EntityPost(20));
                for criterion in ["tick", "levitation", "staged"] {
                    assert!(position(&Criterion(123, criterion.into())) < post_entity);
                    assert!(!events.contains(&Criterion(126, criterion.into())));
                }
                for player in [123, 126] {
                    let pre = events
                        .iter()
                        .position(|e| matches!(e, PlayerPre(uuid, ..) if *uuid == player))
                        .unwrap();
                    let post = position(&PlayerPost(player));
                    assert!(post_entity < pre && pre < post);
                    assert!(post < position(&Criterion(player, "location".into())));
                }
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn player_tick_hooks_observe_advanced_dig_progress_and_skipped_body_still_ticks_game_mode() {
    let heard = Arc::default();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "observe",
            Box::new(Observe {
                events: Arc::clone(&heard),
                cancel: 23,
                check_delayed_destroy: true,
            }),
        )]
        .into(),
        || {
            with_game(|game| {
                game.world_root = game.options.game_dir.clone().unwrap();
                open_to_lan(game);
                game.build_registries();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let start = 20_000;
                game.lan.as_mut().unwrap().hub.ticks = start;
                // MinecraftServer.createLevels opens the Overworld first, so it ticks first.
                game.native_level_opened(Dim::Overworld);
                for (dim, conns) in [(Dim::Overworld, [1, 3]), (Dim::Nether, [2, 4])] {
                    game.enter_level(dim).unwrap();
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    if dim == Dim::Overworld {
                        let [x, y, z] = DELAYED;
                        game.world.set(x, y, z, stone).unwrap();
                    }
                    for conn in conns {
                        let mut player = guest(conn);
                        player.dimension = dim.id().into();
                        player.loaded = true;
                        let spec = conn == 3;
                        let x = if spec { 15 } else { 5 + conn as i32 };
                        player.position = [f64::from(x) + 0.5, 64.0, 8.5];
                        if spec {
                            player.game_mode = SPECTATOR;
                            assert!(!game.guest_body_ticks(&player));
                        }
                        let pos = [x, 63, 8];
                        game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
                        let host = game.lan.as_mut().unwrap();
                        host.hub.guests.push(player);
                        let play = host.play.entry(conn).or_default();
                        if spec {
                            play.destroy_delay = 5;
                        } else {
                            play.arm_mining_for_test(
                                pos,
                                start,
                                (conn == 1).then_some((DELAYED, 0)),
                            );
                        }
                        game.as_guest(conn, |game| {
                            let health = &mut game.player.as_mut().unwrap().health;
                            health.tick_count = 10;
                            health.invulnerable_time = 8;
                            health.hurt.cooldown = 5;
                        })
                        .unwrap();
                    }
                }
                game.enter_level(Dim::Overworld).unwrap();
                game.tick_server(false).unwrap();

                let host = game.lan.as_ref().unwrap();
                for (conn, count) in [(1, 11), (2, 11), (3, 11), (4, 10)] {
                    assert_eq!(
                        host.hub
                            .guest(conn)
                            .unwrap()
                            .motion
                            .as_ref()
                            .unwrap()
                            .health
                            .tick_count,
                        count
                    );
                }
                for conn in [1, 2] {
                    let mining = host.play[&conn].mining.as_ref().unwrap();
                    assert_eq!(mining.ticks, 2);
                    assert!(mining.progress > 0.0);
                }
                assert_eq!(host.play[&4].mining.as_ref().unwrap().ticks, 1);
                assert_eq!(host.play[&3].destroy_delay, 4);
                use Event::*;
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        EntityPre(20, Some(8)),
                        Dig(123, 0),
                        EntityPost(20),
                        EntityPre(22, Some(8)),
                        EntityPost(22),
                        LevelPost("minecraft:overworld"),
                        EntityPre(21, Some(8)),
                        Dig(124, 0),
                        EntityPost(21),
                        EntityPre(23, Some(8)),
                        LevelPost("minecraft:the_nether"),
                        PlayerPre(123, "minecraft:overworld", Some(7), Some(4)),
                        PlayerPost(123),
                        PlayerPre(124, "minecraft:the_nether", Some(7), Some(4)),
                        PlayerPost(124),
                        PlayerPre(126, "minecraft:the_nether", Some(8), Some(5)),
                        PlayerPost(126),
                    ],
                );
            })
        },
    );
}
