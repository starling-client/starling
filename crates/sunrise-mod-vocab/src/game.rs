//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// Which of a column's heights (`Heightmap.Types`) a run's `y` counts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Heightmap {
    WorldSurface,
    MotionBlocking,
    MotionBlockingNoLeaves,
    OceanFloor,
}
/// A run: `height` blocks up a column from `y` (from the column's `surface` height when one is
/// given), set to palette entry `block`, only where filter `only` matches when one is given.
// no wire: a run indexes the same in-process bulk edit's palette and filters.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct BlockRun {
    pub x: i32,
    pub z: i32,
    pub y: i32,
    pub height: u32,
    pub surface: Option<Heightmap>,
    pub block: u32,
    pub only: Option<u32>,
}
/// A bulk edit of complete typed states and partial predicates, named by run indices; NBT is not supported.
// no wire: bulk edits use native block states and predicates directly.
#[derive(Clone, Debug, PartialEq, Default)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct BlockEdit {
    /// Complete states to write; properties are never parsed from command strings.
    pub palette: Vec<BlockState>,
    /// Validated block or tag predicates; omitted properties are wildcards. A run can address
    /// any existing filter at its `only` index.
    pub filters: Vec<StatePredicate>,
    pub runs: Vec<BlockRun>,
}
/// `PlayerSpawnPhantomsEvent.Result`: whether the phantom spawner tries a spawn near a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PhantomSpawnResult {
    /// Always try, skipping the skylight and sea level check.
    Allow,
    /// Try only above sea level with the sky in view.
    Default,
    /// Never try.
    Deny,
}
/// `Player.BedSleepingProblem` as `CanContinueSleepingEvent.getProblem` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SleepProblem {
    /// The sleeper's bed is gone.
    NotPossibleHere,
    /// The bed rule no longer lets a player sleep: it is daytime.
    NotPossibleNow,
}
/// The world's clock as a client mod reads it (`clock`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Clock {
    /// Ticks since the world was created (`Level.getGameTime`).
    pub game_time: i64,
    /// The time of day in ticks (`Level.getOverworldClockTime`): 0 at sunrise, 24000 a
    /// day. It keeps counting past 24000 (`/time add`, days going by), so take
    /// it modulo 24000 for the hour; `/time set` can make it negative.
    pub day_time: i64,
}
/// The client's tick rate state (`ClientLevel.tickRateManager()`), as the server last
/// synchronised it (`ClientboundTickingStatePacket`, `ClientboundTickingStepPacket`).
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientTickRate {
    /// Ticks a second (`TickRateManager.tickrate`): 20 unless `/tick rate` changed it.
    pub tick_rate: f32,
    /// Whether the game is frozen (`TickRateManager.isFrozen`, `/tick freeze`).
    pub frozen: bool,
    /// The steps left to run while frozen (`TickRateManager.frozenTicksToRun`, `/tick step`).
    pub frozen_ticks_to_run: i32,
}

/// The generator's undecorated surface, without slope rules or chunk allocation.
/// Created only by the game.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TerrainColumn {
    /// One above the highest solid block, skipping fluids (OceanFloor).
    pub height: i32,
    /// The block at height minus one.
    pub block: BlockKey,
    /// One above the highest fluid over the surface, absent on dry land.
    pub water: Option<i32>,
    /// Identifier from the world's biome table, including data pack biomes.
    pub biome: String,
}

/// One loaded column's heightmap base, blocks and biome water colour, borrowed for the
/// duration of a [`LevelRef::visit_columns`] callback.
///
/// Colormap-derived grass and foliage tints are client-side only: read blended tints with
/// [`BlockAndTintGetter::block_tint`] inside block tint callbacks or `ClientHost::biome_tint`
/// in other client hooks.
// no wire: borrowed call-scoped view of a streamed column read; never encoded over the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColumnSlice<'a> {
    /// One above the highest block on the requested heightmap.
    pub base: i32,
    /// Complete block states from `base - below` up to `base + above - 1`, bottom first
    /// (air outside the chunk's vertical range).
    pub blocks: &'a [BlockState],
    /// Server-side biome JSON water colour (`BiomeSpecialEffects.waterColor`, `0xRRGGBB`) at
    /// the column's top block (`base - 1`).
    pub water_color: u32,
}

impl<'a> ColumnSlice<'a> {
    /// Construct a borrowed column slice for a streamed read callback.
    pub const fn new(base: i32, blocks: &'a [BlockState], water_color: u32) -> Self {
        Self {
            base,
            blocks,
            water_color,
        }
    }
}
/// The vanilla data type of the root compound passed to `Host::upgrade_nbt`.
// exhaustive: only these vanilla root types are exposed by the standalone upgrader.
// no wire: selects an in-process data fixer, not a field of the saved data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NbtDataType {
    /// `References.ITEM_STACK`: one item stack, including its saved components.
    ItemStack,
    /// `References.BLOCK_STATE`: one block name and its saved state properties.
    BlockState,
    /// `References.BLOCK_ENTITY`: one block entity, including its saved contents.
    BlockEntity,
    /// `References.ENTITY`: one entity, not its riding tree.
    Entity,
    /// `References.ENTITY_TREE`: an entity and the riders or passengers saved with it.
    EntityTree,
}

