//! The `on_item_use_first` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.onItemUseFirst`: the held item's first say on a
    /// click on a block, on both logical sides, after
    /// `use_item_on_block` (`ItemBeforeBlock`) let it through and before the
    /// block's own `useItemOn`. `None` defers; `Pass` goes on to the block.
    /// Any other result skips the block and the item's `useOn`. Only
    /// `Success` swings on the client. A client answer changes prediction,
    /// not the authoritative request; its host allows reads and local effects.
    #[hook(
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "replace",
        out = "answer",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.onItemUseFirst"
    )]
    fn on_item_use_first(
        stack: &mod_api::ItemStack,
        click: &BlockClick<'_>,
        answer: &mut Option<mod_api::InteractionResult>,
    );
}
