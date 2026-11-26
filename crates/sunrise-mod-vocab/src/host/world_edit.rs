//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' writes of the `world_edit` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait WorldEditBackendMut {
    /// Side: server.
    /// Applies the runs of `edit` at once straight into the chunks of this server level (active
    /// or parked), with what `fill ... strict` does for block entities (a block replaced by
    /// another loses its block entity, and a new block gets the one `set_block` would give it, a
    /// mod block's included); no neighbour is told, and changed chunks go to players whole. Runs
    /// in unloaded chunks and parts of runs outside the column's height are skipped. Returns the
    /// number of blocks changed, or a typed [`BlockEditError`] with nothing changed.
    /// The new blocks are visible immediately. If this level has an open transaction, strict
    /// and ordinary writes replay in call order at the level's outermost commit; an abort restores
    /// the blocks, resident block entities and connection groups without broadcasting changes.
    /// Without an open transaction in this level, the edit commits at once, even if another
    /// level has a transaction open. Strict writes never spill container contents.
    fn level_edit_blocks(
        &mut self,
        level: LevelRef,
        edit: &BlockEdit,
    ) -> Result<u64, BlockEditError>;
    /// Side: server.
    /// Reads structure template `id` (`mymod:hut`, found as `StructureTemplateManager.get` finds
    /// it: the world's saved structures, then the game's and the mods' own) as `/place template`
    /// would place it at `at` with `rotation` and `mirror` in this server level (active or
    /// parked), without placing anything. Lends each transformed block as a
    /// [`StructureTemplateBlockView`] in vanilla placement order (full blocks, the others, then
    /// those with a block entity, each by y, x, z), from the palette `/place template` picks at
    /// `at`, stopping early on [`core::ops::ControlFlow::Break`], and returns the rotated
    /// template size. No processor runs and template entities are omitted. Refuses before
    /// visiting with a typed [`StructureTemplateError`].
    /// API @API:structure_template@.
    fn level_visit_structure_template(
        &mut self,
        level: LevelRef,
        id: &str,
        at: [i32; 3],
        rotation: StructureRotation,
        mirror: StructureMirror,
        visit: &mut dyn for<'a> FnMut(StructureTemplateBlockView<'a>) -> core::ops::ControlFlow<()>,
    ) -> Result<[i32; 3], StructureTemplateError>;
    /// Side: server.
    /// Places structure template `source` at `at` in this server level (active or parked) the
    /// way `/place template` and a structure block's load place one, with `rotation`, `mirror`
    /// and `pivot`, the level position the turn and mirroring go around (`at`, which
    /// `/place template` uses, turns the template about its own origin). Blocks, block-entity
    /// data and the template's own entities go through vanilla placement, so states are mirrored
    /// then turned, block entities keep their data and get the loot seed a placement gives them,
    /// and entities are spawned as the game spawns them: no processor runs, nothing is `strict`,
    /// and neighbours of a replaced block are not told. A template read from structure NBT is
    /// read by the game's own loader within vanilla's limits (`NbtAccounter.unlimitedHeap`,
    /// gzipped or plain). Refuses with nothing placed via a typed [`StructureTemplateError`].
    /// API @API:structure_template@.
    fn level_place_template(
        &mut self,
        level: LevelRef,
        source: &StructureTemplateSource,
        at: [i32; 3],
        pivot: [i32; 3],
        rotation: StructureRotation,
        mirror: StructureMirror,
    ) -> Result<(), StructureTemplateError>;
}
