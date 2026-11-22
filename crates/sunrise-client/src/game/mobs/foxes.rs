//! Foxes and dolphins on the game side: what they take from the ground (`Fox.pickUpItem`,
//! `Fox.canHoldItem`, `Dolphin.pickUpItem`) and where `FoxSearchForItemsGoal` and
//! `Dolphin.MoveToItemGoal` walk to.
use super::Game;

impl Game<'_> {
    /// `Fox.ALLOWED_ITEMS` and the looting block of `Mob.aiStep` for foxes, once a tick.
    pub(super) fn tick_foxes(&mut self) {
        sunrise_gameplay::foxes::tick_foxes(self)
    }
    /// `Dolphin.pickUpItem` in `Mob.aiStep`'s looting block, and the item
    /// `Dolphin.MoveToItemGoal` swims to (`Dolphin.ALLOWED_ITEMS`: no pickup delay, alive, in water).
    pub(super) fn tick_dolphins(&mut self) {
        sunrise_gameplay::foxes::tick_dolphins(self)
    }
}
