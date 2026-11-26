//! The `item_picked_up` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` touches the ground item entity `item`
    /// (`ItemEntity.playerTouch`), before its pickup delay and target are
    /// checked: it is asked on every tick the two touch, even while the item
    /// can't be taken. `Default` requires a live delay of exactly zero and
    /// no target or this player as the target. Set `True` to take it despite
    /// its delay or another player's target (a full inventory still leaves it),
    /// or `False` to veto this pickup
    /// attempt without changing the item's pickup delay (matching NeoForge
    /// `ItemEntityPickupEvent.Pre`). The last mod to set it decides. The
    /// handle reads and mutates the live ground item entity on both the host
    /// player and guest paths. Pickup inserts from the stack captured before
    /// Pre, without overwriting a replacement installed by the listener. Server-side.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ItemEntityPickupEvent.Pre",
        version = 3
    )]
    fn item_picked_up(player: u128, item: ItemEntityRef, can_pickup: &mut TriState);
}
