//! The `living_entity_use_item_finish` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0105

pub trait Hooks {
    /// NeoForge's `LivingEntityUseItemEvent.Finish`: `entity`'s use of
    /// `stack` in `hand` ran its time (`LivingEntity.completeUsingItem`),
    /// `duration` the ticks left, always 0. `entity` is as in
    /// `native_living_entity_use_item_start`. `result` arrives with what
    /// `finishUsingItem` left in the hand (the bowl of a stew, none for the
    /// last apple; a mod's own item is kept) and may be replaced. Fired for
    /// consumables, potions, spyglasses and mods' items. Server-side, for every player
    /// of the world (the helpers read the acting one).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LivingEntityUseItemEvent.Finish"
    )]
    fn living_entity_use_item_finish();
}
