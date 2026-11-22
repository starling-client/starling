//! What the player plays with (horizon_client::input_mode): the touch controls, the chat's bar and
//! the on-screen keyboard show while it is the touch screen. On Android the mouse's events come
//! here from the input hook (android.rs) and go on as the window events a computer's mouse
//! makes; a physical keyboard's keys and a controller (game/pad.rs) also count as the input used.
use super::Game;
#[cfg(target_os = "android")]
use super::{event_loop::EventLoopCtl, input_trace::InputOrigin};
use horizon_client::input_mode::InputMode;
#[cfg(target_os = "android")]
use winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
};

/// Where the mouse's pointer last was, in window pixels (`MouseEvent::Moved`).
#[cfg(target_os = "android")]
static POINTER: std::sync::Mutex<(f64, f64)> = std::sync::Mutex::new((0.0, 0.0));

/// The finger a mouse's left button stands for on a card drawn for the touch screen.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const MOUSE_FINGER: u64 = u64::MAX;

/// The touch a mouse's left button makes at `at` (window pixels) on a card that reads only
/// fingers.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn mouse_touch(at: (f64, f64), down: bool) -> winit::event::Touch {
    winit::event::Touch {
        device_id: winit::event::DeviceId::dummy(),
        phase: if down {
            winit::event::TouchPhase::Started
        } else {
            winit::event::TouchPhase::Ended
        },
        location: winit::dpi::PhysicalPosition::new(at.0, at.1),
        force: None,
        id: MOUSE_FINGER,
    }
}

impl Game<'_> {
    /// The player used `input`. Off the touch screen the fingers let go of what they held and the
    /// on-screen keyboard goes (touch_frame); any input but a controller takes the menus back
    /// from the pad's pointer.
    pub(super) fn input_used(&mut self, input: InputMode) {
        let mode = self
            .settings
            .touch
            .input_selection
            .resolve(self.input_mode.after(input, cfg!(target_os = "android")));
        if mode == self.input_mode {
            return;
        }
        if self.input_mode == InputMode::Touch {
            self.touch_release_all();
        }
        if mode != InputMode::Pad {
            self.pad.in_use = false;
        }
        self.input_mode = mode;
    }

    /// The touch controls, the chat's bar and the on-screen keyboard show.
    pub(super) fn touch_shown(&self) -> bool {
        self.input_mode == InputMode::Touch
    }

    /// Once per loop iteration, before the touch frame: the pointer capture Android took back,
    /// a physical keyboard's keys, and the mouse's events since the last iteration, as window
    /// events (its captured motion turns the view as a computer's raw motion does).
    #[cfg(target_os = "android")]
    pub(super) fn android_input_frame(&mut self, event_loop: &dyn EventLoopCtl) {
        if crate::android::pointer_capture_lost() && self.captured {
            self.capture(false);
        }
        if crate::android::keyboard_used() {
            self.input_used(InputMode::KeyboardMouse);
        }
        let events = crate::android::take_mouse_events();
        if events.is_empty() {
            return;
        }
        self.input_used(InputMode::KeyboardMouse);
        let Some(id) = self.window.as_ref().map(|w| w.id()) else {
            return;
        };
        let device_id = DeviceId::dummy();
        for event in events {
            let event = match event {
                crate::android::MouseEvent::Moved(x, y) => {
                    *POINTER.lock().unwrap_or_else(|e| e.into_inner()) = (x, y);
                    WindowEvent::CursorMoved {
                        device_id,
                        position: PhysicalPosition::new(x, y),
                    }
                }
                crate::android::MouseEvent::Button(button, down) => {
                    if button == MouseButton::Left && self.card_click(down) {
                        continue;
                    }
                    WindowEvent::MouseInput {
                        device_id,
                        state: if down {
                            ElementState::Pressed
                        } else {
                            ElementState::Released
                        },
                        button,
                    }
                }
                crate::android::MouseEvent::Wheel(x, y) => WindowEvent::MouseWheel {
                    device_id,
                    delta: MouseScrollDelta::LineDelta(x, y),
                    phase: TouchPhase::Moved,
                },
                crate::android::MouseEvent::Motion(x, y) => {
                    self.grabbed_mouse_motion(x, y);
                    continue;
                }
            };
            self.handle_window_event(event_loop, id, event, InputOrigin::User);
        }
    }

    /// A mouse's left button on a card drawn for the touch screen (the Starling folder's or
    /// Horizon Android's offer, an update, the native builds, the mod catalog): those read only
    /// fingers, so with a mouse their buttons could not be pressed. The button goes to them as a
    /// finger where the pointer is; true when one took it.
    #[cfg(target_os = "android")]
    fn card_click(&mut self, down: bool) -> bool {
        let at = *POINTER.lock().unwrap_or_else(|e| e.into_inner());
        let touch = mouse_touch(at, down);
        self.files_offer_touch(&touch)
            || self.update_offer_touch(&touch)
            || self.native_builds_touch(&touch)
            || self.catalog_touch(&touch)
    }
}

#[cfg(test)]
mod tests {
    use super::mouse_touch;
    use winit::event::TouchPhase;

    #[test]
    fn a_mouse_click_is_a_finger_where_the_pointer_is() {
        let down = mouse_touch((120.5, 300.0), true);
        let up = mouse_touch((120.5, 300.0), false);
        assert_eq!(down.phase, TouchPhase::Started);
        assert_eq!(up.phase, TouchPhase::Ended);
        assert_eq!((up.location.x, up.location.y), (120.5, 300.0));
        // One finger from press to release, so a card that follows fingers (the catalog's list)
        // sees a tap.
        assert_eq!(down.id, up.id);
    }
}
