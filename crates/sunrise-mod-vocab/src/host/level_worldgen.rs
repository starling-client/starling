//! The `level_worldgen` area: biomes, structures and the seed the generator uses. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_worldgen` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWorldgenBackend {
    /// Side: server.
    /// `ServerLevel.findClosestBiome3d`: search this level's generator for an id or `#tag`.
    /// The result follows vanilla's sampled spiral and vertical order; it is not a search
    /// through edited biome cells. Does not load resident chunks or switch parked levels.
    /// None for no match, a stale/unavailable level, an unavailable generator, a negative
    /// radius, or nonpositive sampling steps.
    /// Positive vertical steps larger than the build height sample only the clamped origin.
    fn level_find_closest_biome(
        &self,
        level: LevelRef,
        biome: &str,
        parameters: BiomeSearch,
    ) -> Option<BiomeSearchResult>;
    /// Side: server.
    /// `ServerLevel.findNearestMapStructure` without skipping known starts: the
    /// nearest start of `structure`, an id or a `#tag`, searched in this
    /// level's generator as `/locate structure` does. Random-spread structure
    /// sets are searched in squares of placement cells out to
    /// [`StructureSearch::radius`]; concentric rings (strongholds) check every
    /// ring position whatever the radius. Starts are built as the search needs
    /// them, without loading or generating chunks or switching parked levels.
    /// None for no start, a structure that does not generate in this level,
    /// a world without structures, a stale level, a client, a negative radius,
    /// or an origin outside vanilla's `Level.isInWorldBoundsHorizontal` bound
    /// (`-30_000_000..30_000_000`).
    fn level_find_nearest_structure(
        &self,
        level: LevelRef,
        structure: &str,
        parameters: StructureSearch,
    ) -> Option<StructureSearchResult>;
    /// Side: server.
    /// `StructureManager.getStructureWithPieceAt`: the structures `structure` names (an id or a
    /// `#tag`) with a start that one of its pieces reaches `pos` in, by the structure references
    /// of `pos`'s loaded chunk, in their order and without repeats. Empty where none does. None
    /// for a stale level, a client, and a chunk not loaded: never loads or generates chunks.
    fn level_structures_with_piece_at(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        structure: &str,
    ) -> Option<Vec<String>>;
    /// Side: server.
    /// `StructureManager.getStructureAt`: as [`LevelRef::structures_with_piece_at`], with the
    /// start's whole bounding box reaching `pos` instead of a piece's, so a position between a
    /// structure's pieces counts.
    fn level_structures_at(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        structure: &str,
    ) -> Option<Vec<String>>;
    /// Side: server.
    /// The biome id selected by `LevelReader.getBiome` and this level's biome zoom.
    /// Reads an active or parked server level. None for a
    /// stale handle or an unloaded selected noise column. Never generates or loads chunks.
    fn level_biome_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<String>;
    /// Side: client.
    /// The biome in the client's current level (`LevelReader.getBiome`). None before
    /// login, for a stale or foreign dimension, or an unloaded selected column. Never loads chunks.
    fn level_client_biome(&self, level: LevelRef, pos: [i32; 3]) -> Option<String>;
    /// Side: both.
    /// A biome's natural spawners by category (`MobSpawnSettings.getMobs`), from the level's
    /// biome table, data packs and biome modifiers included, categories sorted by name. None for a
    /// stale handle or an unknown biome. This is the biome's own list; `potential_spawns` is
    /// what a position gets after mods edit it. Reads no chunks.
    fn level_biome_spawners(&self, level: LevelRef, biome: &str) -> Option<Vec<BiomeSpawners>>;
    /// Side: both.
    /// A biome's own sky, fog and water colours (`BiomeSpecialEffects`), from the level's biome
    /// table. None for a stale handle or an unknown biome. Reads no chunks.
    fn level_biome_effects(&self, level: LevelRef, biome: &str) -> Option<BiomeEffects>;
    /// Side: both.
    /// Whether a biome is in a biome tag (`Holder.is(TagKey)`), by the level's tag tables: the
    /// vanilla tags, the world's data packs and mods' biome tags merged. `tag` is a namespaced
    /// id (`minecraft:is_forest`), with or without a leading `#`. A tag no pack defines holds
    /// nothing, so false. None for a stale handle, an unknown biome or a level that has no tag
    /// tables loaded. Reads no chunks.
    fn level_biome_in_tag(&self, level: LevelRef, biome: &str, tag: &str) -> Option<bool>;
    /// Side: both.
    /// A biome's climate by id (`minecraft:plains`), from the level's biome table, data packs
    /// included: temperature, downfall, whether it has precipitation and the frozen modifier
    /// (`Biome.ClimateSettings`). None for a stale handle or an unknown biome. Reads no chunks.
    fn level_biome_properties(&self, level: LevelRef, biome: &str) -> Option<BiomeProperties>;
    /// Side: server.
    /// The world's seed (`WorldOptions.seed`), as `/seed` shows it.
    fn level_seed(&self, level: LevelRef) -> Option<i64>;
    /// Side: server.
    /// The id of the chunk generator this level was made with, as `BuiltInRegistries.CHUNK_GENERATOR`
    /// names its codec (`level.getChunkSource().getGenerator()`): `minecraft:noise`,
    /// `minecraft:flat` (a superflat world, and a level a mod opened with layers or air),
    /// `minecraft:debug`, or the name a mod registered its generator under
    /// (`register_chunk_generator`). Reports the generator the server actually built, not
    /// necessarily the one named in a pack or saved stem: a flat stem takes precedence over
    /// the saved Debug preset. None for a stale level and on a client.
    fn level_generator_type(&self, level: LevelRef) -> Option<Identifier>;
}
/// The handles' writes of the `level_worldgen` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelWorldgenBackendMut {
    /// Side: server.
    /// Place the feature `feature` (a `minecraft:worldgen/feature` id, `minecraft:oak` or
    /// `mymod:ender_tree`) at `pos`, as `/place feature` does (`Feature.place` with the level's
    /// random and chunk generator): the blocks it sets reach players as any block change does.
    /// The bool is what the feature reported: `Ok(true)` when it says it placed, `Ok(false)` when
    /// it says it didn't (a tree with no room). A feature may set blocks before it reports
    /// `false`; they are kept, as `/place feature` keeps them. Refusals are checked before the
    /// feature runs, change nothing and are named by
    /// `PlaceFeatureRefusal`: `Stale`, `Client`, `NotInPlace` (only the level being ticked is
    /// written), `NotLoaded` (a chunk of the 3x3 around `pos` is not loaded, or `pos` is out of
    /// height) and `UnknownFeature`.
    fn level_place_feature(
        &mut self,
        level: LevelRef,
        feature: &str,
        pos: [i32; 3],
    ) -> Result<bool, PlaceFeatureRefusal>;
    /// Side: server.
    /// Place the placed feature `placed_feature` (a `minecraft:worldgen/placed_feature` id,
    /// `minecraft:oak_checked` or `mymod:ender_tree_checked`) at `pos`, as
    /// `PlacedFeature.place` does with the level's random and chunk generator:
    /// its placement modifiers run in order from `pos`, depth first, and its feature is placed at
    /// every position they give. Direct placement has no top feature: a position reaching a
    /// `minecraft:biome` modifier refuses with `PlaceFeatureRefusal::BiomeFilter`, rather than
    /// checking biome membership. If earlier modifiers yield no positions, it returns `Ok(false)`.
    /// Earlier modifiers consume random draws even when the filter refuses. This call's feature
    /// has not run when its filter refuses; nested placement may have written before an inner
    /// refusal. `Ok(true)` means any invocation reported success; `Ok(false)` means no positions
    /// remained or all invocations failed. Blocks set before failure are kept. Other refusals
    /// are checked before placement, as for `level_place_feature`, with `UnknownFeature` for an
    /// id the placed-feature registry lacks.
    fn level_place_placed_feature(
        &mut self,
        level: LevelRef,
        placed_feature: &str,
        pos: [i32; 3],
    ) -> Result<bool, PlaceFeatureRefusal>;
    /// Side: both.
    /// Set the biome of the 4x4x4 cell holding `pos` to `biome` (`minecraft:plains`), as
    /// `/fillbiome` does for one cell. False for a refused write, a stale or parked handle, a
    /// client, an unknown biome and an unchanged cell.
    /// Tentative reads see the new cells inside a transaction. Abort restores the cells
    /// without publishing. Saving, remeshing and resending wait for the outermost commit,
    /// once per column. Outside a transaction they run immediately.
    fn level_set_biome(&mut self, level: LevelRef, pos: [i32; 3], biome: &str) -> bool;
}
