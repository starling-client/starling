//! The `block_use_without_item` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockBehaviour.useWithoutItem`: the captured state's empty-hand method,
    /// on the main hand of either logical side when the selected block-use result
    /// is `TryWithEmptyHand`, including from cancellation of the `Block` phase.
    /// `event.click.stack` still describes what the hand holds. `None` defers to
    /// the template; a supplied result replaces it. `Pass`, `Fail` and
    /// `TryWithEmptyHand` proceed to the item without retrying the empty method.
    /// Only `Success` predicts a swing; the client host allows reads and local effects.
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
        neoforge = "none (vanilla BlockBehaviour.useWithoutItem; PlayerInteractEvent.RightClickBlock cancels or answers before it)"
    )]
    fn use_without_item(
        state: BlockState,
        click: &BlockClick<'_>,
        answer: &mut Option<mod_api::InteractionResult>,
    );
}
