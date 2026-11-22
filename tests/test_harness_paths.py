"""Every repository path a parity harness names exists, and every Cargo target a tool builds is in the package it names.

A harness hashes its sources to key its cached results, and the hash fails on the first missing file, before the
harness compares anything: eleven harnesses named files the game/ area moves had left behind (2026-11-16). Five
tools built `-p sunrise-client --bin sunrise-client` after the game binary moved to sunrise-game, so their harnesses
failed in the build (2026-11-16).
"""
import ast
import json
from pathlib import Path
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
# Where a harness's sources live; build output (target/, .local/) is made by the run.
SOURCES = ("crates/", "mods/", "tools/", "docs/", "xtask/")


class HarnessPaths(unittest.TestCase):
    def test_every_path_a_harness_names_exists(self):
        harnesses = sorted((ROOT / "tools").glob("test_*_parity.py"))
        self.assertTrue(harnesses)
        for harness in harnesses:
            for node in ast.walk(ast.parse(harness.read_text(encoding="utf-8"))):
                if not (isinstance(node, ast.Constant) and isinstance(node.value, str)):
                    continue
                path = node.value
                # A glob or template names no single file.
                if not path.startswith(SOURCES) or any(c in path for c in "*?{}<> \n"):
                    continue
                with self.subTest(harness=harness.name, line=node.lineno):
                    self.assertTrue((ROOT / path).exists(), f"{harness.name}:{node.lineno} names {path}")

    @unittest.skipUnless(shutil.which("cargo"), "needs cargo for the workspace's targets")
    def test_every_cargo_target_a_tool_builds_is_in_its_package(self):
        owners = {}
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT, text=True))
        for package in metadata["packages"]:
            for target in package["targets"]:
                for kind in target["kind"]:
                    owners.setdefault((kind, target["name"]), set()).add(package["name"])
        kinds = {"--bin": "bin", "--example": "example", "--test": "test"}
        tools = sorted(path for path in (ROOT / "tools").rglob("*.py") if "vendor" not in path.parts)
        for tool in tools:
            source = tool.read_text(encoding="utf-8")
            for node in ast.walk(ast.parse(source)):
                if not isinstance(node, ast.List):
                    continue
                words = [e.value if isinstance(e, ast.Constant) and isinstance(e.value, str) else None
                         for e in node.elts]
                packages = [words[i + 1] for i, word in enumerate(words[:-1]) if word in ("-p", "--package")]
                # Only a command spelled out in full: a package given by a variable can't be checked here.
                if "cargo" not in words or not packages or None in packages:
                    continue
                for flag, name in zip(words, words[1:]):
                    if flag not in kinds or name is None:
                        continue
                    owned = owners.get((kinds[flag], name), set())
                    # An example no package has, which the tool copies into place itself before building it
                    # (run_world_views.py stages examples/world_views.rs from a fixture).
                    if not owned and flag == "--example" and f"examples/{name}.rs" in source:
                        continue
                    with self.subTest(tool=tool.name, line=node.lineno, target=name):
                        self.assertTrue(owned & set(packages),
                                        f"{tool.relative_to(ROOT)}:{node.lineno} builds {flag} {name} from {packages}; "
                                        f"it is in {sorted(owned) or 'no package'}")


if __name__ == "__main__":
    unittest.main()
