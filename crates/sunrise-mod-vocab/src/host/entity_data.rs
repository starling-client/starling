//! The `entity_data` area: an entity's tags and saved data. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_data` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityDataBackend {
    /// Side: server.
    /// Borrow the entity's scoreboard tag set (`Entity.entityTags()`).
    /// An entity without tags has an [`empty view`](EntityTags::is_empty). A stale handle
    /// or unavailable context returns `None`. Use [`EntityTags::contains`] for an exact
    /// match, or [`EntityTags::iter_utf16`] to read tags in saved order and choose how
    /// to convert them to UTF-8.
    fn entity_tags(&self, entity: EntityRef) -> Option<EntityTags<'_>>;
}
/// The handles' writes of the `entity_data` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityDataBackendMut {
    /// Side: server.
    /// The entity's data as `/data get entity` shows it
    /// (`Entity.saveWithoutId`): its compound without `id`. An entity with no
    /// UUID yet gets the one it will be saved with. API 386.
    fn entity_nbt(&mut self, entity: EntityRef) -> Option<Compound>;
    /// Side: server.
    /// Merges `nbt` into the entity's data and loads the entity from it, as `/data merge
    /// entity` does (`EntityDataAccessor.setData`): true when it changed. False for a player
    /// (vanilla refuses them too), a kind Sunrise cannot reload yet (end crystals, the dragon
    /// and its parts, cushions, ominous item spawners, area effect clouds, fishing hooks, leash
    /// knots and general entity rows), data nested too deep, or data that changes nothing. An
    /// arrow, spectral arrow or trident reloads as `AbstractArrow.readAdditionalSaveData` reads
    /// it (`life`, `inGround`, `pickup`, ...); other projectiles take only their position,
    /// motion, rotation and flags. API 386.
    fn entity_merge_nbt(&mut self, entity: EntityRef, nbt: Compound) -> bool;
    /// Side: server.
    /// Adds tag `tag` (`Entity.addTag`): false when it has it already, holds
    /// 1024, its kind cannot store tags, or the handle is stale. API 386.
    fn entity_add_tag(&mut self, entity: EntityRef, tag: &str) -> bool;
    /// Side: server.
    /// Removes tag `tag` (`Entity.removeTag`): false when it didn't have it.
    /// API 386.
    fn entity_remove_tag(&mut self, entity: EntityRef, tag: &str) -> bool;
}
