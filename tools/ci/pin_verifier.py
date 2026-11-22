#!/usr/bin/env python3
"""Pins `sunrise-verify` in the server install scripts.

    pin_verifier.py verifier@X.Y.Z --commit SHA [--repository OWNER/NAME] [--from-dir FOLDER]

Downloads the release's SHA256SUMS, checks that it lists exactly the six binaries `verifier.yml` publishes, downloads
each binary and checks it against the sums, then rewrites the one marked pin block in packaging/server/install.sh and
install.ps1 (the version and one SHA-256 per platform). The scripts take a downloaded verifier only if it matches
that pin, so the pin on main is the trust anchor, and SHA256SUMS sits in the same place as the binaries: so before
anything is written, `gh attestation verify` must accept SHA256SUMS and every binary as built by verifier.yml of
REPOSITORY on refs/heads/main at COMMIT, the commit the caller names (the one the release was dispatched from), never
one read from the release. The change goes in an ordinary pull request.
`--repository` changes only where the files are downloaded from. `--from-dir` reads the files from a folder
instead of the release. Both still require the official Sunrise verifier workflow's attestation.
"""
import argparse
import hashlib
import re
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = (ROOT / "packaging/server/install.sh", ROOT / "packaging/server/install.ps1")
REPOSITORY = "sunrise-org/sunrise"
VERSION_COMPONENT = r"(?:0|[1-9][0-9]{0,8})"
TAG = re.compile(rf"verifier@({VERSION_COMPONENT}(?:\.{VERSION_COMPONENT}){{2}})")
SUMS_ROW = re.compile(r"([0-9a-f]{64}) [ *]([^\s/\\]+)")
COMMIT = re.compile(r"[0-9a-f]{40}")
WORKFLOW = ".github/workflows/verifier.yml"
# platform -> the release asset; the names verifier.yml publishes.
PLATFORMS = {
    "linux-x86_64": "sunrise-verify-linux-x86_64",
    "linux-aarch64": "sunrise-verify-linux-aarch64",
    "macos-arm64": "sunrise-verify-macos-arm64",
    "macos-x64": "sunrise-verify-macos-x64",
    "windows-x64": "sunrise-verify-windows-x64.exe",
    "windows-arm64": "sunrise-verify-windows-arm64.exe",
}
BEGIN = "# >>> verifier pin"
END = "# <<< verifier pin"
BLOCK = re.compile(rf"^{re.escape(BEGIN)}.*?^{re.escape(END)}[^\n]*\n", re.S | re.M)
NOTE = "tools/ci/pin_verifier.py rewrites this block; do not edit it by hand"


def block(script: Path, version: str, hashes: dict[str, str]) -> str:
    """The pin block as `script`'s language writes it, with LF line ends."""
    if script.suffix == ".ps1":
        rows = "".join(f"    '{platform}' = '{hashes[platform]}'\n" for platform in PLATFORMS)
        body = f"$VerifierVersion = '{version}'\n$VerifierSha256 = @{{\n{rows}}}\n"
    else:
        rows = "".join(f"VERIFIER_SHA256_{platform.replace('-', '_')}={hashes[platform]}\n" for platform in PLATFORMS)
        body = f"VERIFIER_VERSION={version}\n{rows}"
    return f"{BEGIN} ({NOTE})\n{body}{END}\n"


def find_block(text: str, script: Path) -> re.Match:
    """The script's one pin block: exactly one begin marker and one end marker, in order."""
    if text.count(BEGIN) != 1 or text.count(END) != 1:
        sys.exit(f"{script}: needs exactly one verifier pin block")
    found = BLOCK.search(text)
    if not found:
        sys.exit(f"{script}: no verifier pin block")
    return found


def pinned(script: Path) -> tuple[str, dict[str, str]]:
    """The version and per-platform hashes a script's pin block holds."""
    text = find_block(script.read_bytes().decode("utf-8"), script).group(0)
    if script.suffix == ".ps1":
        version = re.search(r"\$VerifierVersion = '([^']*)'", text).group(1)
        hashes = dict(re.findall(r"'([a-z0-9_-]+)' = '([^']*)'", text))
    else:
        version = re.search(r"^VERIFIER_VERSION=(\S*)\r?$", text, re.M).group(1)
        names = {platform.replace("-", "_"): platform for platform in PLATFORMS}
        hashes = {names[name.removeprefix("VERIFIER_SHA256_")]: value
                  for name, value in re.findall(r"^(VERIFIER_SHA256_\w+)=(\S*)\r?$", text, re.M)}
    return version, hashes


