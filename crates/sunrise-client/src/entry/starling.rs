//! Starling's part of entry.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Runs once the log is open (Android's start-up notes, its platform set-up that logs); set before [`start`].
pub(super) static AFTER_LOG: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>> =
    std::sync::Mutex::new(None);

/// Register what runs once the log is open (see `AFTER_LOG`).
pub fn set_after_log(hook: impl FnOnce() + Send + 'static) {
    *AFTER_LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(hook));
}
