//! The `block_use_without_item` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockBehaviour.useWithoutItem`: the captured state's empty-hand method,
    /// after useItemOn asks for it, on the server and main hand only.
    /// A disabled owner's subscribed interaction passes without running the block template.
    #[hook(
        dispatch = "flat",
        side = "server",
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
