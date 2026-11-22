//! The global cancel dispatcher's per-listener output check for `living_travel`.

/// What a fixture listener heard of one travel: the input and velocity it was handed.
type TravelHeard = Vec<([f64; 3], [f64; 3], bool)>;

/// A listener that records what it hears, then writes `input` and `velocity`.
struct Setter {
    input: [f64; 3],
    velocity: [f64; 3],
    cancel: Cancel,
    heard: Arc<Mutex<TravelHeard>>,
}

#[derive(Clone, Copy)]
enum Cancel {
    None,
    Flag,
    Flow,
}

#[mod_api::__subscriptions]
impl Hooks for Setter {
    fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
        self.heard
            .lock()
            .unwrap()
            .push((*event.input, *event.velocity, event.cancelled));
        *event.input = self.input;
        *event.velocity = self.velocity;
        match self.cancel {
            Cancel::None => Flow::Continue,
            Cancel::Flag => {
                event.cancelled = true;
                Flow::Continue
            }
            Cancel::Flow => Flow::Cancel,
        }
    }
}

fn setter(
    id: &'static str,
    input: [f64; 3],
    velocity: [f64; 3],
    cancel: Cancel,
) -> (Arc<Mutex<TravelHeard>>, ModEntry) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let entry = ModEntry::new(
        id,
        Box::new(Setter {
            input,
            velocity,
            cancel,
            heard: Arc::clone(&heard),
        }),
    );
    (heard, entry)
}

fn dispatch_travel(mut input: [f64; 3], mut velocity: [f64; 3]) -> ([f64; 3], [f64; 3], bool) {
    let mut canceled = false;
    mod_api::dispatch_living_travel(
        &Loop("server"),
        7,
        "minecraft:zombie",
        TravelMode::Air,
        &mut input,
        &mut velocity,
        &mut canceled,
    );
    (input, velocity, canceled)
}

#[test]
fn a_refused_pair_rolls_back_its_cancel_and_later_listeners_hear_the_accepted_result() {
    let input = [0.5, 0.0, 0.0];
    let velocity = [3.0, 0.0, 0.0];
    for cancel in [Cancel::Flag, Cancel::Flow] {
        let (_, accepted) = setter("accepted", input, velocity, Cancel::None);
        let (refused_heard, refused) = setter("refused", [2.0, 0.0, 0.0], [7.0, 0.0, 0.0], cancel);
        let (later_heard, later) = setter("later", input, velocity, Cancel::None);
        install(vec![accepted, refused, later].into());
        let refusals = capture_output_refusals(|| {
            for _ in 0..2 {
                assert_eq!(
                    dispatch_travel([0.1, 0.0, 0.0], [1.0, 0.0, 0.0]),
                    (input, velocity, false),
                    "the accepted result stands; the refused one, cancellation included, is gone"
                );
            }
        });
        assert_eq!(refusals, [("refused", "living_travel", "input"); 2]);
        assert!(!runtime::poisoned(HookId::LivingTravel, "refused"));
        assert_eq!(
            *refused_heard.lock().unwrap(),
            [(input, velocity, false); 2]
        );
        assert_eq!(*later_heard.lock().unwrap(), [(input, velocity, false); 2]);
    }
}

#[test]
fn a_listener_receiving_a_cancellation_cannot_clear_it() {
    struct Clear(Arc<Mutex<TravelHeard>>);
    #[mod_api::__subscriptions]
    impl Hooks for Clear {
        #[listener(receive_cancelled)]
        fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
            self.0
                .lock()
                .unwrap()
                .push((*event.input, *event.velocity, event.cancelled));
            event.cancelled = false;
            Flow::Continue
        }
    }

    let input = [0.5, 0.0, 0.0];
    let velocity = [3.0, 0.0, 0.0];
    let (_, first) = setter("first", input, velocity, Cancel::Flow);
    let heard = Arc::new(Mutex::new(Vec::new()));
    let clear = ModEntry::new("clear", Box::new(Clear(Arc::clone(&heard))));
    let (skipped_heard, skipped) = setter("skipped", [0.0; 3], [0.0; 3], Cancel::None);
    install(vec![first, clear, skipped].into());
    assert_eq!(dispatch_travel([0.0; 3], [0.0; 3]), (input, velocity, true));
    assert_eq!(*heard.lock().unwrap(), vec![(input, velocity, true)]);
    assert!(skipped_heard.lock().unwrap().is_empty());
}

#[test]
fn a_no_op_listener_does_not_propose_the_games_out_of_budget_velocity() {
    struct Observe;
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
            assert_eq!(*event.velocity, [65.0, 0.0, 0.0]);
            Flow::Continue
        }
    }

    install(vec![ModEntry::new("observe", Box::new(Observe))].into());
    let refusals = capture_output_refusals(|| {
        assert_eq!(
            dispatch_travel([0.0; 3], [65.0, 0.0, 0.0]),
            ([0.0; 3], [65.0, 0.0, 0.0], false)
        );
    });
    assert!(refusals.is_empty());
}

mod crate_root {
    mod entity {
        pub(crate) mod types {
            pub(crate) fn refused_move_output(
                accepted: &[f64; 3],
                delta: &[f64; 3],
            ) -> Result<(), &'static str> {
                if accepted.map(f64::to_bits) != delta.map(f64::to_bits)
                    && delta.iter().any(|v| !v.is_finite())
                {
                    Err("delta")
                } else {
                    Ok(())
                }
            }

            /// The fixture's validator: input.x past 1.0 or velocity.x past 64.0 is
            /// refused only on replacement. The real budgets are tested in the vocabulary crate.
            pub(crate) fn refused_travel_output(
                accepted_input: &[f64; 3],
                accepted_velocity: &[f64; 3],
                input: &[f64; 3],
                velocity: &[f64; 3],
            ) -> Result<(), &'static str> {
                if accepted_input[0].to_bits() != input[0].to_bits() && input[0] > 1.0 {
                    Err("input")
                } else if accepted_velocity[0].to_bits() != velocity[0].to_bits()
                    && velocity[0] > 64.0
                {
                    Err("velocity")
                } else {
                    Ok(())
                }
            }
        }
    }
}
