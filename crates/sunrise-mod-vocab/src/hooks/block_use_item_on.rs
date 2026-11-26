//! The `block_use_item_on` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockBehaviour.useItemOn`: the block's own behaviour, with the
    /// held item, on either hand and both logical sides, after the `Block` phase.
    /// `None` defers to the template; `TryWithEmptyHand` selects `useWithoutItem`
    /// on the main hand, and `Pass` or `Fail` lets the item try its `useOn`.
    /// `Success` predicts a swing; `SuccessServer` and `Consume` do not.
    /// The client host allows reads and local effects, not server writes.
    /// A disabled owner's subscribed interaction passes without running the block template.
    #[hook(
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "replace",
        out = "answer",
        scope = "block",
        inert = "*answer = Some(InteractionResult::Pass);",
        key = "state",
        neoforge = "none (vanilla BlockBehaviour.useItemOn; PlayerInteractEvent.RightClickBlock cancels or answers before it)"
    )]
    fn use_item_on(
        state: BlockState,
        click: &BlockClick<'_>,
        answer: &mut Option<mod_api::InteractionResult>,
    );
}
