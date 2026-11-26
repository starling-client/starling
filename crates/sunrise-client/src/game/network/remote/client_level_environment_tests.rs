use crate::game::{
    Dim, Game, Progress, native_entity_refs::tests::with_client_game, packs::stage_pack,
};
use std::sync::Arc;
use sunrise_client::environment::Sample;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};

/// The prepared resources' dimension type `name`, as a data pack writes it, with its
/// `minecraft:visual/cloud_height` set to `height`.
fn with_height(name: &str, height: f64) -> serde_json::Value {
    let resources = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let path = format!("data/minecraft/dimension_type/{name}.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(resources.join(path)).unwrap()).unwrap();
    json["attributes"]["minecraft:visual/cloud_height"] = serde_json::json!(height);
    json
}

/// The server's dimension types: each vanilla one with its cloud height raised (the Overworld's
/// to 250, the Nether's to 300, the End's to 400), and a mod's type with the Overworld's 250.
fn server_types() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("minecraft:overworld", with_height("overworld", 250.0)),
        ("minecraft:the_nether", with_height("the_nether", 300.0)),
        ("minecraft:the_end", with_height("the_end", 400.0)),
        ("test:high_sky", with_height("overworld", 250.0)),
    ]
}

fn spawn(dimension_type: i32, dimension: &str) -> play::SpawnInfo {
    play::SpawnInfo {
        dimension_type,
        dimension: dimension.into(),
        seed: 0,
        game_type: 1,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

/// The registry data a server sends in its configuration: `types` as its dimension types (ids
/// in order).
fn send_types(game: &mut Game<'_>, types: &[(&str, serde_json::Value)]) {
    let mut registries = (*game.remote.as_ref().unwrap().registries).clone();
    registries.registries.insert(
        "minecraft:dimension_type".into(),
        types
            .iter()
            .map(|(name, json)| {
                (
                    (*name).into(),
                    Some(sunrise_client::scoreboard::json_to_tag(json)),
                )
            })
            .collect(),
    );
    game.set_registries(Arc::new(registries)).unwrap();
}

/// The `Login` packet a server sends, into `entered`.
fn login(game: &mut Game<'_>, entered: play::SpawnInfo) {
    game.handle_packet(C::Login(play::Login {
        player_id: 7,
        hardcore: false,
        levels: vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ],
        max_players: 1,
        chunk_radius: 2,
        simulation_distance: 2,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: entered,
        online_mode: false,
        enforces_secure_chat: false,
    }))
    .unwrap();
}

/// Connects to a server whose dimension types are [`server_types`] and joins `entered`.
fn join(game: &mut Game<'_>, entered: play::SpawnInfo) -> Pipe {
    let (client, server) = Pipe::pair();
    game.connect_local(client, "SyncedEnvironmentTest").unwrap();
    send_types(game, &server_types());
    login(game, entered);
    server
}

fn respawn(game: &mut Game<'_>, spawn: play::SpawnInfo) {
    game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
        .unwrap();
}

/// A resource pack in a directory of its own whose Polytone dimension modifiers add a
/// post-process layer over each vanilla dimension's cloud height, each its own amount: the
/// Overworld's 10, the Nether's 20, the End's 30.
struct CloudPack(std::path::PathBuf);

impl CloudPack {
    fn new(case: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "client-level-environment-{case}-{}",
            std::process::id()
        ));
        let modifiers = dir.join("assets/minecraft/polytone/dimension_modifiers");
        std::fs::create_dir_all(&modifiers).unwrap();
        std::fs::write(
            dir.join("pack.mcmeta"),
            br#"{"pack": {"description": "cloud heights", "min_format": 97, "max_format": 97}}"#,
        )
        .unwrap();
        for (dimension, add) in [("overworld", 10), ("the_nether", 20), ("the_end", 30)] {
            std::fs::write(
                modifiers.join(format!("{dimension}.json")),
                format!(
                    r#"{{"attributes_modifiers": {{"post_process": {{"visual/cloud_height": "v + {add}"}}}}}}"#
                ),
            )
            .unwrap();
        }
        Self(dir)
    }

    fn pack(&self) -> sunrise_client::packs::Pack {
        sunrise_client::packs::pack_for(
            "server/cloud_heights".into(),
            sunrise_client::text::Text::literal("cloud heights"),
            sunrise_client::packs::Source::Dir(self.0.clone()),
            sunrise_client::packs::RESOURCE_FORMAT,
        )
        .unwrap()
    }
}

