//! The `menu_slot_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The non-result slots of an open mod menu of type `menu` changed on the server, so its
    /// outputs can follow its inputs, as vanilla processing menus do in
    /// `AbstractContainerMenu.slotsChanged`. Nothing is asked when no part listens for `menu`.
    ///
    /// The contract is Sunrise's own design. Vanilla calls `slotsChanged` from the menu's own
    /// transient container on each `setItem`, one slot at a time, and a block entity's container
    /// never notifies an open menu. Here one call reports one batch: the slots of the menu's own
    /// container that differ between two points, in `changed` in ascending menu slot order.
    ///
    /// - A click reports each click step (a whole multi-slot drag, a swap, a throw or a
    ///   pickup-all is one step; a take whose listener changes the inputs adds a step before the
    ///   click continues), inside the click's transaction: default clicks on blockless owned or
    ///   block-backed mod menus, `menu_quick_move` handlers and `menu_clicked` overrides returning
    ///   `Flow::Cancel`.
    /// - A committed host write (`ItemPlace::MenuSlot`, `ItemPlace::BlockEntity`,
    ///   `BlockEntityRef::set_item_slot`, transactional `merge_data` / `replace_data`) is one batch
    ///   per menu for each outermost committed transaction, never on abort: owned menus by menu
    ///   generation, then block-backed menus by block position.
    /// - A write outside any transaction to the block entity behind an open block-backed menu (a
    ///   hopper, `/item replace block`, non-transactional `merge_data`) is one batch at the write.
    ///
    /// It is batched so a listener sees a whole action and a consistent `context` rather than
    /// the states in between, and block writes report so a menu a hopper fills shows the output a
    /// click would have made. Result, hidden, linked and player inventory slots never count, and a
    /// count-0 stack counts as empty. `context` carries the viewing player, the backing block
    /// position (`None` for a blockless menu even if its opener passed a screen position hint) and
    /// a read-only snapshot of the menu's own containers after the change.
    ///
    /// Push [`MenuSlotWrite`] entries to `outputs` to set declared result slots (or any declared
    /// non-hidden owned slot of a blockless owned menu), applied atomically and never reported
    /// again; a later entry for the same slot wins. Viewers see a click's outputs in its reply and
    /// any other write's at the next `broadcastChanges`. An invalid write (not `Main`, an
    /// out-of-range, hidden, linked, or, on a block-backed menu, non-result slot, an invalid stack,
    /// or a count past the slot's and item's limit), a listener panic, or a changed slot holding a
    /// stack the mod API can't carry (an item with no API key, a component patch the API rejects)
    /// undoes the whole click (cursor, inventory, transient slots or block entity, effects) and
    /// resends the menu. Outside a click the write itself stands: the listener's own writes roll
    /// back, and an unreportable change is logged and not reported.
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        scope = "menu",
        key = "menu",
        neoforge = "none (vanilla Container.setItem and AbstractContainerMenu.slotsChanged; NeoForge adds no event)"
    )]
    fn menu_slot_changed(
        menu: &str,
        changed: &[MenuSlotChange],
        context: &MenuSlotContext<'_>,
        outputs: &mut Vec<MenuSlotWrite>,
    );
}
