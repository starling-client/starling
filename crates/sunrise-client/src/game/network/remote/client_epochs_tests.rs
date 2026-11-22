#[test]
#[ignore = "daily"]
fn client_epochs_follow_level_replacement_and_logout() {
    use crate::game::remote::RemoteDimensionEntry;
    use crate::game::{
        execution_role::ExecutionRole, native_entity_refs::tests::with_game,
        native_lifecycle::NativeLifecycle,
    };
    use mod_api::{CrosshairTarget, EntityRef};
    use sunrise_client::{level_loading::Reason, mobs::Mob, motion::PlayerMotion};
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{Clientbound, Respawn, SpawnInfo},
    };

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        game.native_lifecycle = NativeLifecycle::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "EpochTest").unwrap();
        // A transport alone has no ClientLevel yet.
        assert!(game.native_lifecycle.epoch().is_none());
        let mut spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 1,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let joined = game.native_lifecycle.epoch().unwrap();
        let old = EntityRef::new(joined, 12);
        let remote = game.remote.as_mut().unwrap();
        remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
        let open_generation = remote.menu.generation;
        game.handle_packet(Clientbound::Respawn(Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.native_lifecycle.epoch(), Some(joined));
        let remote = game.remote.as_mut().unwrap();
        assert!(remote.menu.as_ref().is_none());
        assert_ne!(remote.menu.generation, open_generation);
        let inventory_generation = remote.menu.generation;
        game.handle_packet(Clientbound::Respawn(Respawn {
            spawn: spawn.clone(),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.native_lifecycle.epoch(), Some(joined));
        let remote = game.remote.as_mut().unwrap();
        assert!(remote.menu.as_ref().is_none());
        assert_ne!(remote.menu.generation, inventory_generation);
        remote.menu.set(Some((7, "minecraft:generic_9x3".into())));
        assert_ne!(remote.menu.generation, open_generation);
        assert_ne!(remote.menu.generation, inventory_generation);

        // A different level key replaces the world even with the same
        // dimension type; IDs can be reused but old handles cannot.
        spawn.dimension = "test:other_overworld".into();
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Respawn, Reason::Other);
        let replaced = game.native_lifecycle.epoch().unwrap();
        assert_ne!(replaced, joined);
        let current = EntityRef::new(replaced, 12);
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs
            .push(Mob::new(12, kind, Vec3::new(0.0, 1.0, 2.0), 0.0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.yaw = 0.0;
        game.pitch = 0.0;
        game.refresh_client_pick(1.0);
        let Some(CrosshairTarget::Entity { entity, .. }) = game.crosshair_target() else {
            panic!("the crosshair hits the pig in front of the player");
        };
        assert_eq!(entity, current);
        assert!(
            game.native_entity_target(current).is_none(),
            "still server-only"
        );
        // Isolate the existing epoch check from the logical-server guard.
        game.options.execution_role = ExecutionRole::IntegratedServer;
        assert!(game.native_entity_target(current).is_some());
        assert!(game.native_entity_target(old).is_none());
        game.options.execution_role = ExecutionRole::Client;

        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
        assert!(game.crosshair_target().is_none());
        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "EpochTest").unwrap();
        assert!(game.native_lifecycle.epoch().is_none());
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        assert_ne!(game.native_lifecycle.epoch(), Some(replaced));
        game.client_logging_out();
        assert!(game.native_lifecycle.epoch().is_none());
    });
}
