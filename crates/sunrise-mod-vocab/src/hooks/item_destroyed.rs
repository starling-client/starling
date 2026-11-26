//! The `item_destroyed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.onDestroyed` (vanilla `ItemStack.onDestroyed`): a dropped item entity
    /// holding `stack` has reached zero health from `damage_type` (fire, lava, cactus, explosion,
    /// `/damage`, or block-contact damage) and is about to be discarded.
    ///
    /// Runs on the server inside a transaction, before built-in container components are cleared
    /// and before the entity is removed (`stack` and `item.with_stack(host, ...)` both still carry the
    /// original intact stack, matching vanilla `ItemEntity.hurtServer` -> `ItemStack.onDestroyed`
    /// -> `discard()`). `drops` arrives holding any vanilla container contents that will spill
    /// from `stack` (`minecraft:bundle_contents` or `minecraft:container`); push or edit stacks in
    /// `drops` to spawn them at the entity's position (`ItemUtils.onContainerDestroyed`) when the
    /// entity is discarded. Each part's drop list is validated before continuing (an invalid stack
    /// rolls that part's edit back), and a panic or a refused commit rolls back the transaction's
    /// host writes and keeps the vanilla drop baseline while still discarding the destroyed item
    /// entity.
    /// Only the `ItemPart`s attached to `stack`'s item hear it: an item with none costs one
    /// table lookup, and a game with no such part none at all.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "drops",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.onDestroyed"
    )]
    fn item_destroyed(
        stack: &ItemStack,
        item: ItemEntityRef,
        damage_type: &str,
        drops: &mut Vec<ItemStack>,
    );
}
