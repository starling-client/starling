//! The server block-entity ticker, in vanilla's `Level.tickBlockEntities` order.

pub trait Hooks {
    /// A server tick of a resident mod block entity of your registered `kind`.
    /// The game skips the callback if the current block does not support that
    /// entity type, as in `LevelChunk.BoundTickingBlockEntity.tick`. The first
    /// invalid visit logs a warning; a valid tick clears it for the next episode.
    /// `event.subject.with_view` lends its live storage. To reach the level or
    /// another object, return from the loan and call `event.subject.host()`.
    /// Ask for another loan afterwards; it returns `None` if the level, chunk
    /// load, generation or block-entity type validity (`BlockEntityType.isValid`
    /// for the live block state) no longer matches. In-place block-state updates
    /// to another valid state remain borrowable with the live state. For a
    /// Part-state edit, first get the handle with
    /// `event.subject.with_view(|view| view.handle())`, then borrow
    /// `event.subject.host()` to call `edit_state`. Keep a handle only to use
    /// across calls.
    ///
    /// Setters apply checked writes in place, without cloning the row on
    /// read-only or unchanged loans. Edits join the ambient transaction; abort
    /// restores any one-shot client fields waiting before its first edit. Nothing
    /// opens a per-listener transaction and there is no tick reply. Use
    /// `event.subject.transaction` when one operation must edit the subject and
    /// other storage atomically. Views do not automatically mark the chunk
    /// unsaved, notify neighbors, or send a block update; call `set_changed`
    /// explicitly through `event.subject.host()` when persisted state changes
    /// (matching vanilla `BlockEntity.setChanged` / `AbstractFurnaceBlockEntity.serverTick`).
    /// Normal menu synchronization reads the resident data slots. With no Part
    /// subscribed for this type, there is no ticker. An optional selector on
    /// `BlockEntityTypeBuilder::ticker_when` restricts the registered states and sides.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "kind",
        neoforge = "none (vanilla BlockEntityTicker; NeoForge adds no event or extension for it)",
        version = 6
    )]
    fn block_entity_tick(kind: mod_api::BlockEntityTypeKey, subject: mod_api::BlockEntityScope<'_>);
}
