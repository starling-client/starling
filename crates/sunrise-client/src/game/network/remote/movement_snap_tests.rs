use crate::game::remote::{shape_meets, snap_target};

/// The packet riding graph must retain invisible intermediary entities. A custom
/// sit plugin can use an area-effect cloud, whose server position stops updating
/// while it rides a player, with a player or marker stand as its own passenger.
#[test]
#[ignore = "daily"]
fn plugin_player_stack_follows_network_moves_and_local_carriers() {
    use crate::game::{
        Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources,
    };
    use sunrise_client::{motion::PlayerMotion, sprint::Keys};
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::play::{self, Clientbound as C};
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
        ExecutionRole::Client,
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
    game.connect_local(client, "SeatTest").unwrap();
    game.remote.as_mut().unwrap().entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    game.spawn_entity(40, "minecraft:player", [2.0, 64.0, 3.0], [0.0; 3]);
    game.spawn_entity(41, "minecraft:player", [0.0, 64.0, 0.0], [0.0; 3]);
    game.spawn_entity(42, "minecraft:area_effect_cloud", [0.0; 3], [0.0; 3]);
    let set = |g: &mut Game<'_>, vehicle, passengers: Vec<i32>| {
        g.handle_packet(C::SetPassengers(play::SetPassengers {
            vehicle,
            passengers,
        }))
        .unwrap();
    };
    set(&mut game, 40, vec![42]);
    set(&mut game, 42, vec![7, 41]);
    let close = |at: Vec3, x: f64, y: f64, z: f64| {
        assert!(
            (at.x - x).abs() < 1e-5 && (at.y - y).abs() < 1e-5 && (at.z - z).abs() < 1e-5,
            "position {at:?}, expected [{x}, {y}, {z}]"
        );
    };
    assert!(game.remote_ride(Keys::default()));
    close(game.player.as_ref().unwrap().position, 2.0, 65.7, 3.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        2.0,
        65.7,
        3.0,
    );
    game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
        id: 40,
        path: play::PositionPath::Linear([12.0, 67.0, -4.0]),
        yaw: 45.0,
        pitch: 0.0,
        on_ground: true,
    }))
    .unwrap();
    for _ in 0..4 {
        game.tick_remote_entities(true);
        game.remote_ride(Keys::default());
    }
    close(game.player.as_ref().unwrap().position, 12.0, 68.7, -4.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        12.0,
        68.7,
        -4.0,
    );
    // The intermediary's packet position must remain unchanged for relative decoding.
    assert_eq!(
        game.remote.as_ref().unwrap().tracked(42).unwrap().1,
        [0.0; 3]
    );
    // A remote player riding the local player must follow our own movement too.
    set(&mut game, 42, vec![41]);
    set(&mut game, 7, vec![42]);
    game.player.as_mut().unwrap().position = Vec3::new(20.0, 70.0, 25.0);
    game.position_riders();
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        20.0,
        71.7,
        25.0,
    );
    // A direct stack has no intermediary's 0.5-height offset.
    set(&mut game, 40, vec![7, 41]);
    game.remote_ride(Keys::default());
    close(game.player.as_ref().unwrap().position, 12.0, 68.2, -4.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        12.0,
        68.2,
        -4.0,
    );
    // Removing the carrier detaches passengers even without a final empty packet.
    game.remove_entity(40);
    assert_eq!(game.riding, None);
    assert_eq!(
        game.mobs.iter().find(|m| m.id == 41).unwrap().extra.mount,
        None
    );
}

