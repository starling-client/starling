import contextlib
import io
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

from tests.test_ci_concurrency import ROOT, yaml
from tools import check_pe_arch


def pe(machine, reserve=None):
    image = bytearray(0x98)
    image[:2] = b"MZ"
    struct.pack_into("<I", image, 0x3C, 0x80)
    image[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<H", image, 0x84, machine)
    if reserve is not None:
        # PE32+ optional header begins at 0x98; reserve and commit are different fields.
        image.extend(bytearray(0xF0))
        struct.pack_into("<H", image, 0x94, 0xF0)
        struct.pack_into("<H", image, 0x98, 0x20B)
        struct.pack_into("<QQ", image, 0xE0, reserve, 4096)
    return image


class CheckPEArchTests(unittest.TestCase):
    def test_reads_x64_and_arm64_machine_values(self):
        self.assertEqual(check_pe_arch.pe_fields(pe(0x8664)), (0x8664, None))
        self.assertEqual(check_pe_arch.pe_fields(pe(0xAA64)), (0xAA64, None))

    def test_rejects_non_pe_and_truncated_headers(self):
        with self.assertRaisesRegex(ValueError, "MZ"):
            check_pe_arch.pe_fields(b"not PE")
        image = pe(0xAA64)
        struct.pack_into("<I", image, 0x3C, 0xFFFFFFF0)
        with self.assertRaisesRegex(ValueError, "PE signature"):
            check_pe_arch.pe_fields(image)

    def test_stack_check_requires_the_exact_reserve_not_the_commit(self):
        with tempfile.TemporaryDirectory() as folder:
            binary = Path(folder) / "game.exe"
            with contextlib.redirect_stdout(io.StringIO()):
                binary.write_bytes(pe(0x8664, 8388608))
                self.assertTrue(check_pe_arch.check(binary, "x64", stack_reserve=8388608))
            with contextlib.redirect_stderr(io.StringIO()) as stderr:
                binary.write_bytes(pe(0x8664, 1048576))
                self.assertFalse(check_pe_arch.check(binary, "x64", stack_reserve=8388608))
                self.assertIn("SizeOfStackReserve is 1048576, expected 8388608 bytes", stderr.getvalue())

    @unittest.skipUnless(os.name == "posix" and shutil.which("bash"), "Run the packaging steps in Bash")
    def test_packaging_checks_machine_without_imposing_the_workflow_revisions_stack(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/game-build.yml").read_text())
        build = workflow["jobs"]["build"]
        steps = build["steps"]
        start = next(i for i, step in enumerate(steps) if step.get("name", "").startswith("Build the official game"))
        end = next(i for i, step in enumerate(steps) if step.get("name") == "Verify Windows executable architecture")
        # Run the Windows steps between Cargo and the architecture check against pre-policy binaries.
        after_build = "\n".join(step["run"] for step in steps[start + 1:end + 1])
        for arch, machine, other_machine in (("x64", 0x8664, 0xAA64), ("arm64", 0xAA64, 0x8664)):
            row = next(row for row in build["strategy"]["matrix"]["include"] if row.get("pe_arch") == arch)
            commands = re.sub(r"\$\{\{ matrix\.(\w+) \}\}", lambda match: row[match[1]], after_build)
            with self.subTest(arch=arch), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                tools = root / ".workflow-tools/tools"
                tools.mkdir(parents=True)
                shutil.copyfile(ROOT / "tools/check_pe_arch.py", tools / "check_pe_arch.py")
                # Older game refs reserve 136 MiB. The official binary becomes the shipped client.
                for exe in ("sunrise-client", "sunrise-official", "sunrise-ctl", "sunrise-perf", "sunrise-prepare",
                            "sunrise-server", "sunrise-port-pack"):
                    (root / f"{exe}.exe").write_bytes(pe(machine, 136 * 1024 * 1024))
                env = {**os.environ, "BIN": ".", "RUNNER_OS": "Windows", "TEST_PYTHON": sys.executable}
                script = ["bash", "-ec", 'python() { "$TEST_PYTHON" "$@"; }\n' + commands]
                result = subprocess.run(script, cwd=root, env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                (root / "sunrise-official.exe").write_bytes(pe(other_machine, 136 * 1024 * 1024))
                result = subprocess.run(script, cwd=root, env=env, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("sunrise-client.exe: PE machine", result.stderr)


if __name__ == "__main__":
    unittest.main()
