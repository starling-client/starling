//! Plain-data structure-template transforms, errors, and borrowed block views.
use crate::{BlockState, Compound};

/// Rotation of a template about its origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StructureRotation {
    /// No rotation.
    None,
    /// A clockwise quarter turn.
    Clockwise90,
    /// A half turn.
    Clockwise180,
    /// A counterclockwise quarter turn.
    Counterclockwise90,
}

/// Mirror applied before rotating a template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StructureMirror {
    /// No mirror.
    None,
    /// Mirror north and south.
    LeftRight,
    /// Mirror east and west.
    FrontBack,
}

/// Where a placement's template comes from: a template the game already has, by id, or vanilla
/// structure NBT the mod carries.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StructureTemplateSource {
    /// A template by id (`mymod:hut`), found as `StructureTemplateManager.get` finds it: the world's
    /// saved structures, then the game's and the mods' own.
    Id(String),
    /// Vanilla structure NBT, the bytes of a `structures/<namespace>/<path>.nbt` file: gzipped as the
    /// game saves one, or plain. Read by the game's own loader, within vanilla's limits for templates.
    Nbt(Vec<u8>),
}

impl StructureTemplateSource {
    /// The template `id` names.
    pub fn id(id: &str) -> Self {
        Self::Id(id.to_owned())
    }

    /// The template `bytes` hold, as vanilla structure NBT.
    pub fn nbt(bytes: Vec<u8>) -> Self {
        Self::Nbt(bytes)
    }
}

/// Why reading or placing a structure template through [`crate::LevelRef`] was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum StructureTemplateError {
    /// The level is gone or its handle belongs to an earlier lifetime.
    Stale,
    /// The operation is unavailable on this side, phase, host, or missing structure resources.
    Unavailable,
    /// A parked level cannot place a template while a Host transaction is open.
    InTransaction,
    /// The template identifier does not parse or contains a `.` or `..` path segment.
    InvalidId,
    /// No structure template with this identifier exists in the world or data packs.
    NotFound,
    /// The supplied structure NBT bytes failed decompression, parsing, or vanilla template limits.
    InvalidNbt,
    /// The template has no palette, a size below 1 on some axis, or places no blocks or entities.
    EmptyTemplate,
    /// A chunk covered by the template's transformed bounding box is not loaded (`PlaceCommand.checkLoaded`).
    UnloadedChunk {
        /// Coordinates `[cx, cz]` of an unloaded chunk in the template's footprint.
        chunk: [i32; 2],
    },
    /// A block state in the template cannot be mapped to the API block state table.
    InvalidState,
    /// The supplied rotation, mirror, or template source variant is unsupported by this build.
    UnsupportedTransform,
}

impl crate::NoBackend for StructureTemplateError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

impl From<crate::LevelReadError> for StructureTemplateError {
    fn from(error: crate::LevelReadError) -> Self {
        match error {
            crate::LevelReadError::Stale => Self::Stale,
            crate::LevelReadError::Unavailable => Self::Unavailable,
        }
    }
}

// no wire: borrowed call-scoped observation view; never crosses the mod wire.
/// A transformed block and its optional borrowed block-entity data, lent to one
/// [`crate::LevelRef::visit_structure_template`] callback.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct StructureTemplateBlockView<'a> {
    /// Position in the level.
    pub pos: [i32; 3],
    /// Owned state, including properties, after mirroring and rotation.
    pub state: BlockState,
    /// Saved block-entity data, without its position.
    pub nbt: Option<&'a Compound>,
}
