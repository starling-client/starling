"""Lock files are opened without truncating: Windows refuses to open for writing a file another process has locked."""
import importlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tools import datafix_paths, datafix_seeds, parity

ROOT = Path(__file__).resolve().parents[1]

HOLDER = """
import sys
sys.path.insert(0, sys.argv[2])
from tools import file_lock
with open(sys.argv[1], "a") as handle:
    handle.write("held")
    handle.flush()
    file_lock.lock(handle)
    print("locked", flush=True)
    sys.stdin.read()
"""


class LockFileTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def holding(self, path):
        """Another process holds `path` locked, with "held" in it, until the returned process is stopped."""
        process = subprocess.Popen([sys.executable, "-c", HOLDER, str(path), str(ROOT)], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, text=True)
        self.addCleanup(process.wait)
        self.addCleanup(process.stdin.close)
        self.assertEqual(process.stdout.readline().strip(), "locked")
        return process

    def release(self, process):
        process.stdin.close()
        process.wait()

    def test_a_machine_slot_held_elsewhere_keeps_its_content(self):
        slots = self.root / "slots"
        slots.mkdir()
        held = self.holding(slots / "0.lock")
        with mock.patch.object(parity, "SHARED", self.root), mock.patch.object(parity, "MACHINE_JOBS", 2):
            slot = parity.machine_slot()
        self.assertEqual(Path(slot.name).name, "1.lock")
        slot.close()
        self.release(held)
        self.assertEqual((slots / "0.lock").read_text(), "held")

    def test_remembering_fresh_only_keeps_what_the_lock_file_holds(self):
        (self.root / "fresh-only.lock").write_text("held")
        parity.remember_fresh_only(self.root, {"sculk": {"fresh_rerun": "pass"}})
        self.assertEqual((self.root / "fresh-only.lock").read_text(), "held")

    def test_the_datafix_harness_keeps_what_its_generate_lock_holds(self):
        local_root = self.root / "datafix"
        local_root.mkdir()
        (local_root / "generate.lock").write_text("held")
        environment = {datafix_paths.VARIABLE: str(local_root), datafix_seeds.KEEP_VARIABLE: str(self.root / "keep")}
        # The harness reads its environment when imported: a fresh import, whose sys.path and sys.modules
        # entries are undone on the way out.
        with mock.patch.object(sys, "path", list(sys.path)), \
                mock.patch.dict(os.environ, environment, clear=True), \
                mock.patch.dict(sys.modules), \
                mock.patch.object(datafix_paths, "ROOT", self.root):
            sys.modules.pop("tools.test_datafix_parity", None)
            harness = importlib.import_module("tools.test_datafix_parity")
            with mock.patch.object(sys, "argv", ["test_datafix_parity.py", "--vanilla-only"]), \
                    mock.patch.object(datafix_seeds, "snapshot_complete", return_value=True), \
                    mock.patch.object(harness, "java_classpath", return_value=""), \
                    mock.patch.object(harness, "check_single_compounds"):
                harness.main()
        self.assertEqual((local_root / "generate.lock").read_text(), "held")


if __name__ == "__main__":
    unittest.main()
