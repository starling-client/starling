"""Run actual drag tracking, dispatch and cursor update with a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from native_hook_sources import native_hook_sources

from test_world_input_capture import method, pointer_projection_sources

ROOT = Path(__file__).resolve().parents[1]


class ScreenMouseDraggedHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_drag_lifecycle_delta_and_cancelled_cursor(self):
        source = (ROOT / "crates/sunrise-client/src/game.rs").read_text(encoding="utf-8")
        hooks = native_hook_sources()
        hooks = hooks.replace("pub(super) fn", "fn").replace("super::glfw_", "glfw_")
        track = method(hooks, "track_drag_button")
        dispatch = method(hooks, "screen_mouse_dragged")
        start = source.index("                        let projection = self.pointer_projection(size);")
        cursor_end = source.index("                        if self.dialog_open()", start)
        cursor = source[start:cursor_end]
        window_handler = (ROOT / "crates/sunrise-client/src/game/runtime/window_handler.rs").read_text(encoding="utf-8")
        handler = method(window_handler, "handle_window_event_as")
        sample = handler[handler.index("        let cursor_sample ="):handler.index("        // Who caused it,")]
        refresh = method(source, "refresh_window_cursor")
        set_cursor = method(source, "set_window_cursor")
        # The extracted fixture only models the mouse-drag hook. Keep the
        # replay timeline's independent drag handler out of this focused slice.
        studio_drag = cursor.index("                        let [w, h]")
        hook_dispatch = cursor.index("                        match self.screen_mouse_dragged(delta)", studio_drag)
        cursor = cursor[:studio_drag] + cursor[hook_dispatch:]
        start = source.index("        if let WindowEvent::Focused(false)")
        focus = source[start:source.index("        if let WindowEvent::ModifiersChanged", start)]
        fixture = r"""
use std::{cell::RefCell, collections::HashSet};
type MouseButton = i32;
mod winit {
    pub mod dpi {
        #[derive(Clone, Copy, Debug)]
        pub struct PhysicalPosition<T> { pub x: T, pub y: T }
        impl<T> PhysicalPosition<T> { pub fn new(x: T, y: T) -> Self { Self { x, y } } }
        #[derive(Clone, Copy, Debug)]
        pub struct LogicalPosition<T> { pub x: T, pub y: T }
        impl<T> LogicalPosition<T> { pub fn new(x: T, y: T) -> Self { Self { x, y } } }
    }
    pub mod keyboard {
        #[derive(Clone, Copy)]
        pub struct ModifiersState(pub u32);
        impl ModifiersState {
            pub fn empty() -> Self { Self(0) }
        }
    }
}
use winit::{dpi::PhysicalPosition, keyboard::ModifiersState};
extern crate self as sunrise_client;
mod window {
    use super::winit::dpi::{LogicalPosition, PhysicalPosition};
    POINTER_PROJECTION
}
use window::{GuiSize, PointerProjection, PointerSample, PointerSpace};
mod input_trace {
    #[derive(PartialEq)] pub enum InputOrigin { User, Game }
}
mod native_hooks {
    pub enum ScreenMousePost { Drag { button: i32, delta: [f32; 2] } }
    pub enum ScreenMousePre { NotFired, Canceled, Passed(ScreenMousePost) }
}
use native_hooks::{ScreenMousePost, ScreenMousePre};
fn glfw_button(button: i32) -> i32 { button }
fn glfw_modifiers(state: ModifiersState) -> u32 { state.0 }
struct View { mouse: (f32, f32) }
type Point = PhysicalPosition<f64>;
enum WindowEvent { Focused(bool), CursorMoved { position: Point } }
struct Game { drag_button: Option<i32>, physical_buttons: HashSet<i32>, physical_keys: HashSet<i32>,
    container_buttons: HashSet<i32>,
    cursor: (f32, f32), window_cursor: Option<PointerSample>, gui_scale: u32, modifiers: ModifiersState,
    framebuffer: [u32; 2], backing_scale: f64,
    synthetic: bool, open: bool, cancel: bool, handled: usize, posts: usize,
    calls: RefCell<Vec<(i32, [f32; 2], (f32, f32))>> }
