//! The `on_stop_using` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.onStopUsing`: the use of `stack` by
    /// `entity` (a player's id or a mob's) in `hand` is over, with `time_left`
    /// ticks of it left (0 for a finished use). Where
    /// `LivingEntity.stopUsingItem` runs, once, after the use is cleared: a
    /// listener that releases again finds no use. It comes after
    /// `release_using`, `living_entity_use_item_finish` and the item's own
    /// release, whatever stopped the use: a release (even one
    /// `living_entity_use_item_stop` canceled), a finish, a hand that
    /// changed item, a brush, a mob's release, and the server's other stops
    /// (a hotbar switch, swapped hands, a disabled shield, spectator mode,
    /// a dimension trip). An emptied hand is a used-up stack and isn't a
    /// stop. On a server's client it comes where the client ends its use
    /// (a finish, a release, a hand that changed item), not where the
    /// synced flags clear it: the server heard that stop. Both sides, as
    /// `release_using`: change the world only where this `Game` runs the
    /// world, not on a server's client. Only the `ItemPart`s attached to
    /// `stack`'s item are told.
    #[hook(
        dispatch = "flat",
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.onStopUsing",
        version = 2
    )]
    fn on_stop_using(stack: &ItemStack, entity: Option<i32>, hand: Hand, time_left: i32);
}
