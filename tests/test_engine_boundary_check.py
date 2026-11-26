"""The engine-boundary ratchet refuses new gameplay -> engine references and only shrinks."""
import unittest

from tools.ci import check_engine_boundary as boundary


CONFIG = {
    "modules": ["gfx", "audio", "window"],
    "scan": ["crates/sunrise-client/src/game/world"],
    "allowed": {},
}


class EngineBoundaryTests(unittest.TestCase):
    def test_qualified_and_grouped_imports_count(self):
        source = """
            use sunrise_client::{audio::Audio, chunk::Chunk};
            fn f() -> sunrise_client::gfx::Frame { todo!() }
            use crate::window::PointerSample;
        """
        self.assertEqual(boundary.referenced_modules(source, CONFIG["modules"]), {"audio", "gfx", "window"})

    def test_the_split_client_crates_count_as_engine_heads(self):
        source = """
            use sunrise_client_lib::audio::Audio;
            use sunrise_client_render::gfx::Frame;
            use sunrise_client_input::window::PointerSample;
            use sunrise_client_other::gfx::Frame;
        """
        self.assertEqual(
            boundary.referenced_modules(source, CONFIG["modules"]), {"audio", "gfx", "window"}
        )

    def test_a_submodule_named_like_an_engine_module_is_not_a_hit(self):
        source = "use crate::game::{gfx, audio}; use other::gfx::Frame;"
        self.assertEqual(boundary.referenced_modules(source, CONFIG["modules"]), set())

    def test_a_new_offender_fails(self):
        problems, notes = boundary.check(CONFIG, {"a.rs": ["gfx"]})
        self.assertEqual(len(problems), 1)
        self.assertIn("a.rs: references gfx", problems[0])
        self.assertEqual(notes, [])

    def test_a_recorded_offender_passes(self):
        config = dict(CONFIG, allowed={"a.rs": ["gfx"]})
        self.assertEqual(boundary.check(config, {"a.rs": ["gfx"]}), ([], []))

    def test_a_new_module_on_a_recorded_file_fails(self):
        config = dict(CONFIG, allowed={"a.rs": ["gfx"]})
        problems, _ = boundary.check(config, {"a.rs": ["gfx", "audio"]})
        self.assertIn("now also references audio", problems[0])

    def test_an_entry_that_shrank_is_a_note(self):
        config = dict(CONFIG, allowed={"a.rs": ["gfx", "audio"]})
        problems, notes = boundary.check(config, {"a.rs": ["gfx"]})
        self.assertEqual(problems, [])
        self.assertIn("no longer references audio", notes[0])

    def test_an_entry_that_went_away_is_a_note(self):
        config = dict(CONFIG, allowed={"a.rs": ["gfx"]})
        problems, notes = boundary.check(config, {})
        self.assertEqual(problems, [])
        self.assertIn("remove it", notes[0])

    def test_a_branch_may_not_add_or_widen_an_entry(self):
        base = dict(CONFIG, allowed={"a.rs": ["gfx"]})
        config = dict(CONFIG, allowed={"a.rs": ["gfx", "audio"], "b.rs": ["gfx"]})
        problems, _ = boundary.check(config, {"a.rs": ["gfx", "audio"], "b.rs": ["gfx"]}, base)
        self.assertTrue(any("b.rs: added" in problem for problem in problems))
        self.assertTrue(any("a.rs: widened" in problem for problem in problems))

    def test_the_scan_roots_and_modules_are_fixed_between_refs(self):
        base = dict(CONFIG, allowed={})
        config = dict(CONFIG, scan=["elsewhere"], allowed={})
        problems, _ = boundary.check(config, {}, base)
        self.assertIn("scan roots and modules are fixed", problems[0])


if __name__ == "__main__":
    unittest.main()
