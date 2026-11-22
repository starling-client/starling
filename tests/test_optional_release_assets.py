"""Execute release shell steps with optional archives present and absent."""

import base64
import hashlib
from itertools import product
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = {
    "sunrise-linux-x86_64.tar.gz", "sunrise-windows-x64.zip", "sunrise-macos-arm64.zip",
    "sunrise-macos-x64.zip", "sunrise-server-linux-x86_64.tar.gz", "sunrise-source.tar.gz", "sunrise-release.json",
}
OPTIONAL = "sunrise-windows-arm64.zip"
MUSL = "sunrise-linux-musl-x86_64.tar.gz"
# The bash on PATH, as in test_pr_ready: Windows' CreateProcess looks in System32 before PATH, so a bare "bash"
# there is WSL's, which gets none of the steps' env and none of the host's paths.
BASH = shutil.which("bash")
POSIX_BASH = BASH is not None and not (
    os.name == "nt" and os.path.dirname(os.path.normcase(BASH)).endswith(os.path.normcase(r"\system32")))


def run_step(containing):
    workflow = (ROOT / ".github/workflows/nightly-build.yml").read_text()
    for match in re.finditer(r"^        run: \|\n((?:          .*\n|\n)+)", workflow, re.MULTILINE):
        script = "\n".join(line[10:] for line in match[1].splitlines())
        if containing in script:
            return script
    raise AssertionError(f"no workflow step containing {containing}")


@unittest.skipUnless(POSIX_BASH and shutil.which("openssl") and shutil.which("sha256sum"),
                     "release shell checks require a bash that is not WSL's, OpenSSL and sha256sum")
