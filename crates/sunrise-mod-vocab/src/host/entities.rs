//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `entities` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait EntitiesHost {
    /// Side: both.
    /// The entity `id` (the id events carry) as a handle for the `entity_*`
    /// methods: None with no world open, or for no such entity in the accessible
    /// sections of the level in place or among the server's players. On a client (API
    /// @API:client_entity_refs@), any entity the server shows this window,
    /// this window's own player included. API 373.
    fn entity(&self, id: u32) -> Option<EntityRef>;
    /// Side: both.
    /// The level `dimension` (`minecraft:the_nether`) as a handle: on a client,
    /// the level it shows, whatever the server calls it, and each level the
    /// server opened for viewing (`sunrise:levels`) while that view is open. A
    /// viewed level's handle answers its block reads from its own world:
    /// `block_state`, `block`, `block_property`, `clip`, `visit_block_collisions`,
    /// `has_collisions`, and the fluid reads built on `block_state`
    /// (`fluid_shape`, `fluid_height`, `fluid_height_for_camera`,
    /// `fluid_can_be_replaced_with`). `get_enchant_power_bonus` uses the viewed level as context.
    /// Its other reads, its entity reads and
    /// every write refuse it. It answers None while the server has the level
    /// closed; a close and a later open within the same client level make the
    /// old handle read again. A login, a reconfiguration or a respawn into
    /// another level replaces the client level: the old handle stays stale and
    /// the mod asks `host.level` again. None with no world open, or for a
    /// dimension the server hasn't loaded. In `client_level_load`, the new level
    /// resolves by the event's dimension even before it is installed; in
    /// `client_level_unload`, the retiring level still resolves. API 373.
    fn level(&self, dimension: &str) -> Option<LevelRef>;
}
/// The server host's methods of the `entities` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait EntitiesServerHost {
    /// Side: server.
    /// Narrows `entity` to an [`ExperienceOrbRef`] across the active and parked
    /// server levels. Returns `Ok(None)` when the handle is stale, removed, held
    /// out, or names a non-orb entity; returns `Err(LevelReadError::Unavailable)`
    /// on a client or outside the world phase.
    fn experience_orb(&self, entity: EntityRef)
    -> Result<Option<ExperienceOrbRef>, LevelReadError>;
}
/// The handles' reads of the `entities` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntitiesBackend {
    /// Side: server.
    /// The orb's signed points value (`ExperienceOrb.getValue()`, `DATA_VALUE` int).
    /// Works across active and parked server levels. Returns
    /// `Err(LevelReadError::Stale)` for a stale, removed, or held-out orb, and
    /// `Err(LevelReadError::Unavailable)` on a client or outside the world phase.
    fn experience_orb_value(&self, orb: ExperienceOrbRef) -> Result<i32, LevelReadError>;
    /// Side: server.
    /// How many orbs are merged into this one (`ExperienceOrb.count`), each worth
    /// its value. Works across active and parked server levels. Returns
    /// `Err(LevelReadError::Stale)` for a stale, removed, or held-out orb, and
    /// `Err(LevelReadError::Unavailable)` on a client or outside the world phase.
    fn experience_orb_count(&self, orb: ExperienceOrbRef) -> Result<u32, LevelReadError>;
    /// Side: server.
    /// Ticks this orb has lived (`ExperienceOrb.age`, signed int; despawns at 6000).
    /// Works across active and parked server levels. Returns
    /// `Err(LevelReadError::Stale)` for a stale, removed, or held-out orb, and
    /// `Err(LevelReadError::Unavailable)` on a client or outside the world phase.
    fn experience_orb_age(&self, orb: ExperienceOrbRef) -> Result<i32, LevelReadError>;
    /// Side: both.
    /// Where this entity is now, as its Part state ([`EntityPartState`]) checks it:
    /// resident in the level this host is in, out of its lists for its own tick, resident in
    /// another level, or gone (removed or its removal pending, unloaded, moved under a new
    /// id, or a handle of another world open). On a client, in place for an entity the
    /// client has in the level it shows.
    fn entity_residency(&self, entity: EntityRef) -> EntityResidency;
}
/// The handles' writes of the `entities` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntitiesBackendMut {
    /// Side: server.
    /// Writes the orb's signed points value (`ExperienceOrb.DATA_VALUE`, full `i32`).
    /// Saving truncates `Value` to `(short)` as vanilla `ExperienceOrb.addAdditionalSaveData`
    /// does. In `experience_orb_merge`, writing `into`'s value makes that write the
    /// merged orb. Works across active and parked server levels.
    fn experience_orb_set_value(
        &mut self,
        orb: ExperienceOrbRef,
        value: i32,
    ) -> Result<(), ExperienceOrbWriteError>;
    /// Side: server.
    /// Writes how many orbs are merged into this one (`ExperienceOrb.count`).
    /// Rejects `0` or values above `i32::MAX` (`ExtraCodecs.POSITIVE_INT`) with
    /// `Err(ExperienceOrbWriteError::InvalidCount)` without mutating the orb. In
    /// `experience_orb_merge`, writing `into`'s count makes that write the merged orb.
    /// Works across active and parked server levels.
    fn experience_orb_set_count(
        &mut self,
        orb: ExperienceOrbRef,
        count: u32,
    ) -> Result<(), ExperienceOrbWriteError>;
    /// Side: server.
    /// Writes the ticks this orb has lived (`ExperienceOrb.age`, full signed `i32`;
    /// despawns at 6000). Saving truncates `Age` to `(short)` as vanilla
    /// `ExperienceOrb.addAdditionalSaveData` does. Works across active and parked
    /// server levels.
    fn experience_orb_set_age(
        &mut self,
        orb: ExperienceOrbRef,
        age: i32,
    ) -> Result<(), ExperienceOrbWriteError>;
}
/// The handles' client writes of the `entities` area, implemented by the game and reached through [`ClientBackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntitiesClientBackendMut {
    /// Side: client.
    /// Spawn sprint particles when called explicitly by a client mod. A
    /// valid entity returns true even if the block beneath it is invisible.
    /// Returns false for stale, untracked, server-side, or unsupported entities.
    fn entity_sprint_particles(&mut self, entity: EntityRef) -> bool;
}
