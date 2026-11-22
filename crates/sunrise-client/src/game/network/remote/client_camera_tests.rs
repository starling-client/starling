use crate::game::remote::RemoteDimensionEntry;
use crate::game::{
    Game, native_entity_refs::tests::with_client_game, native_lifecycle::NativeLifecycle,
};
use mod_api::ClientHost;
use sunrise_client::{level_loading::Reason, motion::PlayerMotion};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C, SpawnInfo},
};

#[test]
#[ignore = "daily"]
fn client_camera_entity_returns_to_player_after_removal() {
    with_client_game(|game| {
        let camera = |game: &Game<'_>| (game as &dyn ClientHost).camera_entity();
        assert!(camera(game).is_none(), "the fixture has no player");
        game.native_lifecycle = NativeLifecycle::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "CameraReadTest").unwrap();
        let mut spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 3,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        assert!(camera(game).is_none(), "a level alone has no player");
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let local = camera(game).unwrap();
        assert_eq!(
            local.id(),
            7,
            "use the remote identity, not PlayerMotion's default"
        );
        assert_eq!(local.position(game), Some([0.0; 3]));

        let creeper = game
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:creeper")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 31,
            uuid: 9,
            kind: creeper,
            position: [10.0, 64.0, 12.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        game.remote_set_camera(31);
        assert!(!game.controls_camera());
        assert!(game.hides_as_camera(31));
        assert!(game.spectated_post_effect.is_some());
        let attached = camera(game).unwrap();
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![99] }))
            .unwrap();
        assert_eq!(camera(game), Some(attached));
        assert_eq!(attached.id(), 31);
        assert_eq!(attached.position(game), Some([10.0, 64.0, 12.0]));
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 99 }))
            .unwrap();
        assert_eq!(
            camera(game),
            Some(attached),
            "unknown packet targets are ignored"
        );
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![31] }))
            .unwrap();
        assert_eq!(camera(game), Some(local));
        assert!(
            game.controls_camera(),
            "movement input and position packets are enabled"
        );
        assert!(!game.hides_as_camera(31));
        assert!(game.spectated_post_effect.is_none());
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 7 }))
            .unwrap();
        assert_eq!(camera(game), Some(local));

        game.title = Some(game.new_title(game.save.root.clone()));
        assert!(
            camera(game).is_none(),
            "the title screen has no camera entity"
        );
        game.title = None;
        assert_eq!(camera(game), Some(local));

        spawn.dimension = "test:other_overworld".into();
        game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
            .unwrap();
        assert!(
            camera(game).is_none(),
            "replacement waits for its new player"
        );
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let fresh = camera(game).unwrap();
        assert_eq!(fresh.id(), local.id());
        assert_ne!(fresh.epoch(), local.epoch());
        assert_eq!(local.position(game), None, "old level handles stay stale");
        assert_eq!(fresh.position(game), Some([0.0; 3]));
        game.client_logging_out();
        assert!(camera(game).is_none());
        assert_eq!(fresh.position(game), None);
    });
}
