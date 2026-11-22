//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `world_edit` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait WorldEditServerHost {
    /// Side: server.
    /// For a server mod: the runs made at once, straight into the chunks of the authoritative level `dimension`
    /// (the one being ticked, as for `native_set_block`), with what `fill ... strict` does for block entities (a
    /// block replaced by another loses its block entity, and a new block gets the one `native_set_block` would
    /// give it, a mod block's included); no neighbour is told and the replaced block entity's contents do not
    /// drop. Inside a transaction in this level, the new blocks and entities are visible at once; abort quietly
    /// restores both, with the connection group each replaced block had. The changed chunks go to the players
    /// whole, with their resulting connection groups, only after the level's outermost commit.
    /// At positions also written with `LevelRef::set_block`, strict and ordinary edits commit in
    /// call order; the strict edit never spills a container or requests neighbor updates.
    /// Runs in chunks not loaded, and the parts of runs outside the column, are left out. The blocks changed, or
    /// why the edit was left out, with nothing changed: another dimension, a client of a server, a palette entry
    /// or predicate that fails typed validation, a run naming a palette entry or filter past the end,
    /// or more than 65536 filters.
    fn edit_blocks(&mut self, dimension: &str, edit: &BlockEdit) -> Result<u64, String>;
    /// Side: server.
    /// A block entity's data saved at `data_version` (a schematic's, a structure file's, an older world's), upgraded
    /// as the chunk loader upgrades a block entity of a chunk of that version: its type's datafixes up to the game's
    /// DataVersion. Data of exactly the game's version comes back unchanged, and so does a block entity whose `id`
    /// that version doesn't know, or data the fixes can't read. The chunk-wide fixes (blocks around it, bed block
    /// entities removed) need a chunk and don't run. Pure: nothing in the world changes. `Err` names the refusal.
    /// Before any fix runs: a negative version ([`NbtUpgradeError::NegativeVersion`]), one newer than the game's
    /// ([`NbtUpgradeError::FutureVersion`]), or data with a list or compound 512 levels deep or more, counted as
    /// encoded with the wrapper compound of each mixed-list element as a level ([`NbtUpgradeError::TooDeep`], the
    /// depth cap of every NBT read). After the fixes: upgraded data that deep ([`NbtUpgradeError::UpgradedTooDeep`]),
    /// as no NBT read could take it back. There is no size refusal.
    fn upgrade_block_entity_nbt(
        &self,
        data_version: i32,
        nbt: Compound,
    ) -> Result<Compound, NbtUpgradeError>;
    /// Side: server.
    /// An entity's data saved at `data_version`, upgraded as the entity chunk loader upgrades an entity of a chunk
    /// of that version: its type's datafixes, its passengers' included, up to the game's DataVersion. Data of exactly
    /// the game's version, or data the fixes can't read, comes back unchanged. Pure: nothing in the world changes.
    /// Refused by name as `upgrade_block_entity_nbt` refuses.
    fn upgrade_entity_nbt(
        &self,
        data_version: i32,
        nbt: Compound,
    ) -> Result<Compound, NbtUpgradeError>;
    /// Side: server.
    /// For a server mod: each column's height by `heightmap` in the authoritative level `dimension` (the one being
    /// ticked), none for a column not loaded. `Err` says why nothing was read: another dimension or a client of a
    /// server, or more than 65,536 columns (`MAX_COLUMNS`).
    fn surface_heights(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
    ) -> Result<Vec<Option<i32>>, String>;
    /// Side: server.
    /// Read the level generator before features, without creating, holding or loading chunks.
    /// One optional answer per input column, in order, independent of loaded chunks and previous calls.
    /// Height counts from the level top: the Nether returns its roof, not a floor. The top block
    /// omits slope rules and may differ even from an undecorated chunk. Trees, ores and other
    /// features are absent. Void columns and worlds without their own generator return None.
    /// Refuses before reading on a client, for a dimension other than the one being ticked,
    /// or for more than [`MAX_TERRAIN_COLUMNS`] inputs. One noise sample per column on the server
    /// tick; slow calls log the column count when the slow-call threshold is enabled.
    fn terrain_surface(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<TerrainColumn>>, String>;
    /// Side: server.
    /// For a server mod: `below` blocks under and `above` blocks from each column's height by `heightmap`, as a
    /// palette of complete `BlockState` values and indexes, with each column's server-side biome JSON water
    /// colour (`water_colors`, `0xRRGGBB`; `0` where not loaded; air outside the column and in columns not
    /// loaded). Colormap-derived grass and foliage tints are client-side only: read blended tints on the client
    /// with [`BlockAndTintGetter::block_tint`] in block tint callbacks or `ClientHost::biome_tint` in other
    /// client hooks. `Err` says why nothing was read: another dimension or a
    /// client of a server, `below + above` of 0 or more than the level's height, or a read of more than 1,048,576
    /// blocks (`MAX_READ`; it runs on the server tick, so read big areas in pieces).
    fn read_columns(
        &self,
        dimension: &str,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
        range: (u32, u32),
    ) -> Result<ColumnBlocks, String>;
    /// Side: server.
    /// For a server mod: structure template `id` (`mymod:hut`, found as
    /// `StructureTemplateManager.get` finds it: the world's saved structures,
    /// then the game's and the mods' own) as `/place template id` would place
    /// it at `at` with `rotation` and `mirror`, without placing anything. The
    /// blocks come in vanilla's placement order (full blocks, the others, then
    /// those with a block entity, each by y, x, z), from the palette
    /// `/place template` picks at `at`; no processor runs, and the template's
    /// entities are left out. None for an id that doesn't parse or has a `.`
    /// or `..` part, a template the game doesn't have, and on a client.
    /// API @API:structure_template@.
    fn structure_template(
        &mut self,
        id: &str,
        at: [i32; 3],
        rotation: StructureRotation,
        mirror: StructureMirror,
    ) -> Option<StructureTemplateSnapshot>;
    /// Side: server.
    /// For a server mod: template `source` placed at `at` the way `/place template`
    /// and a structure block's load place one, with `rotation`, `mirror` and
    /// `pivot`, the level position the turn and the mirroring go around (`at`,
    /// which `/place template` uses, turns the template about its own origin).
    /// Blocks, block-entity data and the template's own entities come through
    /// vanilla's placement, so states are mirrored then turned, block entities
    /// keep their data and get the loot seed a placement gives them, and
    /// entities are spawned as the game spawns them: no processor runs, nothing
    /// is `strict`, and the neighbours of a replaced block are not told. A
    /// template read from structure NBT is read by the game's own loader, within
    /// vanilla's limits for a template file (a `structure/*.nbt` read under
    /// `NbtAccounter.unlimitedHeap`, gzipped or plain). `Err` says why nothing
    /// was placed: an id that doesn't parse or has a `.` or `..` part, a
    /// template the game doesn't have, NBT that doesn't read or is past those
    /// limits, a template that places nothing, a client of a server, a chunk of
    /// the template's box not loaded, or the game's structure resources missing.
    /// API @API:structure_template@.
    fn place_template(
        &mut self,
        source: &StructureTemplateSource,
        at: [i32; 3],
        pivot: [i32; 3],
        rotation: StructureRotation,
        mirror: StructureMirror,
    ) -> Result<(), String>;
}
