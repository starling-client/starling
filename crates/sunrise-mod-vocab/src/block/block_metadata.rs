//! The read-only metadata resource behind block value operations.
use super::block_state::block_state_from_ordinal;
use crate::{BlockKey, BlockState, StateConstraint, StateProperties};

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
impl std::fmt::Display for BlockStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("block metadata unavailable"),
            Self::UnknownBlock => f.write_str("unknown block"),
            Self::UnknownState => f.write_str("unknown block state"),
            Self::UnknownProperty(index) => {
                write!(f, "unknown block property at constraint {index}")
            }
            Self::InvalidValue(index) => {
                write!(f, "invalid block property value at constraint {index}")
            }
            Self::DuplicateProperty(index) => {
                write!(f, "duplicate block property at constraint {index}")
            }
            Self::InvalidCombination => f.write_str("unregistered block state combination"),
        }
    }
}
impl std::error::Error for BlockStateError {}
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

/// The block registry's read-only metadata: names, keys, default states and state definitions,
/// as vanilla reads `BuiltInRegistries.BLOCK`, `Block.defaultBlockState()` and `StateDefinition`
/// from any thread once the registry is frozen (NeoForge and Fabric mods call the same vanilla
/// methods). A host's block-states service lends it (`host.block_states()`), a chunk generator's
/// worker gets it from `ChunkAccess::metadata`, a tint worker as an argument, and
/// [`BlockStates::metadata`](crate::BlockStates::metadata) hands out an owned copy any thread can
/// keep. This contract takes and returns [`BlockState`] directly; mods normally use the methods on
/// `BlockState`.
// hidden: low-level shared resource contract; BlockState methods are the mod-facing operations.
#[doc(hidden)]
pub trait BlockMetadata {
    /// Borrow the canonical block name from this context's retained registry.
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError>;
    /// The key of the block registered as `id` (`BuiltInRegistries.BLOCK.getValue`), from the
    /// completed registry, mod blocks included. `id` is the full identifier,
    /// `minecraft:deepslate_bricks`: a bare path is not completed with `minecraft:`. None for an
    /// id no block has, and where no metadata is available.
    fn key(&self, id: &str) -> Option<BlockKey>;
    /// The default state of `block` (`Block.defaultBlockState()`), the one it takes when nothing
    /// else says.
    fn default_state(&self, block: BlockKey) -> Result<BlockState, BlockStateError>;
    /// The properties of `block` and their legal values (`StateDefinition.getProperties`,
    /// `Property.getPossibleValues`), in property-name order, borrowed from this resource. A
    /// block without properties has an empty definition.
    fn state_definition(&self, block: BlockKey) -> Result<StateProperties<'_>, BlockStateError>;
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
    /// Visit each property of `state` with its serialized value (names and values as `property`
    /// reads them), in name order as vanilla's `StateDefinition` keeps them. A block with no
    /// properties visits nothing.
    fn visit_properties(
        &self,
        state: BlockState,
        visit: &mut dyn FnMut(&str, &str),
    ) -> Result<(), BlockStateError>;
}

impl BlockMetadata for sunrise_mod_runtime::block_definitions::Blocks {
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
        self.names
            .entries
            .get(block.0 as usize)
            .map(|entry| entry.id.as_str())
            .ok_or(BlockStateError::UnknownBlock)
    }
    fn key(&self, id: &str) -> Option<BlockKey> {
        self.names.slots.get(id).map(|slot| BlockKey(*slot))
    }
    fn default_state(&self, block: BlockKey) -> Result<BlockState, BlockStateError> {
        let definition = self
            .definitions
            .get(block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?;
        Ok(block_state_from_ordinal(block, definition.default_state()))
    }
    fn state_definition(&self, block: BlockKey) -> Result<StateProperties<'_>, BlockStateError> {
        self.definitions
            .get(block.0 as usize)
            .map(|definition| StateProperties::of(definition.properties()))
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
    fn visit_properties(
        &self,
        state: BlockState,
        visit: &mut dyn FnMut(&str, &str),
    ) -> Result<(), BlockStateError> {
        self.definitions
            .get(state.block().0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?
            .visit_properties(state.ordinal(), visit)
            .map_err(Into::into)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use sunrise_mod_runtime::block_definitions::{BlockDefinition, Blocks};
    use sunrise_mod_runtime::registry_keys::Names;

    /// Two blocks of this test's own, as a completed registry publishes them: slots in sorted
    /// id order, `minecraft:oak_slab` with vanilla's six states, its default the fourth.
    fn blocks() -> Blocks {
        let names = Names::default()
            .complete(["minecraft:stone", "minecraft:oak_slab"], |_, id| {
                id.to_owned()
            })
            .unwrap();
        let slab: Vec<BTreeMap<String, String>> = ["top", "bottom", "double"]
            .into_iter()
            .flat_map(|kind| {
                ["true", "false"].map(|waterlogged| {
                    BTreeMap::from([
                        ("type".to_owned(), kind.to_owned()),
                        ("waterlogged".to_owned(), waterlogged.to_owned()),
                    ])
                })
            })
            .collect();
        let stone = BTreeMap::new();
        Blocks::prepare(names, |id| {
            let (states, default) = match id {
                "minecraft:oak_slab" => (slab.iter().collect::<Vec<_>>(), 3),
                _ => (vec![&stone], 0),
            };
            BlockDefinition::new(&states, default).map_err(|error| format!("{error:?}"))
        })
        .unwrap()
    }

    #[test]
    fn an_owned_snapshot_resolves_ids_and_reads_definitions_on_another_thread() {
        let metadata: Arc<dyn BlockMetadata + Send + Sync> = Arc::new(blocks());
        std::thread::spawn(move || {
            assert_eq!(metadata.key("minecraft:oak_slab"), Some(BlockKey(0)));
            assert_eq!(metadata.key("minecraft:stone"), Some(BlockKey(1)));
            assert_eq!(metadata.key("stone"), None);
            assert_eq!(metadata.key("minecraft:dirt"), None);

            let slab = metadata.default_state(BlockKey(0)).unwrap();
            assert_eq!(slab.ordinal(), 3);
            assert_eq!(metadata.property(slab, "type"), Ok(Some("bottom")));
            assert_eq!(metadata.property(slab, "waterlogged"), Ok(Some("false")));
            let properties: Vec<_> = metadata
                .state_definition(BlockKey(0))
                .unwrap()
                .iter()
                .map(|property| (property.name, property.values.to_vec()))
                .collect();
            assert_eq!(
                properties,
                [
                    (
                        "type",
                        vec!["top".to_owned(), "bottom".into(), "double".into()]
                    ),
                    ("waterlogged", vec!["true".to_owned(), "false".into()]),
                ]
            );

            assert_eq!(metadata.default_state(BlockKey(1)).unwrap().ordinal(), 0);
            assert!(metadata.state_definition(BlockKey(1)).unwrap().is_empty());
            assert_eq!(
                metadata.default_state(BlockKey(2)),
                Err(BlockStateError::UnknownBlock)
            );
            assert_eq!(
                metadata.state_definition(BlockKey(2)).map(|p| p.len()),
                Err(BlockStateError::UnknownBlock)
            );
        })
        .join()
        .unwrap();
    }
}
