//! The `general_entity_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The authoritative tick of one general entity row of type `kind`
    /// (`mymod:robot`) in the server level's entity phase:
    /// `ServerLevel.tickNonPassenger`'s `entity.tick()` for a row the type
    /// registry owns (#792 stage 4). The row's `Entity.commonTick` and
    /// `Entity.baseTick` have run first: its tick count went up, its previous
    /// position is its current one, its boarding cooldown counted down, and
    /// `updateFluidInteraction` refreshed its fluids (the eye is `0.85` of the
    /// type's height above its feet), pushed its velocity by a current
    /// (`isPushedByFluid`; a riding row's velocity is zeroed first, as
    /// `rideTick` does, and the current then pushes it too), reset its fall
    /// distance in water and halved it in lava. A row below the world
    /// (`y < min_y - 64`, `checkBelowWorld`) is discarded there and hears no
    /// tick. Nothing applies gravity or moves the row: the listener's own
    /// gravity, drag and `move_by` do, as a vanilla `Entity` subclass's
    /// `tick()` does. A rider is ticked after
    /// its vehicle, depth-first as `tickPassenger` recurses. A passenger
    /// dismounted before its join-order slot can tick as a root if it has
    /// not already ticked this pass. Rows that joined this tick first tick
    /// next pass; parked levels and rows outside entity-ticking chunks do
    /// not tick. Canceling `entity_tick_pre` skips the row's own tick;
    /// cancellation alone does not skip its passengers' ticks. A listener
    /// can separately change the riding graph. `event.subject` is an
    /// [`EntityServerScope`](mod_api::EntityServerScope): `with_view` lends
    /// a live [`EntitySubjectView`](mod_api::EntitySubjectView) of the row
    /// (handle, kind, position, velocity, rotation, UUID and checked velocity,
    /// rotation and fall-distance writes), and `host()` borrows `&mut dyn ServerHost`
    /// sequentially for attachments (`view.handle()` is their holder) and other world
    /// operations. Told only for general entity
    /// types: vanilla families tick their own paths (#6509 owns bringing
    /// them to one join-order walk), so a part attached to `minecraft:zombie`
    /// hears no tick from this hook. Server-side.
    #[hook(
        receiver = "associated",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        scope = "entity",
        key = "kind",
        neoforge = "none (vanilla Entity.tick has no part-scoped counterpart; NeoForge's EntityTickEvent is the global pre/post pair)",
        version = 3
    )]
    fn general_entity_tick(kind: &str, subject: mod_api::EntityServerScope<'_>);
}
