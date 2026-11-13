"""What horizon-mixins refuses in the functions native mods hook, kept out of Starling's own changes.

The upstream game has none of this in these places, and each one failed every mod hooking there.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def body(path, signature):
    """The braces of the function with this signature."""
    text = (ROOT / path).read_text(encoding="utf-8").replace("\r\n", "\n")
    at = text.index(signature)
    depth, start = 0, None
    parens = 0
    for i in range(at, len(text)):
        c = text[i]
        parens += (c == "(") - (c == ")")
        if c == "{" and parens == 0:
            depth += 1
            start = i if start is None else start
        elif c == "}" and parens == 0:
            depth -= 1
            if depth == 0:
                return text[start : i + 1]
    raise AssertionError(signature)


class Targets(unittest.TestCase):
    def test_game_has_no_macro_among_its_items(self):
        text = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        found = re.findall(r"^[a-zA-Z_][\w:]*!\s*[({\[]", text, re.M)
        # Upstream Horizon's own two since sync 6 (the generated module list and the attachment-lifetime impl);
        # Starling adds none. UNVERIFIED: whether the mixin tool still hooks functions under `game` with them there.
        # The sync's mods sweep has to build a native mod that targets one before this exception stays.
        found = [m for m in found if m not in ("include!(", "horizon_mod_runtime::impl_attachment_lifetime!(")]
        self.assertEqual(found, [], "E_UNSUPPORTED_MACRO for every function under `game`")

    def test_hooked_functions_have_no_cfg_inside(self):
        for path, signature in (
            ("crates/horizon-client/src/game/runtime/window_handler.rs", "fn handle_window_event("),
            ("crates/horizon-client/src/game/runtime/window_handler.rs", "fn handle_window_event_inner("),
            ("crates/horizon-client/src/game/interface/play_input.rs", "fn play_mouse("),
            ("crates/horizon-client/src/game/interface/play_input.rs", "fn mapped_key("),
            ("crates/horizon-client/src/game/interface/inventory_input.rs", "fn has_shift_down("),
            ("crates/horizon-client/src/game/interface/item_draw.rs", "fn has_shift_down("),
            ("crates/horizon-client/src/game/interface/inventory_input.rs", "fn has_control_down("),

            ("crates/horizon-client-render/src/options_ui.rs", "fn layout("),
        ):
            self.assertNotRegex(body(path, signature), r"#\s*\[\s*cfg|\bcfg\s*!", signature)

    def test_the_catalog_has_the_file_horizon_mixins_looks_for(self):
        # It doesn't follow the #[path] in starling-catalog: E_INVALID_PATH for every native mod.
        self.assertTrue((ROOT / "crates/starling-catalog/src/launcher/browser.rs").is_file())

    def test_remote_send_state_does_not_log(self):
        text = body("crates/horizon-client/src/game/network/remote/mod.rs", "fn remote_send_state(")
        self.assertNotIn("log::", text)


if __name__ == "__main__":
    unittest.main()
