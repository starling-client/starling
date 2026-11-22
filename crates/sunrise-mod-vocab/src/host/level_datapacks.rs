//! The `level_datapacks` area: data-driven lookups: datapack registry entries, recipe matching and loot tables. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_datapacks` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelDatapacksBackend {
    /// Side: both.
    /// The owned entry IDs (`addon:fire`) of a datapack registry, sorted by ID, as this world
    /// last loaded them. On the logical server, both the active dimension and any loaded parked
    /// server dimension are accepted (matching vanilla `Level.registryAccess()`); stale epochs
    /// and unloaded dimensions return `None`. On a client these are the entries the server sent
    /// at join and after each reload; a declared registry omitted from the snapshot is empty.
    /// An empty list means the registry has no entries. `None` means the level or registry is
    /// unavailable. Reads nothing from disk.
    fn level_datapack_entries(&self, level: LevelRef, registry: &str) -> Option<Vec<String>>;
    /// Side: both.
    /// Borrow the published entry `entry` of `registry` (on a client, from the snapshot its
    /// server last sent). The active set stays pinned for the duration of `visit`, even if its
    /// publication is cleared or replaced. No registry lock is held while `visit` runs.
    ///
    /// `visit` runs exactly once on `Ok(())`. Read `entry.values` in schema order or use
    /// [`DatapackEntry::get`] by field name. The borrow ends when `visit` returns; retain IDs
    /// and look them up again, or clone only the fields you need as an owned snapshot.
    ///
    /// On the logical server, both the active dimension and any loaded parked server dimension
    /// are accepted, matching vanilla `Level.registryAccess()`; stale epochs and unloaded
    /// dimensions are refused. An unavailable set, unknown or unavailable registry, or stale or
    /// unloaded level returns [`DatapackReadError::UnknownRegistry`]; a missing entry in the
    /// registry returns [`DatapackReadError::MissingEntry`]. A host without a backend returns
    /// [`DatapackReadError::Unavailable`]. No error calls `visit`.
    fn level_datapack_read(
        &self,
        level: LevelRef,
        registry: &str,
        entry: &str,
        visit: &mut dyn FnMut(&DatapackEntry),
    ) -> Result<(), DatapackReadError>;
    /// Side: server.
    /// Match and assemble through the engine's recipe matcher, without consuming inputs.
    /// Covers minecraft:crafting, smelting, blasting, smoking, campfire_cooking,
    /// stonecutting and smithing, including data-pack recipes of those types.
    /// Registered crafting serializers match through minecraft:crafting. Read-only and unregistered
    /// mod-owned types return RecipeLookupError::NotMatchable; cache recipes_of_type once
    /// per load/reload and match those in mod memory (no per-tick host crossing).
    /// A valid input without a match returns Ok(None); invalid shapes/items and unavailable
    /// levels are typed refusals. Crafting inputs carry row-major slots matching `width * height`;
    /// a zero area has no slots and is still offered to registered crafting serializers.

    fn level_recipe_for(
        &self,
        level: LevelRef,
        recipe_type: &Identifier,
        inputs: &RecipeInput,
    ) -> Result<Option<RecipeMatch>, RecipeLookupError>;
}
/// The handles' writes of the `level_datapacks` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelDatapacksBackendMut {
    /// Side: server.
    /// One roll of the loot table `table` (`mymod:sifting/gravel`, the world's data packs
    /// merged), handed back instead of dropped: `LootTable.getRandomItems(LootParams)` with
    /// `origin` as ORIGIN, `tool` as TOOL when given and `entity` as THIS_ENTITY when given,
    /// the context `/loot … fish <table> <pos> <tool>` gives. The roll draws from the table's
    /// random sequence, else the level random, as vanilla's does, so it moves later draws: that
    /// is why this is a write. The stacks come split to their stack sizes, empties left out.
    /// Mods' loot kinds see `origin` and this level; no `loot_drops` event, since nothing
    /// drops. None for a table that isn't there, a stale or parked handle, a client, and a
    /// stale `entity`.
    fn level_roll_loot(
        &mut self,
        level: LevelRef,
        table: &str,
        origin: [f64; 3],
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
}
