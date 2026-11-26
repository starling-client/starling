//! The `can_continue_using` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.canContinueUsing`: does the use of `stack`,
    /// the stack it began with (vanilla's `useItem`), go on now that the used
    /// `hand` holds `held`? `entity` is the user's id, as in
    /// `living_entity_use_item_start`. `can_continue`
    /// arrives with vanilla's answer (`ItemStack.isSameItem(held, stack)`).
    /// `true` goes on with `held` as the use's stack; `false` stops the use
    /// without a release. Asked each tick of a use, where
    /// `LivingEntity.updatingUsingItem` starts and before
    /// `living_entity_use_item_tick`, on the server and on a server's client.
    /// Not asked for an emptied hand, which stops the use, nor for a hotbar
    /// switch, which stops a main-hand use before any item check. Only the
    /// `ItemPart`s attached to `stack`'s item are asked.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.canContinueUsing",
        version = 2
    )]
    fn can_continue_using(
        stack: &ItemStack,
        held: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        can_continue: &mut bool,
    );
}
