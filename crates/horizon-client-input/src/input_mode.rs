//! Which input the player plays with, as Bedrock's input modes: the touch controls, the chat's
//! bar and the on-screen keyboard show only while it is the touch screen. Where the touch screen
//! is the platform's own input (Android), the keyboard, a mouse or a controller put them away and
//! the next touch brings them back; elsewhere (a touch laptop) they stay from the first touch on,
//! the keyboard and mouse beside them. And what Android's window system leaves out about the
//! keyboard: which keys are a physical keyboard's, and the modifiers held.
use std::collections::HashSet;
use winit::keyboard::{KeyCode, ModifiersState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Touch,
    Pad,
    KeyboardMouse,
}

impl InputMode {
    /// What the game starts in: the touch screen where it is the platform's own input.
    pub fn first(touch_platform: bool) -> Self {
        if touch_platform {
            InputMode::Touch
        } else {
            InputMode::KeyboardMouse
        }
    }

    /// The mode after the player used `input`.
    pub fn after(self, input: InputMode, touch_platform: bool) -> Self {
        if self == InputMode::Touch && !touch_platform {
            InputMode::Touch
        } else {
            input
        }
    }
}

/// `KeyCharacterMap.VIRTUAL_KEYBOARD`: the device of keys the system makes up (the on-screen
/// keyboard's, the navigation bar's Back).
const VIRTUAL_KEYBOARD: i32 = -1;

/// Whether an Android key press is the player typing on a physical keyboard: one of a keyboard's
/// keys (letters, digits, punctuation, Enter, Escape, the arrows, modifiers, F-keys, the
/// keypad), from a real device that is not a controller. The system's keys (volume, Back, Home,
/// media), a controller's and the on-screen keyboard's (no scan code) are not.
pub fn android_keyboard(source: u32, device: i32, scan_code: i32, keycode: i32) -> bool {
    let typing = matches!(
        keycode,
        7..=16 | 19..=22 | 29..=62 | 66..=77 | 81 | 92 | 93 | 111..=124 | 131..=163
    );
    typing
        && device != VIRTUAL_KEYBOARD
        && scan_code != 0
        && !crate::gamepad::android::from_pad(source)
}

/// The modifiers the held keys make: Android's window system reports none (winit sends no
/// `ModifiersChanged` there).
pub fn held_modifiers(held: &HashSet<KeyCode>) -> ModifiersState {
    let mut modifiers = ModifiersState::empty();
    for (keys, modifier) in [
        (
            [KeyCode::ShiftLeft, KeyCode::ShiftRight],
            ModifiersState::SHIFT,
        ),
        (
            [KeyCode::ControlLeft, KeyCode::ControlRight],
            ModifiersState::CONTROL,
        ),
        ([KeyCode::AltLeft, KeyCode::AltRight], ModifiersState::ALT),
        (
            [KeyCode::SuperLeft, KeyCode::SuperRight],
            ModifiersState::SUPER,
        ),
    ] {
        if keys.iter().any(|k| held.contains(k)) {
            modifiers |= modifier;
        }
    }
    modifiers
}

/// How far a mouse Android has not captured went in one event: its `AXIS_RELATIVE_X/Y`
/// (`relative`, the event's history summed in), or, for a mouse whose events carry none (some
/// phones' Bluetooth and OTG mice), how far its pointer went since the event before (`before` to
/// `at`), as Horizon Android 0.7.8 reads it. With only the first, the view stood still or moved
/// in jerks on those mice while the game held the pointer and Android had not captured it.
pub fn free_mouse_motion(
    relative: (f32, f32),
    before: Option<(f32, f32)>,
    at: (f32, f32),
) -> (f32, f32) {
    if relative != (0.0, 0.0) {
        return relative;
    }
    before.map_or((0.0, 0.0), |(x, y)| (at.0 - x, at.1 - y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_mouse_moves_by_its_relative_axes_or_else_by_its_pointer() {
        // A mouse that reports how far it went: that, whatever its pointer did.
        assert_eq!(
            free_mouse_motion((3.0, -2.0), Some((10.0, 10.0)), (50.0, 50.0)),
            (3.0, -2.0)
        );
        // One that reports nothing there: the pointer's own move since the event before.
        assert_eq!(
            free_mouse_motion((0.0, 0.0), Some((10.0, 20.0)), (14.0, 17.0)),
            (4.0, -3.0)
        );
        // Its first event has nothing to measure from.
        assert_eq!(
            free_mouse_motion((0.0, 0.0), None, (14.0, 17.0)),
            (0.0, 0.0)
        );
    }

    #[test]
    fn a_phone_follows_the_input_used() {
        let mut mode = InputMode::first(true);
        assert_eq!(mode, InputMode::Touch);
        for input in [
            InputMode::Pad,
            InputMode::KeyboardMouse,
            InputMode::Touch,
            InputMode::KeyboardMouse,
            InputMode::Pad,
            InputMode::Touch,
        ] {
            mode = mode.after(input, true);
            assert_eq!(mode, input);
        }
    }

    #[test]
    fn a_computer_keeps_the_touch_controls_once_touched() {
        let mut mode = InputMode::first(false);
        assert_eq!(mode, InputMode::KeyboardMouse);
        mode = mode.after(InputMode::Pad, false);
        assert_eq!(mode, InputMode::Pad);
        mode = mode.after(InputMode::Touch, false);
        for input in [InputMode::KeyboardMouse, InputMode::Pad] {
            mode = mode.after(input, false);
            assert_eq!(mode, InputMode::Touch);
        }
    }

    #[test]
    fn only_a_physical_keyboards_keys_count() {
        const KEYBOARD: u32 = 0x0301;
        // W, Space, Escape, the left arrow, F3, left shift, keypad 5.
        for code in [51, 62, 111, 21, 133, 59, 149] {
            assert!(android_keyboard(KEYBOARD, 4, 30, code), "{code}");
        }
        // Back, volume up, Home, media play/pause, a pad's A.
        for code in [4, 24, 3, 85, 96] {
            assert!(!android_keyboard(KEYBOARD, 4, 30, code), "{code}");
        }
        // The on-screen keyboard's Enter: the virtual device, or no scan code.
        assert!(!android_keyboard(KEYBOARD, VIRTUAL_KEYBOARD, 28, 66));
        assert!(!android_keyboard(KEYBOARD, 4, 0, 66));
        // A controller's D-pad.
        assert!(!android_keyboard(0x0501, 6, 544, 19));
    }

    #[test]
    fn held_keys_make_the_modifiers() {
        let held = HashSet::from([KeyCode::ShiftRight, KeyCode::ControlLeft, KeyCode::KeyA]);
        assert_eq!(
            held_modifiers(&held),
            ModifiersState::SHIFT | ModifiersState::CONTROL
        );
        assert_eq!(held_modifiers(&HashSet::new()), ModifiersState::empty());
        let held = HashSet::from([KeyCode::AltLeft, KeyCode::SuperRight]);
        assert_eq!(
            held_modifiers(&held),
            ModifiersState::ALT | ModifiersState::SUPER
        );
    }
}
