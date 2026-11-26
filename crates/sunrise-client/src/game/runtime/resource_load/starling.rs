//! Starling's part of resource_load.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Progress {
    pub(in super::super) fn note(&self, line: Option<String>) {
        *self
            .note
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = line;
    }

    pub(in super::super) fn noted(&self) -> Option<String> {
        self.note
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}
