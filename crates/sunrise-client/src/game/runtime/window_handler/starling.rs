//! Starling's part of window_handler.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// `event`'s key without the modifiers held (winit's modifier supplement). Starling: Android's winit has
/// none, and the logical key is the nearest; a function apart so `window_event`, which mods target, has no
/// cfg in its body (horizon-mixins refuses one).
pub(super) fn key_without_modifiers(event: &winit::event::KeyEvent) -> winit::keyboard::Key {
    #[cfg(not(target_os = "android"))]
    {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        event.key_without_modifiers()
    }
    #[cfg(target_os = "android")]
    {
        event.logical_key.clone()
    }
}
