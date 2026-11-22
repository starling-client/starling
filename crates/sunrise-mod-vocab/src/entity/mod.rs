//! Entities, mobs, players, effects and their handles.

pub(crate) mod attachment_holder;
pub(crate) mod attachment_query;
pub(crate) mod chunk_areas;
pub(crate) mod collider;
pub(crate) mod entity_reach;
pub(crate) mod entity_tags;
pub(crate) mod entity_type_defaults;
pub(crate) mod mob_ai_pause;
pub(crate) mod original_player;
pub(crate) mod player_data;
pub(crate) mod player_world;
pub(crate) mod ride_player;
pub(crate) mod start_riding;
pub(crate) mod swimming_check;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod behaviors;
pub(crate) mod effect_category;
pub(crate) mod entity_capability;
pub(crate) mod entity_scopes;
pub(crate) mod goals;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod health;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod mobs;
pub(crate) mod part_state;
pub(crate) mod pending_children;
pub(crate) mod player_bots;
pub(crate) mod types;
pub(crate) mod unspawned_mob;
