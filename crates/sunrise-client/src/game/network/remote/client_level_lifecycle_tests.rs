use crate::game::remote::client_has_level;

/// Minecraft.tick: CLIENT_TICK_END only while a level exists. None before
/// the first Login (configuration has finished, the server has not placed
/// the player yet), and none from StartConfiguration until the next Login.
#[test]
fn tick_ends_need_the_level_the_login_makes() {
    assert!(!client_has_level(false, None), "joined, waiting for Login");
    assert!(client_has_level(true, None), "after Login");
    assert!(!client_has_level(true, Some(40)), "reconfiguring");
}

/// #12148: a proxy moves the player to its next server. The last server's
/// columns stay until the next Login, but Reconfigured already put in the
/// next server's biome table, shorter when only the last server had a
/// biome the game lacks: the columns hold an id past it ("the len is 67
/// but the index is 67"). Vanilla's clearClientLevel leaves no player, so
/// getSituationalMusic is MENU, and ServerReconfigScreen (no
/// LevelLoadingScreen) lets MusicManager.tick start it.
#[test]
#[ignore = "daily"]
fn a_reconfiguring_client_plays_menu_music_and_never_indexes_past_the_biomes() {
    use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
    use sunrise_client::{chunk::Chunk, music::Music};
    use sunrise_core::geometry::Vec3;
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
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
    game.init_remote_for_replay().unwrap();
    // The last server's column at the origin: its extra biome came after
    // the built-in ones, one past the table the next server leaves.
    let stale = world_biomes!(game).list.len() as u16;
    let mut column = Chunk::empty(0, 0);
    // Quart 16 above the column's bottom at -64: block y 0.
    column.set_biome_cell(0, 16, 0, stale);
    game.world
        .chunks
        .insert((0, 0), std::sync::Arc::new(column));
    assert_eq!(game.biome_at([0.0; 3]), "minecraft:plains");
    assert_eq!(
        game.biome_name_at(Vec3::new(0.5, 0.5, 0.5)).as_deref(),
        Some("minecraft:plains")
    );

    // StartConfiguration after a Login: no player, ServerReconfigScreen.
    let ticks = game.ticks;
    let r = game.remote.as_mut().unwrap();
    r.logged_in = true;
    r.reconfiguring = Some(ticks);
    game.player = None;
    assert_eq!(game.situational_music().0, Some(Music::menu()));
    // A new manager waits its first 100 ticks, then starts the menu track.
    let menu = (0..=100).any(|_| {
        game.tick_music();
        game.music.current(&game.audio.engine) == Some("minecraft:music.menu")
    });
    assert!(menu, "the menu music starts under ServerReconfigScreen");

    // With a level again the camera probe reads that column: plains' music.
    game.remote.as_mut().unwrap().reconfiguring = None;
    let (music, _) = game.situational_music();
    assert!(music.is_some_and(|m| m != Music::menu()));
}

/// Vanilla ClientPacketListener.handleConfigurationStart -> Minecraft.clearClientLevel ->
/// updateLevelInEngines(null): entering configuration stops sound immediately, so a flat mod
/// audio stream is Stopped at configuration entry without waiting for a new login.
#[test]
#[ignore = "daily"]
fn a_reconfiguring_client_stops_flat_audio_streams_before_configuration() {
    use crate::game::{
        native_entity_refs::tests::with_client_game,
        remote::tests::{connect_play_test, spawn_menu_test},
    };
    use mod_api::{AudioStream, AudioStreamState, SoundSource, StreamSpec};
    use sunrise_client::net::Event;
    use sunrise_protocol::packets::{Empty, play::Clientbound};

    struct Silent;
    impl AudioStream for Silent {
        fn read(&mut self, _pcm: &mut [i16]) -> usize {
            1
        }
    }

    with_client_game(|game| {
        let (_reader, mut writer) = connect_play_test(game, |_, _| {});
        spawn_menu_test(game);
        game.audio.backend_live_for_bench();
        let stream = game
            .audio
            .open_stream(
                "test_mod",
                &StreamSpec::new(SoundSource::Music, 1, 48_000),
                None,
                Box::new(Silent),
            )
            .unwrap();
        assert_eq!(stream.state(), AudioStreamState::Playing);

        writer
            .send(&Clientbound::StartConfiguration(Empty).encode())
            .unwrap();
        game.wait_for_test_remote_event(|event| matches!(event, Event::Reconfiguring(_)));
        game.poll_remote_events(true).unwrap();

        assert!(game.remote.as_ref().unwrap().reconfiguring.is_some());
        assert_eq!(stream.state(), AudioStreamState::Stopped);
    });
}

