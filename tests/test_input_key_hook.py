"""Run the actual window-key callback with fake normal/early-return handlers."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from native_hook_sources import native_hook_sources

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class InputKeyHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_panel_receives_named_nonrepeat_presses(self):
        source = native_hook_sources()
        actual = method(source.replace("    pub(super) fn native_panel_key_event(", "    fn native_panel_key_event("), "native_panel_key_event")
        fixture = r"""
type KeyCode = i32;
mod keybinds {
    pub fn key_name(key: i32) -> Option<&'static str> {
        (key == 1).then_some("key.keyboard.a")
    }
}
#[derive(Default)]
struct Game { calls: Vec<&'static str>, keys: Vec<(String, bool)> }
impl Game {
    fn native_panel_key(&mut self, name: &'static str) { self.calls.push(name); }
    // Starling's old key hook (game/starling/native_compat.rs).
    fn native_key(&mut self, key: &str, pressed: bool) { self.keys.push((key.to_owned(), pressed)); }
    METHOD
}
#[test]
fn panel_receives_only_named_nonrepeat_presses() {
    let mut game = Game::default();
    game.native_panel_key_event(1, true, false);
    game.native_panel_key_event(1, true, true);
    game.native_panel_key_event(1, false, false);
    game.native_panel_key_event(2, true, false);
    assert_eq!(game.calls, ["key.keyboard.a"]);
    // The old hook hears the named key going down and up, never a repeat.
    assert_eq!(game.keys, [("key.keyboard.a".to_owned(), true), ("key.keyboard.a".to_owned(), false)]);
}
""".replace("METHOD", actual)
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as folder:
            src, exe = Path(folder) / "panel.rs", Path(folder) / "panel.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)

    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_original_event_after_handling_and_source_gates(self):
        source = "".join((ROOT / "crates/horizon-client/src" / path).read_text(encoding="utf-8")
                         for path in ("game/runtime/window_handler.rs", "game.rs"))
        actual = method(source, "window_event")
        fixture = r"""
type WindowId = i32;
struct ActiveEventLoop;
struct Window;
impl Window { fn id(&self) -> i32 { 7 } }
mod input_trace {
    #[derive(Clone, Copy)]
    pub enum InputOrigin { User, Agent, Game }
}
mod horizon_client {
    pub mod keybinds {
        pub fn key_name(code: i32) -> Option<&'static str> { (code == 1).then_some("key.keyboard.a") }
    }
}
mod winit {
    pub mod event {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub enum ElementState { Pressed, Released }
    }
}
mod mod_api {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum KeyAction { Press, Release, Repeat }
}
use mod_api::KeyAction;
use winit::event::ElementState;
#[derive(Clone, Debug, PartialEq)]
enum PhysicalKey { Code(i32), Unidentified(i32) }
#[derive(Clone, Debug, PartialEq)]
struct KeyEvent { physical_key: PhysicalKey, text: &'static str, state: ElementState, repeat: bool }
enum WindowEvent { KeyboardInput { event: KeyEvent, is_synthetic: bool }, Other }
fn glfw_modifiers(bits: u32) -> u32 { bits }
struct Game { window: Option<Window>, modifiers: u32, early: bool, input: bool,
              calls: Vec<&'static str>, observed: Vec<(&'static str, KeyAction, u32)> }
impl Game {
    fn scripted_run(&self) -> bool { false }
    fn with_user_input<R>(&mut self, input: bool, f: impl FnOnce(&mut Self) -> R) -> R {
        let was = std::mem::replace(&mut self.input, input);
        let result = f(self);
        self.input = was;
        result
    }
    fn handle_window_event(&mut self, _: &ActiveEventLoop, _: WindowId, mut event: WindowEvent,
                           _: input_trace::InputOrigin) {
        self.calls.push("handle"); self.modifiers = 0;
        if let WindowEvent::KeyboardInput { event, .. } = &mut event { event.physical_key = PhysicalKey::Code(99); event.text = ""; }
        if self.early { return; }
        self.calls.push("finish");
    }
    fn native_input_key(&mut self, name: &'static str, action: KeyAction, modifiers: u32) {
        // The key's hook runs as input: a mod there may reach the clipboard.
        assert!(self.input);
        self.calls.push("hook"); self.observed.push((name, action, modifiers));
    }
    // Starling's old key hook (native_compat.rs), called beside the new one.
    fn native_input_key_old(&mut self, _: &KeyEvent, _: u32) {}
    METHOD
}
#[test]
fn all_actions_consumption_and_window_gates() {
    for (state, repeat, action) in [(ElementState::Pressed, false, KeyAction::Press),
        (ElementState::Pressed, true, KeyAction::Repeat), (ElementState::Released, false, KeyAction::Release)] {
        for (physical_key, name) in [(PhysicalKey::Code(1), "key.keyboard.a"),
            (PhysicalKey::Code(2), "key.keyboard.unknown"), (PhysicalKey::Unidentified(3), "key.keyboard.unknown")] {
            for early in [false, true] {
                let original = KeyEvent { physical_key: physical_key.clone(), text: "z", state, repeat };
                for (window, id, synthetic, fires) in [(true,7,false,true), (true,8,false,false),
                    (true,7,true,false), (false,7,false,false)] {
                    let mut game = Game { window: window.then_some(Window), modifiers: 5, early, input: false,
                        calls: vec![], observed: vec![] };
                    game.window_event(&ActiveEventLoop, id, WindowEvent::KeyboardInput { event: original.clone(), is_synthetic: synthetic });
                    let mut expected = vec!["handle"];
                    if !early { expected.push("finish"); }
                    if fires { expected.push("hook"); }
                    assert_eq!(game.calls, expected);
                    assert_eq!(game.observed, if fires { vec![(name, action, 5)] } else { vec![] });
                }
            }
        }
    }
    let mut game = Game { window: Some(Window), modifiers: 5, early: false, input: false, calls: vec![], observed: vec![] };
    game.window_event(&ActiveEventLoop, 7, WindowEvent::Other);
    assert!(game.observed.is_empty());

}
""".replace("METHOD", actual)
        target = ROOT / "target"
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as folder:
            src, exe = Path(folder) / "key.rs", Path(folder) / "key.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
