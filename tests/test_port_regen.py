import subprocess
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]

from tools import lab, port_regen as regen

def manifest(version, client, server, java=25, library="1"):
    return {"id": version, "downloads": {"client": {"sha1": client}, "server": {"sha1": server}},
            "javaVersion": {"majorVersion": java},
            "libraries": [{"downloads": {"artifact": {"path": "a/b.jar", "sha1": library}}}]}


LOCK = {"minecraft": {"version": "9.9", "manifest_sha1": "m3", "client_sha1": "c3", "server_sha1": "s3",
                      "libraries_sha256": lab.libraries_digest(manifest("9.9", "c3", "s3")), "java_major": 25},
        "vineflower": {"version": "1.12.0"}}


class PinTests(unittest.TestCase):
    def test_another_version_gets_a_new_block(self):
        new = manifest("9.10", "c4", "s4", 26)
        pinned = regen.pinned_lock(LOCK, new, "m4")
        self.assertEqual(pinned["minecraft"], {"version": "9.10", "manifest_sha1": "m4", "client_sha1": "c4",
                                               "server_sha1": "s4", "libraries_sha256": lab.libraries_digest(new),
                                               "java_major": 26})
        self.assertEqual(pinned["vineflower"], LOCK["vineflower"])

    def test_the_same_content_keeps_its_block_whatever_the_files_sha1(self):
        for sha1 in ["m3", "re-issued"]:
            self.assertIs(regen.pinned_lock(LOCK, manifest("9.9", "c3", "s3"), sha1), LOCK)

    def test_the_same_version_with_other_jars_or_libraries_is_repinned(self):
        self.assertEqual(regen.pinned_lock(LOCK, manifest("9.9", "c3b", "s3"), "m3b")["minecraft"]["client_sha1"],
                         "c3b")
        self.assertEqual(regen.pinned_lock(LOCK, manifest("9.9", "c3", "s3", library="2"), "m3c")["minecraft"]
                         ["manifest_sha1"], "m3c")


class CompileErrorTests(unittest.TestCase):
    OUTPUT = """/x/mods/sunrise/src/test/java/dev/sunriselab/parity/ParityData.java:45: error: cannot find symbol
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
                                                                                            ^
  symbol:   variable WORLD
  location: class RegistryLayer
/x/VanillaCommandParseOracle.java:12: error: incompatible types: int cannot be converted to String
2 errors
"""

    def test_names_the_symbols(self):
        self.assertEqual(regen.compile_errors(self.OUTPUT), [
            "ParityData.java:45: cannot find symbol: variable WORLD (class RegistryLayer)",
            "VanillaCommandParseOracle.java:12: incompatible types: int cannot be converted to String"])

    def test_nothing_from_other_failures(self):
        self.assertEqual(regen.compile_errors("expected the root's order to differ"), [])


class InterpreterTests(unittest.TestCase):
    def test_python_is_an_interpreter_that_runs(self):
        found = regen.python()
        done = subprocess.run([found, "-c", "import sys; sys.exit(sys.version_info < (3,))"],
                              capture_output=True)
        self.assertEqual(done.returncode, 0, f"{found} is not a Python 3 that runs")


class UsedThroughVersionPortTests(unittest.TestCase):
    """The module is used through `tools/version_port.py`; run on its own it says so."""

    def test_it_says_how_it_is_used(self):
        result = subprocess.run([sys.executable, str(ROOT / "tools/port_regen.py"), "--help"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("version_port.py", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
