#[test]
#[ignore = "daily"]
fn dimension_transition_screen_is_selected_at_entry_drawn_and_dropped() {
    use crate::game::Screen;
    use crate::game::native_entity_refs::tests::with_client_game;
    use crate::game::remote::RemoteDimensionEntry;
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::{
        ClientHost, DimensionTransition, DimensionTransitionReason as ApiReason,
        DimensionTransitionScreen, DrawContext, content::crash::current_native_mod,
    };
    use std::sync::Mutex;
    use sunrise_client::{hud::HudBatch, level_loading::Reason};
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    const OWNER: &str = "transition_test";
    const A: &str = "transition_test:a";
    const B: &str = "transition_test:b";
    const END: &str = "minecraft:the_end";

    #[derive(Debug, PartialEq)]
    enum Call {
        Create(&'static str, ApiReason),
        Draw(ApiReason, ApiReason, Option<f32>),
    }
    static CALLS: Mutex<Vec<Call>> = Mutex::new(Vec::new());

    struct Arrival;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Arrival {
        fn register_dimension_transition_screen(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDimensionTransitionScreenEvent<'_>,
        ) {
            for (id, transition) in [
                ("marker:login", DimensionTransition::Incoming(A.into())),
                ("marker:death", DimensionTransition::Incoming(B.into())),
                ("marker:end", DimensionTransition::Incoming(END.into())),
                (
                    "marker:travel",
                    DimensionTransition::Conditional {
                        to: B.into(),
                        from: A.into(),
                    },
                ),
            ] {
                event.screens.push(DimensionTransitionScreen::new(
                    transition,
                    move |_, initial_reason| {
                        assert_eq!(current_native_mod(), Some(OWNER));
                        CALLS.lock().unwrap().push(Call::Create(id, initial_reason));
                        Box::new(
                            move |_: &dyn ClientHost,
                                  reason,
                                  progress,
                                  draw: &mut dyn DrawContext| {
                                assert_eq!(current_native_mod(), Some(OWNER));
                                CALLS.lock().unwrap().push(Call::Draw(
                                    initial_reason,
                                    reason,
                                    progress,
                                ));
                                draw.fill([0.0, 0.0, 80.0, 10.0], 0xFF102030);
                            },
                        )
                    },
                ));
            }
        }
    }
    crate::game::in_own_process(
        "game::remote::tests::dimension_transition_tests::dimension_transition_screen_is_selected_at_entry_drawn_and_dropped",
        || {
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(OWNER, Box::new(Arrival))].into(),
                || {
                    let mut registrations = Vec::new();
                    super::Game::native_register_dimension_transition_screen(
                        &mod_api::RegistrationContext::new(),
                        &mut registrations,
                    );
                    sunrise_client_render::dimension_transition::register(registrations);
                    with_client_game(|game| {
                        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                        game.connect_local(client, "TransitionTest").unwrap();
                        let spawn = |dimension: &str| play::SpawnInfo {
                            dimension_type: 0,
                            dimension: dimension.into(),
                            seed: 0,
                            game_type: 0,
                            previous_game_type: None,
                            debug: false,
                            flat: true,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        };
                        let respawn = |dimension, keep| {
                            C::Respawn(play::Respawn {
                                spawn: spawn(dimension),
                                keep,
                            })
                        };
                        game.enter_remote_dimension(
                            &spawn(A),
                            RemoteDimensionEntry::Login,
                            Reason::Other,
                        );
                        let tracker = &mut game.remote.as_mut().unwrap().level_load;
                        assert!(
                            tracker.transition_screen().is_none(),
                            "login skips incoming A"
                        );
                        tracker.finish();

                        game.set_screen(Screen::Dead);
                        game.handle_packet(respawn(B, 0)).unwrap();
                        let tracker = &game.remote.as_ref().unwrap().level_load;
                        assert_eq!(tracker.reason, Reason::Other);
                        assert!(
                            tracker.transition_screen().is_none(),
                            "death skips incoming B and conditional A to B"
                        );

                        game.handle_packet(respawn(A, 3)).unwrap();
                        let tracker = &mut game.remote.as_mut().unwrap().level_load;
                        assert!(
                            tracker.transition_screen().is_none(),
                            "an open vanilla screen also stays in place"
                        );
                        assert!(CALLS.lock().unwrap().is_empty());
                        tracker.finish();

                        game.handle_packet(respawn(B, 3)).unwrap();
                        let tracker = &game.remote.as_ref().unwrap().level_load;
                        let mut batch = HudBatch::scratch([320, 240]);
                        assert!(game.draw_dimension_transition_screen(
                            &mut batch,
                            tracker,
                            Some(0.25)
                        ));
                        assert!(!batch.vertices.is_empty());

                        game.handle_packet(respawn(END, 3)).unwrap();
                        let tracker = &game.remote.as_ref().unwrap().level_load;
                        let mut updated = HudBatch::scratch([320, 240]);
                        assert!(game.draw_dimension_transition_screen(
                            &mut updated,
                            tracker,
                            Some(0.75)
                        ));
                        assert!(!updated.vertices.is_empty());
                        assert_eq!(
                            *CALLS.lock().unwrap(),
                            [
                                Call::Create("marker:travel", ApiReason::Other),
                                Call::Draw(ApiReason::Other, ApiReason::Other, Some(0.25)),
                                Call::Draw(ApiReason::Other, ApiReason::EndPortal, Some(0.75)),
                            ]
                        );

                        game.remote.as_mut().unwrap().level_load.finish();
                        let tracker = &game.remote.as_ref().unwrap().level_load;
                        assert!(!game.draw_dimension_transition_screen(
                            &mut updated,
                            tracker,
                            None
                        ));
                        game.handle_packet(respawn("minecraft:overworld", 3))
                            .unwrap();
                        let tracker = &game.remote.as_ref().unwrap().level_load;
                        let mut vanilla = HudBatch::scratch([320, 240]);
                        assert!(!game.draw_dimension_transition_screen(
                            &mut vanilla,
                            tracker,
                            None
                        ));
                        game.draw_level_loading_background(&mut vanilla, tracker.reason);
                        assert!(!vanilla.vertices.is_empty());
                    });
                },
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_failed_presentation_discards_its_draws_without_disabling_its_factory() {
    use crate::game::native_entity_refs::tests::with_client_game;
    use crate::game::remote::RemoteDimensionEntry;
    use mod_api::{
        __private::game::RegistrationContextNew, CameraType, ClientHost, DimensionTransition,
        DimensionTransitionScreen, DrawContext, SoundPlay, SoundRef, SoundState,
    };
    use std::sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_client::{
        hud::HudBatch,
        level_loading::{self, Reason},
    };
    use sunrise_protocol::packets::play;

    const NETHER: &str = "minecraft:the_nether";
    static CREATES: AtomicUsize = AtomicUsize::new(0);
    static DRAWS: AtomicUsize = AtomicUsize::new(0);
    static FIRST_SOUND: OnceLock<SoundRef> = OnceLock::new();

    struct Arrival;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Arrival {
        fn register_dimension_transition_screen(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterDimensionTransitionScreenEvent<'_>,
        ) {
            event.screens.push(DimensionTransitionScreen::new(
                DimensionTransition::Incoming(NETHER.into()),
                |_, _| {
                    CREATES.fetch_add(1, Ordering::Relaxed);
                    Box::new(|host: &dyn ClientHost, _, _, draw: &mut dyn DrawContext| {
                        draw.fill([0.0, 0.0, 80.0, 10.0], 0xFFFF_0000);
                        host.apply_screen_blur();
                        assert!(host.clear_particles());
                        host.set_camera_type(CameraType::ThirdPersonFront);
                        let frame = DRAWS.fetch_add(1, Ordering::Relaxed);
                        if frame == 0 {
                            let sound = host
                                .play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0)
                                .unwrap();
                            assert!(FIRST_SOUND.set(sound).is_ok());
                            return;
                        }
                        assert!(FIRST_SOUND.get().unwrap().stop(host));
                        assert!(
                            host.play_local_sound(
                                &SoundPlay::ui("minecraft:entity.player.levelup"),
                                0
                            )
                            .is_some()
                        );
                        panic!("test draw panic");
                    })
                },
            ));
        }
    }

    crate::game::in_own_process(
        "game::remote::tests::dimension_transition_tests::a_failed_presentation_discards_its_draws_without_disabling_its_factory",
        || {
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "failed_transition",
                    Box::new(Arrival),
                )]
                .into(),
                || {
                    let mut registrations = Vec::new();
                    super::Game::native_register_dimension_transition_screen(
                        &mod_api::RegistrationContext::new(),
                        &mut registrations,
                    );
                    sunrise_client_render::dimension_transition::register(registrations);
                    with_client_game(|game| {
                        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                        game.connect_local(client, "TransitionTest").unwrap();
                        let spawn = |dimension: &str| play::SpawnInfo {
                            dimension_type: 0,
                            dimension: dimension.into(),
                            seed: 0,
                            game_type: 0,
                            previous_game_type: None,
                            debug: false,
                            flat: true,
                            last_death: None,
                            portal_cooldown: 0,
                            sea_level: 63,
                        };
                        let overworld = spawn("minecraft:overworld");
                        let nether = spawn(NETHER);
                        game.settings.menu_blur = 5;
                        game.enter_remote_dimension(
                            &overworld,
                            RemoteDimensionEntry::Login,
                            Reason::Other,
                        );
                        for trip in [1, 2] {
                            game.remote.as_mut().unwrap().level_load.finish();
                            game.enter_remote_dimension(
                                &nether,
                                RemoteDimensionEntry::Respawn,
                                Reason::NetherPortal,
                            );
                            assert_eq!(CREATES.load(Ordering::Relaxed), trip);

                            let mut vanilla = HudBatch::scratch([320, 240]);
                            game.draw_level_loading_background(&mut vanilla, Reason::NetherPortal);
                            level_loading::draw(&mut vanilla, &game.gui.atlas, None, None);
                            assert!(!vanilla.vertices.is_empty());
                            if trip == 1 {
                                assert_eq!(game.menu_blur(), None);
                                let mut frame = HudBatch::scratch([320, 240]);
                                assert!(game.remote_overlay(&mut frame));
                                assert_eq!(game.menu_blur(), Some(5.0));
                                assert!(game.mod_clear_particles.get());
                                assert_eq!(
                                    game.requested_camera_type.get(),
                                    Some(sunrise_client::camera::CameraType::ThirdPersonFront)
                                );
                            }
                            let sound = FIRST_SOUND.get().unwrap();
                            // Keep requests already pending on entry, including the sound from
                            // the successful draw. Discard the failed draw's requests only.
                            for incoming in [false, true, false] {
                                let camera = incoming
                                    .then_some(sunrise_client::camera::CameraType::ThirdPersonBack);
                                game.screen_blur_requested.set(incoming);
                                game.mod_clear_particles.set(incoming);
                                game.requested_camera_type.set(camera);
                                let mut frame = HudBatch::scratch([320, 240]);
                                assert!(game.remote_overlay(&mut frame));
                                assert_eq!(
                                    bytemuck::cast_slice::<_, u8>(&frame.vertices),
                                    bytemuck::cast_slice::<_, u8>(&vanilla.vertices)
                                );
                                assert_eq!(frame.blends, vanilla.blends);
                                assert_eq!(game.menu_blur(), incoming.then_some(5.0));
                                assert_eq!(game.mod_clear_particles.get(), incoming);
                                assert_eq!(game.requested_camera_type.get(), camera);
                                let pending = game.mod_sounds.queued.borrow();
                                if trip == 1 {
                                    assert_eq!(pending.len(), 1);
                                    assert_eq!(
                                        pending[0].1.identifier,
                                        "minecraft:ui.button.click"
                                    );
                                    assert_eq!(sound.state(game), Some(SoundState::Waiting));
                                } else {
                                    assert!(pending.is_empty());
                                }
                            }
                            if trip == 1 {
                                game.audio.heard.clear();
                                game.tick_sound_engine();
                                assert_eq!(
                                    game.audio.heard.get("minecraft:ui.button.click"),
                                    Some(&1)
                                );
                                assert!(
                                    !game
                                        .audio
                                        .heard
                                        .contains_key("minecraft:entity.player.levelup")
                                );
                            }
                            assert_eq!(DRAWS.load(Ordering::Relaxed), 2);
                            game.remote.as_mut().unwrap().level_load.finish();
                            game.enter_remote_dimension(
                                &overworld,
                                RemoteDimensionEntry::Respawn,
                                Reason::NetherPortal,
                            );
                        }
                    });
                },
            );
        },
    );
}
