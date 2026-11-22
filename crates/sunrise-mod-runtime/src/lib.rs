//! Engine-independent storage and registration machinery shared by the game and mod API.

/// Longest general entity type key (`mod:name`), in bytes: what registration accepts is what the
/// spawn that introduces a row to a client (`sunrise:general_entity`) can carry.
pub const MAX_GENERAL_ENTITY_KEY: usize = 256;

pub mod attachments;
pub mod byte_channel;
/// A level's capability generations, which kept capabilities compare (#3446).
pub mod capability_generations;
/// Action-local ownership and ordered consumption of pending spawn outputs.
pub mod pending_children;
/// Load-owned registry names and numeric slots, without game or API key types.
pub mod registry_keys;

pub mod block_definitions;
pub mod block_entities;
/// Immutable hook subscription bitsets, resolved once at installation.
pub mod subscribers;
/// Session-only watered-area indexes, shared by server gameplay and the mod disable boundary.
pub mod water_tickets;
