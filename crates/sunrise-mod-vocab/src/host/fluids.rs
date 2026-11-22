//! Reads of the current registered cauldron fluid contents and of the open world's fluid
//! interactions.
#![allow(unused_variables)]
use crate::{BlockKey, BlockState, CauldronContent, FluidKey, FluidState};
/// The host's current cauldron content table, including mod registrations.
#[sunrise_mod_api_macros::host_area]
pub trait FluidsHost {
    /// Side: both.
    /// Contents registered for `block`, or none if absent or no table has been built.
    /// Reads the current resource generation, including after reload.
    fn cauldron_content_for_block(&self, block: BlockKey) -> Option<CauldronContent>;
    /// Side: both.
    /// Contents registered for `fluid`, or none if absent or no table has been built.
    fn cauldron_content_for_fluid(&self, fluid: FluidKey) -> Option<CauldronContent>;
    /// Side: both.
    /// Current level of `state`, or none without registered contents or a built table.
    /// Always-full contents answer their maximum level.
    fn cauldron_level(&self, state: BlockState) -> Option<u32>;
}
/// The server host's methods of the `fluids` area, implemented by the game and reached through [`ServerHost`].
#[sunrise_mod_api_macros::host_area]
pub trait FluidsServerHost {
    /// Side: server.
    /// What a liquid cell of `source` becomes beside one cell (`neighbour`, a block state whose
    /// fluid is read through it, so a waterlogged block holds water) with `below` under it:
    /// the open world's fluid interaction table (vanilla's lava rules, then every mod's
    /// entries, as `LiquidBlock.shouldSpreadLiquid` and NeoForge's `FluidInteractionRegistry`
    /// walk it) for that one neighbour, with the same exact-tag and family matching, a source
    /// cell's result for a source and the flowing result otherwise. `None` is no interaction,
    /// and for a fluid with no entries or one that has no world form. A pure read: it changes
    /// no world, fires no `fluid_place_block` and plays no fizz. A liquid checks its five
    /// neighbours in vanilla's order, above, north, south, west, east (never below), and the
    /// first that answers wins: loop over them in that order to ask the same.
    fn fluid_interaction_result(
        &self,
        source: FluidState,
        neighbour: BlockState,
        below: BlockState,
    ) -> Option<BlockState>;
}
