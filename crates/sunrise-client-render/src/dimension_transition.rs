//! Load-time dimension-key resolution and selection of client loading presentations.
use mod_api::{
    __private::game::CallbackOwner, Answered, ClientHost, DimensionKey, DimensionTransition,
    DimensionTransitionPresentation, DimensionTransitionReason, DimensionTransitionScreen,
    DrawContext, Identifier,
};
use std::{
    collections::{HashMap, hash_map::Entry},
    fmt,
    sync::OnceLock,
};

/// A mod's presentation instance for the lifetime of one loading screen.
pub struct ActiveTransitionScreen {
    owner: CallbackOwner,
    presentation: Option<Box<dyn DimensionTransitionPresentation>>,
}

impl ActiveTransitionScreen {
    pub fn new(
        owner: CallbackOwner,
        presentation: Box<dyn DimensionTransitionPresentation>,
    ) -> Self {
        Self {
            owner,
            presentation: Some(presentation),
        }
    }

    pub fn draw(
        &self,
        host: &dyn ClientHost,
        reason: DimensionTransitionReason,
        progress: Option<f32>,
        draw: &mut dyn DrawContext,
    ) -> bool {
        self.owner.is_enabled()
            && self
                .owner
                .call(
                    "dimension transition screen",
                    "dimension_transition_screen_draw",
                    || {
                        self.presentation
                            .as_ref()
                            .expect("active presentation")
                            .draw(host, reason, progress, draw);
                    },
                )
                .is_ok()
    }
}

impl Drop for ActiveTransitionScreen {
    fn drop(&mut self) {
        self.owner.dispose(
            "dimension transition screen",
            "dimension_transition_screen_drop",
            self.presentation.take(),
        );
    }
}

impl fmt::Debug for ActiveTransitionScreen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveTransitionScreen")
            .field("owner", &self.owner.id())
            .finish_non_exhaustive()
    }
}

#[derive(PartialEq, Eq, Hash)]
enum TransitionKey {
    Incoming(DimensionKey),
    Outgoing(DimensionKey),
    Conditional {
        to: DimensionKey,
        from: DimensionKey,
    },
}

