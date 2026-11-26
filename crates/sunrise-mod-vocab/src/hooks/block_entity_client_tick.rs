//! The `block_entity_client_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A client tick of this client's copy of the mod block entity
    /// (`ClientLevel.tickBlockEntities`), when its type is yours and its
    /// registered type's ticker accepts this state on the client:
    /// once per client tick while the level runs, in vanilla's ticker order
    /// among the others, whether or not the copy is drawn. `event.subject`
    /// is a [`BlockEntityClientScope`](mod_api::BlockEntityClientScope): borrow a
    /// read-only [`BlockEntityReadView`](mod_api::BlockEntityReadView) through
    /// `with_view` for the resident copy's handle, position, level, block state,
    /// data slots, item slots and merged NBT (`block_entity_update_tag`), and
    /// borrow `ClientHost` sequentially through `host()` to start, stop and fade
    /// sounds or add particles. Never called on a server, and never again for a
    /// copy removed or whose chunk unloaded.
    #[notes = r##"### Client ticks: hums, smoke and moving parts

`block_entity_client_tick` is vanilla's client `BlockEntityTicker` (`EntityBlock.getTicker` asked with the
client level): what a machine does on the client alone, a looping hum, smoke from its chimney, a part eased
between two server updates. Implement the method on a `BlockEntityPart` attached to the type. It binds the
client ticker for that type's accepted states. Use `BlockEntityTypeBuilder::ticker_when` to select only some
states or sides; with no interested Part, nothing is bound.

It runs on each client (the window's own in single player included) once per client tick while its level runs
(not paused or frozen), for each loaded copy of the type, in vanilla's `Level.blockEntityTickers` order.
A chunk's initial tickers join in packet order; a state change that adds a ticker joins it when enabled.
A loaded copy ticks whether or not it is drawn (`block_entity_submit` runs only for those in view). `event.subject` is a
`BlockEntityClientScope`: call `event.subject.with_view(|view| ...)` to read the resident copy's handle,
position, level, block state, data slots, item slots and merged NBT directly with no per-tick snapshot copy,
then call `event.subject.host()` for the `ClientHost` to play a sound with `play_local_sound` (keeping its
`SoundRef` to stop or fade it) or add particles with `add_particle`. The resident data is read only and holds
what the frame's handling of the server's packets merged into the copy (`block_entity_data_received` told
first): a frame handles what has arrived and then runs its client ticks, so all the ticks of one frame see
the same data and an update arriving during a frame is seen from the next frame's first tick. A copy that is
removed, replaced by another type, changed to a state without a client ticker or whose chunk unloads is not
called again from that tick on, and its handle no longer reads once it is removed, replaced or unloaded: stop
what you keep for it then. It never runs on a server."##]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "subject",
        neoforge = "none (vanilla BlockEntityTicker from EntityBlock.getTicker with the client level; NeoForge adds no event)",
        version = 3
    )]
    fn block_entity_client_tick(subject: mod_api::BlockEntityClientScope<'_>);
}
