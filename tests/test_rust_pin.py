"""CI and the launcher's native-mod toolchain use one Rust: tools/ci/rust-version.

A workflow names no floating channel (`stable`, `beta`, `nightly`) and no release of its own. The allowed forms:
- `bash tools/ci/rustup_install.sh [rustup toolchain install options]`: installs the pin and makes it the job's
  toolchain, so `cargo`, `rustc` and `rustfmt` run bare;
- `dtolnay/rust-toolchain@master` or the same action pinned to a full commit SHA, with
  `toolchain: ${{ steps.<id>.outputs.version }}[-<target>]`, the output of a step that reads the pin file;
- `"$(cat …tools/ci/rust-version)"` (or a shell variable holding it) wherever a command needs the release spelled out.
"""

import os
import re
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
# Upstream's workflows, disabled on this fork and kept as upstream writes them for syncs: nothing runs them.
DISABLED = {"ci.yml", "launcher.yml", "mod-registry.yml", "pack-bench.yml"}
RELEASE = r"(?:stable|beta|nightly|\d+\.\d+(?:\.\d+)?)\b"
FORBIDDEN = [
    (re.compile(rf"rustup_install\.sh\s+{RELEASE}"), "rustup_install.sh takes the pin itself"),
    (re.compile(rf"(?:cargo|rustc|rustfmt|rustdoc)\s+\+{RELEASE}"), "a toolchain override"),
    (re.compile(rf"rustup\s+(?:toolchain\s+install|default|run|which\s+--toolchain|\S+\s+--toolchain)\s+{RELEASE}"),
     "a toolchain spelled out"),
    (re.compile(r"^\s*RUSTUP_TOOLCHAIN\s*:"), "a step's RUSTUP_TOOLCHAIN overrides the job's pin"),
    (re.compile(rf"^\s*toolchain:\s*{RELEASE}"), "a toolchain spelled out"),
]
RUST_TOOLCHAIN_ACTION = re.compile(r"dtolnay/rust-toolchain@([^\s'\"]*)")


def violations(name: str, text: str) -> list[str]:
    found = []
    lines = text.splitlines()
    for number, line in enumerate(lines, 1):
        code = line.split(" #", 1)[0] if not line.lstrip().startswith("#") else ""
        for pattern, why in FORBIDDEN:
            if pattern.search(code):
                found.append(f"{name}:{number}: {why}: {line.strip()}")
        if action := RUST_TOOLCHAIN_ACTION.search(code):
            if not re.fullmatch(r"master|[0-9a-f]{40}", action[1]):
                found.append(f"{name}:{number}: the action must use master or a full commit SHA: {line.strip()}")
                continue
            following = "\n".join(lines[number : number + 4])
            if not re.search(r"toolchain:\s*\$\{\{\s*steps\.\w+\.outputs\.version\s*\}\}", following):
                found.append(f"{name}:{number}: the action takes the pin from a step's output")
    return found


class RustPinTests(unittest.TestCase):
    def test_the_pin_is_one_exact_release(self):
        self.assertRegex((ROOT / "tools/ci/rust-version").read_text(), r"^\d+\.\d+\.\d+\n?$")

    def test_active_workflows_use_the_pin(self):
        found = []
        for path in sorted(WORKFLOWS.glob("*.yml")):
            if path.name not in DISABLED:
                found += violations(path.name, path.read_text())
        self.assertEqual(found, [])

    def test_the_check_catches_each_floating_form(self):
        for line in ("bash tools/ci/rustup_install.sh stable --profile minimal", "cargo +stable test",
                     "rustup run stable rustfmt x.rs", "rustup toolchain install 1.99.1 --profile minimal",
                     "    RUSTUP_TOOLCHAIN: stable", "- uses: dtolnay/rust-toolchain@stable",
                     "rustup which --toolchain 1.99.1 cargo", "  toolchain: 1.99.1"):
            with self.subTest(line=line):
                self.assertTrue(violations("x.yml", line))
        allowed = ('bash tools/ci/rustup_install.sh --profile minimal\n'
                   'rustup toolchain install "$(cat tools/ci/rust-version)" --profile minimal\n'
                   '- uses: dtolnay/rust-toolchain@master\n  with:\n    toolchain: ${{ steps.rust.outputs.version }}\n'
                   '# cargo +stable in a comment\n')
        self.assertEqual(violations("x.yml", allowed), [])

    def test_a_sha_pinned_action_still_requires_the_version_step(self):
        sha = "0123456789abcdef0123456789abcdef01234567"
        uses = f"- uses: dtolnay/rust-toolchain@{sha}"
        with_pin = "\n  with:\n    toolchain: ${{ steps.rust.outputs.version }}\n"
        self.assertEqual(violations("x.yml", uses + " # master" + with_pin), [])
        self.assertTrue(violations("x.yml", uses))
        self.assertTrue(violations("x.yml", uses[:-1] + with_pin))


class RustupInstallTests(unittest.TestCase):
    def test_a_missing_rustup_fails_at_once_without_retrying(self):
        with tempfile.TemporaryDirectory() as directory:
            # Only what the script needs besides rustup.
            for tool in ("tr", "dirname", "sleep"):
                os.symlink(shutil.which(tool), Path(directory, tool))
            started = time.monotonic()
            result = subprocess.run([shutil.which("bash"), str(ROOT / "tools/ci/rustup_install.sh"), "--profile", "minimal"],
                                    env={"PATH": directory}, capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 127)
            self.assertIn("rustup is not installed on this runner", result.stderr)
            self.assertNotIn("retrying", result.stderr)
            self.assertLess(time.monotonic() - started, 5)

if __name__ == "__main__":
    unittest.main()