impl Drop for CloudPack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The client's resources reloaded with `server` as the server's packs, through the reload's
/// own staging and its render-thread apply.
fn reload(game: &mut Game<'_>, server: &[sunrise_client::packs::Pack]) {
    let staged = stage_pack(
        &game.vanilla_root(),
        &[],
        server,
        &game.mods,
        game.register_client_reload_listeners(
            &game
                .capture_candidate_settings(&game.options.resources)
                .unwrap(),
        ),
        &[],
        &game.pack.assets,
        &game.res,
        &game.pack.context,
        false,
        false,
        &Progress::new(),
    )
    .unwrap();
    game.apply_staged_pack(staged).unwrap();
}

/// The frame's sample at the camera: the dimension's value, as no biome sets the height.
fn drawn(game: &Game<'_>) -> sunrise_client::environment::Sky {
    game.view_environment().sample(&Sample::default())
}

/// Gameplay's sample at the camera.
fn gameplay(game: &Game<'_>) -> sunrise_client::environment::Sky {
    game.dim_environment().sample(&Sample::default())
}

/// The joined level's gameplay and frame environments.
type Built = (
    Arc<sunrise_client::environment::Environment>,
    Option<Arc<sunrise_client::environment::Environment>>,
);

fn built(game: &Game<'_>) -> Built {
    let remote = game.remote.as_ref().unwrap();
    (
        remote.level_environment.clone().unwrap(),
        remote.level_pack_environment.clone(),
    )
}

