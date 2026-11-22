//! `handlePlayerPositionChange`: a move that goes up ends the guest's fall.
use super::guest;
use crate::game::Game;
use std::{path::Path, sync::Arc};
use sunrise_client::lan::guest::InboxItem;
use sunrise_client::{
    advancements::Tree,
    chunk::Chunk,
    items::ItemStack,
    lan::{NetConfig, hub::HubConfig},
};
use sunrise_core::position::ChunkPos;
use sunrise_protocol::packets::{Empty, play, play::Serverbound as S};

const NAME: &str = "GuestBodyTest";
/// `fall_from_height` with no condition, then with a fall of at most 1 block, and of 5.
const CRITERIA: [&str; 3] = ["any", "short", "five"];

/// A server with one survival guest in the air at 12.5 110 12.5, over a stone floor at y 99.
fn scene<'a>(resources: &'a mut Option<Arc<crate::game::Resources>>, scratch: &Path) -> Game<'a> {
    let mut g = crate::game::game_tests::tests::server_game(resources);
    g.build_registries();
    g.set_allow_commands(true);
    let folder = scratch.join("data/fall/advancement");
    std::fs::create_dir_all(&folder).unwrap();
    g.world_root = scratch.to_path_buf();
    for (name, distance) in CRITERIA.into_iter().zip([
        serde_json::Value::Null,
        serde_json::json!({"max": 1.0}),
        serde_json::json!({"min": 4.9, "max": 5.1}),
    ]) {
        let mut criterion = serde_json::json!({"trigger": "minecraft:fall_from_height"});
        if !distance.is_null() {
            criterion["conditions"] = serde_json::json!({"distance": {"y": distance}});
        }
        let advancement = serde_json::json!({"criteria": {name: criterion}});
        std::fs::write(folder.join(format!("{name}.json")), advancement.to_string()).unwrap();
    }
    g.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
    assert_eq!(g.adv.tree.list.len(), CRITERIA.len());
    let stone = g.pack.assets.registry.default_state("minecraft:stone");
    let mut chunk = Chunk::empty(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set(x, 99, z, stone.unwrap().0);
        }
    }
    g.world.chunks.insert((0, 0), Arc::new(chunk));
    g.block_ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(g.run_command("forceload add 0 0"));
    assert!(g.run_command("tick freeze"));
    g.lan_host(
        crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, "guest-fall-reset".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let mut guest = guest(1);
    guest.position = [12.5, 110.0, 12.5];
    guest.placed = true;
    guest.loaded = true;
    guest.dimension = "minecraft:overworld".into();
    guest.game_mode = 0;
    guest.on_ground = false;
    guest.load_timeout = 0;
    g.lan.as_mut().unwrap().hub.guests.push(guest);
    g.sync_tick_chunks();
    g.ticked_entities_through = g.next_entity;
    g
}

/// One server packet drain; put a client-tick marker between reports.
fn drain_moves(g: &mut Game<'_>, reports: impl IntoIterator<Item = (f64, u8)>) {
    g.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
        guest
            .inbox
            .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
        for (y, flags) in reports {
            let to = play::MovePlayerPos {
                x: 12.5,
                y,
                z: 12.5,
                flags,
            };
            guest
                .inbox
                .push_back(InboxItem::Packet(S::MovePlayerPos(to), None));
            guest
                .inbox
                .push_back(InboxItem::Packet(S::ClientTickEnd(Empty), None));
        }
    });
    g.tick_lan_packets();
}

/// One server tick in which the guest's client sent these airborne positions.
fn tick_with_moves(
    g: &mut Game<'_>,
    heights: &[f64],
    before_level: (f64, Option<[f64; 3]>, [bool; 3]),
) {
    drain_moves(g, heights.iter().map(|&y| (y, 0)));
    let guest = &g.lan.as_ref().unwrap().hub.guests[0];
    assert_eq!(guest.moves.len(), heights.len(), "every move is accepted");
    assert_eq!(guest.position[1], *heights.last().unwrap());
    assert_eq!(fall(g), before_level, "state before the level tick");
    g.tick_lan(false);
}

/// The guest's fall distance, where its tracked fall began, and which criteria it has.
fn fall(g: &Game<'_>) -> (f64, Option<[f64; 3]>, [bool; 3]) {
    let host = g.lan.as_ref().unwrap();
    let guest = &host.hub.guests[0];
    let done = CRITERIA.map(|name| {
        let i = g.adv.tree.find_id(&format!("fall:{name}")).unwrap();
        guest.advancements.done(&g.adv.tree.list[i])
    });
    (
        guest.motion.as_ref().unwrap().fall_distance,
        host.play.get(&1).unwrap().marks.fall,
        done,
    )
}