/// `Entity.isColliding` and the snap `ClientLevel.syncBlockState` makes: a
/// block the server puts back where the player stands sends it to the
/// position the prediction remembered, and a block that comes back clear of
/// the player, an empty one, or one the player has been teleported since
/// leaves it where it is.
#[test]
fn a_block_the_server_puts_back_snaps_the_player_out_of_it() {
    use sunrise_core::geometry::{Aabb, Vec3, VoxelShape};
    // A player standing in block (0, 64, 0): 0.6 wide, 1.8 tall.
    let player = Aabb::new(Vec3::new(0.2, 64.0, 0.2), Vec3::new(0.8, 65.8, 0.8));
    let cube = VoxelShape::block();
    assert!(
        shape_meets(&cube, [0, 64, 0], player),
        "the block the player is standing in"
    );
    assert!(!shape_meets(&cube, [0, 66, 0], player), "above its head");
    assert!(
        !shape_meets(&cube, [1, 64, 0], player),
        "the block beside it"
    );
    assert!(!shape_meets(&VoxelShape::empty(), [0, 64, 0], player));

    // The snap itself: the remembered position, unless a teleport came after
    // the prediction (`lastTeleportSequence < sequence`).
    let at = [0.5, 64.0, 0.5];
    assert_eq!(snap_target(at, -1, 3, true), Some(at));
    assert_eq!(snap_target(at, -1, 3, false), None, "a block it clears");
    assert_eq!(snap_target(at, 3, 3, true), None, "teleported at 3");
    assert_eq!(
        snap_target(at, 2, 3, true),
        Some(at),
        "teleported before it"
    );
}

/// tooBigToInterpolate: a position sync measures from where the entity
/// is, a teleport from where its interpolation is headed.
#[test]
fn a_far_move_snaps_by_the_packets_own_measure() {
    use super::super::{MovePacket, Path, PosRot, SteppedInterpolation};
    let at = |x: f64| PosRot {
        position: [x, 0.0, 0.0],
        yaw: 0.0,
        pitch: 0.0,
    };
    let mut handler = SteppedInterpolation::new(3);
    // At x 0, headed for x 50.
    handler.interpolate_to(at(0.0), Some(Path::Linear([50.0, 0.0, 0.0])), None);
    let to = [100.0, 0.0, 0.0];
    assert!(MovePacket::PositionSync.snaps(&handler, at(0.0), to));
    assert!(!MovePacket::Teleport.snaps(&handler, at(0.0), to));
    assert!(!MovePacket::Move.snaps(&handler, at(0.0), to));
    // With nothing in flight, both measure from the entity.
    let idle = SteppedInterpolation::new(3);
    assert!(MovePacket::Teleport.snaps(&idle, at(0.0), to));
    assert!(!MovePacket::Teleport.snaps(&idle, at(0.0), [60.0, 0.0, 0.0]));
}

/// A far sync puts the entity there and drops the steps it was walking;
/// a rotation alone keeps the position it is headed for and turns there
/// over the update interval.
#[test]
fn a_living_entitys_move_snaps_or_steps() {
    use super::super::{MovePacket, Path, PosRot, SteppedInterpolation, stepped_move};
    let at = |x: f64, yaw: f32| PosRot {
        position: [x, 0.0, 0.0],
        yaw,
        pitch: 0.0,
    };
    let mut handler = SteppedInterpolation::new(3);
    // Walking to x 5 when the far sync comes.
    handler.interpolate_to(at(0.0, 0.0), Some(Path::Linear([5.0, 0.0, 0.0])), None);
    assert!(handler.active());
    let far = [100.0, 0.0, 0.0];
    let snap = stepped_move(
        &mut handler,
        MovePacket::PositionSync,
        at(0.0, 0.0),
        far,
        Some(Path::Linear(far)),
        Some((90.0, 0.0)),
    );
    assert_eq!(snap, Some(at(100.0, 90.0)));
    assert!(!handler.active());
    assert_eq!(handler.interpolate(), None, "no step back toward x 5");
    // A rotation-only move packet.
    let turn = stepped_move(
        &mut handler,
        MovePacket::Move,
        at(100.0, 90.0),
        far,
        None,
        Some((180.0, 0.0)),
    );
    assert_eq!(turn, None);
    let steps: Vec<_> = std::iter::from_fn(|| handler.interpolate()).collect();
    assert_eq!(
        steps.iter().map(|s| s.yaw).collect::<Vec<_>>(),
        [120.0, 150.0, 180.0]
    );
    assert!(steps.iter().all(|s| s.position == far));
}