/// A joined level's environment attributes start from the dimension type its server synced, as
/// `EnvironmentAttributeSystem.addDimensionLayer` takes `level.dimensionType()`'s attributes, and
/// the packs' Polytone modifiers of that dimension apply over them: a data pack's Overworld cloud
/// height reaches the frame on a client whose own resources say 192.33, the Nether and the End
/// take their own pack modifiers and not the Overworld's, and a resource reload rebuilds them.
#[test]
#[ignore = "daily"]
fn a_joined_levels_synced_cloud_height_reaches_the_frame_under_its_own_pack_layers() {
    with_client_game(|game| {
        let _server = join(game, spawn(0, "minecraft:overworld"));
        // The client's own Overworld says 192.33; the server's type says 250.
        assert_eq!(
            game.res
                .environment()
                .sample(&Sample::default())
                .cloud_height,
            192.33
        );
        assert_eq!(drawn(game).cloud_height, 250.0);
        assert_eq!(gameplay(game).cloud_height, 250.0);
        // A parked Nether keeps the client's Nether (sky light level 4), not the joined
        // Overworld's type.
        assert_eq!(game.sky_darken_in(&game.world, Dim::Nether), 11);

        // A server's resource pack arrives: the reload applies its Overworld modifier over the
        // server's type, 250 + 10, while gameplay keeps the type without the packs.
        let pack = CloudPack::new("joined");
        reload(game, &[pack.pack()]);
        assert!(game.pack.environments.is_some());
        assert_eq!(drawn(game).cloud_height, 260.0);
        assert_eq!(gameplay(game).cloud_height, 250.0);
        let (_, overworld_frame) = built(game);

        // The Nether's synchronized type goes over the packs' Nether stack: its own modifier,
        // 300 + 20, and not the Overworld's (300 + 10).
        respawn(game, spawn(1, "minecraft:the_nether"));
        assert_eq!(drawn(game).cloud_height, 320.0);
        assert_eq!(gameplay(game).cloud_height, 300.0);
        // Its base is the client's Nether, not its Overworld: the sky colour is the Nether's.
        let nether = game.res.nether_environment().sample(&Sample::default());
        let overworld_sky = game.res.environment().sample(&Sample::default()).sky_color;
        assert_ne!(nether.sky_color, overworld_sky);
        assert_eq!(drawn(game).sky_color, nether.sky_color);
        assert_eq!(gameplay(game).sky_color, nether.sky_color);
        assert_eq!(game.sky_darken_in(&game.world, Dim::Nether), 11);

        // The End's goes over the packs' End stack: 400 + 30, not 400 + 10.
        respawn(game, spawn(2, "minecraft:the_end"));
        assert_eq!(drawn(game).cloud_height, 430.0);
        assert_eq!(gameplay(game).cloud_height, 400.0);

        // Back in the Overworld, nothing changed: the environment the reload built is drawn
        // again, not built anew.
        respawn(game, spawn(0, "minecraft:overworld"));
        let (overworld_gameplay, frame) = built(game);
        assert!(Arc::ptr_eq(
            frame.as_ref().unwrap(),
            overworld_frame.as_ref().unwrap()
        ));
        assert_eq!(drawn(game).cloud_height, 260.0);

        // The pack is dropped: the reload builds the frame's environment again, the server's
        // 250 without the modifier; gameplay's, which has no pack layers, is kept.
        reload(game, &[]);
        assert!(game.pack.environments.is_none());
        assert_eq!(drawn(game).cloud_height, 250.0);
        let (gameplay_after, frame) = built(game);
        assert!(frame.is_none());
        assert!(Arc::ptr_eq(&gameplay_after, &overworld_gameplay));
    });
}

