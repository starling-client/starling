//! The read-only metadata resource behind block value operations.
use super::block_state::block_state_from_ordinal;
use crate::{BlockKey, BlockState, StateConstraint};

/// A refused metadata read or state operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockStateError {
    /// Metadata is unavailable in this context.
    Unavailable,
    /// The context does not contain the block key.
    UnknownBlock,
    /// The ordinal is outside the block definition.
    UnknownState,
    /// Constraint index naming a missing property.
    UnknownProperty(u32),
    /// Constraint index naming an invalid value.
    InvalidValue(u32),
    /// Constraint index repeating a property.
    DuplicateProperty(u32),
    /// The requested combination is not a registered state.
    InvalidCombination,
}
impl crate::NoBackend for BlockStateError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
impl From<sunrise_mod_runtime::block_definitions::StateError> for BlockStateError {
    fn from(error: sunrise_mod_runtime::block_definitions::StateError) -> Self {
        use sunrise_mod_runtime::block_definitions::StateError;
        match error {
            StateError::UnknownState => Self::UnknownState,
            StateError::UnknownProperty(index) => Self::UnknownProperty(index as u32),
            StateError::InvalidValue(index) => Self::InvalidValue(index as u32),
            StateError::DuplicateProperty(index) => Self::DuplicateProperty(index as u32),
            StateError::InvalidCombination => Self::InvalidCombination,
        }
    }
}

/// Read-only metadata, lent by a host's block-states service or a tint worker. This contract takes
/// and returns [`BlockState`] directly; mods normally use the methods on `BlockState`.
// hidden: low-level shared resource contract; BlockState methods are the mod-facing operations.
#[doc(hidden)]
pub trait BlockMetadata {
    /// Borrow the canonical block name from this context's retained registry.
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError>;
    /// Borrow the serialized value of `name` on `state` from this resource, or `Ok(None)` if the
    /// property is absent from a known state.
    fn property(&self, state: BlockState, name: &str) -> Result<Option<&str>, BlockStateError>;
    /// Return a `BlockState` with one property changed and all others retained. This does not
    /// mutate `state` or write to the world.
    fn with(
        &self,
        state: BlockState,
        name: &str,
        value: &str,
    ) -> Result<BlockState, BlockStateError>;
    /// Return a `BlockState` by applying partial constraints to the block's default state.
    fn of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockState, BlockStateError>;
}

impl BlockMetadata for sunrise_mod_runtime::block_definitions::Blocks {
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
        self.names
            .entries
            .get(block.0 as usize)
            .map(|entry| entry.id.as_str())
            .ok_or(BlockStateError::UnknownBlock)
    }
    fn property(&self, state: BlockState, name: &str) -> Result<Option<&str>, BlockStateError> {
        self.definitions
            .get(state.block().0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?
            .property(state.ordinal(), name)
            .map_err(Into::into)
    }
    fn with(
        &self,
        state: BlockState,
        name: &str,
        value: &str,
    ) -> Result<BlockState, BlockStateError> {
        let block = state.block();
        let ordinal = self
            .definitions
            .get(block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?
            .with(state.ordinal(), name, value)?;
        Ok(block_state_from_ordinal(block, ordinal))
    }
    fn of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockState, BlockStateError> {
        let definition = self
            .definitions
            .get(block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?;
        let constraints: Vec<_> = properties
            .iter()
            .map(|p| (p.name.as_str(), p.value.as_str()))
            .collect();
        let ordinal = definition.of(&constraints)?;
        Ok(block_state_from_ordinal(block, ordinal))
    }
}