def rewritten(script: Path, version: str, hashes: dict[str, str]) -> bytes:
    """`script`'s bytes with only its pin block replaced (in the file's own line ends)."""
    text = script.read_bytes().decode("utf-8")
    found = find_block(text, script)
    new = block(script, version, hashes)
    if "\r\n" in found.group(0):
        new = new.replace("\n", "\r\n")
    return (text[:found.start()] + new + text[found.end():]).encode("utf-8")


def parse_sums(text: str) -> dict[str, str]:
    """name -> digest of a SHA256SUMS: every row well formed, no name twice."""
    sums = {}
    for line in text.splitlines():
        row = SUMS_ROW.fullmatch(line)
        if not row:
            sys.exit(f"SHA256SUMS has a row that is not '<sha256>  <name>': {line!r}")
        if row.group(2) in sums:
            sys.exit(f"SHA256SUMS lists {row.group(2)} twice")
        sums[row.group(2)] = row.group(1)
    return sums


def attestation_command(path: Path, repository: str, commit: str) -> list[str]:
    """The check `launcher-bridge.yml` makes of the launcher's files, for this repository's verifier workflow."""
    return ["gh", "attestation", "verify", str(path), "-R", repository, "--signer-workflow", f"{repository}/{WORKFLOW}",
            "--source-ref", "refs/heads/main", "--source-digest", commit, "--deny-self-hosted-runners"]


def verify_attestation(path: Path, repository: str, commit: str) -> None:
    done = subprocess.run(attestation_command(path, repository, commit), capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(f"{path.name} has no valid build-provenance attestation from {repository}'s {WORKFLOW} on main "
                 f"at {commit}: nothing pinned\n{done.stderr.strip()}")


def pin(tag: str, repository: str, folder: Path | None, commit: str, scripts: tuple[Path, ...] = SCRIPTS,
        attest=verify_attestation) -> None:
    version = TAG.fullmatch(tag)
    if not version:
        sys.exit(f"{tag}: not a verifier release tag (verifier@X.Y.Z)")
    if not COMMIT.fullmatch(commit):
        sys.exit(f"--commit must be the full 40-hex commit the release was dispatched from, not {commit!r}")
    with tempfile.TemporaryDirectory() as tmp:
        files = {}
        for name in ["SHA256SUMS", *PLATFORMS.values()]:
            files[name] = Path(tmp) / name
            files[name].write_bytes(fetch(repository, tag, name, folder))
        # Provenance first: nothing below trusts a digest from files nobody has authenticated.
        for path in files.values():
            attest(path, REPOSITORY, commit)
        sums = parse_sums(files["SHA256SUMS"].read_bytes().decode("utf-8"))
        if sorted(sums) != sorted(PLATFORMS.values()):
            sys.exit(f"{tag}'s SHA256SUMS lists {sorted(sums)}, expected exactly {sorted(PLATFORMS.values())}")
        hashes = {}
        for platform, name in PLATFORMS.items():
            got = hashlib.sha256(files[name].read_bytes()).hexdigest()
            if got != sums[name]:
                sys.exit(f"{name} is {got}, but {tag}'s SHA256SUMS says {sums[name]}: nothing pinned")
            hashes[platform] = got
    # Every script's new bytes are made before any is written.
    new = {script: rewritten(script, version.group(1), hashes) for script in scripts}
    for script, data in new.items():
        script.write_bytes(data)
        print(f"{script.name}: pinned verifier {version.group(1)}")


def fetch(repository: str, tag: str, name: str, folder: Path | None) -> bytes:
    if folder is not None:
        return (folder / name).read_bytes()
    url = f"https://github.com/{repository}/releases/download/{urllib.parse.quote(tag)}/{urllib.parse.quote(name)}"
    with urllib.request.urlopen(url, timeout=120) as response:
        return response.read()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("tag")
    parser.add_argument("--commit", required=True, help="the full commit the release was dispatched from")
    parser.add_argument("--repository", default=REPOSITORY,
                        help="download from this repository; the signer is always the official Sunrise verifier workflow")
    parser.add_argument("--from-dir", type=Path)
    args = parser.parse_args()
    pin(args.tag, args.repository, args.from_dir, args.commit)