type Registration = (&'static str, DimensionTransitionScreen);

/// One frozen registration, with separate panic state for the factory and presentations.
pub struct RegisteredTransitionScreen {
    pub factory: DimensionTransitionScreen,
    pub factory_owner: CallbackOwner,
    pub draw_owner: CallbackOwner,
}

/// Registrations resolved once at load; lookups never build dimension names.
#[derive(Default)]
struct TransitionScreens(HashMap<TransitionKey, RegisteredTransitionScreen>);

static SCREENS: OnceLock<TransitionScreens> = OnceLock::new();

impl TransitionScreens {
    fn new(registrations: impl IntoIterator<Item = Registration>) -> Self {
        let mut screens = HashMap::new();
        for (owner, registration) in registrations {
            let key = match registration.transition() {
                DimensionTransition::Incoming(to) => resolve(to).map(TransitionKey::Incoming),
                DimensionTransition::Outgoing(from) => resolve(from).map(TransitionKey::Outgoing),
                DimensionTransition::Conditional { to, from } => resolve(to)
                    .zip(resolve(from))
                    .map(|(to, from)| TransitionKey::Conditional { to, from }),
            };
            match key.map(|key| screens.entry(key)) {
                Some(Entry::Vacant(entry)) => {
                    entry.insert(RegisteredTransitionScreen {
                        factory: registration,
                        factory_owner: CallbackOwner::for_mod(owner),
                        draw_owner: CallbackOwner::for_mod(owner),
                    });
                }
                Some(Entry::Occupied(_)) => log::warn!(
                    "[{owner}] Duplicate dimension transition: {:?}",
                    registration.transition()
                ),
                None => log::warn!(
                    "[{owner}] Invalid dimension transition: {:?}",
                    registration.transition()
                ),
            }
        }
        Self(screens)
    }

    /// Conditional, then incoming, then outgoing, as NeoForge's event javadoc specifies.
    fn select(&self, to: DimensionKey, from: DimensionKey) -> Option<&RegisteredTransitionScreen> {
        self.0
            .get(&TransitionKey::Conditional { to, from })
            .or_else(|| self.0.get(&TransitionKey::Incoming(to)))
            .or_else(|| self.0.get(&TransitionKey::Outgoing(from)))
    }
}

fn resolve(id: &str) -> Option<DimensionKey> {
    Identifier::parse_namespaced(id).map(|_| mod_api::__private::dimension_key(id))
}

/// Publish the client registrations once at startup.
pub fn register(registrations: Vec<Answered<DimensionTransitionScreen>>) {
    let registrations = registrations.into_iter().map(|r| (r.mod_id, r.answer));
    if SCREENS.set(TransitionScreens::new(registrations)).is_err() {
        log::warn!("Dimension transition screens already registered; keeping the first table");
    }
}

/// Choose a factory and its owner once, when a new loading screen opens for travel.
/// Without dimension keys (login or death), the caller does not try to select a factory.
pub fn select(to: DimensionKey, from: DimensionKey) -> Option<&'static RegisteredTransitionScreen> {
    SCREENS.get().and_then(|screens| screens.select(to, from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level_loading::{LevelLoadTracker, PlayerCheck, Reason, ServerLoadView, Stage};
    use mod_api::Host;
    use std::sync::Arc;

    struct Client;
    impl Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);

    #[derive(Default)]
    struct Canvas(Option<u32>);
    impl DrawContext for Canvas {
        fn fill(&mut self, _: [f32; 4], argb: u32) {
            self.0 = Some(argb);
        }
    }

    fn key(id: &str) -> DimensionKey {
        mod_api::__private::dimension_key(id)
    }
    fn entry(transition: DimensionTransition, argb: u32) -> Registration {
        (
            "registrant",
            DimensionTransitionScreen::new(transition, move |_, _| {
                Box::new(
                    move |_: &dyn ClientHost, _, _, draw: &mut dyn DrawContext| {
                        draw.fill([0.0, 0.0, 1.0, 1.0], argb);
                    },
                )
            }),
        )
    }
    fn chosen(screens: &TransitionScreens, to: &str, from: &str) -> Option<u32> {
        let registration = screens.select(key(to), key(from))?;
        assert_eq!(registration.factory_owner.id(), "registrant");
        let mut canvas = Canvas::default();
        registration
            .factory
            .create(&Client, Reason::Other.into())
            .draw(&Client, Reason::Other.into(), None, &mut canvas);
        canvas.0
    }

    #[test]
    fn conditional_wins_over_incoming_then_outgoing_and_first_registration_wins() {
        let screens = TransitionScreens::new(vec![
            entry(DimensionTransition::Outgoing("test:a".into()), 1),
            entry(DimensionTransition::Incoming("test:b".into()), 2),
            entry(
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "test:a".into(),
                },
                3,
            ),
            entry(
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "test:a".into(),
                },
                4,
            ),
            entry(DimensionTransition::Incoming("test:b".into()), 5),
            entry(DimensionTransition::Outgoing("test:a".into()), 6),
        ]);
        assert_eq!(chosen(&screens, "test:b", "test:a"), Some(3));
        assert_eq!(chosen(&screens, "test:b", "test:c"), Some(2));
        assert_eq!(chosen(&screens, "test:c", "test:a"), Some(1));
        assert_eq!(chosen(&screens, "test:c", "test:b"), None);
    }

    #[test]
    fn invalid_ids_do_not_reserve_a_transition_or_replace_vanilla() {
        let screens = TransitionScreens::new(vec![
            entry(DimensionTransition::Incoming("INVALID:b".into()), 1),
            entry(DimensionTransition::Outgoing("missing_namespace".into()), 2),
            entry(
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "INVALID:a".into(),
                },
                3,
            ),
            entry(DimensionTransition::Incoming("test:b".into()), 4),
            entry(
                DimensionTransition::Incoming("minecraft:the_nether".into()),
                5,
            ),
        ]);
        assert_eq!(chosen(&screens, "test:b", "test:a"), Some(4));
        assert_eq!(chosen(&screens, "minecraft:the_nether", "test:a"), Some(5));
        assert_eq!(chosen(&screens, "INVALID:b", "test:a"), None);
        assert_eq!(chosen(&screens, "test:c", "missing_namespace"), None);
        assert_eq!(chosen(&screens, "test:b", "INVALID:a"), Some(4));
    }

    #[test]
    fn selected_screen_follows_existing_progress_readiness_and_cleanup_then_falls_back() {
        let screens = TransitionScreens::new([entry(
            DimensionTransition::Conditional {
                to: "test:b".into(),
                from: "test:a".into(),
            },
            0xFF102030,
        )]);
        let player = PlayerCheck {
            outside_build_height: false,
            spectator: false,
            alive: true,
        };
        let mut tracker = LevelLoadTracker::new(500);
        tracker.start_client_load(0, Reason::Other);
        let registration = screens.select(key("test:b"), key("test:a")).unwrap();
        tracker.set_transition_screen(ActiveTransitionScreen::new(
            registration.draw_owner.clone(),
            registration.factory.create(&Client, tracker.reason.into()),
        ));
        let progress = ServerLoadView::default();
        progress.start(Stage::LoadInitialChunks, 0);
        progress.finish(Stage::LoadInitialChunks);
        progress.start(Stage::LoadPlayerChunks, 49);
        progress.update(Stage::LoadPlayerChunks, 20, 49);
        assert_eq!(progress.server_progress(), 30.0 / 59.0);
        let mut canvas = Canvas::default();
        assert!(tracker.transition_screen().unwrap().draw(
            &Client,
            tracker.reason.into(),
            Some(progress.server_progress()),
            &mut canvas,
        ));
        assert_eq!(canvas.0, Some(0xFF102030));
        tracker.tick_client_load(100, player);
        assert!(!tracker.is_level_ready(100));
        tracker.loading_packets_received();
        tracker.tick_client_load(150, player);
        assert!(!tracker.is_level_ready(150));
        tracker.player_section_ready();
        tracker.tick_client_load(200, player);
        assert!(!tracker.is_level_ready(699));
        assert!(tracker.is_level_ready(700));
        assert!(tracker.transition_screen().is_some());
        tracker.finish();
        assert!(tracker.transition_screen().is_none());
        assert!(!tracker.is_active());
        tracker.start_client_load(1000, Reason::Other);
        assert!(screens.select(key("test:c"), key("test:b")).is_none());
        assert!(tracker.transition_screen().is_none());
        tracker.loading_packets_received();
        tracker.player_section_ready();
        tracker.tick_client_load(1100, player);
        assert!(!tracker.is_level_ready(1599));
        assert!(tracker.is_level_ready(1600));
    }

    #[test]
    fn retiring_a_panicking_presentation_cannot_abort_load_completion_or_disconnect() {
        struct BrokenDrop;
        impl DimensionTransitionPresentation for BrokenDrop {
            fn draw(
                &self,
                _: &dyn ClientHost,
                _: DimensionTransitionReason,
                _: Option<f32>,
                _: &mut dyn DrawContext,
            ) {
            }
        }
        impl Drop for BrokenDrop {
            fn drop(&mut self) {
                assert_eq!(
                    mod_api::content::crash::current_native_mod(),
                    Some("broken_presentation")
                );
                panic!("presentation destructor failed");
            }
        }

        mod_api::testing::with_dispatch(Vec::new().into(), || {
            let screens = TransitionScreens::new([(
                "broken_presentation",
                DimensionTransitionScreen::new(
                    DimensionTransition::Incoming("test:b".into()),
                    |_, _| Box::new(BrokenDrop),
                ),
            )]);
            let registration = screens.select(key("test:b"), key("test:a")).unwrap();
            for disconnect in [false, true] {
                let mut tracker = LevelLoadTracker::new(0);
                tracker.start_client_load(0, Reason::Other);
                let presentation = registration
                    .factory_owner
                    .call(
                        "dimension transition screen",
                        "dimension_transition_screen_factory",
                        || registration.factory.create(&Client, tracker.reason.into()),
                    )
                    .unwrap();
                tracker.set_transition_screen(ActiveTransitionScreen::new(
                    registration.draw_owner.clone(),
                    presentation,
                ));
                assert_eq!(
                    tracker.transition_screen().unwrap().draw(
                        &Client,
                        tracker.reason.into(),
                        None,
                        &mut Canvas::default(),
                    ),
                    !disconnect
                );
                if disconnect {
                    // A later load still runs the factory, but drawing is already disabled.
                    // Dropping the tracker, as disconnect does, still retires the instance.
                    drop(tracker);
                } else {
                    tracker.loading_packets_received();
                    tracker.player_section_ready();
                    tracker.tick_client_load(
                        1,
                        PlayerCheck {
                            outside_build_height: false,
                            spectator: false,
                            alive: true,
                        },
                    );
                    assert!(tracker.is_level_ready(1));
                    tracker.finish();
                    assert!(!tracker.is_active());
                    assert!(tracker.transition_screen().is_none());
                }
                let notices = mod_api::__private::take_hook_panics();
                let relevant: Vec<_> = notices
                    .iter()
                    .filter(|notice| notice.mod_id == "broken_presentation")
                    .map(|notice| notice.hook)
                    .collect();
                assert_eq!(relevant, ["dimension_transition_screen_drop"]);
            }
        });
    }

    #[test]
    fn restart_keeps_the_presentation_until_abort_or_timeout_finish() {
        for time_out in [false, true] {
            let mut tracker = LevelLoadTracker::new(0);
            tracker.start_client_load(0, Reason::NetherPortal);
            let token = Arc::new(());
            let lifetime = Arc::downgrade(&token);
            tracker.set_transition_screen(ActiveTransitionScreen::new(
                CallbackOwner::for_mod("registrant"),
                Box::new(move |_: &dyn ClientHost, _, _, _: &mut dyn DrawContext| {
                    assert_eq!(Arc::strong_count(&token), 1);
                }),
            ));
            tracker.start_client_load(100, Reason::EndPortal);
            assert_eq!(tracker.reason, Reason::EndPortal);
            assert!(lifetime.upgrade().is_some());
            if time_out {
                let player = PlayerCheck {
                    outside_build_height: false,
                    spectator: false,
                    alive: true,
                };
                tracker.loading_packets_received();
                tracker.tick_client_load(30_100, player);
                assert!(!tracker.is_level_ready(30_100));
                tracker.tick_client_load(30_101, player);
                assert!(tracker.is_level_ready(30_101));
                assert!(lifetime.upgrade().is_some());
            }
            tracker.finish();
            assert!(tracker.transition_screen().is_none());
            assert!(lifetime.upgrade().is_none());
        }
    }
}
