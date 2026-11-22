//! The `menu_slot_take` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player took a stack out of slot `slot` of the mod menu type `menu`
    /// open on the mod block entity (`Slot.onTake`), on the server: a notice
    /// after the item left the slot, which refuses nothing (the pickup rule,
    /// `menu_slot_rules`' `may_pickup`, refuses). `subject` lends sequential
    /// access to the live resident block entity (`subject.with_view(...)`),
    /// the server `Host` (`subject.host()`), and atomic transactions
    /// (`subject.transaction(...)`); `with_view` returns `None` if the block
    /// entity was removed or replaced during the call. `player` is the
    /// player's UUID; `slot` is the menu's index, the block's own slots first
    /// (linked containers and the player's inventory are never heard). `taken` is
    /// a copy of the stack that left the slot, components included, and `left`
    /// what the slot holds afterwards (`None` when it was emptied). Vanilla
    /// hands `onTake` the taken stack on a click but the stack left in the
    /// slot on a quick move, so both are carried: for a quick move `taken` is
    /// the amount moved (the slot's stack before, less `left`).
    ///
    /// Heard once per take, after it: a pickup to an empty cursor (all of the
    /// slot with the left button, half rounded up with the right); a pickup
    /// onto a matching cursor stack from a slot that refuses placement (a
    /// result slot, all of it or nothing); a hotbar or offhand swap that takes
    /// the slot's item (the whole stack, or the slot's former stack when both
    /// held items; a stack that did not fit goes to the inventory or drops
    /// after the notice); a throw (once per repeat of the control variant);
    /// pick-all (once per slot taken from, with the part taken); and each
    /// quick move that moved something. A shift-click on a crafting result
    /// repeats the move once per craft, so it is heard once per craft, each
    /// with that craft's stack. When a part answered the shift-click with
    /// `menu_quick_move`'s `handled`, it is heard after the move that part
    /// applied, if it moved something, with the stack and the count it moved.
    /// Not heard for an exchange of the cursor and slot stacks, an insertion,
    /// a drag spread, a creative clone, dropping the cursor, a take the pickup
    /// rule refused or a quick move that moved nothing; menu buttons, recipe
    /// placement and creative slot sets are not clicks and never reach it.
    ///
    /// Writes through `subject.with_view(...)` are checked at the boundary
    /// and apply to the live resident block entity in place, marking the
    /// chunk dirty and firing container updates when items change (matching
    /// vanilla `BlockEntity.setChanged`). On game-routed clicks an enclosing
    /// click transaction is open, so a panicking listener is contained and
    /// disabled for `menu_slot_take` for the rest of the session and the click
    /// undoes its transaction before resending the menu. On a shift-click
    /// handled by `menu_quick_move`, the remainder is applied and published
    /// first with no enclosing click transaction: wrap writes that must abort
    /// together in `subject.transaction(...)`, while live writes made outside
    /// a transaction survive (and still publish container updates) alongside
    /// the earlier quick-move settlement. Not `item_crafted` or
    /// `item_smelted`, which keep their meaning: a result slot that gives
    /// experience or unlocks a recipe does it here through the host. Nothing
    /// is asked when no mod listens.
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "kind",
        version = 2,
        neoforge = "none (a Java menu's slot overrides Slot.onTake; NeoForge adds no event)"
    )]
    fn menu_slot_take(
        kind: mod_api::BlockEntityTypeKey,
        subject: mod_api::BlockEntityScope<'_>,
        player: u128,
        menu: &str,
        slot: usize,
        taken: &mod_api::ItemStack,
        left: Option<&mod_api::ItemStack>,
    );
}