impl Game {
    fn pad_synthetic_input(&self) -> bool { self.synthetic }
    // No container hook registered: it takes no drag.
    fn container_drag(&mut self) -> bool { false }
    fn screen_view_now(&self) -> Option<View> { self.open.then_some(View { mouse: self.cursor }) }
    fn interface_origin(&self) -> [f32; 2] { [0.0; 2] }
    fn native_screen_mouse_dragged_pre(&self, view: &View, button: i32, delta: [f32; 2], modifiers: u32, canceled: &mut bool) {
        assert_eq!(modifiers, 5);
        self.calls.borrow_mut().push((button, delta, view.mouse)); *canceled = self.cancel;
    }
    TRACK
    DISPATCH
    POINTER_SETTER
    REFRESH_CURSOR
    fn pointer_space(&self, _: [u32; 2]) -> PointerSpace {
        PointerSpace::new(self.framebuffer, self.backing_scale)
    }
    fn pointer_projection(&self, size: [u32; 2]) -> PointerProjection {
        PointerProjection { gui: GuiSize::new(size, self.gui_scale, false), window: self.pointer_space(size) }
    }
    fn renderer_pointer_projection(&self) -> Option<PointerProjection> {
        Some(self.pointer_projection(self.framebuffer))
    }
    fn pointer(&mut self, position: Point) -> Result<(), ()> {
        let size = self.framebuffer;
        let event = WindowEvent::CursorMoved { position };
        let retained_sample = None;
        let mut mouse_post = None;
        use input_trace::InputOrigin;
        let origin = if self.synthetic { InputOrigin::Game } else { InputOrigin::User };
        CURSOR_SAMPLE
        let sample = cursor_sample.unwrap();
        CURSOR
        self.posts += usize::from(mouse_post.is_some());
        self.handled += 1; Ok(())
    }
    fn focus(&mut self, event: WindowEvent) { FOCUS }
}
#[test]
fn lifecycle_and_cancellation() {
    for cancel in [false, true] {
        let mut game = Game { drag_button: None, physical_buttons: HashSet::from([0, 1]),
            container_buttons: HashSet::new(),
            physical_keys: HashSet::from([10]), cursor: (10.0, 10.0),
            window_cursor: Some(PointerSpace::new([640, 480], 1.0).physical_sample(Point::new(20.0, 20.0))),
            framebuffer: [640, 480], backing_scale: 1.0, gui_scale: 2, modifiers: ModifiersState(5), synthetic: false,
            open: true, cancel, handled: 0, posts: 0, calls: RefCell::new(vec![]) };
        game.track_drag_button(0, true);
        game.track_drag_button(1, true);
        game.track_drag_button(0, false);
        // Any release ends the drag (vanilla clears activeButton): right is still held, no drag.
        assert_eq!(game.drag_button, None);
        game.track_drag_button(1, true);
        game.pointer(Point { x: 50.0, y: 30.0 }).unwrap();
        game.pointer(Point { x: 54.0, y: 34.0 }).unwrap();
        assert_eq!(game.cursor, (27.0, 17.0));
        assert_eq!(game.handled, if cancel { 0 } else { 2 });
        assert_eq!(game.posts, if cancel { 0 } else { 2 });
        assert_eq!(*game.calls.borrow(), [(1, [15.0, 5.0], (25.0, 15.0)), (1, [2.0, 2.0], (27.0, 17.0))]);
        game.calls.borrow_mut().clear();
        game.pointer(Point { x: 54.0, y: 34.0 }).unwrap();
        game.synthetic = true;
        game.pointer(Point { x: 56.0, y: 36.0 }).unwrap();
        game.synthetic = false; game.open = false;
        game.pointer(Point { x: 58.0, y: 38.0 }).unwrap();
        game.open = true; game.physical_buttons.clear();
        game.pointer(Point { x: 60.0, y: 40.0 }).unwrap();
        assert!(game.calls.borrow().is_empty());
        game.track_drag_button(1, false);
        assert_eq!(game.drag_button, None);
        game.track_drag_button(0, true);
        game.physical_buttons.insert(0);
        game.focus(WindowEvent::Focused(false));
        assert_eq!(game.modifiers.0, 0);
        assert_eq!(game.drag_button, None);
        assert!(game.physical_buttons.is_empty() && game.physical_keys.is_empty());
        game.pointer(Point { x: 62.0, y: 42.0 }).unwrap();
        assert!(game.calls.borrow().is_empty());

        game.physical_buttons.insert(0);
        game.track_drag_button(0, true);
        game.modifiers = ModifiersState(5);
        // Changing scale without a new position must not make a drag.
        game.gui_scale = 1;
        game.pointer(Point::new(62.0, 42.0)).unwrap();
        assert!(game.calls.borrow().is_empty());
        // The event still fires when current and previous positions narrow
        // to the same f32; the mod gets the delta narrowed after projection.
        game.pointer(Point::new(62.00000001, 42.0)).unwrap();
        let calls = game.calls.borrow();
        let [(button, [dx, dy], at)] = calls.as_slice() else { panic!("expected one tiny drag, got {calls:?}") };
        assert_eq!((*button, *dy, *at), (0, 0.0, (62.0, 42.0)));
        assert!((*dx - 1e-8).abs() < 1e-12, "wrong delta: {dx}");
    }
}

#[test]
fn scale_change_keeps_a_stationary_pointer_on_the_same_click_target() {
    let mut game = Game { container_buttons: HashSet::new(), drag_button: None, physical_buttons: HashSet::from([0]),
        physical_keys: HashSet::new(), cursor: (0.0, 0.0), window_cursor: None,
        framebuffer: [640, 480], backing_scale: 1.0, gui_scale: 2, modifiers: ModifiersState(5),
        synthetic: false, open: true, cancel: false, handled: 0, posts: 0,
        calls: RefCell::new(vec![]) };
    game.track_drag_button(0, true);
    game.pointer(Point::new(200.0, 100.0)).unwrap();
    game.framebuffer = [1280, 960];
    game.backing_scale = 2.0;
    game.gui_scale = 4;
    // Same refresh MouseInput uses before hit testing, with no new cursor event.
    game.refresh_window_cursor();
    assert!((100.0..108.0).contains(&game.cursor.0) && (50.0..58.0).contains(&game.cursor.1));
    game.pointer(Point::new(400.0, 200.0)).unwrap();
    assert!(game.calls.borrow().is_empty());
}
""".replace("TRACK", track).replace("DISPATCH", dispatch).replace("CURSOR_SAMPLE", sample).replace("REFRESH_CURSOR", refresh).replace("CURSOR", cursor).replace("FOCUS", focus).replace("POINTER_SETTER", set_cursor).replace("POINTER_PROJECTION", pointer_projection_sources())
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "drag.rs", Path(folder) / "drag.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
