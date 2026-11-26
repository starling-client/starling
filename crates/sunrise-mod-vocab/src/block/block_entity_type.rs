//! Typed identities and declarations of block entity types.
use crate::BlockEntityTypeKey;
use std::{fmt, sync::Arc};

/// A block entity `LevelRef::block_entities` found: its block position and its type
/// (`BlockEntity.getType`), not its block's ID nor its saved data.
// no wire: an in-process host read's result, never sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BlockEntityAt {
    /// The block entity's block position.
    pub pos: [i32; 3],
    /// Its type's key; `kind.id(host)` names it, as `minecraft:chest`.
    pub kind: BlockEntityTypeKey,
}

impl BlockEntityAt {
    /// The block entity of type `kind` at `pos`.
    pub fn new(pos: [i32; 3], kind: BlockEntityTypeKey) -> Self {
        Self { pos, kind }
    }
}

/// The mirror followed by rotation applied during structure placement.
// no wire: transform settings are passed only to the local placement callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StructureTransform {
    /// Rotation after mirroring.
    pub rotation: crate::StructureRotation,
    /// Mirror before rotation.
    pub mirror: crate::StructureMirror,
}

impl StructureTransform {
    /// Apply `mirror` first and then `rotation`, as template placement does.
    pub fn new(rotation: crate::StructureRotation, mirror: crate::StructureMirror) -> Self {
        Self { rotation, mirror }
    }
}

/// A deterministic transform could not finish editing the saved data.
// no wire: refusal diagnostics are reported at the local placement call site.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TransformRefusal {
    /// Diagnostic explaining the refusal.
    pub reason: String,
}

impl TransformRefusal {
    /// Decline a transform with a diagnostic.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}
impl fmt::Display for TransformRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(f)
    }
}
impl std::error::Error for TransformRefusal {}
impl From<(crate::Identifier, String)> for TransformRefusal {
    fn from((id, reason): (crate::Identifier, String)) -> Self {
        Self::new(format!("{id}: {reason}"))
    }
}

/// Edits the working saved data for one block in a structure placement, not the reusable template.
/// Runs on the server, possibly on a worldgen worker, after the block state is placed and any
/// loot seed is set, before the block entity loads the data. The engine preserves the original
/// `id` and any original `x`, `y`, `z` fields. Disk and network limits apply only at serialization.
///
/// Validate before editing if you need to leave the data unchanged on refusal: a refusal is
/// reported every time but does not undo edits. A panic is reported and disables this callback
/// for the session; edits made before the panic are not undone either. Mods must ensure
/// determinism, including when a matching `Send + Sync` closure is used.
pub trait BlockEntityTransform: Send + Sync + 'static {
    /// Edit the working compound directly. No Host or world handle is supplied.
    fn transform(
        &self,
        data: &mut crate::Compound,
        transform: StructureTransform,
    ) -> Result<(), TransformRefusal>;
}
impl<F> BlockEntityTransform for F
where
    F: Fn(&mut crate::Compound, StructureTransform) -> Result<(), TransformRefusal>
        + Send
        + Sync
        + 'static,
{
    fn transform(
        &self,
        data: &mut crate::Compound,
        transform: StructureTransform,
    ) -> Result<(), TransformRefusal> {
        self(data, transform)
    }
}

/// A native Part cannot be attached to this type during registration.
///
/// This is a process-local registration error, not a `Wire` type.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::BlockEntityPartError>();
/// ```
// no wire: local block-entity Part registration error.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockEntityPartError {
    /// The key was not issued by this installation's registration.
    InvalidKey {
        /// The invalid block entity type key.
        kind: BlockEntityTypeKey,
    },
    /// Vanilla entities do not expose a native Part's payload.
    VanillaType {
        /// The block entity type that was refused.
        kind: BlockEntityTypeKey,
    },
    /// This Part type is already attached to the entity type.
    Duplicate {
        /// The block entity type that was refused.
        kind: BlockEntityTypeKey,
        /// The Part type that is already attached.
        part: &'static str,
    },
}

impl fmt::Display for BlockEntityPartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey { .. } => f.write_str("invalid block entity type key"),
            Self::VanillaType { kind } => write!(
                f,
                "cannot attach a native block entity Part to vanilla type {}",
                crate::content::registry_keys::block_entity_type_display_name(*kind).as_str()
            ),
            Self::Duplicate { kind, part } => {
                write!(
                    f,
                    "Part {part} is attached to type {} twice",
                    crate::content::registry_keys::block_entity_type_display_name(*kind).as_str()
                )
            }
        }
    }
}

impl std::error::Error for BlockEntityPartError {}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// Builds one mod block entity type during registration.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockEntityTypeBuilder<'a> {
    pub(crate) registrar: &'a mut dyn crate::Registrar,
    pub(crate) id: String,
    pub(crate) blocks: Vec<String>,
    pub(crate) transform: Option<Arc<dyn BlockEntityTransform>>,
    pub(crate) ticker: Option<Arc<crate::parts::BlockEntityTickerSelector>>,
}

impl BlockEntityTypeBuilder<'_> {
    /// The blocks which may contain this type. A bare name is this mod's own, as the type's
    /// id is, so `.blocks(&["press"])` names `<mod id>:press`; the block may be declared later.
    pub fn blocks(mut self, blocks: &[&str]) -> Self {
        self.blocks = blocks.iter().map(|block| (*block).to_owned()).collect();
        self
    }

    /// Set the optional [`BlockEntityTransform`] that edits data during structure placement.
    pub fn transform(mut self, step: impl BlockEntityTransform) -> Self {
        self.transform = Some(Arc::new(step));
        self
    }

    /// Select the states and logical sides of this type that may run their attached Parts' tick methods.
    /// The game calls this at state registration for accepted blocks only, and only on a side with
    /// a ticking Part. Without a selector, every accepted state on that side has the ticker.
    pub fn ticker_when(
        mut self,
        select: impl Fn(&dyn crate::BlockMetadata, crate::BlockState, crate::LogicalSide) -> bool
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.ticker = Some(Arc::new(select));
        self
    }

    /// Register the type and return its typed key.
    pub fn build(self) -> BlockEntityTypeKey {
        let key = self
            .registrar
            .declare_block_entity_type(&self.id, self.blocks);
        if let Some(step) = self.transform {
            self.registrar.set_block_entity_transform(key, step);
        }
        if let Some(select) = self.ticker {
            self.registrar.set_block_entity_ticker_selector(key, select);
        }
        key
    }
}

impl crate::BlockEntityTypeAddBlocksEvent<'_> {
    /// Stage valid blocks for one registered type, leaving its factory unchanged.
    /// The game checks the type and blocks when it freezes the validity table.
    pub fn modify(&mut self, kind: BlockEntityTypeKey, blocks: &[crate::BlockKey]) {
        self.additions
            .extend(blocks.iter().map(|&block| (kind, block)));
    }

    /// Stage valid blocks for every registered type accepted by `predicate`, in `types` order.
    /// The game checks the additions when it freezes the validity table.
    pub fn modify_matching(
        &mut self,
        mut predicate: impl FnMut(BlockEntityTypeKey) -> bool,
        blocks: &[crate::BlockKey],
    ) {
        for &kind in self.types {
            if predicate(kind) {
                self.additions
                    .extend(blocks.iter().map(|&block| (kind, block)));
            }
        }
    }
}
