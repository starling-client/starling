"""Opt-in real-tool checks; the supplied binary must be pinned as the script documents.

HORIZON_MIXINS_TEST_TOOL=/absolute/path/to/horizon-mixins \\
    python3 -m unittest tools.ci.test_mixins_area_execution -v
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from tools import lab

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "tools/ci/mixins_area_execution.py"
TOOL = os.environ.get("HORIZON_MIXINS_TEST_TOOL")


@unittest.skipUnless(TOOL, "Set HORIZON_MIXINS_TEST_TOOL to the pinned external binary")
class MixinsAreaExecutionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tool = Path(TOOL).resolve(strict=True)
        cls.cargo = shutil.which("cargo")
        if cls.cargo is None:
            raise RuntimeError("cargo is required for the opted-in execution tests")
        (ROOT / "target").mkdir(exist_ok=True)
        cls.scratch = Path(tempfile.mkdtemp(prefix="test-mixins-area-", dir=ROOT / "target"))
        cls.script = SCRIPT.read_text(encoding="utf-8")

    def execute(self, name, old=None, new=None):
        case = self.scratch / name
        case.mkdir()
        text = self.script
        if old is not None:
            self.assertEqual(text.count(old), 1, "Mutant must change exactly one site")
            text = text.replace(old, new)
        script = case / "proof.py"
        script.write_text(text, encoding="utf-8")
        observation = case / "build-input.json"
        # Observe deletion at the actual cargo build boundary, then run real Cargo.
        # No transformation, build or execution is replaced with a canned result.
        wrapper = case / "cargo"
        wrapper.write_text(f'''#!{sys.executable}
import json
from pathlib import Path
import subprocess
import sys
args = sys.argv[1:]
if args and args[0] == "build":
    manifest = Path(args[args.index("--manifest-path") + 1])
    prepared = manifest.parent.parent / "prepared"
    Path({str(observation)!r}).write_text(json.dumps({{"prepared_exists": prepared.exists()}}))
sys.exit(subprocess.call([{self.cargo!r}, *args]))
''', encoding="utf-8")
        wrapper.chmod(0o755)
        # Inherited homes are deliberate sentinels: the copied script must replace
        # them with its own fresh homes, resolving lab through --repo even here.
        inherited = {}
        for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                    "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
            home = case / "inherited" / key
            home.mkdir(parents=True)
            inherited[key] = home
        env = lab.game_env(os.environ, **inherited,
                           PATH=str(case) + os.pathsep + os.environ["PATH"])
        result = subprocess.run([sys.executable, str(script), "--repo", str(ROOT),
                                 "--tool", str(self.tool)], env=env, cwd=ROOT,
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                timeout=300)
        (case / "script.log").write_text(result.stdout, encoding="utf-8")
        first = result.stdout.splitlines()[0]
        self.assertTrue(first.startswith("Area execution proof: "), result.stdout)
        root = Path(first.removeprefix("Area execution proof: "))
        self.assertTrue(root.is_relative_to(ROOT / "target"))
        return result, root, observation

    def assert_relocated(self, root, observation):
        self.assertFalse(json.loads(observation.read_text())["prepared_exists"],
                         "Prepared input still existed when Cargo built the output")
        self.assertFalse((root / "prepared").exists())
        self.assertIn("Finished", (root / "build.log").read_text())

    def assert_environment(self, root):
        # Written by the compiled Rust executable, not the Python process launcher.
        observed = dict(line.split("=", 1) for line in (root / "execution-env.txt").read_text().splitlines())
        homes = {}
        for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                    "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
            expected = root / "homes/execute" / key
            homes[key] = expected
            self.assertTrue(expected.is_dir())
        # Use the existing isolation helper's flags, with independently expected
        # paths: no duplicate hand-maintained audio/self-update flag setters.
        self.assertEqual(observed, lab.game_env({}, **homes), "Executable inherited caller home")

    def test_baseline_preserves_source_and_executes_relocated_output(self):
        result, root, observation = self.execute("baseline")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assert_relocated(root, observation)
        self.assert_environment(root)
        self.assertIn("E_TARGET_NOT_FOUND", (root / "raw.log").read_text())
        self.assertEqual((root / "execute.log").read_text().strip(),
                         "AREA_AND_NATIVE_HOOK_HELPERS=211")
        client = root / "source/crates/horizon-client/src"
        for name in ("game.rs", "game/native_hooks/mod.rs"):
            self.assertIn('env!("OUT_DIR")', (client / name).read_text())
        self.assertEqual((client / "game/items/.game-area").read_bytes(), b"")

    def test_each_helper_effect_is_required_by_runtime_assertion(self):
        for name, old, new, value in [
            ("area", '"start_eating", 97)', '"start_eating", 0)', 114),
            ("native", '"native_right_click_block", 103)', '"native_right_click_block", 0)', 108),
        ]:
            with self.subTest(helper=name):
                result, root, observation = self.execute(name, old, new)
                self.assertNotEqual(result.returncode, 0)
                self.assert_relocated(root, observation)
                self.assertIn("execute failed", result.stdout)
                execution = (root / "execute.log").read_text()
                self.assertIn("assertion `left == right` failed", execution)
                self.assertIn(f"left: {value}", execution)
                self.assertIn("right: 211", execution)

    def test_source_map_guard_rejects_changed_added_and_deleted_files(self):
        for name, change in [
            ("changed", '(client / "src/game.rs").write_bytes((client / "src/game.rs").read_bytes() + b"\\n// changed\\n")'),
            ("added", '(source / "unexpected").write_bytes(b"extra")'),
            ("deleted", '(source / "Cargo.lock").unlink()'),
        ]:
            with self.subTest(change=name):
                # Mutate only after real transformation/build/execution, preserving
                # both original OUT_DIR includes: substring guards would miss this.
                guard = "    if source_files() != original:\n"
                result, root, observation = self.execute(name, guard, f"    {change}\n" + guard)
                self.assertNotEqual(result.returncode, 0)
                self.assert_relocated(root, observation)
                self.assertEqual((root / "execute.log").read_text().strip(),
                                 "AREA_AND_NATIVE_HOOK_HELPERS=211")
                self.assertIn("Original download source file map was modified", result.stdout)

    def test_environment_observation_rejects_inherited_home_mutant(self):
        result, root, observation = self.execute(
            "inherited-env", "        env = lab.game_env(os.environ, **homes, CARGO_TARGET_DIR=build)\n",
            '        env = {**os.environ, "CARGO_TARGET_DIR": str(build)}\n')
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assert_relocated(root, observation)
        self.assertEqual((root / "execute.log").read_text().strip(),
                         "AREA_AND_NATIVE_HOOK_HELPERS=211")
        with self.assertRaisesRegex(AssertionError, "inherited caller home"):
            self.assert_environment(root)

    def test_relocation_observation_rejects_retained_input_mutant(self):
        result, root, observation = self.execute(
            "retained", "    shutil.rmtree(prepared)\n", "    # Mutant retains prepared input.\n")
        self.assertEqual(result.returncode, 0, result.stdout)
        # This is not a compiler-error death: the build and execution pass, but
        # the independent boundary observation rejects the invalid relocation proof.
        self.assertEqual((root / "execute.log").read_text().strip(),
                         "AREA_AND_NATIVE_HOOK_HELPERS=211")
        with self.assertRaisesRegex(AssertionError, "still existed"):
            self.assert_relocated(root, observation)


if __name__ == "__main__":
    unittest.main()