/// Cloud preparation and atmospheric fog read the synced attributes, not the skybox, while
/// precipitation in a loaded column uses the joined level's sea level from the spawn packet.
#[test]
#[ignore = "daily"]
fn client_clouds_fog_and_precipitation_follow_the_joined_level() {
    use glam::DVec3;
    use serde_json::json;
    use sunrise_client::chunk::Chunk;
    use sunrise_core::position::BlockPos;

    let precipitation_at = |game: &Game<'_>, x, y, z| {
        crate::game::weather::precipitation_at(
            &game.world,
            world_biomes!(game),
            game.streaming.biome_zoom,
            BlockPos::new(x, y, z),
            game.weather_sea_level(),
        )
    };
    crate::game::in_own_process(
        "game::remote::tests::client_level_environment_tests::client_clouds_fog_and_precipitation_follow_the_joined_level",
        || {
            with_client_game(|game| {
                let mut ty = with_height("overworld", 192.33);
                ty["timelines"] = json!([]);
                ty["attributes"] = json!({
                    "minecraft:visual/cloud_color": "#804db5ed",
                    "minecraft:visual/fog_color": "#335580",
                    "minecraft:visual/sky_color": "#2a4099",
                    "minecraft:visual/sun_angle": 90.0,
                    "minecraft:visual/sunrise_sunset_color": "#99df6619",
                    "minecraft:visual/sky_fog_end_distance": 512.0,
                });
                let types = [
                    ("test:overworld", "overworld"),
                    ("test:none", "none"),
                    ("test:end", "end"),
                    ("test:near", "none"),
                ]
                .map(|(name, skybox)| {
                    let mut ty = ty.clone();
                    ty["skybox"] = json!(skybox);
                    if name == "test:near" {
                        ty["attributes"]["minecraft:visual/sky_fog_end_distance"] = json!(64.0);
                    }
                    (name, ty)
                });
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "ClientWeatherTest").unwrap();
                send_types(game, &types);
                game.settings.sunrise.fog = true;
                let camera = DVec3::new(0.0, 64.0, 0.0);
                let mut clouds = Vec::new();
                let mut fog = Vec::new();

                for (index, (dimension, sea_level)) in [
                    ("minecraft:overworld", 0),
                    ("minecraft:the_nether", 63),
                    ("minecraft:the_end", 120),
                    ("test:near", 120),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut entered = spawn(index as i32, dimension);
                    entered.sea_level = sea_level;
                    entered.seed = 42;
                    if index == 0 {
                        login(game, entered);
                    } else {
                        respawn(game, entered);
                    }
                    assert_eq!(game.overworld_skybox(), index == 0);
                    assert_eq!(game.end_skybox(), index == 2);
                    let sky = drawn(game);
                    let mut transparent = sky;
                    transparent.cloud_color[3] = 0.0;
                    game.settings.clouds = "true".into();
                    let enabled = game.frame_cloud_uv(false, &sky).is_some();
                    let zero_alpha = game.frame_cloud_uv(false, &transparent).is_some();
                    game.settings.clouds = "false".into();
                    let off = game.frame_cloud_uv(false, &sky).is_some();
                    clouds.push([enabled, zero_alpha, off]);

                    let mut no_tint = sky;
                    no_tint.sunrise_color[3] = 0.0;
                    let mut sample = |forward, distance, sky, draws_sky| {
                        game.view.forwards = [forward, 0.0, 0.0];
                        game.streaming.radius = distance;
                        game.settings.sunrise.sky = draws_sky;
                        let f = game.frame_fog(camera, sky, 0, [0.0; 3]);
                        let [r, g, b, _] = f.color;
                        ([r, g, b].map(f32::to_bits), f.sky_end)
                    };
                    fog.push([
                        sample(-1.0, 8, &sky, true),
                        sample(-1.0, 8, &sky, false),
                        sample(1.0, 8, &sky, true),
                        sample(-1.0, 8, &no_tint, true),
                        sample(-1.0, 3, &sky, true),
                        sample(-1.0, 3, &no_tint, true),
                    ]);

                    if index < 3 {
                        assert_eq!(game.weather_sea_level(), sea_level);
                        assert_eq!(precipitation_at(game, 16, 100, 8), None);
                        let biomes = world_biomes!(game);
                        let taiga = biomes.id("minecraft:taiga");
                        let desert = biomes.id("minecraft:desert");
                        let layout = game.world.layout();
                        let mut chunk = Chunk::with_layout(0, 0, layout);
                        for qy in 0..(layout.max_y() - layout.min_y()) / 4 {
                            for qz in 0..4 {
                                for qx in 0..4 {
                                    let biome = if index == 0 && qx == 1 { desert } else { taiga };
                                    chunk.set_biome_cell(qx, qy, qz, biome);
                                }
                            }
                        }
                        if index == 0 {
                            // At this boundary 26.3 chooses desert (quart x=1 with seed 42),
                            // not the raw quart's taiga (quart x=0).
                            assert_eq!(chunk.biome(3, 100, 5), taiga);
                        }
                        game.world.chunks.insert((0, 0), Arc::new(chunk));
                        if index == 0 {
                            assert_eq!(precipitation_at(game, 3, 100, 5), None);
                        }

                        // The same biome and two already sampled positions keep their first
                        // temperature; new positions use the newly joined level's sea level.
                        assert_eq!(
                            [100, 170].map(|y| precipitation_at(game, 10, y, 10)),
                            [Some(true), Some(true)],
                            "cached at sea level {sea_level}"
                        );
                        let fresh = [[true, true], [false, true], [false, false]][index];
                        assert_eq!(
                            [100, 170].map(|y| precipitation_at(game, 10 + index as i32, y, 10)),
                            fresh.map(Some),
                            "fresh at sea level {sea_level}"
                        );
                        if index == 2 {
                            // Only this test's child process sees the competing server table.
                            sunrise_world::level::register_sea_level(
                                sunrise_core::LevelKey::parse(dimension).unwrap(),
                                0,
                            );
                            assert_eq!(game.dim.sea_level(), 0);
                            assert_eq!(game.weather_sea_level(), sea_level);
                            assert_eq!(precipitation_at(game, 13, 170, 10), Some(false));

                            let mut same_level = spawn(index as i32, dimension);
                            same_level.sea_level = 0;
                            respawn(game, same_level);
                            assert_eq!(game.weather_sea_level(), sea_level);
                            assert_eq!(precipitation_at(game, 9, 170, 10), Some(false));
                        }
                    }
                }

                assert_eq!(clouds, [[true, false, false]; 4]);
                assert_eq!(fog[0], fog[1], "no skybox");
                assert_eq!(fog[0], fog[2], "End skybox");
                for [toward, sky_off, away, transparent, close, close_transparent] in &fog {
                    assert_eq!(toward, sky_off);
                    assert_ne!(toward.0, away.0);
                    assert_eq!(away, transparent);
                    assert_eq!(close, close_transparent);
                }
                // AtmosphericFogEnvironment.getBaseColor: first blend the synthetic fog
                // toward sunrise at 153/255, then toward the sky by
                // 1 - pow(lerp(min(distance / 16, 8) / 32, 0.25, 1), 0.25).
                assert_eq!(fog[0][0].0, [0x3f05c604, 0x3eb3733e, 0x3ea575a1]);
                assert_eq!(fog[3][0], ([0x3f0069df, 0x3eb07820, 0x3eadc0aa], 64.0));
            })
        },
    );
}

