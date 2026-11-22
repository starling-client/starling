#!/usr/bin/env python3
"""launcher-bridge.yml: the draft is exactly the release sunrise-launcher's release.yml attested, by name and digest, for this tag and commit."""
import argparse
import hashlib
import json
import sys
from pathlib import Path

PREDICATE = "https://playsunrise.net/attestation/launcher-release/v1"
SIGNER = "https://github.com/sunrise-org/sunrise-launcher/.github/workflows/release.yml@refs/heads/main"


def files_in(dist: Path) -> dict[str, str]:
    """The release files in `dist` and their SHA-256, SHA256SUMS and signatures left out."""
    return {f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in sorted(dist.iterdir())
            if f.is_file() and not f.name.startswith("SHA256SUMS")}


def sums_in(text: str) -> dict[str, str]:
    return {name.lstrip("*"): digest for digest, name in (line.split(maxsplit=1) for line in text.splitlines() if line.strip())}


def check(results: list, tag: str, commit: str, files: dict[str, str], sums: dict[str, str]) -> None:
    """Raises SystemExit unless every attested release record for `tag` (one, or a re-run's identical ones) matches `files` and `sums`."""
    records = [r for r in results
               if r["verificationResult"]["statement"].get("predicateType") == PREDICATE
               and r["verificationResult"]["statement"].get("predicate", {}).get("tag") == tag]
    if not records:
        raise SystemExit(f"no attested release record names {tag}")
    for found in records:
        check_one(found, tag, commit, files, sums)


def check_one(found: dict, tag: str, commit: str, files: dict[str, str], sums: dict[str, str]) -> None:
    statement = found["verificationResult"]["statement"]
    certificate = found["verificationResult"]["signature"]["certificate"]
    predicate = statement["predicate"]
    wanted = {"tag": tag, "version": tag[1:], "commit": commit}
    if {key: predicate.get(key) for key in wanted} != wanted:
        raise SystemExit(f"the release record says {predicate}, not {wanted}")
    built_by = {key: certificate.get(key) for key in ("buildTrigger", "buildSignerURI", "sourceRepositoryRef", "sourceRepositoryDigest", "runnerEnvironment")}
    expected = {"buildTrigger": "workflow_dispatch", "buildSignerURI": SIGNER, "sourceRepositoryRef": "refs/heads/main",
                "sourceRepositoryDigest": commit, "runnerEnvironment": "github-hosted"}
    if built_by != expected:
        raise SystemExit(f"the release record was made by {built_by}, not {expected}")
    attested = {s["name"]: s["digest"]["sha256"] for s in statement["subject"]}
    if attested != files:
        raise SystemExit(f"the draft holds {files}, the build attested {attested}")
    if sums != files:
        raise SystemExit(f"SHA256SUMS lists {sums}, the draft holds {files}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--attestations", type=Path, required=True, help="gh attestation verify --format json output")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--dist", type=Path, required=True)
    args = parser.parse_args(argv)
    files = files_in(args.dist)
    check(json.loads(args.attestations.read_text()), args.tag, args.commit, files, sums_in((args.dist / "SHA256SUMS").read_text()))
    print(f"{args.tag}: the draft's {len(files)} files are exactly the release record release.yml attested at {args.commit}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
