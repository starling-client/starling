//! Server-side cauldron item interactions (`CauldronInteraction`).
use crate::{BlockKey, BlockState, Hand, ItemKey, ItemStack, LevelRef, PlayerRef, ServerHost};
use std::sync::Arc;

/// Immutable inputs captured for one selected server-side cauldron interaction.
// no wire: passed directly to in-process server cauldron callbacks.
// exhaustive: the engine builds one per use from its own state; mods only read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CauldronUse {
    /// Server level owning the target.
    pub level: LevelRef,
    /// Target block position.
    pub pos: [i32; 3],
    /// Complete target state before the action.
    pub state: BlockState,
    /// Acting player, scoped to the current world.
    pub player: PlayerRef,
    /// Hand used for the action.
    pub hand: Hand,
    /// Complete held stack before the action.
    pub held: ItemStack,
    /// Whether water covers the block above the target.
    pub underwater: bool,
    /// Whether creative exchange semantics apply.
    pub creative: bool,
}

/// Server-side interaction outcome; a consuming result stops subsequent normal item use.
// no wire: returned directly from in-process server cauldron callbacks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CauldronDecision {
    /// Let the block phase fall through to normal item-on-block use
    /// (`InteractionResult.TRY_WITH_EMPTY_HAND`). Air use remains a separate turn.
    #[default]
    Pass,
    /// Stop normal item use without swinging the player's arm (`InteractionResult.CONSUME`).
    Consume,
    /// Stop normal item use and swing the player's arm (`InteractionResult.SUCCESS`).
    Success,
}

/// A registration refused before the immutable dispatcher is published.
// no wire: returned directly from in-process cauldron registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CauldronRegistrationError {
    /// The item is absent from the prepared registry.
    UnknownItem,
    /// The block is absent from the prepared registry.
    UnknownBlock,
    /// The item tag is not a resource location (`c:buckets/milk`, with or without `#`).
    InvalidTag,
}

/// NeoForge's `RegisterCauldronInteractionEvent.Interaction` counterpart (`CauldronInteraction`).
/// Custom handlers run only on the server. A matching registration stops speculative item use on
/// Sunrise clients, even if the client would have skipped the block phase. If the server's block
/// phase passes or is skipped, the eligible local item-on-block phase runs once, even when the
/// server's item-on-block phase consumes. Air use and the other hand require both sides to pass.
/// The original slot, held stacks and ray (including eye position and reach) must still match.
/// Vanilla handlers still predict locally. Perform block, item, sound, event, and statistic
/// changes directly through `host`, using
/// [`HostTransaction::transaction`](crate::HostTransaction::transaction) when grouping storage
/// mutations.
pub trait Interaction: Send + Sync + 'static {
    /// Run the server-side cauldron interaction for `input`, returning whether normal item use
    /// should continue (`Pass`), stop quietly (`Consume`), or stop and swing (`Success`).
    fn interact(&self, host: &mut dyn ServerHost, input: &CauldronUse) -> CauldronDecision;
}

/// Prepared dispatcher registration, corresponding to NeoForge's `RegisterCauldronInteractionEvent`.
pub trait CauldronInteractions {
    /// Register an exact item entry for a block's dispatcher, overwriting any existing vanilla
    /// or mod entry in place (matching `CauldronInteraction.Dispatcher.put`).
    /// Admission errors (`UnknownBlock`, `UnknownItem`) are returned immediately and stored:
    /// ignoring an error still fails the whole preparation when it finishes, leaving the
    /// published snapshot unchanged.
    fn register(
        &mut self,
        block: BlockKey,
        item: ItemKey,
        handler: Arc<dyn Interaction>,
    ) -> Result<(), CauldronRegistrationError>;
    /// Queue registration over the complete prepared dispatcher set, including custom blocks
    /// declared later, applied in registration order alongside direct [`Self::register`] calls
    /// so later writes win and overwrite any existing entry for `item` on each block in place.
    /// Item admission errors (`UnknownItem`) are returned and stored.
    fn register_to_all(
        &mut self,
        item: ItemKey,
        handler: Arc<dyn Interaction>,
    ) -> Result<(), CauldronRegistrationError>;
    /// Register an item-tag entry for a block's dispatcher (NeoForge's `register(id, TagKey, …)`):
    /// it answers for every item the tag holds when the item is used, so data-pack changes
    /// apply after a reload. Tag entries are checked before exact item entries (matching
    /// `CauldronInteraction.Dispatcher.get`), in registration order, with vanilla's dye tag
    /// (`minecraft:cauldron_can_remove_dye`) checked first on `minecraft:water_cauldron`.
    /// Registering the same tag again on one block overwrites its handler in place at its
    /// original position, including replacing vanilla's dye tag handler.
    fn register_tag(
        &mut self,
        block: BlockKey,
        tag: &str,
        handler: Arc<dyn Interaction>,
    ) -> Result<(), CauldronRegistrationError>;
    /// [`Self::register_tag`] over the complete prepared dispatcher set, applied in registration
    /// order alongside direct [`Self::register_tag`] calls (NeoForge's
    /// `registerToAll(TagKey, …)`), overwriting any matching tag entry on each block in place.
    fn register_tag_to_all(
        &mut self,
        tag: &str,
        handler: Arc<dyn Interaction>,
    ) -> Result<(), CauldronRegistrationError>;
}

/// Current registration owner for engine diagnostics.
pub fn cauldron_owner() -> &'static str {
    crate::content::crash::current_native_mod().unwrap_or("unknown")
}
