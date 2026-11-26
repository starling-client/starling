//! The `level_dimension` area: the dimension a level is: its key, type, height, bounds and passages. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_dimension` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelDimensionBackend {
    /// Side: both.
    /// The dimension's logical height (`ServerLevel.getLogicalHeight`), which may differ
    /// from build height. Reads active and parked server levels or the client's current level.
    /// None for a stale or unavailable level. Does not load chunks.
    fn level_logical_height(&self, level: LevelRef) -> Option<i32>;
    /// Side: both.
    /// Borrows the canonical dimension id. `None` if the host has no backend or
    /// the slot was never issued.
    fn dimension_key_id(&self, key: DimensionKey) -> Option<&str>;
    /// Side: both.
    /// The lowest block `y` and the height of the level
    /// (`LevelHeightAccessor.getMinY`, `getHeight`). Reads any loaded server level
    /// or the client's current level. None for a stale handle, before client login,
    /// or for a dimension unavailable on this side. Does not load chunks. API 381.
    fn level_height(&self, level: LevelRef) -> Option<(i32, i32)>;
    /// Side: both.
    /// `Level.isInWorldBounds`: `pos` within 30,000,000 blocks on x and z and inside the level's
    /// build height. None as for `native_level_height`. Does not load chunks.
    fn level_in_world_bounds(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: both.
    /// `Level.isInSpawnableBounds`: 30,000,000 blocks on x and z, 20,000,000 on y. None as for
    /// `native_level_height`. Does not load chunks.
    fn level_in_spawnable_bounds(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: both.
    /// Dimension shape of a loaded server level, including parked levels.
    fn level_dimension_shape(&self, level: LevelRef) -> Option<LevelDimensionShape>;
    /// Side: both.
    /// The level's dimension type (`Level.dimensionType()` and its gameplay attributes): coordinate
    /// scale, ambient light, fixed time, infiniburn, monster spawn light, sky and the gameplay
    /// switches a mod's `DimensionSettings` or a data pack sets. A level whose type neither gave
    /// fields for has the Overworld's. None for a stale handle; on a client, the level it shows.
    fn level_dimension_type(&self, level: LevelRef) -> Option<LevelDimensionType<'_>>;
    /// Side: server.
    /// `Level.getSeaLevel`: 63 in the Overworld, 32 in the Nether, 0 in the End; for a level of the
    /// noise generator the `sea_level` of its noise settings (a data pack's too, when it replaces
    /// them), for another level the `sea_level` its `DimensionSettings` gave, else 63. It is the sea
    /// level the game uses for the level: terrain and aquifers, mob spawning and mob behaviour, rain
    /// and snow, and maps. None for a stale handle and on a client.
    fn level_sea_level(&self, level: LevelRef) -> Option<i32>;
    /// Side: server.
    /// `ServerLevel.isVillage`: whether `pos` is within one section of a section
    /// with an occupied village point of interest. Uses the same village centers
    /// as raids. Returns `Stale` for a stale level, or `Unavailable` if the level
    /// is not in place or the position is unloaded or outside build height.
    /// Does not open levels, load chunks, or take tickets.
    fn level_is_village(&self, level: LevelRef, pos: [i32; 3]) -> Result<bool, LevelReadError>;
    /// Side: server.
    /// Where an entity of box `size` (width, height), not a player, centred in the portal block
    /// at `portal_pos` of this level arrives through it
    /// (`getPortalDestination`), changing nothing. Nether portals search the closest portal
    /// around the scaled, border-clamped exit; End portals answer the End's fixed spawn point or
    /// the Overworld's shared spawn; End gateways answer their stored exit. No platform, ticket,
    /// sound or hook. Diverges from vanilla, which loads chunks: loaded state only, and
    /// `ChunksNotLoaded` for a level not open or a chunk of the search (16 blocks into the
    /// Nether, 128 out, plus a chunk) not loaded.
    fn level_portal_destination(
        &self,
        level: LevelRef,
        portal_pos: [i32; 3],
        size: [f64; 2],
    ) -> Result<PortalExit, PortalError>;
}
/// The handles' writes of the `level_dimension` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelDimensionBackendMut {
    /// Side: server.
    /// Registers `passage` in the level ([`Passage`], #3775) and numbers it. The engine
    /// tells it to every Sunrise client holding its chunk ([`Passage::chunk`]) at the end of
    /// the server's tick, and to a client that gets the chunk later as the chunk is sent
    /// (`chunk_sent`); a client forgets it with the chunk. Passages are not saved and a
    /// datapack reload does not bring them back: they hold until `remove_passage`, the
    /// level's close or the world's, so a mod registers its own again in `level_load`. An
    /// id is never given again while the game runs. A level holds 256 at most and a chunk 8.
    /// A passage removed before the tick's end is never told. Refused with nothing
    /// registered for a stale handle, a client, a passage that isn't a finite rectangle
    /// with unit axes at a right angle and sides above 0 and at most 32 blocks, one with a
    /// corner past vanilla's position limits, a full level or chunk, and once every id was
    /// given.
    fn level_register_passage(
        &mut self,
        level: LevelRef,
        passage: Passage,
    ) -> Result<PassageId, PassageRegistrationError>;
    /// Side: server.
    /// Removes the level's passage `id`: the clients that were told of it are told it is
    /// gone at the end of the server's tick. True when it was registered in this level.
    /// False for another level's or an unknown id, a stale handle and a client.
    fn level_remove_passage(&mut self, level: LevelRef, id: PassageId) -> bool;
    /// Side: server.
    /// `LevelRef::portal_destination`, then the exit portal built by `PortalForcer.createPortal`
    /// (the spiral search, obsidian frame and the entry's axis) when none was found, with the
    /// `portal_create` hook told as for travel, which cannot veto it. Refused inside a transaction
    /// (`InTransaction`): a built portal cannot be rolled back. Needs the chunks around the
    /// built exit loaded (`ChunksNotLoaded`); opens no level. `NoExit` when none was found and
    /// none could be built.
    fn level_find_or_create_portal_exit(
        &mut self,
        level: LevelRef,
        portal_pos: [i32; 3],
        size: [f64; 2],
    ) -> Result<PortalExit, PortalError>;
}
