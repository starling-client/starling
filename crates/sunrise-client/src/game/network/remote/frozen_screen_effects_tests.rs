//! `/tick freeze` stops a client's level, not its HUD: vanilla 26.3's `Minecraft.tick` runs
//! `Gui.tick` on every tick the game isn't paused, and `Hud.tick` eases the vignette there
//! (`updateVignetteBrightness`). Only the vignette moved off the level's tick: the player's own
//! screen effects (the portal's, Nausea's, the water vision) still tick with it.
use crate::game::item_entities::thrown_items_tests::with_remote_game;
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// `Hud.vignetteBrightness` in vanilla 26.3 at the end of matched client ticks, as float bits:
/// the player's eye at sky light 15 at noon (the target is 0), read by a temporary hook in the
/// bench mod (`.local/evidence/VanillaBench-vignette.java`) every tick of a run that froze the
/// level (`tick freeze`), then unfroze it (`tick unfreeze`). Ticks 206 to 218 of that run: the
/// last running tick, then twelve frozen ones; and 266 to 271: two frozen, four running again.
const JAVA_FREEZING: [u32; 13] = [
    0x3dff_bfeb,
    0x3dfd_3133,
    0x3dfa_a907,
    0x3df8_2756,
    0x3df5_ac10,
    0x3df3_3724,
    0x3df0_c882,
    0x3dee_601a,
    0x3deb_fddc,
    0x3de9_a1b9,
    0x3de7_4ba0,
    0x3de4_fb82,
    0x3de2_b150,
];
const JAVA_UNFREEZING: [u32; 6] = [
    0x3d8b_ef63,
    0x3d8a_8927,
    0x3d89_2680,
    0x3d87_c765,
    0x3d86_6bcd,
    0x3d85_13af,
];

#[test]
#[ignore = "daily"]
fn a_frozen_level_eases_the_vignette_as_vanillas_does() {
    with_remote_game(|game| {
        let spawn = play::SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec![spawn.dimension.clone()],
            max_players: 1,
            chunk_radius: 2,
            simulation_distance: 2,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn,
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        // Java's camera: the eye at 176.62 over an open sky at noon.
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [8.5, 175.0, 8.5],
            delta: [0.0; 3],
            yaw: 180.0,
            pitch: -35.0,
            relatives: 0,
        }))
        .unwrap();
        game.world.day_time = 6000;
        let mut chunk = sunrise_client::chunk::Chunk::empty(0, 0);
        chunk.lit = true;
        game.streaming.insert_remote(&mut game.world, chunk);
        let ticking = |frozen| C::TickingState(play::TickingState { rate: 20.0, frozen });
        let bits = |game: &crate::game::Game<'_>| game.screen_fx.vignette_brightness.to_bits();

        game.screen_fx.vignette_brightness = f32::from_bits(JAVA_FREEZING[0]);
        game.handle_packet(ticking(true)).unwrap();
        assert!(game.clock.frozen());
        for (tick, want) in JAVA_FREEZING.iter().enumerate().skip(1) {
            game.tick_remote().unwrap();
            assert_eq!(bits(game), *want, "frozen tick {tick}");
        }

        game.screen_fx.vignette_brightness = f32::from_bits(JAVA_UNFREEZING[0]);
        game.tick_remote().unwrap();
        assert_eq!(bits(game), JAVA_UNFREEZING[1], "the last frozen tick");
        game.handle_packet(ticking(false)).unwrap();
        assert!(!game.clock.frozen());
        for (tick, want) in JAVA_UNFREEZING.iter().enumerate().skip(2) {
            game.tick_remote().unwrap();
            assert_eq!(bits(game), *want, "running tick {tick}");
        }
    });
}
