//! Client loading-screen registrations and their presentation data.

use crate::{ClientHost, DrawContext};

type TransitionFactory = dyn Fn(&dyn ClientHost, DimensionTransitionReason) -> Box<dyn DimensionTransitionPresentation>
    + Send
    + Sync
    + 'static;

// no wire: the registration owns a factory with process-local code and captures.
/// A loading-screen factory registered once by `Hooks::register_dimension_transition_screen`.
/// Only travel that opens a new loading screen can select it; login and death never select one.
// exhaustive: private storage keeps the travel key and its per-load factory together.
pub struct DimensionTransitionScreen {
    transition: DimensionTransition,
    factory: Box<TransitionFactory>,
}

impl DimensionTransitionScreen {
    /// Register a factory for `transition` (NeoForge's `RegisterDimensionTransitionScreenEvent`).
    /// On a matching new load, the client calls the factory once as the registering mod and owns
    /// the returned presentation until the loading screen closes, even if the tracker restarts
    /// for another level. Capture per-load state in the returned presentation, not in the
    /// registration itself.
    pub fn new(
        transition: DimensionTransition,
        factory: impl Fn(
            &dyn ClientHost,
            DimensionTransitionReason,
        ) -> Box<dyn DimensionTransitionPresentation>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            transition,
            factory: Box::new(factory),
        }
    }

    /// Which travel selects this factory.
    pub fn transition(&self) -> &DimensionTransition {
        &self.transition
    }

    /// Create the presentation for one new loading screen with its initial reason.
    pub fn create(
        &self,
        host: &dyn ClientHost,
        reason: DimensionTransitionReason,
    ) -> Box<dyn DimensionTransitionPresentation> {
        (self.factory)(host, reason)
    }
}

// no wire: each presentation is a process-local callback or object retained by one load.
/// The complete presentation of one loading screen, either an object or a draw closure.
/// It controls what is painted, not when the client finishes loading.
pub trait DimensionTransitionPresentation: Send + Sync + 'static {
    /// Draw under the registering mod's identity using this frame's reason and optional
    /// smoothed integrated-server progress. The tracker, not the presentation, decides when
    /// the screen closes. The reason can change while the same presentation remains active.
    fn draw(
        &self,
        host: &dyn ClientHost,
        reason: DimensionTransitionReason,
        progress: Option<f32>,
        draw: &mut dyn DrawContext,
    );
}

impl<F> DimensionTransitionPresentation for F
where
    F: Fn(&dyn ClientHost, DimensionTransitionReason, Option<f32>, &mut dyn DrawContext)
        + Send
        + Sync
        + 'static,
{
    fn draw(
        &self,
        host: &dyn ClientHost,
        reason: DimensionTransitionReason,
        progress: Option<f32>,
        draw: &mut dyn DrawContext,
    ) {
        self(host, reason, progress, draw);
    }
}

/// NeoForge's incoming, outgoing and conditional registration variants.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the three registration variants
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DimensionTransition {
    /// Travel to this dimension (`registerIncomingEffect`).
    Incoming(String),
    /// Travel from this dimension (`registerOutgoingEffect`).
    Outgoing(String),
    /// Travel to `to` from `from` (`registerConditionalEffect`), highest priority.
    Conditional {
        /// Destination dimension id.
        to: String,
        /// Source dimension id.
        from: String,
    },
}

/// Why the client is displaying its level-loading screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: LevelLoadingScreen.Reason's three variants
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DimensionTransitionReason {
    /// Travel involving the Nether, except a death respawn.
    NetherPortal,
    /// Travel involving the End, except a death respawn or Nether travel.
    EndPortal,
    /// Login, death respawn, or other travel.
    Other,
}

#[cfg(test)]
mod dimension_transition_tests {
    use crate::*;

    struct Client;
    impl Host for Client {}
    crate::impl_host_areas!(Client);
    impl ClientHost for Client {}
    crate::impl_client_host_areas!(Client);

    #[derive(Default)]
    struct Canvas(Vec<([f32; 4], u32)>);
    impl DrawContext for Canvas {
        fn with_scissor(
            &mut self,
            _: [f32; 4],
            _: &mut dyn FnMut(&mut dyn DrawContext),
        ) -> Result<(), GuiClipRefusal> {
            unreachable!("the transition under test does not open a scissor")
        }
        fn size(&self) -> [f32; 2] {
            [320.0, 240.0]
        }
        fn fill(&mut self, rect: [f32; 4], color: u32) {
            self.0.push((rect, color));
        }
    }

    struct Arrival;
    #[crate::__subscriptions]
    impl Hooks for Arrival {
        fn register_dimension_transition_screen(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterDimensionTransitionScreenEvent<'_>,
        ) {
            event.screens.push(DimensionTransitionScreen::new(
                DimensionTransition::Conditional {
                    to: "test:b".into(),
                    from: "test:a".into(),
                },
                |_, initial_reason| {
                    Box::new(
                        move |_: &dyn ClientHost, reason, progress, draw: &mut dyn DrawContext| {
                            assert_eq!(initial_reason, DimensionTransitionReason::Other);
                            assert_eq!(reason, DimensionTransitionReason::NetherPortal);
                            let [w, h] = draw.size();
                            draw.fill([0.0, 0.0, w, h], 0xFF102030);
                            if let Some(progress) = progress {
                                draw.fill([0.0, 230.0, w * progress, 10.0], 0xFF00FF00);
                            }
                        },
                    )
                },
            ));
        }
    }

    #[test]
    fn dimension_transition_factory_keeps_the_registrant_and_draws_with_live_data() {
        testing::with_dispatch(
            vec![ModEntry::new("test_registrant", Box::new(Arrival))].into(),
            || {
                let mut screens = Vec::new();
                dispatch_register_dimension_transition_screen(
                    &RegistrationContext::new(),
                    &mut screens,
                );
                let [screen] = screens.as_slice() else {
                    panic!("expected one registration");
                };
                assert_eq!(screen.mod_id, "test_registrant");
                assert_eq!(
                    screen.answer.transition(),
                    &DimensionTransition::Conditional {
                        to: "test:b".into(),
                        from: "test:a".into()
                    },
                );
                let presentation = screen
                    .answer
                    .create(&Client, DimensionTransitionReason::Other);
                let mut canvas = Canvas::default();
                presentation.draw(
                    &Client,
                    DimensionTransitionReason::NetherPortal,
                    Some(0.25),
                    &mut canvas,
                );
                assert_eq!(
                    canvas.0,
                    [
                        ([0.0, 0.0, 320.0, 240.0], 0xFF102030),
                        ([0.0, 230.0, 80.0, 10.0], 0xFF00FF00)
                    ]
                );
            },
        );
    }

    #[test]
    fn dimension_transition_without_listeners_collects_nothing() {
        testing::with_dispatch(Vec::new().into(), || {
            let mut screens = Vec::new();
            dispatch_register_dimension_transition_screen(
                &RegistrationContext::new(),
                &mut screens,
            );
            assert!(screens.is_empty());
        });
    }
}
