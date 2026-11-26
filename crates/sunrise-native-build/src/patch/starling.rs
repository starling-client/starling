//! Starling's part of patch.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The horizon-mixins tool at `identity`, fetched or built once per pin (for a program that only
/// readies it, as Starling's Android store base does before its offline builds).
pub fn mixins_tool(rust: &dyn Toolchain, identity: &MixinsIdentity) -> Result<PathBuf, String> {
    super::super::mixins_patch::tool(rust, identity)
}
