//! The block-state definition types a mod registers (`register_block_state_models`, NeoForge's
//! `RegisterBlockStateModels`) and the picks their providers answer.
use crate::{Identifier, RegisterBlockStateModelsEvent, Tag};
use std::{collections::BTreeMap, sync::Arc};

/// One answer a block-state definition provider gives for a blockstate file naming it
/// (`register_block_state_models`): the model one of the block's states draws, or a refusal of
/// the whole file. A state no `Model` names inherits the lower pack layer's selection
/// when present; otherwise it draws no model.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods construct picks and the game matches every kind
// no wire: produced in-process by a registered native block-state definition provider.
pub enum BlockStateModelPick {
    /// One state draws one model, as a blockstate variant names it.
    Model {
        /// The state: an index into the input's `states`.
        state: u32,
        /// The model (`mymod:block/lamp_on`).
        model: String,
        /// Quarter turns about X, Y and Z (a variant's `x`, `y` and `z` over 90), taken modulo
        /// four.
        turns: [u8; 3],
        /// Whether the textures stay still as the model turns (a variant's `uvlock`).
        uvlock: bool,
    },
    /// The file doesn't read as this type: it is refused whole, as a vanilla definition that
    /// doesn't read is.
    Refuse {
        /// Why, logged with the file's name.
        reason: String,
    },
}

/// Borrowed input passed to a registered block-state definition provider when a blockstate file is
/// read.
// no wire: borrowed block-state definition instantiation input.
#[non_exhaustive]
pub struct BlockStateDefinitionInput<'a> {
    /// The block the blockstate file belongs to.
    pub block: &'a Identifier,
    /// The file's whole JSON object converted to a `Tag`.
    pub config: &'a Tag,
    /// The block's states in registry order, each as its property names and values.
    pub states: &'a [BTreeMap<String, String>],
}

impl<'a> BlockStateDefinitionInput<'a> {
    // hidden: the game supplies this read-only callback input
    #[doc(hidden)]
    pub fn new(
        block: &'a Identifier,
        config: &'a Tag,
        states: &'a [BTreeMap<String, String>],
    ) -> Self {
        Self {
            block,
            config,
            states,
        }
    }
}

/// A retained block-state definition provider callback bound at `register_block_state_models`.
// no wire: retained process-local block-state definition provider callback.
pub type BlockStateDefinitionFn = dyn Fn(&BlockStateDefinitionInput<'_>) -> Result<Vec<BlockStateModelPick>, String>
    + Send
    + Sync;

impl RegisterBlockStateModelsEvent<'_> {
    /// Register a block-state definition provider under `id` (NeoForge's
    /// `RegisterBlockStateModels::registerDefinition`).
    ///
    /// Any valid `<namespace>:<path>` identifier is accepted; a later registration for the same ID
    /// replaces the earlier one (last-writer-wins, matching NeoForge's `LateBoundIdMapper`).
    /// The provider runs once per matching blockstate file on each resource load and reload, never
    /// per frame.
    pub fn register<F>(&mut self, id: Identifier, provider: F)
    where
        F: Fn(&BlockStateDefinitionInput<'_>) -> Result<Vec<BlockStateModelPick>, String>
            + Send
            + Sync
            + 'static,
    {
        self.definitions.push(BlockStateDefinitionRegistration {
            id,
            instantiate: Arc::new(provider),
        });
    }
}

/// A staged block-state definition provider; dispatch attributes it to the registering mod.
// no wire: retained process-local block-state definition provider callback.
// hidden: retained registration staging belongs to the game
#[doc(hidden)]
#[non_exhaustive]
pub struct BlockStateDefinitionRegistration {
    /// The block-state definition type identifier.
    pub id: Identifier,
    /// The provider callback invoked when a blockstate file names this definition type.
    pub instantiate: Arc<BlockStateDefinitionFn>,
}
