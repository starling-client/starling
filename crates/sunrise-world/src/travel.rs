//! Which `LivingEntity.travel` branch a living body is about to run — the
//! game-side twin of the mod API's `TravelMode` (the `living_travel` hook of
//! #2308/#2414), kept separate so the physics below the game stays free of
//! mod-API types, as `entity_move::MoverType` is.

/// The resolved branch of a `LivingEntity.travel` invocation. Replacing the
/// input or velocity does not change which branch runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TravelMode {
    /// `LivingEntity.travelInAir`: the ordinary ground-and-air branch.
    Air,
    /// `LivingEntity.travelInFluid` with water: swimming.
    Water,
    /// `LivingEntity.travelInFluid` with lava.
    Lava,
    /// `LivingEntity.travelFallFlying`: gliding.
    Glide,
    /// The flying branch (`Player` abilities flying or a mob's flight).
    Flying,
    /// Move using the existing velocity alone, as squid do in and out of water.
    VelocityOnly,
}
