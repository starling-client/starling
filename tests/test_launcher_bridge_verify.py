"""launcher-bridge.yml: sign_release.sh signs a launcher release for both tags, and verify_launcher_release.sh accepts only what was signed."""
import base64
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SIGN = ROOT / "tools/ci/sign_release.sh"
VERIFY = ROOT / "tools/ci/verify_launcher_release.sh"
ASSETS = ["sunrise-launcher-linux-x86_64.tar.gz", "sunrise-launcher-setup.exe"]


@unittest.skipUnless(shutil.which("openssl") and shutil.which("bash") and shutil.which("sha256sum"), "needs bash, OpenSSL and sha256sum")
class LauncherBridge(unittest.TestCase):
    def setUp(self):
        self.work = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.work)
        self.dist = self.work / "dist"
        self.dist.mkdir()
        for name in ASSETS:
            (self.dist / name).write_bytes(name.encode())

    def sign(self, tag="launcher@1.1.0", launcher_for="v1.1.0"):
        files = [str(self.dist / name) for name in ASSETS]
        env = {"PATH": "/usr/bin:/bin", "EPHEMERAL": "yes", "LAUNCHER_FOR": launcher_for}
        result = subprocess.run(["bash", SIGN, tag, *files], env=env, capture_output=True, text=True)
        if result.returncode == 0:
            self.keys = self.work / "keys.txt"
            shutil.move(self.dist / "ephemeral.pub", self.keys)
        return result.returncode == 0

    def accepts(self, *args) -> bool:
        return subprocess.run(["bash", VERIFY, self.dist, *map(str, args)], capture_output=True).returncode == 0

    def test_both_signatures_check_with_the_key_that_made_them(self):
        self.assertTrue(self.sign())
        self.assertTrue(self.accepts("launcher@1.1.0", self.keys, self.dist / "SHA256SUMS.sig"))
        self.assertTrue(self.accepts("v1.1.0", self.keys, self.dist / "SHA256SUMS.launcher.sig"))

    def test_a_signature_never_passes_for_the_other_tag_or_another_version(self):
        self.assertTrue(self.sign())
        self.assertFalse(self.accepts("v1.1.0", self.keys, self.dist / "SHA256SUMS.sig"))
        self.assertFalse(self.accepts("launcher@1.1.0", self.keys, self.dist / "SHA256SUMS.launcher.sig"))
        self.assertFalse(self.accepts("v1.2.0", self.keys, self.dist / "SHA256SUMS.launcher.sig"))

    def test_an_unlisted_or_changed_file_is_refused(self):
        self.assertTrue(self.sign())
        (self.dist / "sunrise-launcher-extra.zip").write_bytes(b"unsigned")
        self.assertFalse(self.accepts())
        (self.dist / "sunrise-launcher-extra.zip").unlink()
        self.assertTrue(self.accepts())
        (self.dist / ASSETS[0]).write_bytes(b"changed")
        self.assertFalse(self.accepts())

    def test_the_launcher_tag_must_be_the_bridged_version(self):
        self.assertFalse(self.sign(launcher_for="v1.2.0"))
        self.assertFalse(self.sign(tag="nightly", launcher_for="v1.1.0"))
        self.assertFalse(self.sign(launcher_for="v1.1.0-rc1"))

    def test_a_release_key_from_the_environment_signs_both(self):
        der = subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-outform", "DER"], check=True, capture_output=True).stdout
        public = subprocess.run(["openssl", "pkey", "-inform", "DER", "-pubout", "-outform", "DER"], input=der, check=True, capture_output=True).stdout[-32:]
        keys = self.work / "release-keys.txt"
        keys.write_text(base64.b64encode(public).decode() + "\n")
        env = {"PATH": "/usr/bin:/bin", "LAUNCHER_FOR": "v1.1.0", "RELEASE_SIGNING_KEY": base64.b64encode(der).decode()}
        files = [str(self.dist / name) for name in ASSETS]
        subprocess.run(["bash", SIGN, "launcher@1.1.0", *files], env=env, check=True, capture_output=True)
        self.assertFalse((self.dist / "ephemeral.pub").exists())
        self.assertTrue(self.accepts("launcher@1.1.0", keys, self.dist / "SHA256SUMS.sig"))
        self.assertTrue(self.accepts("v1.1.0", keys, self.dist / "SHA256SUMS.launcher.sig"))

    def test_without_a_listed_key_nothing_passes(self):
        self.assertTrue(self.sign())
        empty = self.work / "empty.txt"
        empty.write_text("# no key\n")
        self.assertFalse(self.accepts("v1.1.0", empty, self.dist / "SHA256SUMS.launcher.sig"))


if __name__ == "__main__":
    unittest.main()
