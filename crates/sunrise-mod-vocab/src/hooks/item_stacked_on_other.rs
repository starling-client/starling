//! The `item_stacked_on_other` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ItemStackedOnOtherEvent` and vanilla's `Item.overrideStackedOnOther` /
    /// `Item.overrideOtherStackedOnMe`, fired inside a resolved pickup click (a direct left/right
    /// `Pickup` when no drag is active, or a one-slot left/right `QuickCraft` drag release; never
    /// on multi-slot or middle-button drags, active-drag resets, or creative catalogue slots), with
    /// the cursor and both stacks frozen in `event`.
    ///
    /// Dispatch runs inside one decide/fire/apply transaction in vanilla
    /// `AbstractContainerMenu.tryItemClickBehaviourOverride` order:
    /// 1. Global `Hooks::item_stacked_on_other` listeners (`event.direction == Event`),
    ///    authoritative server side only.
    /// 2. The carried stack's `ItemPart::item_stacked_on_other` (`CarriedOnOther`), both sides
    ///    (server clicks, client prediction, and creative inventory-tab/hotbar pickups).
    /// 3. Vanilla's carried override (`BundleItem.overrideStackedOnOther`, dispatched by item type)
    ///    using the real carried outcome (skipping step 4 when handled).
    /// 4. The clicked stack's `ItemPart::item_stacked_on_other` (`OtherStackedOnMe`), both sides.
    /// 5. Vanilla's clicked override (`BundleItem.overrideOtherStackedOnMe`), then plain pickup.
    ///
    /// Global listener contract (`Hooks::item_stacked_on_other`): `canceled` skips later override
    /// phases and vanilla's bundle override, and a cancelled click that is also `handled` is done
    /// with the stacks `result` sets (a cancelled click not `handled` runs plain pickup; setting a
    /// stack on any other answer or using `ClickedStackResult::Take` refuses the click). A global
    /// replacement cannot change a result slot or take a stack from a filled brewing potion slot.
    ///
    /// `ItemPart` override contract (`CarriedOnOther` / `OtherStackedOnMe`): represents both
    /// vanilla outcomes inside the transaction:
    /// - Handled (stop): set `result.handled = true` and return `Flow::Cancel` (with optional
    ///   `carried`/`clicked` edits). Stops later overrides and plain pickup.
    /// - Not handled (continue): leave `result.handled = false` and return `Flow::Continue` (with
    ///   optional `carried`/`clicked` edits, which are passed to later overrides and plain pickup).
    ///
    /// For `result.clicked`, `ClickedStackResult::Set(stack)` writes the slot directly (`Slot.set`,
    /// without running `Slot.onTake` side effects — including clearing an occupied output/result
    /// slot to `None` with no furnace XP or take events), while `ClickedStackResult::Take(count)`
    /// takes from the slot through `Slot.safeTake` / `Slot.onTake` (running furnace XP,
    /// `recipes_used`, crafting remainders, and take hooks).
    ///
    /// Contradictory `ItemPart` answers (`canceled != result.handled`), invalid stacks, writes
    /// through `Host` to what the click read, or panics refuse the click and roll back every
    /// write. During client prediction, server reconciliation remains authoritative.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "canceled",
        part = "item",
        neoforge = "ItemStackedOnOtherEvent",
        cancellable = true,
        version = 6
    )]
    fn item_stacked_on_other(
        event: &mod_api::item::menu::StackedOnOther,
        result: &mut mod_api::item::menu::StackedOnOtherResult,
        canceled: &mut bool,
    );
}
