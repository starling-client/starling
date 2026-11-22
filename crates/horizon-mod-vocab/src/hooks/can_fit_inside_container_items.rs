//! The `can_fit_inside_container_items` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.canFitInsideContainerItems`: may `stack` go inside an item that holds items? `fits` arrives with vanilla's answer (`Item.canFitInsideContainerItems`): every stack fits but a shulker box. Set it `true` or `false`. It is asked, on both sides, by a shulker box's slots (`ShulkerBoxSlot.mayPlace`): in its menu, the window's own and a guest's. A `true` for a shulker box lets shulker boxes nest there; that is the mod's choice, and the NBT readers' depth limit still bounds it. Bundles and the item slots a mod gives an item (`item_container`) keep vanilla's rule and do not ask, nor does a hopper filling a shulker box, which vanilla tests by the block there. Only the parts attached to `stack`'s item are asked. It is given the loop `context` of the side that runs the menu and takes no `Host`, so answer from the stack and `context` alone, and fast: a click may ask it for each slot it checks. With no part on the stack's item it costs one table lookup and the stack isn't converted. The contract beyond the method's name and parameters is Horizon's reading, not checked against NeoForge's. Diverges from upstream: new hook.
    #[notes = r#"## Items inside container items

- **Item Part method `can_fit_inside_container_items(context, event)`** (NeoForge's `IItemExtension.canFitInsideContainerItems`): set `event.fits`, which starts as vanilla's answer (every stack fits but a shulker box). It is asked by a shulker box's slots (`ShulkerBoxSlot.mayPlace`) in its menu, the window's own and a guest's, on both sides; `true` for a shulker box lets them nest there. Bundles, a mod's item slots (`item_container`) and hoppers keep vanilla's rule. Only the parts of the stack's item are asked; with none the stack isn't converted. Diverges from upstream: new hook."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "item",
        key = "stack",
        cancellable = false,
        target = "horizon_client_lib::menu::bundle::native_can_fit_inside_container_items",
        neoforge = "IItemExtension.canFitInsideContainerItems"
    )]
    fn can_fit_inside_container_items();
}
