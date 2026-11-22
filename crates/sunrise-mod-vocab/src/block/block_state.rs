//! Process-local block values, independent of the game's numeric registry.
use crate::{BlockKey, BlockMetadata, BlockStateError, StateConstraint};

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
    /// Borrow the serialized property value from the supplied immutable metadata. For a known
    /// state with no such property, return `Ok(None)`.
    pub fn property<'a>(
        self,
        metadata: &'a dyn BlockMetadata,
        name: &str,
    ) -> Result<Option<&'a str>, BlockStateError> {
        metadata.property(self, name)
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
