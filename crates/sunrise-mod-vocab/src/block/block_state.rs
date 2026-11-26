//! Process-local block values, independent of the game's numeric registry.
use crate::{
    BlockKey, BlockMetadata, BlockStateError, BlockStates, FluidState, StateConstraint, TagKey,
    Tags,
};

/// A copyable block key and local state ordinal. Persist canonical names and properties, not this value.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BlockState {
    block: BlockKey,
    state: u16,
}
impl BlockState {
    /// The block this state belongs to, without needing a host or metadata lookup.
    pub const fn block(self) -> BlockKey {
        self.block
    }
    /// Its process-local ordinal, not the game's numeric state identifier.
    pub const fn ordinal(self) -> u16 {
        self.state
    }
    /// Whether this state's block is in `tag` (`BlockBehaviour.BlockStateBase.is(TagKey)`),
    /// delegating through [`Self::block`]. An unknown tag is `Some(false)`; unavailable tags
    /// or mappings return `None`.
    pub fn is_in_tag(self, tags: &dyn Tags, tag: TagKey<BlockKey>) -> Option<bool> {
        self.block().is_in_tag(tags, tag)
    }
    /// Borrow this state's outline boxes (`BlockBehaviour.BlockStateBase.getShape` with
    /// `EmptyBlockGetter.INSTANCE` at `BlockPos.ZERO`). Each box is `[min x, y, z, max x, y, z]`
    /// in block-local space, without a random offset or world context. The slice is empty for
    /// air and other states without an outline. An unknown state or unavailable tables give `None`.
    pub fn outline<'a>(self, states: &BlockStates<'a>) -> Option<&'a [[f64; 6]]> {
        states.backend().block_state_outline(self).ok().flatten()
    }
    /// The fluid state this state holds (`BlockBehaviour.BlockStateBase.getFluidState`): water
    /// for a waterlogged state, the empty fluid for most. `None` for an unknown state or
    /// unavailable tables.
    pub fn fluid_state(self, states: &BlockStates<'_>) -> Option<FluidState> {
        states
            .backend()
            .block_state_fluid_state(self)
            .ok()
            .flatten()
    }
    /// Borrow the serialized property value from the supplied immutable metadata. For a known
    /// state with no such property, return `Ok(None)`.
    pub fn property<'a>(
        self,
        metadata: &'a dyn BlockMetadata,
        name: &str,
    ) -> Result<Option<&'a str>, BlockStateError> {
        metadata.property(self, name)
    }
    /// Visit each of this state's properties with its serialized value, in name order (as
    /// vanilla's `StateDefinition` keeps them), from the supplied immutable metadata: what a
    /// colour provider or an adapter forwards as the state's whole property set. A block with no
    /// properties visits nothing.
    pub fn visit_properties(
        self,
        metadata: &dyn BlockMetadata,
        visit: &mut dyn FnMut(&str, &str),
    ) -> Result<(), BlockStateError> {
        metadata.visit_properties(self, visit)
    }
    /// Return a new state with one property changed and the other properties retained. This does
    /// not change the original state, the metadata or any block in the world.
    pub fn with(
        self,
        metadata: &dyn BlockMetadata,
        name: &str,
        value: &str,
    ) -> Result<Self, BlockStateError> {
        metadata.with(self, name, value)
    }
    /// Construct a state by applying partial property constraints to the block's default from
    /// `metadata`. Omitted properties keep their defaults; invalid constraints return an error.
    pub fn of(
        metadata: &dyn BlockMetadata,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<Self, BlockStateError> {
        metadata.of(block, properties)
    }
}
/// Construct a state without looking up the key or ordinal in block metadata.
// hidden: the game has the validated ordinal; mods construct states through BlockMetadata.
#[doc(hidden)]
pub const fn block_state_from_ordinal(block: BlockKey, ordinal: u16) -> BlockState {
    BlockState {
        block,
        state: ordinal,
    }
}