#[test]
#[ignore = "daily"]
fn a_guest_move_that_goes_up_ends_its_fall() {
    crate::game::in_own_process(
        "game::lan_actions::guest_body::tests::fall_reset::a_guest_move_that_goes_up_ends_its_fall",
        || {
            let mut resources = None;
            let scratch = tempfile::tempdir().unwrap();
            let mut g = scene(&mut resources, scratch.path());
            // One block down: a fall of 1, tracked from where the tick left the guest.
            tick_with_moves(&mut g, &[109.0], (1.0, None, [false; 3]));
            assert_eq!(fall(&g), (1.0, Some([12.5, 109.0, 12.5]), [false; 3]));
            // Half a block up: no fall left, and the fall of half a block is awarded.
            tick_with_moves(&mut g, &[109.5], (0.0, None, [true, true, false]));
            assert_eq!(fall(&g), (0.0, None, [true, true, false]));

            for name in ["any", "short"] {
                assert!(g.run_command(&format!("advancement revoke {NAME} only fall:{name}")));
            }
            tick_with_moves(&mut g, &[108.5], (1.0, None, [false; 3]));
            assert_eq!(fall(&g), (1.0, Some([12.5, 108.5, 12.5]), [false; 3]));
            // Three moves in one server tick: down to 104, up to 104.5, down to 103.5. The move up
            // ends the fall where it is, 4 blocks below its start, not at the tick's end, 5 below.
            // The last move starts a new fall of 1.
            tick_with_moves(
                &mut g,
                &[104.0, 104.5, 103.5],
                (1.0, None, [true, false, false]),
            );
            assert_eq!(
                fall(&g),
                (1.0, Some([12.5, 103.5, 12.5]), [true, false, false])
            );

            // A player that dies later in the tick was alive at its move up, so the fall still counts:
            // half a heart left, and a block at its head to suffocate in once it has moved.
            assert!(g.run_command(&format!("advancement revoke {NAME} only fall:any")));
            assert!(g.run_command("gamerule natural_health_regeneration false"));
            let health = g.lan.as_ref().unwrap().hub.guests[0].health;
            assert!(g.run_command(&format!("damage {NAME} {} minecraft:generic", health - 0.5)));
            // The damage clock runs out, so the next damage counts.
            for _ in 0..20 {
                g.tick_lan(false);
            }
            assert!(g.run_command("setblock 12 105 12 minecraft:stone"));
            tick_with_moves(&mut g, &[104.0], (0.0, None, [true, true, false]));
            let guest = &g.lan.as_ref().unwrap().hub.guests[0];
            assert!(
                guest.health <= 0.0,
                "the guest suffocated: {}",
                guest.health
            );
            // Half a block up from where the last fall began: any fall, and a short one.
            assert_eq!(fall(&g).2, [true, true, false]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn entering_water_ends_a_tracked_fall_before_the_next_move() {
    crate::game::in_own_process(
        "game::lan_actions::guest_body::tests::fall_reset::entering_water_ends_a_tracked_fall_before_the_next_move",
        || {
            let mut resources = None;
            let scratch = tempfile::tempdir().unwrap();
            let mut g = scene(&mut resources, scratch.path());
            assert!(g.run_command("setblock 12 104 12 minecraft:water"));
            tick_with_moves(&mut g, &[105.0], (5.0, None, [false; 3]));
            assert_eq!(fall(&g).1, Some([12.5, 105.0, 12.5]));

            // Enter airborne water a half block down, then fall below it in the same batch.
            drain_moves(&mut g, [(104.5, 0), (101.5, 0)]);
            assert_eq!(g.lan.as_ref().unwrap().hub.guests[0].position[1], 101.5);
            assert_eq!(fall(&g), (0.0, None, [true, true, false]));
        },
    );
}

fn lethal_landing(totem: bool) {
    let mut resources = None;
    let scratch = tempfile::tempdir().unwrap();
    let mut g = scene(&mut resources, scratch.path());
    tick_with_moves(&mut g, &[105.0], (5.0, None, [false; 3]));
    g.as_guest(1, |game| {
        game.player
            .as_mut()
            .unwrap()
            .set_health(0.5, false)
            .unwrap();
        game.inventory[0] = Some(ItemStack::new("minecraft:diamond", 1));
        if totem {
            game.offhand = Some(ItemStack::new("minecraft:totem_of_undying", 1));
        }
    })
    .unwrap();

    // Land, then report stepping up, with no world or connection tick in between.
    drain_moves(&mut g, [(100.0, 1), (100.5, 0)]);
    let guest = &g.lan.as_ref().unwrap().hub.guests[0];
    if totem {
        assert!(!guest.dead);
        assert_eq!(guest.health, 1.0);
        assert!(guest.offhand.is_none());
        assert_eq!(guest.position[1], 100.5);
        assert_eq!(fall(&g), (0.0, None, [true, false, true]));
    } else {
        assert!(guest.dead);
        assert_eq!(guest.position[1], 100.0);
        assert_eq!(guest.last_death.as_ref().unwrap().1, [12, 100, 12]);
        let drop = g
            .dropped
            .iter()
            .find(|item| item.stack.item.as_ref() == "minecraft:diamond")
            .unwrap();
        assert_eq!(
            drop.position.y,
            100.0 + guest.eye_height() - f64::from(0.3_f32)
        );
        assert_eq!(fall(&g), (0.0, None, [false; 3]));
    }
}

#[test]
#[ignore = "daily"]
fn a_lethal_landing_records_death_and_drops_before_the_next_move() {
    crate::game::in_own_process(
        "game::lan_actions::guest_body::tests::fall_reset::a_lethal_landing_records_death_and_drops_before_the_next_move",
        || {
            lethal_landing(false);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_totem_saves_the_guest_before_its_tracked_fall_ends() {
    crate::game::in_own_process(
        "game::lan_actions::guest_body::tests::fall_reset::a_totem_saves_the_guest_before_its_tracked_fall_ends",
        || {
            lethal_landing(true);
        },
    );
}
