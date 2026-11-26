#!/usr/bin/env python3
"""Replace the local feature and structure error tuples. Run from the repo root."""

from pathlib import Path


VOCAB = Path("crates/sunrise-mod-vocab/src")
MODDING = Path("crates/sunrise-client/src/game/modding")


def error_tuple(name, role):
    return f'''#[derive(Debug)]
// exhaustive: mods build it themselves as a {role}'s error answer
#[derive(sunrise_mod_api_macros::Wire)]
pub struct {name}(pub String);
impl std::fmt::Display for {name} {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        f.write_str(&self.0)
    }}
}}
impl std::error::Error for {name} {{}}
impl From<(crate::Identifier, String)> for {name} {{
    fn from((source, reason): (crate::Identifier, String)) -> Self {{
        Self(format!("{{source}}: {{reason}}"))
    }}
}}'''


def native_error(name):
    return f'''// no wire: retains the native source chain for this in-process callback.
#[derive(Debug)]
// exhaustive: its field is private, so mods cannot build or destructure it
pub struct {name} {{
    source: Box<dyn std::error::Error + Send + Sync>,
}}
impl {name} {{
    /// Preserve the original error for [`std::error::Error::source`].
    /// For a standalone message, use `std::io::Error::other(message)`.
    pub fn new(cause: impl std::error::Error + Send + Sync + 'static) -> Self {{
        Self {{
            source: Box::new(cause),
        }}
    }}
}}
impl std::fmt::Display for {name} {{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{
        std::fmt::Display::fmt(&self.source, f)
    }}
}}
impl std::error::Error for {name} {{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {{
        Some(self.source.as_ref())
    }}
}}'''


BLOCK_ERROR_IMPLS = '''impl std::fmt::Display for BlockStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("block metadata unavailable"),
            Self::UnknownBlock => f.write_str("unknown block"),
            Self::UnknownState => f.write_str("unknown block state"),
            Self::UnknownProperty(index) => write!(f, "unknown block property at constraint {index}"),
            Self::InvalidValue(index) => write!(f, "invalid block property value at constraint {index}"),
            Self::DuplicateProperty(index) => write!(f, "duplicate block property at constraint {index}"),
            Self::InvalidCombination => f.write_str("unregistered block state combination"),
        }
    }
}
impl std::error::Error for BlockStateError {}
'''


def main():
    edits = {
        VOCAB / "world/feature_type.rs": [
            (error_tuple("FeatureError", "feature"), native_error("FeatureError"), 1),
        ],
        VOCAB / "world/structure_type.rs": [
            (error_tuple("StructureError", "piece"), native_error("StructureError"), 1),
        ],
        # The example's block metadata calls already return BlockStateError. Make
        # it usable as a source directly rather than formatting or wrapping it.
        VOCAB / "block/block_metadata.rs": [
            (
                "impl crate::NoBackend for BlockStateError {",
                BLOCK_ERROR_IMPLS + "impl crate::NoBackend for BlockStateError {",
                1,
            ),
        ],
        Path("docs/modding/examples/src/worldgen.rs"): [
            (
                '.map_err(|error| StructureError(format!("saved material: {error:?}")))?',
                ".map_err(StructureError::new)?",
                1,
            ),
            (
                '.map_err(|error| StructureError(format!("saved block properties: {error:?}")))?',
                ".map_err(StructureError::new)?",
                1,
            ),
            (
                ".map_err(|error| StructureError(error.to_string()))",
                ".map_err(StructureError::new)",
                1,
            ),
            (
                '''Err(StructureError(
                    "saved block does not match this pillar's material".into(),
                ))''',
                '''Err(StructureError::new(std::io::Error::other(
                    "saved block does not match this pillar's material",
                )))''',
                1,
            ),
            (
                'StructureError(format!("missing or invalid saved {key}"))',
                'StructureError::new(std::io::Error::other(format!("missing or invalid saved {key}")))',
                2,
            ),
            (
                'FeatureError("height was not decoded".into())',
                'FeatureError::new(std::io::Error::other("height was not decoded"))',
                1,
            ),
            (
                '.map_err(|error| FeatureError(format!("log state: {error:?}")))?',
                ".map_err(FeatureError::new)?",
                1,
            ),
        ],
        MODDING / "native_structure_types/tests.rs": [
            (
                'StructureError(format!("invalid saved {key}"))',
                'StructureError::new(std::io::Error::other(format!("invalid saved {key}")))',
                1,
            ),
            (
                'StructureError("fixture has no saved pieces".into())',
                'StructureError::new(std::io::Error::other("fixture has no saved pieces"))',
                2,
            ),
            (
                'StructureError("missing fixture mode".into())',
                'StructureError::new(std::io::Error::other("missing fixture mode"))',
                1,
            ),
        ],
        Path("crates/sunrise-mod-api/src/parts/structures.rs"): [
            (
                'mod_api::StructureError("fixture has no pieces".into())',
                'mod_api::StructureError::new(std::io::Error::other("fixture has no pieces"))',
                1,
            ),
        ],
        MODDING / "native_feature_types.rs": [
            (".map_err(|error| error.0)", ".map_err(|error| error.to_string())", 1),
        ],
        MODDING / "native_structure_types.rs": [
            ("PlacementQueryError::Failed(e.0)", "PlacementQueryError::Failed(e.to_string())", 2),
        ],
    }
    rewritten = {}
    for path, replacements in edits.items():
        source = path.read_text()
        for before, after, expected in replacements:
            count = source.count(before)
            if count != expected:
                raise SystemExit(f"{path}: expected {expected} matches for {before!r}, found {count}")
            source = source.replace(before, after)
        rewritten[path] = source

    for path, source in rewritten.items():
        path.write_text(source)


if __name__ == "__main__":
    main()
