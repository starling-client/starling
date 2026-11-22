//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `entities` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait EntitiesHost {
    /// Side: both.
    /// The entity `id` (the id events carry) as a handle for the `entity_*`
    /// methods: None with no world open, or for no such entity in the level
    /// in place or among the server's players. On a client (API
    /// @API:client_entity_refs@), any entity the server shows this window,
    /// this window's own player included. API 373.
    fn entity(&self, id: u32) -> Option<EntityRef>;
    /// Side: both.
    /// Where the entity `entity` names is now, as its Part state (`EntityPartState`) checks it:
    /// resident in the level this host is in, out of its lists for its own tick, resident in
    /// another level, or gone (removed or its removal pending, unloaded, moved under a new
    /// id, or a handle of another world open). On a client, in place for an entity the
    /// client has in the level it shows.
    fn entity_residency(&self, entity: EntityRef) -> EntityResidency;
    /// Side: both.
    /// The level `dimension` (`minecraft:the_nether`) as a handle: on a client,
    /// the level it shows, whatever the server calls it, and each level the
    /// server opened for viewing (`sunrise:levels`) while that view is open. A
    /// viewed level's handle answers its block reads from its own world:
    /// `block_state`, `block`, `block_property`, `clip`, `collision_boxes`,
    /// `has_collisions`, and the fluid reads built on `block_state`
    /// (`fluid_shape`, `fluid_height`, `fluid_height_for_camera`,
    /// `fluid_can_be_replaced_with`). `get_enchant_power_bonus` uses the viewed level as context.
    /// Its other reads, its entity reads and
    /// every write refuse it. It answers None while the server has the level
    /// closed; a close and a later open within the same client level make the
    /// old handle read again. A login, a reconfiguration or a respawn into
    /// another level replaces the client level: the old handle stays stale and
    /// the mod asks `host.level` again. None with no world open, or for a
    /// dimension the server hasn't loaded. API 373.
    fn level(&self, dimension: &str) -> Option<LevelRef>;
}
/// The server host's methods of the `entities` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait EntitiesServerHost {
    /// Side: server.
    /// Finds a loaded entity by UUID in the server level in place for this
    /// hook. Returns `None` for unloaded entities, entities in another level,
    /// and dragon parts. It also returns `None` for entities held out of
    /// lookup, taken from their resident storage, unavailable under a scoped
    /// item loan, queued for removal, or vehicles undergoing destruction.
    /// The lookup leaves entity and level state alone.
    fn entity_by_uuid(&self, uuid: EntityUuid) -> Option<EntityRef>;
    /// Side: server.
    /// Spawns an entity of `kind` (`minecraft:item_display`) at `pos` in the
    /// level in place, `dimension`, from `nbt` (its compound, as `/summon
    /// <kind> <pos> <nbt>` takes it; passengers included): its id. None on a
    /// client's copy of the world, for another dimension, a position outside
    /// `/summon`'s bounds, a chunk that isn't loaded, a kind Sunrise can't
    /// load, or one a mod kept out (`entity_join_level`). Inside a transaction its join waits for
    /// the root commit, so a mod keeping it out is not known at the call: the id is returned and
    /// names nothing from the commit on.
    fn summon(&mut self, dimension: &str, kind: &str, pos: [f64; 3], nbt: Compound) -> Option<u32>;
    /// Side: server.
    /// Writes the experience orb `id`'s `value` (the points a pickup gives), `count`
    /// (orbs merged into it, each worth `value`) and `age` (ticks lived; an orb
    /// despawns at 6000); a `None` is left as it is. Signed values are preserved,
    /// including zero and negative values.
    /// False, and nothing written, for an orb not in the server level `dimension`,
    /// a `count` of 0 or above `i32::MAX`, an `age` above `i32::MAX` (`ExperienceOrb.age`
    /// is an int), a `value` outside -32768..=32767 (a save
    /// keeps `Value` as a short and `Count` as an int, so a write it accepts reloads
    /// unchanged), and on a client of a server. In `experience_orb_merge`, a write to
    /// `into` is the merged orb.
    fn set_experience_orb(
        &mut self,
        dimension: &str,
        id: u32,
        value: Option<i32>,
        count: Option<u32>,
        age: Option<u32>,
    ) -> bool;
    /// Side: server.
    /// The experience orbs of the server level `dimension` whose box meets
    /// the box `min`..`max`, in the level's order: what a mod reads before it
    /// cancels an orb in `entity_join_level`, to give its points with
    /// `PlayerRef::give_experience_points`. Empty for a level not in place, on a
    /// client of a server and for a bound that is not finite (NaN or infinite); a
    /// minimum above its maximum is the same box with the two swapped (`AABB`).
    fn experience_orbs(&self, dimension: &str, min: [f64; 3], max: [f64; 3]) -> Vec<ExperienceOrb>;
    /// Side: server.
    /// Throws a falling block of `state` from `pos` (its bottom centre) at
    /// `velocity` (blocks a tick) in the level in place, `dimension`: it
    /// falls, lands and places itself as sand does, or drops as an item
    /// where it can't. The block at `pos` is left as it is: take it away first
    /// with `native_set_block` to make a block fall out of the world. False,
    /// with nothing thrown, on a client's copy of the world, for another
    /// dimension, an air or unknown state, a chunk that isn't loaded, a value
    /// that isn't finite, or with 1024 falling blocks in the level already.
    /// Each axis of `velocity` is cut to 3.9.
    fn spawn_falling_block(
        &mut self,
        dimension: &str,
        state: BlockState,
        pos: [f64; 3],
        velocity: [f64; 3],
        falling: Falling,
    ) -> bool;
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
