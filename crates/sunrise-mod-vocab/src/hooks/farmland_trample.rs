//! The `farmland_trample` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player or mob that fell onto farmland is about to turn it to its base block
    /// (`BlockEvent.FarmlandTrampleEvent`, from `FarmlandBlock.fallOn`). `entity` is the landing
    /// living entity: read its pose, sneak state and type through the handle. It fires after
    /// vanilla's own chance and checks (the draw, the mob-griefing rule or spawn protection,
    /// and the size test), so a listener only ever sees a trample that was going to happen.
    /// Cancel and the farmland and its crop stay as they are; the entity takes its fall
    /// damage either way.
    ///
    /// The listener has the entity's level and can read and write it. The server asks inside
    /// the landing, before the lift, block write or fall damage. A completed listener's writes
    /// commit whether it cancels or not, matching NeoForge's `CommonHooks.onFarmlandTrample`:
    /// cancellation skips only the trample apply (the lift, base-block write and block-change
    /// game event). If the landing entity is gone or the block at `pos` is no longer `state`,
    /// the apply refuses without recomputing, rolling back only a partial lift while keeping
    /// the listener's committed writes. Otherwise, an allowed trample first lifts every
    /// intersecting body, then writes the base block and posts the block-change game event.
    /// If a listener panics or incurs a phase refusal, only its own writes and cancel flag are
    /// discarded; later eligible listeners still run. Only a panic disables that mod's callback
    /// for the session. Fall damage follows whether the trample applies or not. With no
    /// listener, the lift, write and damage run in the same order, without a transaction.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BlockEvent.FarmlandTrampleEvent",
        cancellable = true,
        version = 5,
        listener_transaction = "always"
    )]
    fn farmland_trample(
        entity: mod_api::LivingRef,
        pos: BlockPos,
        state: BlockState,
        fall_distance: f64,
        cancel: &mut bool,
    );
}