/// A built-in level whose synchronized type is a mod's has no pack stack of its type: it draws
/// the type as the server sent it, not the packs' Overworld.
#[test]
#[ignore = "daily"]
fn an_overworld_of_a_mods_type_draws_the_type_without_the_packs_overworld() {
    with_client_game(|game| {
        let pack = CloudPack::new("mod-type");
        reload(game, &[pack.pack()]);
        assert!(game.pack.environments.is_some());
        let _server = join(game, spawn(3, "minecraft:overworld"));
        assert_eq!(drawn(game).cloud_height, 250.0);
        assert_eq!(gameplay(game).cloud_height, 250.0);
        assert!(built(game).1.is_none());
    });
}

/// Entering a level whose type, JSON and packs are what an earlier entry built from reuses the
/// built environments; a type whose JSON changed (a proxy's next server, through a new
/// configuration and `Login`) is built again from the new JSON.
#[test]
#[ignore = "daily"]
fn an_unchanged_type_reuses_its_environments_and_a_changed_one_is_built_again() {
    with_client_game(|game| {
        let pack = CloudPack::new("reuse");
        reload(game, &[pack.pack()]);
        let _server = join(game, spawn(0, "minecraft:overworld"));
        let (own, frame) = built(game);
        // A death respawn into the same level.
        respawn(game, spawn(0, "minecraft:overworld"));
        let (again_own, again_frame) = built(game);
        assert!(Arc::ptr_eq(&own, &again_own));
        assert!(Arc::ptr_eq(
            frame.as_ref().unwrap(),
            again_frame.as_ref().unwrap()
        ));

        // The next server's Overworld says 270: the same type name with other JSON is built
        // from that JSON, not taken from what 250 built.
        let mut types = server_types();
        types[0].1 = with_height("overworld", 270.0);
        send_types(game, &types);
        login(game, spawn(0, "minecraft:overworld"));
        assert_eq!(drawn(game).cloud_height, 280.0);
        assert_eq!(gameplay(game).cloud_height, 270.0);
        let (changed_own, changed_frame) = built(game);
        assert!(!Arc::ptr_eq(&own, &changed_own));
        assert!(!Arc::ptr_eq(
            frame.as_ref().unwrap(),
            changed_frame.as_ref().unwrap()
        ));
    });
}
