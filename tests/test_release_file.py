import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import release_file  # noqa: E402

VERSION_RS = '''//! What Minecraft 9.9 states about itself.

/// `WorldVersion.id()`.
pub const VERSION_ID: &str = "9.9";
/// `WorldVersion.name()`.
pub const VANILLA_VERSION: &str = "9.9 Pre-Release 1";
/// `SharedConstants.getProtocolVersion()`.
pub const PROTOCOL_VERSION: i32 = 801;
/// The Java major version the game runs on.
pub const JAVA_VERSION: u32 = 27;
'''
COMMIT = "0123456789abcdef0123456789abcdef01234567"


class ReleaseFileTest(unittest.TestCase):
    def test_the_file_names_the_build_the_game_and_the_contract(self):
        files = {release_file.VERSION_RS: VERSION_RS, release_file.RUST_VERSION: "1.99.1\n"}
        self.assertEqual(release_file.facts(files.__getitem__, "dev", "2026-11-16", COMMIT), {
            "contract": 1,
            "launcher_min": "1.0.0",
            "channel": "dev",
            "build": "dev-2026-11-16-0123456",
            "commit": COMMIT,
            "version_id": "9.9",
            "vanilla": "9.9 Pre-Release 1",
            "protocol": 801,
            "java_major": 27,
            "rust": "1.99.1",
        })

    def test_bad_inputs_are_refused(self):
        files = {release_file.VERSION_RS: VERSION_RS, release_file.RUST_VERSION: "1.99.1"}
        for channel, date, commit in [("release", "2026-11-16", COMMIT), ("nightly", "16.11.2026", COMMIT),
                                      ("nightly", "2026-11-16", COMMIT[:7])]:
            with self.subTest(channel=channel, date=date, commit=commit), self.assertRaises(SystemExit):
                release_file.facts(files.__getitem__, channel, date, commit)
        files[release_file.VERSION_RS] = "pub const VERSION_ID: &str = \"9.9\";\n"
        with self.assertRaises(SystemExit):
            release_file.facts(files.__getitem__, "nightly", "2026-11-16", COMMIT)

    def test_this_checkout_writes_a_file_from_its_own_commit(self):
        head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, check=True, capture_output=True,
                              text=True).stdout.strip()
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "sunrise-release.json"
            release_file.main(["write", "--channel", "nightly", "--date", "2026-11-16", "--commit", head, str(out)])
            written = json.loads(out.read_text())
        self.assertEqual(written["commit"], head)
        self.assertEqual(written["build"], f"nightly-2026-11-16-{head[:7]}")
        self.assertEqual(written["rust"], (ROOT / "tools/ci/rust-version").read_text().strip())
        self.assertRegex(written["version_id"], r"^\d+\.\d+")
        self.assertIsInstance(written["protocol"], int)


if __name__ == "__main__":
    unittest.main()
