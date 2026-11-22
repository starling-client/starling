"""check_nested_locks.py fails on a nested workspace whose own Cargo.lock no longer matches its manifests."""
import contextlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

from tools.ci import check_nested_locks


def package(folder, name, extra=""):
    (folder / "src").mkdir(parents=True)
    (folder / "src/lib.rs").write_text("")
    (folder / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n{extra}')


@unittest.skipUnless(shutil.which("cargo") and shutil.which("git"), "needs cargo and git")
class NestedLockTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        # A game crate, and a nested workspace that takes it by path, as the telemetry server takes sunrise-core.
        package(self.root / "crates/core", "core")
        self.server = self.root / "crates/server"
        package(self.server, "server", '\n[workspace]\n\n[dependencies]\ncore = { path = "../core" }\n')
        # A package copied into another workspace before it builds: its lock is not this check's.
        package(self.root / "tools/copied", "copied", '\n[dependencies]\ncore = { path = "../../crates/core" }\n')
        (self.root / "tools/copied/Cargo.lock").write_text("version = 4\n")
        subprocess.run(["cargo", "generate-lockfile", "--offline", "--manifest-path", str(self.server / "Cargo.toml")],
                       check=True, capture_output=True)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        subprocess.run(["git", "add", "-A"], cwd=self.root, check=True)
        environment = mock.patch.dict(os.environ, {"CARGO_NET_OFFLINE": "true"})
        environment.start()
        self.addCleanup(environment.stop)

    def run_check(self):
        output, errors = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            status = check_nested_locks.main(self.root)
        return status, output.getvalue()

    def test_a_matching_lock_passes_and_a_crate_gaining_a_dependency_fails_without_changing_it(self):
        self.assertEqual(self.run_check(), (0, ""))
        package(self.root / "crates/extra", "extra")
        with (self.root / "crates/core/Cargo.toml").open("a") as manifest:
            manifest.write('\n[dependencies]\nextra = { path = "../extra" }\n')
        lock = (self.server / "Cargo.lock").read_text()
        status, output = self.run_check()
        self.assertEqual(status, 1)
        self.assertIn("::error file=crates/server/Cargo.lock::", output)
        self.assertIn("cargo update --workspace --manifest-path crates/server/Cargo.toml", output)
        self.assertNotIn("tools/copied", output)
        self.assertEqual((self.server / "Cargo.lock").read_text(), lock)


if __name__ == "__main__":
    unittest.main()
