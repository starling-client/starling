//! The `item_stacked_on_other` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ItemStackedOnOtherEvent` and vanilla's `Item.overrideStackedOnOther` /
    /// `Item.overrideOtherStackedOnMe`, fired inside a resolved pickup click (a direct left/right
    /// `Pickup` when no drag is active, or a one-slot left/right `QuickCraft` drag release; never
    /// on multi-slot or middle-button drags, active-drag resets, or creative catalogue slots).
    /// `click` lends the cursor and the clicked slot for this call: read them with `carried` and
    /// `clicked`, change them with `set_carried`, `set_clicked` and `take_clicked`. Reads see
    /// what earlier phases and this call changed.
    ///
    /// The phases run in vanilla `AbstractContainerMenu.tryItemClickBehaviourOverride` order:
    /// 1. Global `Hooks::item_stacked_on_other` listeners (`click.direction == Event`),
    ///    authoritative server side only.
    /// 2. The carried stack's `ItemPart::item_stacked_on_other` (`CarriedOnOther`), both sides
    ///    (server clicks, client prediction, and creative inventory-tab/hotbar pickups).
    /// 3. Vanilla's carried override (`BundleItem.overrideStackedOnOther`) on the stacks as the
    ///    earlier phases left them (skipping step 4 when it takes the click).
    /// 4. The clicked stack's `ItemPart::item_stacked_on_other` (`OtherStackedOnMe`), both sides.
    /// 5. Vanilla's clicked override (`BundleItem.overrideOtherStackedOnMe`), then plain pickup.
    ///
    /// Global listeners: `canceled` skips both item overrides and vanilla's bundle overrides.
    /// A cancelled click with `click.handled` (the start value, as in NeoForge) is done with
    /// the edits made; one without runs plain pickup. Edits on any other answer, a take, or a
    /// write to a result slot or a filled brewing potion slot refuse the click.
    ///
    /// `ItemPart` overrides answer as vanilla's `boolean` does: `Flow::Cancel` stops the click
    /// (nothing later runs), `Flow::Continue` goes on to the later phases and plain pickup, which
    /// see the edits made. `click.handled` is read only from global listeners.
    ///
    /// The edits are applied to the menu in the order they were made once every phase has
    /// answered. `set_clicked` writes the slot directly (`Slot.set`, without the slot's take
    /// side effects), and `take_clicked` takes through `Slot.safeTake`, running furnace
    /// experience, `recipes_used`, crafting remainders and take hooks for each take in turn.
    ///
    /// The click is refused, and every write rolled back, for:
    /// - an invalid stack, or a slot rule that refuses an edit;
    /// - a write through `Host` to what the click read;
    /// - a panic.
    ///
    /// On the server, this includes the mount's saddle, body equipment and chest slots as well as
    /// the player's inventory and cursor. An enclosing abort restores them together; the mount menu
    /// is reconciled from settled storage. During client prediction, server reconciliation remains
    /// authoritative.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "canceled",
        part = "item",
        neoforge = "ItemStackedOnOtherEvent",
        cancellable = true,
        version = 7
    )]
    fn item_stacked_on_other(
        click: &mut mod_api::item::menu::StackedOnOther<'_>,
        canceled: &mut bool,
    );
}
