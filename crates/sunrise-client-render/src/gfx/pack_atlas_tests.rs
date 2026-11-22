//! The probes of the block atlas a shader pack draws with: the atlas itself is built by
//! `sunrise_render_shaderpack::pack_atlas`, and what these read is what the client's terrain,
//! mesher and flat draws make of it.

use sunrise_render_shaderpack::pack_atlas::PackAtlas;

#[cfg(test)]
#[path = "pack_atlas/fade_test.rs"]
mod fade_test;

#[cfg(test)]
#[path = "pack_atlas/midpoint_test.rs"]
mod midpoint_test;

#[cfg(test)]
#[path = "pack_atlas/midpoint_support.rs"]
mod midpoint_support;

#[cfg(test)]
#[path = "pack_atlas/source_ownership_test.rs"]
mod source_ownership_test;
