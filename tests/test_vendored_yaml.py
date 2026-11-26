"""The vendored PyYAML loader refuses an archive that is not the pinned one, and puts nothing on sys.path then."""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from tools.ci import vendored_yaml

CI = Path(__file__).resolve().parents[1] / "tools/ci"
# Run in a child with -S: the parser must come from the archive, not from site packages.
PROBE = """
import sys
before = list(sys.path)
from tools.ci import vendored_yaml
try:
    vendored_yaml.load()
except ImportError as error:
    print("refused:", error)
    assert sys.path == before
else:
    raise SystemExit("loaded a changed archive")
"""


class VendoredYamlTests(unittest.TestCase):
    def test_the_pinned_archive_parses(self):
        yaml = vendored_yaml.load()
        self.assertEqual(yaml.load("a: [1, x]\non: y\n", Loader=yaml.BaseLoader), {"a": ["1", "x"], "on": "y"})

    def test_a_changed_archive_is_refused_before_it_is_imported(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp) / "tools/ci"
            folder.mkdir(parents=True)
            shutil.copyfile(CI / "vendored_yaml.py", folder / "vendored_yaml.py")
            shutil.copytree(CI / "vendor", folder / "vendor")
            with (folder / "vendor/pyyaml-6.0.3.zip").open("ab") as archive:
                archive.write(b"changed")
            result = subprocess.run([sys.executable, "-S", "-c", PROBE], cwd=temp, env={**os.environ, "PYTHONPATH": temp},
                                    capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("refused: vendored YAML parser checksum mismatch", result.stdout)


if __name__ == "__main__":
    unittest.main()
