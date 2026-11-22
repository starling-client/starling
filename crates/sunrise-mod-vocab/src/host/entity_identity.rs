//! The `entity_identity` area: what an entity is: its name, type, level and UUID, and the players it is sent to. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `entity_identity` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityIdentityBackend {
    /// Side: both.
    /// The entity's name as plain text (`Entity.getName`): its custom name,
    /// a player's name, else its type's name. On a client (API
    /// @API:client_entity_refs@), from the custom name the server synced
    /// (`Entity.DATA_CUSTOM_NAME`) and a player's profile name. API
    /// 386.
    fn entity_name(&self, entity: EntityRef) -> Option<String>;
    /// Side: server.
    /// Whether the entity's name shows over it at any distance
    /// (`Entity.isCustomNameVisible`); false for the kinds that keep no name.
    fn entity_custom_name_visible(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The category of the entity's type (`EntityType.getCategory`): a mob's
    /// from its type, `Monster` for the dragon, `Misc` for every other kind.
    fn entity_classification(&self, entity: EntityRef) -> Option<MobCategory>;
    /// Side: server.
    /// The players this entity is sent to (`ChunkMap.TrackedEntity.seenBy`), in the order the
    /// server keeps its players: those in the entity's level whose client has the entity from
    /// its tracking pass. A player is not among its own trackers. Fake players track nothing.
    /// None on a client, and for a stale handle or an entity that is not in the level in place.
    fn entity_trackers(&self, entity: EntityRef) -> Option<Vec<PlayerRef>>;
    /// Side: server.
    /// The entity's resolved type key (`Entity.getType`). Compare keys directly;
    /// request `key.id(host)` explicitly when its canonical name is needed.
    /// A general row returns its declared type. This read,
    /// `entity_uuid` and `entity_level` also answer for a mob a kill credit
    /// names in another loaded level. API 373.
    fn entity_kind(&self, entity: EntityRef) -> Option<EntityTypeKey>;
    /// Side: server.
    /// The level the entity is in (`Entity.level`): on a client, the
    /// client's level. API 373.
    fn entity_level(&self, entity: EntityRef) -> Option<LevelRef>;
    /// Side: server.
    /// The entity that fired it (`TraceableEntity.getOwner`), as a resolved
    /// entity: a projectile's owner (`Projectile.getOwner`; a trident a player
    /// threw belongs to that player) or a general row's, as
    /// [`LevelRef::spawn_entity`] set it. None for an entity with no owner or
    /// of a kind that has none, an owner that is gone or in another level, a
    /// client, or a stale handle.
    fn entity_owner(&self, entity: EntityRef) -> Option<EntityRef>;
    /// Side: both.
    /// Whether the entity is made of parts that are entities of their own
    /// (`Entity.isMultipartEntity`): the ender dragon. Its parts are not the
    /// mod API's behaviour Parts. A part is not itself multipart. None for a
    /// stale handle.
    fn entity_is_multipart(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// The entities a multipart entity is made of (`Entity.getParts`), in
    /// `getSubEntities` order: for the dragon its head, neck, body, three
    /// tail segments and two wings. Each is a handle of the part's own entity
    /// id, the eight after the dragon's, with the dragon's type, its own
    /// position, rotation, velocity and box, and no living state; the
    /// handle stops resolving with the dragon. Empty for an entity that is not
    /// multipart, None for a stale handle.
    fn entity_parts(&self, entity: EntityRef) -> Option<Vec<EntityRef>>;
    /// Side: server.
    /// `Entity.getSoundSource`: the group an entity's sounds play under.
    /// Players are `Players`; `Monster`s, the dragon, hoglins, ghasts,
    /// phantoms, shulkers, slimes and a killer bunny are `Hostile`; dropped
    /// items and experience orbs `Ambient`; every other entity `Neutral`.
    fn entity_sound_source(&self, entity: EntityRef) -> Option<SoundSource>;
}
/// The handles' writes of the `entity_identity` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait EntityIdentityBackendMut {
    /// Side: server.
    /// Names the entity (`Entity.setCustomName`), or takes its name away
    /// with `None`: its name, its saved `CustomName` and, for a mob or an
    /// armor stand, what the players tracking it are sent. False for a
    /// player, a kind whose name Sunrise does not retain (projectiles, the
    /// dragon and crystals, primed TNT, falling blocks, experience orbs,
    /// ominous item spawners), a stale handle or on a client.
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn entity_set_custom_name(
        &mut self,
        entity: EntityRef,
        name: Option<Text>,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Shows the entity's name at any distance, or only when looked at
    /// (`Entity.setCustomNameVisible`): saved, and sent to the players
    /// tracking a mob or an armor stand. False where `set_custom_name` is.
    fn entity_set_custom_name_visible(&mut self, entity: EntityRef, visible: bool) -> bool;
    /// Side: server.
    /// The entity's UUID (`Entity.getUUID`), the one it is saved with; an
    /// entity that has none yet gets it now, as a new vanilla entity does.
    /// On a client, the one the server added it with (a player's is its
    /// profile's). API 373.
    fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128>;
}
