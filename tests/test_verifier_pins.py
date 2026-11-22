"""The install scripts pin one `sunrise-verify`: the same version and the same SHA-256 per platform.

PURPOSE: install.sh and install.ps1 each carry the pin block tools/ci/pin_verifier.py rewrites; a hand edit of one
that leaves the other behind would make a platform check against a hash the other script doesn't trust, and the
updater must pin only what verifier.yml on main attested for the commit it was told.
REFS: tools/ci/pin_verifier.py, packaging/server/install.sh, packaging/server/install.ps1.
Headless, no network (the attestation check is replaced by a fake; its command is checked as text).
"""

import hashlib
import tempfile
import unittest
from pathlib import Path

from tools.ci import pin_verifier

SH, PS1 = pin_verifier.SCRIPTS
COMMIT = "0123456789abcdef0123456789abcdef01234567"
ASSETS = [
    "sunrise-verify-linux-aarch64",
    "sunrise-verify-linux-x86_64",
    "sunrise-verify-macos-arm64",
    "sunrise-verify-macos-x64",
    "sunrise-verify-windows-arm64.exe",
    "sunrise-verify-windows-x64.exe",
]


def attested(path, repository, commit):
    """A fake `gh attestation verify` that accepts everything."""


def not_attested(path, repository, commit):
    raise SystemExit(f"{path.name} has no valid build-provenance attestation")


class VerifierPins(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.release = self.root / "release"
        self.release.mkdir()
        self.binaries = {name: f"binary of {name}".encode() for name in ASSETS}
        for name, data in self.binaries.items():
            (self.release / name).write_bytes(data)
        self.write_sums(self.binaries)
        self.scripts = tuple(self.root / script.name for script in pin_verifier.SCRIPTS)
        for copy, original in zip(self.scripts, pin_verifier.SCRIPTS):
            copy.write_bytes(original.read_bytes())

    def write_sums(self, binaries, extra=""):
        rows = "".join(f"{hashlib.sha256(data).hexdigest()}  {name}\n" for name, data in binaries.items())
        (self.release / "SHA256SUMS").write_text(rows + extra)

    def pin(self, attest=attested, tag="verifier@2.3.4", commit=COMMIT, repository=pin_verifier.REPOSITORY):
        pin_verifier.pin(tag, repository, self.release, commit, self.scripts, attest)

    def assert_scripts_unchanged(self, before):
        self.assertEqual(before, [script.read_bytes() for script in self.scripts])

    def refused(self, **kwargs):
        before = [script.read_bytes() for script in self.scripts]
        with self.assertRaises(SystemExit):
            self.pin(**kwargs)
        self.assert_scripts_unchanged(before)

    def test_both_scripts_pin_the_same_version_and_hashes(self):
        sh, ps1 = pin_verifier.pinned(SH), pin_verifier.pinned(PS1)
        self.assertEqual(sh, ps1)
        self.assertEqual(set(sh[1]), set(pin_verifier.PLATFORMS))
        # A real pin is a version and 64-hex hashes; until the first release both are the same placeholder, which
        # makes the scripts refuse (they take nothing "unpinned").
        if sh[0] != "unpinned":
            self.assertRegex(sh[0], r"^[0-9]+\.[0-9]+\.[0-9]+$")
            for platform, digest in sh[1].items():
                self.assertRegex(digest, r"^[0-9a-f]{64}$", platform)
        else:
            self.assertEqual(set(sh[1].values()), {"unpinned"})

    def test_pinning_an_attested_release_rewrites_only_the_block_in_both_scripts(self):
        originals = [script.read_bytes() for script in self.scripts]
        self.pin()
        want = {
            platform: hashlib.sha256(self.binaries[name]).hexdigest()
            for platform, name in zip(
                ["linux-aarch64", "linux-x86_64", "macos-arm64", "macos-x64", "windows-arm64", "windows-x64"], ASSETS
            )
        }
        for script, original in zip(self.scripts, originals):
            self.assertEqual(pin_verifier.pinned(script), ("2.3.4", want))
            outside = pin_verifier.BLOCK.sub("", script.read_bytes().decode())
            self.assertEqual(outside, pin_verifier.BLOCK.sub("", original.decode()))

    def test_a_crlf_script_keeps_its_bytes_outside_the_block_and_its_line_ends_inside(self):
        text = self.scripts[1].read_bytes().decode().replace("\n", "\r\n")
        self.scripts[1].write_bytes(text.encode())
        self.pin()
        after = self.scripts[1].read_bytes().decode()
        self.assertNotIn("\n", after.replace("\r\n", ""))
        self.assertEqual(pin_verifier.pinned(self.scripts[1])[0], "2.3.4")
        self.assertEqual(pin_verifier.BLOCK.sub("", after), pin_verifier.BLOCK.sub("", text))

    def test_files_that_match_their_own_sums_but_have_no_attestation_pin_nothing(self):
        self.refused(attest=not_attested)

    def test_download_repository_does_not_change_the_signer_or_callers_commit(self):
        seen = []
        self.pin(repository="mirror/verifier", attest=lambda path, repo, commit: seen.append((path.name, repo, commit)))
        self.assertEqual({(repo, commit) for _, repo, commit in seen}, {("sunrise-org/sunrise", COMMIT)})
        self.assertEqual(sorted(name for name, _, _ in seen), sorted(["SHA256SUMS", *ASSETS]))
        self.refused(commit="main")
        self.refused(commit=COMMIT.upper())

    def test_the_attestation_check_names_the_workflow_ref_and_commit(self):
        command = pin_verifier.attestation_command(Path("f"), "sunrise-org/sunrise", COMMIT)
        self.assertEqual(command[:3], ["gh", "attestation", "verify"])
        for flag, value in [
            ("--signer-workflow", "sunrise-org/sunrise/.github/workflows/verifier.yml"),
            ("--source-ref", "refs/heads/main"),
            ("--source-digest", COMMIT),
            ("-R", "sunrise-org/sunrise"),
        ]:
            self.assertEqual(command[command.index(flag) + 1], value)
        self.assertIn("--deny-self-hosted-runners", command)

    def test_a_binary_that_is_not_what_sums_lists_pins_nothing(self):
        (self.release / ASSETS[1]).write_bytes(b"swapped")
        self.refused()

    def test_malformed_duplicate_or_missing_rows_pin_nothing(self):
        good = (self.release / "SHA256SUMS").read_text()
        digest = hashlib.sha256(self.binaries[ASSETS[0]]).hexdigest()
        for bad in [
            good + "not a row\n",
            good + f"{digest}  {ASSETS[0]}\n",
            good + f"{digest}  ../escape\n",
            "".join(good.splitlines(keepends=True)[1:]),
            good + f"{digest}  extra-file\n",
        ]:
            (self.release / "SHA256SUMS").write_text(bad)
            self.refused()

    def test_a_tag_must_match_the_publishing_version_grammar(self):
        for tag in ["verifier@1.0.\u0661", "verifier@1.0", "launcher@1.0.0",
                    "verifier@1.00.0", "verifier@1.0.1234567890"]:
            with self.subTest(tag=tag):
                self.refused(tag=tag)

    def test_a_script_with_two_pin_blocks_is_refused_and_nothing_is_written(self):
        text = self.scripts[1].read_bytes().decode()
        self.scripts[1].write_bytes((text + pin_verifier.BLOCK.search(text).group(0)).encode())
        self.refused()


if __name__ == "__main__":
    unittest.main()
