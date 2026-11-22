"""A `windows` row can leave Linux coverage only with proof from its Cargo target root."""
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.ci import check_ignored_test_rows as check, ignored_shards, ignored_tests


class WindowsTargetCoverage(unittest.TestCase):
    def setUp(self):
        scratch = self.enterContext(tempfile.TemporaryDirectory())
        self.root = Path(scratch)
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/game"]\n')
        self.write("crates/game/Cargo.toml", '[package]\nname = "game"\n')
        self.enterContext(patch.object(ignored_shards, "WORKSPACES", (".",)))
        self.enterContext(patch.object(ignored_shards, "workspace_members", return_value={
            "game": ("crates/game", "."), "launcher": ("launcher", "launcher"),
        }))

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def problems(self, package="game", target="test:exporters"):
        row = ignored_tests.Row(package, "check_exporters", target, "daily", "daily.txt", "windows")
        return check.windows_target_problems([row], self.root)

    def test_a_windows_only_test_root_is_accepted(self):
        self.write("crates/game/tests/exporters.rs", '//! Check the exporters.\n#![cfg(windows)]\n'
                   '#[test]\n#[ignore = "daily"]\nfn check_exporters() {}\n')
        self.assertEqual(self.problems(), [])

    def test_a_linux_capable_test_cannot_leave_linux_coverage(self):
        self.write("crates/game/tests/exporters.rs", '// #![cfg(windows)] for a different target\n'
                   '#[test]\n#[ignore = "daily"]\nfn check_exporters() { #[cfg(unix)] unix_check(); }\n')
        self.assertIn("tests/exporters.rs to start with #![cfg(windows)]", self.problems()[0])

    def test_an_explicit_cargo_target_cannot_borrow_an_unrelated_files_gate(self):
        self.write("crates/game/Cargo.toml", '[package]\nname = "game"\n'
                   '[[test]]\nname = "exporters"\npath = "tests/alternate.rs"\n')
        self.write("crates/game/tests/exporters.rs", '#![cfg(windows)]\n')
        self.write("crates/game/tests/alternate.rs", '#[test]\nfn check_exporters() {}\n')
        self.assertIn("requires the Cargo target root to be crates/game/tests/exporters.rs", self.problems()[0])

    def test_only_integration_test_targets_are_supported(self):
        for target in ("lib", "bin:game"):
            with self.subTest(target=target):
                self.assertIn("requires a Cargo integration test target (test:NAME)", self.problems(target=target)[0])

    def test_the_launcher_workspace_is_not_supported(self):
        self.assertIn("only supported in the game workspace, not launcher", self.problems(package="launcher")[0])
