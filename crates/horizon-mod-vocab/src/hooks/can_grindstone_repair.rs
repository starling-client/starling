//! The `can_grindstone_repair` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.canGrindstoneRepair`: does the grindstone repair `stack` as it repairs a damageable item? `can_repair` arrives with vanilla's answer, whether the stack is damageable (`ItemStack.isDamageableItem`); set it `true` or `false`. It is asked where the grindstone reads that rule, on both sides (the server's menu and the client's own result): `GrindstoneMenu.mergeItems`, for the top stack when both inputs are single stacks of one item, where `true` takes the repair branch (one item, both inputs' enchantments merged, then all but the curses removed; the durability is merged only when the result has durability, as vanilla checks it again on the copy) and `false` the other one (two equal stacks of an item that stacks give two, anything else nothing); and the input slots' `mayPlace` (damageable or enchanted), where a stack it grants may be placed. A single enchanted input is never asked: that branch merges nothing. Only the parts attached to `stack`'s item are asked. It is given the loop `context` of the side that runs the menu and takes no `Host`, so answer from the stack and `context` alone, and fast. With no part on the stack's item it costs one table lookup and the stack isn't converted. The contract beyond the method's name and parameters is Horizon's reading, not checked against NeoForge's. Diverges from upstream: new hook.
    #[notes = r#"## Grindstone repair

- **Item Part method `can_grindstone_repair(context, event)`** (NeoForge's `IItemExtension.canGrindstoneRepair`): set `event.can_repair`, which starts as whether `event.stack` is damageable (`isDamageableItem`). It is asked by `GrindstoneMenu.mergeItems` for the top stack when both inputs are single stacks of one item (`true`: one repaired item with the inputs' enchantments merged, the curses kept; `false`: two equal stacks of a stackable item give two, anything else nothing) and by the input slots' `mayPlace` (damageable or enchanted), on both sides. A single enchanted input never asks. Only the parts of the stack's item are asked; with none the stack isn't converted. Diverges from upstream: new hook."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "item",
        key = "stack",
        cancellable = false,
        target = "horizon_client_lib::stations::native_can_grindstone_repair",
        neoforge = "IItemExtension.canGrindstoneRepair"
    )]
    fn can_grindstone_repair();
}
