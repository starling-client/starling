//! The `living_travel` fixture: shared cancel dispatch passes input, velocity
//! and cancellation through independently without synthetic caps or a rollback
//! validator, and `#[listener(receive_cancelled)]` listeners may clear cancellation.

/// What a fixture listener heard of one travel: the input, velocity and cancel flag.
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
fn edits_above_former_synthetic_maxima_and_cancellation_pass_through() {
    let input = [2.0, 0.0, 0.0];
    let velocity = [80.0, 0.0, 0.0];
    let (_, first) = setter("first", input, velocity, Cancel::None);
    let (later_heard, later) = setter("later", [3.0, 0.0, 0.0], [90.0, 0.0, 0.0], Cancel::None);
    install(vec![first, later].into());
    assert_eq!(
        dispatch_travel([0.1, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ([3.0, 0.0, 0.0], [90.0, 0.0, 0.0], false)
    );
    assert_eq!(*later_heard.lock().unwrap(), vec![(input, velocity, false)]);

    for cancel in [Cancel::Flag, Cancel::Flow] {
        let (_, canceller) = setter("canceller", [f64::NAN, 0.0, 0.0], [90.0, 0.0, 0.0], cancel);
        let (skipped_heard, skipped) = setter("skipped", [0.0; 3], [0.0; 3], Cancel::None);
        install(vec![canceller, skipped].into());
        let (out_input, out_velocity, canceled) = dispatch_travel([0.1, 0.0, 0.0], [1.0, 0.0, 0.0]);
        assert!(canceled, "cancellation is independent of output values");
        assert!(out_input[0].is_nan());
        assert_eq!(out_velocity, [90.0, 0.0, 0.0]);
        assert!(skipped_heard.lock().unwrap().is_empty());
    }
}

#[test]
fn a_listener_receiving_a_cancellation_may_clear_it() {
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

    let input = [2.0, 0.0, 0.0];
    let velocity = [70.0, 0.0, 0.0];
    let (_, first) = setter("first", input, velocity, Cancel::Flow);
    let heard = Arc::new(Mutex::new(Vec::new()));
    let clear = ModEntry::new("clear", Box::new(Clear(Arc::clone(&heard))));
    let (after_heard, after) = setter("after", [0.5, 0.0, 0.0], [3.0, 0.0, 0.0], Cancel::None);
    install(vec![first, clear, after].into());
    assert_eq!(
        dispatch_travel([0.0; 3], [0.0; 3]),
        ([0.5, 0.0, 0.0], [3.0, 0.0, 0.0], false)
    );
    assert_eq!(*heard.lock().unwrap(), vec![(input, velocity, true)]);
    assert_eq!(*after_heard.lock().unwrap(), vec![(input, velocity, false)]);
}

#[test]
fn a_no_op_listener_leaves_the_games_velocity_unchanged() {
    struct Observe;
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
            assert_eq!(*event.velocity, [65.0, 0.0, 0.0]);
            Flow::Continue
        }
    }

    install(vec![ModEntry::new("observe", Box::new(Observe))].into());
    assert_eq!(
        dispatch_travel([0.0; 3], [65.0, 0.0, 0.0]),
        ([0.0; 3], [65.0, 0.0, 0.0], false)
    );
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
            pub(crate) fn refused_velocity_output(
                accepted: &[f64; 3],
                velocity: &[f64; 3],
            ) -> Result<(), &'static str> {
                if accepted.map(f64::to_bits) != velocity.map(f64::to_bits)
                    && velocity.iter().any(|v| !v.is_finite())
                {
                    Err("velocity")
                } else {
                    Ok(())
                }
            }
        }
    }
}
