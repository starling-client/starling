"""Exercise screen release cancellation and its actual mapping dispatch."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from native_hook_sources import native_hook_sources

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class ScreenKeyReleasedHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_cancelled_release_still_releases_every_binding(self):
        client = ROOT / "crates/sunrise-client/src"
        source = "".join((client / path).read_text(encoding="utf-8")
                         for path in ("game/runtime/window_handler.rs", "game.rs"))
        play_input = (client / "game/interface/play_input.rs").read_text(encoding="utf-8")
        key_codes = (client / "game/interface/key_codes.rs").read_text(encoding="utf-8")
        key_codes = key_codes.replace("\npub(super) fn", "\n    pub(super) fn")
        gate = method(native_hook_sources(), "screen_key_released")
        mappings = "\n".join(method(play_input, name) for name in (
            "mapped_key_event", "remove_device_key", "sync_device_key",
        ))
        buttons = "\n".join(method(key_codes, name) for name in (
            "keyboard_button", "gameplay_has_keyboard", "container_key_clones",
        ))
        start = source.index("            if let WindowEvent::KeyboardInput", source.index("let result: Result<()> = (|| {"))
        opening = source.index("{", source.index("&& self.screen_key_released(key)", start))
        end, depth = opening + 1, 1
        while depth:
            depth += (source[end] == "{") - (source[end] == "}")
            end += 1
        dispatch = source[start:end]
        fixture = r"""
use std::{cell::Cell, collections::HashSet};
type KeyCode = i32;
type Result<T> = std::result::Result<T, ()>;
#[derive(Clone, Copy, PartialEq)] enum ElementState { Pressed, Released }
#[derive(Clone, Copy)] enum PhysicalKey { Code(i32), Unidentified }
enum WindowEvent { KeyboardInput { event: KeyEvent }, Other }
struct KeyEvent { state: ElementState, physical_key: PhysicalKey }
#[derive(Clone, Copy, Debug, PartialEq)] enum KeyAction { Press, Release, Repeat }
#[derive(Clone, Copy, Debug, PartialEq)] enum MouseButton { Left, Middle }
#[derive(Clone, Copy, PartialEq)] enum Screen { Playing, Options }
fn glfw_modifiers(bits: u32) -> u32 { bits }
BUTTONS
mod keybinds {
    pub fn key_name(key: i32) -> Option<&'static str> { (key == 1).then_some("key.keyboard.a") }
    pub struct VanillaMappingSlot;
    impl VanillaMappingSlot {
        pub fn by_default_key(key: i32) -> Option<Self> { (key == 1).then_some(Self) }
    }
}
mod sunrise_client { pub(crate) use crate::keybinds; }
enum Source { Keyboard }
struct Bindings;
impl Bindings {
    fn translate_all(&self, _: i32) -> Vec<std::result::Result<i32, MouseButton>> {
        vec![Ok(10), Ok(11), Err(MouseButton::Left)]
    }
}
struct Settings { keys: Bindings }
struct MappingState { is_down: bool }
struct VanillaMappings { movement: MappingState, events: Vec<(KeyAction, bool)> }
impl VanillaMappings {
    fn state_mut(&mut self, _: keybinds::VanillaMappingSlot) -> &mut MappingState { &mut self.movement }
    fn key_event(&mut self, _: &Bindings, name: &str, action: KeyAction, screen_open: bool) {
        assert_eq!(name, "key.keyboard.a");
        self.events.push((action, screen_open));
    }
}
struct Replay;
impl Replay { fn playing(&self) -> bool { false } }
#[derive(Debug, PartialEq)] enum Binding { Key(i32), Mouse(MouseButton) }
struct Game { keys: HashSet<i32>, settings: Settings, screen: Screen, title: Option<()>, modifiers: u32,
    vanilla_mappings: VanillaMappings, replay: Replay,
    open: bool, cancel: bool, calls: Cell<usize>, handled: usize, released: Vec<Binding> }
impl Game {
    fn screen_over_playing(&self) -> bool { false }
    fn mod_mappings_screen_open(&self) -> bool {
        !gameplay_has_keyboard(self.screen, self.title.is_some(), self.screen_over_playing())
    }
    fn screen_view_now(&self) -> Option<&'static str> { self.open.then_some("options") }
    fn native_screen_key_released_pre(&self, screen: &&str, name: &str, modifiers: u32, canceled: &mut bool) {
        assert_eq!((*screen, name, modifiers), ("options", "key.keyboard.a", 5));
        self.calls.set(self.calls.get() + 1); *canceled = self.cancel;
    }
    fn mapped_key(&mut self, key: i32, pressed: bool, repeat: bool, raw: i32, _: Source) -> Result<()> {
        assert!(!pressed && !repeat && raw == 1); self.released.push(Binding::Key(key)); Ok(())
    }
    fn mouse_mapping(&mut self, button: MouseButton, pressed: bool, _: Source) -> Result<()> {
        assert!(!pressed); self.released.push(Binding::Mouse(button)); Ok(())
    }
    fn inventory_pick_key(&mut self) { panic!("release cannot clone") }
    GATE
    MAPPINGS
    fn dispatch(&mut self, event: WindowEvent) -> Result<()> {
        DISPATCH
        self.handled += 1; Ok(())
    }
}
#[test]
fn release_and_screen_gates() {
    for (open, cancel, state, key, expected_calls, handled) in [
        (true, true, ElementState::Released, PhysicalKey::Code(1), 1, 0),
        (true, false, ElementState::Released, PhysicalKey::Code(1), 1, 1),
        (false, true, ElementState::Released, PhysicalKey::Code(1), 0, 1),
        (true, true, ElementState::Pressed, PhysicalKey::Code(1), 0, 1),
        (true, true, ElementState::Released, PhysicalKey::Code(2), 0, 1),
        (true, true, ElementState::Released, PhysicalKey::Unidentified, 0, 1)] {
        let mut game = Game { keys: HashSet::from([1]), settings: Settings { keys: Bindings }, screen: Screen::Options,
            vanilla_mappings: VanillaMappings { movement: MappingState { is_down: true }, events: vec![] },
            replay: Replay, title: None, modifiers: 5, open, cancel, calls: Cell::new(0), handled: 0, released: vec![] };
        game.dispatch(WindowEvent::KeyboardInput { event: KeyEvent { state, physical_key: key } }).unwrap();
        assert_eq!((game.calls.get(), game.handled), (expected_calls, handled));
        if handled == 0 {
            assert!(!game.keys.contains(&1));
            assert!(!game.vanilla_mappings.movement.is_down);
            assert_eq!(game.vanilla_mappings.events, [(KeyAction::Release, true)]);
            assert_eq!(game.released, [Binding::Key(10), Binding::Key(11)]);
        }
        game.dispatch(WindowEvent::Other).unwrap();
        game.released.clear();
        game.screen = Screen::Playing;
        game.mapped_key_event(1, false, false).unwrap();
        assert_eq!(game.released, [Binding::Key(10), Binding::Key(11), Binding::Mouse(MouseButton::Left)]);
        game.released.clear();
        game.title = Some(());
        game.mapped_key_event(1, false, false).unwrap();
        assert_eq!(game.released, [Binding::Key(10), Binding::Key(11)]);
    }
}
"""
        fixture = (fixture.replace("GATE", gate.replace("super::glfw_modifiers", "glfw_modifiers"))
                   .replace("MAPPINGS", mappings).replace("DISPATCH", dispatch).replace("BUTTONS", buttons))
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "release.rs", Path(folder) / "release.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
