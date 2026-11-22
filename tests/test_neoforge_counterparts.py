"""The counterparts chapter is a live, virtual chapter: xtask renders it from the catalog, the
book preprocessor expands the draft at its published URL, and nothing is committed.

The declarations' parsing and row grouping are tested in Rust (`sunrise-mod-api-gen`'s
`counterparts` module); this file covers the mdBook side.
"""
from pathlib import Path
import json
import os
import shlex
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
PAGE = "reference/neoforge-counterparts.md"

with patch.object(sys, "path", [*sys.path, str(ROOT / "tools")]):
    import modding_hook_notes as notes


class Counterparts(unittest.TestCase):
    def book(self):
        return {"sections": [{"Chapter": dict(name="Reference", path="reference/api.md",
            content="", number=[1], parent_names=[], sub_items=[{"Chapter": dict(
                name="NeoForge counterparts", path=None, source_path=None, content="", number=[1, 2],
                parent_names=["Reference"], sub_items=[])}])}]}

    def chapter(self):
        return notes.chapter({"root": str(ROOT / "docs/modding")}, self.book(), "counterparts",
            "NeoForge counterparts", PAGE)

    def test_virtual_reference_keeps_published_url_navigation_and_generated_content(self):
        with patch.object(notes, "generated", return_value="# NeoForge counterparts\n\nFresh.") as generated:
            book = self.chapter()
        chapter = book["sections"][0]["Chapter"]["sub_items"][0]["Chapter"]
        self.assertEqual(generated.call_args.args[1], "counterparts")
        self.assertEqual(chapter["path"], PAGE)
        self.assertIsNone(chapter["source_path"])
        self.assertEqual(chapter["number"], [1, 2])
        self.assertEqual(chapter["parent_names"], ["Reference"])
        self.assertIn("Fresh.", chapter["content"])
        self.assertFalse((ROOT / "docs/modding/src" / PAGE).exists())

    def test_missing_draft_chapter_fails_instead_of_dropping_the_reference(self):
        with patch.object(notes, "generated", return_value="rendered"):
            with self.assertRaisesRegex(AssertionError, "one draft"):
                notes.chapter({"root": "/checkout/docs/modding"}, {"sections": []},
                    "counterparts", "NeoForge counterparts", PAGE)

    def test_catalog_failure_is_not_silently_replaced_with_an_empty_page(self):
        with patch.object(notes.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "cargo")):
            with self.assertRaises(subprocess.CalledProcessError):
                self.chapter()

    def test_uses_mdbook_root_not_callers_working_directory(self):
        with patch.object(notes.subprocess, "run", return_value=subprocess.CompletedProcess(
                [], 0, stdout="# NeoForge counterparts\n\nLatest.")) as run:
            result = notes.chapter({"root": "/checkout/docs/modding"}, self.book(),
                "counterparts", "NeoForge counterparts", PAGE)
        self.assertEqual(run.call_args.kwargs["cwd"], Path("/checkout").resolve())
        self.assertEqual(run.call_args.args[0][-2:], ["mod-api", "counterparts"])
        self.assertTrue(run.call_args.kwargs["check"])
        self.assertIn("Latest.", result["sections"][0]["Chapter"]["sub_items"][0]["Chapter"]["content"])

    def test_book_command_accepts_the_mdbook_protocol_outside_the_checkout(self):
        config = tomllib.loads((ROOT / "docs/modding/book.toml").read_text())
        command = shlex.split(config["preprocessor"]["neoforge-counterparts"]["command"])
        context = {"root": str(ROOT / "docs/modding")}
        with tempfile.TemporaryDirectory() as elsewhere:
            # A stand-in `cargo` answers for xtask, which would otherwise compile here: it renders the page only when
            # asked from the checkout for `mod-api counterparts`. The real page is the Rust tests' (`counterparts`
            # in sunrise-mod-api-gen) and the modding-book workflow's `mdbook build`, which runs this command.
            cargo = Path(elsewhere, "bin", "cargo")
            cargo.parent.mkdir()
            cargo.write_text(f"""#!{sys.executable}
import os, sys
assert os.getcwd() == {str(ROOT)!r}, os.getcwd()
assert sys.argv[1:] == ["run", "--quiet", "-p", "xtask", "--", "mod-api", "counterparts"], sys.argv
print("# NeoForge counterparts\\n\\n| `item_decorations` | client | `IItemDecorator.render` |")
""")
            cargo.chmod(0o755)
            env = {**os.environ, "PATH": f"{cargo.parent}{os.pathsep}{os.environ['PATH']}"}
            supported = subprocess.run([*command, "supports", "html"], cwd=elsewhere, env=env,
                check=True, capture_output=True, text=True)
            self.assertEqual(supported.stdout, "")
            result = subprocess.run(command, cwd=elsewhere, env=env, check=True, capture_output=True,
                text=True, input=json.dumps([context, self.book()]))
        book = json.loads(result.stdout)
        chapter = book["sections"][0]["Chapter"]["sub_items"][0]["Chapter"]
        self.assertEqual(chapter["path"], PAGE)
        self.assertIn("| `item_decorations` | client | `IItemDecorator.render` |", chapter["content"])


if __name__ == "__main__":
    unittest.main()
