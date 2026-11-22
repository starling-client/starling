from pathlib import Path
import tempfile
import unittest

from tools.hotspots import check_stand_ins as check

HOOK = '''pub trait Hooks {
    /// A doc comment that names `Doc` and "Quoted" types is not code.
    #[hook(side = "server", contract = "listen", neoforge = "X.y")]
    fn placed(stack: &ItemStack, face: Face, pos: [i32; 3], map: Option<Vec<Known>>) -> Reply;
}
'''


class StandInsTest(unittest.TestCase):
    def setUp(self):
        self._dir = tempfile.TemporaryDirectory()
        self.addCleanup(self._dir.cleanup)
        self.root = Path(self._dir.name)
        self.write(f"{check.FIXTURES}/shared.rs", "//! Shared.\nmod stubs {\n    pub(super) use super::Api;\n}\n")
        self.write(f"{check.FIXTURES}/api/ItemStack.rs", "pub struct ItemStack;\nimpl ItemStack {\n    fn n(&self) {}\n}\n")
        self.write(f"{check.FIXTURES}/stubs/Known.rs", "pub(super) struct Known;\n")
        self.write(f"{check.FIXTURES}/stubs/Reply.rs", "pub(super) use crate::tests::mod_api::Reply;\n"
                   "pub(super) enum Reply {\n    A,\n}\n")
        self.write(f"{check.FIXTURES}/placed.rs", "mod api {\n    pub struct Face;\n}\n")
        self.write(f"{check.HOOKS}/placed.rs", HOOK)

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_a_clean_tree_passes(self):
        self.assertEqual(check.problems(self.root), [])

    def test_a_stand_in_file_defines_the_item_it_is_named_after(self):
        self.write(f"{check.FIXTURES}/stubs/Known.rs", "pub(super) struct Known;\npub(super) struct Extra;\n")
        self.write(f"{check.FIXTURES}/api/Wrong.rs", "pub struct Right;\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 2, found)
        self.assertIn("stubs/Known.rs: a stand-in file defines exactly the item it is named after, `Known`; it "
                      "defines `Known`, `Extra`", found[0])
        self.assertIn("api/Wrong.rs", found[1])

    def test_a_name_lives_in_one_folder(self):
        self.write(f"{check.FIXTURES}/api/Known.rs", "pub struct Known;\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("`Known` has a stand-in in both", found[0])

    def test_shared_rs_keeps_no_stand_ins(self):
        self.write(f"{check.FIXTURES}/shared.rs",
                   "//! Shared.\nmod stubs {\n    pub(super) use super::Api;\n    pub(super) struct Back;\n}\n"
                   "mod api {\n    pub struct A;\n}\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 2, found)
        self.assertIn("`mod stubs` defines `Back`; only `use` lines stay there. Move it to "
                      f"{check.FIXTURES}/stubs/Back.rs", found[0])
        self.assertIn("`mod api` is back", found[1])

    def test_a_hook_signature_type_needs_a_stand_in(self):
        (self.root / f"{check.FIXTURES}/placed.rs").unlink()
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("hook `placed` names type `Face`, which has no dispatch-test stand-in. Add "
                      f"{check.FIXTURES}/api/Face.rs", found[0])

    def test_only_types_used_by_generated_dispatch_need_stand_ins(self):
        self.write(f"{check.HOOKS}/engine.rs", '''pub trait Hooks {
    #[hook(target = "mod_render::render::draw")]
    fn draw(draw: &mut RawDraw<'_>);

    #[hook(target = "game::Game::native_tick", /* target = "mod_render::render::tick" */)]
    fn tick(#[engine] make: impl FnOnce(EngineKey, i32) -> EngineCall<'_>,
            directory: &std::path::Path, event: Missing) -> Reply;
}
''')
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("hook `tick` names type `Missing`", found[0])

    def test_a_missing_shared_rs_fails_naming_it(self):
        (self.root / f"{check.FIXTURES}/shared.rs").unlink()
        self.assertEqual(check.problems(self.root), [f"{check.FIXTURES}/shared.rs: missing; "
                                                     "tools/hotspots/check_stand_ins.py reads it. If it moved, "
                                                     "update the script's path."])

    def test_no_hooks_fail_instead_of_passing(self):
        self.write(f"{check.HOOKS}/placed.rs", "pub trait Hooks {}\n")
        found = check.problems(self.root)
        self.assertEqual(len(found), 1, found)
        self.assertIn("no `#[hook(...)] fn` found", found[0])

if __name__ == "__main__":
    unittest.main()
