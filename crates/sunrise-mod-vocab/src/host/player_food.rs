//! The `player_food` area: food level, saturation and exhaustion. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_food` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerFoodBackend {
    /// Side: both.
    /// `FoodData.needsFood`: the food level is below 20.
    fn player_needs_food(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `FoodData.hasEnoughFood`: the food level is above 6, enough to
    /// sprint.
    fn player_has_enough_food(&self, player: PlayerRef) -> Option<bool>;
    /// Side: both.
    /// `Player.canEat(always)`: an invulnerable player (creative), `always`
    /// (food that can always be eaten) or a food level below 20.
    fn player_can_eat(&self, player: PlayerRef, always: bool) -> Option<bool>;
    /// Side: both.
    /// `FoodData.getFoodLevel/getSaturationLevel/getExhaustionLevel`; Paper
    /// `HumanEntity#getFoodLevel/getSaturation/getExhaustion`. None until the
    /// guest's body exists, as well as for stale or unavailable players.
    fn player_food(&self, player: PlayerRef) -> Option<PlayerFood>;
}
/// The handles' writes of the `player_food` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerFoodBackendMut {
    /// Side: server.
    /// `FoodData.eat(nutrition, saturationModifier)`: adds `nutrition` to
    /// the food level (0 to 20) and `nutrition * modifier * 2` to the
    /// saturation, which stays at most the food level.
    fn player_feed(&mut self, player: PlayerRef, nutrition: i32, saturation_modifier: f32) -> bool;
    /// Side: server.
    /// `Player.causeFoodExhaustion`: adds exhaustion (at most 40), as
    /// sprinting or jumping does. An invulnerable player (creative) or a
    /// spectator gets none, and the call still returns true.
    fn player_add_exhaustion(&mut self, player: PlayerRef, amount: f32) -> bool;
    /// Side: server.
    /// `FoodData.setFoodLevel`: sets the food level as it is, unclamped,
    /// as vanilla does.
    fn player_set_food_level(&mut self, player: PlayerRef, level: i32) -> bool;
    /// Side: server.
    /// `FoodData.setSaturation`: sets the saturation as it is, unclamped.
    fn player_set_saturation(&mut self, player: PlayerRef, saturation: f32) -> bool;
    /// Side: server.
    /// Sets the exhaustion as it is (Paper's `HumanEntity#setExhaustion`;
    /// vanilla's `FoodData` only adds to it).
    fn player_set_exhaustion(&mut self, player: PlayerRef, exhaustion: f32) -> bool;
}
