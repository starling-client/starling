//! Starling's part of game.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Bedrock Inventory: how far from the middle each of its two windows goes, and the slots of the
/// crafting grid and its result it adds to the inventory's.
pub(super) const BEDROCK_SHIFT: f32 = 100.0;

pub(super) const BEDROCK_CRAFT: [usize; 5] = [100, 101, 102, 103, 200];

pub(super) const MOBILE_PLATFORM: bool = cfg!(target_os = "android");

impl Game<'_> {
    pub(super) fn mobile_holds_key(&self, _value: KeyCode, _pressed: bool) -> bool {
        #[cfg(target_os = "android")]
        {
            !_pressed && self.android_custom_key_held(_value)
        }
        #[cfg(not(target_os = "android"))]
        false
    }
    pub(super) fn mobile_holds_mouse(&self, _value: MouseButton, _pressed: bool) -> bool {
        #[cfg(target_os = "android")]
        {
            !_pressed && self.android_custom_mouse_held(_value)
        }
        #[cfg(not(target_os = "android"))]
        false
    }
    pub(super) fn platform_shift_down(&self) -> bool {
        #[cfg(target_os = "android")]
        {
            self.mobile_shift_down()
        }
        #[cfg(not(target_os = "android"))]
        false
    }
    pub(super) fn platform_control_down(&self) -> bool {
        #[cfg(target_os = "android")]
        {
            self.mobile_control_down()
        }
        #[cfg(not(target_os = "android"))]
        false
    }
    pub(super) fn platform_keypad_event(
        &mut self,
        _events: &dyn EventLoopCtl,
        _event: &mut WindowEvent,
    ) -> bool {
        #[cfg(target_os = "android")]
        if let WindowEvent::KeyboardInput { event: key, .. } = _event {
            if let Some(code) =
                sunrise_client::keypad::resolve_key(key.physical_key, &key.logical_key)
            {
                key.physical_key = PhysicalKey::Code(code);
            }
            if self.mobile_keypad_key(_events, key) {
                return true;
            }
        }
        false
    }
    pub(super) fn platform_held_modifiers(&mut self, _event: &WindowEvent) {
        #[cfg(target_os = "android")]
        if matches!(
            _event,
            WindowEvent::KeyboardInput { .. } | WindowEvent::Focused(false)
        ) {
            self.modifiers = sunrise_client::input_mode::held_modifiers(&self.physical_keys);
        }
    }
    pub(super) fn platform_frame_hint(&self, _rendered: bool, _cpu: Duration) {
        #[cfg(target_os = "android")]
        if _rendered {
            crate::android::frame_hint(_cpu, self.framerate_limit());
        }
    }
    pub(super) fn platform_diagnostic_heartbeat(&mut self) {
        #[cfg(target_os = "android")]
        self.diagnostic_heartbeat();
    }
    pub(super) fn platform_first_frame(&self) {
        #[cfg(target_os = "android")]
        crate::android::first_frame_shown();
    }
}

pub(super) const MACOS_PLATFORM: bool = cfg!(target_os = "macos");

impl Game<'_> {
    // A function of its own: native mods can't hook one with a `cfg` in its body, and mods hook `hud`.
    #[cfg(target_os = "android")]
    pub(super) fn draw_phone_offers(&mut self, batch: &mut HudBatch) {
        self.draw_files_offer(batch);
        self.draw_update_offer(batch);
        self.draw_catalog(batch);
        self.draw_native_builds(batch);
    }

    #[cfg(not(target_os = "android"))]
    pub(super) fn draw_phone_offers(&mut self, _batch: &mut HudBatch) {}
}