/// Why `Host::upgrade_nbt` returned no upgraded value. Depth is counted as the data is
/// encoded (the root compound at 0): the compound the NBT writer puts around an element
/// of a mixed list is a level of its own. There is no size refusal.
// no wire: returned locally by the NBT upgrade API, not encoded with the NBT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NbtUpgradeError {
    /// The data version is newer than the game's: the fixes only go forward. Refused before any fix runs.
    FutureVersion(i32),
    /// The compound lacks the selected type's readable `id`, or the fixers cannot read it.
    /// A block state names its block in `Name` when saved before data version 5006 and in
    /// `id` from 5006 on, where vanilla's `BlockStateFieldNamesFix` renames the field. This
    /// is not a registry check.
    MalformedCompound,
    /// The legacy fixers' tables could not be read from the prepared game resources.
    LegacyTablesUnavailable,
    /// The data given has a list or compound 512 levels deep or more, the depth cap of every NBT read. Refused
    /// before any fix runs.
    TooDeep,
    /// The data given was within the cap, but the fixes made it deeper (1.9's `Riding` to `Passengers` doubles a
    /// chain's depth) and the upgraded data has a list or compound 512 levels deep or more: no NBT read, vanilla's
    /// included, could take it back. Nothing is returned.
    UpgradedTooDeep,
}
impl std::fmt::Display for NbtUpgradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FutureVersion(v) => write!(f, "data version {v} is newer than the game's"),
            Self::MalformedCompound => f.write_str("malformed saved compound"),
            Self::LegacyTablesUnavailable => {
                f.write_str("could not load the legacy datafix tables")
            }
            Self::TooDeep => f.write_str("the data nests 512 levels deep or more"),
            Self::UpgradedTooDeep => f.write_str("the upgraded data nests 512 levels deep or more"),
        }
    }
}
impl std::error::Error for NbtUpgradeError {}
/// Which side reads a config and when (NeoForge's `ModConfig.Type`).
// no wire: the in-process config loader uses this to select a file and timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConfigKind {
    /// The window only, when the game starts.
    Client,
    /// Once per process: the window when the game starts, or a dedicated
    /// server when it starts. A single player's server thread shares the
    /// window's.
    Common,
    /// The server, before `server_about_to_start`, for each world it
    /// opens: the world's `serverconfig/<file>` when there is one, else
    /// `config/<file>`.
    Server,
}
impl ConfigKind {
    /// The default file name's suffix, as NeoForge's.
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Common => "common",
            Self::Server => "server",
        }
    }
}
/// A data map type a mod registers (`RegisterDataMapTypesEvent`).
// no wire: local registration metadata, not the data map's file contents.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct DataMapType {
    /// A namespaced identifier (`namespace:path`). Its files are
    /// `data/<namespace>/data_maps/<registry folder>/<path>.json`, the
    /// registry's own path for a `minecraft` registry, `<namespace>/<path>`
    /// otherwise.
    pub id: String,
    /// The registry it is for: any the game names (`minecraft:item`, `minecraft:entity_type`...).
    pub registry: String,
}
/// A mod the world was saved with that differs from the loaded mods
/// (`ModMismatchEvent`'s `MismatchedVersionInfo`).
// no wire: a local report from comparing saved and loaded mod versions.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ModMismatch {
    /// The mod's `[mod] id`.
    pub mod_id: String,
    /// Its version when the world was saved; `""` when it wasn't known.
    pub saved_version: String,
    /// Its version now; `None` when the mod is no longer loaded.
    pub current_version: Option<String>,
}
/// What a mod's falling block does when it lands and when it hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Falling {
    /// `FallingBlockEntity.dropItem`: whether it drops as an item where it
    /// can't be placed.
    pub drop_item: bool,
    /// `FallingBlockEntity.hurtEntities`, at 2 a block fallen and at most 40,
    /// an anvil's.
    pub hurts: bool,
}
impl Default for Falling {
    fn default() -> Self {
        Self {
            drop_item: true,
            hurts: false,
        }
    }
}
/// The block a ray along the view meets.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Hit {
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
    /// Where the ray meets the block.
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The block's resolved identity, without state properties.
    pub block: BlockKey,
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A map as a native mod sees it when `map_decorations` is asked.
// Native mods read it; nothing in Sunrise does.
#[derive(Clone, Copy, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MapInfo<'a> {
    /// The map's id (`map_<id>`).
    pub id: i32,
    /// The map's centre block `[x, z]`.
    pub center: [i32; 2],
    /// `MapItemSavedData.scale`: a map pixel is `1 << scale` blocks.
    pub scale: u8,
    /// The dimension the map shows (`minecraft:overworld`).
    pub dimension: &'a str,
    pub locked: bool,
    /// Whether it is drawn in an item frame, not held or on the cartography screen.
    pub in_frame: bool,
}
/// An icon a native mod puts on a map: the same place and pose as vanilla's
/// decorations (`MapDecoration`).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct MapDecoration {
    /// An atlas sprite: vanilla's `map/decorations/<name>`, or a content
    /// mod's texture. An unknown sprite is left out.
    pub sprite: String,
    /// The position on the map in half map pixels, -128 (left, top) to 127.
    /// For the world position `(wx, wz)`: `(wx - center_x) / (1 << scale) * 2`,
    /// and the same for z, floored and clamped.
    pub x: i8,
    pub y: i8,
    /// 0 to 15, in 22.5 degree steps clockwise (only the low four bits count).
    pub rot: u8,
    /// Whether it shows in an item frame and on the cartography screen
    /// (`MapDecorationType.showOnItemFrame`); a held map shows all.
    pub shown_on_frame: bool,
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// One mob's frame, as `EntityRenderer.extractRenderState` fills a
/// `LivingEntityRenderState`: what a model reads to pose and animate itself.
/// Angles are in degrees, interpolated to the frame's partial tick.
// Native mods read it; nothing in Sunrise does.
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MobRender<'a> {
    /// The mob's network id, as the mob helpers and mod messages name it.
    pub id: u32,
    /// Its entity type (`minecraft:villager`).
    pub kind: &'a str,
    /// Its `CustomName` as plain text, when it has one.
    pub custom_name: Option<&'a str>,
    /// `isBaby`.
    pub baby: bool,
    /// The frame's partial tick, 0 to 1.
    pub partial: f32,
    /// `ageInTicks`: ticks lived plus the partial tick, for idle motion.
    pub age: f32,
    /// `bodyRot`: the way the body faces (0 is south, 90 west).
    pub body_yaw: f32,
    /// `yRot`: the head's turn from the body, -180 to 180.
    pub head_yaw: f32,
    /// `xRot`: the head's pitch, positive looking down.
    pub pitch: f32,
    /// `walkAnimationPos`: how far the legs have swung, for a walk cycle
    /// (a vanilla quadruped's legs swing `cos(pos * 0.6662) * 1.4 * speed` radians).
    pub walk_position: f32,
    /// `walkAnimationSpeed`: how fast it walks, 0 standing to about 1.
    pub walk_speed: f32,
    /// Ticks left of the red hurt flash (10 when hit).
    pub hurt_time: i32,
    /// Ticks since it died, 0 while alive (vanilla tips the body over 20).
    pub death_time: i32,
    pub on_ground: bool,
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// Which player a [`MobRender`] is, for `render_player_pre` and
/// `render_player_post`.
// Native mods read it; nothing in Sunrise does.
#[derive(Clone, Copy, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct PlayerRender<'a> {
    /// The player's profile name.
    pub name: &'a str,
    /// Whether it is the player of this window, drawn in third person.
    pub local: bool,
}
// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// One face of a `native_draw_cube` box: its sprite (`mymod:entity/cog`,
/// `mymod:block/casing`) and its UVs in texels of 16 per side, as a model
/// face's `uv` (none takes the box's own, as a model's default).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct CubeFace<'a> {
    pub sprite: &'a str,
    pub uv: Option<[f32; 4]>,
}
impl<'a> CubeFace<'a> {
    /// A face of `sprite` with the box's own UVs.
    pub const fn new(sprite: &'a str) -> Self {
        Self { sprite, uv: None }
    }
    /// The same face with UVs `uv`, `[u0, v0, u1, v1]` in texels of 16 per side.
    pub const fn with_uv(self, uv: [f32; 4]) -> Self {
        Self {
            uv: Some(uv),
            ..self
        }
    }
}

#[doc(hidden)]
pub(crate) mod built {
    #[allow(unused_imports)]
    use super::*;
    // exhaustive: the game builds it with a literal; mods only read it
    #[derive(sunrise_mod_api_macros::Wire)]
    pub struct TerrainColumn {
        pub height: i32,
        pub block: BlockKey,
        pub water: Option<i32>,
        pub biome: String,
    }
    impl From<TerrainColumn> for super::TerrainColumn {
        fn from(v: TerrainColumn) -> Self {
            Self {
                height: v.height,
                block: v.block,
                water: v.water,
                biome: v.biome,
            }
        }
    }
}

/// Saved-world resolution records and scoped `ModMismatchEvent` operations.
#[path = "mismatch_resolution.rs"]
mod mismatch_resolution;
pub use mismatch_resolution::ModMismatchResolution;
