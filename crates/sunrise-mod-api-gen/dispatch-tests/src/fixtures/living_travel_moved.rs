//! The `living_travel_moved` fixture: a collect dispatch whose listeners chain the velocity, where a
//! non-finite proposal is dropped for that listener alone and later listeners hear the last
//! accepted velocity.

/// What a fixture listener heard of one moved travel: the velocity it was given.
type MovedHeard = Vec<[f64; 3]>;

/// A listener that records the velocity it hears, then proposes `velocity`.
struct Proposer {
    velocity: [f64; 3],
    heard: Arc<Mutex<MovedHeard>>,
}

#[mod_api::__subscriptions]
impl Hooks for Proposer {
    fn living_travel_moved(&self, _: &dyn LoopContext, event: &mut LivingTravelMovedEvent<'_>) {
        self.heard.lock().unwrap().push(*event.velocity);
        *event.velocity = self.velocity;
    }
}

fn proposer(id: &'static str, velocity: [f64; 3]) -> (Arc<Mutex<MovedHeard>>, ModEntry) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let entry = ModEntry::new(
        id,
        Box::new(Proposer {
            velocity,
            heard: Arc::clone(&heard),
        }),
    );
    (heard, entry)
}

fn dispatch_moved(mut velocity: [f64; 3]) -> [f64; 3] {
    mod_api::dispatch_living_travel_moved(
        &Loop("server"),
        7,
        "minecraft:zombie",
        TravelMode::Lava,
        [0.0, -0.1, 0.0],
        false,
        true,
        false,
        &mut velocity,
    );
    velocity
}

#[test]
fn finite_proposals_chain_and_a_non_finite_one_is_dropped_for_its_listener_alone() {
    let (_, first) = proposer("first", [3.0, 4.0, 5.0]);
    let (refused_heard, refused) = proposer("refused", [f64::NAN, 0.0, 0.0]);
    let (later_heard, later) = proposer("later", [9.0, 0.0, 0.0]);
    install(vec![first, refused, later].into());
    assert_eq!(dispatch_moved([1.0, 0.0, 0.0]), [9.0, 0.0, 0.0]);
    assert_eq!(*refused_heard.lock().unwrap(), vec![[3.0, 4.0, 5.0]]);
    assert_eq!(
        *later_heard.lock().unwrap(),
        vec![[3.0, 4.0, 5.0]],
        "a later listener hears the last accepted velocity, not the refused one"
    );
}

#[test]
fn no_listener_leaves_the_velocity_unchanged() {
    install(Vec::<ModEntry>::new().into());
    assert_eq!(dispatch_moved([1.0, -2.0, 3.0]), [1.0, -2.0, 3.0]);
}
