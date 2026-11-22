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
    /// the landing, before the lift, block write or fall damage. The listener's writes and the
    /// apply run in one transaction. If the landing entity is gone or the block at `pos` is no
    /// longer `state`, the apply refuses without recomputing, and transactional writes roll back.
    /// Otherwise, an allowed trample first lifts every intersecting body, then writes the base
    /// block and posts the block-change game event. A real cancellation or stale preconditions
    /// roll back the whole trample transaction. If a listener panics or incurs a phase refusal,
    /// only its own writes and cancel flag are discarded; later eligible listeners still run.
    /// Only a panic disables that mod's callback for the session. Fall damage follows whether
    /// the trample applies or not. With no listener, the lift, write and damage run in the same
    /// order, without a transaction.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BlockEvent.FarmlandTrampleEvent",
        cancellable = true,
        version = 4
    )]
    fn farmland_trample(
        entity: mod_api::LivingRef,
        pos: BlockPos,
        state: BlockState,
        fall_distance: f64,
        cancel: &mut bool,
    );
}

/// Original upstream notes; the existing fork event above owns this moment.
pub trait UpstreamNotes {
    #[notes = r###"farmland trampling: `farmland_trample(host, event)` is heard when a player or mob that fell onto farmland is about to turn it to dirt, after vanilla's chance and checks (NeoForge's `FarmlandTrampleEvent`, Bukkit's physical `PlayerInteractEvent` and `EntityInteractEvent`). Cancel and the farmland and its crop stay. Version 3 takes the server host and a `LivingRef`, not a loop context. Cancellation or stale state rolls back the whole action. A failed callback rolls back only its own writes and does not cancel; later eligible listeners still run. Otherwise bodies are lifted before the base block is written and `BLOCK_CHANGE` is posted. Fall damage follows either way. Bundles that use it need `"hooks": @API:trample@`."###]
    fn trample();
}