#[test]
#[ignore = "daily"]
fn client_level_transitions_stop_streams_opened_during_unload() {
    use crate::game::{
        integrated::Integrated, native_client_session::tests::logged_client,
        native_entity_refs::tests::with_client_game, remote::RemoteDimensionEntry,
    };
    use mod_api::{
        AudioStream, AudioStreamRef, AudioStreamState, ClientDisconnectingEvent, ClientHost,
        ClientLevelUnloadEvent, Hooks, ModEntry, SoundSource, StreamSpec,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_client::level_loading::Reason;
    use sunrise_protocol::packets::play::SpawnInfo;

    struct Silent;
    impl AudioStream for Silent {
        fn read(&mut self, _pcm: &mut [i16]) -> usize {
            1
        }
    }

    struct Listener(Arc<Mutex<Vec<AudioStreamRef>>>);
    impl Listener {
        fn open(&self, host: &dyn ClientHost) {
            let stream = host
                .open_audio_stream(
                    &StreamSpec::new(SoundSource::Music, 1, 48_000),
                    Box::new(Silent),
                )
                .unwrap();
            assert_eq!(stream.state(), AudioStreamState::Playing);
            self.0.lock().unwrap().push(stream);
        }
    }

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn client_level_unload(
            &self,
            host: &mut dyn ClientHost,
            _: &mut ClientLevelUnloadEvent<'_>,
        ) {
            self.open(host);
        }

        fn client_disconnecting(&self, host: &dyn ClientHost, _: &mut ClientDisconnectingEvent) {
            self.open(host);
        }
    }

    let streams = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "unload_audio",
        Box::new(Listener(streams.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            game.audio.backend_live_for_bench();
            let spawn = SpawnInfo {
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
            for (replace, integrated) in [(true, false), (false, false), (false, true)] {
                let _server = logged_client(game);
                game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
                let existing = game
                    .audio
                    .open_stream(
                        "test_mod",
                        &StreamSpec::new(SoundSource::Music, 1, 48_000),
                        None,
                        Box::new(Silent),
                    )
                    .unwrap();
                game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Respawn, Reason::Other);
                assert!(
                    streams.lock().unwrap().is_empty(),
                    "same-level respawn does not unload"
                );
                assert_eq!(existing.state(), AudioStreamState::Playing);
                if replace {
                    let mut other = spawn.clone();
                    other.dimension = "minecraft:the_nether".into();
                    game.enter_remote_dimension(
                        &other,
                        RemoteDimensionEntry::Respawn,
                        Reason::Other,
                    );
                } else {
                    if integrated {
                        game.integrated = Some(Integrated::stopped_for_test());
                    }
                    game.leave_server(None).unwrap();
                }
                {
                    let mut heard = streams.lock().unwrap();
                    assert_eq!(heard.len(), if replace { 1 } else { 2 });
                    for stream in heard.drain(..) {
                        assert_eq!(stream.state(), AudioStreamState::Stopped);
                    }
                }
                assert_eq!(existing.state(), AudioStreamState::Stopped);
                if replace {
                    game.leave_server(None).unwrap();
                    streams.lock().unwrap().clear();
                }
            }
        });
    });
}
