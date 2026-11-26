//! Starling's part of resource_tables.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Resources {
    /// Whether the world's resources (items, mobs...) have loaded: menus show before they do.
    pub(in super::super) fn rest_ready(&self) -> bool {
        self.rest.get().is_some()
    }
}
