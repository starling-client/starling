//! World persistence in the vanilla 26.3 layout, readable by both Sunrise and
//! the vanilla game:
//! - `level.dat` (`Data`: name, game type, time, spawn, difficulty, version)
//!   plus the saved-data files `data/minecraft/{weather,world_clocks,
//!   world_gen_settings,game_rules}.dat`;
//! - the player in `players/data/<uuid>.dat` (`singleplayer_uuid`);
//! - per dimension `dimensions/minecraft/<name>/region/r.X.Z.mca` (chunks:
//!   paletted sections with light, heightmaps, block entities, scheduled
//!   block/fluid ticks) and `entities/r.X.Z.mca` (entities, `entity_nbt`).
//!
//! Data Sunrise does not simulate (structure references, unknown block
//! entities and entities, player fields such as the ender chest or effects,
//! game rules) is kept from the loaded files and written back unchanged.
//! Worlds saved by older Sunrise builds (`region/`, `DIM-1/`, `Player` in
//! `level.dat`) are migrated on open.
use crate::mod_chunk_tickets::{ModTicket, ModTickets, TicketOwner};
mod backpressure;
use compression_pool::{CompressedWrite, compress_records};
pub mod biome_regions;
pub mod block_entity_encode;
mod block_entity_order;
mod far_regions;
mod horizon_names;
pub(crate) use horizon_names::adopt_in_player as adopt_horizon_player;
#[cfg(test)]
mod horizon_names_tests;
pub mod level_mods;
mod respawn;
mod writes;
use writes::{Outstanding, Writer};

use crate::{
    Result,
    beehive::BeehiveExt,
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, ColumnLayout, Light, Section, block_strategy},
    mobs::{spawner::SpawnerExt, trial_spawner::TrialSpawnerExt},
    streaming::ChunkGenerator,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
};
use sunrise_core::{
    paletted::PalettedContainer,
    position::{BlockPos, ChunkPos},
    ticks::{SavedTick, TickPriority, TickTypeId},
};
use sunrise_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound};
use sunrise_world::worldgen::blending::BlendingData;
use {
    sunrise_core::{
        nbt,
        nbt::{Compound, JavaString, Limits, NamedTag, Tag},
    },
    sunrise_world::{
        DATA_VERSION,
        chunk::{BlockState, block_entity_pos},
        chunk_status::ChunkStatus,
        compression::Compression,
        heightmap::{Heightmap, HeightmapChunk, HeightmapState, HeightmapType},
        region::{ChunkRecord, RegionFile},
        registry::{BlockRegistry, BlockStateId},
        worldgen::terrain::TerrainBlockStates,
    },
};

pub const VERSION_NAME: &str = sunrise_core::VANILLA_VERSION;

mod chunk_disk;
#[cfg(test)]
mod chunk_disk_tests;
#[cfg(test)]
mod compression_bench;
mod compression_pool;
#[cfg(test)]
mod compression_tests;
mod gateway;
#[cfg(test)]
mod loot_tests;
mod record;
use record::{Encoding, Record};

/// Snapshot live block entities and custom names for a save batch without
/// mutating the world. Each map is visited once, retaining its iteration order
/// within each chunk (including repeated chunk coordinates in the input).
/// An empty batch skips both maps; one chunk uses the direct filter path.
pub fn chunks_with_block_entities<'a, E: std::hash::BuildHasher, N: std::hash::BuildHasher>(
    chunks: impl IntoIterator<Item = &'a Arc<Chunk>>,
    entities: &HashMap<(i32, i32, i32), crate::containers::BlockEntity, E>,
    names: &HashMap<(i32, i32, i32), Tag, N>,
) -> Vec<Chunk> {
    let chunks: Vec<_> = chunks.into_iter().collect();
    if chunks.is_empty() {
        return Vec::new();
    }
    if chunks.len() == 1 {
        let mut c = (**chunks[0]).clone();
        c.block_entities = entities
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, e)| (*p, e.clone()))
            .collect();
        c.block_names = names
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, n)| (*p, n.clone()))
            .collect();
        return vec![c];
    }
    // Index only requested chunks, borrowing their entries so unsaved entities
    // are never cloned. Clone one chunk at a time for inventory cache locality.
    let mut grouped = crate::fast_hash::FxHashMap::<_, (Vec<_>, Vec<_>)>::default();
    grouped.reserve(chunks.len());
    for c in &chunks {
        grouped.entry((c.x, c.z)).or_default();
    }
    for (p, e) in entities {
        if let Some((entries, _)) = grouped.get_mut(&(p.0 >> 4, p.2 >> 4)) {
            entries.push((p, e));
        }
    }
    for (p, n) in names {
        if let Some((_, entries)) = grouped.get_mut(&(p.0 >> 4, p.2 >> 4)) {
            entries.push((p, n));
        }
    }
    chunks
        .into_iter()
        .map(|chunk| {
            let mut c = (**chunk).clone();
            let (entities, names) = &grouped[&(c.x, c.z)];
            c.block_entities = entities.iter().map(|(p, e)| (**p, (*e).clone())).collect();
            c.block_names = names.iter().map(|(p, n)| (**p, (*n).clone())).collect();
            c
        })
        .collect()
}

use sunrise_client_core::nbt_value::string;
pub use sunrise_client_core::nbt_value::{float, number, text};
/// Insert or replace a key of a compound tag.
pub(crate) fn put(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(map) = tag {
        map.insert(key.into(), value);
    }
}
fn compound_or_empty(tag: Option<Tag>) -> Tag {
    match tag {
        Some(t @ Tag::Compound(_)) => t,
        _ => Tag::Compound(Compound::new()),
    }
}

/// `dimensions/<namespace>/<path>` inside a world folder. An id that is no safe folder name
/// (`test:../minecraft/overworld`, an absolute path) never leaves the world: it gets a folder
/// of its own under `dimensions/%invalid/`, named by its bytes, which no valid id can reach.
pub fn dimension_dir(world: &Path, dimension: &str) -> PathBuf {
    let id = if dimension.contains(':') {
        dimension.to_owned()
    } else {
        format!("minecraft:{dimension}")
    };
    let dimensions = world.join("dimensions");
    match id.split_once(':') {
        Some((namespace, path))
            if sunrise_core::level_id::valid_level_namespace(namespace)
                && sunrise_core::level_id::valid_level_path(path) =>
        {
            dimensions.join(namespace).join(path)
        }
        _ => {
            let hex: String = id.bytes().map(|b| format!("{b:02x}")).collect();
            dimensions.join("%invalid").join(hex)
        }
    }
}
/// `DirectoryLock`: `session.lock` holds "\u{2603}" (UTF-16BE) under an exclusive
/// file lock for as long as the world is open, so two instances cannot share it.
pub struct SessionLock(fs::File);
impl Drop for SessionLock {
    /// `DirectoryLock.close`: release the lock, then close the file. Closing
    /// alone keeps the lock while any copy of the descriptor lives on, such as
    /// the one a process spawned on another thread holds until its `exec`.
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
impl SessionLock {
    pub fn acquire(world: &Path) -> Result<Self> {
        use std::io::Write;
        fs::create_dir_all(world)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(world.join("session.lock"))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(
                    format!("{} is already open in another instance", world.display()).into(),
                );
            }
            // Starling: a filesystem without file locks (Android's shared storage): the world
            // opens unlocked, as vanilla's DirectoryLock does where locking is unsupported.
            Err(fs::TryLockError::Error(e)) => log::warn!(
                "Sunrise: {} cannot be locked ({e}); opening it unlocked",
                world.display()
            ),
        }
        file.set_len(0)?;
        file.write_all(&[0x26, 0x03])?;
        file.flush()?;
        Ok(Self(file))
    }
    /// `DirectoryLock.isLocked`: another copy holds the world's `session.lock`.
    /// A world without the file is not locked; one whose file may not be opened
    /// is.
    pub fn is_locked(world: &Path) -> bool {
        let file = match fs::OpenOptions::new()
            .write(true)
            .open(world.join("session.lock"))
        {
            Ok(file) => file,
            Err(e) => return e.kind() == std::io::ErrorKind::PermissionDenied,
        };
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) => true,
            Err(_) => false,
            // Take the probe's own lock back at once, as `SessionLock` does on drop.
            Ok(()) => {
                let _ = file.unlock();
                false
            }
        }
    }
}
/// Bring a world last saved by an older game version to the current format, as
/// 26.3 does when it opens one: the file fixes and level.dat first
/// (`FileFixerUpper`), then the DataFixers upgrades of every chunk, entity chunk,
/// POI chunk, player, saved data, advancements and statistics file. Vanilla runs
/// the latter as it reads each file; Sunrise runs them all on opening, which reads
/// the same data, and region records still older in a world whose level.dat is
/// current are upgraded as they are read (`upgrade_record`).
pub fn upgrade_older_world(world: &Path) -> Result<()> {
    upgrade_older_world_with(
        world,
        &sunrise_world::datafix::progress::UpgradeProgress::new(),
    )
}
/// [`upgrade_older_world`] reporting to `progress`, which `FileFixerProgressScreen` reads.
pub fn upgrade_older_world_with(
    world: &Path,
    progress: &sunrise_world::datafix::progress::UpgradeProgress,
) -> Result<()> {
    // Starling: a world whose level.dat is missing opens from level.dat_old.
    let Some(root) = read_level_dat(world) else {
        return Ok(());
    };
    let version = root.get("Data").map_or(i64::from(DATA_VERSION), |d| {
        number(d.get("DataVersion")).unwrap_or(-1) // saves of 1.8 and before have none
    });
    if version >= i64::from(DATA_VERSION) {
        return Ok(());
    }
    if version < i64::from(sunrise_world::datafix::OLDEST_SUPPORTED) {
        return Err(format!(
            "{} was saved with DataVersion {version}; worlds older than DataVersion {} \
             cannot be upgraded yet (tools/upgrade_world.py runs vanilla's own upgrader)",
            world.display(),
            sunrise_world::datafix::OLDEST_SUPPORTED
        )
        .into());
    }
    sunrise_world::datafix::files::upgrade_world_with(world, progress)
        .map_err(|e| format!("upgrading {}: {e}", world.display()))?;
    Ok(())
}
/// Whether opening the world runs file fixes, which vanilla asks for a backup
/// first (`BackupStatus.FILE_FIXING_REQUIRED`). Saves of 1.8 and before have no
/// DataVersion, read as -1.
pub fn needs_file_fixes(world: &Path) -> bool {
    let Some(root) = read_dat(&world.join("level.dat")) else {
        return false;
    };
    let Some(data) = root.get("Data") else {
        return false;
    };
    let version = number(data.get("DataVersion")).unwrap_or(-1);
    i32::try_from(version).is_ok_and(sunrise_world::datafix::files::requires_file_fixing)
}
/// `LevelVersion.parse`: the saved game version's name, this build's when the `Version`
/// compound has no `Name`, and empty without a `Version` (`getWorldVersionName` then
/// shows `selectWorld.versionUnknown`).
fn saved_version_name(version: Option<&Tag>) -> String {
    match version {
        Some(v) => text(v.get("Name")).unwrap_or_else(|| VERSION_NAME.to_string()),
        None => String::new(),
    }
}
/// `LevelSummary.isCompatible` (`DataVersion.isCompatible`: the saved series must be
/// the current `main` one). None when the world opens; otherwise the saved game
/// version's name, empty when it has none (`getWorldVersionName`).
pub fn incompatible_version(world: &Path) -> Option<String> {
    let root = read_dat(&world.join("level.dat"))?;
    let version = root.get("Data")?.get("Version");
    let series = text(version.and_then(|v| v.get("Series"))).unwrap_or_else(|| "main".into());
    if series == "main" {
        return None;
    }
    Some(saved_version_name(version))
}
/// `LevelSummary.backupStatus` DOWNGRADE: the saved game version's name (empty when it
/// has none) when the world's `Version.Id` is newer than this build's `DataVersion`
/// (`LevelVersion.parse`: no `Version` reads as 0, no `Id` as the current one).
pub fn downgraded_from(world: &Path) -> Option<String> {
    let root = read_dat(&world.join("level.dat"))?;
    let version = root.get("Data")?.get("Version");
    let id = version.map_or(0, |v| {
        number(v.get("Id")).unwrap_or_else(|| i64::from(DATA_VERSION))
    });
    if id <= i64::from(DATA_VERSION) {
        return None;
    }
    Some(saved_version_name(version))
}
/// `LevelStorageAccess.checkForLowDiskSpace`: under 64 MiB usable.
pub const LOW_DISK_SPACE: u64 = 67_108_864;

/// The `PlayerData::inventory` slots of `EquipmentSlot.BODY` and `SADDLE`.
/// Vanilla saves them in the `equipment` compound with the armor and off hand.
pub const BODY_SLOT: u8 = 151;
pub const SADDLE_SLOT: u8 = 152;

/// The `EquipmentSlot` ordinals a player saves in `equipment`, with the
/// `PlayerData::inventory` slot each is held in: armor 100 (feet) to 103
/// (head), the off hand 150, body 151, saddle 152. The main hand is the
/// selected inventory slot and never in `equipment`.
const PLAYER_EQUIPMENT_SLOTS: [(usize, u8); 7] = [
    (sunrise_items::equipment::FEET, 100),
    (sunrise_items::equipment::LEGS, 101),
    (sunrise_items::equipment::CHEST, 102),
    (sunrise_items::equipment::HEAD, 103),
    (sunrise_items::equipment::OFFHAND, 150),
    (sunrise_items::equipment::BODY, BODY_SLOT),
    (sunrise_items::equipment::SADDLE, SADDLE_SLOT),
];

/// The `EquipmentSlot` ordinal a saved slot number stands for, for the
/// slots a player holder keeps in its `Equipment`.
pub fn equipment_slot(slot: u8) -> Option<usize> {
    match slot {
        BODY_SLOT => Some(sunrise_items::equipment::BODY),
        SADDLE_SLOT => Some(sunrise_items::equipment::SADDLE),
        _ => None,
    }
}
/// The saved slot number of a held `EquipmentSlot` ordinal (the inverse).
pub fn saved_slot(equipment_slot: usize) -> Option<u8> {
    match equipment_slot {
        sunrise_items::equipment::BODY => Some(BODY_SLOT),
        sunrise_items::equipment::SADDLE => Some(SADDLE_SLOT),
        _ => None,
    }
}
/// `LevelStorageAccess.estimateDiskSpace`: the usable bytes of the disk holding `dir`
/// (`FileStore.getUsableSpace`), `u64::MAX` when it cannot be asked.
pub fn estimate_disk_space(dir: &Path) -> u64 {
    fs4::available_space(dir).unwrap_or(u64::MAX)
}
/// `LevelStorageAccess.checkForLowDiskSpace`.
pub fn low_disk_space(dir: &Path) -> bool {
    estimate_disk_space(dir) < LOW_DISK_SPACE
}
/// `WorldOptions.isOldCustomizedWorld`: the generator settings keep `legacy_custom_options`.
pub fn is_old_customized_world(world: &Path) -> bool {
    world_gen_settings(world).is_some_and(|s| text(s.get("legacy_custom_options")).is_some())
}
/// A dimension's `LevelStem` field read as a resource location (an inline definition is not one).
fn stem_id(tag: Option<&Tag>) -> Option<String> {
    let id = text(tag)?;
    Some(if id.contains(':') {
        id
    } else {
        format!("minecraft:{id}")
    })
}
/// `WorldDimensions.isVanillaLike`: whether a saved dimension is the stable Overworld, Nether or
/// End (`isStableOverworld`, `isStableNether`, `isStableEnd`); any other key never is.
fn dimension_is_vanilla_like(key: &str, dimension: &Tag) -> bool {
    let kind = stem_id(dimension.get("type"));
    let generator = dimension.get("generator");
    let source = generator.and_then(|g| g.get("biome_source"));
    let source_kind = source.and_then(|s| stem_id(s.get("type")));
    // MultiNoiseBiomeSource.stable(preset): a reference to the preset, not its parameters.
    let multi_noise_preset = |preset: &str| {
        source_kind.as_deref() == Some("minecraft:multi_noise")
            && stem_id(source.and_then(|s| s.get("preset"))).as_deref() == Some(preset)
    };
    // NoiseBasedChunkGenerator.stable(settings): the generator uses the settings by key.
    let noise_settings = |settings: &str| {
        generator.is_some_and(|g| stem_id(g.get("type")).as_deref() == Some("minecraft:noise"))
            && stem_id(generator.and_then(|g| g.get("settings"))).as_deref() == Some(settings)
    };
    match key {
        "minecraft:overworld" => {
            matches!(
                kind.as_deref(),
                Some("minecraft:overworld" | "minecraft:overworld_caves")
            ) && (source_kind.as_deref() != Some("minecraft:multi_noise")
                || multi_noise_preset("minecraft:overworld"))
        }
        "minecraft:the_nether" => {
            kind.as_deref() == Some("minecraft:the_nether")
                && noise_settings("minecraft:nether")
                && multi_noise_preset("minecraft:nether")
        }
        "minecraft:the_end" => {
            kind.as_deref() == Some("minecraft:the_end")
                && noise_settings("minecraft:end")
                && source_kind.as_deref() == Some("minecraft:the_end")
        }
        _ => false,
    }
}
/// A data pack's JSON as the NBT `dimension_is_vanilla_like` reads (what a codec gets from
/// either one is the same data).
pub(crate) fn json_tag(value: &serde_json::Value) -> Tag {
    use serde_json::Value;
    match value {
        Value::Null => Tag::compound(std::iter::empty::<(&str, Tag)>()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => n.as_i64().map_or_else(
            || Tag::Double(n.as_f64().unwrap_or_default().to_bits()),
            Tag::Long,
        ),
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(items) => Tag::List(items.iter().map(json_tag).collect()),
        Value::Object(map) => Tag::compound(map.iter().map(|(k, v)| (k.as_str(), json_tag(v)))),
    }
}
/// The level stems the world's enabled data packs define, as `WorldOpenFlows.loadWorldStem`'s
/// `datapackDimensions`.
fn datapack_dimensions(world: &Path) -> Vec<(String, Tag)> {
    let packs = crate::datapack::discover(world);
    let enabled_features = Level::read(world)
        .map(|l| l.enabled_features)
        .unwrap_or_default();
    let enabled = crate::datapack::enabled_ids(world, &packs, &enabled_features);
    let enabled: Vec<&crate::datapack::DataPack> = enabled
        .iter()
        .filter_map(|id| packs.iter().find(|p| &p.id == id))
        .collect();
    crate::datapack::dimension_stems(&enabled)
        .into_iter()
        .map(|(id, json)| (id, json_tag(&json)))
        .collect()
}
/// `data.worldGenSettingsLifecycle() != Lifecycle.stable()` (`WorldDimensions.bake`): the known
/// dimensions (the saved ones and the enabled data packs', a pack's replacing the saved one of
/// its id) lack one of the three builtin ones, or have one that is not the vanilla one.
pub fn world_gen_unstable(world: &Path) -> bool {
    let saved = world_gen_settings(world)
        .as_ref()
        .and_then(|s| s.get("dimensions"))
        .cloned();
    let stems = datapack_dimensions(world);
    if saved.is_none() && stems.is_empty() {
        return false;
    }
    let mut dimensions: std::collections::BTreeMap<String, Tag> = match saved {
        Some(Tag::Compound(saved)) => saved
            .into_iter()
            .map(|(key, dimension)| (key.to_string_lossy(), dimension))
            .collect(),
        _ => std::collections::BTreeMap::new(),
    };
    // `baseDimensions.getOptional(key).or(saved)`.
    dimensions.extend(stems);
    let builtin = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ];
    builtin.iter().any(|key| !dimensions.contains_key(*key))
        || dimensions
            .iter()
            .any(|(key, dimension)| !dimension_is_vanilla_like(key, dimension))
}
/// Move a world saved by an older Sunrise build (pre-26.3 layout with a
/// DataVersion that keeps vanilla from upgrading it) into the 26.3 layout.
pub fn migrate_legacy(world: &Path) -> Result<()> {
    for (old, dimension) in [
        ("", "minecraft:overworld"),
        ("DIM-1", "minecraft:the_nether"),
        ("DIM1", "minecraft:the_end"),
    ] {
        let from = if old.is_empty() {
            world.to_owned()
        } else {
            world.join(old)
        };
        let to = dimension_dir(world, dimension);
        for folder in ["region", "entities", "poi"] {
            let (src, dst) = (from.join(folder), to.join(folder));
            if src.is_dir() && !dst.exists() {
                fs::create_dir_all(&to)?;
                fs::rename(&src, &dst)?;
            }
        }
        if !old.is_empty() && from.is_dir() && fs::read_dir(&from)?.next().is_none() {
            let _ = fs::remove_dir(&from);
        }
    }
    Ok(())
}

/// Registry lookups shared by chunk encoding and decoding.
pub struct SaveTables {
    /// Block registry IDs (`BlockId`, scheduled tick types) in registry order.
    block_names: Vec<String>,
    block_ids: HashMap<String, u16>,
    /// Default state of each state's block, to write vanilla's short palette form.
    default_state: Vec<bool>,
    block_entity: Vec<bool>,
    heightmaps: crate::assets::Properties,
    air: u32,
    /// The POI type and its max tickets of each state (`PoiTypes`).
    poi: Vec<Option<(&'static str, i32)>>,
}
impl SaveTables {
    pub fn new(assets: &crate::assets::Assets) -> Self {
        let registry = assets.registry;
        let mut block_names: Vec<String> = Vec::new();
        let mut default_state = Vec::with_capacity(registry.len());
        for id in 0..registry.len() {
            let state = registry
                .state(BlockStateId(id as u32))
                .expect("registry state");
            if block_names.last() != Some(&state.name) {
                block_names.push(state.name.clone());
            }
            default_state
                .push(registry.default_state(&state.name) == Some(BlockStateId(id as u32)));
        }
        let block_ids = block_names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i as u16))
            .collect();
        Self {
            block_names,
            block_ids,
            default_state,
            block_entity: Vec::new(),
            heightmaps: assets.properties.clone(),
            air: registry.default_state("minecraft:air").map_or(0, |s| s.0),
            poi: (0..registry.len())
                .map(|id| {
                    registry
                        .state(BlockStateId(id as u32))
                        .and_then(sunrise_world::poi::poi_type)
                })
                .collect(),
        }
    }
    /// Tables with only the heightmaps' state facts, for tests.
    #[cfg(test)]
    pub(crate) fn for_heightmaps(heightmaps: crate::assets::Properties) -> Self {
        Self {
            block_names: Vec::new(),
            block_ids: HashMap::new(),
            default_state: Vec::new(),
            block_entity: Vec::new(),
            heightmaps,
            air: 0,
            poi: Vec::new(),
        }
    }
    /// Which states carry a block entity (`BlockFacts`), to drop entries for
    /// unknown block entities whose block is gone.
    pub fn with_block_entities(mut self, facts: &crate::blocks::BlockFacts) -> Self {
        self.block_entity = facts.states.iter().map(|s| s.block_entity).collect();
        self
    }
}

pub use crate::chunk::{ChunkExtras, PendingContainerLoot, SavedProto};

fn tick_list(ticks: &[SavedTick], name: impl Fn(u32) -> Option<String>) -> Tag {
    Tag::List(
        ticks
            .iter()
            .filter_map(|t| {
                Some(Tag::compound([
                    ("i", string(&name(t.kind.0)?)),
                    ("x", Tag::Int(t.pos.x)),
                    ("y", Tag::Int(t.pos.y)),
                    ("z", Tag::Int(t.pos.z)),
                    ("t", Tag::Int(t.delay)),
                    ("p", Tag::Int(t.priority as i32)),
                ]))
            })
            .collect(),
    )
}
fn read_ticks(
    tag: Option<&Tag>,
    chunk: (i32, i32),
    id: impl Fn(&str) -> Option<u32>,
) -> Vec<SavedTick> {
    let Some(Tag::List(list)) = tag else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|t| {
            let kind = id(&text(t.get("i"))?)?;
            let pos = BlockPos::new(
                number(t.get("x"))? as i32,
                number(t.get("y"))? as i32,
                number(t.get("z"))? as i32,
            );
            // SavedTick.filterTickListForChunk
            if (pos.x >> 4, pos.z >> 4) != chunk {
                return None;
            }
            Some(SavedTick {
                kind: TickTypeId(kind),
                pos,
                delay: number(t.get("t")).unwrap_or(0) as i32,
                priority: TickPriority::by_value(number(t.get("p")).unwrap_or(0) as i32),
            })
        })
        .collect()
}
fn fluid_name(kind: u32) -> Option<String> {
    use sunrise_world::fluid::FluidKind;
    match FluidKind::from_tick_id(kind)? {
        FluidKind::Empty => None,
        fluid => Some(fluid.name().to_owned()),
    }
}
fn fluid_id(name: &str) -> Option<u32> {
    use sunrise_world::fluid::FluidKind;
    FluidKind::from_name(name)
        .filter(|kind| *kind != FluidKind::Empty)
        .map(FluidKind::tick_id)
}

pub use sunrise_items::item_nbt::{
    decode_items, encode_item, encode_items, item_from_nbt, item_to_nbt,
};

/// The block entity type ID for the block it sits in (a chest-like container
/// is a chest, trapped chest or barrel).
pub fn block_entity_id<'a>(e: &'a crate::containers::BlockEntity, block: &'a str) -> &'a str {
    let block = crate::block_entity_types::factory_block(block);
    match (e, block) {
        (
            crate::containers::BlockEntity::Chest(_),
            "minecraft:trapped_chest"
            | "minecraft:barrel"
            | "minecraft:dropper"
            | "minecraft:ender_chest",
        ) => block,
        (crate::containers::BlockEntity::Chest(_), b) if b.ends_with("shulker_box") => {
            "minecraft:shulker_box"
        }
        _ => e.id(),
    }
}
/// `block_entity_nbt::overlay`, then a container that now holds items loses its
/// loot table (vanilla clears it when the loot is generated into the container).
fn merge_block_entity(original: &Tag, loaded: &Tag, current: &Tag, items_are_payload: bool) -> Tag {
    let mut merged = crate::block_entity_nbt::overlay(original, loaded, current, items_are_payload);
    let has_items = !items_are_payload
        && matches!(current.get("Items"), Some(Tag::List(items)) if !items.is_empty());
    if has_items
        && current.get("LootTable").is_none()
        && let Tag::Compound(map) = &mut merged
    {
        map.remove(&"LootTable".into());
        map.remove(&"LootTableSeed".into());
    }
    merged
}
/// `BannerBlockEntity.loadAdditional` (and the `components` a block entity keeps).
pub fn decode_banner(tag: &Tag) -> crate::containers::Banner {
    crate::containers::Banner {
        layers: crate::banner::layers_from_nbt(tag.get("patterns")),
        name: text(tag.get("CustomName")),
        components: match tag.get("components") {
            Some(Tag::Compound(c)) => c.clone(),
            _ => Compound::new(),
        },
    }
}
/// `TheEndGatewayBlockEntity.loadAdditional`.
pub fn decode_end_gateway(tag: &Tag) -> crate::containers::EndGateway {
    crate::containers::EndGateway {
        age: number(tag.get("Age")).unwrap_or(0),
        exit_portal: tag
            .get("exit_portal")
            .and_then(crate::block_entity_nbt::gateway_exit),
        exact_teleport: crate::entity_nbt::flag(tag, "ExactTeleport"),
    }
}
/// A mod's block entity: its compound without `id` and the position, and
/// (when `decode_items` is true, i.e. container-backed or unknown-block loads)
/// the `Items` rows that can be represented losslessly as modeled slots.
/// Slotless holders loaded at a known block keep `Items` verbatim in `nbt`.
/// The game sizes the modeled vector to its container when it uses it.
fn decode_mod_block_entity(
    id: String,
    tag: &Tag,
    decode_items: bool,
) -> Option<crate::containers::ModBlockEntity> {
    use crate::block_entity_nbt::saved_item_slot;
    let Tag::Compound(map) = tag else {
        return None;
    };
    let mut nbt = map.clone();
    for key in ["id", "x", "y", "z"] {
        nbt.remove(&key.into());
    }
    // The published shapes are no field of the mod's: saved data that does not check is dropped.
    let shapes = nbt
        .remove(&crate::containers::SHAPES_KEY.into())
        .and_then(|tag| crate::containers::PublishedShapes::from_tag(&tag));
    let mut items = Vec::new();
    if decode_items {
        let key = "Items".into();
        if let Some(Tag::List(entries)) = nbt.get_mut(&key) {
            let mut occurrences = [0u8; 256];
            for slot in entries.iter().filter_map(saved_item_slot) {
                occurrences[slot] = occurrences[slot].saturating_add(1);
            }
            entries.retain(|raw| {
                let Some(slot) = saved_item_slot(raw) else {
                    return true;
                };
                if occurrences[slot] == 1
                    && let Some(stack) = item_from_nbt(raw)
                    && let Tag::Compound(mut canonical) = Tag::from(&stack)
                {
                    canonical.insert("Slot".into(), Tag::Byte(slot as i8));
                    if raw == &Tag::Compound(canonical) {
                        if items.len() <= slot {
                            items.resize(slot + 1, None);
                        }
                        items[slot] = Some(stack);
                        return false;
                    }
                }
                true
            });
            if entries.is_empty() && !items.is_empty() {
                nbt.remove(&key);
            }
        }
    }
    let mut entity = crate::containers::ModBlockEntity::new(
        id,
        crate::containers::ModBlockEntityFields {
            nbt,
            items,
            data: Vec::new(),
        },
    );
    entity.set_shapes(shapes.map(std::sync::Arc::new));
    Some(entity)
}
/// Whether a mod block owns the saved `Items` field as engine-managed inventory.
fn block_has_mod_item_slots(block: &str) -> bool {
    mod_api::__private::parts_installation::block_container(block).is_some_and(|c| c.slots > 0)
}

/// Decode an entity at a known block, refusing undeclared IDs and invalid native membership.
/// Canonical membership wins; blocks absent from the table use vanilla membership.
pub fn decode_block_entity_for_block(
    tag: &Tag,
    block: &str,
) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    decode_block_entity_replacing(tag, block, None)
}

/// Like [`decode_block_entity_for_block`], retaining mutable set history when
/// reading new NBT onto an existing trial spawner.
pub fn decode_block_entity_replacing(
    tag: &Tag,
    block: &str,
    previous: Option<&crate::containers::BlockEntity>,
) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    let decoded = decode_block_entity_with_history(tag, previous, block_has_mod_item_slots(block))?;
    // Several vanilla types share a decoded representation (for example barrel
    // and chest). Membership belongs to the saved type, not that representation.
    let saved_id = text(tag.get("id"))?;
    let valid = match &decoded.1 {
        crate::containers::BlockEntity::Mod(m) => {
            crate::block_entity_types::valid(block, m.id()) == Some(true)
        }
        _ => crate::block_entity_types::valid(block, &saved_id)
            .unwrap_or_else(|| decoded.1.fits(block)),
    };
    valid.then_some(decoded)
}

pub fn decode_block_entity(tag: &Tag) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    decode_block_entity_with_history(tag, None, true)
}

fn decode_block_entity_with_history(
    tag: &Tag,
    previous: Option<&crate::containers::BlockEntity>,
    decode_mod_items: bool,
) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    use crate::containers::BlockEntity;
    let int = |k: &str| tag.get(k).and_then(crate::nbt_path::int_value); // getIntOr
    let id = text(tag.get("id"))?;
    let pos = (int("x")?, int("y")?, int("z")?);
    if id.contains(':') && !id.starts_with("minecraft:") {
        return Some((
            pos,
            BlockEntity::Mod(Box::new(decode_mod_block_entity(
                id,
                tag,
                decode_mod_items,
            )?)),
        ));
    }
    if id == "minecraft:end_gateway" {
        return Some((pos, BlockEntity::EndGateway(decode_end_gateway(tag))));
    }
    if id == "minecraft:jukebox" {
        let record = tag.get("RecordItem").and_then(item_from_nbt);
        let clock = number(tag.get("ticks_since_song_started"));
        // `JukeboxBlockEntity.loadAdditional` -> `setSongWithoutPlaying`: only a
        // saved clock brings a song back, and it comes back without playing.
        let clocked = record.is_some() && clock.is_some();
        return Some((
            pos,
            BlockEntity::Jukebox(crate::containers::Jukebox {
                items: [record],
                ticks: clock.unwrap_or(0),
                song_clock_present: clocked,
                playing: clocked,
                loaded_clock: clocked,
            }),
        ));
    }
    if id == "minecraft:creaking_heart" {
        let heart = crate::containers::CreakingHeart {
            creaking: uuid_from_tag(tag.get("creaking")).map(|uuid| (None, uuid)),
            ..Default::default()
        };
        return Some((pos, BlockEntity::CreakingHeart(Box::new(heart))));
    }
    if id == "minecraft:lectern" {
        // `LecternBlockEntity.loadAdditional`: the page is `Mth.clamp(Page, 0,
        // getPageCount(book) - 1)`, so a stored page before the book's first
        // page or past its last comes back as the page the block holds. No
        // book, or a book with no pages, counts 0 pages: the maximum is then
        // -1, and `Mth.clamp` (`Math.min(Math.max(value, min), max)`) answers
        // it, -1.
        let book = tag.get("Book").and_then(item_from_nbt);
        let count = book.as_ref().map_or(0, crate::menu::book_pages) as i32;
        let lectern = crate::containers::Lectern {
            book,
            page: int("Page").unwrap_or(0).max(0).min(count - 1),
        };
        return Some((pos, BlockEntity::Lectern(lectern)));
    }
    if id == "minecraft:banner" {
        return Some((pos, BlockEntity::Banner(decode_banner(tag))));
    }
    if id == "minecraft:beacon" {
        let beacon = crate::containers::Beacon {
            // BeaconBlockEntity.loadAdditional never reads `Levels`: updateBase sets them.
            primary: text(tag.get("primary_effect")),
            secondary: text(tag.get("secondary_effect")),
            last_check_y: i32::MIN / 2,
            ..Default::default()
        };
        return Some((pos, BlockEntity::Beacon(Box::new(beacon))));
    }
    // A skull's block entity id is not its block's name (every head block).
    if id == "minecraft:skull" {
        return Some((
            pos,
            BlockEntity::Skull(crate::containers::Skull::new(
                tag.get("profile").cloned(),
                text(tag.get("note_block_sound")),
                tag.get("custom_name")
                    .and_then(crate::block_entity_nbt::component),
            )),
        ));
    }
    // A spawner's block entity id is not its block's name.
    let block = match id.as_str() {
        "minecraft:mob_spawner" => "minecraft:spawner",
        other => other,
    };
    let mut entity = BlockEntity::for_block(block)?;
    match &mut entity {
        BlockEntity::Chest(items) => {
            // Randomizable containers reset their inventory, then load Items only
            // when tryLoadLootTable did not read a valid loot-table key.
            let has_table = text(tag.get("LootTable"))
                .and_then(|id| sunrise_client_core::identifier::normalize_id(&id))
                .is_some();
            if !has_table {
                *items = decode_items(tag.get("Items"), items.len());
            }
        }
        BlockEntity::Furnace(f) => {
            let items = decode_items(tag.get("Items"), 3);
            f.items = [items[0].clone(), items[1].clone(), items[2].clone()];
            f.lit_remaining = int("lit_time_remaining").unwrap_or(0);
            f.lit_total = int("lit_total_time").unwrap_or(0);
            f.cooking_timer = int("cooking_time_spent").unwrap_or(0);
            f.cooking_total = int("cooking_total_time").unwrap_or(0);
            if let Some(speed) = float(tag.get("speed_multiplier")) {
                f.speed = speed as f32;
            }
            // A save without it owes nothing: the field is newer than some worlds.
            if let Some(Tag::Compound(used)) = tag.get("RecipesUsed") {
                for (id, count) in used {
                    let count = match count {
                        Tag::Int(n) => *n,
                        _ => continue,
                    };
                    // Saves from before the key was namespaced hold both forms of an id.
                    *f.recipes_used
                        .entry(crate::containers::recipe_used_key(&id.to_string_lossy()))
                        .or_insert(0) += count.max(0) as u32;
                }
            }
        }
        BlockEntity::Brewing(b) => {
            let items = decode_items(tag.get("Items"), 5);
            b.items = [
                items[0].clone(),
                items[1].clone(),
                items[2].clone(),
                items[3].clone(),
                items[4].clone(),
            ];
            b.brew_time = int("BrewTime").unwrap_or(0);
            b.total_brew_time = int("total_brew_time").unwrap_or(400);
            if b.brew_time > 0 {
                b.ingredient = b.items[3].as_ref().map(|s| s.item.clone());
            }
            b.fuel = int("Fuel").unwrap_or(0);
            b.total_fuel = int("total_fuel").unwrap_or(20);
            b.speed = float(tag.get("speed_multiplier")).map_or(1.0, |v| v as f32);
        }
        BlockEntity::Brushable(b) => {
            b.loot_table = tag.get("LootTable").cloned();
            b.table_seed_tag = tag.get("LootTableSeed").cloned();
            b.item = if b.loot_table.is_none() {
                tag.get("item").and_then(item_from_nbt)
            } else {
                None
            };
            b.hit_direction = int("hit_direction").filter(|v| (0..=5).contains(v));
        }
        BlockEntity::Sign(sign) => sign.decode(tag),
        BlockEntity::Sculk(s) => s.load(tag),
        BlockEntity::Shelf(shelf) => {
            for (slot, item) in decode_items(tag.get("Items"), 3).into_iter().enumerate() {
                shelf.items[slot] = item;
            }
            shelf.align_to_bottom = int("align_items_to_bottom").unwrap_or(0) != 0;
        }
        // ChiseledBookShelfBlockEntity.loadAdditional.
        BlockEntity::ChiseledBookshelf(b) => {
            for (slot, item) in decode_items(tag.get("Items"), 6).into_iter().enumerate() {
                b.items[slot] = item;
            }
            b.last_slot = int("last_interacted_slot").unwrap_or(-1);
        }
        // CampfireBlockEntity.loadAdditional: missing time arrays stay zeros.
        BlockEntity::Campfire(c) => {
            for (slot, item) in decode_items(tag.get("Items"), 4).into_iter().enumerate() {
                c.items[slot] = item;
            }
            let ints = |key: &str| match tag.get(key) {
                Some(Tag::IntArray(v)) => std::array::from_fn(|i| v.get(i).copied().unwrap_or(0)),
                _ => [0; 4],
            };
            c.progress = ints("CookingTimes");
            c.total = ints("CookingTotalTimes");
        }
        BlockEntity::Crafter(c) => {
            for (slot, item) in decode_items(tag.get("Items"), 9).into_iter().enumerate() {
                c.items[slot] = item;
            }
            if let Some(Tag::IntArray(slots)) = tag.get("disabled_slots") {
                for &slot in slots {
                    if (0..9).contains(&slot) && c.items[slot as usize].is_none() {
                        c.disabled[slot as usize] = true;
                    }
                }
            }
            c.triggered = int("triggered").unwrap_or(0);
            c.state_triggered = None;
            c.crafting_ticks = int("crafting_ticks_remaining").unwrap_or(0);
        }
        // PotentSulfurBlockEntity.loadAdditional.
        BlockEntity::PotentSulfur(sulfur) => {
            if let Some(countdown) = int("countdown") {
                sulfur.waiting_countdown = countdown;
            }
        }
        BlockEntity::Jukebox(_)
        | BlockEntity::Beacon(_)
        | BlockEntity::Lectern(_)
        | BlockEntity::Banner(_)
        | BlockEntity::CreakingHeart(_)
        | BlockEntity::EndGateway(_)
        | BlockEntity::Skull(_)
        | BlockEntity::DaylightDetector
        | BlockEntity::Bell
        | BlockEntity::EndPortal
        | BlockEntity::EnchantingTable
        | BlockEntity::Mod(_) => {}
        // ConduitBlockEntity.loadAdditional: EntityReference.read through
        // UUIDUtil.CODEC, an int array of four.
        BlockEntity::Conduit(target) => {
            *target = match tag.get("Target") {
                Some(Tag::IntArray(v)) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
                _ => None,
            };
        }
        // `BlockEntity.loadAdditional`: the components the block entity keeps.
        BlockEntity::CopperGolemStatue(components) => {
            *components = match tag.get("components") {
                Some(Tag::Compound(c)) => c.clone(),
                _ => Compound::new(),
            };
        }
        BlockEntity::Spawner(s) => **s = crate::mobs::spawner::Spawner::load(tag),
        BlockEntity::TrialSpawner(s) => {
            let previous = match previous {
                Some(BlockEntity::TrialSpawner(old)) => Some(old.as_ref()),
                _ => None,
            };
            **s = crate::mobs::trial_spawner::TrialSpawner::load(tag, previous);
        }
        BlockEntity::Vault(v) => **v = crate::vault::Vault::load(tag),
        BlockEntity::Beehive(h) => **h = crate::beehive::Beehive::from_nbt(tag),
        BlockEntity::CommandBlock(c) => **c = crate::command_block::CommandBlock::load(tag),
        BlockEntity::StructureBlock(b) => **b = crate::structure_block::StructureBlock::load(tag),
        BlockEntity::TestBlock(b) => **b = crate::test_block::TestBlock::load(tag),
        BlockEntity::TestInstanceBlock(b) => b.load(tag),
        // The state is not known here (Game::decode_block_entity_at applies it).
        BlockEntity::JigsawBlock(b) => {
            **b = crate::jigsaw_block::JigsawBlock::load(
                tag,
                crate::jigsaw_block::JointType::Rollable,
            )
        }
    }
    Some((pos, entity))
}

mod villager_poi;
pub use villager_poi::encode_poi;
/// A record's `free_tickets` from the POI file the chunk had, while the same
/// POI is still there: a bed or job site a villager claimed stays claimed.
fn keep_claims(saved: &mut NamedTag, old: &NamedTag) {
    let key = |r: &Tag| match (r.get("pos"), text(r.get("type"))) {
        (Some(Tag::IntArray(pos)), Some(kind)) => Some((pos.clone(), kind)),
        _ => None,
    };
    let mut claims = HashMap::new();
    if let Some(Tag::Compound(sections)) = old.tag.get("Sections") {
        for section in sections.values() {
            if let Some(Tag::List(records)) = section.get("Records") {
                for r in records {
                    if let Some(k) = key(r) {
                        claims.insert(k, number(r.get("free_tickets")).unwrap_or(0) as i32);
                    }
                }
            }
        }
    }
    let Tag::Compound(root) = &mut saved.tag else {
        return;
    };
    let Some(Tag::Compound(sections)) = root.get_mut(&"Sections".into()) else {
        return;
    };
    sections.for_each_mut(|_, section| {
        let Tag::Compound(section) = section else {
            return;
        };
        let Some(Tag::List(records)) = section.get_mut(&"Records".into()) else {
            return;
        };
        for r in records {
            let Some(&tickets) = key(r).and_then(|k| claims.get(&k)) else {
                continue;
            };
            if let Tag::Compound(r) = r {
                if tickets == 0 {
                    r.remove(&"free_tickets".into());
                } else {
                    r.insert("free_tickets".into(), Tag::Int(tickets));
                }
            }
        }
    });
}
fn poi_root(sections: Compound) -> NamedTag {
    NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("Sections", Tag::Compound(sections)),
        ]),
    }
}

pub use crate::chunk::LevelShape;

/// A column's sections as `Heightmap.primeHeightmaps` reads them.
struct HeightmapView<'a> {
    sections: &'a [Arc<Section>],
    /// The minimum Y of `sections[0]`.
    column_min_y: i32,
    tables: &'a SaveTables,
    min_y: i32,
    height: i32,
}
impl HeightmapChunk for HeightmapView<'_> {
    type State = crate::assets::StateFacts;
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.height
    }
    fn highest_section_y(&self) -> i32 {
        let top = self
            .sections
            .iter()
            .rposition(|s| !s.is_uniform(self.tables.air))
            .unwrap_or(0);
        self.column_min_y + top as i32 * 16
    }
    fn block_state(&self, x: i32, y: i32, z: i32) -> Self::State {
        // Chunk.get: no block outside the sections.
        let state = usize::try_from((y - self.column_min_y) >> 4)
            .ok()
            .and_then(|s| self.sections.get(s))
            .map_or(0, |s| s.get(crate::chunk::index(x, y, z)));
        self.tables.heightmaps.heightmap_state(BlockStateId(state))
    }
}
/// Heightmaps kept after world generation (`ChunkStatus.FULL.heightmapsAfter`).
fn heightmaps(chunk: &Chunk, tables: &SaveTables, shape: LevelShape) -> Tag {
    let kinds = [
        HeightmapType::WorldSurface,
        HeightmapType::OceanFloor,
        HeightmapType::MotionBlocking,
        HeightmapType::MotionBlockingNoLeaves,
    ];
    let mut maps: Vec<Heightmap> = kinds
        .iter()
        .filter_map(|k| Heightmap::new(*k, shape.min_y, shape.height).ok())
        .collect();
    let view = HeightmapView {
        sections: &chunk.sections,
        column_min_y: chunk.min_y(),
        tables,
        min_y: shape.min_y,
        height: shape.height,
    };
    let mut refs: Vec<&mut Heightmap> = maps.iter_mut().collect();
    if Heightmap::prime(&view, &mut refs).is_err() {
        return Tag::Compound(Compound::new());
    }
    Tag::compound(maps.iter().map(|m| {
        (
            m.kind().name(),
            Tag::LongArray(m.raw_data().iter().map(|&w| w as i64).collect()),
        )
    }))
}
/// The client heightmaps of a column's sections (from `column_min_y`), in a
/// level from `min_y`, `height` blocks high.
pub fn client_heightmaps_in(
    sections: &[Arc<Section>],
    column_min_y: i32,
    tables: &SaveTables,
    min_y: i32,
    height: i32,
) -> Vec<(i32, Vec<i64>)> {
    let kinds = [
        (1, HeightmapType::WorldSurface),
        (4, HeightmapType::MotionBlocking),
        (5, HeightmapType::MotionBlockingNoLeaves),
    ];
    let mut maps: Vec<(i32, Heightmap)> = kinds
        .iter()
        .filter_map(|(id, k)| Heightmap::new(*k, min_y, height).ok().map(|m| (*id, m)))
        .collect();
    let view = HeightmapView {
        sections,
        column_min_y,
        tables,
        min_y,
        height,
    };
    let mut refs: Vec<&mut Heightmap> = maps.iter_mut().map(|(_, m)| m).collect();
    if Heightmap::prime(&view, &mut refs).is_err() {
        return Vec::new();
    }
    maps.iter()
        .map(|(id, m)| (*id, m.raw_data().iter().map(|&w| w as i64).collect()))
        .collect()
}
/// Vanilla light layer (nibbles, index y<<8|z<<4|x, low nibble first).
fn nibbles(light: &Light, sky: bool) -> Vec<u8> {
    let mut out = vec![0u8; 2048];
    for (i, byte) in out.iter_mut().enumerate() {
        let value = |j: usize| {
            let v = light.get(j);
            if sky { v >> 4 } else { v & 15 }
        };
        *byte = value(2 * i) | value(2 * i + 1) << 4;
    }
    out
}

pub fn encode(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    items: &crate::items::Items,
) -> NamedTag {
    encode_with(chunk, registry, biomes, None, LevelShape::OVERWORLD, items)
}
/// A save's lookup of an old chunk's calculated blending data.
pub type BlendingLookup = Box<dyn Fn(i32, i32) -> Option<Arc<BlendingData>> + Send + Sync>;
/// [`encode_with`] for a save of `SaveDir`: an old chunk whose blending data
/// the generation calculated saves it (`BlendingData.pack`), when the record
/// it was loaded from has the same section range; otherwise `blending_data`
/// stays as it was loaded.
fn encode_saved(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    blending: Option<&BlendingData>,
    items: &crate::items::Items,
) -> NamedTag {
    let mut root = encode_with(chunk, registry, biomes, tables, shape, items);
    if let Some(data) = blending {
        put_calculated_blending(&mut root, data);
    }
    root
}
fn put_calculated_blending(root: &mut NamedTag, data: &BlendingData) {
    let Tag::Compound(map) = &mut root.tag else {
        return;
    };
    let Some(saved) = map.get(&"blending_data".into()) else {
        return;
    };
    let (min_section, max_section, heights) = data.pack();
    if saved.get("min_section") != Some(&Tag::Int(min_section))
        || saved.get("max_section") != Some(&Tag::Int(max_section))
    {
        return;
    }
    let mut fields = vec![
        ("min_section", Tag::Int(min_section)),
        ("max_section", Tag::Int(max_section)),
    ];
    if let Some(heights) = heights {
        fields.push((
            "heights",
            Tag::List(heights.iter().map(|h| Tag::Float(h.to_bits())).collect()),
        ));
    }
    map.insert("blending_data".into(), Tag::compound(fields));
}
/// The chunk NBT vanilla's `SerializableChunkData.write` produces for a full
/// chunk in a level of `shape`.
pub fn encode_with(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    items: &crate::items::Items,
) -> NamedTag {
    encode_full(chunk, registry, biomes, tables, shape, items, true)
}

fn encode_full(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    items: &crate::items::Items,
    apply_block_entity_edits: bool,
) -> NamedTag {
    let sky_light = shape.sky_light;
    let mut sections = Vec::new();
    // Sky layers below the highest section with light other than 15 are
    // written (even all-zero ones): vanilla derives missing layers from the
    // layer above them.
    let top_sky = chunk
        .sections
        .iter()
        .rposition(|s| s.light.any_not(0xF0, 0xF0));
    // `SerializableChunkData.copyOf` checks the engine's whole light range, not just
    // sections with block data. A dimension can be narrower than the chunk's allocation.
    let light_y = shape.min_section() - 1..=(shape.max_y() >> 4);
    let first = (chunk.min_y() >> 4) - 1;
    for y in light_y {
        let b = (y - first) as usize;
        let section = match b {
            0 => chunk.pad_below.as_ref(),
            _ if b == chunk.sections.len() + 1 => chunk.pad_above.as_ref(),
            _ => chunk.sections.get(b - 1),
        };
        let Some(section) = section else { continue };
        let layers = if chunk.lit {
            if let Some(stored) = chunk.stored_light() {
                let [sky, block] = stored.presence(b);
                if let Some([sky_array, block_array]) = stored.saved_arrays(b) {
                    // `SerializableChunkData.copyOf`: omit empty layers restored from
                    // section status, but keep materialized arrays, including zero arrays.
                    [
                        sky && (sky_array || section.light.any(0xF0)),
                        block && (block_array || section.light.any(15)),
                    ]
                } else {
                    [sky, block]
                }
            } else if shape.has_section(y) {
                [
                    top_sky.is_some_and(|top| b <= top + 2),
                    section.light.any(15),
                ]
            } else {
                [false; 2]
            }
        } else {
            [false; 2]
        };
        let mut entries = vec![("Y", Tag::Byte(y as i8))];
        if layers[1] {
            entries.push(("BlockLight", Tag::ByteArray(nibbles(&section.light, false))));
        }
        if sky_light && layers[0] {
            entries.push(("SkyLight", Tag::ByteArray(nibbles(&section.light, true))));
        }
        if !shape.has_section(y) {
            if entries.len() > 1 {
                sections.push(Tag::compound(entries));
            }
            continue;
        }
        // PalettedContainer.pack: distinct states in index order, indices at
        // the saved width.
        let packed = section.blocks.pack();
        let mut states = vec![(
            "palette",
            Tag::List(
                packed
                    .palette
                    .iter()
                    .map(|&id| {
                        let state = registry.state(BlockStateId(id));
                        match state {
                            // BlockState.CODEC: the block name for default states.
                            Some(s)
                                if tables.is_some_and(|t| {
                                    t.default_state.get(id as usize) == Some(&true)
                                }) =>
                            {
                                string(&s.name)
                            }
                            Some(s) => s.to_nbt(),
                            None => string("minecraft:air"),
                        }
                    })
                    .collect(),
            ),
        )];
        if let Some(data) = &packed.data {
            states.push((
                "data",
                Tag::LongArray(data.iter().map(|&w| w as i64).collect()),
            ));
        }
        let biome_packed = section.biomes.pack();
        let mut biome_tag = vec![(
            "palette",
            Tag::List(
                biome_packed
                    .palette
                    .iter()
                    .map(|&b| string(&biomes.saved_name(b as u16)))
                    .collect(),
            ),
        )];
        if let Some(data) = &biome_packed.data {
            biome_tag.push((
                "data",
                Tag::LongArray(data.iter().map(|&w| w as i64).collect()),
            ));
        }
        entries.extend([
            ("block_states", Tag::compound(states)),
            ("biomes", Tag::compound(biome_tag)),
        ]);
        sections.push(Tag::compound(entries));
    }
    let extras = chunk.saved.as_deref();
    let block_name = |p: &(i32, i32, i32)| -> String {
        registry
            .state(chunk.block(p.0 & 15, p.1, p.2 & 15))
            .map_or_else(String::new, |s| s.name.clone())
    };
    let mut block_entities: Vec<Tag> = chunk
        .block_entities
        .iter()
        .map(|(p, e)| {
            let block = block_name(p);
            let stored = extras
                .and_then(|x| x.block_entity_sources.iter().find(|(q, _)| q == p))
                .map(|(_, tag)| tag);
            let source = stored.filter(|tag| {
                crate::block_entity_nbt::id(tag).as_deref() == Some(block_entity_id(e, &block))
            });
            let pending = extras.and_then(|x| x.pending_loot.get(p));
            let lock = source.and_then(|tag| tag.get("lock"));
            let hopper_cooldown = (block == "minecraft:hopper").then(|| {
                source
                    .and_then(|tag| number(tag.get("TransferCooldown")))
                    .unwrap_or(-1) as i32
            });
            let current = block_entity_encode::encode_with(
                *p,
                e,
                &block,
                block_entity_encode::DataFields {
                    lock,
                    custom_name: chunk
                        .block_names
                        .iter()
                        .find(|(q, _)| q == p)
                        .map(|(_, name)| name),
                    pending,
                    hopper_cooldown,
                },
                items,
                |_| {
                    if apply_block_entity_edits {
                        extras?.block_entity_save_edits.as_ref()?.get(p).cloned()
                    } else {
                        None
                    }
                },
            );
            let mut saved = match source {
                Some(original) => {
                    let items_are_payload = matches!(e, crate::containers::BlockEntity::Mod(_))
                        && !block_has_mod_item_slots(&block);
                    let loaded =
                        decode_block_entity_with_history(original, None, !items_are_payload)
                            .map_or_else(
                                || current.clone(),
                                |(q, e)| {
                                    // External name removal must win over the source overlay.
                                    let custom_name = (!matches!(
                                        e,
                                        crate::containers::BlockEntity::Banner(_)
                                            | crate::containers::BlockEntity::Sign(_)
                                            | crate::containers::BlockEntity::Mod(_)
                                    ))
                                    .then(|| {
                                        original
                                            .get("CustomName")
                                            .and_then(crate::block_entity_nbt::component)
                                    })
                                    .flatten();
                                    block_entity_encode::encode_with(
                                        q,
                                        &e,
                                        &block,
                                        block_entity_encode::DataFields {
                                            lock,
                                            custom_name: custom_name.as_ref(),
                                            pending: PendingContainerLoot::from_tag(original)
                                                .as_ref(),
                                            hopper_cooldown,
                                        },
                                        items,
                                        |_| None,
                                    )
                                },
                            );
                    merge_block_entity(original, &loaded, &current, items_are_payload)
                }
                None => current,
            };
            // A consumed loot table must not be restored from an opaque source.
            let chest = matches!(e, crate::containers::BlockEntity::Chest(_));
            if chest
                && pending.is_none()
                && let Tag::Compound(map) = &mut saved
            {
                map.remove(&"LootTable".into());
                map.remove(&"LootTableSeed".into());
            }
            // The overlay may restore an unchanged stack's original patch. Apply
            // catalog validation without rebuilding the Java table when replacing or
            // dropping fields. If normalization supplies missing defaults, use its
            // fresh field order, including the common metadata tail.
            let live_items = (chest && pending.is_some() && e.slots().iter().any(Option::is_some))
                .then(|| encode_items(e.slots(), items));
            if let Some(mut canonical) = crate::block_entity_nbt::canonical_with_stack(
                &saved,
                &|tag| {
                    crate::block_entity_nbt::stack(tag)
                        .and_then(|tag| item_from_nbt(&tag))
                        .map(|stack| item_to_nbt(&stack, items))
                },
                live_items,
            ) {
                if let Tag::Compound(map) = &mut saved
                    && canonical.keys().all(|key| map.contains_key(key))
                {
                    map.retain(|key, value| {
                        if let Some(admitted) = canonical.remove(key) {
                            *value = admitted;
                            true
                        } else {
                            false
                        }
                    });
                } else {
                    saved = Tag::Compound(canonical);
                }
            }
            if let Some(limit) = stored.and_then(|tag| tag.get("sunrise:stack_limit"))
                && let Tag::Compound(map) = &mut saved
            {
                map.insert("sunrise:stack_limit".into(), limit.clone());
            }
            if let Some((_, attachments)) =
                extras.and_then(|e| e.block_entity_attachments.iter().find(|(pos, _)| pos == p))
                && let Tag::Compound(map) = &mut saved
                && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
            {
                let data = attachments.to_nbt();
                if data.is_empty() {
                    map.remove(&ATTACHMENTS_KEY.into());
                } else {
                    map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
                }
            }
            saved
        })
        .collect();
    if let Some(extras) = extras {
        for raw in &extras.block_entities {
            let pos = (
                number(raw.get("x")).unwrap_or(0) as i32,
                number(raw.get("y")).unwrap_or(0) as i32,
                number(raw.get("z")).unwrap_or(0) as i32,
            );
            let state = chunk.get(pos.0 & 15, pos.1, pos.2 & 15) as usize;
            let still_there =
                tables.is_none_or(|t| t.block_entity.get(state).copied().unwrap_or(true));
            if still_there && !chunk.block_entities.iter().any(|(p, _)| *p == pos) {
                block_entities
                    .push(crate::block_entity_nbt::canonical(raw).unwrap_or_else(|| raw.clone()));
            }
        }
    }
    let mut root = extras
        .map(|e| Tag::Compound(e.root.clone()))
        .unwrap_or_else(|| Tag::Compound(Compound::new()));
    for (key, value) in [
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("xPos", Tag::Int(chunk.x)),
        ("zPos", Tag::Int(chunk.z)),
        ("yPos", Tag::Int(shape.min_section())),
        ("Status", string("minecraft:full")),
        ("LastUpdate", Tag::Long(extras.map_or(0, |e| e.last_update))),
        ("isLightOn", Tag::Byte(i8::from(chunk.lit))),
        ("sections", Tag::List(sections)),
        ("block_entities", Tag::List(block_entities)),
    ] {
        put(&mut root, key, value);
    }
    if let Some(Tag::Compound(map)) = Some(&mut root) {
        if let Some(extras) = extras
            && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
        {
            let attachments = extras.attachments.to_nbt();
            if attachments.is_empty() {
                map.remove(&ATTACHMENTS_KEY.into());
            } else {
                map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(attachments));
            }
        }
        map.entry("InhabitedTime".into()).or_insert(Tag::Long(0));
        map.entry("structures".into()).or_insert_with(|| {
            Tag::compound([
                ("References", Tag::Compound(Compound::new())),
                ("starts", Tag::Compound(Compound::new())),
            ])
        });
        if !chunk.structure_refs.is_empty()
            && let Some(Tag::Compound(s)) = map.get_mut(&"structures".into())
        {
            let mut refs: std::collections::BTreeMap<&str, Vec<i64>> = Default::default();
            for (id, (x, z)) in &chunk.structure_refs {
                refs.entry(id)
                    .or_default()
                    .push(i64::from(*x as u32) | i64::from(*z) << 32);
            }
            let refs: Compound = refs
                .into_iter()
                .map(|(id, v)| (id.into(), Tag::LongArray(v)))
                .collect();
            s.insert("References".into(), Tag::Compound(refs));
        }
        let starts: Compound = chunk
            .structure_starts
            .iter()
            .filter_map(|start| match start.save() {
                Some(tag) => Some((start.name.as_str().into(), tag)),
                None => {
                    log::warn!(
                        "Sunrise: structure start {} not saved: one of its pieces cannot be",
                        start.name
                    );
                    None
                }
            })
            .collect();
        if !starts.is_empty()
            && let Some(Tag::Compound(s)) = map.get_mut(&"structures".into())
        {
            s.insert("starts".into(), Tag::Compound(starts));
        }
        map.entry("PostProcessing".into())
            .or_insert_with(|| Tag::List(vec![Tag::List(Vec::new()); shape.section_count()]));
    }
    if let Some(t) = tables {
        put(&mut root, "Heightmaps", heightmaps(chunk, t, shape));
        let names = |k: u32| t.block_names.get(k as usize).cloned();
        let (blocks, fluids) = extras.map_or((&[][..], &[][..]), |e| {
            (&e.block_ticks[..], &e.fluid_ticks[..])
        });
        put(&mut root, "block_ticks", tick_list(blocks, names));
        put(&mut root, "fluid_ticks", tick_list(fluids, fluid_name));
    }
    NamedTag {
        name: "".into(),
        tag: root,
    }
}
/// Root keys `encode_with` regenerates; everything else is kept as loaded.
const GENERATED_KEYS: [&str; 14] = [
    "DataVersion",
    "xPos",
    "zPos",
    "yPos",
    "Status",
    "LastUpdate",
    "isLightOn",
    "sections",
    "block_entities",
    "Heightmaps",
    "block_ticks",
    "fluid_ticks",
    "entities",
    "CarvingMask",
];
pub fn decode(root: &NamedTag, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
    decode_with(root, registry, biomes, None, ColumnLayout::VANILLA)
}
/// Only full chunks load here; partially generated ones (vanilla's world
/// border of proto chunks) resume generation through `decode_proto`.
pub fn decode_with(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<Chunk> {
    if text(root.tag.get("Status")).is_some_and(|s| s != "minecraft:full" && s != "full") {
        return None;
    }
    decode_column(root, registry, biomes, tables, layout)
}
/// `decode_with` whatever the chunk's status, into a chunk of `layout`
/// (sections outside it are left out).
fn decode_column(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<Chunk> {
    let tag = &root.tag;
    let Tag::Int(x) = tag.get("xPos")? else {
        return None;
    };
    let Tag::Int(z) = tag.get("zPos")? else {
        return None;
    };
    let mut chunk = Chunk::with_layout(*x, *z, layout);
    // structures.References: structure id -> packed start chunk positions.
    if let Some(Tag::Compound(refs)) = tag.get("structures").and_then(|s| s.get("References")) {
        for (id, v) in refs {
            if let Tag::LongArray(starts) = v {
                let id: Arc<str> = Arc::from(id.to_string_lossy());
                for &p in starts {
                    chunk
                        .structure_refs
                        .push((id.clone(), (p as i32, (p >> 32) as i32)));
                }
            }
        }
    }
    let air = registry.default_state("minecraft:air")?.0;
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return Some(chunk);
    };
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let Some(si) = layout.section_index(y as i32 * 16) else {
            continue;
        };
        if let Some(states) = section.get("block_states")
            && let Some(Tag::List(palette)) = states.get("palette")
        {
            let palette: Vec<u32> = palette
                .iter()
                .map(|p| registry.saved_state_id(p).map_or(air, |id| id.0))
                .collect();
            let target = Arc::make_mut(&mut chunk.sections[si]);
            let data = match states.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            if let Ok(blocks) =
                PalettedContainer::from_packed(block_strategy(), palette, data.as_deref())
            {
                target.blocks = blocks;
            }
        }
        if let Some(b) = section.get("biomes")
            && let Some(Tag::List(palette)) = b.get("palette")
        {
            let ids: Vec<u16> = palette
                .iter()
                .map(|p| match p {
                    Tag::String(s) => biomes.saved_id(&s.to_string_lossy()),
                    _ => 0,
                })
                .collect();
            let data = match b.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            let palette: Vec<u32> = ids.iter().map(|&id| u32::from(id)).collect();
            let target = Arc::make_mut(&mut chunk.sections[si]);
            if let Ok(container) =
                PalettedContainer::from_packed(BIOME_STRATEGY, palette.clone(), data.as_deref())
            {
                target.biomes = container;
            } else if let Some(&first) = palette.first() {
                target.biomes = PalettedContainer::new(BIOME_STRATEGY, first);
            }
        }
    }
    let mut extras = ChunkExtras {
        last_update: number(tag.get("LastUpdate")).unwrap_or(0),
        ..ChunkExtras::default()
    };
    if let Some(Tag::List(entities)) = tag.get("block_entities") {
        for e in entities {
            if let Some(at) = block_entity_order::saved_at(e, [*x, *z]) {
                extras.block_entity_order.push(at);
            }
            let int = |k: &str| number(e.get(k)).map(|v| v as i32);
            let decoded = if let (Some(x), Some(y), Some(z)) = (int("x"), int("y"), int("z")) {
                let state = chunk.block(x & 15, y, z & 15);
                registry
                    .state(state)
                    .and_then(|block| decode_block_entity_for_block(e, &block.name))
                    .map(|(_, be)| (block_entity_pos((x, y, z), [chunk.x, chunk.z]), be))
            } else {
                None
            };
            match decoded {
                Some(be) => {
                    if let Ok(Some(data)) = attachment_compound(e.get(ATTACHMENTS_KEY)) {
                        extras.block_entity_attachments.push((
                            be.0,
                            sunrise_mod_runtime::attachments::Attachments::from_nbt(data.clone()),
                        ));
                    }
                    let name = if matches!(
                        be.1,
                        crate::containers::BlockEntity::Banner(_)
                            | crate::containers::BlockEntity::Sign(_)
                            | crate::containers::BlockEntity::Mod(_)
                    ) {
                        text(e.get("CustomName")).map(|name| string(&name))
                    } else {
                        e.get("CustomName")
                            .and_then(crate::block_entity_nbt::component)
                    };
                    if let Some(name) = name {
                        chunk.block_names.push((be.0, name));
                    }
                    let randomizable = matches!(be.1, crate::containers::BlockEntity::Chest(_));
                    if randomizable && let Some(pending) = PendingContainerLoot::from_tag(e) {
                        extras.pending_loot.insert(be.0, pending);
                    }
                    let mut source = e.clone();
                    if randomizable && let Tag::Compound(map) = &mut source {
                        map.remove(&"LootTable".into());
                        map.remove(&"LootTableSeed".into());
                    }
                    extras.block_entity_sources.push((be.0, source));
                    chunk.block_entities.push(be);
                }
                None => extras.block_entities.push(e.clone()),
            }
        }
    }
    if let Tag::Compound(map) = tag {
        if let Ok(Some(data)) = attachment_compound(map.get(&ATTACHMENTS_KEY.into())) {
            extras.attachments =
                sunrise_mod_runtime::attachments::Attachments::from_nbt(data.clone());
        }
        extras.root = map
            .iter()
            .filter(|(k, _)| !GENERATED_KEYS.contains(&k.to_string_lossy().as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }
    if let Some(t) = tables {
        extras.block_ticks = read_ticks(tag.get("block_ticks"), (*x, *z), |n| {
            t.block_ids.get(n).map(|&i| u32::from(i))
        });
        extras.fluid_ticks = read_ticks(tag.get("fluid_ticks"), (*x, *z), fluid_id);
    }
    chunk.saved = Some(Arc::new(extras));
    Some(chunk)
}

pub fn encode_proto(
    proto: &SavedProto,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    items: &crate::items::Items,
) -> NamedTag {
    let mut root = encode_with(&proto.column, registry, biomes, tables, shape, items);
    let status = if proto.decorated {
        ChunkStatus::Features
    } else {
        ChunkStatus::Terrain
    };
    // ProtoChunk.postProcessing: one list per section, packed section offsets.
    let mut marks = vec![Vec::new(); shape.section_count()];
    for &[x, y, z] in &proto.column.postprocess {
        let section = usize::try_from((y - shape.min_y) >> 4).ok();
        if let Some(list) = section.and_then(|s| marks.get_mut(s)) {
            list.push(Tag::Short(
                ((x & 15) | (y & 15) << 4 | (z & 15) << 8) as i16,
            ));
        }
    }
    for (key, value) in [
        ("Status", string(status.name())),
        ("isLightOn", Tag::Byte(0)),
        ("block_entities", Tag::List(proto.block_entities.clone())),
        ("entities", Tag::List(proto.entities.clone())),
        (
            "PostProcessing",
            Tag::List(marks.into_iter().map(Tag::List).collect()),
        ),
    ] {
        put(&mut root.tag, key, value);
    }
    root
}
/// A proto chunk generation can resume: `terrain` (written to by a
/// neighbour's decoration) up to `spawn`. Earlier statuses hold nothing that
/// generating the chunk again doesn't reproduce.
pub fn decode_proto(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<SavedProto> {
    let status = ChunkStatus::parse(&text(root.tag.get("Status"))?).ok()?;
    if !(ChunkStatus::Terrain..ChunkStatus::Full).contains(&status) {
        return None;
    }
    let mut column = decode_column(root, registry, biomes, tables, layout)?;
    column.block_entities.clear();
    let base = number(root.tag.get("yPos")).map_or(column.min_y(), |y| y as i32 * 16);
    if let Some(Tag::List(sections)) = root.tag.get("PostProcessing") {
        for (si, marks) in sections.iter().enumerate() {
            let Tag::List(marks) = marks else {
                continue;
            };
            for mark in marks {
                if let Tag::Short(p) = mark {
                    let p = i32::from(*p);
                    column.postprocess.push([
                        column.x * 16 + (p & 15),
                        base + si as i32 * 16 + ((p >> 4) & 15),
                        column.z * 16 + ((p >> 8) & 15),
                    ]);
                }
            }
        }
    }
    let list = |key: &str| match root.tag.get(key) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    Some(SavedProto {
        column,
        decorated: status >= ChunkStatus::Features,
        block_entities: list("block_entities"),
        entities: list("entities"),
    })
}

/// Read the `SkyLight`/`BlockLight` byte arrays of a chunk saved with
/// `isLightOn` (`SerializableChunkData.read`, which queues the arrays via
/// `ThreadedLevelLightEngine.queueSectionData`). The render cache inherits missing
/// sky light from the bottom row of the next stored layer above, or gets full
/// sky above all stored layers. Empty layers omitted by NBT are restored from
/// published section status when the chunk's light is initialized.
pub fn load_light(tag: &Tag, chunk: &mut Chunk, sky_light: bool) {
    if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
        return;
    }
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return;
    };
    let mut layers = vec![(None, None); chunk.sections.len() + 2];
    let first = (chunk.min_y() >> 4) - 1;
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let Some(slot) = y
            .checked_sub(i64::from(first))
            .and_then(|b| usize::try_from(b).ok())
            .and_then(|b: usize| layers.get_mut(b))
        else {
            continue;
        };
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
            _ => None,
        };
        *slot = (layer("SkyLight"), layer("BlockLight"));
    }
    crate::light::light_from_saved_layers(chunk, &layers, sky_light);
}

type RegionCache = Mutex<RegionFiles>;
/// `RegionFileStorage.MAX_CACHE_SIZE`: the region files of a folder kept
/// open. Each holds a file descriptor: kept for every region ever visited,
/// a server whose players travel runs out of them (`Too many open files`).
const MAX_OPEN_REGIONS: usize = 256;
/// A folder's region files, and the ones known to be missing, by region,
/// with when each was last asked for.
#[derive(Default)]
struct RegionFiles {
    regions: HashMap<(i32, i32), (Option<Arc<Mutex<RegionFile>>>, u64)>,
    clock: u64,
    /// Each region's write stamp (`far_regions::writing`).
    written: HashMap<(i32, i32), u64>,
    #[cfg(test)]
    close_on_evict: Option<fn(RegionFile) -> std::io::Result<()>>,
}
impl RegionFiles {
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
    /// The open region files.
    #[cfg(test)]
    fn open(&self) -> usize {
        self.regions.values().filter(|(r, _)| r.is_some()).count()
    }
    /// `RegionFileStorage.cache`: remove the least recently used region before closing it.
    /// A region a reader or the writer still holds stays: closing it under them would let a
    /// second handle read a header older than its sectors.
    fn make_room(&mut self) -> Vec<((i32, i32), std::io::Error)> {
        let close = RegionFile::close;
        #[cfg(test)]
        let close = self.close_on_evict.unwrap_or(close);
        let mut failures = Vec::new();
        while self.regions.len() >= MAX_OPEN_REGIONS {
            let oldest = self
                .regions
                .iter()
                .filter(|(_, (r, _))| r.as_ref().is_none_or(|r| Arc::strong_count(r) == 1))
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key);
            let Some(key) = oldest else {
                break;
            };
            let (file, _) = self.regions.remove(&key).unwrap();
            if let Some(file) = file {
                // The cache owns the only reference, as checked above under its lock.
                let closed = Arc::into_inner(file)
                    .unwrap()
                    .into_inner()
                    .map_err(|_| std::io::Error::other("region file poisoned"))
                    .and_then(close);
                if let Err(error) = closed {
                    failures.push((key, error));
                }
            }
        }
        failures
    }
}

#[derive(Clone, Copy)]
struct RegionStorage<'a> {
    root: &'a Path,
    folder: Folder,
    cache: &'a RegionCache,
    reporting: &'a writes::SaveReporting,
}

impl RegionStorage<'_> {
    /// Readers and writers share a cache, so a read never sees a header older than the sectors
    /// it points to. `create` makes a missing file; otherwise a missing file is `None`.
    fn open_cached(
        self,
        (rx, rz): (i32, i32),
        create: bool,
    ) -> std::io::Result<Option<Arc<Mutex<RegionFile>>>> {
        let key = (rx, rz);
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| std::io::Error::other("region cache poisoned"))?;
        cache.clock += 1;
        let now = cache.clock;
        if let Some((cached, used)) = cache.regions.get_mut(&key)
            && (cached.is_some() || !create)
        {
            *used = now;
            return Ok(cached.clone());
        }
        let failed_evictions = cache.make_room();
        let opened = (|| {
            let path = region_path(self.root, self.folder.name(), rx, rz);
            if create && let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            // A file this process can't write (a read-only world) is still read.
            let region = match RegionFile::open_writable(&path, create) {
                Err(e) if !create && e.kind() == std::io::ErrorKind::PermissionDenied => {
                    RegionFile::open(&path)
                }
                other => other,
            };
            let region = match region {
                Ok(r) => Some(Arc::new(Mutex::new(r))),
                Err(e) if !create && e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            cache.regions.insert(key, (region.clone(), now));
            Ok(region)
        })();
        drop(cache);
        // Isolate the evicted region's failure: the requested region must use its own result.
        for ((rx, rz), error) in failed_evictions {
            let error = std::io::Error::new(
                error.kind(),
                format!(
                    "Failed to close region [{rx}, {rz}] in {}: {error}",
                    self.folder.name()
                ),
            );
            self.reporting
                .report(self.folder, (rx << 5, rz << 5), &error);
        }
        opened
    }
}
/// Write `records` into the region files of `folder` in place, as vanilla's
/// `RegionFile.write` does: each chunk takes free sectors and the header
/// then points to it; the region's other chunks are not touched. `None`
/// clears a chunk. The chunks are encoded and compressed before their region
/// is locked for the writes (`compress_records`), and each region is flushed
/// to the disk once. An unencodable record is skipped, leaving its old slot alone.
fn write_region_records(
    storage: RegionStorage<'_>,
    records: &[((i32, i32), &Record)],
    sync_data: &impl Fn(&fs::File) -> std::io::Result<()>,
) -> RegionWriteOutcome {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let compressed = compress_records(records, timestamp);
    write_prepared_region_records(storage, timestamp, records, compressed, sync_data)
}

#[derive(Default)]
struct RegionWriteOutcome {
    failed_records: BTreeMap<(i32, i32), std::io::Error>,
    sync_error: Option<std::io::Error>,
}

fn write_prepared_region_records(
    storage: RegionStorage<'_>,
    timestamp: u32,
    records: &[((i32, i32), &Record)],
    compressed: Vec<compression_pool::Prepared>,
    sync_data: &impl Fn(&fs::File) -> std::io::Result<()>,
) -> RegionWriteOutcome {
    type RegionWrite<'a> = (ChunkPos, &'a Record, Option<CompressedWrite>);
    let mut by_region: BTreeMap<(i32, i32), Vec<RegionWrite<'_>>> = BTreeMap::new();
    let mut outcome = RegionWriteOutcome::default();
    let failed = &mut outcome.failed_records;
    for (&((x, z), source), record) in records.iter().zip(compressed) {
        match record {
            Ok(record) => by_region.entry((x >> 5, z >> 5)).or_default().push((
                ChunkPos::new(x, z),
                source,
                record,
            )),
            Err(error) => {
                failed.insert((x, z), error);
            }
        }
    }
    for ((rx, rz), mut list) in by_region {
        // Clearing chunks of a region that doesn't exist changes nothing.
        let create = list.iter().any(|(_, _, r)| r.is_some());
        let result: std::io::Result<()> = (|| {
            let Some(region) = storage.open_cached((rx, rz), create)? else {
                return Ok(());
            };
            // Keep far readers on the exact locked read path until the flush completes.
            let _writing = far_regions::writing(storage.cache, (rx, rz));
            let mut region = region
                .lock()
                .map_err(|_| std::io::Error::other("region file poisoned"))?;
            let mut needs_sync = false;
            for (pos, source, record) in &mut list {
                let changed = match record.take() {
                    Some(record) => {
                        write_chunk_record(&mut region, *pos, source, record).map(|()| true)
                    }
                    None => region.clear(*pos, timestamp),
                };
                match changed {
                    Ok(changed) => needs_sync |= changed,
                    Err(error) => {
                        failed.insert((pos.x, pos.z), error);
                    }
                }
            }
            if needs_sync {
                let synced = match region.sync_handle() {
                    Ok(handle) => {
                        drop(region);
                        sync_data(&handle)
                    }
                    // Out of descriptors: flush under the lock rather than skip durability.
                    Err(_) => region.sync(),
                };
                if let Err(error) = synced {
                    log::warn!(
                        "Failed to synchronize {} region [{rx}, {rz}]: {error}",
                        storage.folder.name()
                    );
                    outcome.sync_error.get_or_insert(error);
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            // Open or lock failed before any record reached this region.
            let error = Arc::new(error);
            for (pos, _, _) in list {
                failed.insert(
                    (pos.x, pos.z),
                    std::io::Error::new(error.kind(), error.clone()),
                );
            }
        }
    }
    outcome
}

fn write_chunk_record(
    region: &mut RegionFile,
    pos: ChunkPos,
    source: &Record,
    CompressedWrite { record, primary }: CompressedWrite,
) -> std::io::Result<()> {
    region.write(pos, &record)?;
    if primary {
        source.accept_block_entity_edits();
    }
    Ok(())
}

/// `job(i)` for every `i` below `count`, in order, shared between this thread
/// and up to `helpers - 1` scoped threads, each taking the next index as it
/// finishes one. A helper the system refuses to start (`start` false, or a
/// container at its limit of threads: EAGAIN) leaves its share to the
/// others: this thread does all of it at worst, and never panics for it, as
/// `thread::scope`'s own `spawn` would, which stopped the region writer.
#[cfg(test)]
fn share_out<R: Send>(
    count: usize,
    helpers: usize,
    job: impl Fn(usize) -> R + Sync,
    mut start: impl FnMut() -> bool,
) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let work = || {
        let mut out = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= count {
                return out;
            }
            out.push((i, job(i)));
        }
    };
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let threads: Vec<_> = (1..helpers)
            .filter(|_| start())
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, work).ok())
            .collect();
        let mut done = work();
        for t in threads {
            done.extend(t.join().unwrap_or_else(|e| std::panic::resume_unwind(e)));
        }
        done
    });
    done.sort_unstable_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

fn region_path(root: &Path, folder: &str, rx: i32, rz: i32) -> PathBuf {
    root.join(format!("{folder}/r.{rx}.{rz}.mca"))
}

/// A queued POI record as it is written: claims kept from the saved one, and
/// a chunk that lost its last POI gets an empty one, as vanilla trusts a
/// saved section. None when the chunk never had any.
fn resolve_poi(storage: RegionStorage<'_>, (x, z): (i32, i32), record: &Record) -> Option<Record> {
    let Record::Poi(chunk, tables, professions) = record else {
        return Some(record.clone());
    };
    let old = read_region_record(storage, x, z)
        .and_then(|old| {
            old.map(|mut old| {
                upgrade_record(Folder::Poi, &mut old, || None)?;
                Ok(old)
            })
            .transpose()
        })
        .unwrap_or_else(|e| {
            log::warn!("Sunrise: could not read the poi record of chunk {x} {z}: {e}");
            None
        });
    let tag = match (encode_poi(chunk, tables, professions.as_deref()), old) {
        (Some(mut tag), Some(old)) => {
            keep_claims(&mut tag, &old);
            tag
        }
        (Some(tag), None) => tag,
        (None, Some(_)) => poi_root(Compound::new()),
        (None, None) => return None,
    };
    Some(Record::Tag(Arc::new(tag)))
}
fn read_region_record(storage: RegionStorage<'_>, x: i32, z: i32) -> Result<Option<NamedTag>> {
    let Some(region) = storage.open_cached((x >> 5, z >> 5), false)? else {
        return Ok(None);
    };
    let record = region
        .lock()
        .map_err(|_| "region file poisoned")?
        .read(ChunkPos::new(x, z))?;
    Ok(match record {
        Some(r) => Some(r.nbt(Limits::saved())?),
        None => None,
    })
}
/// The `__context` the chunk fixes read: the dimension and its generator type
/// (`ChunkMap.getChunkDataFixContextTag`).
struct DatafixContext {
    dimension: String,
    generator: Option<String>,
}
impl DatafixContext {
    /// From a dimension folder made by [`dimension_dir`] and the world's
    /// `world_gen_settings.dat`; None for a folder outside a world's `dimensions/`.
    fn of(root: &Path) -> Option<Self> {
        let mut parts = Vec::new();
        let mut at = root;
        while at.file_name()? != "dimensions" {
            parts.push(at.file_name()?.to_str()?);
            at = at.parent()?;
        }
        let world = at.parent()?;
        let namespace = parts.pop()?;
        if parts.is_empty() {
            return None;
        }
        parts.reverse();
        let dimension = format!("{namespace}:{}", parts.join("/"));
        let generator = sunrise_world::datafix::files::generators(world)
            .ok()
            .and_then(|g| g.into_iter().find(|(d, _)| *d == dimension))
            .map(|(_, g)| g);
        Some(Self {
            dimension,
            generator,
        })
    }
}
/// `SimpleRegionStorage.upgradeChunkTag`, as `ChunkMap.upgradeChunkTag` (chunks),
/// `EntityStorage` and `SectionStorage.PackedChunk.parse` (POI) call it on every
/// record they read: a world 26.3 has opened keeps the chunks nobody loaded since
/// an older version in that version's format, and `upgrade_older_world` only runs
/// for an older level.dat. A record without a DataVersion counts as -1 (POI: 1945).
/// `context` is needed only for an older chunk; without one it stays as saved.
fn upgrade_record<'a>(
    folder: Folder,
    record: &mut NamedTag,
    context: impl FnOnce() -> Option<&'a DatafixContext>,
) -> Result<()> {
    use sunrise_world::datafix::{self, schema::Ty};
    horizon_names::adopt_in_record(folder, &mut record.tag);
    let default = if folder == Folder::Poi { 1945 } else { -1 };
    if number(record.tag.get("DataVersion")).unwrap_or(default) >= i64::from(DATA_VERSION) {
        return Ok(());
    }
    let context = match folder {
        Folder::Region => match context() {
            Some(c) => Some(c),
            None => return Ok(()),
        },
        _ => None,
    };
    let tag = &mut record.tag;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match (folder, context) {
        (Folder::Region, Some(c)) => {
            datafix::update_chunk(tag, &c.dimension, c.generator.as_deref());
        }
        (Folder::Entities, _) => datafix::update_region_tag(Ty::EntityChunk, tag),
        (Folder::Poi, _) => datafix::update_region_tag(Ty::PoiChunk, tag),
        (Folder::Region, None) => {}
    }))
    .map_err(|_| "upgrading an older record failed".into())
}
/// Chunk and entity records waiting for the writer thread, by (entities?,
/// x, z): reads see them before the region files do.
type PendingWrites = Mutex<HashMap<(Folder, i32, i32), (u64, Record)>>;
/// The region folders of a dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Folder {
    Region,
    Entities,
    Poi,
}
impl Folder {
    fn of(name: &str) -> Self {
        match name {
            "entities" => Self::Entities,
            "poi" => Self::Poi,
            _ => Self::Region,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::Entities => "entities",
            Self::Poi => "poi",
        }
    }
}

/// One dimension folder (`dimensions/minecraft/<name>`). Region files are
/// read lazily by worker threads.
pub struct SaveDir {
    pub root: PathBuf,
    save_reporting: writes::SaveReporting,
    /// The loaded catalog for stack encoding; absent for metadata-only readers.
    items: Option<Arc<crate::items::Items>>,
    regions: Arc<RegionCache>,
    entity_regions: Arc<RegionCache>,
    /// `PersistentEntitySectionManager` leaves a failed read pending for the life of the level.
    failed_entity_reads: Mutex<HashSet<(i32, i32)>>,
    poi_regions: Arc<RegionCache>,
    pending: Arc<PendingWrites>,
    sequence: AtomicU64,
    disk: OnceLock<Arc<chunk_disk::DiskRuntime>>,
    /// The server report destination shared by all dimensions and their I/O workers.
    writer: Mutex<Option<Writer>>,
    outstanding: Arc<Outstanding>,
    /// Set by `Persistent` before the first load.
    pub tables: OnceLock<Arc<SaveTables>>,
    professions: std::sync::RwLock<Option<Arc<crate::mobs::villager::ResolvedProfessions>>>,
    trusted_fluid_load: bool,
    /// The dimension's skylight and height, from its folder name.
    pub shape: LevelShape,
    /// The region file headers kept for far readers (`far_record`).
    far_regions: far_regions::FarRegions,
    /// Saved chunks decoded for far terrain, shared by its sampling threads
    /// (`lod_surfaces_shared`), oldest first.
    lod_decoded: Mutex<LodDecoded>,
    /// What older chunks are upgraded with, found on the first one read.
    datafix_context: OnceLock<Option<DatafixContext>>,
    /// The Overworld's old-world blending cache, read when a save is requested
    /// (installed where `SavedOldWorld` is made for this same folder).
    blending_lookup: OnceLock<BlendingLookup>,
}
fn intern_saved_fluids(root: &NamedTag) {
    if let Some(Tag::List(entities)) = root.tag.get("block_entities") {
        for entity in entities {
            if let Some(Tag::List(tanks)) = entity.get("fluids") {
                for tank in tanks {
                    if let Some(Tag::String(id)) = tank.get("Fluid") {
                        mod_api::__private::intern_unresolved_fluid(&id.to_string_lossy());
                    }
                }
            }
        }
    }
}

fn mod_ticket_to_nbt(ticket: &ModTicket, controller: &str) -> Tag {
    let owner = match ticket.owner {
        TicketOwner::Block(pos) => ("block", Tag::IntArray(pos.to_vec())),
        TicketOwner::Entity(uuid) => (
            "entity",
            Tag::IntArray((0..4).map(|i| (uuid >> (96 - 32 * i)) as i32).collect()),
        ),
    };
    Tag::compound([
        ("controller", Tag::String(controller.into())),
        ("owner", Tag::compound([owner])),
        (
            "chunk_pos",
            Tag::IntArray(vec![ticket.chunk.0, ticket.chunk.1]),
        ),
        (
            "natural_spawning",
            Tag::Byte(i8::from(ticket.natural_spawning)),
        ),
    ])
}
/// A saved ticket: the id of the controller that held it, and the ticket without its slot.
fn mod_ticket_from_nbt(tag: &Tag) -> Option<(String, ModTicket)> {
    let Some(Tag::String(controller)) = tag.get("controller") else {
        return None;
    };
    let owner = tag.get("owner")?;
    let owner = match (owner.get("block"), owner.get("entity")) {
        (Some(Tag::IntArray(p)), _) if p.len() == 3 => TicketOwner::Block([p[0], p[1], p[2]]),
        (_, Some(Tag::IntArray(u))) if u.len() == 4 => TicketOwner::Entity(
            u.iter()
                .fold(0u128, |acc, &n| acc << 32 | u128::from(n as u32)),
        ),
        _ => return None,
    };
    let Some(Tag::IntArray(pos)) = tag.get("chunk_pos") else {
        return None;
    };
    let controller = controller.to_string().ok()?;
    (pos.len() == 2).then(|| {
        let ticket = ModTicket {
            controller: 0,
            owner,
            chunk: (pos[0], pos[1]),
            // Tickets saved without the flag don't force natural spawning.
            natural_spawning: matches!(tag.get("natural_spawning"), Some(Tag::Byte(b)) if *b != 0),
        };
        (controller, ticket)
    })
}
fn is_forced_ticket(ticket: &Tag) -> bool {
    matches!(ticket.get("type"), Some(Tag::String(t)) if *t == JavaString::from("minecraft:forced"))
}
fn is_portal_ticket(ticket: &Tag) -> bool {
    matches!(ticket.get("type"), Some(Tag::String(t)) if *t == JavaString::from("minecraft:portal"))
}
#[derive(Default)]
struct LodDecoded {
    chunks: crate::fast_hash::FxHashMap<(i32, i32), (std::time::Instant, LodSurfaces)>,
    order: std::collections::VecDeque<(i32, i32)>,
}
/// Saved chunks kept decoded for far terrain (each tens of kilobytes), and
/// for how long: edits saved later show after it.
const LOD_DECODED: usize = 1024;
const LOD_DECODED_TTL: std::time::Duration = std::time::Duration::from_secs(30);
impl SaveDir {
    /// This dimension's server-owned mod attachments: `data/sunrise/attachments.dat`, Sunrise's
    /// own namespace. A file that doesn't read is moved aside to `attachments.dat.corrupt`, with
    /// a warning, and the level opens without them (as vanilla's `SavedDataStorage` starts a
    /// saved data over when its file is broken), so the next save can't overwrite it.
    pub fn load_attachments(&self) -> sunrise_mod_runtime::attachments::Attachments {
        let mut path = self.attachments_path();
        let mut read = try_read_dat(&path);
        if matches!(read, Err(DatIssue::Missing)) {
            // A dimension Horizon saved: its file, written under Sunrise's name at the next save.
            path = horizon_names::horizon_attachments_path(&self.root);
            read = try_read_dat(&path);
        }
        let data = match read {
            Err(DatIssue::Missing) => return Default::default(),
            Ok(root) => match root.get("data") {
                Some(Tag::Compound(data)) => Ok(data.clone()),
                _ => Err("no data compound".to_owned()),
            },
            Err(DatIssue::Unreadable(why)) => Err(why),
        };
        data.map(sunrise_mod_runtime::attachments::Attachments::from_nbt)
            .unwrap_or_else(|why| {
                let aside = path.with_extension("dat.corrupt");
                log::warn!(
                    "{}: {why}; mod attachments start over, the file is kept as {}",
                    path.display(),
                    aside.display()
                );
                let _ = fs::rename(&path, &aside);
                Default::default()
            })
    }

    /// Save this dimension's server-owned mod attachments; nothing while it has none and never had.
    pub fn save_attachments(
        &self,
        attachments: &sunrise_mod_runtime::attachments::Attachments,
    ) -> Result<()> {
        let path = self.attachments_path();
        let data = attachments.to_nbt();
        // Emptied after a Horizon file was read: an empty file under Sunrise's name keeps it unread.
        if data.is_empty()
            && !path.exists()
            && !horizon_names::horizon_attachments_path(&self.root).exists()
        {
            return Ok(());
        }
        write_dat(
            &path,
            Tag::compound([
                ("data", Tag::Compound(data)),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
    }

    fn attachments_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("sunrise")
            .join("attachments.dat")
    }

    /// The saves of a level of `shape` at `root` (its dimension folder).
    pub fn new(root: PathBuf, shape: LevelShape, items: Option<Arc<crate::items::Items>>) -> Self {
        Self {
            save_reporting: writes::SaveReporting::new(
                root.clone(),
                "minecraft:overworld",
                Arc::default(),
                crate::chunk_io::PROCESS_REPORTS.clone(),
            ),
            root,
            items,
            regions: Default::default(),
            entity_regions: Default::default(),
            failed_entity_reads: Default::default(),
            poi_regions: Default::default(),
            pending: Default::default(),
            sequence: AtomicU64::new(0),
            disk: OnceLock::new(),
            writer: Mutex::new(None),
            outstanding: Default::default(),
            tables: OnceLock::new(),
            professions: Default::default(),
            trusted_fluid_load: false,
            shape,
            far_regions: Default::default(),
            lod_decoded: Default::default(),
            datafix_context: OnceLock::new(),
            blending_lookup: OnceLock::new(),
        }
    }
    /// Share the world's record diagnostic sink across its levels, before any writes.
    pub fn set_save_reporting(
        &mut self,
        world_root: PathBuf,
        dimension: &'static str,
        reporter: Arc<crate::chunk_io::SaveFailureReporter>,
        inbox: Arc<crate::chunk_io::Reports>,
    ) {
        self.save_reporting = writes::SaveReporting::new(world_root, dimension, reporter, inbox);
    }

    pub fn chunk_reports(&self) -> &Arc<crate::chunk_io::Reports> {
        &self.save_reporting.inbox
    }

    pub fn save_failure_reporter(&self) -> &Arc<crate::chunk_io::SaveFailureReporter> {
        &self.save_reporting.reporter
    }

    fn region_storage(&self, folder: Folder) -> RegionStorage<'_> {
        RegionStorage {
            root: &self.root,
            folder,
            cache: match folder {
                Folder::Region => &self.regions,
                Folder::Entities => &self.entity_regions,
                Folder::Poi => &self.poi_regions,
            },
            reporting: &self.save_reporting,
        }
    }

    fn items_for_save(&self) -> Result<&Arc<crate::items::Items>> {
        self.items
            .as_ref()
            .ok_or_else(|| "cannot save chunks without a loaded item catalog".into())
    }
    /// Mark this as a server-owned world save, not a remote cache.
    pub fn enable_trusted_fluid_load(&mut self) {
        self.trusted_fluid_load = true;
    }
    /// Lets saves of this folder take an old chunk's calculated blending data
    /// (`lookup` must not calculate). Set once, before the folder saves.
    pub fn set_blending_lookup(&self, lookup: BlendingLookup) {
        let _ = self.blending_lookup.set(lookup);
    }
    /// The calculated blending data of the old chunk at `x, z`, if this folder
    /// has a lookup and its cache holds one.
    pub fn calculated_blending(&self, x: i32, z: i32) -> Option<Arc<BlendingData>> {
        self.blending_lookup.get().and_then(|lookup| lookup(x, z))
    }
    /// 26.3's TicketStorage: persistent tickets live in each dimension's
    /// data/minecraft/chunk_tickets.dat, with ChunkPos.CODEC's [x, z] int array.
    pub fn load_forced_chunks(&self) -> HashSet<(i32, i32)> {
        let Some(data) = read_saved_data(&self.root, "chunk_tickets") else {
            return HashSet::new();
        };
        let Some(Tag::List(tickets)) = data.get("tickets") else {
            return HashSet::new();
        };
        tickets
            .iter()
            .filter(|t| is_forced_ticket(t))
            .filter_map(|t| match t.get("chunk_pos")? {
                Tag::IntArray(pos) if pos.len() == 2 => Some((pos[0], pos[1])),
                _ => None,
            })
            .collect()
    }
    /// The saved `PORTAL` tickets, for `TicketStorage.activateAllDeactivatedTickets`
    /// (`Ticket.CODEC`: a non-negative level, `ticks_left` 0 when missing).
    pub fn load_portal_tickets(&self) -> Vec<crate::ticket_levels::PortalTicket> {
        let Some(Tag::List(tickets)) =
            read_saved_data(&self.root, "chunk_tickets").and_then(|d| d.get("tickets").cloned())
        else {
            return Vec::new();
        };
        tickets
            .iter()
            .filter(|t| is_portal_ticket(t))
            .filter_map(|t| {
                let chunk = match t.get("chunk_pos")? {
                    Tag::IntArray(pos) if pos.len() == 2 => (pos[0], pos[1]),
                    _ => return None,
                };
                let level = match t.get("level")? {
                    &Tag::Int(level) if level >= 0 => level,
                    _ => return None,
                };
                let ticks_left = match t.get("ticks_left") {
                    None => 0,
                    Some(&Tag::Long(n)) => n,
                    Some(&Tag::Int(n)) => n.into(),
                    Some(&Tag::Short(n)) => n.into(),
                    Some(&Tag::Byte(n)) => n.into(),
                    Some(_) => return None,
                };
                Some(crate::ticket_levels::PortalTicket {
                    chunk,
                    level,
                    ticks_left,
                })
            })
            .collect()
    }
    /// `TicketStorage.packTickets`: the forced chunks and the `PORTAL` tickets
    /// the level holds now, in place of the ones saved before.
    pub fn save_chunk_tickets(
        &self,
        forced: &HashSet<(i32, i32)>,
        portal: &[crate::ticket_levels::PortalTicket],
    ) -> Result<()> {
        if forced.is_empty()
            && portal.is_empty()
            && !saved_data_path(&self.root, "chunk_tickets").exists()
        {
            return Ok(());
        }
        let mut data = compound_or_empty(read_saved_data(&self.root, "chunk_tickets"));
        // Preserve persistent tickets Sunrise does not simulate.
        let mut tickets = match data.get("tickets") {
            Some(Tag::List(tickets)) => tickets
                .iter()
                .filter(|t| !is_forced_ticket(t) && !is_portal_ticket(t))
                .cloned()
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let mut chunks: Vec<_> = forced.iter().copied().collect();
        chunks.sort_unstable();
        tickets.extend(chunks.into_iter().map(|(x, z)| {
            Tag::compound([
                ("type", Tag::String("minecraft:forced".into())),
                ("level", Tag::Int(31)), // ChunkMap.FORCED_TICKET_LEVEL: ENTITY_TICKING.
                ("chunk_pos", Tag::IntArray(vec![x, z])),
            ])
        }));
        tickets.extend(portal.iter().map(|t| {
            let mut ticket = Tag::compound([
                ("type", Tag::String("minecraft:portal".into())),
                ("level", Tag::Int(t.level)),
                ("chunk_pos", Tag::IntArray(vec![t.chunk.0, t.chunk.1])),
            ]);
            // `optionalFieldOf("ticks_left", 0L)` leaves out its default.
            if t.ticks_left != 0 {
                put(&mut ticket, "ticks_left", Tag::Long(t.ticks_left));
            }
            ticket
        }));
        put(&mut data, "tickets", Tag::List(tickets));
        write_saved_data(&self.root, "chunk_tickets", data)
    }
    /// The chunk tickets of mods (`data/sunrise/mod_chunk_tickets.dat`), kept
    /// out of vanilla's `chunk_tickets.dat` because vanilla decodes its ticket
    /// types through a registry. Each is saved under its controller's id;
    /// `slot_of` gives the slot of an id a loaded mod registered. A ticket of
    /// any other id (a mod that is gone, or a file from before controllers) is
    /// dropped with a warning, and one that doesn't read is skipped.
    pub fn load_mod_tickets(&self, slot_of: impl Fn(&str) -> Option<u32>) -> Vec<ModTicket> {
        let Some(root) = read_dat(&self.mod_tickets_path()) else {
            return Vec::new();
        };
        let Some(Tag::List(tickets)) = root.get("data").and_then(|data| data.get("tickets")) else {
            return Vec::new();
        };
        let mut dropped = std::collections::BTreeSet::new();
        let loaded = tickets
            .iter()
            .filter_map(|tag| {
                let Some((controller, ticket)) = mod_ticket_from_nbt(tag) else {
                    log::warn!("Dropping a saved mod chunk ticket that has no readable controller");
                    return None;
                };
                let Some(slot) = slot_of(&controller) else {
                    dropped.insert(controller);
                    return None;
                };
                Some(ModTicket {
                    controller: slot,
                    ..ticket
                })
            })
            .collect();
        for controller in dropped {
            log::warn!(
                "Dropping the saved chunk tickets of ticket controller {controller}: no loaded mod registers it"
            );
        }
        loaded
    }
    /// Save the tickets of mods under their controllers' ids (`id_of` gives the id of a slot);
    /// nothing while there are none and never were.
    pub fn save_mod_tickets(
        &self,
        tickets: &ModTickets,
        id_of: impl Fn(u32) -> Option<&'static str>,
    ) -> Result<()> {
        let path = self.mod_tickets_path();
        let list: Vec<Tag> = tickets
            .to_save()
            .filter_map(|t| Some(mod_ticket_to_nbt(t, id_of(t.controller)?)))
            .collect();
        if list.is_empty() && !path.exists() {
            return Ok(());
        }
        write_dat(
            &path,
            Tag::compound([
                ("data", Tag::compound([("tickets", Tag::List(list))])),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
    }
    fn mod_tickets_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("sunrise")
            .join("mod_chunk_tickets.dat")
    }
    fn region_path(&self, folder: &str, rx: i32, rz: i32) -> PathBuf {
        region_path(&self.root, folder, rx, rz)
    }
    /// The chunks of a region file that an older version generated, as
    /// `IOWorker.createOldDataForRegion` finds them (bit `x + z * 32`): saved
    /// before DataVersion 4997, or with `blending_data`.
    pub fn old_chunk_bits(&self, rx: i32, rz: i32) -> [u64; 16] {
        let mut bits = [0u64; 16];
        let Ok(region) = RegionFile::open(self.region_path("region", rx, rz)) else {
            return bits;
        };
        let present: Vec<ChunkPos> = region.chunks().map(|(pos, _)| pos).collect();
        drop(region);
        for pos in present {
            // `IOWorker` reads the record as saved, before any upgrade.
            let Some(root) = self.read_record_raw("region", pos.x, pos.z) else {
                continue;
            };
            if record_is_old(&root) {
                let index = ((pos.x & 31) + (pos.z & 31) * 32) as usize;
                bits[index / 64] |= 1 << (index % 64);
            }
        }
        bits
    }
    /// Whether the saved chunk at `x, z` is old ([`Self::old_chunk_bits`]'s test), from its
    /// record alone: false for a chunk not saved.
    pub fn is_old_chunk(&self, x: i32, z: i32) -> bool {
        self.read_record_raw("region", x, z)
            .is_some_and(|root| record_is_old(&root))
    }
    /// A saved chunk brought to the current version, for the readers of its
    /// fields that `load` does not decode (`blending_data`).
    pub fn raw_chunk(&self, x: i32, z: i32) -> Option<NamedTag> {
        self.read_record("region", x, z)
    }
    /// The references a saved chunk keeps for its structure start `name`
    /// (`StructureStart.getReferences`, saved as `references`): 0 for a chunk
    /// not saved yet, a start it does not have or one without the field.
    pub fn start_references(&self, x: i32, z: i32, name: &str) -> i32 {
        let Some(root) = self.read_record("region", x, z) else {
            return 0;
        };
        match root
            .tag
            .get("structures")
            .and_then(|s| s.get("starts"))
            .and_then(|s| s.get(name))
            .and_then(|s| s.get("references"))
        {
            Some(Tag::Int(n)) => *n,
            _ => 0,
        }
    }
    /// A record as saved: the pending write first, then the region file.
    fn read_record_raw(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        self.try_read_record_raw(folder, x, z, Record::tag_for_metadata)
            .inspect_err(|e| {
                self.chunk_reports()
                    .report_load_failure(&self.root, x, z, e)
            })
            .ok()
            .flatten()
    }
    /// `read_record_raw`, telling a record that isn't there (`Ok(None)`) from a read that failed.
    fn try_read_record_raw(
        &self,
        folder: &str,
        x: i32,
        z: i32,
        read_pending: fn(&Record) -> Option<Arc<NamedTag>>,
    ) -> Result<Option<NamedTag>> {
        let kind = Folder::of(folder);
        let pending = self
            .pending
            .lock()
            .map_err(|_| "pending writes poisoned")?
            .get(&(kind, x, z))
            .map(|(_, r)| r.clone());
        if let Some(record) = pending
            && let Ok(tag) = compression_pool::catch_encoding_panic(|| Ok(read_pending(&record)))
        {
            return Ok(tag.map(|t| (*t).clone()));
        }
        read_region_record(self.region_storage(kind), x, z).inspect(|record| {
            if self.trusted_fluid_load
                && kind == Folder::Region
                && let Some(root) = record
            {
                intern_saved_fluids(root);
            }
        })
    }
    /// A record brought to the current version (`upgrade_record`); `Err` when that failed.
    fn try_read_record(
        &self,
        folder: &str,
        x: i32,
        z: i32,
        read_pending: fn(&Record) -> Option<Arc<NamedTag>>,
    ) -> Result<Option<NamedTag>> {
        let Some(mut record) = self.try_read_record_raw(folder, x, z, read_pending)? else {
            return Ok(None);
        };
        self.upgrade_record(Folder::of(folder), &mut record)?;
        Ok(Some(record))
    }
    /// A record brought to the current version (`upgrade_record`).
    fn read_record(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        self.try_read_record(folder, x, z, Record::tag_for_metadata)
            .inspect_err(|e| {
                self.chunk_reports()
                    .report_load_failure(&self.root, x, z, e)
            })
            .ok()
            .flatten()
    }
    fn upgrade_record(&self, folder: Folder, record: &mut NamedTag) -> Result<()> {
        upgrade_record(folder, record, || {
            self.datafix_context
                .get_or_init(|| DatafixContext::of(&self.root))
                .as_ref()
        })
    }
    /// `ChunkMap.handleChunkLoadFailure`: once this chunk's read failure has been reported,
    /// let generation replace it, as `ChunkMap.createEmptyChunk` does.
    pub fn load(&self, x: i32, z: i32, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
        self.try_load(x, z, registry, biomes).ok().flatten()
    }
    /// `load`, telling a chunk that isn't saved as a full chunk (`Ok(None)`) from a read that
    /// failed (`Err`): a later read may succeed.
    fn try_load(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> std::result::Result<Option<Chunk>, ()> {
        let Some(root) = self
            .try_read_record("region", x, z, Record::tag)
            .map_err(|e| {
                self.chunk_reports()
                    .report_load_failure(&self.root, x, z, &e)
            })?
        else {
            return Ok(None);
        };
        let tables = self.tables.get().map(|t| &**t);
        let Some(mut chunk) = decode_with(&root, registry, biomes, tables, self.shape.column())
        else {
            return Ok(None);
        };
        load_light(&root.tag, &mut chunk, self.shape.sky_light);
        chunk.entities = sunrise_world::chunk::legacy_entities(&root.tag);
        // A failing load listener does not fail the load: the chunk keeps its stored attachments.
        self.load_disk_attachments(&mut chunk, mod_api::ChunkDiskKind::Full)
            .map_err(|_| ())?;
        Ok(Some(chunk))
    }
    /// A proto chunk saved before it became final (`decode_proto`).
    pub fn load_proto(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Option<SavedProto> {
        let root = self.read_record("region", x, z)?;
        let tables = self.tables.get().map(|t| &**t);
        let mut proto = decode_proto(&root, registry, biomes, tables, self.shape.column())?;
        self.load_disk_attachments(&mut proto.column, mod_api::ChunkDiskKind::Proto)
            .ok()?;
        Some(proto)
    }
    /// Queue proto chunks for the writer thread; loads see them meanwhile.
    pub fn save_protos(
        &self,
        protos: Vec<SavedProto>,
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<()> {
        biomes.ensure_saveable()?;
        let encoding = Arc::new(Encoding {
            items: self.items_for_save()?.clone(),
            registry,
            biomes: biomes.clone(),
            tables: self.tables.get().cloned(),
            shape: self.shape,
        });
        let records = protos
            .into_iter()
            .map(|p| {
                (
                    (p.column.x, p.column.z),
                    Record::Proto(Arc::new(p), encoding.clone()),
                )
            })
            .collect();
        self.queue(Folder::Region, records, false)
    }
    /// `EntityStorage.loadEntities`: read `entities/` separately from the terrain.
    /// An absent record is empty; a failed read leaves the entity chunk pending until the level closes.
    pub fn load_entities(&self, x: i32, z: i32) -> Result<Vec<Tag>> {
        if self.failed_entity_reads.lock().unwrap().contains(&(x, z)) {
            return Err("an earlier entity chunk read failed".into());
        }
        let record = self
            .try_read_record("entities", x, z, Record::tag_for_metadata)
            .inspect_err(|e| {
                self.failed_entity_reads.lock().unwrap().insert((x, z));
                log::error!("Failed to load entity chunk {x},{z}: {e}");
                self.chunk_reports()
                    .report_load_failure(&self.root, x, z, e);
            })?;
        let mut out = Vec::new();
        if let Some(root) = record
            && let Some(Tag::List(list)) = root.tag.get("Entities")
        {
            out.extend(list.iter().cloned());
        }
        Ok(out)
    }
    /// Write `chunks` into their region files and wait until they are on disk.
    pub fn save(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Result<usize> {
        biomes.ensure_saveable()?;
        let items = self.items_for_save()?;
        if let Some(result) = self.save_disk_sync(chunks, registry, biomes, items) {
            return result;
        }
        let tables = self.tables.get().map(|t| &**t);
        let records = chunks
            .iter()
            .filter_map(|c| {
                let key = (c.x, c.z);
                match compression_pool::catch_encoding_panic(|| {
                    let blending = self.calculated_blending(c.x, c.z);
                    Ok(encode_saved(
                        c,
                        registry,
                        biomes,
                        tables,
                        self.shape,
                        blending.as_deref(),
                        items,
                    ))
                }) {
                    Ok(tag) => Some((key, Some(tag))),
                    Err(error) => {
                        self.report_record_failure(Folder::Region, key, &error);
                        None
                    }
                }
            })
            .collect();
        self.write_records("region", records, true)?;
        self.save_poi(chunks, true)?;
        Ok(chunks.len())
    }
    fn save_poi(&self, chunks: &[Arc<Chunk>], wait: bool) -> Result<()> {
        let Some(tables) = self.tables.get() else {
            return Ok(());
        };
        let professions = self
            .professions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let records = chunks
            .iter()
            .map(|c| {
                (
                    (c.x, c.z),
                    Record::Poi(c.clone(), tables.clone(), professions.clone()),
                )
            })
            .collect();
        self.queue(Folder::Poi, records, wait)
    }
    /// `save` on the writer thread: returns before the files are written;
    /// loads see the chunks meanwhile.
    pub fn save_later(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<usize> {
        let encoding = self.encoding_for_save_later(chunks, registry, biomes)?;
        let records = chunks
            .iter()
            .map(|c| {
                let blending = self.calculated_blending(c.x, c.z);
                (
                    (c.x, c.z),
                    Record::Chunk(c.clone(), encoding.clone(), blending),
                )
            })
            .collect();
        self.queue(Folder::Region, records, false)?;
        self.save_poi(chunks, false)?;
        Ok(chunks.len())
    }
    /// Write the entity chunks (`EntityStorage`): an empty list deletes it.
    pub fn save_entities(&self, chunks: Vec<((i32, i32), Vec<Tag>)>) -> Result<()> {
        self.write_entities(chunks, true)
    }
    /// `save_entities` on the writer thread.
    pub fn save_entities_later(&self, chunks: Vec<((i32, i32), Vec<Tag>)>) -> Result<()> {
        self.write_entities(chunks, false)
    }
    /// `PersistentEntitySectionManager.storeChunkSections`: never store a pending entity chunk.
    fn write_entities(&self, mut chunks: Vec<((i32, i32), Vec<Tag>)>, wait: bool) -> Result<()> {
        {
            let failed = self.failed_entity_reads.lock().unwrap();
            if !failed.is_empty() {
                chunks.retain(|(key, _)| !failed.contains(key));
            }
        }
        let records = chunks
            .into_iter()
            .map(|((x, z), entities)| {
                let tag = (!entities.is_empty()).then(|| NamedTag {
                    name: "".into(),
                    tag: Tag::compound([
                        ("DataVersion", Tag::Int(DATA_VERSION)),
                        ("Position", Tag::IntArray(vec![x, z])),
                        ("Entities", Tag::List(entities)),
                    ]),
                });
                ((x, z), tag)
            })
            .collect();
        self.write_records("entities", records, wait)
    }
}

pub fn read_dat(path: &Path) -> Option<Tag> {
    try_read_dat(path).ok()
}
/// `read_dat`, with why the file did not read.
fn try_read_dat(path: &Path) -> std::result::Result<Tag, DatIssue> {
    // Starling: the newest queued copy first (`later`).
    let bytes = match later::queued(path) {
        Some(bytes) => bytes,
        None => fs::read(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DatIssue::Missing,
            _ => DatIssue::Unreadable(e.to_string()),
        })?,
    };
    let raw = match Compression::Gzip.decode(&bytes, Limits::saved().max_bytes) {
        Ok(raw) => raw,
        Err(_) => bytes,
    };
    crate::recipe_book::read_saved(&raw)
        .map(|(named, _)| named.tag)
        .map_err(|e| DatIssue::Unreadable(e.to_string()))
}
/// Starling: World files written off the render thread while the game saves in the background (the
/// autosave): Android's shared storage takes milliseconds per file call, and a save writes a
/// dozen small files. One thread writes them in order; reads see what is still queued; a
/// synchronous write, and [`flush`], wait for the queue first.
pub mod later {
    use super::{Path, PathBuf, Tag};
    use std::{
        cell::Cell,
        collections::HashMap,
        sync::{Condvar, Mutex, OnceLock, mpsc::Sender},
    };

    pub enum Job {
        Dat(Tag),
        Bytes(Vec<u8>),
    }
    impl Job {
        /// The bytes the file will hold (what a read must see before the write is done).
        fn bytes(&self) -> Option<Vec<u8>> {
            match self {
                Job::Bytes(b) => Some(b.clone()),
                Job::Dat(tag) => {
                    let named = sunrise_core::nbt::NamedTag {
                        name: "".into(),
                        tag: tag.clone(),
                    };
                    sunrise_core::nbt::write_named(&named, sunrise_core::nbt::Limits::default())
                        .ok()
                }
            }
        }
    }
    struct Queue {
        tx: Mutex<Option<Sender<(u64, PathBuf, Job)>>>,
        /// The newest queued bytes of each file, by the job that will write them.
        pending: Mutex<HashMap<PathBuf, (u64, Vec<u8>)>>,
        /// (jobs queued, jobs written).
        counts: Mutex<(u64, u64)>,
        done: Condvar,
    }
    thread_local! {
        static DEFERRED: Cell<bool> = const { Cell::new(false) };
    }
    fn queue_state() -> &'static Queue {
        static QUEUE: OnceLock<Queue> = OnceLock::new();
        QUEUE.get_or_init(|| Queue {
            tx: Mutex::new(None),
            pending: Mutex::default(),
            counts: Mutex::new((0, 0)),
            done: Condvar::new(),
        })
    }
    /// Runs `f` with its world file writes queued.
    pub fn deferred<R>(f: impl FnOnce() -> R) -> R {
        DEFERRED.with(|d| d.set(true));
        let out = f();
        DEFERRED.with(|d| d.set(false));
        out
    }
    pub(super) fn is_deferred() -> bool {
        DEFERRED.with(Cell::get)
    }
    pub(super) fn queue(path: &Path, job: Job) {
        let q = queue_state();
        let mut counts = q.counts.lock().unwrap_or_else(|e| e.into_inner());
        let seq = counts.0 + 1;
        if let Some(bytes) = job.bytes()
            && let Ok(mut pending) = q.pending.lock()
        {
            pending.insert(path.to_owned(), (seq, bytes));
        }
        let mut tx = q.tx.lock().unwrap_or_else(|e| e.into_inner());
        if tx.is_none() {
            let (send, receive) = std::sync::mpsc::channel::<(u64, PathBuf, Job)>();
            let started = std::thread::Builder::new()
                .name("sunrise-world-files".into())
                .spawn(move || {
                    for (seq, path, job) in receive {
                        let written = match job {
                            Job::Dat(tag) => super::write_dat_now(&path, tag),
                            Job::Bytes(bytes) => super::write_file_now(&path, &bytes),
                        };
                        if let Err(e) = written {
                            log::warn!("Sunrise: could not save {}: {e}", path.display());
                        }
                        let q = queue_state();
                        if let Ok(mut pending) = q.pending.lock()
                            && pending.get(&path).is_some_and(|(s, _)| *s == seq)
                        {
                            pending.remove(&path);
                        }
                        let mut counts = q.counts.lock().unwrap_or_else(|e| e.into_inner());
                        counts.1 += 1;
                        q.done.notify_all();
                    }
                });
            if started.is_err() {
                drop(tx);
                drop(counts);
                // No writer thread: write it now.
                let _ = match job {
                    Job::Dat(tag) => super::write_dat_now(path, tag),
                    Job::Bytes(bytes) => super::write_file_now(path, &bytes),
                };
                if let Ok(mut pending) = q.pending.lock() {
                    pending.remove(path);
                }
                return;
            }
            *tx = Some(send);
        }
        counts.0 = seq;
        if let Some(send) = tx.as_ref() {
            let _ = send.send((seq, path.to_owned(), job));
        }
    }
    /// The newest queued bytes of `path`, when its write is not done yet.
    pub(super) fn queued(path: &Path) -> Option<Vec<u8>> {
        let q = queue_state();
        q.pending
            .lock()
            .ok()?
            .get(path)
            .map(|(_, bytes)| bytes.clone())
    }
    /// Waits until every queued file is written.
    pub fn flush() {
        let q = queue_state();
        let mut counts = q.counts.lock().unwrap_or_else(|e| e.into_inner());
        while counts.1 < counts.0 {
            counts = q.done.wait(counts).unwrap_or_else(|e| e.into_inner());
        }
    }
}
/// What `LevelStorageAccess.collectIssues` finds wrong with a level.dat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatIssue {
    /// No such file (`FileNotFoundException`).
    Missing,
    /// It exists but does not read as a level: why.
    Unreadable(String),
}
/// `LevelStorageAccess.collectIssues`: why level.dat (level.dat_old with
/// `old`) can't be read as the world's level data, `None` when it can.
pub fn level_dat_issue(dir: &Path, old: bool) -> Option<DatIssue> {
    let file = if old { "level.dat_old" } else { "level.dat" };
    match try_read_dat(&dir.join(file)) {
        Ok(root) if matches!(root.get("Data"), Some(Tag::Compound(_))) => None,
        Ok(_) => Some(DatIssue::Unreadable("Invalid root tag".into())),
        Err(issue) => Some(issue),
    }
}
/// `WorldOpenFlows.openWorldLoadLevelData`: what keeps `world`'s level data from reading, which
/// opens RecoverWorldDataScreen instead of the world. A folder with neither level.dat nor
/// level.dat_old is a world not made yet, not a broken one.
pub fn level_data_needs_recovery(world: &Path) -> Option<DatIssue> {
    match level_dat_issue(world, false)? {
        DatIssue::Missing if !world.join("level.dat_old").exists() => None,
        issue => Some(issue),
    }
}
/// `LevelStorageAccess.restoreLevelDataFromOld` (`Util.safeReplaceOrMoveFile`):
/// level.dat moves aside to `level.dat_corrupted_<time>` and level.dat_old
/// takes its place. Whether it worked. Vanilla passes `noRollback` and reports
/// success even when level.dat_old could not be moved in; here the broken
/// level.dat goes back and the failure is reported.
pub fn restore_level_data_from_old(dir: &Path) -> bool {
    let (current, latest) = (dir.join("level.dat"), dir.join("level.dat_old"));
    let backup = dir.join(format!("level.dat_corrupted_{}", backup_stamp()));
    if current.exists() && fs::rename(&current, &backup).is_err() {
        return false;
    }
    if fs::rename(&latest, &current).is_err() {
        if backup.exists() {
            let _ = fs::rename(&backup, &current);
        }
        return false;
    }
    true
}
/// `FileNameDateFormatter`: the time in the names of backed-up unreadable files.
fn backup_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}
/// `LevelStorageAccess.getUnfixedDataTagWithFallback`, which the dedicated server's `Main` calls:
/// a level.dat that does not read falls back to level.dat_old, which is restored in its place
/// (`restoreLevelDataFromOld`). Whether it did; `false` leaves the world as it was, for the
/// caller to refuse (both files broken, or nothing to fall back to).
pub fn fall_back_to_level_dat_old(world: &Path) -> bool {
    level_dat_issue(world, false).is_some()
        && world.join("level.dat_old").exists()
        && level_dat_issue(world, true).is_none()
        && restore_level_data_from_old(world)
}
/// `LevelStorageSource.readLevelDataTagFixed`: level.dat, else level.dat_old.
fn read_level_dat(dir: &Path) -> Option<Tag> {
    read_dat(&dir.join("level.dat")).or_else(|| read_dat(&dir.join("level.dat_old")))
}
/// `restoreLevelDataFromOld`: an unreadable level.dat is kept as
/// `level.dat_corrupted_<time>` and level.dat_old takes its place, so the next
/// write does not bury either file.
fn restore_level_dat(dir: &Path) {
    let path = dir.join("level.dat");
    if read_dat(&path).is_some() {
        return;
    }
    if path.exists() {
        let _ = fs::rename(
            &path,
            dir.join(format!("level.dat_corrupted_{}", backup_stamp())),
        );
    }
    let old = dir.join("level.dat_old");
    if read_dat(&old).is_some() {
        let _ = fs::rename(old, path);
    }
}
/// `PlayerDataStorage.backup`: a player file that fails to load is copied to
/// `<uuid>_corrupted_<time>.dat` before it is written over.
fn back_up_unreadable_player(path: &Path) {
    if path.is_file()
        && read_dat(path).is_none()
        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
    {
        let _ = fs::copy(
            path,
            path.with_file_name(format!("{stem}_corrupted_{}.dat", backup_stamp())),
        );
    }
}
/// Write a gzip NBT file the way vanilla does (`<name>_new`, keep `_old`).
pub fn write_dat(path: &Path, tag: Tag) -> Result<()> {
    // Starling: inside `later::deferred` it goes to the background writer.
    if later::is_deferred() {
        later::queue(path, later::Job::Dat(tag));
        return Ok(());
    }
    // A queued older copy must not land after this one.
    later::flush();
    write_dat_now(path, tag)
}
/// Starling: a file's new bytes (`fs::write` through `_new` and a rename); queued like `write_dat`.
pub fn write_file(path: &Path, bytes: Vec<u8>) -> Result<()> {
    if later::is_deferred() {
        later::queue(path, later::Job::Bytes(bytes));
        return Ok(());
    }
    later::flush();
    write_file_now(path, &bytes)
}
fn write_file_now(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let temporary = path.with_file_name(format!("{file}_new"));
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}
/// Starling: a file's bytes, the newest queued ones first.
pub fn read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    match later::queued(path) {
        Some(bytes) => Ok(bytes),
        None => fs::read(path),
    }
}
fn write_dat_now(path: &Path, tag: Tag) -> Result<()> {
    write_dat_as(path, tag, false)
}

/// [`write_dat`], synced to the disk: the new file before it is renamed into place, and the
/// folder after, so a power loss leaves the old file or the new one. A hand-over's commit
/// point (`player-confirm`) replies only once its files are.
pub fn write_dat_durable(path: &Path, tag: Tag) -> Result<()> {
    later::flush();
    write_dat_as(path, tag, true)
}

/// `bytes` written to `path` through `<name>_new` and a rename, synced to the disk as
/// [`write_dat_durable`] syncs: never a half-written file in place.
pub fn write_durable(path: &Path, bytes: &[u8]) -> Result<()> {
    later::flush();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let temporary = path.with_file_name(format!("{file}_new"));
    write_synced(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    sync_folder(path)?;
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The folder of `path` synced, so that a rename into it is on the disk (POSIX; Windows has
/// no folder handle to sync, and its rename is journaled by NTFS).
fn sync_folder(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_dat_as(path: &Path, tag: Tag, durable: bool) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let bytes = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag,
        },
        Limits::saved(),
    )
    .map_err(|e| e.to_string())?;
    let compressed = Compression::Gzip.encode(&bytes)?;
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("data.dat");
    let temporary = path.with_file_name(format!("{file}_new"));
    if durable {
        write_synced(&temporary, &compressed)?;
    } else {
        fs::write(&temporary, compressed)?;
    }
    if path.exists() {
        let old = match file.strip_suffix(".dat") {
            Some(stem) => path.with_file_name(format!("{stem}.dat_old")),
            None => path.with_file_name(format!("{file}_old")),
        };
        // Util.safeReplaceFile: no backup, no replacement.
        fs::rename(path, old).map_err(|e| format!("{}: {e}", path.display()))?;
        use sunrise_world::crash_fixture::{CrashFixture, await_save_kill, selected};
        // Between the two renames, a reader needs the backup.
        match selected() {
            Some(CrashFixture::SaveLevel) if file == "level.dat" => await_save_kill("level"),
            Some(CrashFixture::SavePlayer)
                if path
                    .parent()
                    .is_some_and(|parent| parent.ends_with("players/data")) =>
            {
                await_save_kill("player");
            }
            _ => {}
        }
    }
    fs::rename(temporary, path)?;
    if durable {
        sync_folder(path)?;
    }
    Ok(())
}
mod saved_data;
use saved_data::{read_saved_data, saved_data_path, write_saved_data};
/// The generator settings used for this world session. A replacement from
/// enabled packs is kept in memory until the first save.
#[derive(Clone, Debug)]
pub struct WorldGenerator {
    data: Tag,
    recovered: bool,
}

impl WorldGenerator {
    pub fn settings(&self) -> &Tag {
        &self.data
    }

    fn read(world: &Path, level_data: Option<&Tag>) -> (Option<Self>, i64) {
        use sunrise_world::datafix::{files, schema::Ty, update_saved_data};
        // Before file fixes (world previews), project the old settings into the
        // current layout. During startup the migration has already run.
        let old = level_data
            .and_then(|data| number(data.get("DataVersion")))
            .and_then(|version| i32::try_from(version).ok())
            .is_some_and(files::requires_file_fixing);
        let candidates = [
            old.then(|| files::current_world_gen_settings(world).ok().flatten())
                .flatten(),
            read_dat(&saved_data_path(world, "world_gen_settings")).map(|mut root| {
                update_saved_data(Ty::SavedWorldGenSettings, &mut root);
                root.get("data")
                    .cloned()
                    .unwrap_or(Tag::Compound(Default::default()))
            }),
            level_data.and_then(|data| data.get("WorldGenSettings").cloned()),
        ];
        let usable = |s: &Tag| {
            number(s.get("seed")).is_some()
                && s.get("dimensions").is_some_and(valid_world_gen_dimensions)
        };
        // Old summaries may still display a seed from level.dat even if its
        // settings are unusable. A dedicated world only uses a validated generator.
        let displayed_seed = candidates
            .iter()
            .flatten()
            .find_map(|s| number(s.get("seed")))
            .unwrap_or(0);
        let generator = candidates
            .into_iter()
            .flatten()
            .find(usable)
            .map(|data| Self {
                data,
                recovered: false,
            });
        let seed = generator
            .as_ref()
            .and_then(|g| number(g.data.get("seed")))
            .unwrap_or(displayed_seed);
        (generator, seed)
    }

    /// Read the prepared settings for a newly created world, which does not
    /// have level.dat yet.
    pub fn read_new(world: &Path) -> Option<Self> {
        Self::read(world, None).0
    }
}

/// The world's usable generator settings, for previews outside the world session.
pub fn world_gen_settings(world: &Path) -> Option<Tag> {
    let root = read_level_dat(world);
    WorldGenerator::read(world, root.as_ref().and_then(|r| r.get("Data")))
        .0
        .map(|generator| generator.data)
}
include!("save/preset_saved.rs");
pub use sunrise_client_core::uuid::uuid_from_text;
pub use sunrise_core::uuid::uuid_text;
/// A random (version 4) UUID as vanilla's four-int form.
///
/// Every call gets its own: the generator is seeded once from the clock and
/// the process, then each call takes the next step of one shared counter. A
/// seed from the clock alone repeats within its resolution (a microsecond on
/// macOS): a pack of mobs generated together all had the same UUID, and a
/// vanilla client keeps only one entity per UUID (the rest never moved).
pub fn random_uuid() -> [i32; 4] {
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        AtomicU64::new(t ^ (std::process::id() as u64).rotate_left(32))
    });
    // SplitMix64: distinct steps give distinct outputs.
    let split = || {
        let mut z = next
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            .wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let (a, b) = (split(), split());
    let a = (a & !0xF000) | 0x4000;
    let b = (b & !(0xC000_0000_0000_0000)) | 0x8000_0000_0000_0000;
    [(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]
}
/// `Mth.createInsecureUUID`: a new entity's UUID is its own random's first two
/// draws (`Entity.uuid`).
pub fn insecure_uuid(random: &mut impl sunrise_core::random::RandomSource) -> [i32; 4] {
    let most = random.next_long() & -61441 | 16384;
    let least = random.next_long() & 4_611_686_018_427_387_903 | i64::MIN;
    [
        (most >> 32) as i32,
        most as i32,
        (least >> 32) as i32,
        least as i32,
    ]
}
pub(crate) fn uuid_tag(u: [i32; 4]) -> Tag {
    Tag::IntArray(u.to_vec())
}
pub(crate) fn uuid_from_tag(tag: Option<&Tag>) -> Option<[i32; 4]> {
    match tag? {
        Tag::IntArray(v) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
        Tag::String(s) => uuid_from_text(&s.to_string_lossy()),
        _ => None,
    }
}
// `DoubleTag.valueOf` / `FloatTag.valueOf` store -0.0 as the shared +0.0
// `ZERO`, so a resting entity's `Motion` reads `0.0d`, never `-0.0d`.
pub(crate) fn doubles(values: &[f64]) -> Tag {
    Tag::List(
        values
            .iter()
            .map(|&v| Tag::Double(if v == 0.0 { 0 } else { v.to_bits() }))
            .collect(),
    )
}
pub(crate) fn floats(values: &[f32]) -> Tag {
    Tag::List(
        values
            .iter()
            .map(|&v| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() }))
            .collect(),
    )
}
pub(crate) fn read_doubles<const N: usize>(tag: Option<&Tag>) -> Option<[f64; N]> {
    match tag? {
        Tag::List(l) if l.len() == N => {
            let mut out = [0.0; N];
            for (o, v) in out.iter_mut().zip(l) {
                *o = float(Some(v))?;
            }
            Some(out)
        }
        _ => None,
    }
}

/// The world presets of 26.3's `worldgen/world_preset` that the create
/// screen offers, in the order of `WorldPresetTags.NORMAL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldPreset {
    Normal,
    Flat,
    LargeBiomes,
    Amplified,
    SingleBiomeSurface,
    /// `minecraft:debug_all_block_states`: a grid of every block state
    /// (`DebugLevelSource`). The create screen offers it only with Alt held;
    /// a data pack, `level-type` or a saved world names it too.
    Debug,
}
impl WorldPreset {
    /// The presets, in the order the world-type button cycles through them.
    pub const ALL: [Self; 5] = [
        Self::Normal,
        Self::Flat,
        Self::LargeBiomes,
        Self::Amplified,
        Self::SingleBiomeSurface,
    ];
    /// The preset's registry id (`minecraft:normal`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Normal => "minecraft:normal",
            Self::Flat => "minecraft:flat",
            Self::LargeBiomes => "minecraft:large_biomes",
            Self::Amplified => "minecraft:amplified",
            Self::SingleBiomeSurface => "minecraft:single_biome_surface",
            Self::Debug => "minecraft:debug_all_block_states",
        }
    }
    /// The preset an id names, if it is one of them.
    pub fn of_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Debug])
            .find(|preset| preset.id() == id)
    }
    /// The language key the world-type button shows
    /// (`Identifier.toLanguageKey("generator")`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "generator.minecraft.normal",
            Self::Flat => "generator.minecraft.flat",
            Self::LargeBiomes => "generator.minecraft.large_biomes",
            Self::Amplified => "generator.minecraft.amplified",
            Self::SingleBiomeSurface => "generator.minecraft.single_biome_surface",
            Self::Debug => "generator.minecraft.debug_all_block_states",
        }
    }
    /// The preset the world-type button cycles to next.
    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
    /// The preset the world-type button cycles to previously.
    pub fn prev(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
    /// The preset a saved overworld generator is: the noise settings name
    /// the preset, and the single-biome preset shares the overworld's noise
    /// settings, so its fixed biome source tells the two apart.
    pub fn of_generator(generator: &Tag) -> Self {
        if text(generator.get("type")).as_deref() == Some("minecraft:flat") {
            return Self::Flat;
        }
        if text(generator.get("type")).as_deref() == Some("minecraft:debug") {
            return Self::Debug;
        }
        let source = generator.get("biome_source");
        let fixed = text(source.and_then(|s| s.get("type"))).as_deref() == Some("minecraft:fixed");
        match text(generator.get("settings")).as_deref() {
            Some("minecraft:large_biomes") => Self::LargeBiomes,
            Some("minecraft:amplified") => Self::Amplified,
            Some("minecraft:overworld") if fixed => Self::SingleBiomeSurface,
            _ => Self::Normal,
        }
    }
}
/// World and player state stored in `level.dat`, the saved-data files and
/// the player file.
/// The world options CreateWorldScreen sets (`WorldOptions` and the world
/// preset) that `world_gen_settings` keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldGen {
    /// The world preset the world is created with.
    pub preset: WorldPreset,
    pub structures: bool,
    pub bonus_chest: bool,
}
impl WorldGen {
    /// A saved world's choices, from its `world_gen_settings`.
    pub fn read(world: &Path) -> Option<Self> {
        world_gen_settings(world).as_ref().map(Self::from_settings)
    }

    /// Options of the generator selected by the world loader.
    pub fn from_settings(settings: &Tag) -> Self {
        let flag = |key: &str, default: bool| number(settings.get(key)).map_or(default, |v| v != 0);
        let generator = settings
            .get("dimensions")
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"));
        Self {
            preset: generator.map_or(WorldPreset::Normal, WorldPreset::of_generator),
            structures: flag("generate_structures", true),
            bonus_chest: flag("bonus_chest", false),
        }
    }
    /// The bundled world preset a saved world's overworld was generated
    /// with, from its generator: `minecraft:amplified` and
    /// `minecraft:large_biomes` by their noise settings, and
    /// `minecraft:single_biome_surface` by a fixed biome source. Anything
    /// else (and any world without settings) is `minecraft:normal`.
    pub fn noise_preset(settings: Option<&Tag>) -> &'static str {
        let generator = settings
            .and_then(|s| s.get("dimensions"))
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"));
        let noise = text(generator.and_then(|g| g.get("settings")));
        let fixed_biome = generator
            .and_then(|g| g.get("biome_source"))
            .and_then(|b| text(b.get("type")))
            .is_some_and(|t| t == "minecraft:fixed");
        match noise.as_deref() {
            Some("minecraft:amplified") => "minecraft:amplified",
            Some("minecraft:large_biomes") => "minecraft:large_biomes",
            Some("minecraft:overworld") if fixed_biome => "minecraft:single_biome_surface",
            _ => "minecraft:normal",
        }
    }
}
impl Default for WorldGen {
    fn default() -> Self {
        Self {
            preset: WorldPreset::Normal,
            structures: true,
            bonus_chest: false,
        }
    }
}
/// Writes a new world's `world_gen_settings` (seed, options and dimensions).
pub fn write_world_gen_settings(dir: &Path, seed: i64, options: WorldGen) -> Result<()> {
    write_world_gen_settings_with(dir, seed, options, None, None, None)
}
include!("save/preset_generation.rs");
const VANILLA_FEATURE_FLAGS: [&str; 1] = ["minecraft:vanilla"];
/// The effective feature flags stored by a level. Fresh and defaulted saves have vanilla's set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureFlagIds(Arc<[String]>);

impl Default for FeatureFlagIds {
    fn default() -> Self {
        static VANILLA: std::sync::LazyLock<Arc<[String]>> =
            std::sync::LazyLock::new(|| VANILLA_FEATURE_FLAGS.map(str::to_owned).into());
        Self(Arc::clone(&VANILLA))
    }
}

impl FeatureFlagIds {
    /// Share the flag ids with views of this level without copying them.
    pub fn as_arc(&self) -> &Arc<[String]> {
        &self.0
    }
}

impl std::ops::Deref for FeatureFlagIds {
    type Target = [String];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Into<Arc<[String]>>> From<T> for FeatureFlagIds {
    fn from(ids: T) -> Self {
        Self(ids.into())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Level {
    pub name: String,
    pub seed: i64,
    /// The generator selected for the session, including any replacement settings.
    #[doc(hidden)]
    pub generator: Option<WorldGenerator>,
    pub creative: bool,
    /// The game mode id (0 survival, 1 creative, 2 adventure, 3 spectator);
    /// `creative` wins when the two disagree.
    pub game_mode: i32,
    /// `previousPlayerGameType`: the mode the player left last (the F3+F4 switcher's
    /// return mode), when it has one.
    pub previous_game_mode: Option<i32>,
    pub time: i64,
    pub day_time: i64,
    /// The End's clock (`world_clocks.dat`'s `minecraft:the_end`); `None` in a world that has
    /// none yet, which starts it at the game time.
    pub end_clock: Option<i64>,
    /// The world spawn or a legacy bare Overworld spawn column. Personal
    /// points use `respawn_elsewhere` so their blocks are checked on death.
    pub spawn: Option<[f64; 3]>,
    /// A respawn point resolved through its block when the player respawns
    /// (a bed, a respawn anchor, a forced point in any dimension):
    /// dimension, block, yaw, pitch and `forced`.
    pub respawn_elsewhere: Option<(String, [i32; 3], f32, f32, bool)>,
    pub position: Option<[f64; 3]>,
    pub rotation: [f32; 2],
    pub health: f32,
    /// LivingEntity `AbsorptionAmount`: the golden hearts left.
    pub absorption: f32,
    pub food: i32,
    pub saturation: f32,
    /// (slot, item, count, damage): 0..36 inventory, 100..103 armor (feet..head), 150 offhand,
    /// 151 body and 152 saddle (`BODY_SLOT`).
    pub inventory: Vec<(u8, String, u32, u32)>,
    /// Extra item components of `inventory` entries, by slot.
    pub inventory_components: HashMap<u8, Box<crate::item_data::StackData>>,
    /// Player.XpSeed: the enchanting table seed.
    pub xp_seed: i32,
    /// ServerPlayer.seenCredits: the end poem has been shown.
    pub seen_credits: bool,
    /// ServerPlayer.postEffects: the `post_effect` chains `/posteffect` set.
    pub post_effects: Vec<String>,
    /// Player.lastDeathLocation: the dimension and block of the last death.
    pub last_death: Option<(String, [i32; 3])>,
    /// Player.active_effects: (id, duration, amplifier, ambient,
    /// show_particles, show_icon).
    pub active_effects: Vec<crate::effects::SavedEffect>,
    /// Player.EnderItems: the ender chest inventory (27 slots).
    pub ender_items: Vec<Option<crate::items::ItemStack>>,
    /// The `recipeBook` compound: open/filter settings and unlocked recipes.
    pub recipe_book: crate::recipe_book::Settings,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub xp_total: i32,
    pub weather: crate::world::Weather,
    /// Player.Dimension (minecraft:overworld / minecraft:the_nether).
    pub dimension: String,
    /// World spawn (`level.dat` `spawn`), distinct from a bed respawn point.
    pub world_spawn: Option<[i32; 3]>,
    /// The world spawn's yaw and pitch (`setworldspawn`'s rotation).
    pub world_spawn_angle: [f32; 2],
    /// The level the world spawn is in (`spawn.dimension`, `RespawnData.dimension`); None for
    /// the Overworld.
    pub world_spawn_dimension: Option<String>,
    pub difficulty: String,
    pub hardcore: bool,
    /// `difficulty_settings.locked` (LevelData.isDifficultyLocked).
    pub difficulty_locked: bool,
    /// `allowCommands`: read from `level.dat` or chosen on the Create New
    /// World screen; unset is written as off.
    pub allow_commands: Option<bool>,
    /// `DataPackConfig` (Create World's Data Packs and Experiments): the
    /// enabled and disabled pack ids. Unset keeps vanilla's defaults.
    pub data_packs: Option<(Vec<String>, Vec<String>)>,
    /// `WorldDataConfiguration`'s `enabled_features`: the effective feature flag ids.
    /// An omitted `level.dat` tag decodes as vanilla's default, `minecraft:vanilla`.
    pub enabled_features: FeatureFlagIds,
    /// The code mods (`fml.LoadingModList` beside `Data`), then the content mods (`sunrise.ContentModList`), the world was saved with; `None` without either list or when both are empty.
    pub saved_mods: Option<Vec<level_mods::ModRecord>>,
    /// Game rule values by name without namespace (`keep_inventory`, ...).
    pub game_rules: BTreeMap<String, i32>,
    /// Definitions used by this world session.
    pub rule_registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    pub player_uuid: Option<[i32; 4]>,
    pub selected_slot: i32,
    pub motion: [f64; 3],
    pub fall_distance: f64,
    pub on_ground: bool,
    /// `LivingEntity.addAdditionalSaveData`'s `FallFlying`: the player was
    /// gliding (shared flag 7), so the glide goes on after the load.
    pub fall_flying: bool,
    /// Current body's save projection. None before a live body supplies it;
    /// retained raw data stays intact until then.
    pub player_impulse_grace: Option<i32>,
    pub player_fall_particles: Option<bool>,
    pub food_exhaustion: f32,
    pub food_tick_timer: i32,
    /// Abilities.flying: whether creative/spectator flight is active.
    pub flying: bool,
    /// Abilities.flyingSpeed, when the file has one (vanilla's is 0.05).
    pub fly_speed: Option<f32>,
    /// The `abilities` compound of the player file (`Abilities.Packed`), as
    /// read and as a server player's stored abilities write it. When none,
    /// `player_tag` derives it from the game mode.
    pub abilities: Option<crate::lan::abilities::PlayerAbilities>,
    /// The vehicle the player rides, by UUID (`ServerPlayer.saveParentVehicle`'s
    /// "RootVehicle"), so the ride comes back on the next login.
    pub root_vehicle: Option<[i32; 4]>,
    /// The player compound as loaded, for fields Sunrise does not model
    /// (ender chest, effects, abilities, recipe book, attributes, ...).
    pub player_extra: Option<Tag>,
    /// The player was read from the file level.dat names (`singleplayer_uuid`,
    /// or its `.dat_old`): `PlayerList.loadPlayerData` gives the world's owner
    /// that file, not their own.
    pub singleplayer_file: bool,
}

impl Level {
    /// `PrimaryLevelData.setDifficulty`: the level data holds the world's
    /// difficulty, so the game and the file follow each other and a world
    /// that was set to hard is hard when it is opened again.
    pub fn set_difficulty(&mut self, difficulty: u8) {
        self.difficulty = crate::title::DIFFICULTIES[difficulty as usize % 4].to_owned();
    }

    /// `PrimaryLevelData.getDifficulty`: the id of the level data's
    /// difficulty (peaceful 0 .. hard 3), normal when it is not one of them.
    pub fn difficulty_id(&self) -> u8 {
        crate::title::DIFFICULTIES
            .iter()
            .position(|d| *d == self.difficulty)
            .map_or(2, |i| i as u8)
    }
    /// Initialize the offline single-player identity before saving or dropping items.
    pub fn ensure_player_uuid(&mut self) -> [i32; 4] {
        *self.player_uuid.get_or_insert_with(|| {
            let u = sunrise_protocol::crypto::offline_uuid("Player");
            [
                (u >> 96) as u32 as i32,
                (u >> 64) as u32 as i32,
                (u >> 32) as u32 as i32,
                u as u32 as i32,
            ]
        })
    }
    pub fn game_rule(&self, name: &str) -> i32 {
        self.game_rules
            .get(name)
            .copied()
            .or_else(|| {
                self.rule_registry
                    .rule(name)
                    .and_then(|rule| self.game_rules.get(&rule.name).copied())
            })
            .or_else(|| {
                self.rule_registry
                    .rule(name)
                    .map(|rule| match rule.default {
                        crate::gamerules::Value::Bool(value) => i32::from(value),
                        crate::gamerules::Value::Int(value) => value,
                    })
            })
            .unwrap_or(0)
    }
    /// The saved game mode id.
    pub fn mode(&self) -> i32 {
        match (self.creative, self.game_mode) {
            (true, _) => 1,
            (false, 1) => 0,
            (false, m) => m.clamp(0, 3),
        }
    }
    pub fn read(dir: &Path) -> Option<Self> {
        Self::read_with_registry(dir, Default::default())
    }
    /// Load a world using its session rule definitions.
    pub fn read_with_registry(
        dir: &Path,
        registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    ) -> Option<Self> {
        Self::read_root(dir, read_level_dat(dir)?, registry)
    }
    /// Select a generator for an existing dedicated world with unusable settings.
    pub fn recover_world_generator(
        &mut self,
        overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
        random_seed: impl FnOnce() -> i64,
    ) -> Result<()> {
        use sunrise_world::worldgen::overrides::DIMENSIONS;
        if self.generator.is_some() {
            return Ok(());
        }
        log::warn!(
            "Sunrise: world generator settings do not read; using a random world seed and datapack dimensions"
        );
        self.seed = random_seed();
        let mut data = new_world_gen_settings(self.seed, WorldGen::default(), None, None);
        let dimensions = data.get_mut("dimensions").expect("default dimension map");
        let Tag::Compound(stems) = dimensions else {
            unreachable!()
        };
        for (id, stem) in overrides.entries(DIMENSIONS) {
            stems.insert(
                id.into(),
                crate::scoreboard::json_to_tag(&serde_json::from_str(stem)?),
            );
        }
        if !valid_world_gen_dimensions(dimensions) {
            return Err("invalid datapack dimensions for generator recovery".into());
        }
        self.generator = Some(WorldGenerator {
            data,
            recovered: true,
        });
        Ok(())
    }
    /// `read` from level.dat alone, without the level.dat_old fallback: what
    /// `LevelStorageSource.readLevelSummary` reads for the world list.
    pub fn read_summary(dir: &Path) -> Option<Self> {
        Self::read_root(dir, read_dat(&dir.join("level.dat"))?, Default::default())
    }
    fn read_root(
        dir: &Path,
        root: Tag,
        registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    ) -> Option<Self> {
        let data = root.get("Data")?;
        let n = |t: Option<&Tag>| number(t).unwrap_or(0);
        let mut level = Self {
            rule_registry: registry,
            name: text(data.get("LevelName")).unwrap_or_else(|| "World".into()),
            creative: n(data.get("GameType")) == 1,
            game_mode: n(data.get("GameType")) as i32,
            time: n(data.get("Time")),
            health: 20.0,
            food: 20,
            saturation: 5.0,
            dimension: "minecraft:overworld".into(),
            difficulty: "normal".into(),
            ..Self::default()
        };
        // 26.3: spawn {pos: [I; x, y, z], dimension, yaw, pitch}; older: SpawnX/Y/Z.
        level.world_spawn = match data.get("spawn").and_then(|s| s.get("pos")) {
            Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
            _ => data.get("SpawnX").map(|_| {
                [
                    n(data.get("SpawnX")) as i32,
                    n(data.get("SpawnY")) as i32,
                    n(data.get("SpawnZ")) as i32,
                ]
            }),
        };
        let angle = |k: &str| match data.get("spawn").and_then(|s| s.get(k)) {
            Some(Tag::Float(b)) => f32::from_bits(*b),
            _ => 0.0,
        };
        level.world_spawn_angle = [angle("yaw"), angle("pitch")];
        level.world_spawn_dimension = text(data.get("spawn").and_then(|s| s.get("dimension")))
            .filter(|d| d != "minecraft:overworld");
        if let Some(d) = data.get("difficulty_settings") {
            level.difficulty = text(d.get("difficulty")).unwrap_or_else(|| "normal".into());
            level.hardcore = n(d.get("hardcore")) != 0;
            level.difficulty_locked = n(d.get("locked")) != 0;
        }
        level.allow_commands = Some(n(data.get("allowCommands")) != 0);
        // WorldDataConfiguration.MAP_CODEC's lenient optional field uses the default
        // for both absent and malformed input. A valid empty list is not the default.
        if let Some(Tag::List(flags)) = data.get("enabled_features")
            && let Some(flags) = flags
                .iter()
                .map(|flag| text(Some(flag)))
                .collect::<Option<Arc<[_]>>>()
        {
            level.enabled_features = flags.into();
        }
        level.saved_mods = level_mods::saved(&root);
        if let Some(packs) = data.get("DataPacks") {
            let ids = |key| match packs.get(key) {
                Some(Tag::List(ids)) => ids.iter().filter_map(|id| text(Some(id))).collect(),
                _ => Vec::new(),
            };
            level.data_packs = Some((ids("Enabled"), ids("Disabled")));
        }
        let registry = level.rule_registry.clone();
        let mut rules: BTreeMap<String, i32> = registry
            .available_rules(false)
            .map(|rule| {
                (
                    rule.name.to_owned(),
                    match rule.default {
                        crate::gamerules::Value::Bool(value) => i32::from(value),
                        crate::gamerules::Value::Int(value) => value,
                    },
                )
            })
            .collect();
        let saved_rules = read_saved_data(dir, "game_rules");
        if let Some(Tag::Compound(map)) = &saved_rules {
            for (k, v) in map {
                let name = k.to_string_lossy();
                let name = if name.contains(':') {
                    name
                } else {
                    format!("minecraft:{name}")
                };
                if let Some(v) = number(Some(v)) {
                    rules.insert(name, v as i32);
                }
            }
        }
        // GameRules(enabledFeatures, map) removes disabled experimental rules
        // and supplies their defaults when enabled but absent from saved data.
        let registry = level.rule_registry.clone();
        let minecart_rule = registry.rule("max_minecart_speed").expect("vanilla rule");
        if level
            .enabled_features
            .iter()
            .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS)
        {
            let crate::gamerules::Value::Int(default) = minecart_rule.default else {
                unreachable!()
            };
            let value = rules
                .entry(minecart_rule.name.to_owned())
                .or_insert(default);
            if !(minecart_rule.min..=minecart_rule.max).contains(value) {
                *value = default;
            }
        } else {
            rules.remove(&minecart_rule.name);
        }
        level.game_rules = rules;
        (level.generator, level.seed) = WorldGenerator::read(dir, Some(data));
        // Day time: the overworld clock (26.3) or Data.DayTime.
        level.day_time = read_saved_data(dir, "world_clocks")
            .and_then(|c| {
                number(
                    c.get("minecraft:overworld")
                        .and_then(|o| o.get("total_ticks")),
                )
            })
            .or_else(|| number(data.get("DayTime")))
            .unwrap_or(level.time);
        level.end_clock = read_saved_data(dir, "world_clocks").and_then(|c| {
            number(
                c.get("minecraft:the_end")
                    .and_then(|o| o.get("total_ticks")),
            )
        });
        let weather = read_saved_data(dir, "weather");
        let w = |new: &str, old: &str| {
            number(weather.as_ref().and_then(|w| w.get(new)))
                .or_else(|| number(data.get(old)))
                .unwrap_or(0)
        };
        let raining = w("raining", "raining") != 0;
        let thundering = w("thundering", "thundering") != 0;
        level.weather = crate::world::Weather {
            clear_time: w("clear_weather_time", "clearWeatherTime") as i32,
            rain_time: w("rain_time", "rainTime") as i32,
            thunder_time: w("thunder_time", "thunderTime") as i32,
            raining,
            thundering,
            rain_level: if raining { (1.0, 1.0) } else { (0.0, 0.0) },
            thunder_level: if thundering { (1.0, 1.0) } else { (0.0, 0.0) },
        };
        // The player: players/data/<singleplayer_uuid>.dat, or the legacy Data.Player. With no
        // `singleplayer_uuid` (a world from a server, a Realms download) the owner loads their
        // own file, not another player's (`PlayerList.loadPlayerData`): `load_owner` does.
        let uuid = uuid_from_tag(data.get("singleplayer_uuid"));
        let player_dir = dir.join("players").join("data");
        // PlayerDataStorage.load: the player's file, else its .dat_old.
        let owner = uuid.and_then(|u| {
            let name = uuid_text(u);
            read_dat(&player_dir.join(format!("{name}.dat")))
                .or_else(|| read_dat(&player_dir.join(format!("{name}.dat_old"))))
        });
        level.singleplayer_file = owner.is_some();
        let mut player = owner.or_else(|| data.get("Player").cloned());
        if let Some(player) = &mut player {
            horizon_names::adopt_in_player(player);
        }
        level.player_uuid = uuid_from_tag(player.as_ref().and_then(|p| p.get("UUID"))).or(uuid);
        let spawn = level
            .world_spawn
            .map(|s| [f64::from(s[0]), f64::from(s[1]), f64::from(s[2])]);
        level.spawn = spawn;
        if let Some(p) = &player {
            level.read_player(p);
        }
        level.player_extra = player;
        Some(level)
    }
    pub fn read_player(&mut self, p: &Tag) {
        self.player_impulse_grace = None;
        self.player_fall_particles = None;
        let n = |k: &str| number(p.get(k));
        let f = |k: &str| float(p.get(k));
        self.position = read_doubles::<3>(p.get("Pos"));
        if let Some(r) = read_doubles::<2>(p.get("Rotation")) {
            self.rotation = [r[0] as f32, r[1] as f32];
        }
        self.motion = read_doubles::<3>(p.get("Motion")).unwrap_or_default();
        self.health = f("Health").map_or(20.0, |v| v as f32);
        self.absorption = f("AbsorptionAmount").map_or(0.0, |v| v as f32);
        self.food = n("foodLevel").map_or(20, |v| v as i32);
        self.saturation = f("foodSaturationLevel").map_or(5.0, |v| v as f32);
        self.food_exhaustion = f("foodExhaustionLevel").unwrap_or(0.0) as f32;
        self.food_tick_timer = n("foodTickTimer").unwrap_or(0) as i32;
        self.xp_level = n("XpLevel").unwrap_or(0) as i32;
        self.xp_progress = f("XpP").unwrap_or(0.0) as f32;
        self.xp_total = n("XpTotal").unwrap_or(0) as i32;
        self.selected_slot = n("SelectedItemSlot").unwrap_or(0) as i32;
        self.fall_distance = f("fall_distance").unwrap_or(0.0);
        self.on_ground = n("OnGround").unwrap_or(0) != 0;
        // LivingEntity.readAdditionalSaveData: `getBooleanOr("FallFlying",
        // false)` sets shared flag 7; any numeric tag counts, as a byte.
        self.fall_flying = n("FallFlying").unwrap_or(0) != 0;
        self.seen_credits = n("seenCredits").unwrap_or(0) != 0;
        self.post_effects = match p.get("post_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|t| match t {
                    Tag::String(s) => Some(s.to_string_lossy()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        self.last_death = p.get("LastDeathLocation").and_then(|d| {
            let Some(Tag::IntArray(pos)) = d.get("pos") else {
                return None;
            };
            Some((
                text(d.get("dimension"))?,
                [*pos.first()?, *pos.get(1)?, *pos.get(2)?],
            ))
        });
        if let Some(mode) = n("playerGameType") {
            self.creative = mode == 1;
            self.game_mode = mode as i32;
        }
        self.previous_game_mode = n("previousPlayerGameType").map(|m| m as i32);
        self.flying = p
            .get("abilities")
            .and_then(|a| number(a.get("flying")))
            .is_some_and(|v| v != 0);
        self.fly_speed = p
            .get("abilities")
            .and_then(|a| float(a.get("flySpeed")))
            .map(|v| v as f32);
        self.abilities = p.get("abilities").map(|a| {
            let flag = |key: &str, default: bool| number(a.get(key)).map_or(default, |v| v != 0);
            let speed = |key: &str, default: f32| {
                float(a.get(key))
                    .map(|v| v as f32)
                    .filter(|v| v.is_finite())
                    .unwrap_or(default)
            };
            crate::lan::abilities::PlayerAbilities {
                invulnerable: flag("invulnerable", false),
                flying: flag("flying", false),
                may_fly: flag("mayfly", false),
                instabuild: flag("instabuild", false),
                may_build: flag("mayBuild", true),
                flying_speed: speed("flySpeed", 0.05),
                walking_speed: speed("walkSpeed", 0.1),
            }
        });
        if let Some(d) = text(p.get("Dimension")) {
            self.dimension = d;
        }
        if let Some(respawn) = respawn::read(p) {
            self.respawn_elsewhere = Some(respawn);
        }
        let mut inventory = Vec::new();
        let mut inventory_components = HashMap::new();
        if let Some(Tag::List(items)) = p.get("Inventory") {
            for it in items {
                let slot = number(it.get("Slot")).unwrap_or(0) as i8 as u8;
                if let Some(stack) = item_from_nbt(it) {
                    let slot = if slot as i8 == -106 { 150 } else { slot };
                    if let Some(extra) = &stack.extra {
                        inventory_components.insert(slot, extra.clone());
                    }
                    // Legacy armor slots 100..103 and offhand -106 (150).
                    let slot = if slot as i8 == -106 { 150 } else { slot };
                    inventory.push((slot, stack.item.to_string(), stack.count, stack.damage));
                }
            }
        }
        if let Some(eq) = p.get("equipment") {
            let eq = sunrise_items::equipment::Equipment::read(eq);
            for (ordinal, slot) in PLAYER_EQUIPMENT_SLOTS {
                if let Some(stack) = eq.get(ordinal) {
                    inventory.retain(|(s, ..)| *s != slot);
                    if let Some(extra) = &stack.extra {
                        inventory_components.insert(slot, extra.clone());
                    }
                    inventory.push((slot, stack.item.to_string(), stack.count, stack.damage));
                }
            }
        }
        self.inventory = inventory;
        self.ender_items = decode_items(p.get("EnderItems"), 27);
        self.recipe_book = crate::recipe_book::Settings::read(p.get("recipeBook"));
        self.inventory_components = inventory_components;
        self.xp_seed = n("XpSeed").unwrap_or(0) as i32;
        self.active_effects = match p.get("active_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| Some(read_effect_details(e, &text(e.get("id"))?)))
                .collect(),
            _ => Vec::new(),
        };
        // ServerPlayer.saveParentVehicle: the mount the player rides, by
        // UUID; the vehicle tree stays in the raw compound until it is back.
        self.root_vehicle = p
            .get("RootVehicle")
            .and_then(|r| uuid_from_tag(r.get("Attach")));
    }
    /// The player compound: loaded data with current state written over it.
    /// `LivingEntity.addAdditionalSaveData` / `ServerPlayer.addAdditionalSaveData`:
    /// the live fall fields and the player's empty brain are saved too.
    pub fn player_tag(&self, items: &crate::items::Items) -> Tag {
        let mut p = compound_or_empty(self.player_extra.clone());
        // Keys rewritten from scratch.
        if let Tag::Compound(map) = &mut p {
            for key in [
                "Inventory",
                "EnderItems",
                "equipment",
                "active_effects",
                "post_effects",
                "Pos",
                "respawn",
                "recipeBook",
                "LastDeathLocation",
                "previousPlayerGameType",
            ] {
                map.remove(&JavaString::from(key));
            }
        }
        let mut inventory = Vec::new();
        let mut equipment = sunrise_items::equipment::Equipment::default();
        for (slot, id, count, damage) in &self.inventory {
            let extra = self.inventory_components.get(slot).cloned();
            let stack = crate::items::ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra,
            };
            match *slot {
                0..=35 => {
                    let mut entries = vec![("Slot", Tag::Byte(*slot as i8))];
                    entries.extend(encode_item(&stack, items));
                    inventory.push(Tag::compound(entries));
                }
                _ => {
                    if let Some((ordinal, _)) = PLAYER_EQUIPMENT_SLOTS
                        .iter()
                        .find(|(_, saved)| saved == slot)
                    {
                        equipment.set(*ordinal, Some(stack));
                    }
                }
            }
        }
        // The stored abilities, else GameType.updatePlayerAbilities.
        let mode = self.mode();
        let (creative, spectator) = (mode == 1, mode == 3);
        let mut abilities = compound_or_empty(p.get("abilities").cloned());
        if let Some(a) = self.abilities {
            for (k, v) in [
                ("invulnerable", a.invulnerable),
                ("flying", a.flying),
                ("mayfly", a.may_fly),
                ("instabuild", a.instabuild),
                ("mayBuild", a.may_build),
            ] {
                put(&mut abilities, k, Tag::Byte(i8::from(v)));
            }
            put(
                &mut abilities,
                "flySpeed",
                Tag::Float(a.flying_speed.to_bits()),
            );
            put(
                &mut abilities,
                "walkSpeed",
                Tag::Float(a.walking_speed.to_bits()),
            );
        } else {
            for (k, v) in [
                ("invulnerable", creative || spectator),
                ("mayfly", creative || spectator),
                ("instabuild", creative),
                ("mayBuild", mode < 2),
            ] {
                put(&mut abilities, k, Tag::Byte(i8::from(v)));
            }
            if spectator {
                put(&mut abilities, "flying", Tag::Byte(1));
            } else if creative {
                put(&mut abilities, "flying", Tag::Byte(i8::from(self.flying)));
            } else {
                put(&mut abilities, "flying", Tag::Byte(0));
            }
        }
        if let Tag::Compound(map) = &mut abilities {
            map.entry("mayBuild".into()).or_insert(Tag::Byte(1));
            if let Some(speed) = self.fly_speed {
                map.insert("flySpeed".into(), Tag::Float(speed.to_bits()));
            }
            map.entry("flySpeed".into())
                .or_insert(Tag::Float(0.05f32.to_bits()));
            map.entry("walkSpeed".into())
                .or_insert(Tag::Float(0.1f32.to_bits()));
            map.entry("flying".into()).or_insert(Tag::Byte(0));
        }
        let impulse_grace = self
            .player_impulse_grace
            .map(Tag::Int)
            .or_else(|| p.get("current_impulse_context_reset_grace_time").cloned())
            .unwrap_or(Tag::Int(0));
        let fall_particles = self
            .player_fall_particles
            .map(|v| Tag::Byte(i8::from(v)))
            .or_else(|| p.get("spawn_extra_particles_on_fall").cloned())
            .unwrap_or(Tag::Byte(0));
        let uuid = self.player_uuid.unwrap_or([0, 0, 0, 0]);
        let mut entries: Vec<(&str, Tag)> = vec![
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("UUID", uuid_tag(uuid)),
            ("Rotation", floats(&self.rotation)),
            ("Motion", doubles(&self.motion)),
            ("OnGround", Tag::Byte(i8::from(self.on_ground))),
            ("fall_distance", Tag::Double(self.fall_distance.to_bits())),
            // LivingEntity.addAdditionalSaveData: `putBoolean("FallFlying",
            // isFallFlying())`, always written.
            ("FallFlying", Tag::Byte(i8::from(self.fall_flying))),
            ("Health", Tag::Float(self.health.to_bits())),
            // Player inherits LivingEntity.makeBrain: a new empty Brain;
            // Brain.Packed's codec stores its empty memories map.
            (
                "Brain",
                Tag::compound([("memories", Tag::Compound(Default::default()))]),
            ),
            ("current_impulse_context_reset_grace_time", impulse_grace),
            ("spawn_extra_particles_on_fall", fall_particles),
            ("AbsorptionAmount", Tag::Float(self.absorption.to_bits())),
            ("foodLevel", Tag::Int(self.food)),
            ("foodSaturationLevel", Tag::Float(self.saturation.to_bits())),
            (
                "foodExhaustionLevel",
                Tag::Float(self.food_exhaustion.to_bits()),
            ),
            ("foodTickTimer", Tag::Int(self.food_tick_timer)),
            ("XpLevel", Tag::Int(self.xp_level)),
            ("XpP", Tag::Float(self.xp_progress.to_bits())),
            ("XpTotal", Tag::Int(self.xp_total)),
            ("SelectedItemSlot", Tag::Int(self.selected_slot)),
            ("XpSeed", Tag::Int(self.xp_seed)),
            (
                "active_effects",
                Tag::List(
                    self.active_effects
                        .iter()
                        .map(|e| {
                            let mut effect = vec![("id", string(&e.id))];
                            effect.extend(effect_details(e));
                            Tag::compound(effect)
                        })
                        .collect(),
                ),
            ),
            ("playerGameType", Tag::Int(self.mode())),
            ("Inventory", Tag::List(inventory)),
            ("EnderItems", encode_items(&self.ender_items, items)),
            ("recipeBook", self.recipe_book.write()),
            ("abilities", abilities),
            (
                "Dimension",
                string(if self.dimension.is_empty() {
                    "minecraft:overworld"
                } else {
                    &self.dimension
                }),
            ),
        ];
        // LivingEntity.addAdditionalSaveData omits empty equipment and an empty
        // effect list. PlayerEquipment.isEmpty also counts the selected hotbar
        // item, which the codec never writes: holding one saves `equipment: {}`.
        let holding = self
            .inventory
            .iter()
            .any(|(slot, _, count, _)| i32::from(*slot) == self.selected_slot && *count > 0);
        match equipment.write(items) {
            Some(tag) => entries.push(("equipment", tag)),
            None if holding => entries.push(("equipment", Tag::Compound(Default::default()))),
            None => {}
        }
        if self.active_effects.is_empty() {
            entries.retain(|(key, _)| *key != "active_effects");
        }
        if let Some(pos) = self.position {
            entries.push(("Pos", doubles(&pos)));
        }
        entries.push(("seenCredits", Tag::Byte(i8::from(self.seen_credits))));
        // `output.store` writes the list even when it is empty.
        entries.push((
            "post_effects",
            Tag::List(self.post_effects.iter().map(|e| string(e)).collect()),
        ));
        if let Some((dimension, pos)) = &self.last_death {
            entries.push((
                "LastDeathLocation",
                Tag::compound([
                    ("dimension", string(dimension)),
                    ("pos", Tag::IntArray(pos.to_vec())),
                ]),
            ));
        }
        // A respawn point other than the world spawn: an anchor elsewhere, or a bed.
        if let Some(saved) = &self.respawn_elsewhere {
            entries.push(("respawn", respawn::write(saved)));
        } else if let Some(s) = self.spawn {
            let block = [
                s[0].floor() as i32,
                s[1].floor() as i32,
                s[2].floor() as i32,
            ];
            // A point of the player's own in the Overworld other than a bed
            // (`respawn_elsewhere`): `spawnpoint`'s, forced.
            if Some(block) != self.world_spawn {
                entries.push((
                    "respawn",
                    Tag::compound([
                        ("dimension", string("minecraft:overworld")),
                        ("pos", Tag::IntArray(block.to_vec())),
                        ("yaw", Tag::Float(0f32.to_bits())),
                        ("pitch", Tag::Float(0f32.to_bits())),
                        ("forced", Tag::Byte(1)),
                    ]),
                ));
            }
        }
        // ServerPlayer.storeGameTypes: storeNullable, so absent without one.
        if let Some(mode) = self.previous_game_mode {
            entries.push(("previousPlayerGameType", Tag::Int(mode)));
        }
        for (key, value) in entries {
            put(&mut p, key, value);
        }
        if let Tag::Compound(map) = &mut p {
            for (key, value) in [
                ("Air", Tag::Short(300)),
                ("Fire", Tag::Short(-20)),
                ("Invulnerable", Tag::Byte(0)),
                ("PortalCooldown", Tag::Int(0)),
                ("Score", Tag::Int(0)),
                ("SleepTimer", Tag::Short(0)),
                ("XpSeed", Tag::Int(0)),
                ("seenCredits", Tag::Byte(0)),
                ("HurtTime", Tag::Short(0)),
                ("DeathTime", Tag::Short(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        // ServerPlayer.saveParentVehicle: the savers put the whole
        // RootVehicle in the compound; a mount known by UUID alone (older
        // Sunrise files) is still named.
        if let Some(vehicle) = self.root_vehicle
            && p.get("RootVehicle").is_none()
        {
            put(
                &mut p,
                "RootVehicle",
                Tag::compound([("Attach", uuid_tag(vehicle))]),
            );
        }
        p
    }
    /// Write every file; a world without a player UUID gets a random one.
    pub fn write(&mut self, dir: &Path, items: &crate::items::Items) -> Result<()> {
        self.write_internal(dir, Some(items), false, false)
    }

    /// Dedicated servers save connected players separately.
    pub fn write_without_player(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, None, false, false)
    }

    /// `level.dat` without a new player file: the single-player owner's file stays as last
    /// saved, and a legacy `Data.Player` (before the first normal save the owner's only copy)
    /// stays in `level.dat`.
    pub fn write_keeping_player(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, None, false, true)
    }

    /// The temporary world a Realms upload packs
    /// (`RealmsCreateWorldFlow.createTemporaryWorldFolder`): level.dat as
    /// `WorldData.createTag(null)` writes it — no `singleplayer_uuid` — with the world
    /// generation and game rules, and no player file or weather/clocks the server makes
    /// on its own. The caller writes the data packs and removes the folder.
    pub fn write_upload(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, None, true, false)
    }

    /// The player's file alone, `players/data/<singleplayer_uuid>.dat`
    /// (`PlayerDataStorage.save`): the one level.dat names.
    pub fn write_player(&mut self, dir: &Path, items: &crate::items::Items) -> Result<()> {
        let uuid = self.ensure_player_uuid();
        let player_file = dir
            .join("players")
            .join("data")
            .join(format!("{}.dat", uuid_text(uuid)));
        back_up_unreadable_player(&player_file);
        write_dat(&player_file, self.player_tag(items))
    }

    fn write_internal(
        &mut self,
        dir: &Path,
        items: Option<&crate::items::Items>,
        upload: bool,
        keep_legacy_player: bool,
    ) -> Result<()> {
        fs::create_dir_all(dir)?;
        restore_level_dat(dir);
        let mut root = compound_or_empty(read_level_dat(dir));
        let legacy = root
            .get("Data")
            .is_some_and(|d| d.get("Player").is_some() || d.get("WorldGenSettings").is_some());
        let mut data = compound_or_empty(root.get("Data").cloned());
        let kept_player = keep_legacy_player
            .then(|| data.get("Player").cloned())
            .flatten();
        if let Tag::Compound(map) = &mut data {
            // Pre-26.3 keys now stored elsewhere (a legacy Sunrise level.dat).
            for key in [
                "Player",
                "WorldGenSettings",
                "DayTime",
                "SpawnX",
                "SpawnY",
                "SpawnZ",
                "SpawnAngle",
                "clearWeatherTime",
                "rainTime",
                "thunderTime",
                "raining",
                "thundering",
                "Difficulty",
                "hardcore",
                "GameRules",
                "DimensionData",
                "WanderingTraderSpawnChance",
                "WanderingTraderSpawnDelay",
                "WanderingTraderId",
                "BorderCenterX",
                "BorderCenterZ",
                "BorderSize",
                "BorderSafeZone",
                "BorderWarningBlocks",
                "BorderWarningTime",
                "BorderSizeLerpTime",
                "BorderSizeLerpTarget",
                "BorderDamagePerBlock",
                "DifficultyLocked",
                "ScheduledEvents",
                "CustomBossEvents",
                "DragonFight",
            ] {
                map.remove(&JavaString::from(key));
            }
        }
        // An upload's level.dat names no player (`createTag(null)`); every other file
        // names this world's own.
        // A keep-owner write names nothing new: the on-disk `singleplayer_uuid` (or its absence)
        // stays, so reopening loads the same owner file as before.
        let uuid = (!upload && !keep_legacy_player).then(|| self.ensure_player_uuid());
        // A new world's first spawn point is its world spawn.
        if self.world_spawn.is_none() {
            self.world_spawn = self.spawn.map(|s| {
                [
                    s[0].floor() as i32,
                    s[1].floor() as i32,
                    s[2].floor() as i32,
                ]
            });
        }
        let spawn = self.world_spawn.unwrap_or([0, 64, 0]);
        let mut history: Vec<Tag> = match data.get("version_history") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        if history.last() != Some(&Tag::Int(DATA_VERSION)) {
            history.push(Tag::Int(DATA_VERSION));
        }
        let mut brands: Vec<Tag> = match data.get("ServerBrands") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        if !brands.contains(&string("sunrise")) {
            brands.push(string("sunrise"));
        }
        let mut difficulty = compound_or_empty(data.get("difficulty_settings").cloned());
        put(
            &mut difficulty,
            "difficulty",
            string(if self.difficulty.is_empty() {
                "normal"
            } else {
                &self.difficulty
            }),
        );
        put(
            &mut difficulty,
            "hardcore",
            Tag::Byte(i8::from(self.hardcore)),
        );
        put(
            &mut difficulty,
            "locked",
            Tag::Byte(i8::from(self.difficulty_locked)),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        for (key, value) in [
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "Version",
                Tag::compound([
                    ("Name", string(VERSION_NAME)),
                    ("Id", Tag::Int(DATA_VERSION)),
                    ("Snapshot", Tag::Byte(0)),
                    ("Series", string("main")),
                ]),
            ),
            ("version_history", Tag::List(history)),
            ("version", Tag::Int(19133)),
            ("LevelName", string(&self.name)),
            ("GameType", Tag::Int(self.mode())),
            ("Time", Tag::Long(self.time)),
            ("LastPlayed", Tag::Long(now)),
            (
                "spawn",
                Tag::compound([
                    ("pos", Tag::IntArray(spawn.to_vec())),
                    (
                        "dimension",
                        string(
                            self.world_spawn_dimension
                                .as_deref()
                                .unwrap_or("minecraft:overworld"),
                        ),
                    ),
                    ("yaw", Tag::Float(self.world_spawn_angle[0].to_bits())),
                    ("pitch", Tag::Float(self.world_spawn_angle[1].to_bits())),
                ]),
            ),
            ("difficulty_settings", difficulty),
            ("initialized", Tag::Byte(1)),
            ("ServerBrands", Tag::List(brands)),
        ] {
            put(&mut data, key, value);
        }
        level_mods::write(&mut root);
        if let Some(uuid) = uuid {
            put(&mut data, "singleplayer_uuid", uuid_tag(uuid));
        }
        if let Tag::Compound(map) = &mut data {
            map.entry("allowCommands".into()).or_insert(Tag::Byte(0));
            if let Some(allow) = self.allow_commands {
                map.insert("allowCommands".into(), Tag::Byte(i8::from(allow)));
            }
            map.entry("WasModded".into()).or_insert(Tag::Byte(0));
            // WorldDataConfiguration omits the default, even when the retained Data had a tag.
            if *self.enabled_features == VANILLA_FEATURE_FLAGS {
                map.remove(&JavaString::from("enabled_features"));
            } else {
                map.insert(
                    "enabled_features".into(),
                    Tag::List(self.enabled_features.iter().map(|s| string(s)).collect()),
                );
            }
            map.entry("DataPacks".into()).or_insert_with(|| {
                let (enabled, disabled) = self.data_packs.clone().unwrap_or_else(|| {
                    (
                        vec!["vanilla".into()],
                        [
                            "minecart_improvements",
                            "redstone_experiments",
                            "trade_rebalance",
                        ]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    )
                });
                let list = |ids: Vec<String>| Tag::List(ids.iter().map(|s| string(s)).collect());
                Tag::compound([("Enabled", list(enabled)), ("Disabled", list(disabled))])
            });
        }
        // Saved data: weather, clocks, and (when missing or legacy) world
        // generation settings and game rules. An upload's temporary world carries only
        // what the server needs before its first tick.
        if !upload {
            write_saved_data(
                dir,
                "weather",
                Tag::compound([
                    ("clear_weather_time", Tag::Int(self.weather.clear_time)),
                    ("rain_time", Tag::Int(self.weather.rain_time)),
                    ("thunder_time", Tag::Int(self.weather.thunder_time)),
                    ("raining", Tag::Byte(i8::from(self.weather.raining))),
                    ("thundering", Tag::Byte(i8::from(self.weather.thundering))),
                ]),
            )?;
            let mut clocks = compound_or_empty(read_saved_data(dir, "world_clocks"));
            let mut overworld = compound_or_empty(clocks.get("minecraft:overworld").cloned());
            put(&mut overworld, "total_ticks", Tag::Long(self.day_time));
            put(&mut clocks, "minecraft:overworld", overworld);
            let mut end = compound_or_empty(clocks.get("minecraft:the_end").cloned());
            put(
                &mut end,
                "total_ticks",
                Tag::Long(self.end_clock.unwrap_or(self.time)),
            );
            put(&mut clocks, "minecraft:the_end", end);
            write_saved_data(dir, "world_clocks", clocks)?;
        }
        if let Some(generator) = self.generator.as_mut().filter(|g| g.recovered) {
            write_saved_data(dir, "world_gen_settings", generator.data.clone())?;
            generator.recovered = false;
        } else if legacy || !saved_data_path(dir, "world_gen_settings").exists() {
            write_world_gen_settings(dir, self.seed, WorldGen::default())?;
        }
        if !saved_data_path(dir, "game_rules").exists() || !self.game_rules.is_empty() {
            // Preserve other namespaces even when their mod is not currently loaded.
            let mut rules = match read_saved_data(dir, "game_rules") {
                Some(Tag::Compound(entries)) => Tag::Compound(
                    entries
                        .into_iter()
                        .filter(|(key, _)| {
                            key.to_string().is_ok_and(|s| {
                                s.split_once(':').is_some_and(|(ns, _)| ns != "minecraft")
                            })
                        })
                        .collect(),
                ),
                _ => Tag::Compound(Compound::new()),
            };
            let registry = &self.rule_registry;
            for rule in registry.available_rules(
                self.enabled_features
                    .iter()
                    .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS),
            ) {
                let tag = match rule.default {
                    crate::gamerules::Value::Bool(default) => Tag::Byte(i8::from(
                        self.game_rules
                            .get(&rule.name)
                            .copied()
                            .unwrap_or(i32::from(default))
                            != 0,
                    )),
                    crate::gamerules::Value::Int(default) => {
                        Tag::Int(self.game_rules.get(&rule.name).copied().unwrap_or(default))
                    }
                };
                put(&mut rules, &rule.name.clone(), tag);
            }
            if self
                .enabled_features
                .iter()
                .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS)
            {
                let rule = registry.rule("max_minecart_speed").expect("vanilla rule");
                let crate::gamerules::Value::Int(default) = rule.default else {
                    unreachable!()
                };
                let value = self
                    .game_rules
                    .get(&rule.name)
                    .copied()
                    .filter(|v| (rule.min..=rule.max).contains(v))
                    .unwrap_or(default);
                put(&mut rules, &rule.name.clone(), Tag::Int(value));
            }
            write_saved_data(dir, "game_rules", rules)?;
        }
        if let Some(items) = items {
            self.write_player(dir, items)?;
        }
        if let Some(player) = kept_player {
            put(&mut data, "Player", player);
        }
        put(&mut root, "Data", data);
        write_dat(&dir.join("level.dat"), root)?;
        Ok(())
    }
}
/// Loads saved chunks before falling back to generation.
pub struct Persistent<G> {
    pub inner: Arc<G>,
    pub save: Arc<SaveDir>,
    pub registry: Arc<crate::assets::Assets>,
    pub biomes: Arc<Biomes>,
}
type LodSurfaces = Arc<[crate::lod::Column; 256]>;
/// The coarsest far tile spacing whose columns read saved chunks: 16 blocks
/// a column, and 4 with Reduced far detail (`LodSampler::set_approximate`).
const SAVED_LOD_STRIDE: i32 = 16;
const SAVED_LOD_STRIDE_REDUCED: i32 = 4;
struct SavedLodSampler<G: ChunkGenerator> {
    generator: Arc<Persistent<G>>,
    inner: Box<dyn crate::streaming::LodSampler>,
    cache: std::collections::VecDeque<((i32, i32), Option<LodSurfaces>)>,
    details: Vec<crate::streaming::LodDetail>,
    /// Reduced far detail: saved chunks only in tiles up to
    /// `SAVED_LOD_STRIDE_REDUCED` blocks a column.
    approximate: bool,
    /// Chunks the generator decorated for far terrain alone, kept from
    /// earlier sessions; saved chunks take precedence over them.
    generated: Arc<crate::lod::cache::DiskCache>,
    /// The generator's far samples from earlier sessions (`lod::bake`).
    bake: Option<Arc<crate::lod::bake::LodBake>>,
}
impl<G: ChunkGenerator> SavedLodSampler<G> {
    fn saved_columns(&mut self, key: (i32, i32)) -> Option<LodSurfaces> {
        let cached = match self.cache.iter().position(|(at, _)| *at == key) {
            Some(index) => self.cache.remove(index).unwrap(),
            None => (
                key,
                self.generator.save.lod_surfaces_shared(
                    key.0,
                    key.1,
                    &self.generator.registry,
                    &self.generator.biomes,
                ),
            ),
        };
        let columns = cached.1.clone();
        self.cache.push_front(cached);
        self.cache.truncate(128);
        columns
    }
}
impl<G: ChunkGenerator> crate::streaming::LodSampler for SavedLodSampler<G> {
    fn fork(&self) -> Option<Box<dyn crate::streaming::LodSampler>> {
        Some(Box::new(Self {
            generator: self.generator.clone(),
            inner: self.inner.fork()?,
            cache: Default::default(),
            details: Vec::new(),
            approximate: self.approximate,
            generated: self.generated.clone(),
            bake: self.bake.clone(),
        }))
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
        self.inner.set_approximate(approximate);
    }
    fn request_detail(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        let mut missing = Vec::new();
        for &key in chunks {
            // Generated chunks from earlier sessions answer only these
            // requests: reading them for every sampled cell of a coarse tile
            // slowed sampling more than it saved.
            if let Some(columns) = self
                .saved_columns(key)
                .or_else(|| self.generated.get(key.0, key.1))
            {
                self.details.push((key, columns));
            } else {
                missing.push(key);
            }
        }
        self.inner.request_detail(&missing, center);
    }
    fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
        let mut details = std::mem::take(&mut self.details);
        for (key, generated) in self.inner.drain_details() {
            // A generation batch also returns neighbouring, unrequested chunks.
            // Their saved builds must take precedence too.
            let columns = self.saved_columns(key).unwrap_or_else(|| {
                self.generated.put(key.0, key.1, &generated);
                generated
            });
            details.push((key, columns));
        }
        details
    }
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<crate::lod::Column> {
        let approximate = self.approximate;
        let baked = (self.bake.as_ref()).and_then(|b| b.grid(approximate, [x, z], stride, side));
        let mut columns = baked.unwrap_or_else(|| {
            let columns = self.inner.grid(x, z, stride, side);
            if let Some(bake) = &self.bake {
                bake.put_grid(approximate, [x, z], stride, side, &columns);
            }
            columns
        });
        // Saved chunks show in tiles up to 16 blocks a column. With Reduced far
        // detail only up to 4 (within about a kilometre): coarser tiles keep
        // the generator's terrain, which saved chunks were made from, since
        // decoding up to 256 saved chunks for each of them held the whole far
        // view back for minutes in large saves. Edits that far away show once
        // their tiles refine on approach.
        let limit = if self.approximate {
            SAVED_LOD_STRIDE_REDUCED
        } else {
            SAVED_LOD_STRIDE
        };
        if stride <= limit {
            for (index, column) in columns.iter_mut().enumerate() {
                let (wx, wz) = (
                    x + (index % side) as i32 * stride,
                    z + (index / side) as i32 * stride,
                );
                if let Some(saved) = self.saved_columns((wx >> 4, wz >> 4)) {
                    *column = crate::lod::Column::reduced(
                        &saved,
                        (wx & 15) as usize,
                        (wz & 15) as usize,
                        stride as usize,
                    );
                }
            }
        }
        columns
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::columns::Surface> {
        self.column(x, z).surface
    }
    fn column(&mut self, x: i32, z: i32) -> crate::lod::Column {
        self.reduced_column(x, z, 1)
    }
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> crate::lod::Column {
        if width <= 16
            && let Some(columns) = self.saved_columns((x >> 4, z >> 4))
        {
            return crate::lod::Column::reduced(
                &columns,
                (x & 15) as usize,
                (z & 15) as usize,
                width as usize,
            );
        }
        self.inner.reduced_column(x, z, width)
    }
}

/// Unpack a section's paletted container into `out` (4096 blocks or 64 biome
/// cells): each entry's palette index as stored, unchecked. Returns the
/// palette's length, `None` for a container vanilla does not write. Once a
/// section: far terrain looked both up by name for each block (#6386).
fn lod_palette_indices(container: &Tag, out: &mut [u16], minimum_bits: u32) -> Option<usize> {
    let palette = match container.get("palette")? {
        Tag::List(palette) if (1..=out.len()).contains(&palette.len()) => palette,
        _ => return None,
    };
    if palette.len() == 1 {
        out.fill(0);
        return Some(1);
    }
    let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(minimum_bits) as usize;
    let per_word = 64 / bits;
    let Tag::LongArray(data) = container.get("data")? else {
        return None;
    };
    if data.len() != out.len().div_ceil(per_word) {
        return None;
    }
    for (word, indices) in data.iter().zip(out.chunks_mut(per_word)) {
        let mut word = *word as u64;
        for index in indices {
            *index = (word & ((1 << bits) - 1)) as u16;
            word >>= bits;
        }
    }
    Some(palette.len())
}
fn lod_light_layers(sections: &[(i32, &Tag)], layout: ColumnLayout, sky_light: bool) -> Box<[u8]> {
    let mut layers = vec![(None, None); layout.sections()];
    for &(y, section) in sections {
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(data)) if data.len() == 2048 => Some(&data[..]),
            _ => None,
        };
        layers[(y - (layout.min_y() >> 4)) as usize] = (
            if sky_light { layer("SkyLight") } else { None },
            layer("BlockLight"),
        );
    }
    let mut light = vec![0u8; layout.sections() * 4096].into_boxed_slice();
    let mut above = [if sky_light { 15u8 } else { 0 }; 256];
    let nibble = |data: &[u8], index: usize| (data[index / 2] >> ((index & 1) * 4)) & 15;
    for si in (0..layout.sections()).rev() {
        let (sky, block) = layers[si];
        let section = &mut light[si * 4096..(si + 1) * 4096];
        for (index, value) in section.iter_mut().enumerate() {
            *value = sky.map_or(above[index & 255], |data| nibble(data, index)) << 4
                | block.map_or(0, |data| nibble(data, index));
        }
        // Missing sky layers inherit the bottom layer above, as load_light does.
        for (index, value) in above.iter_mut().enumerate() {
            *value = section[index] >> 4;
        }
    }
    light
}
fn lod_face_light(light: &[u8], x: usize, y: usize, z: usize) -> u16 {
    let index = y * 256 + z * 16 + x;
    let mut sky = light[index] >> 4;
    let mut block = light[index] & 15;
    for (valid, index) in [
        (x > 0, index.wrapping_sub(1)),
        (x < 15, index + 1),
        (z > 0, index.wrapping_sub(16)),
        (z < 15, index + 16),
        (y > 0, index.wrapping_sub(256)),
        (index + 256 < light.len(), index + 256),
    ] {
        if valid {
            sky = sky.max(light[index] >> 4);
            block = block.max(light[index] & 15);
        }
    }
    u16::from(sky) << 4 | u16::from(block) << 12
}
fn decode_lod_surfaces(
    root: &NamedTag,
    assets: &crate::assets::Assets,
    biomes: &Biomes,
    layout: ColumnLayout,
    sky_light: bool,
) -> Option<LodSurfaces> {
    let Tag::List(sections) = root.tag.get("sections")? else {
        return None;
    };
    let water_state = assets.registry.default_state("minecraft:water");
    let mut sections: Vec<_> = sections
        .iter()
        .filter_map(|section| {
            let y = number(section.get("Y"))? as i32;
            layout
                .section_index(y * 16)
                .is_some()
                .then_some((y, section))
        })
        .collect();
    sections.sort_unstable_by_key(|(y, _)| std::cmp::Reverse(*y));
    let mut columns: Box<[crate::lod::Column; 256]> =
        Box::new(std::array::from_fn(|_| crate::lod::Column {
            surface: None,
            spans: crate::lod::Spans::new(),
        }));
    let mut water = [None; 256];
    let light = lod_light_layers(&sections, layout, sky_light);
    let mut states = BTreeMap::new();
    let mut indices = [0u16; 4096];
    for (section_y, section) in sections {
        if let Some(blocks) = section.get("block_states") {
            let Tag::List(palette) = blocks.get("palette")? else {
                return None;
            };
            // Resolve each state once per palette, rather than parsing NBT for every block.
            let palette: Vec<_> = palette
                .iter()
                .map(|entry| {
                    let descriptor = BlockState::from_nbt(entry).ok()?;
                    let state = *states.entry(descriptor.clone()).or_insert_with(|| {
                        assets
                            .registry
                            .id(&descriptor)
                            .or_else(|| assets.registry.default_state(&descriptor.name))
                    });
                    Some(state.map(|state| {
                        let properties = assets.properties.heightmap_state(state);
                        let fluid = matches!(
                            descriptor.name.as_str(),
                            "minecraft:water" | "minecraft:lava"
                        );
                        // Chorus is thin and non-solid but still far-End detail.
                        let chorus = matches!(
                            descriptor.name.as_str(),
                            "minecraft:chorus_plant" | "minecraft:chorus_flower"
                        );
                        let solid = properties.blocks_motion_in_heightmap();
                        // Kelp and seagrass: the water they hold. Left out, the
                        // water stopped short where they reach the surface.
                        match water_state {
                            Some(water) if !fluid && !solid && properties.has_fluid() => {
                                (water, true, true)
                            }
                            _ => (state, fluid, fluid || solid || chorus),
                        }
                    }))
                })
                .collect::<Option<_>>()?;
            let length = lod_palette_indices(blocks, &mut indices, 4)?;
            if indices.iter().any(|&index| usize::from(index) >= length) {
                return None;
            }
            // The air above the ground, mostly: nothing here is far terrain.
            if !palette.iter().flatten().any(|&(_, _, kept)| kept) {
                continue;
            }
            // Each 4x4x4 cell's biome; 0 where the section's biomes do not say.
            let mut cell_biomes = [0u16; 64];
            if let Some(container) = section.get("biomes")
                && let Some(Tag::List(names)) = container.get("palette")
                && lod_palette_indices(container, &mut cell_biomes, 1).is_some()
            {
                let ids: Vec<u16> = names
                    .iter()
                    .map(|entry| match entry {
                        Tag::String(name) => biomes.id(&name.to_string_lossy()),
                        _ => 0,
                    })
                    .collect();
                for biome in &mut cell_biomes {
                    *biome = ids.get(usize::from(*biome)).copied().unwrap_or(0);
                }
            }
            for z in 0..16 {
                for x in 0..16 {
                    let cell = z * 16 + x;
                    let column = &mut columns[cell];
                    for y in (0..16).rev() {
                        let Some((state, fluid, true)) =
                            palette[usize::from(indices[y * 256 + cell])]
                        else {
                            continue;
                        };
                        let biome = cell_biomes[(y / 4) * 16 + (z / 4) * 4 + x / 4];
                        let height = section_y * 16 + y as i32 + 1;
                        if column.surface.is_none() {
                            if fluid {
                                water[cell].get_or_insert((height, state));
                            } else {
                                column.surface = Some(crate::columns::Surface {
                                    height,
                                    state,
                                    water: water[cell],
                                    biome,
                                });
                            }
                        }
                        let light =
                            lod_face_light(&light, x, (height - 1 - layout.min_y()) as usize, z);
                        let min = (height - 1) as i16;
                        let max = height as i16;
                        if let Some(previous) = column.spans.last_mut()
                            && previous.min == max
                            && (previous.state, previous.biome, previous.light)
                                == (state, biome, light)
                        {
                            previous.min = min;
                        } else {
                            column.spans.push(crate::lod::Span {
                                min,
                                max,
                                state,
                                biome,
                                light,
                            });
                        }
                    }
                }
            }
        }
    }
    // Spans are pushed top down, so the reverse puts the topmost last, as
    // `Column::from_chunk` leaves them. Water keeps the chunk's own light,
    // a level less per block, as the generator's far columns shade it.
    for column in &mut *columns {
        column.spans.reverse();
    }
    Some(Arc::from(columns))
}

impl SaveDir {
    /// `lod_surfaces`, decoded once for all far sampling threads: each saved
    /// chunk lies under far tiles of several levels, sampled on any thread.
    fn lod_surfaces_shared(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        if let Some((at, columns)) = self.lod_decoded.lock().unwrap().chunks.get(&(x, z))
            && at.elapsed() < LOD_DECODED_TTL
        {
            return Some(columns.clone());
        }
        let columns = self.lod_surfaces(x, z, assets, biomes)?;
        let mut decoded = self.lod_decoded.lock().unwrap();
        let fresh = (std::time::Instant::now(), columns.clone());
        if decoded.chunks.insert((x, z), fresh).is_none() {
            decoded.order.push_back((x, z));
            while decoded.order.len() > LOD_DECODED {
                if let Some(old) = decoded.order.pop_front() {
                    decoded.chunks.remove(&old);
                }
            }
        }
        Some(columns)
    }
    fn lod_surfaces(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        let root = self.far_record(x, z)?;
        // Proto saves may still be missing decoration from their neighbors.
        if text(root.tag.get("Status"))
            .is_some_and(|status| status != "minecraft:full" && status != "full")
        {
            return None;
        }
        decode_lod_surfaces(
            &root,
            assets,
            biomes,
            self.shape.column(),
            self.shape.sky_light,
        )
    }
}

impl<G: ChunkGenerator> ChunkGenerator for Persistent<G> {
    type Worker = G::Worker;
    fn worker(&self) -> G::Worker {
        let save = self.save.clone();
        self.inner.set_saved_starts(Arc::new(move |pos| {
            let root = save.raw_chunk(pos.x, pos.z)?;
            match root.tag.get("structures")?.get("starts")? {
                Tag::Compound(starts) => Some(starts.clone()),
                _ => None,
            }
        }));
        self.inner.worker()
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn crate::streaming::LodSampler> {
        let inner = ChunkGenerator::lod_sampler(self.inner.clone());
        let generated = Arc::new(crate::lod::cache::DiskCache::new(
            self.save.root.join("data/sunrise_far_terrain"),
            crate::lod::cache::build_key(self.registry.registry.len(), self.biomes.list.len()),
        ));
        let bake = crate::lod::bake::LodBake::open(
            &self.save.root,
            self.save.shape,
            self.registry.registry.len(),
            self.biomes.list.len(),
        )
        .map(Arc::new);
        Box::new(SavedLodSampler {
            generator: self,
            inner,
            cache: Default::default(),
            details: Vec::new(),
            approximate: false,
            generated,
            bake,
        })
    }
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let mut chunk = self.save.load(x, z, self.registry.registry, &self.biomes)?;
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        if let Ok(entities) = self.save.load_entities(x, z) {
            chunk.entities.extend(entities);
        }
        Some(chunk)
    }
    fn lod_surface(
        &self,
        worker: &mut G::Worker,
        x: i32,
        z: i32,
    ) -> Option<crate::columns::Surface> {
        // Resident/visited summaries override this procedural fallback in the LOD cache.
        self.inner.lod_surface(worker, x, z)
    }
    fn prepare_loaded(&self, worker: &mut G::Worker, chunk: &Chunk) {
        self.inner.prepare_loaded(worker, chunk);
    }
    fn all_idle(&self) {
        self.inner.all_idle();
    }
    fn idle(&self, worker: &mut G::Worker) {
        self.inner.idle(worker);
    }
    fn proto(&self, worker: &mut G::Worker, x: i32, z: i32) -> crate::streaming::Proto {
        let mut proto = self.inner.proto(worker, x, z);
        if let Some(saved) = self
            .save
            .load_proto(x, z, self.registry.registry, &self.biomes)
        {
            self.inner.restore_proto(&mut proto, saved);
        }
        proto
    }
    fn is_decorated(&self, proto: &crate::streaming::Proto) -> bool {
        self.inner.is_decorated(proto)
    }
    fn proto_status(
        &self,
        proto: &crate::streaming::Proto,
    ) -> Option<sunrise_world::chunk_status::ChunkStatus> {
        self.inner.proto_status(proto)
    }
    fn proto_block(&self, proto: &crate::streaming::Proto, pos: [i32; 3]) -> Option<u32> {
        self.inner.proto_block(proto, pos)
    }
    fn light_view(&self, proto: &crate::streaming::Proto) -> Option<Chunk> {
        self.inner.light_view(proto)
    }
    fn non_air_sections(
        &self,
        proto: &crate::streaming::Proto,
    ) -> Option<crate::section_mask::SectionMask> {
        self.inner.non_air_sections(proto)
    }
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        self.inner.cache_census()
    }
    fn proto_heap_bytes(&self, proto: &crate::streaming::Proto) -> ([usize; 6], [usize; 16]) {
        self.inner.proto_heap_bytes(proto)
    }
    fn save_protos(&self, protos: &[&crate::streaming::Proto]) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let saved: Vec<SavedProto> = protos
            .iter()
            .filter_map(|p| self.inner.saved_proto(p))
            .collect();
        if let Err(e) = self
            .save
            .save_protos(saved, self.registry.registry, &self.biomes)
        {
            log::warn!("Sunrise: failed to save proto chunks: {e}");
        }
    }
    fn save_finished(&self, mut chunk: Chunk) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let key = (chunk.x, chunk.z);
        let entities = std::mem::take(&mut chunk.entities);
        let saved = self
            .save
            .save_later(&[Arc::new(chunk)], self.registry.registry, &self.biomes)
            .and_then(|_| self.save.save_entities_later(vec![(key, entities)]));
        if let Err(e) = saved {
            log::warn!("Sunrise: failed to save chunk {} {}: {e}", key.0, key.1);
        }
    }
    fn decorates(&self) -> bool {
        self.inner.decorates()
    }
    fn decorates_beside_final(&self) -> bool {
        self.inner.decorates_beside_final()
    }
    fn proto_beside_final(
        &self,
        worker: &mut G::Worker,
        snapshot: &Chunk,
    ) -> Option<crate::streaming::Proto> {
        self.inner.proto_beside_final(worker, snapshot)
    }
    fn final_writes(
        &self,
        snapshot: &Chunk,
        proto: &crate::streaming::Proto,
    ) -> Vec<([i32; 3], u32)> {
        self.inner.final_writes(snapshot, proto)
    }
    fn final_block_entities(
        &self,
        entries: &[(&crate::streaming::Proto, &[([i32; 3], u32)])],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        self.inner.final_block_entities(entries)
    }
    fn decorate(&self, worker: &mut G::Worker, region: &mut [crate::streaming::Proto; 9]) {
        self.inner.decorate(worker, region);
    }
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        self.inner.finish(proto)
    }
}

/// `IOWorker.createOldDataForRegion`'s test of one record: saved before DataVersion 4997, or
/// with `blending_data`.
fn record_is_old(root: &NamedTag) -> bool {
    let before_blending = match root.tag.get("DataVersion") {
        Some(Tag::Int(version)) => *version < 4997,
        _ => true,
    };
    before_blending || root.tag.get("blending_data").is_some()
}

/// `MobEffectInstance.Details` read under its effect's `id`, the
/// `hidden_effect` chain included.
fn read_effect_details(e: &Tag, id: &str) -> crate::effects::SavedEffect {
    let get = |k: &str| number(e.get(k));
    // `Details` stores `amplifier` as `ExtraCodecs.UNSIGNED_BYTE`: the NBT
    // byte is signed, and `UnsignedBytes.toInt` maps it back to 0..=255. A
    // plain sign-extend turns 255 (byte -1) into -1, which
    // `EffectInstance::new` then clamps to the lowest level.
    let amplifier = match e.get("amplifier") {
        Some(Tag::Byte(v)) => i32::from(*v as u8),
        other => number(other).unwrap_or(0) as i32,
    };
    let particles = get("show_particles").is_none_or(|v| v != 0);
    crate::effects::SavedEffect {
        id: id.to_owned(),
        duration: get("duration").unwrap_or(0) as i32,
        amplifier,
        ambient: get("ambient").unwrap_or(0) != 0,
        particles,
        // Details.CODEC: `show_icon` defaults to `show_particles`.
        icon: get("show_icon").map_or(particles, |v| v != 0),
        hidden: e
            .get("hidden_effect")
            .map(|h| Box::new(read_effect_details(h, id))),
    }
}
/// `MobEffectInstance.Details.MAP_CODEC`'s fields, `hidden_effect` nested.
fn effect_details(e: &crate::effects::SavedEffect) -> Vec<(&'static str, Tag)> {
    let mut details = vec![("duration", Tag::Int(e.duration))];
    // Amplifier defaults to zero; when written it is the codec's unsigned
    // byte: `Mth.clamp` to 0..=255, then `byteValue`.
    if e.amplifier != 0 {
        details.push((
            "amplifier",
            Tag::Byte(e.amplifier.clamp(0, 255) as u8 as i8),
        ));
    }
    // optionalFieldOf("ambient", false) and ("show_particles", true):
    // defaults are not written; show_icon is always written.
    if e.ambient {
        details.push(("ambient", Tag::Byte(1)));
    }
    if !e.particles {
        details.push(("show_particles", Tag::Byte(0)));
    }
    details.push(("show_icon", Tag::Byte(i8::from(e.icon))));
    if let Some(hidden) = &e.hidden {
        details.push(("hidden_effect", Tag::compound(effect_details(hidden))));
    }
    details
}

#[cfg(test)]
mod tests {
    use crate::mobs::spawner::SpawnDataExt;

    #[test]
    fn saved_preset_editor_stems_supply_shapes_and_facts_below_pack_stems() {
        use super::*;
        use sunrise_world::worldgen::overrides::{DIMENSIONS, WorldgenOverrides};
        let dir = std::env::temp_dir().join(format!(
            "sunrise-preset-shapes-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dimensions = json_tag(&serde_json::json!({
            "minecraft:overworld": {"type":"minecraft:the_nether","generator":{"type":"minecraft:debug"}},
            "example:pocket": {
                "type":{"min_y":-128,"height":448,"logical_height":320,"has_skylight":false,"has_ceiling":true},
                "generator":{"type":"minecraft:debug"}
            }
        }));
        write_world_gen_settings_with(&dir, 1, WorldGen::default(), None, None, Some(dimensions))
            .unwrap();
        let generator = WorldGenerator::read_new(&dir).unwrap();
        let mut overrides = WorldgenOverrides::default();
        generator.merge_dimensions(&mut overrides);
        let overworld = overrides.level_height("minecraft:overworld").unwrap();
        assert_eq!((overworld.min_y, overworld.height), (0, 256));
        assert!(overworld.has_ceiling && !overworld.has_skylight);
        let pocket = overrides.level_height("example:pocket").unwrap();
        assert_eq!(
            (pocket.min_y, pocket.height, pocket.logical_height),
            (-128, 448, 320)
        );
        assert!(pocket.has_ceiling && !pocket.has_skylight);
        assert!(
            overrides
                .level_facts("minecraft:overworld")
                .unwrap()
                .unwrap()
                .fast_lava
        );
        let pack_stem = r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}"#;
        overrides.set(DIMENSIONS, "minecraft:overworld", pack_stem.into());
        generator.merge_dimensions(&mut overrides);
        assert_eq!(
            overrides
                .entries(DIMENSIONS)
                .find(|(id, _)| *id == "minecraft:overworld")
                .unwrap()
                .1,
            pack_stem
        );
        assert_eq!(
            overrides.level_height("minecraft:overworld").unwrap().min_y,
            -64
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_ender_chest_saves_none_of_the_players_items() {
        use crate::containers::BlockEntity;
        let mut slots = vec![None; 27];
        slots[0] = Some(crate::items::ItemStack::new("minecraft:diamond", 3));
        let chest = super::block_entity_encode::encode(
            (1, 2, 3),
            &BlockEntity::Chest(slots.clone()),
            "minecraft:chest",
            &Default::default(),
        );
        assert!(
            chest.get("Items").is_some(),
            "control: a chest saves its items"
        );
        let ender = super::block_entity_encode::encode(
            (1, 2, 3),
            &BlockEntity::Chest(slots),
            "minecraft:ender_chest",
            &Default::default(),
        );
        assert_eq!(
            ender.get("id"),
            Some(&super::string("minecraft:ender_chest"))
        );
        assert!(ender.get("Items").is_none(), "{ender:?}");
    }
    #[test]
    fn registered_rule_survives_level_save_and_absence() {
        use crate::gamerules::{Rule, RuleRegistry, Value};
        let dir = std::env::temp_dir().join(format!("sunrise-mod-rule-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rule = Rule {
            name: "example:enabled".into(),
            category: "example:machines".into(),
            default: Value::Bool(true),
            min: 0,
            max: 0,
        };
        let registry = std::sync::Arc::new(
            RuleRegistry::with_mod_rules(&["example:machines".into()], &[rule]).unwrap(),
        );
        let mut level = super::Level {
            rule_registry: registry.clone(),
            ..Default::default()
        };
        level.game_rules.insert("example:enabled".into(), 0);
        level.write(&dir, &Default::default()).unwrap();
        assert_eq!(
            super::read_saved_data(&dir, "game_rules")
                .unwrap()
                .get("example:enabled"),
            Some(&super::Tag::Byte(0))
        );
        let mut absent = super::Level::read(&dir).unwrap();
        absent.write(&dir, &Default::default()).unwrap();
        assert_eq!(
            super::read_saved_data(&dir, "game_rules")
                .unwrap()
                .get("example:enabled"),
            Some(&super::Tag::Byte(0))
        );
        let mut returned = super::Level::read_with_registry(&dir, registry).unwrap();
        assert_eq!(returned.game_rules.get("example:enabled"), Some(&0));
        returned.write(&dir, &Default::default()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dedicated_generator_recovery_uses_a_new_seed_and_persists_pack_dimensions() {
        use super::*;
        use sunrise_world::worldgen::overrides::{DIMENSIONS, WorldgenOverrides};

        const RECOVERY_SEED: i64 = -546_172;
        let scratch = tempfile::tempdir().unwrap();
        let world = scratch.path();
        let settings = saved_data_path(world, "world_gen_settings");
        Level::default().write_without_player(world).unwrap();
        let mut overrides = WorldgenOverrides::default();
        overrides.set(
            DIMENSIONS,
            "minecraft:overworld",
            r#"{"type":"test:tall","generator":{"type":"minecraft:noise","settings":"minecraft:overworld","biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#.to_owned(),
        );

        for case in ["missing", "unreadable", "seedless", "no-dimensions"] {
            match case {
                "missing" => fs::remove_file(&settings).unwrap(),
                "unreadable" => fs::write(&settings, b"unreadable generator settings").unwrap(),
                "seedless" | "no-dimensions" => {
                    let Tag::Compound(mut invalid) =
                        read_saved_data(world, "world_gen_settings").unwrap()
                    else {
                        panic!("expected saved settings")
                    };
                    let missing = if case == "seedless" {
                        "seed"
                    } else {
                        invalid.insert("seed".into(), Tag::Long(13_579));
                        "dimensions"
                    };
                    invalid.remove(&missing.into());
                    write_saved_data(world, "world_gen_settings", Tag::Compound(invalid)).unwrap();
                }
                _ => unreachable!(),
            }
            let before = fs::read(&settings).ok();
            let mut recovered = Level::read(world).unwrap();
            recovered
                .recover_world_generator(&overrides, || RECOVERY_SEED)
                .unwrap();
            assert_eq!(recovered.seed, RECOVERY_SEED, "{case}");
            assert!(world_gen_settings(world).is_none(), "{case}");
            assert_eq!(fs::read(&settings).ok(), before, "{case}");
            recovered.write_without_player(world).unwrap();
            let saved = read_saved_data(world, "world_gen_settings").unwrap();
            assert_eq!(saved.get("seed"), Some(&Tag::Long(RECOVERY_SEED)));
            assert_eq!(
                saved
                    .get("dimensions")
                    .and_then(|d| d.get("minecraft:overworld"))
                    .and_then(|d| d.get("type")),
                Some(&Tag::String("test:tall".into())),
            );
        }

        write_world_gen_settings(world, 0, WorldGen::default()).unwrap();
        let mut valid = Level::read(world).unwrap();
        valid
            .recover_world_generator(&overrides, || {
                panic!("a valid saved zero must not draw a new seed")
            })
            .unwrap();
        assert_eq!(valid.seed, 0);
    }

    #[test]
    fn a_server_falls_back_to_level_dat_old_and_restores_it() {
        let world = std::env::temp_dir().join(format!("sunrise-dat-old-{}", std::process::id()));
        let _ = fs::remove_dir_all(&world);
        fs::create_dir_all(&world).unwrap();
        let level = |name: &str| {
            Tag::compound([(
                "Data",
                Tag::compound([("LevelName", Tag::String(name.into()))]),
            )])
        };
        // Both files broken, or only level.dat_old there: nothing to fall back to.
        fs::write(world.join("level.dat"), b"cut short").unwrap();
        assert!(!fall_back_to_level_dat_old(&world));
        fs::write(world.join("level.dat_old"), b"cut short").unwrap();
        assert!(!fall_back_to_level_dat_old(&world));
        // A good level.dat_old: it takes level.dat's place and the broken file is kept.
        write_dat(&world.join("level.dat_old"), level("old")).unwrap();
        assert!(fall_back_to_level_dat_old(&world));
        assert!(level_dat_issue(&world, false).is_none());
        let kept = fs::read_dir(&world)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("level.dat_corrupted_")
            })
            .count();
        assert_eq!(kept, 1);
        // A readable level.dat is left alone.
        assert!(!fall_back_to_level_dat_old(&world));
        let _ = fs::remove_dir_all(&world);
    }

    #[test]
    fn points_of_interest_save_as_poi_sections() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let assets = crate::assets::Assets::load(&PathBuf::from(root)).expect("assets");
        let tables = SaveTables::new(&assets);
        let state = |name: &str, props: &[(&str, &str)]| {
            let mut s = assets
                .registry
                .state(assets.registry.default_state(name).unwrap())
                .unwrap()
                .clone();
            for (k, v) in props {
                s.properties.insert((*k).into(), (*v).into());
            }
            assets.registry.id(&s).unwrap().0
        };
        let mut chunk = Chunk::empty(2, -1);
        chunk.set(1, 70, 1, state("minecraft:red_bed", &[("part", "head")]));
        chunk.set(1, 70, 2, state("minecraft:red_bed", &[("part", "foot")]));
        chunk.set(5, 71, 5, state("minecraft:bell", &[]));
        chunk.set(6, 71, 5, state("minecraft:stone", &[]));
        chunk.set(9, 70, 9, state("minecraft:straw_bed", &[("part", "head")]));
        assert!(encode_poi(&Chunk::empty(0, 0), &tables, None).is_none());
        let saved = encode_poi(&chunk, &tables, None).expect("records");
        let Some(Tag::Compound(sections)) = saved.tag.get("Sections") else {
            panic!("no sections");
        };
        assert_eq!(sections.len(), 1);
        let section = &sections[&"4".into()];
        assert_eq!(section.get("Valid"), Some(&Tag::Byte(1)));
        let Some(Tag::List(records)) = section.get("Records") else {
            panic!("no records");
        };
        let records: Vec<_> = records
            .iter()
            .map(|r| {
                (
                    text(r.get("type")).unwrap(),
                    r.get("pos").cloned(),
                    number(r.get("free_tickets")),
                )
            })
            .collect();
        assert_eq!(
            records,
            [
                (
                    "minecraft:home".into(),
                    Some(Tag::IntArray(vec![33, 70, -15])),
                    Some(1)
                ),
                (
                    "minecraft:meeting".into(),
                    Some(Tag::IntArray(vec![37, 71, -11])),
                    Some(32)
                ),
            ]
        );
    }

    /// A block a mod added to `minecraft:home` (`extend_poi_types`) saves as a
    /// home record, as a bed head does.
    #[test]
    fn states_a_mod_added_to_a_poi_type_save_as_its_records() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let assets = crate::assets::Assets::load(&PathBuf::from(root)).expect("assets");
        let hay_states: Vec<_> = (0..assets.registry.len())
            .filter_map(|i| assets.registry.state(BlockStateId(i as u32)))
            .filter(|state| state.name == "minecraft:hay_block")
            .map(|state| sunrise_world::poi::PoiExtension {
                state: state.clone(),
                poi: "minecraft:home".into(),
            })
            .collect();
        assert_eq!(hay_states.len(), 3);
        // The tables are built once per registry, so they keep the addition.
        let tables = sunrise_world::poi::with_extensions(hay_states, |refusals| {
            assert_eq!(refusals, Vec::<String>::new());
            SaveTables::new(&assets)
        });
        let hay = assets
            .registry
            .default_state("minecraft:hay_block")
            .unwrap();
        assert_eq!(tables.poi[hay.0 as usize], Some(("minecraft:home", 1)));
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        assert_eq!(tables.poi[stone.0 as usize], None);
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(3, 70, 3, hay.0);
        chunk.set(4, 70, 3, stone.0);
        let saved = encode_poi(&chunk, &tables, None).expect("records");
        let Some(Tag::Compound(sections)) = saved.tag.get("Sections") else {
            panic!("no sections");
        };
        let Some(Tag::List(records)) = sections[&"4".into()].get("Records") else {
            panic!("no records");
        };
        let records: Vec<_> = records
            .iter()
            .map(|r| (text(r.get("type")).unwrap(), r.get("pos").cloned()))
            .collect();
        assert_eq!(
            records,
            [("minecraft:home".into(), Some(Tag::IntArray(vec![3, 70, 3])))]
        );
    }

    #[test]
    fn a_chunk_that_loses_its_last_poi_saves_empty_sections() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Arc::new(
            Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
                root.to_path_buf(),
            ))
            .expect("biomes"),
        );
        let bell = assets.registry.default_state("minecraft:bell").unwrap().0;
        let dir = std::env::temp_dir().join(format!("sunrise-poi-save-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        let _ = save.tables.set(Arc::new(SaveTables::new(&assets)));
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(5, 71, 5, bell);
        let sections = |save: &SaveDir| match save.read_record("poi", 0, 0)?.tag.get("Sections") {
            Some(Tag::Compound(s)) => Some(s.len()),
            _ => None,
        };
        save.save_later(&[Arc::new(chunk.clone())], assets.registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        assert_eq!(sections(&save), Some(1));
        chunk.set(5, 71, 5, 0);
        save.save_later(&[Arc::new(chunk)], assets.registry, &biomes)
            .unwrap();
        save.flush().unwrap();
        assert_eq!(
            sections(&SaveDir::new(dir.clone(), LevelShape::OVERWORLD, None)),
            Some(0)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_poi_records_keep_their_claims() {
        let record = |pos: [i32; 3], kind: &str, tickets: Option<i32>| {
            let mut r = vec![("pos", Tag::IntArray(pos.to_vec())), ("type", string(kind))];
            if let Some(t) = tickets {
                r.push(("free_tickets", Tag::Int(t)));
            }
            Tag::compound(r)
        };
        let poi = |records: Vec<Tag>| {
            let mut sections = Compound::new();
            sections.insert(
                "4".into(),
                Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(records))]),
            );
            poi_root(sections)
        };
        let old = poi(vec![
            record([1, 70, 1], "minecraft:home", None),
            record([5, 71, 5], "minecraft:meeting", Some(30)),
        ]);
        let mut saved = poi(vec![
            record([1, 70, 1], "minecraft:home", Some(1)),
            record([5, 71, 5], "minecraft:meeting", Some(32)),
            record([9, 71, 9], "minecraft:home", Some(1)),
        ]);
        keep_claims(&mut saved, &old);
        let expected = poi(vec![
            record([1, 70, 1], "minecraft:home", None),
            record([5, 71, 5], "minecraft:meeting", Some(30)),
            record([9, 71, 9], "minecraft:home", Some(1)),
        ]);
        assert_eq!(saved.tag, expected.tag);
    }

    #[test]
    fn level_attachments_survive_save_and_load() {
        use sunrise_mod_runtime::attachments::{
            AttachmentCodec, AttachmentError, AttachmentType, Attachments,
        };
        let dir =
            std::env::temp_dir().join(format!("sunrise-level-attachments-{}", std::process::id()));
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        let key = AttachmentType {
            namespace: "test",
            name: "counter",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
                decode: |c: &Compound| match c.get(&"count".into()) {
                    Some(Tag::Int(n)) => Ok(*n),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            copy_on_death: false,
            sync: false,
        };
        let mut map = Attachments::default();
        key.set(&mut map, sunrise_mod_runtime::attachments::Holder::Level, 7)
            .unwrap();
        save.save_attachments(&map).unwrap();
        assert_eq!(
            key.get(
                &mut save.load_attachments(),
                sunrise_mod_runtime::attachments::Holder::Level
            ),
            Ok(7)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn loaded_chunk_uses_saved_monument_from_its_referenced_start_chunk() {
        use sunrise_core::{
            geometry::Direction,
            random::{LegacyRandom, WorldgenRandom},
        };
        use sunrise_world::worldgen::structure::{
            legacy::{LegacyPiece, random_horizontal},
            monument::MonumentBuilding,
        };
        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").expect("resources"));
        let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
        let biomes = Arc::new(
            Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
                root.clone(),
            ))
            .expect("biomes"),
        );
        let item_data = crate::test_resources::item_data(
            &root,
            sunrise_world::native_load_context::DataLoadOrigin::ServerDataLoad,
        )
        .expect("item data");
        let inner = Arc::new(
            crate::vanillagen::VanillaTerrain::new(
                crate::vanillagen::TerrainResources {
                    data: None,
                    dir: &root.join("worldgen"),
                    registry: assets.registry,
                    biomes: &biomes,
                    item_data: &item_data,
                },
                12345,
            )
            .unwrap(),
        );
        // A moved, rotated monument that generation cannot reproduce at the owning chunk.
        // Vanilla reseeds, draws the default even with a saved orientation, then rebuilds.
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_seed(12345, -2, 3);
        random_horizontal(&mut random);
        let top = MonumentBuilding::new(&mut random, 100, -300, Direction::East);
        let expected = Tag::compound([
            ("id", string("minecraft:monument")),
            ("ChunkX", Tag::Int(-2)),
            ("ChunkZ", Tag::Int(3)),
            ("references", Tag::Int(5)),
            (
                "Children",
                Tag::List(vec![Tag::Compound(top.save().unwrap())]),
            ),
        ]);
        let mut saved = encode(
            &Chunk::empty(-2, 3),
            assets.registry,
            &biomes,
            &Default::default(),
        );
        let Some(Tag::Compound(structures)) = saved.tag.get_mut("structures") else {
            panic!("structures")
        };
        structures.insert(
            "starts".into(),
            Tag::compound([("minecraft:monument", expected.clone())]),
        );
        let save = Arc::new(SaveDir::new(
            std::env::temp_dir().join(format!("sunrise-saved-monument-{}", std::process::id())),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        save.pending
            .lock()
            .unwrap()
            .insert((Folder::Region, -2, 3), (0, Record::Tag(Arc::new(saved))));
        let generator = Persistent {
            inner: inner.clone(),
            save,
            registry: assets,
            biomes,
        };
        let mut worker = generator.worker();
        let mut referring = Chunk::empty(-1, 3);
        referring
            .structure_refs
            .push(("minecraft:monument".into(), (-2, 3)));
        generator.prepare_loaded(&mut worker, &referring);
        let starts = inner.starts_at(-2, 3);
        let restored = starts
            .iter()
            .find(|s| s.name == "minecraft:monument")
            .expect("referenced saved monument");
        assert_eq!(restored.save(), Some(expected));
    }

    #[test]
    fn loaded_chunk_uses_saved_jigsaw_from_its_referenced_start_chunk() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
        let biomes = Arc::new(
            Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
                root.clone(),
            ))
            .expect("biomes"),
        );
        let item_data = crate::test_resources::item_data(
            &root,
            sunrise_world::native_load_context::DataLoadOrigin::ServerDataLoad,
        )
        .expect("item data");
        let inner = Arc::new(
            crate::vanillagen::VanillaTerrain::new(
                crate::vanillagen::TerrainResources {
                    data: None,
                    dir: &root.join("worldgen"),
                    registry: assets.registry,
                    biomes: &biomes,
                    item_data: &item_data,
                },
                12345,
            )
            .unwrap(),
        );
        let mut original = inner
            .structure_start("minecraft:ancient_city", 2, -3)
            .unwrap();
        Arc::get_mut(&mut original).unwrap().references = 4;
        let starts = Tag::compound([
            ("minecraft:ancient_city", original.save().unwrap()),
            (
                "minecraft:mineshaft",
                Tag::compound([("id", string("INVALID"))]),
            ),
        ]);
        let mut saved = encode(
            &Chunk::empty(2, -3),
            assets.registry,
            &biomes,
            &Default::default(),
        );
        let Tag::Compound(root) = &mut saved.tag else {
            panic!("chunk root")
        };
        let Some(Tag::Compound(structures)) = root.get_mut(&"structures".into()) else {
            panic!("structures")
        };
        structures.insert("starts".into(), starts.clone());
        let loaded = decode(&saved, assets.registry, &biomes).expect("chunk");
        let Tag::Compound(expected) = &starts else {
            unreachable!()
        };
        assert_eq!(loaded.saved_structure_starts(), Some(expected));
        let again = encode(&loaded, assets.registry, &biomes, &Default::default());
        assert_eq!(
            again.tag.get("structures").and_then(|s| s.get("starts")),
            Some(&starts)
        );
        let save = Arc::new(SaveDir::new(
            std::env::temp_dir().join(format!("sunrise-saved-starts-{}", std::process::id())),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        // A pending record is authoritative even before any region file exists.
        save.pending
            .lock()
            .unwrap()
            .insert((Folder::Region, 2, -3), (0, Record::Tag(Arc::new(saved))));
        let generator = Persistent {
            inner: inner.clone(),
            save,
            registry: assets,
            biomes,
        };
        let mut worker = generator.worker();
        let mut referring = Chunk::empty(3, -3);
        referring
            .structure_refs
            .push(("minecraft:ancient_city".into(), (2, -3)));
        generator.prepare_loaded(&mut worker, &referring);
        let starts = inner.starts_at(2, -3);
        let restored = starts
            .iter()
            .find(|s| s.name == original.name)
            .expect("referenced saved start");
        assert_eq!(restored.save(), original.save());
        assert_eq!(format!("{restored:?}"), format!("{original:?}"));
    }

    #[test]
    fn chunk_attachments_survive_save_and_load() {
        use sunrise_mod_runtime::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes");
        let key = AttachmentType {
            namespace: "test",
            name: "counter",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
                decode: |c: &Compound| match c.get(&"count".into()) {
                    Some(Tag::Int(n)) => Ok(*n),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            copy_on_death: false,
            sync: false,
        };
        let mut chunk = Chunk::empty(2, 3);
        let mut extras = ChunkExtras::default();
        key.set(
            &mut extras.attachments,
            sunrise_mod_runtime::attachments::Holder::Level,
            42,
        )
        .unwrap();
        let mut block_data = sunrise_mod_runtime::attachments::Attachments::default();
        key.set(
            &mut block_data,
            sunrise_mod_runtime::attachments::Holder::Level,
            21,
        )
        .unwrap();
        let pos = (33, 64, 50);
        let chest = assets.registry.default_state("minecraft:chest").unwrap();
        chunk.set(1, 64, 2, chest.0);
        chunk
            .block_entities
            .push((pos, crate::containers::BlockEntity::Chest(vec![None; 27])));
        extras.block_entity_attachments.push((pos, block_data));
        chunk.saved = Some(Arc::new(extras));
        let saved = encode(&chunk, assets.registry, &biomes, &Default::default());
        let mut loaded = decode(&saved, assets.registry, &biomes).expect("chunk");
        assert_eq!(
            key.get(
                &mut Arc::make_mut(loaded.saved.as_mut().unwrap()).attachments,
                sunrise_mod_runtime::attachments::Holder::Level
            ),
            Ok(42)
        );
        let extras = Arc::make_mut(loaded.saved.as_mut().unwrap());
        assert_eq!(extras.block_entity_attachments[0].0, pos);
        assert_eq!(
            key.get(
                &mut extras.block_entity_attachments[0].1,
                sunrise_mod_runtime::attachments::Holder::Level
            ),
            Ok(21)
        );
    }

    #[test]
    fn unopened_pending_containers_save_their_loot_table() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes");
        let tables = SaveTables::new(&assets);
        let chest = assets.registry.default_state("minecraft:chest").unwrap();
        let mut slots = vec![None; 27];
        slots[3] = Some(crate::items::ItemStack::new("minecraft:bone", 2));
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(1, 64, 2, chest.0);
        chunk.block_entities.push((
            (1, 64, 2),
            crate::containers::BlockEntity::Chest(slots.clone()),
        ));
        let pending = PendingContainerLoot {
            table: string("minecraft:chests/simple_dungeon"),
            seed: Some(Tag::Long(42)),
        };
        chunk.block_entities[0].1.slots_mut().fill(None);
        chunk.saved = Some(Arc::new(ChunkExtras {
            pending_loot: [((1, 64, 2), pending)].into(),
            ..ChunkExtras::default()
        }));
        let saved_chest = |chunk: &Chunk| -> Tag {
            let saved = encode_with(
                chunk,
                assets.registry,
                &biomes,
                Some(&tables),
                LevelShape::OVERWORLD,
                &Default::default(),
            );
            let Some(Tag::List(list)) = saved.tag.get("block_entities") else {
                panic!("no block entities");
            };
            list[0].clone()
        };
        let unopened = saved_chest(&chunk);
        assert_eq!(
            text(unopened.get("LootTable")).as_deref(),
            Some("minecraft:chests/simple_dungeon")
        );
        assert_eq!(number(unopened.get("LootTableSeed")), Some(42));
        assert!(unopened.get("Items").is_none());
        Arc::make_mut(chunk.saved.as_mut().unwrap())
            .pending_loot
            .remove(&(1, 64, 2));
        // The shared unpack transition consumes metadata before inventory changes.
        if let crate::containers::BlockEntity::Chest(s) = &mut chunk.block_entities[0].1 {
            s[3] = None;
            s[4] = Some(crate::items::ItemStack::new("minecraft:bone", 1));
        }
        let opened = saved_chest(&chunk);
        assert!(opened.get("LootTable").is_none());
        assert!(opened.get("Items").is_some());
        // Loaded with its table, rolled, then emptied: no table, or it refills on load.
        let mut extras = ChunkExtras::clone(chunk.saved.as_ref().unwrap());
        extras.block_entity_sources = vec![(
            (1, 64, 2),
            Tag::compound([
                ("id", string("minecraft:chest")),
                ("x", Tag::Int(1)),
                ("y", Tag::Int(64)),
                ("z", Tag::Int(2)),
                ("LootTable", string("minecraft:chests/simple_dungeon")),
                ("LootTableSeed", Tag::Long(42)),
            ]),
        )];
        chunk.saved = Some(Arc::new(extras));
        if let crate::containers::BlockEntity::Chest(s) = &mut chunk.block_entities[0].1 {
            s.fill(None);
        }
        let emptied = saved_chest(&chunk);
        assert!(emptied.get("LootTable").is_none());
        assert!(emptied.get("LootTableSeed").is_none());
    }

    #[test]
    fn a_tall_level_saves_and_loads_its_whole_height() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes");
        let tables = SaveTables::new(&assets);
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        // To The Max's Overworld: -64..2032, 131 sections.
        let shape = LevelShape::new(-64, 2096, 2096, true, false).unwrap();
        let mut chunk = Chunk::with_layout(0, 0, shape.column());
        chunk.set(0, -64, 0, stone.0);
        chunk.set(0, 2000, 0, stone.0);
        let saved = encode_with(
            &chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            shape,
            &Default::default(),
        );
        assert_eq!(number(saved.tag.get("yPos")), Some(-4));
        let Some(Tag::List(sections)) = saved.tag.get("sections") else {
            panic!("no sections");
        };
        assert_eq!(sections.len(), 131);
        // 12-bit heightmap entries: y 2000's top reads 2001 - -64.
        let Some(Tag::LongArray(surface)) = saved
            .tag
            .get("Heightmaps")
            .and_then(|h| h.get("WORLD_SURFACE"))
        else {
            panic!("no WORLD_SURFACE");
        };
        assert_eq!(surface[0] & 0xfff, 2065);
        let back = decode_with(
            &saved,
            assets.registry,
            &biomes,
            Some(&tables),
            shape.column(),
        )
        .expect("decodes");
        assert_eq!(back.sections.len(), 131);
        assert_eq!(back.get(0, 2000, 0), stone.0);
        assert_eq!(back.get(0, -64, 0), stone.0);
        // Read into the vanilla column, the sections above it are left out.
        let short = decode_with(
            &saved,
            assets.registry,
            &biomes,
            Some(&tables),
            ColumnLayout::VANILLA,
        )
        .expect("decodes");
        assert_eq!(short.sections.len(), SECTIONS);
        assert_eq!(short.get(0, 2000, 0), 0);
        assert_eq!(short.get(0, -64, 0), stone.0);
    }

    /// `BlockEntity.getPosFromTag`: a block entity saved under another chunk's
    /// coordinates is read into the chunk that saved it (vanilla warns and
    /// keeps the low four bits), instead of landing where that chunk has no
    /// block.
    #[test]
    fn a_block_entity_saved_under_another_chunks_coordinates_moves_here() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes");
        // The destination must contain a chest: the fork validates saved block
        // entity membership before loading, independently of position repair.
        // The current saved block-state codec uses `id`, not the old `Name` key.
        let tag = sunrise_core::nbt::parse_snbt(
            r#"{xPos:0,zPos:0,yPos:-4,Status:"minecraft:full",sections:[{Y:4b,block_states:{palette:[{id:"minecraft:chest"}]}}],block_entities:[{id:"minecraft:chest",x:37,y:70,z:-3,Items:[]}]}"#,
        )
        .unwrap()
        .0;
        let chunk = decode_with(
            &NamedTag {
                name: "".into(),
                tag,
            },
            assets.registry,
            &biomes,
            None,
            ColumnLayout::VANILLA,
        )
        .expect("chunk");
        let positions: Vec<(i32, i32, i32)> =
            chunk.block_entities.iter().map(|(p, _)| *p).collect();
        assert_eq!(
            positions,
            [(5, 70, 13)],
            "moved into the chunk that saved it"
        );
        let order = &chunk.saved.as_ref().expect("extras").block_entity_order;
        assert_eq!(
            order,
            &[(5, 70, 13)],
            "load order follows the moved position"
        );
    }

    #[test]
    fn nether_and_end_chunks_save_with_their_own_height() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes");
        let tables = SaveTables::new(&assets);
        let rack = assets
            .registry
            .default_state("minecraft:netherrack")
            .unwrap();
        let mut chunk = Chunk::empty(0, 0);
        for y in 0..128 {
            chunk.set(0, y, 0, rack.0);
        }
        let saved = encode_with(
            &chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            LevelShape::NETHER,
            &Default::default(),
        );
        assert_eq!(number(saved.tag.get("yPos")), Some(0));
        let Some(Tag::List(sections)) = saved.tag.get("sections") else {
            panic!("no sections");
        };
        let ys: Vec<i32> = sections
            .iter()
            .filter_map(|s| number(s.get("Y")).map(|y| y as i32))
            .collect();
        assert_eq!(ys, (0..16).collect::<Vec<_>>());
        let Some(Tag::List(post)) = saved.tag.get("PostProcessing") else {
            panic!("no PostProcessing");
        };
        assert_eq!(post.len(), 16);
        // Heightmap values count from the level's minimum: the column's top
        // block at y=127 reads 128 (vanilla), not 192 (from the Overworld's -64).
        let Some(Tag::LongArray(surface)) = saved
            .tag
            .get("Heightmaps")
            .and_then(|h| h.get("WORLD_SURFACE"))
        else {
            panic!("no WORLD_SURFACE");
        };
        assert_eq!(surface[0] & 0x1ff, 128);
        let overworld = encode_with(
            &chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            LevelShape::OVERWORLD,
            &Default::default(),
        );
        assert_eq!(number(overworld.tag.get("yPos")), Some(-4));
    }

    #[test]
    fn saved_far_surface_reads_builds_and_water_without_gameplay_chunks() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.to_path_buf(),
        ))
        .expect("biomes");
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let sand = assets.registry.default_state("minecraft:sand").unwrap();
        let dirt = assets.registry.default_state("minecraft:dirt").unwrap();
        let leaves = assets
            .registry
            .default_state("minecraft:oak_leaves")
            .unwrap();
        let end_stone = assets
            .registry
            .default_state("minecraft:end_stone")
            .unwrap();
        let chorus = assets
            .registry
            .default_state("minecraft:chorus_plant")
            .unwrap();
        let flower = assets
            .registry
            .default_state("minecraft:chorus_flower")
            .unwrap();
        let mut chunk = Chunk::empty(-2, 3);
        chunk.set(0, 109, 0, stone.0);
        chunk.set(3, 121, 5, stone.0);
        chunk.set(3, 122, 5, stone.0);
        chunk.set(3, 120, 5, dirt.0);
        chunk.set(3, 119, 5, dirt.0);
        chunk.set(3, 115, 5, stone.0);
        chunk.set(11, 125, 11, stone.0);
        chunk.set(14, 127, 14, stone.0);
        chunk.set(14, 112, 14, stone.0);
        for y in 95..98 {
            chunk.set(2, y, 6, stone.0);
        }
        chunk.lit = true;
        let mut cave_light = Box::new([0x03; 4096]);
        cave_light[crate::chunk::index(12, 125, 11)] = 0xa7;
        Arc::make_mut(&mut chunk.sections[((121 - MIN_Y) >> 4) as usize]).light =
            Light::Full(cave_light);
        chunk.set(7, 90, 9, sand.0);
        chunk.set(7, 98, 9, leaves.0);
        let forest = biomes.id("minecraft:forest");
        chunk.set_biome_cell(7 / 4, (98 - MIN_Y) / 4, 9 / 4, forest);
        chunk.set(4, 54, 4, sand.0);
        for y in 55..64 {
            chunk.set(4, y, 4, water.0);
        }
        // Chorus is thin and non-solid but still part of the far End.
        chunk.set(12, 100, 2, end_stone.0);
        chunk.set(12, 101, 2, chorus.0);
        chunk.set(12, 102, 2, flower.0);
        let mut saved = encode(&chunk, assets.registry, &biomes, &Default::default());
        put(
            &mut saved.tag,
            "block_entities",
            Tag::String("not decoded by far sampling".into()),
        );
        let dir = std::env::temp_dir().join(format!("sunrise-lod-save-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        save.write_records("region", vec![((-2, 3), Some(saved))], true)
            .unwrap();
        let reopened = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        let surfaces = reopened
            .lod_surfaces(-2, 3, &assets, &biomes)
            .expect("saved summary");
        assert_eq!(surfaces[0].surface.unwrap().height, 110);
        assert_eq!(surfaces[0].surface.unwrap().state, stone);
        let narrow_build = surfaces[5 * 16 + 3].surface.unwrap();
        assert_eq!((narrow_build.height, narrow_build.state), (123, stone));
        assert!(
            surfaces[5 * 16 + 2].surface.is_none(),
            "adjacent columns stay distinct"
        );
        let strata = &surfaces[5 * 16 + 3].spans;
        assert_eq!(
            strata
                .iter()
                .map(|span| (span.min, span.max, span.state))
                .collect::<Vec<_>>(),
            vec![(115, 116, stone), (119, 121, dirt), (121, 123, stone)]
        );
        assert!(
            strata.iter().all(|span| span.light == 0x3000),
            "saved cave darkness and block light survive"
        );
        assert_eq!(
            surfaces[11 * 16 + 11].spans[0].light,
            0x70a0,
            "neighbor face light uses separate sky/block channels"
        );
        let boundaries = &surfaces[14 * 16 + 14].spans;
        assert_eq!(boundaries.len(), 2);
        assert!(
            boundaries.iter().all(|span| span.light == 0x30f0),
            "face light crosses section boundaries above and below"
        );
        let across_sections = &surfaces[6 * 16 + 2].spans;
        assert_eq!(across_sections.len(), 1);
        assert_eq!((across_sections[0].min, across_sections[0].max), (95, 98));
        let canopy = surfaces[9 * 16 + 7].surface.unwrap();
        assert_eq!(
            (canopy.height, canopy.state, canopy.biome),
            (99, leaves, forest)
        );
        let underwater = surfaces[4 * 16 + 4].surface.unwrap();
        assert_eq!((underwater.height, underwater.state), (55, sand));
        assert_eq!(underwater.water, Some((64, water)));
        let chorus_column = &surfaces[2 * 16 + 12];
        assert_eq!(
            chorus_column
                .surface
                .map(|surface| (surface.height, surface.state)),
            Some((103, flower))
        );
        assert_eq!(
            chorus_column
                .spans
                .iter()
                .map(|span| (span.min, span.max, span.state))
                .collect::<Vec<_>>(),
            vec![
                (100, 101, end_stone),
                (101, 102, chorus),
                (102, 103, flower)
            ]
        );
        assert!(surfaces[15].surface.is_none(), "saved void stays empty");
        assert!(
            reopened.regions.lock().unwrap().is_empty(),
            "far reads do not grow gameplay region cache"
        );
        let mut proto = encode(&chunk, assets.registry, &biomes, &Default::default());
        put(
            &mut proto.tag,
            "Status",
            string(ChunkStatus::Terrain.name()),
        );
        put(&mut proto.tag, "xPos", Tag::Int(-3));
        save.write_records("region", vec![((-3, 3), Some(proto))], true)
            .unwrap();
        assert!(
            reopened.lod_surfaces(-3, 3, &assets, &biomes).is_none(),
            "unfinished proto saves must not suppress complete decoration"
        );
        let nether = SaveDir::new(dir.clone(), LevelShape::NETHER, Some(Default::default()));
        let dark = nether.lod_surfaces(-2, 3, &assets, &biomes).unwrap();
        assert!(
            dark.iter()
                .flat_map(|c| &c.spans)
                .all(|s| s.light & 0xf0 == 0),
            "no invented skylight in Nether saves"
        );
        struct SavedOnly(Vec<crate::streaming::LodDetail>);
        impl crate::streaming::LodSampler for SavedOnly {
            fn sample(&mut self, _: i32, _: i32) -> Option<crate::columns::Surface> {
                panic!("saved data must win")
            }
            fn request_detail(&mut self, chunks: &[(i32, i32)], _: [i32; 2]) {
                assert!(
                    chunks.is_empty(),
                    "saved builds must not request speculative generation"
                );
            }
            fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
                std::mem::take(&mut self.0)
            }
        }
        let inner = Arc::new(crate::testgen::TestTerrain::new(assets.registry, &biomes).unwrap());
        let generator = Arc::new(Persistent {
            inner,
            save: Arc::new(reopened),
            registry: Arc::new(assets),
            biomes: Arc::new(biomes),
        });
        let mut sampler = SavedLodSampler {
            generator,
            inner: Box::new(SavedOnly(Vec::new())),
            cache: Default::default(),
            details: Vec::new(),
            approximate: false,
            generated: Arc::new(crate::lod::cache::DiskCache::new(dir.join("far"), 1)),
            bake: None,
        };
        use crate::streaming::LodSampler;
        sampler.request_detail(&[(-2, 3)], [-32, 48]);
        let details = sampler.drain_details();
        assert_eq!(details.len(), 1);
        assert_eq!(&details[0].1[..], &surfaces[..]);
        sampler.cache.clear();
        let generated: LodSurfaces =
            Arc::new(std::array::from_fn(|_| crate::lod::Column::default()));
        sampler.inner = Box::new(SavedOnly(vec![
            ((-2, 3), generated.clone()),
            ((-9, 9), generated.clone()),
        ]));
        let details = sampler.drain_details();
        assert_eq!(
            &details[0].1[..],
            &surfaces[..],
            "unrequested batch neighbours must preserve saved builds"
        );
        assert!(Arc::ptr_eq(&details[1].1, &generated));
        assert!(
            sampler.generated.get(-2, 3).is_none(),
            "saved builds are not cached as generated"
        );
        assert_eq!(sampler.generated.get(-9, 9).unwrap(), generated);
        // A later session reads the generated chunk back instead of
        // generating it again.
        sampler.request_detail(&[(-9, 9)], [-144, 144]);
        assert_eq!(sampler.drain_details()[0].1, generated);
        sampler.inner =
            crate::streaming::ChunkGenerator::lod_sampler(sampler.generator.inner.clone());
        let grid = sampler.grid(-32, 48, 2, 16);
        for z in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    grid[z * 16 + x],
                    crate::lod::Column::reduced(&surfaces, x * 2, z * 2, 2),
                    "saved builds must override a batched direct grid"
                );
            }
        }
        drop((save, sampler, nether));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saved_far_surface_rejects_invalid_packed_palettes() {
        let mut states = Tag::compound([
            ("palette", Tag::List(vec![Tag::Byte(0), Tag::Byte(1)])),
            ("data", Tag::LongArray(vec![0; 256])),
        ]);
        let mut indices = [9u16; 4096];
        assert_eq!(lod_palette_indices(&states, &mut indices, 4), Some(2));
        assert_eq!(indices, [0; 4096]);
        // Two palette entries take 4 bits a block, 16 blocks a long: block 17
        // is the second of the second long, block 4095 the last of the last.
        let mut data = vec![0i64; 256];
        data[1] = 0x10;
        data[255] = 1 << 60;
        put(&mut states, "data", Tag::LongArray(data));
        assert_eq!(lod_palette_indices(&states, &mut indices, 4), Some(2));
        let set: Vec<usize> = (0..4096).filter(|&i| indices[i] != 0).collect();
        assert_eq!(set, [17, 4095]);
        assert_eq!((indices[17], indices[4095]), (1, 1));
        put(&mut states, "data", Tag::LongArray(vec![0; 255]));
        assert_eq!(lod_palette_indices(&states, &mut indices, 4), None);
        put(&mut states, "palette", Tag::List(Vec::new()));
        assert_eq!(lod_palette_indices(&states, &mut indices, 4), None);
        // One entry needs no data; the biomes' 64 cells pack from 1 bit.
        let one = Tag::compound([("palette", Tag::List(vec![Tag::Byte(0)]))]);
        let mut cells = [9u16; 64];
        assert_eq!(lod_palette_indices(&one, &mut cells, 1), Some(1));
        assert_eq!(cells, [0; 64]);
        let three = Tag::compound([
            ("palette", Tag::List(vec![Tag::Byte(0); 3])),
            // 2 bits a cell, 32 cells a long: cell 1 is 2, cell 33 is 1.
            ("data", Tag::LongArray(vec![0b1000, 0b0100])),
        ]);
        assert_eq!(lod_palette_indices(&three, &mut cells, 1), Some(3));
        assert_eq!((cells[0], cells[1], cells[33], cells[63]), (0, 2, 1, 0));
    }

    #[test]
    #[ignore = "daily"]
    fn saved_far_surface_checks_packed_indices_against_their_palettes() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes");
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let forest = biomes.id("minecraft:forest");
        let desert = biomes.id("minecraft:desert");
        assert!(forest != 0 && desert != 0 && forest != desert);
        // One section, y 0..16. Blocks: air and stone, 4 bits a block.
        // Biomes: three of them, 2 bits a 4x4x4 cell, 32 cells a long.
        let chunk = |blocks: Vec<i64>, cells: i64| NamedTag {
            name: "".into(),
            tag: Tag::compound([(
                "sections",
                Tag::List(vec![Tag::compound([
                    ("Y", Tag::Byte(0)),
                    (
                        "block_states",
                        Tag::compound([
                            (
                                "palette",
                                Tag::List(vec![
                                    Tag::String("minecraft:air".into()),
                                    Tag::String("minecraft:stone".into()),
                                ]),
                            ),
                            ("data", Tag::LongArray(blocks)),
                        ]),
                    ),
                    (
                        "biomes",
                        Tag::compound([
                            (
                                "palette",
                                Tag::List(vec![
                                    Tag::String("minecraft:forest".into()),
                                    Tag::String("minecraft:desert".into()),
                                    Tag::String("minecraft:plains".into()),
                                ]),
                            ),
                            ("data", Tag::LongArray(vec![cells, 0])),
                        ]),
                    ),
                ])]),
            )]),
        };
        let decode = |root: &NamedTag| {
            decode_lod_surfaces(root, &assets, &biomes, LevelShape::OVERWORLD.column(), true)
        };
        // Stone in the bottom layer (the first 256 blocks, 16 a long).
        let mut blocks = vec![0i64; 256];
        blocks[..16].fill(0x1111_1111_1111_1111);
        // The first cell (x 0..4) is desert, the second (x 4..8) names a
        // fourth biome the palette does not have, the others are forest.
        let columns = decode(&chunk(blocks.clone(), 0b1101)).expect("a valid section");
        let surface = |x: usize| columns[x].surface.unwrap();
        assert_eq!((surface(0).height, surface(0).state), (1, stone));
        assert_eq!(surface(0).biome, desert);
        assert_eq!(surface(4).biome, 0, "an index past the palette is no biome");
        assert_eq!(surface(8).biome, forest);
        // The last block names a third state the palette does not have.
        blocks[255] = 2 << 60;
        assert!(
            decode(&chunk(blocks, 0b1101)).is_none(),
            "a block index past the palette makes no far columns"
        );
    }

    /// #6386: far terrain looked a section's palette and data up by name for
    /// each of its 4096 blocks, and its biomes again for each block it kept:
    /// milliseconds for one saved chunk, most of them over the air above the
    /// ground, on every far sampling thread.
    #[test]
    #[ignore = "daily"]
    fn saved_far_surface_reads_a_section_once_not_once_a_block() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes");
        let dandelion = assets
            .registry
            .default_state("minecraft:dandelion")
            .unwrap();
        // A flower in each section and air around it: nothing far terrain
        // keeps, so what is left to pay is reading the sections' blocks, as
        // loading the chunk for play does.
        let mut chunk = Chunk::empty(3, -5);
        for section in 0..chunk.sections.len() as i32 {
            chunk.set(8, MIN_Y + section * 16 + 8, 8, dandelion.0);
        }
        let saved = encode(&chunk, assets.registry, &biomes, &Default::default());
        let layout = LevelShape::OVERWORLD.column();
        let far = || decode_lod_surfaces(&saved, &assets, &biomes, layout, true);
        let columns = far().expect("far columns");
        assert!(
            columns
                .iter()
                .all(|column| column.surface.is_none() && column.spans.is_empty()),
            "a flower is no far terrain"
        );
        // The fastest of several runs of each, taken in turns, so that a busy
        // machine does not decide.
        let time = |run: &dyn Fn()| {
            let started = std::time::Instant::now();
            run();
            started.elapsed()
        };
        let (mut far_time, mut load_time) = (std::time::Duration::MAX, std::time::Duration::MAX);
        for _ in 0..25 {
            far_time = far_time.min(time(&|| {
                std::hint::black_box(far());
            }));
            load_time = load_time.min(time(&|| {
                std::hint::black_box(decode_with(&saved, assets.registry, &biomes, None, layout));
            }));
        }
        // Measured, optimized and not: 3 to 8 loads (the light layers come on
        // top of the blocks), and 100 to 300 with a lookup by name for each
        // block.
        assert!(
            far_time <= load_time * 32,
            "the far columns of a chunk took {far_time:?}, loading it {load_time:?}"
        );
    }

    #[test]
    fn save_batch_preserves_order_coordinates_and_ownership() {
        use crate::containers::BlockEntity;
        let mut entities = HashMap::new();
        let mut names = HashMap::new();
        for x in [i32::MIN, -17, -16, -1, 0, 15, 16, i32::MAX] {
            for z in [-17, -16, -1, 0, 15, 16] {
                let p = (x, 64, z);
                entities.insert(p, BlockEntity::Chest(vec![None; 27]));
                names.insert(p, string(&format!("container {x},{z}")));
            }
        }
        // Names without a modelled entity must also survive the snapshot.
        names.insert((-2, 80, -3), string("orphan name"));
        let mut source = Chunk::empty(-1, -1);
        source
            .block_entities
            .push(((99, 0, 99), BlockEntity::Chest(vec![])));
        source.block_names.push(((99, 0, 99), string("stale")));
        source.saved = Some(Arc::new(ChunkExtras::default()));
        let source = Arc::new(source);
        let batch = vec![
            source.clone(),
            Arc::new(Chunk::empty(1, 1)),
            Arc::new(Chunk::empty(-2, 0)),
            source.clone(), // Preserve input order and repeated coordinates.
            Arc::new(Chunk::empty(i32::MIN >> 4, -2)),
            Arc::new(Chunk::empty(i32::MAX >> 4, 1)),
            Arc::new(Chunk::empty(100, 100)),
        ];
        let encode = |entries: &[((i32, i32, i32), BlockEntity)]| {
            entries
                .iter()
                .map(|(p, e)| {
                    block_entity_encode::encode(*p, e, "minecraft:chest", &Default::default())
                })
                .collect::<Vec<_>>()
        };
        for input in [&batch[..0], &batch[..1], &batch[..]] {
            let actual = chunks_with_block_entities(input, &entities, &names);
            assert_eq!(actual.len(), input.len());
            for (c, original) in actual.iter().zip(input) {
                let expected: Vec<_> = entities
                    .iter()
                    .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                    .map(|(p, e)| (*p, e.clone()))
                    .collect();
                let expected_names: Vec<_> = names
                    .iter()
                    .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                    .map(|(p, n)| (*p, n.clone()))
                    .collect();
                assert_eq!((c.x, c.z), (original.x, original.z));
                assert_eq!(encode(&c.block_entities), encode(&expected));
                assert_eq!(c.block_names, expected_names);
                assert!(Arc::ptr_eq(&c.sections[0], &original.sections[0]));
                if let Some(saved) = &original.saved {
                    assert!(Arc::ptr_eq(c.saved.as_ref().unwrap(), saved));
                }
            }
        }
        let snapshot = chunks_with_block_entities(&batch, &entities, &names);
        entities.clear();
        names.clear();
        assert!(!snapshot[0].block_entities.is_empty());
        assert!(!snapshot[0].block_names.is_empty());
        assert_eq!(source.block_names[0].1, string("stale"));
        assert_eq!(source.block_entities[0].0, (99, 0, 99));
    }

    #[test]
    fn work_shared_out_is_all_done_in_order_when_no_helper_can_start() {
        use super::share_out;
        let squares = |i: usize| i * i;
        let want: Vec<usize> = (0..40).map(squares).collect();
        // Every helper refused (a container at its thread limit): this thread does it all.
        assert_eq!(share_out(40, 4, squares, || false), want);
        // Some refused, some started.
        let mut asked = 0;
        let some = || {
            asked += 1;
            asked % 2 == 0
        };
        assert_eq!(share_out(40, 4, squares, some), want);
        assert_eq!(share_out(40, 4, squares, || true), want);
        assert_eq!(share_out(0, 4, squares, || true), Vec::<usize>::new());
    }

    /// `RegionFile.getVersion` writes a chunk with the version
    /// `region-file-compression` selected: `compress_records` asks
    /// `Compression::selected()` for every record it encodes, so a server told
    /// to write lz4, gzip or uncompressed regions does
    /// (`RegionFileVersion.configure`).
    #[test]
    fn a_chunk_is_written_with_the_selected_region_version() {
        use super::{Record, compress_records};
        let tag = Arc::new(NamedTag {
            name: "".into(),
            tag: Tag::compound([("xPos", Tag::Int(3))]),
        });
        let records = [((3, 4), Record::Tag(Arc::clone(&tag)))];
        let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
        for name in ["none", "lz4", "gzip", "deflate"] {
            assert!(Compression::configure(name), "{name}");
            let written = compress_records(&refs, 99)
                .pop()
                .expect("one record")
                .expect("an encoded record")
                .expect("a chunk, not a deletion")
                .record;
            assert_eq!(
                written.compression,
                Compression::selected(),
                "written with {name}"
            );
            // The bytes are that codec's, and read back as the chunk.
            assert_eq!(written.nbt(Limits::default()).unwrap(), *tag);
        }
        // Leave the default behind for the rest of this process's tests.
        assert!(Compression::configure("deflate"));
    }

    #[test]
    fn records_compressed_on_helpers_match_one_thread_in_order() {
        use super::{Record, compress_records};
        let records: Vec<((i32, i32), Record)> = (0..53i32)
            .map(|i| {
                let record = if i % 7 == 3 {
                    Record::Delete
                } else {
                    let payload: Vec<i64> = (0..i * 97 % 2000).map(|v| i64::from(v * i)).collect();
                    Record::Tag(Arc::new(NamedTag {
                        name: "".into(),
                        tag: Tag::compound([
                            ("xPos", Tag::Int(i)),
                            ("Data", Tag::LongArray(payload)),
                        ]),
                    }))
                };
                ((i, -i), record)
            })
            .collect();
        let refs: Vec<_> = records.iter().map(|(k, r)| (*k, r)).collect();
        let compressed = compress_records(&refs, 1234);
        assert_eq!(compressed.len(), records.len());
        for ((_, record), got) in records.iter().zip(compressed) {
            let want = record
                .tag()
                .map(|t| {
                    ChunkRecord::from_nbt(&t, Compression::selected(), 1234, Limits::default())
                })
                .transpose()
                .unwrap();
            let got = got.unwrap().map(|r| r.record);
            assert_eq!(
                got.as_ref().map(|r| (&r.compressed, r.timestamp)),
                want.as_ref().map(|r| (&r.compressed, r.timestamp))
            );
        }
    }

    #[test]
    fn older_worlds_are_upgraded_on_opening() {
        let dir = std::env::temp_dir().join(format!("sunrise-dv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let level =
            |v: i32| Tag::compound([("Data", Tag::compound([("DataVersion", Tag::Int(v))]))]);
        let version = |dir: &std::path::Path| {
            super::read_dat(&dir.join("level.dat"))
                .and_then(|r| super::number(r.get("Data")?.get("DataVersion")))
        };
        // 1.12.2 and 1.16.5: upgraded too, every server fix is ported. Data from before 1.13
        // needs the data fixers' tables of the prepared resources: without them (a checkout
        // nothing was prepared in, CI's verify) only 1.16.5 is upgraded here.
        let olds: &[i32] = if sunrise_world::datafix::tables_available() {
            &[1343, 2586]
        } else {
            &[2586]
        };
        for &old in olds {
            super::write_dat(&dir.join("level.dat"), level(old)).unwrap();
            super::upgrade_older_world(&dir).unwrap();
            assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
        }
        // 1.21.1: upgraded to the current version.
        super::write_dat(&dir.join("level.dat"), level(3955)).unwrap();
        super::upgrade_older_world(&dir).unwrap();
        assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
        super::upgrade_older_world(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn region_files_kept_open_stay_under_vanillas_cache_size() {
        let open_fds = || std::fs::read_dir("/dev/fd").map_or(0, |d| d.count());
        let dir = std::env::temp_dir().join(format!("sunrise-save-fds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save =
            super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        let before = open_fds();
        let storage = save.region_storage(Folder::Region);
        let held = storage.open_cached((0, 0), true).unwrap().unwrap();
        for x in 1..=600 {
            storage.open_cached((x, 0), true).unwrap().unwrap();
        }
        // Other tests open files at the same time: a margin, far below 600.
        let grown = open_fds().saturating_sub(before);
        assert!(
            grown <= super::MAX_OPEN_REGIONS + 32,
            "{grown} descriptors more"
        );
        let cache = save.regions.lock().unwrap();
        assert!(
            cache.open() <= super::MAX_OPEN_REGIONS,
            "{} open",
            cache.open()
        );
        // The one still held is never closed under its holder.
        assert!(Arc::ptr_eq(
            &cache.regions[&(0, 0)].0.clone().unwrap(),
            &held
        ));
        drop(cache);
        // What was written to regions since closed reads back from the disk.
        let entity = |x: i32| Tag::compound([("x", Tag::Int(x))]);
        save.save_entities_later((0..300).map(|i| ((i * 32, 0), vec![entity(i)])).collect())
            .unwrap();
        save.flush().unwrap();
        assert!(save.entity_regions.lock().unwrap().open() <= super::MAX_OPEN_REGIONS);
        for i in [0, 1, 150, 299] {
            assert_eq!(save.load_entities(i * 32, 0).unwrap(), vec![entity(i)]);
        }
        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn write_completion_polls_without_waiting_and_keeps_latest_snapshot() {
        let dir = std::env::temp_dir().join(format!("sunrise-save-poll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save =
            super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        let lock = super::SessionLock::acquire(&dir).unwrap();
        let region = save
            .region_storage(Folder::Entities)
            .open_cached((0, 0), true)
            .unwrap()
            .unwrap();
        let chunk_region = save
            .region_storage(Folder::Region)
            .open_cached((0, 0), true)
            .unwrap()
            .unwrap();
        // Hold the actual region locks, rather than relying on disk speed or sleeps. The
        // writer takes the Region folder before Entities in a pass, so both are held.
        let blocked = (region.lock().unwrap(), chunk_region.lock().unwrap());
        let entity = |version| Tag::compound([("version", Tag::Int(version))]);
        save.save_entities_later(vec![((1, 2), vec![entity(1)])])
            .unwrap();
        save.save_entities_later(vec![((1, 2), vec![entity(2)])])
            .unwrap();
        assert!(!save.writes_finished().unwrap());
        assert_eq!(save.active_chunk_writes(), 0); // entity batches do not count
        save.write_records("region", vec![((0, 0), None), ((1, 0), None)], false)
            .unwrap();
        // The writer is blocked above: two chunk records in ONE queued batch.
        assert_eq!(save.active_chunk_writes(), 2);
        assert_eq!(save.load_entities(1, 2).unwrap(), vec![entity(2)]);
        assert!(super::SessionLock::acquire(&dir).is_err());
        drop(blocked);
        save.flush().unwrap();
        assert!(save.writes_finished().unwrap());
        assert_eq!(save.active_chunk_writes(), 0);
        let reopened =
            super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        assert_eq!(reopened.load_entities(1, 2).unwrap(), vec![entity(2)]);
        save.save_entities_later(vec![((1, 2), vec![])]).unwrap();
        save.flush().unwrap();
        assert!(
            super::SaveDir::new(dir.clone(), LevelShape::OVERWORLD, None)
                .load_entities(1, 2)
                .unwrap()
                .is_empty()
        );
        drop((reopened, save, lock));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_entity_read_does_not_clear_the_stored_record() {
        let dir = tempfile::tempdir().unwrap();
        let save = super::SaveDir::new(dir.path().to_owned(), LevelShape::OVERWORLD, None);
        let entity = Tag::compound([("id", Tag::String("minecraft:pig".into()))]);
        // Put matching records in two different regions.
        save.save_entities(vec![
            ((0, 0), vec![entity.clone()]),
            ((32, 0), vec![entity.clone()]),
        ])
        .unwrap();
        let region_b = save.region_path("entities", 1, 0);
        let saved = std::fs::read(&region_b).unwrap();
        let mut broken = saved.clone();
        let sector = u32::from_be_bytes(saved[..4].try_into().unwrap()) >> 8;
        // Fail decompression without changing the slot or its record length.
        broken[sector as usize * 4096 + 5] = 0;
        std::fs::write(&region_b, &broken).unwrap();
        assert_eq!(save.load_entities(0, 0).unwrap(), vec![entity.clone()]);
        assert!(save.load_entities(32, 0).is_err());
        assert!(save.load_entities(64, 0).unwrap().is_empty());
        std::fs::write(&region_b, &saved).unwrap();

        // Leaving and returning to the chunk does not restart the failed entity load.
        assert!(save.load_entities(32, 0).is_err());
        // The same save pass can clear a healthy chunk and must leave B alone.
        save.save_entities(vec![((0, 0), vec![]), ((32, 0), vec![])])
            .unwrap();
        drop(save);
        let reopened = super::SaveDir::new(dir.path().to_owned(), LevelShape::OVERWORLD, None);
        assert!(reopened.load_entities(0, 0).unwrap().is_empty());
        assert_eq!(reopened.load_entities(32, 0).unwrap(), vec![entity]);
    }

    #[test]
    fn a_save_pass_resumes_at_the_write_watermark_instead_of_flushing() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-save-watermark-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let save = std::sync::Arc::new(super::SaveDir::new(
            dir.clone(),
            super::LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        let lock = super::SessionLock::acquire(&dir).unwrap();
        // Hold the region lock so the writer stops there deterministically, rather
        // than relying on disk speed or sleeps: it keeps the lock while it writes.
        let region = save
            .region_storage(Folder::Region)
            .open_cached((0, 0), true)
            .unwrap()
            .unwrap();
        let blocked = region.lock().unwrap();
        let record = |x: i32| ((x, 0), None);
        save.write_records("region", (0..4).map(record).collect(), false)
            .unwrap();
        assert_eq!(save.active_chunk_writes(), 4);
        assert!(!save.writes_finished().unwrap());
        // The four queued records are at the watermark: a save pass resumes at
        // once and leaves them to the writer. On its own thread so that a pass
        // that waited like `flush` fails the test instead of hanging it.
        let (resumed, resume_result) =
            std::sync::mpsc::channel::<std::result::Result<(), String>>();
        let resuming = {
            let save = save.clone();
            std::thread::spawn(move || {
                resumed
                    .send(
                        save.wait_for_chunk_writes_below(4)
                            .map_err(|e| e.to_string()),
                    )
                    .unwrap()
            })
        };
        resume_result
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        resuming.join().unwrap();
        assert_eq!(save.active_chunk_writes(), 4);
        assert!(!save.writes_finished().unwrap());
        // One below it, on that same queue, the wait does hold the caller, as a
        // flush does: only releasing the lock below can wake either.
        let (waited, wait_result) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
        let waiting = {
            let save = save.clone();
            std::thread::spawn(move || {
                waited
                    .send(
                        save.wait_for_chunk_writes_below(3)
                            .map_err(|e| e.to_string()),
                    )
                    .unwrap()
            })
        };
        let (flushed, flush_result) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
        let flushing = {
            let save = save.clone();
            std::thread::spawn(move || {
                flushed
                    .send(save.flush().map_err(|e| e.to_string()))
                    .unwrap()
            })
        };
        let quiet = |rx: &std::sync::mpsc::Receiver<std::result::Result<(), String>>| {
            matches!(
                rx.recv_timeout(std::time::Duration::from_millis(200)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            )
        };
        assert!(quiet(&wait_result), "the wait returned below its watermark");
        assert!(quiet(&flush_result), "flush returned while blocked");
        drop(blocked);
        wait_result
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        flush_result
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        flushing.join().unwrap();
        waiting.join().unwrap();
        assert!(save.writes_finished().unwrap());
        assert_eq!(save.active_chunk_writes(), 0);
        drop((save, lock));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_async_io_failure_retires_the_pending_snapshot_without_retrying_it() {
        let dir = tempfile::tempdir().unwrap();
        // A file where a directory is needed causes a real writer I/O error.
        let blocked = dir.path().join("entities");
        std::fs::write(&blocked, b"blocked").unwrap();
        let save = super::SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let entity = Tag::compound([("version", Tag::Int(3))]);
        save.save_entities_later(vec![((0, 0), vec![entity])])
            .unwrap();
        save.flush().unwrap();
        assert!(save.writes_finished().unwrap());
        assert_eq!(save.record_save_failures(), 1);

        // Fix the directory without submitting another snapshot. A fresh read
        // does not see the unpersisted entity and a barrier does not retry it.
        std::fs::remove_file(&blocked).unwrap();
        save.flush().unwrap();
        assert!(save.load_entities(0, 0).unwrap().is_empty());
        assert!(!blocked.exists());
        assert_eq!(save.record_save_failures(), 1);
    }

    #[test]
    fn session_lock_is_exclusive() {
        let dir = std::env::temp_dir().join(format!("sunrise-lock-{}", std::process::id()));
        let first = super::SessionLock::acquire(&dir).expect("first lock");
        assert!(super::SessionLock::acquire(&dir).is_err());
        drop(first);
        // Windows refuses a read through another handle while the file is
        // locked, so the bytes are read once the lock is released.
        assert_eq!(
            std::fs::read(dir.join("session.lock")).unwrap(),
            [0x26, 0x03]
        );
        assert!(super::SessionLock::acquire(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// #749: a copy of the lock's descriptor (a child process spawned on
    /// another thread holds one until it execs) must not keep a released
    /// world locked.
    #[test]
    fn session_lock_releases_while_a_descriptor_copy_lives() {
        let dir = std::env::temp_dir().join(format!("sunrise-lock-copy-{}", std::process::id()));
        let lock = super::SessionLock::acquire(&dir).expect("first lock");
        let copy = lock.0.try_clone().unwrap();
        drop(lock);
        assert!(super::SessionLock::acquire(&dir).is_ok());
        drop(copy);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn crafter_block_entity_round_trips() {
        let mut crafter = crate::containers::Crafter::default();
        crafter.items[2] = Some(crate::items::ItemStack::new("minecraft:stick", 5));
        crafter.disabled[4] = true;
        crafter.triggered = 1;
        crafter.crafting_ticks = 3;
        let tag = block_entity_encode::encode(
            (7, 8, 9),
            &crate::containers::BlockEntity::Crafter(crafter),
            "minecraft:crafter",
            &Default::default(),
        );
        let (pos, back) = decode_block_entity(&tag).expect("crafter");
        assert_eq!(pos, (7, 8, 9));
        let crate::containers::BlockEntity::Crafter(back) = back else {
            panic!("not a crafter");
        };
        assert_eq!(back.items[2].as_ref().map(|s| s.count), Some(5));
        assert!(back.disabled[4] && !back.disabled[3]);
        assert!(back.is_triggered());
        assert_eq!(back.crafting_ticks, 3);
        assert_eq!(back.redstone_signal(), 2);
    }
    /// `triggered` is an int vanilla saves as it is and calls triggered only at 1, so other
    /// values come back unchanged and untriggered.
    #[test]
    fn crafter_triggered_keeps_the_whole_int() {
        for value in [0, 1, 5, -1, 65537] {
            let crafter = crate::containers::Crafter {
                triggered: value,
                ..Default::default()
            };
            let tag = block_entity_encode::encode(
                (0, 0, 0),
                &crate::containers::BlockEntity::Crafter(crafter),
                "minecraft:crafter",
                &Default::default(),
            );
            let (_, back) = decode_block_entity(&tag).expect("crafter");
            let crate::containers::BlockEntity::Crafter(back) = back else {
                panic!("not a crafter");
            };
            assert_eq!(back.triggered, value);
            assert_eq!(back.is_triggered(), value == 1, "triggered {value}");
        }
    }
    #[test]
    fn crafter_triggered_double_load_floors_and_saturates() {
        for (value, expected) in [(-0.5_f64, -1), (1.5, 1), (4294967297.0, i32::MAX)] {
            let tag = Tag::compound([
                ("id", string("minecraft:crafter")),
                ("x", Tag::Int(0)),
                ("y", Tag::Int(0)),
                ("z", Tag::Int(0)),
                ("triggered", Tag::Double(value.to_bits())),
            ]);
            let (_, back) = decode_block_entity(&tag).expect("crafter");
            let crate::containers::BlockEntity::Crafter(back) = back else {
                panic!("not a crafter");
            };
            assert_eq!(back.triggered, expected, "triggered {value}d");
            assert_eq!(back.is_triggered(), expected == 1, "triggered {value}d");
        }
    }
    #[test]
    fn campfire_block_entity_round_trips() {
        let mut c = crate::containers::Campfire::default();
        c.items[1] = Some(crate::items::ItemStack::new("minecraft:beef", 1));
        c.progress[1] = 120;
        c.total[1] = 600;
        let tag = block_entity_encode::encode(
            (3, 64, -2),
            &crate::containers::BlockEntity::Campfire(Box::new(c.clone())),
            "minecraft:soul_campfire",
            &Default::default(),
        );
        // One block entity type for both campfires.
        assert_eq!(text(tag.get("id")).as_deref(), Some("minecraft:campfire"));
        let (pos, back) = decode_block_entity(&tag).expect("campfire");
        assert_eq!(pos, (3, 64, -2));
        let crate::containers::BlockEntity::Campfire(back) = back else {
            panic!("not a campfire");
        };
        assert_eq!(*back, c);
        // The update tag a server's clients get: the items alone.
        let campfire = crate::containers::BlockEntity::Campfire(Box::new(c));
        let update = crate::lan::chunks::entity_tag(&campfire, false, &Default::default())
            .expect("update tag");
        assert!(update.get("Items").is_some() && update.get("CookingTimes").is_none());
    }
    /// `LecternBlockEntity.loadAdditional`: `Page` is clamped to
    /// `0..getPageCount(book) - 1`, so a stored page outside the book loads as
    /// the first or last page the block holds. A bookless lectern counts no
    /// pages and loads page -1.
    #[test]
    fn a_lecterns_page_loads_within_its_book() {
        let page_of = |tag: &str| {
            let tag = sunrise_core::nbt::parse_snbt(tag).unwrap().0;
            let (_, entity) = decode_block_entity(&tag).expect("lectern");
            let crate::containers::BlockEntity::Lectern(l) = entity else {
                panic!("not a lectern");
            };
            l.page
        };
        let book = |pages: &str| {
            format!(
                r#"id:"minecraft:written_book",count:1,components:{{
                    "minecraft:written_book_content":{{title:"t",author:"a",pages:[{pages}]}}
                }}"#
            )
        };
        // Two pages: a stored 7 is the last page, a stored -3 the first.
        assert_eq!(
            page_of(&format!(
                r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:7,Book:{{{}}}}}"#,
                book(r#""one","two""#)
            )),
            1
        );
        assert_eq!(
            page_of(&format!(
                r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:-3,Book:{{{}}}}}"#,
                book(r#""one","two""#)
            )),
            0
        );
        // No book counts no pages, so any page loads as -1, as vanilla's.
        assert_eq!(
            page_of(r#"{id:"minecraft:lectern",x:1,y:64,z:2,Page:4}"#),
            -1
        );
        // A page already inside the book is kept, and saved back as it is.
        let tag = format!(
            r#"{{id:"minecraft:lectern",x:1,y:64,z:2,Page:1,Book:{{{}}}}}"#,
            book(r#""one","two","three""#)
        );
        let tag = sunrise_core::nbt::parse_snbt(&tag).unwrap().0;
        let (pos, entity) = decode_block_entity(&tag).expect("lectern");
        let crate::containers::BlockEntity::Lectern(l) = entity else {
            panic!("not a lectern");
        };
        assert_eq!(l.page, 1);
        let saved = block_entity_encode::encode(
            pos,
            &crate::containers::BlockEntity::Lectern(l),
            "minecraft:lectern",
            &Default::default(),
        );
        assert_eq!(number(saved.get("Page")), Some(1));
    }
    #[test]
    fn a_chest_loot_table_takes_precedence_over_saved_items() {
        let mut saved = sunrise_core::nbt::parse_snbt(
            r#"{id:"minecraft:chest",x:1,y:2,z:3,Items:[{Slot:3b,id:"minecraft:diamond",count:7}],LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}"#,
        )
        .unwrap()
        .0;
        let (_, chest) = decode_block_entity(&saved).expect("chest");
        assert!(
            chest.slots().iter().all(Option::is_none),
            "ChestBlockEntity.loadAdditional ignores Items when a loot table loads"
        );
        // The command path keeps an empty inventory and the pending table/seed
        // for data get and saving. No loot resources are needed to keep that state.
        let pending = PendingContainerLoot::from_tag(&saved).unwrap();
        let stored = block_entity_encode::data(
            (1, 2, 3),
            &chest,
            "minecraft:chest",
            block_entity_encode::DataFields {
                pending: Some(&pending),
                ..Default::default()
            },
            &Default::default(),
        );
        assert_eq!(stored.get("LootTable"), saved.get("LootTable"));
        assert_eq!(stored.get("LootTableSeed"), saved.get("LootTableSeed"));
        assert!(stored.get("Items").is_none());
        let Tag::Compound(fields) = &mut saved else {
            panic!("compound");
        };
        fields.remove(&"LootTable".into());
        let (_, chest) = decode_block_entity(&saved).expect("chest without table");
        let item = chest.slots()[3].as_ref().expect("saved item");
        assert_eq!(item.item.as_ref(), "minecraft:diamond");
        assert_eq!(item.count, 7);
    }

    /// `tryLoadLootTable` in every `RandomizableContainer`'s `loadAdditional`
    /// (barrel, shulker boxes, hopper, dispenser, dropper):
    /// a valid table leaves the saved `Items` unread, as for the chest.
    #[test]
    fn every_randomizable_container_lets_its_loot_table_win_over_saved_items() {
        for (id, size) in [
            ("minecraft:barrel", 27),
            ("minecraft:shulker_box", 27),
            ("minecraft:white_shulker_box", 27),
            ("minecraft:hopper", crate::containers::HOPPER_SLOTS),
            ("minecraft:dispenser", 9),
            ("minecraft:dropper", 9),
        ] {
            let mut saved = sunrise_core::nbt::parse_snbt(&format!(
                r#"{{id:"{id}",x:1,y:2,z:3,Items:[{{Slot:3b,id:"minecraft:diamond",count:7}}],LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}}"#
            ))
            .unwrap()
            .0;
            let (_, container) = decode_block_entity(&saved).expect(id);
            assert_eq!(container.slots().len(), size, "{id}");
            assert!(
                container.slots().iter().all(Option::is_none),
                "{id}: loadAdditional ignores Items when a loot table loads"
            );
            let pending = PendingContainerLoot::from_tag(&saved).unwrap();
            let stored = block_entity_encode::data(
                (1, 2, 3),
                &container,
                id,
                block_entity_encode::DataFields {
                    pending: Some(&pending),
                    ..Default::default()
                },
                &Default::default(),
            );
            assert_eq!(stored.get("LootTable"), saved.get("LootTable"), "{id}");
            assert_eq!(
                stored.get("LootTableSeed"),
                saved.get("LootTableSeed"),
                "{id}"
            );
            assert!(stored.get("Items").is_none(), "{id}");
            let Tag::Compound(fields) = &mut saved else {
                panic!("compound");
            };
            fields.remove(&"LootTable".into());
            let (_, container) = decode_block_entity(&saved).expect(id);
            let item = container.slots()[3].as_ref().expect("saved item");
            assert_eq!(item.item.as_ref(), "minecraft:diamond", "{id}");
            assert_eq!(item.count, 7, "{id}");
        }
    }

    #[test]
    fn invalid_container_loot_tables_leave_saved_items_intact() {
        for id in [
            "minecraft:chest",
            "minecraft:trapped_chest",
            "minecraft:barrel",
            "minecraft:shulker_box",
            "minecraft:hopper",
            "minecraft:dispenser",
            "minecraft:dropper",
        ] {
            for invalid in ["INVALID TABLE", "bad:table/with spaces", "bad@id"] {
                let saved = sunrise_core::nbt::parse_snbt(&format!(
                    r#"{{id:"{id}",x:1,y:2,z:3,Items:[{{Slot:0b,id:"minecraft:diamond",count:3}}],LootTable:"{invalid}"}}"#
                ))
                .unwrap()
                .0;
                let (_, container) = decode_block_entity(&saved).expect(id);
                let item = container.slots()[0]
                    .as_ref()
                    .expect("saved item with invalid table");
                assert_eq!(item.item.as_ref(), "minecraft:diamond");
                assert_eq!(item.count, 3);
            }
        }
    }

    /// `LockCode`: a chest's `lock` is not in Sunrise's model, so `data get`
    /// takes it from the tag the block entity was loaded (or set) with.
    #[test]
    fn a_locked_chest_keeps_its_lock_through_decode_and_encode() {
        let saved = sunrise_core::nbt::parse_snbt(
            r#"{id:"minecraft:chest",x:1,y:2,z:3,lock:{items:"minecraft:gold_ingot"}}"#,
        )
        .unwrap()
        .0;
        let (pos, chest) = decode_block_entity(&saved).expect("chest");
        assert_eq!(chest.slots().len(), 27);
        assert_eq!(pos, (1, 2, 3));
        // The encoded entity has no field to carry the lock...
        let plain =
            block_entity_encode::encode(pos, &chest, "minecraft:chest", &Default::default());
        assert!(plain.get("lock").is_none());
        // ...so `data get` puts back the one it was loaded with.
        let data = block_entity_encode::data(
            pos,
            &chest,
            "minecraft:chest",
            block_entity_encode::DataFields {
                lock: saved.get("lock"),
                ..Default::default()
            },
            &Default::default(),
        );
        let lock = sunrise_core::nbt::parse_snbt(r#"{items:"minecraft:gold_ingot"}"#)
            .unwrap()
            .0;
        assert_eq!(data.get("lock"), Some(&lock));
    }
    #[test]
    fn spawner_block_entity_round_trips() {
        // A dungeon spawner as vanilla saves it (`id` is mob_spawner, not the block).
        let saved = sunrise_core::nbt::parse_snbt(
            r#"{id:"minecraft:mob_spawner",x:67,y:-52,z:-64,Delay:20s,SpawnData:{entity:{id:"minecraft:zombie"}},SpawnPotentials:[]}"#,
        )
        .unwrap()
        .0;
        let (pos, back) = decode_block_entity(&saved).expect("spawner");
        assert_eq!(pos, (67, -52, -64));
        let crate::containers::BlockEntity::Spawner(spawner) = &back else {
            panic!("not a spawner");
        };
        let next = spawner.next_data().and_then(|d| d.id());
        assert_eq!(next.as_deref(), Some("minecraft:zombie"));
        let tag = block_entity_encode::encode(pos, &back, "minecraft:spawner", &Default::default());
        let (_, again) = decode_block_entity(&tag).expect("spawner again");
        let crate::containers::BlockEntity::Spawner(again) = again else {
            panic!("not a spawner");
        };
        assert_eq!(again.update_tag(), spawner.update_tag());
    }
    #[test]
    #[ignore = "daily"]
    fn native_saved_ids_must_match_the_declared_block_membership() {
        // The production registry is immutable. Exercise its real readers in a fresh process.
        const CHILD: &str = "SUNRISE_TEST_NATIVE_SAVED_MEMBERSHIP";
        if std::env::var_os(CHILD).is_none() {
            for mode in ["accepted", "revoked"] {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--include-ignored",
                        "--exact",
                        "save::tests::native_saved_ids_must_match_the_declared_block_membership",
                        "--nocapture",
                    ])
                    .env(CHILD, mode)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "test=save::tests::native_saved_ids_must_match_the_declared_block_membership exit_code={:?} category=fixture-child-failed",
                    output.status.code()
                );
            }
            return;
        }
        use crate::block_entity_types::{self, BlockEntityTypes};
        let mut types = BlockEntityTypes::new(
            [
                ("test:press".into(), "test:machine".into()),
                ("test:mill".into(), "test:machine".into()),
                ("test:board".into(), "test:other".into()),
            ]
            .into(),
            vec![
                "test:machine".into(),
                "test:other".into(),
                "minecraft:chest".into(),
            ],
        );
        // Seed the same canonical pairs as production when resources are available.
        // The explicit shared-representation cases below also run without resources.
        let mut vanilla_blocks: Vec<String> = [
            "chest",
            "trapped_chest",
            "barrel",
            "dropper",
            "dispenser",
            "hopper",
            "shulker_box",
        ]
        .into_iter()
        .map(|name| format!("minecraft:{name}"))
        .collect();
        for color in [
            "white",
            "orange",
            "magenta",
            "light_blue",
            "yellow",
            "lime",
            "pink",
            "gray",
            "light_gray",
            "cyan",
            "purple",
            "blue",
            "brown",
            "green",
            "red",
            "black",
        ] {
            vanilla_blocks.push(format!("minecraft:{color}_shulker_box"));
        }
        if let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") {
            let path = sunrise_protocol::registry::Builtin::find(&PathBuf::from(root))
                .expect("registries.json");
            let builtin = sunrise_protocol::registry::Builtin::load(&path).unwrap();
            vanilla_blocks.extend(
                builtin
                    .get("minecraft:block")
                    .unwrap()
                    .names
                    .iter()
                    .cloned(),
            );
        }
        for block in vanilla_blocks {
            if let Some(kind) = crate::lan::chunks::block_entity_type(&block) {
                types.template(block.clone(), block, kind.into());
            }
        }
        types.template(
            "minecraft:trapped_chest".into(),
            "minecraft:trapped_chest".into(),
            "minecraft:trapped_chest".into(),
        );
        let accepted = std::env::var(CHILD).unwrap() == "accepted";
        if accepted {
            types
                .add_valid("minecraft:trapped_chest", "minecraft:chest")
                .unwrap();
        }
        block_entity_types::install(types).unwrap();
        let tag = |id: &str| {
            sunrise_core::nbt::parse_snbt(&format!(r#"{{id:"{id}",x:1,y:64,z:2,energy:900L}}"#,))
                .unwrap()
                .0
        };
        for (block, kind) in block_entity_types::current().unwrap().memberships() {
            if block.starts_with("minecraft:") {
                let saved = tag(kind);
                // Some vanilla types have no client representation yet. Membership
                // must not discard any entity that the decoder can represent.
                assert_eq!(
                    decode_block_entity_for_block(&saved, block).is_some(),
                    decode_block_entity(&saved).is_some(),
                    "{kind} on {block}"
                );
            }
        }
        assert_eq!(
            decode_block_entity_for_block(&tag("minecraft:chest"), "minecraft:trapped_chest")
                .is_some(),
            accepted
        );
        assert!(
            decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:stone").is_none()
        );
        assert!(
            decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:trapped_chest")
                .is_none()
        );
        for block in [
            "trapped_chest",
            "barrel",
            "dropper",
            "dispenser",
            "hopper",
            "shulker_box",
            "red_shulker_box",
        ] {
            let block = format!("minecraft:{block}");
            let kind = crate::lan::chunks::block_entity_type(&block).unwrap();
            assert!(
                decode_block_entity_for_block(&tag(kind), &block).is_some(),
                "{kind} on {block}"
            );
        }
        for block in ["test:press", "test:mill"] {
            let (_, entity) = decode_block_entity_for_block(&tag("test:machine"), block).unwrap();
            assert!(entity.fits(block));
            assert_eq!(entity.id(), "test:machine");
            let crate::containers::BlockEntity::Mod(m) = entity else {
                panic!("not native");
            };
            assert_eq!(m.nbt.get(&"energy".into()), Some(&Tag::Long(900)));
            for id in [
                "test:press",
                "test:other",
                "test:unknown",
                "minecraft:banner",
            ] {
                assert!(
                    decode_block_entity_for_block(&tag(id), block).is_none(),
                    "{id}"
                );
            }
        }
        for block in [
            "test:casing",
            "test:board",
            "other:press",
            "minecraft:stone",
        ] {
            assert!(decode_block_entity_for_block(&tag("test:machine"), block).is_none());
        }
        assert!(
            decode_block_entity_for_block(&tag("minecraft:banner"), "minecraft:white_banner")
                .is_some()
        );
    }

    #[test]
    fn queued_client_update_fields_are_not_saved_or_taken_by_a_save() {
        let saved =
            sunrise_core::nbt::parse_snbt(r#"{id:"test:machine",x:5,y:-60,z:12,progress:7}"#)
                .unwrap()
                .0;
        let (pos, entity) = decode_block_entity(&saved).unwrap();
        let crate::containers::BlockEntity::Mod(machine) = &entity else {
            panic!("not a mod's");
        };
        let batch = Compound::from([("effect".into(), Tag::Int(23))]);
        machine.set_client_update_fields(batch.clone());
        assert_eq!(
            block_entity_encode::encode(pos, &entity, "test:machine", &Default::default()),
            saved
        );
        assert_eq!(machine.take_client_update_fields(), Some(batch));
    }

    #[test]
    fn published_shapes_are_saved_with_the_block_entity_and_come_back_on_load() {
        use crate::containers::{BlockEntity, PublishedShapes, SHAPES_KEY};
        let saved = sunrise_core::nbt::parse_snbt(r#"{id:"test:bus",x:5,y:-60,z:12,progress:7}"#)
            .unwrap()
            .0;
        let (pos, entity) = decode_block_entity(&saved).unwrap();
        let BlockEntity::Mod(mut bus) = entity else {
            panic!("not a mod's");
        };
        assert!(bus.shapes().is_none());
        let shapes = PublishedShapes::new(mod_api::BlockEntityShapes {
            outline: vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]],
            collision: vec![[-0.25, 0.0, 0.0, 1.0, 1.5, 1.0]],
        })
        .unwrap();
        bus.set_shapes(Some(std::sync::Arc::new(shapes)));
        let saved = block_entity_encode::encode(
            pos,
            &BlockEntity::Mod(bus),
            "test:bus",
            &Default::default(),
        );
        // Kept apart from the mod's own fields.
        let (_, back) = decode_block_entity(&saved).unwrap();
        let BlockEntity::Mod(back) = back else {
            panic!("not a mod's");
        };
        assert_eq!(back.nbt.len(), 1);
        assert!(!back.nbt.contains_key(&SHAPES_KEY.into()));
        let loaded = back.shapes().expect("restored on load");
        assert_eq!(loaded.boxes().collision, [[-0.25, 0.0, 0.0, 1.0, 1.5, 1.0]]);
        // Saved data that is over a limit loads without them.
        let Tag::Compound(mut bad) = saved else {
            unreachable!()
        };
        bad.insert(
            SHAPES_KEY.into(),
            Tag::Compound(Compound::from([
                ("outline".into(), Tag::LongArray(vec![0; 6 * 65])),
                ("collision".into(), Tag::LongArray(vec![])),
            ])),
        );
        let (_, bad) = decode_block_entity(&Tag::Compound(bad)).unwrap();
        let BlockEntity::Mod(bad) = bad else {
            panic!("not a mod's");
        };
        assert!(bad.shapes().is_none());
    }

    #[test]
    fn mod_block_entity_items_claim_only_the_slots_written() {
        use crate::{
            containers::{BlockEntity, ModBlockEntity},
            items::ItemStack,
        };
        let parse = |s| sunrise_core::nbt::parse_snbt(s).unwrap().0;
        let saved = parse(
            r#"{id:"test:machine",x:0,y:64,z:0,Items:[{Slot:0b,raw:"superseded"},{Slot:1b,raw:"retained"},{raw:"no slot"}]}"#,
        );
        let (pos, BlockEntity::Mod(mut data)) = decode_block_entity(&saved).unwrap() else {
            panic!("expected mod block entity")
        };
        let encode = |data: &ModBlockEntity| {
            block_entity_encode::encode(
                pos,
                &BlockEntity::Mod(Box::new(data.clone())),
                "test:machine",
                &Default::default(),
            )
        };
        data.items.resize(2, None);
        *data.item_slot_mut(0).unwrap() = Some(ItemStack::new("minecraft:stone", 2));
        let encoded = encode(&data);
        assert_eq!(
            encoded.get("Items"),
            Some(&parse(
                r#"[{Slot:0b,id:"minecraft:stone",count:2},{Slot:1b,raw:"retained"},{raw:"no slot"}]"#
            ))
        );
        let (_, BlockEntity::Mod(reloaded)) = decode_block_entity(&encoded).unwrap() else {
            panic!("expected mod block entity")
        };
        assert_eq!(reloaded.items[0], data.items[0]);
        assert_eq!(encode(&reloaded), encoded);
        *data.item_slot_mut(0).unwrap() = None;
        // A tick reply replacing the mutable fields, including shrinking the
        // modeled vector to zero, cannot restore the old saved row for slot 0.
        let mut snapshot = (**data).clone();
        snapshot.items.clear();
        let revision = data.revision().changes() + 1;
        data.apply_snapshot(snapshot, revision);
        assert_eq!(
            encode(&data).get("Items"),
            Some(&parse(r#"[{Slot:1b,raw:"retained"},{raw:"no slot"}]"#))
        );
        data.items.resize(2, None);
        *data.item_slot_mut(1).unwrap() = None;
        assert_eq!(
            encode(&data).get("Items"),
            Some(&parse(r#"[{raw:"no slot"}]"#))
        );
        let (_, BlockEntity::Mod(mixed)) = decode_block_entity(&parse(
            r#"{id:"test:machine",x:0,y:64,z:0,Items:[{Slot:0b,raw:"opaque"},{Slot:1b,id:"minecraft:stone",count:2}]}"#,
        ))
        .unwrap() else {
            panic!("expected mod block entity")
        };
        assert_eq!(
            mixed.items,
            [None, Some(ItemStack::new("minecraft:stone", 2))]
        );
        data.nbt.insert("Items".into(), parse("{energy:5}"));
        assert_eq!(encode(&data).get("Items"), Some(&parse("{energy:5}")));
        data.nbt.remove(&"Items".into());
        assert_eq!(encode(&data).get("Items"), None);
    }

    #[test]
    fn mod_block_entity_round_trips() {
        let saved = sunrise_core::nbt::parse_snbt(
            r#"{id:"gtmod:macerator",x:5,y:-60,z:12,energy:1200L,progress:7,Items:[{Slot:0b,id:"minecraft:iron_ore",count:3}]}"#,
        )
        .unwrap()
        .0;
        let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
        assert_eq!(pos, (5, -60, 12));
        let crate::containers::BlockEntity::Mod(m) = &back else {
            panic!("not a mod's");
        };
        assert_eq!(m.id(), "gtmod:macerator");
        // The mod sees its own fields only.
        for key in ["id", "x", "y", "z"] {
            assert!(!m.nbt.contains_key(&key.into()), "{key}");
        }
        assert_eq!(m.nbt.get(&"energy".into()), Some(&Tag::Long(1200)));
        // `Items` are its slots, not the mod's compound.
        assert!(!m.nbt.contains_key(&"Items".into()));
        assert_eq!(m.items.len(), 1);
        assert_eq!(m.items[0].as_ref().map(|s| s.count), Some(3));
        assert_eq!(
            block_entity_encode::encode(pos, &back, "gtmod:macerator", &Default::default()),
            saved
        );
        // `Items` that are not an item list stay the mod's, as saved.
        for items in [
            r#"Items:{energy:5}"#,
            r#"Items:[1,2,3]"#,
            r#"Items:[{Slot:0b,id:"minecraft:air",count:1}]"#,
            r#"Items:[{Slot:0b,id:"minecraft:stone",count:1,note:"mine"}]"#,
        ] {
            let saved = sunrise_core::nbt::parse_snbt(&format!(
                r#"{{id:"gtmod:cable",x:0,y:0,z:0,{items}}}"#
            ))
            .unwrap()
            .0;
            let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
            let crate::containers::BlockEntity::Mod(m) = &back else {
                panic!("not a mod's");
            };
            assert!(m.items.is_empty(), "{items}");
            assert_eq!(
                block_entity_encode::encode(pos, &back, "gtmod:cable", &Default::default()),
                saved,
                "{items}"
            );
        }
        // Entries load by their own `Slot`, in any list order (`ContainerHelper.loadAllItems`).
        let saved = sunrise_core::nbt::parse_snbt(
            r#"{id:"gtmod:macerator",x:5,y:-60,z:12,Items:[{Slot:2b,id:"minecraft:stone",count:2},{Slot:0b,id:"minecraft:iron_ore",count:3}]}"#,
        )
        .unwrap()
        .0;
        let (pos, back) = decode_block_entity(&saved).expect("mod block entity");
        let crate::containers::BlockEntity::Mod(m) = &back else {
            panic!("not a mod's");
        };
        assert!(!m.nbt.contains_key(&"Items".into()));
        assert_eq!(m.items.len(), 3);
        assert_eq!(
            m.items[0].as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:iron_ore", 3))
        );
        assert!(m.items[1].is_none());
        assert_eq!(
            m.items[2].as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:stone", 2))
        );
        // Saved again, they are written in slot order.
        let reordered = sunrise_core::nbt::parse_snbt(
            r#"{id:"gtmod:macerator",x:5,y:-60,z:12,Items:[{Slot:0b,id:"minecraft:iron_ore",count:3},{Slot:2b,id:"minecraft:stone",count:2}]}"#,
        )
        .unwrap()
        .0;
        assert_eq!(
            block_entity_encode::encode(pos, &back, "gtmod:macerator", &Default::default()),
            reordered
        );
        // A lossless slot is modeled even beside an opaque row. Duplicate
        // slot indices stay opaque because the engine cannot model both stacks.
        let duplicate = r#"Items:[{Slot:1b,id:"minecraft:stone",count:1},{Slot:1b,id:"minecraft:dirt",count:1}]"#;
        let modeled = [Some(crate::items::ItemStack::new("minecraft:stone", 1))];
        let parse = |items| {
            sunrise_core::nbt::parse_snbt(&format!(r#"{{id:"gtmod:cable",x:0,y:0,z:0,{items}}}"#))
                .unwrap()
                .0
        };
        for (input, output, slots) in [
            (
                r#"Items:[{Slot:1b,id:"minecraft:stone",count:1,note:"mine"},{Slot:0b,id:"minecraft:stone",count:1}]"#,
                r#"Items:[{Slot:0b,id:"minecraft:stone",count:1},{Slot:1b,id:"minecraft:stone",count:1,note:"mine"}]"#,
                modeled.as_slice(),
            ),
            (duplicate, duplicate, &[]),
        ] {
            let (pos, back) = decode_block_entity(&parse(input)).expect("mod block entity");
            let crate::containers::BlockEntity::Mod(m) = &back else {
                panic!("not a mod's");
            };
            assert_eq!(m.items.as_slice(), slots, "{input}");
            assert_eq!(
                block_entity_encode::encode(pos, &back, "gtmod:cable", &Default::default()),
                parse(output),
                "{input}"
            );
        }
        // Vanilla's types are never a mod's, modelled or not.
        let ender = sunrise_core::nbt::parse_snbt(r#"{id:"minecraft:ender_chest",x:0,y:0,z:0}"#)
            .unwrap()
            .0;
        assert!(!matches!(
            decode_block_entity(&ender),
            Some((_, crate::containers::BlockEntity::Mod(_)))
        ));
    }
    /// `CopperGolemStatueBlockEntity.createStatue`: the golem's styled custom
    /// name lives in the statue's components, through every weathering stage,
    /// a save and its item.
    #[test]
    fn copper_golem_statue_keeps_the_golem_name() {
        use crate::containers::BlockEntity;
        let name = Tag::compound([
            ("text", string("Bob")),
            ("color", string("gold")),
            ("bold", Tag::Byte(1)),
        ]);
        let components: Compound = [("minecraft:custom_name".into(), name.clone())].into();
        for block in [
            "minecraft:copper_golem_statue",
            "minecraft:exposed_copper_golem_statue",
            "minecraft:weathered_copper_golem_statue",
            "minecraft:oxidized_copper_golem_statue",
            "minecraft:waxed_oxidized_copper_golem_statue",
        ] {
            // Scraping and waxing swap the block, so every stage keeps the entity.
            let entity = BlockEntity::for_block(block).expect(block);
            assert!(entity.fits(block), "{block}");
            let BlockEntity::CopperGolemStatue(_) = entity else {
                panic!("{block}: not a statue");
            };
        }
        let entity = BlockEntity::CopperGolemStatue(components.clone());
        let tag = block_entity_encode::encode(
            (1, 2, 3),
            &entity,
            "minecraft:oxidized_copper_golem_statue",
            &Default::default(),
        );
        assert_eq!(
            tag.get("id"),
            Some(&string("minecraft:copper_golem_statue"))
        );
        let (pos, back) = decode_block_entity(&tag).expect("statue");
        assert_eq!(pos, (1, 2, 3));
        let BlockEntity::CopperGolemStatue(back) = back else {
            panic!("not a statue");
        };
        assert_eq!(back, components);
        // The statue item carries the name (`getItem`: collectComponents).
        let saved = crate::block_entity_nbt::canonical(&tag).expect("canonical statue");
        let patch = crate::block_entity_item::pick(&saved);
        assert_eq!(patch.get(&"minecraft:custom_name".into()), Some(&name));
        // Placing that item gives the entity the name back.
        let fresh = crate::block_entity_nbt::canonical(&block_entity_encode::encode(
            (1, 2, 3),
            &BlockEntity::CopperGolemStatue(Compound::new()),
            "minecraft:copper_golem_statue",
            &Default::default(),
        ))
        .expect("canonical statue");
        let placed = crate::block_entity_item::place(&fresh, &patch, false);
        let (_, placed) = decode_block_entity(&placed).expect("placed statue");
        assert!(matches!(placed, BlockEntity::CopperGolemStatue(c) if c == components));
    }
    #[test]
    fn banner_block_entity_round_trips() {
        let banner = crate::containers::Banner {
            layers: vec![crate::banner::BannerLayer::new("minecraft:creeper", "lime")],
            name: Some("Flag".into()),
            components: [("minecraft:rarity".into(), string("uncommon"))].into(),
        };
        let tag = block_entity_encode::encode(
            (1, 2, 3),
            &crate::containers::BlockEntity::Banner(banner.clone()),
            "minecraft:red_banner",
            &Default::default(),
        );
        let (pos, back) = decode_block_entity(&tag).expect("banner");
        assert_eq!(pos, (1, 2, 3));
        let crate::containers::BlockEntity::Banner(back) = back else {
            panic!("not a banner");
        };
        assert_eq!(back.layers, banner.layers);
        assert_eq!(back.name, banner.name);
        assert_eq!(back.components, banner.components);
    }
    use super::*;
    use crate::chunk::{MIN_Y, SECTIONS};

    /// `TheEndGatewayBlockEntity.loadAdditional` reads `ExactTeleport` with
    /// `getBooleanOr`: a number is true when its byte value is not 0, so an
    /// int 256 is false and 257 true.
    #[test]
    fn an_end_gateways_exact_teleport_is_read_as_a_byte() {
        let exact = |saved: Tag| {
            decode_end_gateway(&Tag::compound([("ExactTeleport", saved)])).exact_teleport
        };
        assert!(exact(Tag::Byte(1)));
        assert!(!exact(Tag::Byte(0)));
        assert!(!exact(Tag::Int(256)));
        assert!(exact(Tag::Int(257)));
        assert!(!decode_end_gateway(&Tag::compound([("Age", Tag::Long(1))])).exact_teleport);
    }

    #[test]
    fn a_mods_fluid_ticks_are_saved_by_name() {
        use sunrise_world::fluid::{FluidKind, ModFluidRules, ModFluidType, register_mod_fluid};
        let fluid = register_mod_fluid(ModFluidType {
            id: "savetest:oil".into(),
            flowing_id: "savetest:flowing_oil".into(),
            bucket: None,
            rules: ModFluidRules {
                tick_delay: 5,
                drop_off: 1,
                slope_find_distance: 4,
                converts_to_source: false,
            },
        });
        let fluid = fluid.unwrap();
        let tick = |kind: FluidKind| SavedTick {
            kind: sunrise_core::ticks::TickTypeId(kind.tick_id()),
            pos: BlockPos::new(1, 64, 2),
            delay: 3,
            priority: sunrise_core::ticks::TickPriority::Normal,
        };
        let ticks = [
            tick(FluidKind::ModFlowing(fluid)),
            tick(FluidKind::Mod(fluid)),
            tick(FluidKind::FlowingWater),
            tick(FluidKind::Lava),
        ];
        let tag = tick_list(&ticks, fluid_name);
        let names: Vec<_> = match &tag {
            Tag::List(list) => list.iter().filter_map(|t| text(t.get("i"))).collect(),
            _ => Vec::new(),
        };
        assert_eq!(
            names,
            [
                "savetest:flowing_oil",
                "savetest:oil",
                "minecraft:flowing_water",
                "minecraft:lava"
            ]
        );
        let back = read_ticks(Some(&tag), (0, 0), fluid_id);
        assert_eq!(
            back.iter().map(|t| t.kind).collect::<Vec<_>>(),
            ticks.iter().map(|t| t.kind).collect::<Vec<_>>()
        );
    }

    #[test]
    fn only_server_disk_records_intern_saved_fluid_names() {
        // The game initializes the platform at startup; the unresolved-fluid table lives there,
        // and without it nothing is interned. Alone, nothing else in this process has done it.
        mod_platform::__private::initialize().unwrap();
        let root =
            std::env::temp_dir().join(format!("sunrise-fluid-source-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut save = SaveDir::new(
            root.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        save.enable_trusted_fluid_load();
        let record = |id: &str| NamedTag {
            name: "".into(),
            tag: Tag::compound([(
                "block_entities",
                Tag::List(vec![Tag::compound([(
                    "fluids",
                    Tag::List(vec![Tag::compound([("Fluid", Tag::String(id.into()))])]),
                )])]),
            )]),
        };
        let disk = "fixture:trusted_saved_fluid";
        let live = "fixture:untrusted_pending_fluid";
        fs::create_dir_all(root.join("region")).unwrap();
        let mut region = RegionFile::open_writable(save.region_path("region", 0, 0), true).unwrap();
        region
            .write(
                ChunkPos::new(0, 0),
                &ChunkRecord::from_nbt(&record(disk), Compression::Zlib, 0, Limits::saved())
                    .unwrap(),
            )
            .unwrap();
        drop(region);
        save.read_record_raw("region", 0, 0).unwrap();
        assert!(mod_api::__private::lookup_unresolved_fluid(disk).is_some());
        let oversized = format!("fixture:{}", "x".repeat(257));
        let original = record(&oversized);
        intern_saved_fluids(&original);
        assert!(mod_api::__private::lookup_unresolved_fluid(&oversized).is_none());
        assert_eq!(original, record(&oversized));
        save.pending.lock().unwrap().insert(
            (Folder::Region, 1, 0),
            (0, Record::Tag(Arc::new(record(live)))),
        );
        save.read_record_raw("region", 1, 0).unwrap();
        assert!(mod_api::__private::lookup_unresolved_fluid(live).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn end_gateway_block_entity_round_trips() {
        use crate::containers::{BlockEntity, EndGateway};
        let plain = BlockEntity::for_block("minecraft:end_gateway").expect("gateway");
        assert!(matches!(&plain, BlockEntity::EndGateway(g) if *g == EndGateway::default()));
        for exit in [None, Some([8, 100, 0])] {
            for exact in [false, true] {
                let gateway = EndGateway {
                    age: 201,
                    exit_portal: exit,
                    exact_teleport: exact,
                };
                let tag = block_entity_encode::encode(
                    (702, 100, 0),
                    &BlockEntity::EndGateway(gateway.clone()),
                    "minecraft:end_gateway",
                    &Default::default(),
                );
                assert_eq!(
                    text(tag.get("id")).as_deref(),
                    Some("minecraft:end_gateway")
                );
                assert_eq!(tag.get("Age"), Some(&Tag::Long(201)));
                assert_eq!(
                    tag.get("exit_portal"),
                    exit.map(|p| Tag::IntArray(p.to_vec())).as_ref()
                );
                assert_eq!(
                    tag.get("ExactTeleport"),
                    exact.then_some(Tag::Byte(1)).as_ref()
                );
                let (pos, back) = decode_block_entity(&tag).expect("gateway");
                assert_eq!(pos, (702, 100, 0));
                assert!(matches!(back, BlockEntity::EndGateway(g) if g == gateway));
            }
        }
    }

    #[test]
    fn end_gateway_exit_requires_spawnable_bounds() {
        for (exit, valid) in [
            (vec![-30_000_000, -20_000_000, -30_000_000], true),
            (vec![29_999_999, 19_999_999, 29_999_999], true),
            (vec![30_000_000, 0, 0], false),
            (vec![0, 0, 30_000_000], false),
            (vec![-30_000_001, 0, 0], false),
            (vec![0, 0, -30_000_001], false),
            (vec![0, 20_000_000, 0], false),
            (vec![0, -20_000_001, 0], false),
            (vec![i32::MIN, 0, 0], false),
            (vec![0, 100], false),
        ] {
            let tag = Tag::compound([
                ("id", string("minecraft:end_gateway")),
                ("x", Tag::Int(702)),
                ("y", Tag::Int(100)),
                ("z", Tag::Int(0)),
                ("exit_portal", Tag::IntArray(exit)),
            ]);
            let (_, crate::containers::BlockEntity::EndGateway(g)) =
                decode_block_entity(&tag).unwrap()
            else {
                panic!("gateway");
            };
            assert_eq!(g.exit_portal.is_some(), valid);
            let canonical = crate::block_entity_nbt::canonical(&tag).expect("gateway NBT");
            assert_eq!(canonical.get("exit_portal").is_some(), valid);
        }
        assert_eq!(
            decode_end_gateway(&Tag::Compound(Compound::new())),
            crate::containers::EndGateway::default()
        );
    }

    #[test]
    fn skull_block_entity_keeps_its_profile() {
        // A placed `player_head` as vanilla saves it: the owning player's
        // `profile` survives load, encode and `canonical`.
        let saved = sunrise_core::nbt::parse_snbt(
            r#"{id:"minecraft:skull",x:10,y:64,z:20,profile:{name:"Steve",id:[I;1,2,3,4],properties:[{name:"textures",value:"abc"}]},note_block_sound:"minecraft:entity.player.hurt",custom_name:"Trophy"}"#,
        )
        .unwrap()
        .0;
        let (pos, back) = decode_block_entity(&saved).expect("skull");
        assert_eq!(pos, (10, 64, 20));
        let crate::containers::BlockEntity::Skull(skull) = &back else {
            panic!("not a skull");
        };
        assert_eq!(skull.profile, saved.get("profile").cloned());
        assert_eq!(
            skull.note_block_sound.as_deref(),
            Some("minecraft:entity.player.hurt")
        );
        assert_eq!(back.id(), "minecraft:skull");
        let tag =
            block_entity_encode::encode(pos, &back, "minecraft:player_head", &Default::default());
        assert_eq!(
            crate::block_entity_nbt::id(&tag).as_deref(),
            Some("minecraft:skull")
        );
        assert_eq!(tag.get("profile"), saved.get("profile"));
        let (_, again) = decode_block_entity(&tag).expect("skull again");
        let crate::containers::BlockEntity::Skull(again) = again else {
            panic!("not a skull");
        };
        assert_eq!(again.profile, skull.profile);
        // A profile-less head (skeleton, creeper) round-trips too.
        let plain = block_entity_encode::encode(
            pos,
            &crate::containers::BlockEntity::Skull(crate::containers::Skull::default()),
            "minecraft:creeper_head",
            &Default::default(),
        );
        let (_, decoded) = decode_block_entity(&plain).expect("plain skull");
        assert!(matches!(decoded, crate::containers::BlockEntity::Skull(_)));
        // The heads and skulls have one; the piston's head is no skull.
        use crate::containers::BlockEntity;
        assert!(matches!(
            BlockEntity::for_block("minecraft:player_wall_head"),
            Some(BlockEntity::Skull(_))
        ));
        assert!(BlockEntity::for_block("minecraft:piston_head").is_none());
    }

    #[test]
    fn uuid_text_round_trips() {
        let u = [-158327916, 1572358581, -2125825222, -487948715];
        assert_eq!(uuid_from_text(&uuid_text(u)), Some(u));
        assert_eq!(
            uuid_text([0, 1, 2, 3]),
            "00000000-0000-0001-0000-000200000003"
        );
        let r = random_uuid();
        assert_eq!((r[1] >> 12) & 0xF, 4, "version 4");
    }
    #[test]
    fn random_uuids_made_at_once_differ() {
        let made: std::collections::HashSet<[i32; 4]> =
            (0..10_000).map(|_| random_uuid()).collect();
        assert_eq!(made.len(), 10_000);
    }
    #[test]
    fn creaking_heart_keeps_its_protector_uuid() {
        use crate::containers::{BlockEntity, CreakingHeart};
        let heart = CreakingHeart {
            creaking: Some((Some(7), [1, 2, 3, 4])),
            ..Default::default()
        };
        let tag = block_entity_encode::encode(
            (3, 70, -5),
            &BlockEntity::CreakingHeart(Box::new(heart)),
            "minecraft:creaking_heart",
            &Default::default(),
        );
        assert_eq!(
            tag.get("creaking"),
            Some(&Tag::IntArray(vec![1, 2, 3, 4])),
            "vanilla stores the protector's UUID under `creaking`"
        );
        let (pos, entity) = decode_block_entity(&tag).expect("heart decodes");
        assert_eq!(pos, (3, 70, -5));
        let BlockEntity::CreakingHeart(h) = entity else {
            panic!("a creaking heart");
        };
        // The entity id is resolved again once the creaking is loaded.
        assert_eq!(h.creaking, Some((None, [1, 2, 3, 4])));
        let lonely = block_entity_encode::encode(
            (0, 0, 0),
            &BlockEntity::CreakingHeart(Box::default()),
            "minecraft:creaking_heart",
            &Default::default(),
        );
        assert!(lonely.get("creaking").is_none());
    }
    #[test]
    fn light_nibbles_use_vanilla_order() {
        let mut a = Box::new([0u8; 4096]);
        a[0] = 0xF3; // sky 15, block 3 at x=0
        a[1] = 0x52; // sky 5, block 2 at x=1
        let light = Light::Full(a);
        assert_eq!(nibbles(&light, false)[0], 0x23);
        assert_eq!(nibbles(&light, true)[0], 0x5F);
    }
    /// The independent reference: parse the saved layers and let
    /// `crate::light::light_from_layers` expand and repack them.
    fn reference(tag: &Tag, chunk: &mut Chunk, sky_light: bool) {
        if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
            return;
        }
        let Some(Tag::List(sections)) = tag.get("sections") else {
            return;
        };
        let mut layers: Vec<(Option<&[u8]>, Option<&[u8]>)> = vec![(None, None); SECTIONS + 2];
        for section in sections {
            let Some(y) = number(section.get("Y")) else {
                continue;
            };
            let si = y as i32 - (MIN_Y >> 4) + 1;
            if !(0..SECTIONS as i32 + 2).contains(&si) {
                continue;
            }
            let layer = |name: &str| match section.get(name) {
                Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
                _ => None,
            };
            layers[si as usize] = (layer("SkyLight"), layer("BlockLight"));
        }
        crate::light::light_from_layers(chunk, &layers, sky_light);
    }

    #[test]
    fn stored_light_loads_with_vanilla_defaults() {
        let mut sky = vec![0u8; 2048];
        sky[0] = 0x5A; // x=0 -> 10, x=1 -> 5
        let section = |y: i8, entries: Vec<(&str, Tag)>| {
            let mut all = vec![("Y", Tag::Byte(y))];
            all.extend(entries);
            Tag::compound(all)
        };
        let tag = Tag::compound([
            ("isLightOn", Tag::Byte(1)),
            (
                "sections",
                Tag::List(vec![
                    section(
                        -4,
                        vec![
                            ("SkyLight", Tag::ByteArray(sky)),
                            ("BlockLight", Tag::ByteArray(vec![0x21; 2048])),
                        ],
                    ),
                    // No sky layer: repeats the bottom layer of the section above.
                    section(-3, vec![]),
                ]),
            ),
        ]);
        let mut chunk = Chunk::empty(0, 0);
        load_light(&tag, &mut chunk, true);
        assert!(chunk.lit && chunk.light_loaded);
        let at = |c: &Chunk, si: usize, i: usize| c.sections[si].light(i);
        assert_eq!(at(&chunk, 0, 0), 0xA1, "stored sky 10, block 1");
        assert_eq!(at(&chunk, 0, 1), 0x52, "stored sky 5, block 2");
        // Section -3 has no layers; above it (-2..) sky defaults to 15.
        assert_eq!(at(&chunk, 1, 0), 0xF0);
        assert_eq!(at(&chunk, SECTIONS - 1, 5), 0xF0);
        let mut dark = Chunk::empty(0, 0);
        load_light(&tag, &mut dark, false);
        assert_eq!(at(&dark, 1, 0), 0x00, "no skylight dimension");
        let mut unlit = Chunk::empty(0, 0);
        load_light(
            &Tag::compound([("isLightOn", Tag::Byte(0))]),
            &mut unlit,
            true,
        );
        assert!(!unlit.lit && !unlit.light_loaded);
    }
    #[test]
    #[ignore = "daily"]
    fn both_light_pads_survive_full_chunk_save_and_reload() {
        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let assets = crate::assets::Assets::load(&root).unwrap();
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root,
        ))
        .unwrap();
        let mut original = Chunk::empty(0, 0);
        let mut layers = vec![(None, None); SECTIONS + 2];
        layers[0] = (None, Some(vec![0x55; 2048]));
        layers[SECTIONS + 1] = (Some(vec![0x27; 2048]), Some(vec![0; 2048]));
        crate::light::light_from_layers(&mut original, &layers, true);
        let saved = encode(&original, assets.registry, &biomes, &Default::default());
        let Some(Tag::List(sections)) = saved.tag.get("sections") else {
            panic!("missing saved sections");
        };
        assert_eq!(sections.len(), SECTIONS + 2);
        for section in [sections.first().unwrap(), sections.last().unwrap()] {
            assert!(section.get("block_states").is_none());
            assert!(section.get("biomes").is_none());
        }
        let mut loaded = decode(&saved, assets.registry, &biomes).unwrap();
        load_light(&saved.tag, &mut loaded, true);
        assert_eq!(loaded.light(0, MIN_Y - 1, 0), 0x75);
        assert_eq!(loaded.light(1, MIN_Y - 1, 0), 0x25);
        assert_eq!(loaded.light(0, 70, 0), 0x70);
        assert_eq!(loaded.light(1, 70, 0), 0x20);
        assert_eq!(loaded.light(0, loaded.max_y(), 0), 0x70);
        assert_eq!(loaded.light(1, loaded.max_y(), 0), 0x20);
        assert_eq!(loaded.stored_light_layers(0), Some([false, true]));
        assert_eq!(loaded.stored_light_layers(SECTIONS + 1), Some([true, true]));
    }

    #[test]
    #[ignore = "daily"]
    fn saved_light_matches_reference_for_layers_and_metadata() {
        let section = |y: Tag, sky: Option<Tag>, block: Option<Tag>| {
            let mut entries = vec![("Y", y)];
            if let Some(v) = sky {
                entries.push(("SkyLight", v));
            }
            if let Some(v) = block {
                entries.push(("BlockLight", v));
            }
            Tag::compound(entries)
        };
        let root = |sections| {
            Tag::compound([
                ("isLightOn", Tag::Byte(1)),
                ("sections", Tag::List(sections)),
            ])
        };
        let check = |tag: &Tag| {
            for sky_light in [false, true] {
                let mut expected = Chunk::empty(-3, 5);
                let s = Arc::make_mut(&mut expected.sections[0]);
                s.light = Light::Full(Box::new([0xab; 4096]));
                s.blocks.set(0, 1);
                s.blocks.set(0, 0);
                s.biomes.set(0, 1);
                s.biomes.set(0, 0);
                let mut actual = expected.clone();
                reference(tag, &mut expected, sky_light);
                load_light(tag, &mut actual, sky_light);
                assert_eq!(
                    (actual.lit, actual.light_loaded),
                    (expected.lit, expected.light_loaded)
                );
                for (a, b) in actual.sections.iter().zip(&expected.sections) {
                    assert_eq!(
                        std::mem::discriminant(&a.light),
                        std::mem::discriminant(&b.light)
                    );
                    assert_eq!(
                        format!("{:?}", a.blocks.to_wire()),
                        format!("{:?}", b.blocks.to_wire())
                    );
                    assert_eq!(
                        format!("{:?}", a.biomes.to_wire()),
                        format!("{:?}", b.biomes.to_wire())
                    );
                    for i in 0..4096 {
                        assert_eq!(a.light(i), b.light(i), "index {i}, sky {sky_light}");
                    }
                }
            }
        };
        for tag in [
            Tag::compound(Vec::<(&str, Tag)>::new()),
            Tag::compound([("isLightOn", Tag::Byte(0))]),
            Tag::compound([("isLightOn", Tag::Byte(2))]),
            Tag::compound([("isLightOn", Tag::Int(1)), ("sections", Tag::List(vec![]))]),
            Tag::compound([("isLightOn", Tag::Byte(1)), ("sections", Tag::Int(0))]),
            root(vec![]),
        ] {
            check(&tag);
        }
        // Cover every repeated byte, including 0xF0/0x0F: a uniform byte
        // array need not hold uniform nibbles.
        for byte in 0..=255u8 {
            check(&root(vec![section(
                Tag::Byte(19),
                Some(Tag::ByteArray(vec![byte; 2048])),
                Some(Tag::ByteArray(vec![255 - byte; 2048])),
            )]));
        }
        // Every pair of uniform sky/block nibble values, with missing
        // sections below inheriting the stored bottom row.
        for sky in 0..16u8 {
            for block in 0..16u8 {
                check(&root(vec![section(
                    Tag::Byte(1),
                    Some(Tag::ByteArray(vec![sky * 17; 2048])),
                    Some(Tag::ByteArray(vec![block * 17; 2048])),
                )]));
            }
        }
        for length in [0, 1, 2047, 2048, 2049] {
            for invalid in [
                Tag::Int(1),
                Tag::List(vec![]),
                Tag::ByteArray(vec![0xf0; length]),
            ] {
                check(&root(vec![section(
                    Tag::Byte(-4),
                    Some(invalid.clone()),
                    Some(invalid),
                )]));
            }
        }
        let varied: Vec<u8> = (0..2048)
            .map(|i| (i as u8).wrapping_mul(73).wrapping_add(31))
            .collect();
        check(&root(vec![
            section(Tag::Byte(-5), Some(Tag::ByteArray(varied.clone())), None),
            section(Tag::Byte(20), Some(Tag::ByteArray(varied.clone())), None),
            section(
                Tag::String("19".into()),
                Some(Tag::ByteArray(varied.clone())),
                None,
            ),
            section(Tag::Byte(19), Some(Tag::ByteArray(varied.clone())), None),
            section(Tag::Byte(17), None, Some(Tag::ByteArray(varied.clone()))),
            section(Tag::Byte(-4), Some(Tag::ByteArray(varied.clone())), None),
            // Last duplicate wins, even when its arrays have wrong types.
            section(
                Tag::Byte(-4),
                Some(Tag::Int(15)),
                Some(Tag::ByteArray(vec![7; 2047])),
            ),
            section(Tag::Byte(0), None, Some(Tag::ByteArray(varied))),
        ]));
    }
    /// A saved sky layer is the resident `Light::Sky` layer byte for byte,
    /// so a section whose block light is uniformly zero can adopt it.
    #[test]
    fn a_varying_sky_layer_is_adopted_without_expansion() {
        let saved: Vec<u8> = (0..2048).map(|i| (i as u8).wrapping_mul(29)).collect();
        let tag = Tag::compound([
            ("isLightOn", Tag::Byte(1)),
            (
                "sections",
                Tag::List(vec![Tag::compound([
                    ("Y", Tag::Byte(3)),
                    ("SkyLight", Tag::ByteArray(saved.clone())),
                ])]),
            ),
        ]);
        let mut chunk = Chunk::empty(0, 0);
        let mut expected = chunk.clone();
        load_light(&tag, &mut chunk, true);
        reference(&tag, &mut expected, true);
        for i in 0..4096 {
            assert_eq!(chunk.sections[7].light(i), expected.sections[7].light(i));
        }
        let Light::Sky(layer, 0) = &chunk.sections[7].light else {
            panic!("varying sky with no block light is Light::Sky(_, 0)");
        };
        assert_eq!(
            &layer[..],
            &saved[..],
            "resident layer equals the saved one"
        );
    }
    /// #2571: the difficulty is a property of the world: the level data
    /// holds it, so a world that was set to hard is hard when it is opened
    /// again, and a world that was downloaded keeps the difficulty it was
    /// made with.
    #[test]
    fn the_level_data_holds_the_difficulty() {
        let dir = std::env::temp_dir().join(format!("sunrise-difficulty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut level = Level {
            name: "Test".into(),
            ..Level::default()
        };
        // The world's difficulty is the level data's.
        level.set_difficulty(3);
        assert_eq!(level.difficulty, "hard");
        assert_eq!(level.difficulty_id(), 3);
        // It survives the save, so the world comes back with it.
        level.write(&dir, &Default::default()).unwrap();
        let back = Level::read(&dir).unwrap();
        assert_eq!(back.difficulty, "hard");
        assert_eq!(back.difficulty_id(), 3);
        // A world whose difficulty is not one of ours is normal.
        let other = Level {
            difficulty: "extreme".into(),
            ..Level::default()
        };
        assert_eq!(other.difficulty_id(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Dedicated world saves must not overwrite separately managed player data.
    #[test]
    fn dedicated_world_save_preserves_player_file() {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-dedicated-player-save-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);

        let mut level = Level {
            name: "Test".into(),
            position: Some([12.5, 70.0, -8.25]),
            inventory: vec![(0, "minecraft:diamond".into(), 7, 0)],
            ..Level::default()
        };

        // Create a real player file using the existing singleplayer writer.
        level.write(&dir, &Default::default()).unwrap();
        let uuid = level.player_uuid.unwrap();
        let player_file = dir
            .join("players")
            .join("data")
            .join(format!("{}.dat", uuid_text(uuid)));

        let before = fs::read(&player_file).unwrap();

        // Simulate an incomplete world-level player state.
        level.position = None;
        level.inventory.clear();
        level.day_time = 12345;

        // Dedicated saves must update metadata without touching player data.
        level.write_without_player(&dir).unwrap();

        let after = fs::read(&player_file).unwrap();
        assert_eq!(before, after, "dedicated save overwrote player data");

        // The world metadata must still be persisted.
        let reopened = Level::read(&dir).unwrap();
        assert_eq!(reopened.day_time, 12345);

        let _ = fs::remove_dir_all(&dir);
    }

    /// A crash save that excludes the single-player owner writes `level.dat` without a new
    /// player file and keeps a legacy `Data.Player`, the owner's only copy before a normal save.
    #[test]
    fn keeping_the_owner_writes_no_player_file_and_keeps_legacy_data_player() {
        let dir = std::env::temp_dir().join(format!("sunrise-keep-owner-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut level = Level {
            name: "Test".into(),
            ..Level::default()
        };
        level.write_without_player(&dir).unwrap();
        let legacy = Tag::compound([("Health", Tag::Float(7.0_f32.to_bits()))]);
        let mut root = read_dat(&dir.join("level.dat")).unwrap();
        let Tag::Compound(root_map) = &mut root else {
            panic!("level.dat is a compound");
        };
        let Some(Tag::Compound(data)) = root_map.get_mut(&JavaString::from("Data")) else {
            panic!("level.dat has Data");
        };
        data.insert(JavaString::from("Player"), legacy.clone());
        write_dat(&dir.join("level.dat"), root).unwrap();

        level.position = Some([1.0, 2.0, 3.0]);
        level.day_time = 777;
        level.write_keeping_player(&dir).unwrap();

        let root = read_dat(&dir.join("level.dat")).unwrap();
        assert_eq!(
            root.get("Data").and_then(|d| d.get("Player")),
            Some(&legacy)
        );
        assert!(!dir.join("players").join("data").exists());
        assert_eq!(Level::read(&dir).unwrap().day_time, 777);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A keep-owner write leaves `singleplayer_uuid` as it was on disk, absent included:
    /// naming the offline `Player` would make a reopen load another account's file.
    #[test]
    fn keeping_the_owner_keeps_the_on_disk_singleplayer_uuid() {
        let dir = std::env::temp_dir().join(format!("sunrise-keep-uuid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut level = Level {
            name: "Test".into(),
            ..Level::default()
        };
        let set_uuid = |uuid: Option<Tag>| {
            let mut root = read_dat(&dir.join("level.dat")).unwrap();
            let Tag::Compound(root_map) = &mut root else {
                panic!("level.dat is a compound");
            };
            let Some(Tag::Compound(data)) = root_map.get_mut(&JavaString::from("Data")) else {
                panic!("level.dat has Data");
            };
            match uuid {
                Some(tag) => data.insert(JavaString::from("singleplayer_uuid"), tag),
                None => data.remove(&JavaString::from("singleplayer_uuid")),
            };
            write_dat(&dir.join("level.dat"), root).unwrap();
        };
        let on_disk = || {
            read_dat(&dir.join("level.dat"))
                .unwrap()
                .get("Data")
                .and_then(|d| d.get("singleplayer_uuid"))
                .cloned()
        };
        level.write_without_player(&dir).unwrap();

        set_uuid(None);
        level.write_keeping_player(&dir).unwrap();
        assert_eq!(on_disk(), None, "a level.dat without a uuid keeps none");

        let owner = Tag::IntArray(vec![1, 2, 3, 4]);
        set_uuid(Some(owner.clone()));
        level.write_keeping_player(&dir).unwrap();
        assert_eq!(on_disk(), Some(owner), "a uuid stays the same");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_ends_clock_keeps_its_own_time() {
        let dir = std::env::temp_dir().join(format!("sunrise-end-clock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut level = Level {
            name: "Test".into(),
            time: 5000,
            day_time: 9000,
            ..Level::default()
        };
        // A world without one starts the End's clock at its game time.
        level.write(&dir, &Default::default()).unwrap();
        assert_eq!(Level::read(&dir).unwrap().end_clock, Some(5000));
        // Then it keeps its own count, apart from the game time and the day.
        level.time = 8000;
        level.end_clock = Some(321);
        level.write(&dir, &Default::default()).unwrap();
        let back = Level::read(&dir).unwrap();
        assert_eq!(
            (back.time, back.day_time, back.end_clock),
            (8000, 9000, Some(321))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn level_round_trips_through_the_vanilla_files() {
        let dir = std::env::temp_dir().join(format!("sunrise-level-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut level = Level {
            name: "Test".into(),
            seed: -42,
            time: 1234,
            day_time: 25000,
            creative: true,
            position: Some([1.5, 70.0, -3.25]),
            rotation: [90.0, 10.0],
            health: 17.0,
            absorption: 5.0,
            food: 15,
            saturation: 2.5,
            inventory: vec![
                (0, "minecraft:stone".into(), 12, 0),
                (5, "minecraft:iron_sword".into(), 1, 3),
                (103, "minecraft:iron_helmet".into(), 1, 0),
            ],
            xp_level: 3,
            xp_progress: 0.5,
            xp_total: 30,
            dimension: "minecraft:the_nether".into(),
            world_spawn: Some([8, 64, 8]),
            world_spawn_angle: [45.0, -10.0],
            // The world spawn's level is the world's, not the player's.
            world_spawn_dimension: Some("minecraft:the_end".into()),
            spawn: Some([20.0, 70.0, 20.0]),
            difficulty_locked: true,
            flying: true,
            ..Level::default()
        };
        level.weather.raining = true;
        level.weather.rain_time = 500;
        level
            .game_rules
            .insert("minecraft:keep_inventory".into(), 1);
        level.write(&dir, &Default::default()).unwrap();
        let saved = read_dat(&dir.join("level.dat")).unwrap();
        let spawn = saved.get("Data").and_then(|d| d.get("spawn"));
        assert_eq!(
            text(spawn.and_then(|s| s.get("dimension"))).as_deref(),
            Some("minecraft:the_end"),
            "level.dat's spawn.dimension"
        );
        let back = Level::read(&dir).unwrap();
        assert_eq!(back.name, "Test");
        assert_eq!(back.seed, -42);
        assert_eq!((back.time, back.day_time), (1234, 25000));
        assert!(back.creative);
        assert!(back.flying);
        assert_eq!(back.position, Some([1.5, 70.0, -3.25]));
        assert_eq!(back.rotation, [90.0, 10.0]);
        assert_eq!((back.health, back.food, back.saturation), (17.0, 15, 2.5));
        assert_eq!(back.absorption, 5.0);
        let mut inv = back.inventory.clone();
        inv.sort();
        assert_eq!(
            inv,
            vec![
                (0, "minecraft:stone".into(), 12, 0),
                (5, "minecraft:iron_sword".into(), 1, 3),
                (103, "minecraft:iron_helmet".into(), 1, 0)
            ]
        );
        assert_eq!(
            (back.xp_level, back.xp_progress, back.xp_total),
            (3, 0.5, 30)
        );
        assert_eq!(back.dimension, "minecraft:the_nether");
        assert_eq!(back.world_spawn, Some([8, 64, 8]));
        assert_eq!(back.world_spawn_angle, [45.0, -10.0]);
        assert_eq!(
            back.world_spawn_dimension.as_deref(),
            Some("minecraft:the_end")
        );
        assert_eq!(back.spawn, Some([8.0, 64.0, 8.0]));
        assert_eq!(
            back.respawn_elsewhere,
            Some(("minecraft:overworld".into(), [20, 70, 20], 0.0, 0.0, true))
        );
        assert!(back.difficulty_locked);
        assert!(back.weather.raining);
        assert_eq!(back.weather.rain_time, 500);
        assert_eq!(back.game_rule("keep_inventory"), 1);
        assert_eq!(back.game_rule("random_tick_speed"), 3);
        // Unknown player data survives another save.
        let mut extra = back.player_extra.clone().unwrap();
        put(
            &mut extra,
            "ShoulderEntityLeft",
            Tag::compound([("id", string("minecraft:parrot"))]),
        );
        let mut ender = vec![None; 27];
        ender[3] = Some(crate::items::ItemStack::new("minecraft:diamond", 2));
        let mut again = Level {
            player_extra: Some(extra),
            ender_items: ender,
            // The live amount, not the one the loaded compound held (#1307).
            absorption: 2.0,
            ..back
        };
        again.write(&dir, &Default::default()).unwrap();
        let third = Level::read(&dir).unwrap();
        assert_eq!(third.absorption, 2.0);
        assert!(matches!(
            third
                .player_extra
                .as_ref()
                .and_then(|p| p.get("ShoulderEntityLeft")),
            Some(Tag::Compound(_))
        ));
        // EnderItems round-trip by slot.
        assert_eq!(
            third.ender_items[3].as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:diamond", 2))
        );
        // A dead player's empty inventory replaces the previously loaded items.
        let mut dead = third;
        dead.health = 0.0;
        dead.inventory.clear();
        dead.inventory_components.clear();
        dead.write(&dir, &Default::default()).unwrap();
        let dead = Level::read(&dir).unwrap();
        assert_eq!(dead.health, 0.0);
        assert!(dead.inventory.is_empty());
        assert!(dead.inventory_components.is_empty());
        assert!(dead.player_extra.unwrap().get("equipment").is_none());
        assert!(dir.join("data/minecraft/world_gen_settings.dat").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn eff(
        id: &str,
        duration: i32,
        amplifier: i32,
        ambient: bool,
        particles: bool,
        icon: bool,
    ) -> crate::effects::SavedEffect {
        crate::effects::SavedEffect {
            ambient,
            particles,
            icon,
            ..crate::effects::SavedEffect::new(id, duration, amplifier)
        }
    }

    #[test]
    fn a_player_keeps_the_effects_hidden_under_a_stronger_one() {
        // MobEffectInstance.Details.CODEC nests `hidden_effect` (no id of its
        // own): Poison II for 6 s over Poison I for 30 s over Poison I
        // forever, as `/effect give` leaves them.
        let tag = Tag::compound([(
            "active_effects",
            Tag::List(vec![Tag::compound([
                ("id", string("minecraft:poison")),
                ("duration", Tag::Int(120)),
                ("amplifier", Tag::Byte(1)),
                ("show_icon", Tag::Byte(1)),
                (
                    "hidden_effect",
                    Tag::compound([
                        ("duration", Tag::Int(600)),
                        ("ambient", Tag::Byte(1)),
                        ("show_icon", Tag::Byte(1)),
                        (
                            "hidden_effect",
                            Tag::compound([
                                ("duration", Tag::Int(-1)),
                                ("show_particles", Tag::Byte(0)),
                                ("show_icon", Tag::Byte(0)),
                            ]),
                        ),
                    ]),
                ),
            ])]),
        )]);
        let mut level = Level::default();
        level.read_player(&tag);
        let expected = crate::effects::SavedEffect {
            hidden: Some(Box::new(crate::effects::SavedEffect {
                hidden: Some(Box::new(eff(
                    "minecraft:poison",
                    -1,
                    0,
                    false,
                    false,
                    false,
                ))),
                ..eff("minecraft:poison", 600, 0, true, true, true)
            })),
            ..eff("minecraft:poison", 120, 1, false, true, true)
        };
        assert_eq!(level.active_effects, [expected]);
        // Written back, the chain is the same compound.
        assert_eq!(
            level.player_tag(&Default::default()).get("active_effects"),
            tag.get("active_effects")
        );
    }

    #[test]
    fn a_saved_effect_without_show_icon_takes_show_particles() {
        // MobEffectInstance.Details.CODEC: `show_icon` defaults to
        // `show_particles`; an explicit one is kept apart from it.
        let effect = |extra: Vec<(&'static str, Tag)>| {
            let mut e = vec![
                ("id", string("minecraft:darkness")),
                ("duration", Tag::Int(260)),
            ];
            e.extend(extra);
            Tag::compound(e)
        };
        let mut level = Level::default();
        level.read_player(&Tag::compound([(
            "active_effects",
            Tag::List(vec![
                effect(vec![("show_particles", Tag::Byte(0))]),
                effect(vec![]),
                effect(vec![("show_icon", Tag::Byte(0))]),
            ]),
        )]));
        let flags: Vec<_> = level
            .active_effects
            .iter()
            .map(|e| (e.particles, e.icon))
            .collect();
        assert_eq!(flags, [(false, false), (true, true), (true, false)]);
    }

    #[test]
    fn effects_that_ended_are_not_saved_again() {
        // A player loaded with an effect that has since run out: the loaded
        // compound still holds it, but the save writes only what is active.
        let mut level = Level {
            active_effects: vec![eff("minecraft:speed", 200, 1, false, true, true)],
            ..Level::default()
        };
        let loaded = level.player_tag(&Default::default());
        assert!(loaded.get("active_effects").is_some());
        level.player_extra = Some(loaded);
        level.active_effects.clear();
        assert!(
            level
                .player_tag(&Default::default())
                .get("active_effects")
                .is_none()
        );
    }

    #[test]
    fn effect_amplifier_default_is_omitted_from_saved_players() {
        let effects = vec![
            eff("minecraft:luck", -1, 0, false, false, false),
            eff("minecraft:speed", 200, 1, true, false, true),
            eff("minecraft:regeneration", 40, 128, false, true, false),
            eff("minecraft:strength", -1, 255, false, true, true),
        ];
        for raw in [
            None,
            Some(Tag::compound([
                ("H100Keep", Tag::Int(7)),
                (
                    "active_effects",
                    Tag::List(vec![Tag::compound([
                        ("id", string("minecraft:luck")),
                        ("amplifier", Tag::Byte(42)),
                    ])]),
                ),
            ])),
        ] {
            let level = Level {
                active_effects: effects.clone(),
                player_extra: raw.clone(),
                ..Level::default()
            };
            let saved = level.player_tag(&Default::default());
            let Some(Tag::List(entries)) = saved.get("active_effects") else {
                panic!("modeled live effects must be saved even without raw input")
            };
            assert_eq!(entries.len(), 4);
            assert_eq!(
                entries
                    .iter()
                    .map(|e| e.get("amplifier").cloned())
                    .collect::<Vec<_>>(),
                vec![
                    None,
                    Some(Tag::Byte(1)),
                    Some(Tag::Byte(-128)),
                    Some(Tag::Byte(-1))
                ],
            );
            for (entry, e) in entries.iter().zip(&effects) {
                assert_eq!(entry.get("id"), Some(&string(&e.id)));
                assert_eq!(entry.get("duration"), Some(&Tag::Int(e.duration)));
                // MobEffectInstance.Details.MAP_CODEC leaves out ambient false
                // and show_particles true.
                let ambient_tag = e.ambient.then_some(Tag::Byte(1));
                assert_eq!(entry.get("ambient"), ambient_tag.as_ref());
                let particles_tag = (!e.particles).then_some(Tag::Byte(0));
                assert_eq!(entry.get("show_particles"), particles_tag.as_ref());
                assert_eq!(entry.get("show_icon"), Some(&Tag::Byte(i8::from(e.icon))));
            }
            assert_eq!(
                level.active_effects, effects,
                "saving must not mutate live state"
            );
            assert_eq!(
                saved.get("H100Keep"),
                raw.as_ref().and_then(|t| t.get("H100Keep"))
            );
        }
    }

    #[test]
    fn effect_amplifier_255_reloads_as_an_unsigned_byte() {
        // `MobEffectInstance.MAX_AMPLIFIER` is 255. `Details.MAP_CODEC` writes
        // it with `ExtraCodecs.UNSIGNED_BYTE`: `255.byteValue()` is -1 and
        // `128.byteValue()` is -128; `UnsignedBytes.toInt` restores both.
        let effects = vec![
            eff("minecraft:strength", -1, 255, false, true, true),
            eff("minecraft:regeneration", -1, 128, false, true, true),
            eff("minecraft:speed", 40, 1, false, true, true),
        ];
        let level = Level {
            active_effects: effects.clone(),
            ..Level::default()
        };
        let tag = level.player_tag(&Default::default());
        let Tag::List(saved) = tag.get("active_effects").expect("active_effects") else {
            panic!("active_effects list");
        };
        assert_eq!(
            saved
                .iter()
                .map(|e| e.get("amplifier").cloned())
                .collect::<Vec<_>>(),
            vec![
                Some(Tag::Byte(-1)),
                Some(Tag::Byte(-128)),
                Some(Tag::Byte(1)),
            ]
        );
        let mut loaded = Level::default();
        loaded.read_player(&tag);
        assert_eq!(loaded.active_effects, effects);
    }

    #[test]
    fn forced_overworld_spawnpoint_reloads_as_a_configuration() {
        // A personal point coinciding with the world spawn must still keep
        // its forced flag and yaw, rather than turning into a world fallback.
        for pos in [[5, 64, 5], [0, 80, 0]] {
            let point = ("minecraft:overworld".to_owned(), pos, 37.0, 0.0, true);
            let level = Level {
                respawn_elsewhere: Some(point.clone()),
                ..Level::default()
            };
            let mut loaded = Level {
                spawn: Some([0.0, 80.0, 0.0]),
                world_spawn: Some([0, 80, 0]),
                ..Level::default()
            };
            loaded.read_player(&level.player_tag(&Default::default()));
            assert_eq!(loaded.respawn_elsewhere, Some(point));
            assert_eq!(loaded.spawn, Some([0.0, 80.0, 0.0]));
            assert_eq!(
                loaded.player_tag(&Default::default()).get("respawn"),
                level.player_tag(&Default::default()).get("respawn")
            );
        }
    }

    #[test]
    fn creative_flight_status_persists_in_save_and_resets_for_survival() {
        let mut level = Level {
            creative: true,
            flying: true,
            ..Level::default()
        };
        let tag = level.player_tag(&Default::default());
        let flying_byte = tag
            .get("abilities")
            .and_then(|a| number(a.get("flying")))
            .unwrap_or(0);
        assert_eq!(flying_byte, 1);

        let mut loaded = Level::default();
        loaded.read_player(&tag);
        assert!(loaded.flying);

        // Survival mode resets flying ability to 0 in save.
        level.creative = false;
        level.game_mode = 0;
        let tag_survival = level.player_tag(&Default::default());
        let flying_survival = tag_survival
            .get("abilities")
            .and_then(|a| number(a.get("flying")))
            .unwrap_or(0);
        assert_eq!(flying_survival, 0);
    }

    #[test]
    fn unconsumed_root_vehicle_keeps_the_complete_vanilla_tree() {
        let entity = Tag::compound([
            ("id", Tag::String("minecraft:horse".into())),
            ("UUID", uuid_tag([5, 6, 7, 8])),
            (
                "Passengers",
                Tag::List(vec![Tag::compound([
                    ("id", Tag::String("minecraft:pig".into())),
                    ("UUID", uuid_tag([1, 2, 3, 4])),
                ])]),
            ),
        ]);
        let vehicle = Tag::compound([("Attach", uuid_tag([1, 2, 3, 4])), ("Entity", entity)]);
        let raw = Tag::compound([("RootVehicle", vehicle.clone())]);
        let mut level = Level::default();
        level.read_player(&raw);
        level.player_extra = Some(raw);
        assert_eq!(level.root_vehicle, Some([1, 2, 3, 4]));
        assert_eq!(
            level.player_tag(&Default::default()).get("RootVehicle"),
            Some(&vehicle)
        );
        // Even saving before the vehicle is restored must not destroy it.
        level.root_vehicle = None;
        assert_eq!(
            level.player_tag(&Default::default()).get("RootVehicle"),
            Some(&vehicle)
        );
    }

    /// `RecipesUsed` is what a furnace owes in experience, so a save that drops it pays the
    /// player nothing after a reload.
    #[test]
    fn a_furnaces_recipes_used_survive_a_save() {
        use crate::containers::BlockEntity;
        let Some(BlockEntity::Furnace(mut f)) = BlockEntity::for_block("minecraft:furnace") else {
            panic!("a furnace");
        };
        f.recipes_used
            .insert("minecraft:cooked_porkchop".to_owned(), 3);
        let tag = block_entity_encode::encode(
            (1, 64, 2),
            &BlockEntity::Furnace(f),
            "minecraft:furnace",
            &Default::default(),
        );
        let Some((_, BlockEntity::Furnace(back))) = decode_block_entity(&tag) else {
            panic!("a furnace back");
        };
        assert_eq!(back.recipes_used.get("minecraft:cooked_porkchop"), Some(&3));
    }
    /// The world's save (Level.write) writes the rules as they are now, and a
    /// world opened again reads them back (#74).
    #[test]
    fn level_save_keeps_changed_rules() {
        use crate::gamerules::{GameRules, Value};
        let dir =
            std::env::temp_dir().join(format!("sunrise-gamerules-level-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut level = super::Level::default();
        level.write(&dir, &Default::default()).unwrap();
        // Rules changed while playing, then saved and loaded again.
        let mut rules = GameRules::load(&dir);
        assert_eq!(rules, GameRules::default());
        assert!(rules.set("advance_time", Value::Bool(false)));
        assert!(rules.set("advance_weather", Value::Bool(false)));
        assert!(rules.set("random_tick_speed", Value::Int(0)));
        level.game_rules = rules.by_name();
        level.write(&dir, &Default::default()).unwrap();
        assert_eq!(GameRules::load(&dir), rules);
        let read = super::Level::read(&dir).unwrap();
        assert_eq!(read.game_rules, rules.by_name());
        assert_eq!(read.game_rule("advance_time"), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn level_save_rule_files_round_trip_without_byte_changes() {
        for absent_mod in [false, true] {
            let dir = std::env::temp_dir().join(format!(
                "sunrise-rule-file-roundtrip-{}-{absent_mod}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let mut level = super::Level::default();
            level.write(&dir, &Default::default()).unwrap();
            if absent_mod {
                let mut data = super::read_saved_data(&dir, "game_rules").unwrap();
                super::put(&mut data, "absent:rule", super::Tag::Long(42));
                super::write_saved_data(&dir, "game_rules", data).unwrap();
            }
            let mut before_level = super::read_dat(&dir.join("level.dat")).unwrap();
            let path = super::saved_data_path(&dir, "game_rules");
            let before_rules = std::fs::read(&path).unwrap();
            super::Level::read(&dir)
                .unwrap()
                .write(&dir, &Default::default())
                .unwrap();
            let mut after_level = super::read_dat(&dir.join("level.dat")).unwrap();
            for level in [&mut before_level, &mut after_level] {
                if let super::Tag::Compound(root) = level
                    && let Some(super::Tag::Compound(data)) = root.get_mut(&"Data".into())
                {
                    data.remove(&"LastPlayed".into());
                }
            }
            assert_eq!(after_level, before_level);
            assert_eq!(std::fs::read(path).unwrap(), before_rules);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn level_read_retains_the_session_registry() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-level-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = std::sync::Arc::new(crate::gamerules::RuleRegistry::vanilla());
        let second = std::sync::Arc::new(crate::gamerules::RuleRegistry::vanilla());
        let mut level = super::Level {
            rule_registry: first.clone(),
            ..Default::default()
        };
        level.write(&dir, &Default::default()).unwrap();
        let loaded = super::Level::read_with_registry(&dir, first.clone()).unwrap();
        assert!(std::sync::Arc::ptr_eq(&loaded.rule_registry, &first));
        let next = super::Level::read_with_registry(&dir, second.clone()).unwrap();
        assert!(!std::sync::Arc::ptr_eq(
            &loaded.rule_registry,
            &next.rule_registry
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn level_saved_rules_have_vanilla_keys_types_and_defaults() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-rule-defaults-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut level = super::Level::default();
        level.write(&dir, &Default::default()).unwrap();
        let super::Tag::Compound(rules) = super::read_saved_data(&dir, "game_rules").unwrap()
        else {
            panic!("rule compound")
        };
        assert_eq!(rules.len(), 58);
        assert!(
            rules
                .keys()
                .all(|key| key.to_string().unwrap().starts_with("minecraft:"))
        );
        assert_eq!(
            rules.get(&"minecraft:random_tick_speed".into()),
            Some(&super::Tag::Int(3))
        );
        assert_eq!(
            rules.get(&"minecraft:keep_inventory".into()),
            Some(&super::Tag::Byte(0))
        );
        assert_eq!(
            rules.get(&"minecraft:max_snow_accumulation_height".into()),
            Some(&super::Tag::Int(1))
        );
        assert!(!rules.contains_key(&"minecraft:max_minecart_speed".into()));
        level.enabled_features = [crate::gamerules::MINECART_IMPROVEMENTS.to_owned()].into();
        level.game_rules =
            crate::gamerules::GameRules::with_features(&level.enabled_features).by_name();
        level.write(&dir, &Default::default()).unwrap();
        assert_eq!(
            super::read_saved_data(&dir, "game_rules")
                .unwrap()
                .get("minecraft:max_minecart_speed"),
            Some(&super::Tag::Int(8))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn legacy_bare_saved_rules_load_under_canonical_keys() {
        let dir = std::env::temp_dir().join(format!("sunrise-rule-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        super::Level::default()
            .write(&dir, &Default::default())
            .unwrap();
        super::write_saved_data(
            &dir,
            "game_rules",
            super::Tag::compound([
                ("keep_inventory", super::Tag::Byte(1)),
                ("random_tick_speed", super::Tag::Int(7)),
            ]),
        )
        .unwrap();
        let level = super::Level::read(&dir).unwrap();
        assert_eq!(level.game_rules.get("minecraft:keep_inventory"), Some(&1));
        assert_eq!(
            level.game_rules.get("minecraft:random_tick_speed"),
            Some(&7)
        );
        assert!(!level.game_rules.contains_key("keep_inventory"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn level_save_preserves_absent_mod_rules() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-gamerules-unknown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut level = super::Level::default();
        level.write(&dir, &Default::default()).unwrap();
        super::write_saved_data(
            &dir,
            "game_rules",
            super::Tag::compound([("absent:rule", super::Tag::Long(42))]),
        )
        .unwrap();
        level.game_rules.insert("minecraft:advance_time".into(), 0);
        level.write(&dir, &Default::default()).unwrap();
        assert_eq!(
            super::read_saved_data(&dir, "game_rules")
                .unwrap()
                .get("absent:rule"),
            Some(&super::Tag::Long(42))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A save written before the key was namespaced holds an id twice, once bare: vanilla
    /// refuses such a furnace, so a load merges them under the full id.
    #[test]
    fn a_furnaces_recipes_used_merges_bare_and_full_ids() {
        use crate::containers::BlockEntity;
        let Some(BlockEntity::Furnace(mut f)) = BlockEntity::for_block("minecraft:furnace") else {
            panic!("a furnace");
        };
        f.recipes_used
            .insert("iron_ingot_from_smelting_iron_ore".to_owned(), 2);
        f.recipes_used
            .insert("minecraft:iron_ingot_from_smelting_iron_ore".to_owned(), 3);
        let tag = block_entity_encode::encode(
            (1, 64, 2),
            &BlockEntity::Furnace(f),
            "minecraft:furnace",
            &Default::default(),
        );
        let Some((_, BlockEntity::Furnace(back))) = decode_block_entity(&tag) else {
            panic!("a furnace back");
        };
        assert_eq!(back.recipes_used.len(), 1);
        assert_eq!(
            back.recipes_used
                .get("minecraft:iron_ingot_from_smelting_iron_ore"),
            Some(&5)
        );
    }

    /// A world with an experiment writes and reads its full feature set.
    #[test]
    fn experimental_feature_flags_round_trip() {
        let dir = gen_world("enabled-features");
        let mut level = Level {
            name: "Features".into(),
            enabled_features: [
                "minecraft:vanilla".into(),
                "minecraft:minecart_improvements".into(),
            ]
            .into(),
            ..Level::default()
        };
        level.write(&dir, &Default::default()).unwrap();
        let back = Level::read(&dir).unwrap();
        assert_eq!(back.enabled_features, level.enabled_features);
        let raw = read_level_dat(&dir).unwrap();
        assert!(matches!(
            raw.get("Data").unwrap().get("enabled_features"),
            Some(Tag::List(_)),
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn omitted_feature_flags_decode_as_vanilla_and_stay_omitted() {
        let dir = gen_world("omitted-feature-flags");
        let output = gen_world("resaved-feature-flags");
        let mut level = Level::default();
        assert_eq!(*level.enabled_features, ["minecraft:vanilla"]);
        level.write(&dir, &Default::default()).unwrap();
        let raw = read_level_dat(&dir).unwrap();
        assert!(raw.get("Data").unwrap().get("enabled_features").is_none());

        let mut read = Level::read(&dir).unwrap();
        assert_eq!(*read.enabled_features, ["minecraft:vanilla"]);
        // The encoder must drop even a tag retained from an older file.
        let mut experimental = Level {
            enabled_features: [
                "minecraft:vanilla".into(),
                "minecraft:minecart_improvements".into(),
            ]
            .into(),
            ..Level::default()
        };
        experimental.write(&output, &Default::default()).unwrap();
        read.write(&output, &Default::default()).unwrap();
        let raw = read_level_dat(&output).unwrap();
        assert!(raw.get("Data").unwrap().get("enabled_features").is_none());
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(output);
    }

    /// `WorldDataConfiguration.MAP_CODEC` preserves a valid empty set, but defaults an
    /// invalid tag instead of interpreting a failed list decode as an empty set.
    #[test]
    fn explicit_empty_and_malformed_feature_flags_have_distinct_save_outcomes() {
        for (case, input, expected_flags, expected_tag) in [
            ("empty", Tag::List(vec![]), &[][..], Some(Tag::List(vec![]))),
            (
                "non-string",
                Tag::List(vec![Tag::Int(7)]),
                &["minecraft:vanilla"][..],
                None,
            ),
            ("non-list", Tag::Int(7), &["minecraft:vanilla"][..], None),
        ] {
            let dir = gen_world(&format!("feature-flags-{case}"));
            write_dat(
                &dir.join("level.dat"),
                Tag::compound([(
                    "Data",
                    Tag::compound([
                        ("DataVersion", Tag::Int(DATA_VERSION)),
                        ("enabled_features", input),
                    ]),
                )]),
            )
            .unwrap();
            let mut level = Level::read(&dir).unwrap();
            assert_eq!(&*level.enabled_features, expected_flags, "{case}");
            level.write_without_player(&dir).unwrap();
            let raw = read_level_dat(&dir).unwrap();
            assert_eq!(
                raw.get("Data").unwrap().get("enabled_features"),
                expected_tag.as_ref(),
                "{case}"
            );
            let _ = fs::remove_dir_all(dir);
        }
    }

    fn gen_world(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sunrise-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `WorldOpenFlows.openWorldCheckWorldStemCompatibility`: every world a preset makes is
    /// stable, and a world with a changed dimension, a custom one or a customized generator
    /// is asked about.
    #[test]
    fn world_stem_compatibility_flags_unstable_and_customized_worlds() {
        for preset in [
            WorldPreset::Normal,
            WorldPreset::Flat,
            WorldPreset::LargeBiomes,
            WorldPreset::Amplified,
            WorldPreset::SingleBiomeSurface,
            WorldPreset::Debug,
        ] {
            let dir = gen_world("stem-stable");
            write_world_gen_settings(
                &dir,
                7,
                WorldGen {
                    preset,
                    structures: true,
                    bonus_chest: false,
                },
            )
            .unwrap();
            assert!(!world_gen_unstable(&dir), "{preset:?}");
            assert!(!is_old_customized_world(&dir));
            let _ = fs::remove_dir_all(dir);
        }
        // Without world_gen_settings there is nothing to ask about.
        let dir = gen_world("stem-none");
        assert!(!world_gen_unstable(&dir) && !is_old_customized_world(&dir));
        let _ = fs::remove_dir_all(dir);

        let dir = gen_world("stem-edit");
        write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
        let settings = world_gen_settings(&dir).unwrap();
        let edit = |change: &dyn Fn(&mut Tag)| {
            let mut tag = settings.clone();
            change(&mut tag);
            write_saved_data(&dir, "world_gen_settings", tag).unwrap();
        };
        fn dimension<'a>(tag: &'a mut Tag, key: &str) -> &'a mut Tag {
            tag.get_mut("dimensions").unwrap().get_mut(key).unwrap()
        }
        // A custom biome source in the Nether is not the preset.
        edit(&|tag| {
            let nether = dimension(tag, "minecraft:the_nether");
            let generator = nether.get_mut("generator").unwrap();
            *generator.get_mut("biome_source").unwrap() = Tag::compound([
                ("type", string("minecraft:fixed")),
                ("biome", string("minecraft:plains")),
            ]);
        });
        assert!(world_gen_unstable(&dir));
        // The End on another dimension type.
        edit(&|tag| {
            let end = dimension(tag, "minecraft:the_end");
            *end.get_mut("type").unwrap() = string("minecraft:overworld");
        });
        assert!(world_gen_unstable(&dir));
        // Another noise settings in the Overworld is fine (amplified), in the Nether it is not.
        edit(&|tag| {
            let nether = dimension(tag, "minecraft:the_nether");
            *nether
                .get_mut("generator")
                .unwrap()
                .get_mut("settings")
                .unwrap() = string("minecraft:overworld");
        });
        assert!(world_gen_unstable(&dir));
        // A fourth dimension, and a missing builtin one.
        edit(&|tag| {
            if let Some(Tag::Compound(dimensions)) = tag.get_mut("dimensions") {
                let end = dimensions.get(&"minecraft:the_end".into()).unwrap().clone();
                dimensions.insert("mypack:moon".into(), end);
            }
        });
        assert!(world_gen_unstable(&dir));
        edit(&|tag| {
            if let Some(Tag::Compound(dimensions)) = tag.get_mut("dimensions") {
                dimensions.remove(&"minecraft:the_end".into());
            }
        });
        assert!(world_gen_unstable(&dir));
        // legacy_custom_options marks an old customized world.
        edit(&|tag| {
            if let Tag::Compound(map) = tag {
                map.insert("legacy_custom_options".into(), string("{\"seaLevel\":63}"));
            }
        });
        assert!(is_old_customized_world(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    /// `WorldOpenFlows.loadWorldStem`: the enabled data packs' level stems are the base of
    /// `WorldDimensions.bake`, replacing the saved one of their id.
    #[test]
    fn world_stem_compatibility_reads_data_pack_dimensions() {
        let dir = gen_world("stem-datapack");
        write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
        assert!(!world_gen_unstable(&dir));
        let write = |pack: &str, file: &str, text: &str| {
            let path = dir.join("datapacks").join(pack).join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write("p", "pack.mcmeta", r#"{"pack":{"description":"fixture"}}"#);
        // A pack's dimension of another id is not vanilla-like.
        write(
            "p",
            "data/mypack/dimension/moon.json",
            r#"{"type":"minecraft:the_end","generator":{"type":"minecraft:noise","settings":"minecraft:end","biome_source":{"type":"minecraft:the_end"}}}"#,
        );
        assert!(world_gen_unstable(&dir));
        // Disabled, it counts for nothing.
        fs::write(dir.join("datapacks.json"), r#"{"disabled":["file/p"]}"#).unwrap();
        assert!(!world_gen_unstable(&dir));
        fs::remove_file(dir.join("datapacks.json")).unwrap();
        fs::remove_file(dir.join("datapacks/p/data/mypack/dimension/moon.json")).unwrap();
        assert!(!world_gen_unstable(&dir));
        // A pack's Nether with another biome source replaces the saved vanilla one.
        write(
            "p",
            "data/minecraft/dimension/the_nether.json",
            r#"{"type":"minecraft:the_nether","generator":{"type":"minecraft:noise","settings":"minecraft:nether","biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#,
        );
        assert!(world_gen_unstable(&dir));
        // And a vanilla-like one stays stable.
        write(
            "p",
            "data/minecraft/dimension/the_nether.json",
            r#"{"type":"minecraft:the_nether","generator":{"type":"minecraft:noise","settings":"minecraft:nether","biome_source":{"type":"minecraft:multi_noise","preset":"minecraft:nether"}}}"#,
        );
        assert!(!world_gen_unstable(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    /// A world that still needs its file fixes keeps its generator settings in level.dat; they
    /// are read as the fixes would leave them, so the question is asked on the first open.
    #[test]
    fn world_gen_settings_of_a_world_needing_file_fixes_are_read_in_memory() {
        let dir = gen_world("stem-prefix");
        write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
        let settings = world_gen_settings(&dir).unwrap();
        let customized = {
            let mut tag = settings.clone();
            if let Tag::Compound(map) = &mut tag {
                map.insert("legacy_custom_options".into(), string("{\"seaLevel\":63}"));
            }
            tag
        };
        // 26.3 layout moved back to the old one: settings in level.dat, no saved data file.
        fs::remove_file(saved_data_path(&dir, "world_gen_settings")).unwrap();
        write_dat(
            &dir.join("level.dat"),
            Tag::compound([(
                "Data",
                Tag::compound([
                    ("DataVersion", Tag::Int(4700)),
                    ("WorldGenSettings", customized),
                ]),
            )]),
        )
        .unwrap();
        assert!(needs_file_fixes(&dir));
        let read = world_gen_settings(&dir).expect("settings");
        assert!(read.get("dimensions").is_some());
        assert!(!world_gen_unstable(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    /// Settings that fail validation in the old layout do not hide valid saved data.
    #[test]
    fn invalid_projected_settings_fall_through_to_saved_data() {
        let dir = gen_world("stem-prefix");
        write_world_gen_settings(&dir, 7, WorldGen::default()).unwrap();
        write_dat(
            &dir.join("level.dat"),
            Tag::compound([(
                "Data",
                Tag::compound([
                    ("DataVersion", Tag::Int(4700)),
                    ("WorldGenSettings", Tag::compound([("seed", Tag::Long(5))])),
                ]),
            )]),
        )
        .unwrap();
        assert!(needs_file_fixes(&dir));
        assert_eq!(
            number(world_gen_settings(&dir).unwrap().get("seed")),
            Some(7)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn low_disk_space_is_under_64_mib_usable() {
        assert_eq!(LOW_DISK_SPACE, 64 * 1024 * 1024);
        let dir = std::env::temp_dir();
        assert!(estimate_disk_space(&dir) > 0);
        assert_eq!(
            low_disk_space(&dir),
            estimate_disk_space(&dir) < LOW_DISK_SPACE
        );
        // A path that does not exist cannot be asked: vanilla reads that as plenty.
        #[cfg(unix)]
        assert_eq!(
            estimate_disk_space(&dir.join("sunrise-no-such-folder/x")),
            u64::MAX
        );
    }

    #[test]
    fn mod_attachments_save_in_sunrises_namespace_only_when_some() {
        use sunrise_mod_runtime::attachments::Attachments;
        let root = std::env::temp_dir().join(format!("sunrise-attach-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let save = SaveDir::new(
            root.clone(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        let file = root.join("data/sunrise/attachments.dat");
        save.save_attachments(&Attachments::default()).unwrap();
        assert!(
            !root.join("data").exists(),
            "a level without mod data writes nothing"
        );
        let data = Compound::from_iter([("example:count".into(), Tag::Int(3))]);
        save.save_attachments(&Attachments::from_nbt(data.clone()))
            .unwrap();
        assert!(file.is_file());
        assert_eq!(save.load_attachments().to_nbt(), data);
        fs::write(&file, b"not nbt").unwrap();
        assert!(save.load_attachments().to_nbt().is_empty());
        assert_eq!(
            fs::read(root.join("data/sunrise/attachments.dat.corrupt")).unwrap(),
            b"not nbt"
        );
        assert!(!file.exists());
        fs::remove_dir_all(&root).unwrap();
    }

    /// A level id never names a folder outside the world's `dimensions` folder, and no two ids
    /// share a folder.
    #[test]
    fn a_level_folder_stays_inside_the_world() {
        let world = Path::new("/w");
        assert_eq!(
            dimension_dir(world, "minecraft:the_nether"),
            Path::new("/w/dimensions/minecraft/the_nether")
        );
        assert_eq!(
            dimension_dir(world, "test:a/b"),
            Path::new("/w/dimensions/test/a/b")
        );
        let mut seen = std::collections::HashSet::new();
        for id in [
            "test:../minecraft/overworld",
            "test:a/../../b",
            "test:/abs",
            "..:x",
            "test:a//b",
            "test:Pocket",
        ] {
            let dir = dimension_dir(world, id);
            assert!(
                dir.starts_with("/w/dimensions/%invalid")
                    && dir
                        .components()
                        .all(|c| !matches!(c, std::path::Component::ParentDir)),
                "{id}: {}",
                dir.display()
            );
            assert!(seen.insert(dir), "{id} shares a folder");
        }
    }

    #[test]
    fn forced_chunks_survive_dimension_restarts_and_removals() {
        let root = std::env::temp_dir().join(format!("sunrise-forced-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dimensions = [
            "minecraft:overworld",
            "minecraft:the_nether",
            "minecraft:the_end",
        ];
        for (i, dim) in dimensions.iter().enumerate() {
            let save = SaveDir::new(
                dimension_dir(&root, dim),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            );
            assert!(save.load_forced_chunks().is_empty());
            let forced = HashSet::from([(i as i32, -17), (-23, i as i32), (-1, -1)]);
            save.save_chunk_tickets(&forced, &[]).unwrap();
        }
        for (i, dim) in dimensions.iter().enumerate() {
            let path = dimension_dir(&root, dim);
            let save = SaveDir::new(
                path.clone(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            );
            let mut forced = save.load_forced_chunks();
            assert_eq!(
                forced,
                HashSet::from([(i as i32, -17), (-23, i as i32), (-1, -1)])
            );
            let mut data = read_saved_data(&path, "chunk_tickets").unwrap();
            let other = Tag::compound([
                ("type", Tag::String("example:loader".into())),
                ("level", Tag::Int(30)),
                ("chunk_pos", Tag::IntArray(vec![7, -8])),
                ("ticks_left", Tag::Long(42)),
            ]);
            if let Some(Tag::List(tickets)) = data.get_mut("tickets") {
                assert!(
                    tickets
                        .iter()
                        .all(|t| t.get("level") == Some(&Tag::Int(31)))
                );
                tickets.push(other.clone());
            }
            write_saved_data(&path, "chunk_tickets", data).unwrap();
            forced.remove(&(-1, -1));
            save.save_chunk_tickets(&forced, &[]).unwrap();
            drop(save);
            let reopened = SaveDir::new(
                path.clone(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            );
            assert_eq!(reopened.load_forced_chunks(), forced);
            reopened.save_chunk_tickets(&HashSet::new(), &[]).unwrap();
            assert!(
                SaveDir::new(path.clone(), LevelShape::OVERWORLD, None)
                    .load_forced_chunks()
                    .is_empty()
            );
            assert_eq!(
                read_saved_data(&path, "chunk_tickets")
                    .unwrap()
                    .get("tickets"),
                Some(&Tag::List(vec![other]))
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// The Debug preset (`worldgen/world_preset/debug_all_block_states.json`): its id names it,
    /// the saved overworld generator is `minecraft:debug` and reads back as it, and the create
    /// screen's button never cycles to it.
    #[test]
    fn the_debug_preset_is_saved_and_read_back() {
        assert_eq!(
            WorldPreset::of_id("minecraft:debug_all_block_states"),
            Some(WorldPreset::Debug)
        );
        assert!(!WorldPreset::ALL.contains(&WorldPreset::Debug));
        let dir = gen_world("debug-preset");
        write_world_gen_settings(
            &dir,
            7,
            WorldGen {
                preset: WorldPreset::Debug,
                structures: true,
                bonus_chest: false,
            },
        )
        .unwrap();
        let settings = world_gen_settings(&dir).unwrap();
        let generator = settings
            .get("dimensions")
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"))
            .unwrap();
        assert_eq!(
            text(generator.get("type")).as_deref(),
            Some("minecraft:debug")
        );
        assert_eq!(WorldGen::read(&dir).unwrap().preset, WorldPreset::Debug);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn world_preset_next_and_prev_cycle_bidirectionally() {
        let first = WorldPreset::Normal;
        assert_eq!(first.next(), WorldPreset::Flat);
        assert_eq!(first.prev(), WorldPreset::SingleBiomeSurface);
        assert_eq!(WorldPreset::SingleBiomeSurface.next(), WorldPreset::Normal);
        for preset in WorldPreset::ALL {
            assert_eq!(preset.next().prev(), preset);
            assert_eq!(preset.prev().next(), preset);
        }
    }
    /// A region record of the dimension folder `save` at chunk (0, 0), written
    /// as an older version left it.
    fn write_old_record(save: &SaveDir, folder: &str, tag: Tag) {
        let record = Record::Tag(Arc::new(NamedTag {
            name: "".into(),
            tag,
        }));
        let result = super::write_region_records(
            save.region_storage(Folder::of(folder)),
            &[((0, 0), &record)],
            &fs::File::sync_data,
        );
        assert!(result.failed_records.is_empty() && result.sync_error.is_none());
    }

    #[test]
    fn older_records_of_a_current_world_upgrade_as_they_are_read() {
        let world = std::env::temp_dir().join(format!("sunrise-lazy-dv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        let save = SaveDir::new(
            super::dimension_dir(&world, "minecraft:overworld"),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        // A 1.20.1 chunk (DataVersion 3465): 1.20.3 renamed its grass to short_grass.
        let section = Tag::compound([
            ("Y", Tag::Byte(-4)),
            (
                "block_states",
                Tag::compound([(
                    "palette",
                    Tag::List(vec![Tag::compound([("Name", string("minecraft:grass"))])]),
                )]),
            ),
            (
                "biomes",
                Tag::compound([("palette", Tag::List(vec![string("minecraft:plains")]))]),
            ),
        ]);
        write_old_record(
            &save,
            "region",
            Tag::compound([
                ("DataVersion", Tag::Int(3465)),
                ("xPos", Tag::Int(0)),
                ("zPos", Tag::Int(0)),
                ("yPos", Tag::Int(-4)),
                ("Status", string("minecraft:full")),
                ("sections", Tag::List(vec![section])),
            ]),
        );
        let chunk = save.raw_chunk(0, 0).expect("the chunk reads");
        assert_eq!(
            number(chunk.tag.get("DataVersion")),
            Some(i64::from(DATA_VERSION))
        );
        let Some(Tag::List(sections)) = chunk.tag.get("sections") else {
            panic!("no sections: {:?}", chunk.tag);
        };
        let Some(Tag::List(palette)) = sections[0]
            .get("block_states")
            .and_then(|b| b.get("palette"))
        else {
            panic!("no palette");
        };
        assert_eq!(
            text(palette[0].get("id")).as_deref(),
            Some("minecraft:short_grass")
        );
        // Decoded, it keeps its blocks: read raw, the 26.x decoder finds no `id` in a
        // 1.20 palette (`Name`) and the whole column is air.
        if let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") {
            let root = PathBuf::from(root);
            let assets = crate::assets::Assets::load(&root).expect("assets");
            let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
                root.to_path_buf(),
            ))
            .expect("biomes");
            let grass = assets
                .registry
                .default_state("minecraft:short_grass")
                .unwrap();
            let loaded = save
                .load(0, 0, assets.registry, &biomes)
                .expect("the chunk loads");
            assert_eq!(loaded.get(3, -60, 7), grass.0);
        }
        // A 1.20.1 item entity: 1.20.5 made its stack `count` (an int) from `Count`.
        write_old_record(
            &save,
            "entities",
            Tag::compound([
                ("DataVersion", Tag::Int(3465)),
                ("Position", Tag::IntArray(vec![0, 0])),
                (
                    "Entities",
                    Tag::List(vec![Tag::compound([
                        ("id", string("minecraft:item")),
                        (
                            "Item",
                            Tag::compound([
                                ("id", string("minecraft:stone")),
                                ("Count", Tag::Byte(3)),
                            ]),
                        ),
                    ])]),
                ),
            ]),
        );
        let entities = save.load_entities(0, 0).unwrap();
        let item = entities[0].get("Item").expect("the item stack");
        assert_eq!(item.get("count"), Some(&Tag::Int(3)));
        assert!(item.get("Count").is_none());
        let _ = std::fs::remove_dir_all(&world);
    }

    #[test]
    fn old_chunks_are_found_in_the_saved_records_and_old_poi_upgrade() {
        let world = std::env::temp_dir().join(format!("sunrise-lazy-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        let nether = SaveDir::new(
            super::dimension_dir(&world, "minecraft:the_nether"),
            LevelShape::NETHER,
            Some(Default::default()),
        );
        // A 1.20.1 Nether chunk: upgraded it has no `blending_data` (only Overworld
        // chunks blend) and the current DataVersion, so only the saved record says
        // it is older than blending (4997), as `IOWorker.isOldChunk` reads it.
        write_old_record(
            &nether,
            "region",
            Tag::compound([
                ("DataVersion", Tag::Int(3465)),
                ("xPos", Tag::Int(0)),
                ("zPos", Tag::Int(0)),
                ("yPos", Tag::Int(0)),
                ("Status", string("minecraft:full")),
                ("sections", Tag::List(Vec::new())),
            ]),
        );
        let upgraded = nether.raw_chunk(0, 0).expect("the chunk reads");
        assert!(upgraded.tag.get("blending_data").is_none());
        assert_ne!(nether.old_chunk_bits(0, 0)[0] & 1, 0);
        // One chunk's record tells the same, without the region's other chunks; a chunk this
        // version saved, and one not saved, are not old.
        let current = Record::Tag(Arc::new(NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("xPos", Tag::Int(1)),
                ("zPos", Tag::Int(0)),
                ("yPos", Tag::Int(0)),
                ("Status", string("minecraft:full")),
                ("sections", Tag::List(Vec::new())),
            ]),
        }));
        let result = super::write_region_records(
            nether.region_storage(Folder::Region),
            &[((1, 0), &current)],
            &fs::File::sync_data,
        );
        assert!(result.failed_records.is_empty() && result.sync_error.is_none());
        assert!(nether.is_old_chunk(0, 0));
        assert!(!nether.is_old_chunk(1, 0));
        assert!(!nether.is_old_chunk(2, 0));
        assert_eq!(nether.old_chunk_bits(0, 0)[0] & 0b110, 0);
        // A POI record of before 1.14.4 (no DataVersion, read as 1945): its
        // sections at the root, which 1946 moved under `Sections`.
        let overworld = SaveDir::new(
            super::dimension_dir(&world, "minecraft:overworld"),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        write_old_record(
            &overworld,
            "poi",
            Tag::compound([(
                "0",
                Tag::compound([("Valid", Tag::Byte(1)), ("Records", Tag::List(Vec::new()))]),
            )]),
        );
        let poi = overworld.read_record("poi", 0, 0).expect("the POI reads");
        assert!(poi.tag.get("Sections").and_then(|s| s.get("0")).is_some());
        assert_eq!(
            number(poi.tag.get("DataVersion")),
            Some(i64::from(DATA_VERSION))
        );
        let _ = std::fs::remove_dir_all(&world);
    }

    #[test]
    fn a_chunk_from_before_1_18_is_brought_to_the_current_layout() {
        let world = std::env::temp_dir().join(format!("sunrise-lazy-117-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        let save = SaveDir::new(
            super::dimension_dir(&world, "minecraft:overworld"),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        // A 1.17.1 chunk (DataVersion 2730), its fields under `Level`: section 0 all stone.
        let section = Tag::compound([
            ("Y", Tag::Byte(0)),
            (
                "Palette",
                Tag::List(vec![
                    Tag::compound([("Name", string("minecraft:air"))]),
                    Tag::compound([("Name", string("minecraft:stone"))]),
                ]),
            ),
            (
                "BlockStates",
                Tag::LongArray(vec![0x1111_1111_1111_1111; 256]),
            ),
        ]);
        write_old_record(
            &save,
            "region",
            Tag::compound([
                ("DataVersion", Tag::Int(2730)),
                (
                    "Level",
                    Tag::compound([
                        ("xPos", Tag::Int(0)),
                        ("zPos", Tag::Int(0)),
                        ("Status", string("full")),
                        ("Biomes", Tag::IntArray(vec![1; 1024])),
                        ("Sections", Tag::List(vec![section])),
                    ]),
                ),
            ]),
        );
        let chunk = save.raw_chunk(0, 0).expect("the chunk reads");
        assert!(
            chunk.tag.get("Level").is_none(),
            "still wrapped: {:?}",
            chunk.tag
        );
        assert_eq!(number(chunk.tag.get("xPos")), Some(0));
        // 1.18's height fix: an Overworld chunk older than the new heights blends with them.
        assert!(chunk.tag.get("blending_data").is_some());
        let _ = std::fs::remove_dir_all(&world);
    }

    /// A chunk last saved by 1.16.5 (DataVersion 2586) keeps its entities in its own
    /// `Level.Entities`: the upgrade moves them to `entities` and the load adds them
    /// (`SerializableChunkData.postLoadChunk`, `ServerLevel.addLegacyChunkEntities`), so its
    /// minecart is in the world (#14424).
    #[test]
    #[ignore = "daily"]
    fn a_chunk_from_before_1_17_loads_the_entities_it_keeps() {
        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes");
        let world = std::env::temp_dir().join(format!("sunrise-lazy-116-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        let save = SaveDir::new(
            super::dimension_dir(&world, "minecraft:overworld"),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        let section = Tag::compound([
            ("Y", Tag::Byte(0)),
            (
                "Palette",
                Tag::List(vec![
                    Tag::compound([("Name", string("minecraft:air"))]),
                    Tag::compound([("Name", string("minecraft:stone"))]),
                ]),
            ),
            (
                "BlockStates",
                Tag::LongArray(vec![0x1111_1111_1111_1111; 256]),
            ),
        ]);
        let minecart = Tag::compound([
            ("id", string("minecraft:minecart")),
            (
                "Pos",
                Tag::List([3.5f64, 16.0, 7.5].map(|v| Tag::Double(v.to_bits())).into()),
            ),
            ("UUID", Tag::IntArray(vec![1, 2, 3, 4])),
        ]);
        write_old_record(
            &save,
            "region",
            Tag::compound([
                ("DataVersion", Tag::Int(2586)),
                (
                    "Level",
                    Tag::compound([
                        ("xPos", Tag::Int(0)),
                        ("zPos", Tag::Int(0)),
                        ("Status", string("full")),
                        ("Biomes", Tag::IntArray(vec![1; 1024])),
                        ("Sections", Tag::List(vec![section])),
                        ("Entities", Tag::List(vec![minecart])),
                    ]),
                ),
            ]),
        );
        let id = |e: &Tag| text(e.get("id"));
        let chunk = save
            .load(0, 0, assets.registry, &biomes)
            .expect("the chunk loads");
        let ids: Vec<_> = chunk.entities.iter().map(id).collect();
        assert_eq!(ids, [Some("minecraft:minecart".to_owned())]);
        // There is no `entities/` record for it: these are all it has.
        assert!(save.load_entities(0, 0).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&world);
    }
}

/// Far-terrain floating-cell census on a real save: `SUNRISE_TEST_RESOURCES`
/// (prepared data), `SUNRISE_FAR_SAVE` (a world folder), `SUNRISE_FAR_SEED`
/// and `SUNRISE_FAR_AT` ("x,z", the camera). Samples the tiles each level
/// draws around the camera as the far sampler gives them (saved chunks up to
/// 16 blocks a cell, the seed beyond) and counts components of drawn cells
/// that touch neither the world's bottom nor the tile's halo.
#[cfg(test)]
mod far_census {
    use super::*;
    use crate::lod::Column;
    use crate::streaming::ChunkGenerator;

    #[derive(Default, Debug)]
    struct Census {
        tiles: usize,
        floating: usize,
        cells: usize,
        kinds: std::collections::BTreeMap<&'static str, usize>,
        water_over_air: usize,
    }

    fn kind(name: &str) -> &'static str {
        if name.ends_with("_leaves") {
            "leaves"
        } else if name.ends_with("_log") || name.ends_with("_wood") || name.contains("stem") {
            "log"
        } else if name == "minecraft:water" || name == "minecraft:lava" {
            "fluid"
        } else if name.contains("snow") || name.contains("ice") {
            "snow/ice"
        } else {
            "ground"
        }
    }

    /// `columns`: `side` x `side`, the outer ring a halo.
    fn count(
        columns: &[Column],
        side: usize,
        name: &dyn Fn(sunrise_world::registry::BlockStateId) -> String,
        out: &mut Census,
        origin: (i32, i32, i32),
    ) {
        // Runs of touching spans, per column: (min, max, top state).
        let runs: Vec<Vec<(i16, i16, sunrise_world::registry::BlockStateId)>> = columns
            .iter()
            .map(|column| {
                let mut runs: Vec<(i16, i16, _)> = Vec::new();
                for span in column.spans.iter().filter(|s| s.min < s.max) {
                    match runs.last_mut() {
                        Some(run) if run.1 == span.min => {
                            run.1 = span.max;
                            run.2 = span.state;
                        }
                        _ => runs.push((span.min, span.max, span.state)),
                    }
                }
                runs
            })
            .collect();
        let mut first = Vec::with_capacity(runs.len());
        let mut total = 0;
        for column in &runs {
            first.push(total);
            total += column.len();
        }
        let mut parent: Vec<usize> = (0..total).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (a, b) = (find(parent, a), find(parent, b));
            if a != b {
                parent[a] = b;
            }
        };
        for z in 0..side {
            for x in 0..side {
                let i = z * side + x;
                for j in [
                    (x + 1 < side).then(|| i + 1),
                    (z + 1 < side).then(|| i + side),
                ]
                .into_iter()
                .flatten()
                {
                    for (a, &(lo, hi, _)) in runs[i].iter().enumerate() {
                        for (b, &(other_lo, other_hi, _)) in runs[j].iter().enumerate() {
                            // Cells meeting at an edge (an acacia's
                            // diagonal branch) look joined too.
                            if lo <= other_hi && other_lo <= hi {
                                union(&mut parent, first[i] + a, first[j] + b);
                            }
                        }
                    }
                }
            }
        }
        // Per component: grounded, touches the halo, cells, top state.
        let mut grounded = vec![false; total];
        let mut halo = vec![false; total];
        let mut cells = vec![0usize; total];
        let mut top = vec![None; total];
        let mut at = vec![(0, 0); total];
        for z in 0..side {
            for x in 0..side {
                let i = z * side + x;
                let ring = x == 0 || z == 0 || x + 1 == side || z + 1 == side;
                for (a, &(lo, hi, state)) in runs[i].iter().enumerate() {
                    let root = find(&mut parent, first[i] + a);
                    grounded[root] |= i32::from(lo) <= crate::chunk::MIN_Y;
                    halo[root] |= ring;
                    cells[root] += 1;
                    if top[root].is_none_or(|(y, _)| hi > y) {
                        top[root] = Some((hi, state));
                        at[root] = (
                            origin.0 + x as i32 * origin.2,
                            origin.1 + z as i32 * origin.2,
                        );
                    }
                }
                if !ring {
                    let spans = &columns[i].spans;
                    for (k, span) in spans.iter().enumerate() {
                        let n = name(span.state);
                        if kind(&n) == "fluid" && (k == 0 || spans[k - 1].max < span.min) {
                            out.water_over_air += 1;
                            if std::env::var_os("SUNRISE_FAR_DEBUG").is_some() {
                                println!(
                                    "  fluid over air at {} {} y {}..{} {n}",
                                    origin.0 + x as i32 * origin.2,
                                    origin.1 + z as i32 * origin.2,
                                    span.min,
                                    span.max
                                );
                            }
                        }
                    }
                }
            }
        }
        for root in 0..total {
            if find(&mut parent, root) != root || grounded[root] || halo[root] {
                continue;
            }
            out.floating += 1;
            out.cells += cells[root];
            let (y, state) = top[root].unwrap();
            *out.kinds.entry(kind(&name(state))).or_default() += 1;
            if std::env::var_os("SUNRISE_FAR_DEBUG").is_some() {
                println!(
                    "  floating at {:?} top {y} {} ({} cells)",
                    at[root],
                    name(state),
                    cells[root]
                );
            }
        }
    }

    #[test]
    #[ignore = "requires prepared data and a save (see the module docs)"]
    fn far_floating_cells_per_level() {
        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name}"));
        let root = PathBuf::from(var("SUNRISE_TEST_RESOURCES"));
        let world = PathBuf::from(var("SUNRISE_FAR_SAVE"));
        let seed: i64 = var("SUNRISE_FAR_SEED").parse().unwrap();
        let at: Vec<i32> = var("SUNRISE_FAR_AT")
            .split(',')
            .map(|n| n.trim().parse().unwrap())
            .collect();
        let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
        let biomes = Arc::new(
            Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
                root.to_path_buf(),
            ))
            .expect("biomes"),
        );
        let registry = assets.registry;
        let item_data = crate::test_resources::item_data(
            &root,
            sunrise_world::native_load_context::DataLoadOrigin::ServerDataLoad,
        )
        .expect("item data");
        let terrain = Arc::new(
            crate::vanillagen::VanillaTerrain::new(
                crate::vanillagen::TerrainResources {
                    data: None,
                    dir: &root.join("worldgen"),
                    registry,
                    biomes: &biomes,
                    item_data: &item_data,
                },
                seed,
            )
            .expect("terrain"),
        );
        let save = Arc::new(SaveDir::new(
            world.join("dimensions/minecraft/overworld"),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        ));
        let generator = Arc::new(Persistent {
            inner: terrain,
            save: save.clone(),
            registry: assets.clone(),
            biomes: biomes.clone(),
        });
        let mut sampler = ChunkGenerator::lod_sampler(generator);
        let name = |state| {
            registry
                .state(state)
                .map_or_else(String::new, |s| s.name.to_string())
        };
        let limit: usize = std::env::var("SUNRISE_FAR_TILES")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(48)
            .max(1);
        for level in 1..=6u8 {
            let size = 16 << level;
            let width = size / 16;
            // The ring this level is drawn in (`Plan::visit` splits a tile
            // closer than 8 of its widths).
            let (near, far) = (8 * size, 16 * size);
            let mut tiles = Vec::new();
            for tz in (at[1] - far).div_euclid(size)..=(at[1] + far).div_euclid(size) {
                for tx in (at[0] - far).div_euclid(size)..=(at[0] + far).div_euclid(size) {
                    let (cx, cz) = (tx * size + size / 2, tz * size + size / 2);
                    let d = (((cx - at[0]) as f64).powi(2) + ((cz - at[1]) as f64).powi(2)).sqrt();
                    if d >= f64::from(near) && d < f64::from(far) {
                        tiles.push((tx, tz));
                    }
                }
            }
            let step = tiles.len().div_ceil(limit).max(1);
            let mut saved = Census::default();
            let mut seeded = Census::default();
            let started = std::time::Instant::now();
            for &(tx, tz) in tiles.iter().step_by(step) {
                let (x, z) = (tx * size - width, tz * size - width);
                let columns = sampler.grid(x, z, width, 18);
                let all_saved = width <= SAVED_LOD_STRIDE
                    && (x.div_euclid(16)..=(x + 18 * width - 1).div_euclid(16)).all(|cx| {
                        (z.div_euclid(16)..=(z + 18 * width - 1).div_euclid(16))
                            .all(|cz| save.lod_surfaces_shared(cx, cz, &assets, &biomes).is_some())
                    });
                let out = if all_saved { &mut saved } else { &mut seeded };
                out.tiles += 1;
                count(&columns, 18, &name, out, (x, z, width));
            }
            println!(
                "level {level} (cells {width}, {near}..{far} blocks, {:.1}s): saved {saved:?} | seed {seeded:?}",
                started.elapsed().as_secs_f32()
            );
        }
    }
}

#[cfg(test)]
mod jukebox_song_clock_tests {
    use super::*;
    use crate::containers::{BlockEntity, Jukebox};

    fn input(clock: Option<i64>, with_record: bool) -> Tag {
        let record = if with_record {
            "RecordItem:{id:\"minecraft:music_disc_cat\",count:1,components:{\"minecraft:custom_data\":{h032:1}}},"
        } else {
            ""
        };
        let clock = clock.map_or(String::new(), |n| format!("ticks_since_song_started:{n}L,"));
        nbt::parse_snbt(&format!(
            "{{id:\"minecraft:jukebox\",x:4,y:200,z:4,{record}{clock}}}"
        ))
        .unwrap()
        .0
    }
    fn record_kept(tag: &Tag) {
        let record = tag.get("RecordItem").expect("actual modeled record");
        assert_eq!(
            record.get("id"),
            Some(&Tag::String("minecraft:music_disc_cat".into()))
        );
        assert_eq!(record.get("count"), Some(&Tag::Int(1)));
        assert_eq!(
            record
                .get("components")
                .and_then(|c| c.get("minecraft:custom_data"))
                .and_then(|c| c.get("h032")),
            Some(&Tag::Int(1))
        );
    }
    #[test]
    fn model_save_preserves_song_clock_presence_without_raw() {
        let mut failures = Vec::new();
        let mut check = |label: &str, tag: &Tag, expected: Option<i64>| {
            let got = tag.get("ticks_since_song_started").cloned();
            let want = expected.map(Tag::Long);
            if got != want {
                failures.push(format!("{label}: {got:?}, expected {want:?}"));
            }
        };
        for (label, clock, with_record) in [
            ("empty", None, false),
            ("record-only", None, true),
            ("loaded-zero", Some(0), true),
            ("loaded-41", Some(41), true),
        ] {
            let (pos, model) =
                decode_block_entity(&input(clock, with_record)).expect("actual load path");
            let BlockEntity::Jukebox(j) = &model else {
                panic!("wrong modeled type")
            };
            assert_eq!(j.record().is_some(), with_record);
            assert_eq!(j.ticks, clock.unwrap_or(0));
            // `setSongWithoutPlaying`: a saved clock, unstarted; else no song.
            assert_eq!(j.playing, clock.is_some());
            assert_eq!(j.loaded_clock, clock.is_some());
            let saved =
                block_entity_encode::encode(pos, &model, "minecraft:jukebox", &Default::default());
            let command = block_entity_encode::data(
                pos,
                &model,
                "minecraft:jukebox",
                Default::default(),
                &Default::default(),
            );
            check(label, &saved, clock);
            check(label, &command, clock);
            if with_record {
                record_kept(&saved);
                record_kept(&command);
            } else {
                assert!(saved.get("RecordItem").is_none());
            }
            if clock.is_some() {
                let mut detached = model.clone();
                let BlockEntity::Jukebox(j) = &mut detached else {
                    unreachable!()
                };
                j.playing = false;
                // Decoded clock state, not this separate legacy flag, owns saving.
                let saved = block_entity_encode::encode(
                    pos,
                    &detached,
                    "minecraft:jukebox",
                    &Default::default(),
                );
                check("clock independent of legacy playing", &saved, clock);
                record_kept(&saved);
            }
        }
        let empty = BlockEntity::Jukebox(Jukebox::default());
        check(
            "default modeled/no raw",
            &block_entity_encode::encode(
                (4, 200, 4),
                &empty,
                "minecraft:jukebox",
                &Default::default(),
            ),
            None,
        );
        let raw = input(Some(41), true);
        let (pos, loaded_model) = decode_block_entity(&raw).unwrap();
        let loaded = block_entity_encode::encode(
            pos,
            &loaded_model,
            "minecraft:jukebox",
            &Default::default(),
        );
        let BlockEntity::Jukebox(j) = loaded_model else {
            unreachable!()
        };
        let current = BlockEntity::Jukebox(Jukebox {
            items: j.items,
            playing: true,
            ..Default::default()
        });
        let current =
            block_entity_encode::encode(pos, &current, "minecraft:jukebox", &Default::default());
        check("modeled record-only/no raw", &current, None);
        record_kept(&current);
        // The chunk writer uses this real overlay after encoding its model.
        // A stale input clock must not resurrect the removed modeled field.
        let merged = merge_block_entity(&raw, &loaded, &current, false);
        check("contradictory raw clock", &merged, None);
        record_kept(&merged);
        let canonical = crate::block_entity_nbt::canonical(&merged).unwrap();
        check(
            "chunk canonical after actual model overlay",
            &canonical,
            None,
        );
        record_kept(&canonical);
        assert!(
            failures.is_empty(),
            "modeled song clock presence mismatches: {failures:?}"
        );
    }
}

/// Added controls (H077): an old chunk's calculated blending data is taken
/// when its save is requested and written with `BlendingData.pack`; the raw
/// compound stays for the rest. The heights are the official 26.3 server's
/// for the constructed edge chunk (12, 0).
#[cfg(test)]
mod calculated_blending_save_tests {
    use super::*;
    use sunrise_world::worldgen::blending::BlendingData;

    fn heights() -> [f32; 16] {
        let mut h = [f32::MAX; 16];
        h[12..].copy_from_slice(&[70.0, 70.0, 69.0, 68.0]);
        h
    }
    fn calculated(min: i32, max: i32) -> Arc<BlendingData> {
        Arc::new(BlendingData::unpack(min, max, Some(&heights())).unwrap())
    }
    /// A loaded old chunk: its raw `blending_data` kept in the loaded root.
    fn old_chunk(x: i32, z: i32) -> Chunk {
        let mut chunk = Chunk::empty(x, z);
        let mut extras = crate::chunk::ChunkExtras::default();
        extras.root.insert(
            "blending_data".into(),
            Tag::compound([("min_section", Tag::Int(-4)), ("max_section", Tag::Int(20))]),
        );
        chunk.saved = Some(Arc::new(extras));
        chunk
    }
    fn saved_heights(save: &SaveDir, x: i32, z: i32) -> Option<Option<Vec<u32>>> {
        let root = save.read_record_raw("region", x, z)?;
        let data = root.tag.get("blending_data")?.clone();
        Some(match data.get("heights") {
            Some(Tag::List(list)) => Some(
                list.iter()
                    .map(|v| match v {
                        Tag::Float(bits) => *bits,
                        other => panic!("{other:?}"),
                    })
                    .collect(),
            ),
            _ => None,
        })
    }
    fn resources() -> Option<(crate::assets::Assets, Arc<Biomes>)> {
        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES")?);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Arc::new(
            Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
                root.clone(),
            ))
            .expect("biomes"),
        );
        Some((assets, biomes))
    }
    fn folder(name: &str, assets: &crate::assets::Assets) -> (PathBuf, SaveDir) {
        let dir = std::env::temp_dir().join(format!("sunrise-h077-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD, Some(Default::default()));
        let _ = save.tables.set(Arc::new(SaveTables::new(assets)));
        (dir, save)
    }
    fn want() -> Vec<u32> {
        heights().map(f32::to_bits).to_vec()
    }

    /// The owner of a cache entry, as `SavedOldWorld` owns its cache.
    struct Owner(Arc<BlendingData>);
    /// A lookup that reads `owner`'s entry through a weak link, as the game's does.
    fn hook(save: &SaveDir, at: (i32, i32), owner: &Arc<Owner>) {
        let weak = Arc::downgrade(owner);
        save.set_blending_lookup(Box::new(move |x, z| {
            if (x, z) == at {
                weak.upgrade().map(|o| o.0.clone())
            } else {
                None
            }
        }));
    }

    /// The real `save_later` path writes the calculated heights, and only for
    /// the chunk the lookup has them for.
    #[test]
    fn save_later_writes_the_calculated_heights_of_a_hooked_folder() {
        let Some((assets, biomes)) = resources() else {
            return;
        };
        let (dir, save) = folder("later", &assets);
        let owner = Arc::new(Owner(calculated(-4, 20)));
        hook(&save, (12, 0), &owner);
        save.save_later(
            &[Arc::new(old_chunk(12, 0)), Arc::new(old_chunk(11, 0))],
            assets.registry,
            &biomes,
        )
        .unwrap();
        save.flush().unwrap();
        assert_eq!(saved_heights(&save, 12, 0), Some(Some(want())));
        // No calculation for (11, 0): its raw compound, without heights.
        assert_eq!(saved_heights(&save, 11, 0), Some(None));
        let _ = fs::remove_dir_all(&dir);
    }

    /// The owner is gone before the queued record is encoded: the record
    /// keeps the snapshot taken at the request (a lookup at encoding would
    /// find nothing).
    #[test]
    fn a_queued_record_keeps_its_snapshot_after_the_owner_is_dropped() {
        let Some((assets, biomes)) = resources() else {
            return;
        };
        let (dir, save) = folder("owner", &assets);
        let owner = Arc::new(Owner(calculated(-4, 20)));
        hook(&save, (12, 0), &owner);
        let encoding = Arc::new(Encoding {
            items: Default::default(),
            registry: assets.registry,
            biomes: biomes.clone(),
            tables: save.tables.get().cloned(),
            shape: save.shape,
        });
        // As `save_later` makes it: the snapshot taken at the request.
        let record = Record::Chunk(
            Arc::new(old_chunk(12, 0)),
            encoding,
            save.calculated_blending(12, 0),
        );
        drop(owner);
        assert!(
            save.calculated_blending(12, 0).is_none(),
            "the owner is gone"
        );
        let tag = record.tag().expect("a chunk record encodes");
        let heights = match tag.tag.get("blending_data").and_then(|b| b.get("heights")) {
            Some(Tag::List(list)) => list
                .iter()
                .map(|v| match v {
                    Tag::Float(bits) => *bits,
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<u32>>(),
            other => panic!("heights {other:?}"),
        };
        assert_eq!(heights, want());
        let _ = fs::remove_dir_all(&dir);
    }

    /// The same coordinates in another dimension's folder (no lookup), and a
    /// lookup whose section range differs from the saved record: the raw
    /// compound stays.
    #[test]
    fn an_unhooked_folder_or_another_section_range_keeps_the_raw_compound() {
        let Some((assets, biomes)) = resources() else {
            return;
        };
        let (dir, nether) = folder("nether", &assets);
        let owner = Arc::new(Owner(calculated(-4, 20)));
        nether
            .save_later(&[Arc::new(old_chunk(12, 0))], assets.registry, &biomes)
            .unwrap();
        nether.flush().unwrap();
        assert_eq!(saved_heights(&nether, 12, 0), Some(None));
        let _ = fs::remove_dir_all(&dir);
        let (dir, other) = folder("range", &assets);
        let elsewhere = Arc::new(Owner(calculated(0, 16)));
        hook(&other, (12, 0), &elsewhere);
        other
            .save(&[Arc::new(old_chunk(12, 0))], assets.registry, &biomes)
            .unwrap();
        assert_eq!(saved_heights(&other, 12, 0), Some(None));
        drop(owner);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod biome_tests;
