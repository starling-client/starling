"""Runs tests/install_ps1_signature.ps1: packaging/server/install.ps1 installs only a release sunrise-verify checked.

PURPOSE: keep install.ps1's signature check and its refusals from regressing (docs/release-flow.md, "Signed checksums").
REFS: tests/install_ps1_signature.ps1, tests/test_install_sh_signature.py (the same for install.sh),
crates/sunrise-release-sign/tests/install_ps1.rs (the daily run's Windows leg sets SUNRISE_VERIFY_TEST_BIN).
Skips quietly off Windows, without PowerShell or OpenSSL 3, and without SUNRISE_VERIFY_TEST_BIN. Headless, no network.
"""

import os
import shutil
import subprocess
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("install_ps1_signature.ps1")


class InstallPs1Signature(unittest.TestCase):
    # install.ps1 installs the Windows server: junctions, %LOCALAPPDATA%, the user's PATH.
    @unittest.skipUnless(os.name == "nt", "install.ps1 is for Windows")
    @unittest.skipUnless(shutil.which("pwsh") or shutil.which("powershell"), "PowerShell needed")
    @unittest.skipUnless(os.environ.get("SUNRISE_VERIFY_TEST_BIN"), "SUNRISE_VERIFY_TEST_BIN (the daily run sets it) needed")
    def test_signature_cases(self):
        shell = shutil.which("pwsh") or shutil.which("powershell")
        run = subprocess.run(
            [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(SCRIPT)],
            capture_output=True,
            text=True,
            timeout=300,
        )
        if run.returncode == 77:
            self.skipTest(run.stdout.strip())
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)


if __name__ == "__main__":
    unittest.main()
