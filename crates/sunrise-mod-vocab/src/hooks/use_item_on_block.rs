//! The `use_item_on_block` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `UseItemOnBlockEvent`: after a phase was chosen, before its Part or
    /// vanilla behaviour. Cancel skips that behaviour and returns
    /// `cancellation_result` (initially `TryWithEmptyHand`). In `ItemBeforeBlock`,
    /// `Pass` continues to the block, skipping the item's `on_item_use_first`
    /// Part; any result other than `Pass` ends the block interaction.
    /// A block-phase `TryWithEmptyHand` can still try the empty-hand behaviour.
    /// On a server-registered cauldron click, the client waits for block fallthrough before
    /// `ItemAfterBlock`; a consuming server item result doesn't skip this local phase.
    /// Other clicks can predict vanilla on a client without the mod; the server corrects it.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "UseItemOnBlockEvent",
        cancellable = true
    )]
    fn use_item_on_block(
        click: &BlockClick<'_>,
        phase: UsePhase,
        cancellation_result: &mut mod_api::InteractionResult,
        canceled: &mut bool,
    );
}