class OptionalReleaseAssetsTests(unittest.TestCase):
    def test_sign_and_publish_with_each_optional_archive_combination(self):
        for repository, arm64 in product(("sunrise-org/sunrise", "horizon-rust/horizon"), (False, True)):
            with self.subTest(repository=repository, arm64=arm64), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                dist = root / "dist"
                dist.mkdir()
                present = REQUIRED | ({OPTIONAL} if arm64 else set())
                # The fork's Nightly carries its SDK's index in its signed sums; upstream's has no SDK.
                if repository == "sunrise-org/sunrise":
                    present |= {"sdk.json"}
                # A musl archive must never be signed or uploaded by the main release jobs, even if present.
                for name in present | {MUSL}:
                    (dist / name).write_text(name)
                script_dir = root / "tools/ci"
                script_dir.mkdir(parents=True)
                shutil.copy(ROOT / "tools/ci/sign_release.sh", script_dir)
                # The steps' own env from nightly-build.yml: SHORT is the commit the title and
                # SHA256SUMS.v2.sig name (ALSO_FOR, nightly-<date>-<short>).
                env = dict(os.environ, GITHUB_REPOSITORY=repository, EPHEMERAL="yes", SHA="0123456789abcdef", SHORT="0123456", DATE="2026-11-04",
                           KIND="nightly", SIGN_FOR="nightly", ALSO_FOR="nightly-2026-11-04-0123456", SOURCE_TAG="nightly-test",
                           RELEASE_SIGNING_KEY="", RELEASE_SIGNING_KEY_NEXT="")
                signed = subprocess.run([BASH, "-e", "-c", run_step('sign_release.sh "$SIGN_FOR"')],
                                        cwd=root, env=env, text=True, capture_output=True)
                self.assertEqual(signed.returncode, 0, signed.stderr)
                # Windows' sha256sum defaults to binary mode, marked by a * before each name.
                sums = {name.removeprefix("*"): digest for digest, name in
                        (line.split() for line in (dist / "SHA256SUMS").read_text().splitlines())}
                self.assertEqual(sums, {name: hashlib.sha256(name.encode()).hexdigest() for name in present})
                self.assertTrue((dist / "SHA256SUMS.sig").stat().st_size)
                self.assertTrue((dist / "SHA256SUMS.v2.sig").stat().st_size)
                (dist / "smoke-test.json").write_text("{}")
                (dist / "nightly-notes.md").write_text("test")
                # Isolate GitHub and tag rotation; validate the exact attachment arguments
                # produced by the real workflow, including paths with display labels.
                stubs = '''
python3() { :; }
gh() {
  for arg in "$@"; do
    case "$arg" in dist/*)
      file=${arg%%#*}
      test -f "$file" || return 1
      printf '%s\\n' "$file" >> published
    esac
  done
}
'''
                published = subprocess.run([BASH, "-e", "-c", stubs + run_step("gh release create nightly")],
                                           cwd=root, env=env, text=True, capture_output=True)
                self.assertEqual(published.returncode, 0, published.stderr)
                files = {Path(name).name for name in (root / "published").read_text().splitlines()}
                self.assertEqual(files, present | {"SHA256SUMS", "SHA256SUMS.sig", "SHA256SUMS.v2.sig", "smoke-test.json",
                                                   "nightly-notes.md"})
                if not arm64:
                    self.assertIn(f"::warning::{OPTIONAL} is missing", signed.stdout)
                    self.assertIn(f"::warning::{OPTIONAL} is missing", published.stdout)

    def test_a_stable_release_has_a_nightlys_assets_both_signed_for_its_tag(self):
        # A launcher checks a stable release as signed_digest does: SHA256SUMS signed for "horizon release <tag>".
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            dist = root / "dist"
            dist.mkdir()
            present = REQUIRED | {OPTIONAL, "sdk.json"}
            for name in present | {MUSL}:
                (dist / name).write_text(name)
            (root / "tools/ci").mkdir(parents=True)
            shutil.copy(ROOT / "tools/ci/sign_release.sh", root / "tools/ci")
            tag = "26.3-0.1.0"
            env = dict(os.environ, GITHUB_REPOSITORY="sunrise-org/sunrise", EPHEMERAL="yes", SHA="0123456789abcdef",
                       SHORT="0123456", DATE="2026-11-20", KIND="stable", SIGN_FOR=tag, ALSO_FOR=tag, TAG=tag,
                       RELEASE_SIGNING_KEY="", RELEASE_SIGNING_KEY_NEXT="")
            signed = subprocess.run([BASH, "-e", "-c", run_step('sign_release.sh "$SIGN_FOR"')],
                                    cwd=root, env=env, text=True, capture_output=True)
            self.assertEqual(signed.returncode, 0, signed.stderr)
            message = root / "message"
            message.write_bytes(f"horizon release {tag}\n".encode() + (dist / "SHA256SUMS").read_bytes())
            # ephemeral.pub is the raw 32-byte key; OpenSSL takes it as DER SubjectPublicKeyInfo.
            spki = bytes.fromhex("302a300506032b6570032100") + base64.b64decode((dist / "ephemeral.pub").read_text())
            (root / "key.der").write_bytes(spki)
            for name in ("SHA256SUMS.sig", "SHA256SUMS.v2.sig"):
                lines = (dist / name).read_text().split()
                self.assertEqual(len(lines), 1, name)
                (root / "sig").write_bytes(base64.b64decode(lines[0]))
                checked = subprocess.run(["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", str(root / "key.der"),
                                          "-keyform", "DER", "-rawin", "-in", str(message), "-sigfile", str(root / "sig")],
                                         capture_output=True, text=True)
                self.assertEqual(checked.returncode, 0, f"{name}: {checked.stdout}{checked.stderr}")
            # Never signed for another tag beside its own.
            other = subprocess.run([BASH, "-e", "-c", run_step('sign_release.sh "$SIGN_FOR"')], cwd=root,
                                   env=dict(env, ALSO_FOR="26.3-0.1.1"), text=True, capture_output=True)
            self.assertNotEqual(other.returncode, 0)
            self.assertIn("nor a stable tag's own", other.stderr)
            (dist / "smoke-test.json").write_text("{}")
            (dist / "nightly-notes.md").write_text("test")
            subprocess.run([BASH, "-e", "-c", run_step('sign_release.sh "$SIGN_FOR"')], cwd=root, env=env, check=True,
                           capture_output=True)
            stubs = """
python3() { :; }
gh() {
  printf '%s\\n' "$*" >> command
  for arg in "$@"; do
    case "$arg" in dist/*) test -f "${arg%%#*}" || return 1; printf '%s\\n' "${arg%%#*}" >> published; esac
  done
}
"""
            published = subprocess.run([BASH, "-e", "-c", stubs + run_step('gh release create "$TAG" --verify-tag --latest=false')],
                                       cwd=root, env=env, text=True, capture_output=True)
            self.assertEqual(published.returncode, 0, published.stderr)
            files = {Path(name).name for name in (root / "published").read_text().splitlines()}
            self.assertEqual(files, present | {"SHA256SUMS", "SHA256SUMS.sig", "SHA256SUMS.v2.sig", "smoke-test.json",
                                               "nightly-notes.md"}, "the Nightly's asset set")
            command = (root / "command").read_text()
            self.assertIn(f"release create {tag} --verify-tag --latest=false", command)
            self.assertNotIn("--target", command)
            self.assertNotIn("--prerelease", command)

    def test_a_dev_build_is_signed_for_its_channel_and_published_under_its_own_tag(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            dist = root / "dist"
            dist.mkdir()
            present = REQUIRED | {OPTIONAL}
            for name in present | {MUSL}:
                (dist / name).write_text(name)
            (root / "tools/ci").mkdir(parents=True)
            shutil.copy(ROOT / "tools/ci/sign_release.sh", root / "tools/ci")
            env = dict(os.environ, GITHUB_REPOSITORY="horizon-rust/horizon", EPHEMERAL="yes", SHA="0123456789abcdef",
                       SHORT="0123456", DATE="2026-11-13", KIND="dev", SIGN_FOR="dev", ALSO_FOR="dev-2026-11-13-0123456",
                       TAG="dev-2026-11-13-0123456", RELEASE_SIGNING_KEY="", RELEASE_SIGNING_KEY_NEXT="")
            signed = subprocess.run([BASH, "-e", "-c", run_step('sign_release.sh "$SIGN_FOR"')],
                                    cwd=root, env=env, text=True, capture_output=True)
            self.assertEqual(signed.returncode, 0, signed.stderr)
            (dist / "smoke-test.json").write_text("{}")
            (dist / "nightly-notes.md").write_text("test")
            stubs = """
python3() { :; }
gh() { printf '%s\\n' "$@" > gh-args; for arg in "$@"; do case "$arg" in dist/*) test -f "$arg" || return 1; esac; done; }
"""
            published = subprocess.run([BASH, "-e", "-c", stubs + run_step('gh release create "$TAG"')],
                                       cwd=root, env=env, text=True, capture_output=True)
            self.assertEqual(published.returncode, 0, published.stderr)
            args = (root / "gh-args").read_text().splitlines()
            self.assertEqual(args[:3], ["release", "create", "dev-2026-11-13-0123456"])
            self.assertIn("Dev 2026-11-13 · 0123456", args)
            self.assertIn("--prerelease", args)
            self.assertNotIn("nightly", args)
            files = {Path(arg).name for arg in args if arg.startswith("dist/")}
            self.assertEqual(files, present | {"SHA256SUMS", "SHA256SUMS.sig", "SHA256SUMS.v2.sig", "smoke-test.json",
                                               "nightly-notes.md"})
            # A Dev build's version signed beside the nightly channel is refused before anything is signed.
            refused = subprocess.run([BASH, "-e", "-c", run_step('sign_release.sh "$SIGN_FOR"')], cwd=root,
                                     env=dict(env, KIND="nightly", SIGN_FOR="nightly"), text=True, capture_output=True)
            self.assertNotEqual(refused.returncode, 0)
