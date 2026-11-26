//! Per-state collision classification and landing coefficients.

/// What `BlockStates::collision` tells of a state's collision.
// no wire: a local read of the game's shape tables
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StateCollision {
    /// These boxes at every position, for every entity.
    Fixed(Vec<[f64; 6]>),
    /// These boxes with an empty context, but an entity's context can change them: scaffolding
    /// (standing above it, sneaking), powder snow (leather boots, falling) and liquids. Ask per
    /// entity where it matters.
    Contextual(Vec<[f64; 6]>),
    /// Differs by position or block entity: the random offset of bamboo and pointed dripstone,
    /// shulker boxes, moving pistons, block entities that publish shapes, and states whose block
    /// parts answer collision queries (`BlockPart::collision_shape` takes the position). Read it
    /// per position with `ClientHost::block_collision` or `LevelRef::collision_boxes`.
    Positional,
}

/// What `BlockStates::landing` tells of landing on a state's block.
// exhaustive: the game builds it with a literal; mods only read it
// no wire: a local read of the game's movement table
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockLanding {
    /// Share of downward speed an entity keeps, reversed, when landing on the block.
    pub bounce_restitution: f32,
    /// Share of fall distance the block removes on landing.
    pub fall_distance_reduction: f32,
}
