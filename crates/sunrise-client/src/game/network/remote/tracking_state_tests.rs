use crate::game::remote::{Tracked, entity_sound_state, seat_passengers};

#[test]
#[ignore = "daily"]
fn tracking_emitters_follow_the_packet_target() {
    use crate::game::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
        totem::TOTEM_EVENT,
    };
    use sunrise_client::{motion::PlayerMotion, particles::ParticleType, pose::Pose};
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::{
        entity_data::{Value, serializer},
        packets::play::{self, Clientbound as C},
    };

    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "EmitterTest").unwrap();
    let remote = game.remote.as_mut().unwrap();
    remote.entity_id = 7;
    remote.logged_in = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap());
    game.particles.seed_randoms(42, 43, 44);

    let check = |game: &Game<'_>, ty, feet: [f64; 3], height: f32| {
        let mut seen = false;
        for (options, at, velocity) in game.particles.requested.as_ref().unwrap() {
            if options.ty != ty {
                continue;
            }
            seen = true;
            let y = feet[1] + f64::from(height) * (0.5 + (velocity[1] - 0.2) / 4.0);
            assert!((at[0] - feet[0]).abs() <= f64::from(0.6_f32) / 4.0);
            assert!(
                (at[1] - y).abs() < 1e-10,
                "{ty:?} at {at:?}, expected y {y}"
            );
            assert!((at[2] - feet[2]).abs() <= f64::from(0.6_f32) / 4.0);
        }
        assert!(seen, "no {ty:?} emissions");
    };
    let types = [
        ParticleType::Crit,
        ParticleType::EnchantedHit,
        ParticleType::TotemOfUndying,
    ];
    let target = 40;
    let start = [5.0, 64.0, 7.0];
    game.spawn_entity(target, "minecraft:player", start, [0.0; 3]);
    game.particles.requested = Some(Vec::new());
    for action in [1, 2] {
        game.handle_packet(C::Animate(play::Animate { id: target, action }))
            .unwrap();
    }
    game.handle_packet(C::EntityEvent(play::EntityEvent {
        id: target,
        event: TOTEM_EVENT,
    }))
    .unwrap();
    for ty in types {
        check(&game, ty, start, 1.8);
    }

    let moved = [15.0, 65.0, -2.0];
    game.particles.requested = Some(Vec::new());
    game.handle_packet(C::SetEntityData(play::SetEntityData {
        id: target,
        values: vec![(6, serializer::POSE, Value::Id(Pose::Crouching.id()))],
    }))
    .unwrap();
    game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
        id: target,
        path: play::PositionPath::Linear(moved),
        yaw: 0.0,
        pitch: 0.0,
        on_ground: true,
    }))
    .unwrap();
    for _ in 0..4 {
        game.tick_remote_entities(true);
    }
    game.tick_particles();
    for ty in types {
        check(&game, ty, moved, 1.5);
    }

    // The old entity moved again before unpairing. Re-pairing its id before
    // the next particle tick keeps emitting at the old entity's final body.
    game.particles.requested = Some(Vec::new());
    let last = [18.0, 65.0, -2.0];
    game.mob_index
        .get_mut(&mut game.mobs, target as u32)
        .unwrap()
        .position = Vec3::new(last[0], last[1], last[2]);
    game.handle_packet(C::RemoveEntities(play::RemoveEntities {
        ids: vec![target],
    }))
    .unwrap();
    game.spawn_entity(target, "minecraft:player", [28.0, 67.0, 1.0], [0.0; 3]);
    game.tick_particles();
    for ty in types {
        check(&game, ty, last, 1.5);
    }

    game.particles.clear();
    game.particles.requested = Some(Vec::new());
    game.handle_packet(C::EntityEvent(play::EntityEvent {
        id: 7,
        event: TOTEM_EVENT,
    }))
    .unwrap();
    check(&game, ParticleType::TotemOfUndying, [0.0, 64.0, 0.0], 1.8);
    game.particles.requested = Some(Vec::new());
    let player = game.player.as_mut().unwrap();
    player.position = Vec3::new(-4.0, 68.0, 8.0);
    player.set_pose(Pose::Swimming).unwrap();
    game.tick_particles();
    check(&game, ParticleType::TotemOfUndying, [-4.0, 68.0, 8.0], 0.6);
}

/// `ClientPacketListener.handleSetEntityPassengersPacket`: the vehicle
/// ejects its passengers, then each listed entity `startRiding`s it, so
/// `isVehicle` holds while the list is not empty, whatever the passenger's
/// type: a server's item display riding a player hides that player's name
/// tag like a text display does (#5666).
#[test]
fn any_tracked_passenger_makes_its_vehicle_a_vehicle() {
    use std::collections::{HashMap, HashSet};
    let tracked = |kind: &str| Tracked {
        kind: kind.to_owned(),
        position: [0.0, 64.0, 0.0],
        previous: [0.0, 64.0, 0.0],
        yaw: 0.0,
        pitch: 0.0,
        previous_yaw: 0.0,
        previous_pitch: 0.0,
        codec_base: [0.0, 64.0, 0.0],
        on_fire: false,
        mod_fire_immune: false,
        silent: false,
        vehicle: None,
        tick_count: 0,
        size_answer: None,
    };
    let mut entities = HashMap::from([
        (42, tracked("minecraft:player")),
        (43, tracked("minecraft:player")),
        (9, tracked("minecraft:item_display")),
        (10, tracked("minecraft:block_display")),
    ]);
    let vehicles = |e: &HashMap<i32, Tracked>| -> HashSet<u32> {
        e.values().filter_map(|t| t.vehicle).collect()
    };
    seat_passengers(&mut entities, 42, &[9, 10]);
    assert_eq!(vehicles(&entities), HashSet::from([42]));
    // The list replaces the old one: 10 got off, 9 still rides.
    seat_passengers(&mut entities, 42, &[9]);
    assert_eq!(entities[&10].vehicle, None);
    assert_eq!(entities[&9].vehicle, Some(42));
    // startRiding leaves the old vehicle first.
    seat_passengers(&mut entities, 43, &[9]);
    assert_eq!(vehicles(&entities), HashSet::from([43]));
    // An empty list: nobody rides 43 any more.
    seat_passengers(&mut entities, 43, &[]);
    assert!(vehicles(&entities).is_empty());
}

#[test]
fn a_dragon_sound_uses_live_movement_without_losing_mob_metadata() {
    use sunrise_client::sound_engine::SoundEntityState;
    let stale = SoundEntityState {
        position: [0.0, 64.0, 0.0],
        silent: true,
        removed: false,
    };
    let tracked = std::collections::HashMap::new();
    for position in [[24.0, 80.0, -16.0], [25.5, 81.0, -18.0]] {
        let state =
            entity_sound_state(7, None, Some(stale), &tracked, Some((7, position))).unwrap();
        assert_eq!(state.position, position);
        assert!(state.silent);
        assert!(!state.removed);
    }
    let unrelated =
        entity_sound_state(7, None, Some(stale), &tracked, Some((8, [99.0; 3]))).unwrap();
    assert_eq!(unrelated.position, stale.position);
    assert!(entity_sound_state(7, None, None, &tracked, Some((7, [99.0; 3]))).is_none());
}
