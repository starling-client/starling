#!/usr/bin/env python3
"""The Nightly, the only build Sunrise ships (.github/workflows/nightly-build.yml, docs/release-flow.md "Nightly").

    nightly.py pick [--force]      which commit tonight's nightly builds: GitHub outputs and nightly-notes.md
    nightly.py pick --kind dev     builds NIGHTLY_BASE (default main's head), a commit of main with the gate green,
                                   and publishes it as dev-<date>-<short>
    nightly.py rotate              renames the current `nightly` pre-release to nightly-<its date>, frees the tag
    nightly.py prune [--days 7]    deletes the nightly-<date> pre-releases and tags older than that, nothing else
    nightly.py prune-dev [--days 7] deletes dev-<date>-<short> pre-releases older than that, except the newest;
                                   never an sdk-<sha> SDK (Dev builds and Nightlies reference those)
    nightly.py prune-builds [--days 7]   deletes the nightly/build-<date>-<n> branches older than that, but the
                                   newest and any a published nightly points to (Ludovic, 2026-10-27)
    nightly.py keep-source <sha> <date>  tags the nightly's commit nightly-source/<date>-<short>, for good
    nightly.py report-musl-failure <kind> <step>  files a bug for the failed manual-download job, deduped by step
    nightly.py release-identity <tag> <sha> <sums>  the id of the release publish just made: its tag names <sha> and
                                   its SHA256SUMS holds the bytes of <sums>
    nightly.py attach <id> <tag> <sha> <file>...  uploads the files, in order, to release <id> while it is still <tag>
                                   at <sha>; a rerun keeps a file already there with the same bytes and refuses one
                                   with other bytes
    nightly.py sdk-archives        the assets the SDK's packing legs make (tools/prebuilt_game.py gather), one per line
    nightly.py sdk-assets          every asset a complete sdk-<sha> release holds: those, sdk.json and the signed sums
    nightly.py sdk-index <sha> <out>     writes sdk-<sha>'s sdk.json to <out>, checked against its SHA256SUMS
    nightly.py sdk-previous <sha> <out>  prints the SDK published before <sha> on its history, and writes its
                                   surface.json to <out>, checked; prints nothing when there is none
    nightly.py stable-mod-api <sha> <tag> <notes>  checks the stable mod API step and appends changed mixin targets
    nightly.py verify-sums --tag T [--signature F] [--only N]... [--key K]... [--publishing] DIR
                                         checks a release folder's signed sums with OpenSSL and main's release keys
    nightly.py break-glass-legs <sha>    the daily legs a break-glass daily run of <sha> runs (daily.yml resolve)
    nightly.py break-glass-notes <notes>  adds a break-glass release's section, with its verified approvals

A nightly builds main plus the pull requests the trusted reviewers picked (the set: tools/ci/nightly_set.py), merged
in their order and pushed as nightly/build-<date>-<n>; with no set, main alone. Main is the commit the
`daily-green` tag points to (release_train.daily_green_commit; on Sunrise through release_train.channel_head, below),
or NIGHTLY_BASE when the caller gives that commit. It is published as the
`nightly` pre-release; the previous one stays a week as nightly-<date>.
On sunrise-org/sunrise only, the base is the commit `daily-green` points to (the newest daily run that passed every
leg, which also published its SDK: release_train.daily_green_commit), with a complete published `sdk-<sha>` release
of the current package format for that exact SHA, whose sdk.json the Nightly carries in its signed sums. An explicit base that is any other commit (unless NIGHTLY_EXCEPTION names the owner's one-time grant for it), or has no SDK, fails; PR picks are
disabled, and the final SHA is rechecked before pushing.
Its notes list what reached main since the previous nightly, then the picked pull requests. Runs from main's
checkout with `gh`; it reads and merges the branches with git and never runs anything from them.

Each nightly ships its exact source: sunrise-source.tar.gz (`git archive` of the commit, signed with the builds) on
the release, and the tag nightly-source/<date>-<short>, which no prune deletes. Native mods pin a nightly's commit
(docs/native-mods.md), so that commit must stay downloadable after its release and build branch are gone.
"""
from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import io
import json
import os
import re
import subprocess
import string
import sys
import tarfile
import tempfile
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path

if __package__:  # imported as tools.ci.nightly (the tests)
    from . import nightly_set as ns
    from .. import release_train as train
else:  # run as tools/ci/nightly.py: its folder is on the path
    import nightly_set as ns
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import release_train as train

DATED = re.compile(r"^nightly-(\d{4}-\d{2}-\d{2})$")
TITLE_DATE = re.compile(r"Nightly (\d{4}-\d{2}-\d{2})")
SOURCE_TAG = "nightly-source/"
MUSL_NOTE = "The optional musl Linux archive follows separately if its build, signing and upload succeed."
# Load from the trusted workflow checkout before pick() switches to a possibly older green tree.
SUNRISE_PLAYERS = Path(__file__).with_name("nightly-players.md").read_text().strip()


@dataclass
class Ref:
    name: str
    sha: str
    date: dt.datetime  # committer date


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=REPOSITORY, check=True, capture_output=True, text=True).stdout.strip()


def gh(*args: str, check: bool = True) -> str:
    out = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and out.returncode != 0:
        sys.exit(f"gh {' '.join(args)}: {out.stderr.strip()}")
    return out.stdout.strip() if out.returncode == 0 else ""


def is_ancestor(a: str, b: str) -> bool:
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b], cwd=REPOSITORY).returncode == 0


def output(**values: str) -> None:
    with open(os.environ.get("GITHUB_OUTPUT", "/dev/stdout"), "a") as f:
        for k, v in values.items():
            f.write(f"{k}={v}\n")


PLAYERS = ("The launcher installs it by itself; your worlds stay as they are. A Nightly can include pull requests "
           "the Team picked before they are merged (below). Report bugs with the version the game shows "
           "(\"Nightly <date> · <commit>\").")


def source_tag(today: str, sha: str) -> str:
    """The permanent tag of a nightly's commit: neither `prune` (nightly-<date> only) nor `prune-builds` touches it."""
    return f"{SOURCE_TAG}{today}-{sha[:7]}"


def source_line(today: str, sha: str) -> str:
    return (f"Source: commit {sha}, kept for good as the tag `{source_tag(today, sha)}`; sunrise-source.tar.gz is "
            "its tree. Native mods built for this Nightly pin that commit.")


def notes(today: str, chosen: Ref, main: Ref, picked: list[dict], left_out: list[str], merged: list[dict],
          issues: list[str]) -> str:
    """A nightly's notes: what reached main since the previous nightly (New, Fixed, as a release's notes), then the
    reviewers' picks, the known issues and the thanks."""
    shown = [pr for pr in merged if pr["paths"] and "skip-release" not in pr["labels"]]
    new = [pr for pr in shown if "feature" in pr["labels"]]
    fixed = [pr for pr in shown if "feature" not in pr["labels"]]
    built = f"main ({main.sha[:7]})" + (f" with {len(picked)} pull requests merged in" if picked else "")
    # The banner first: people who land on the Nightly ran its game zip for the launcher (a player, 2026-10-30).
    fork = train.REPO == "sunrise-org/sunrise"
    parts = [SUNRISE_PLAYERS if fork else train.PLAYERS,
             f"Nightly {today} · {chosen.sha[:7]}: {built}, built from `{chosen.name}` ({chosen.sha}).",
             source_line(today, chosen.sha), MUSL_NOTE]
    if not fork:
        parts.append(PLAYERS)
    if not shown and not picked:
        parts.append("Nothing new for players on main since the previous Nightly.")
    if new:
        parts.append("### New\n" + "\n".join(map(train.bullet, new)))
    if fixed:
        parts.append("### Fixed\n" + "\n".join(map(train.bullet, fixed)))
    if picked:
        parts.append(f"### Not merged yet ({len(picked)})\n" + "\n".join(
            f"- {ns.plain(e['title'])} (#{e['number']}) by {e['author']}, at {e['head'][:7]}, picked by {e['asked_by']}"
            for e in picked))
    if left_out:
        parts.append(f"### Left out ({len(left_out)})\n" + "\n".join(f"- {ns.plain(why)}" for why in left_out))
    if issues:
        parts.append("### Known issues\n" + "\n".join(issues))
    contributors = sorted({pr["author"] for pr in shown if pr["author"] and pr["author"] != train.MAINTAINER})
    if contributors:
        parts.append("Thanks to " + ", ".join(f"@{name}" for name in contributors) + ".")
    parts.append(train.DISCLAIMER)
    return "\n\n".join(parts) + "\n"


def merged_since(previous: str | None, main: str) -> list[dict]:
    """The pull requests merged into main since the previous nightly (its commit holds the main it was built on)."""
    if not previous or subprocess.run(["git", "cat-file", "-e", f"{previous}^{{commit}}"], cwd=REPOSITORY).returncode != 0:
        return []
    return train.merged_between("game", previous, main)


def refresh(entries: list[dict], get, patterns: list[str]) -> tuple[list[dict], list[str]]:
    """The scheduled nightly's set: each pull request still at the head it was picked at (a push after the pick
    waits for someone to pick it again); merged, closed and refused ones are out."""
    kept, left_out = [], []
    for e in entries:
        pr = get(e["number"])
        if pr is not None and pr.state == "merged":
            left_out.append(f"#{e['number']} {e['title']}: merged, so it's in main")
        elif why := ns.check(pr, e["number"], patterns):
            left_out.append(f"{why}: left out")
        else:
            kept.append(e)
    return kept, left_out


def requested(spec: str, get) -> list[dict]:
    """A dry run's set from its input: `2640@<head> 2642@<head>`."""
    prs = []
    for word in spec.split():
        number, _, head = word.partition("@")
        pr = get(int(number))
        prs.append({"number": int(number), "head": head or (pr.head if pr else ""), "title": pr.title if pr else "?",
                    "author": pr.author if pr else "?", "asked_by": "dry run"})
    return prs


def stop(repo: str, issue: str, text: str) -> None:
    """Says why the build stopped, where it was asked for, and fails the job."""
    print(text)
    if issue and repo:
        ns.answer(repo, issue, text)
    sys.exit(1)


def build_set(main: Ref, prs: list[dict], left_out: list[str], today: str, force: bool, dry_run: bool) -> None:
    """main + the set, merged with git only (nightly_set.assemble), pushed as nightly/build-<date>-<n>."""
    repo, issue = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("NIGHTLY_ISSUE", "")
    token, event = os.environ.get("GH_TOKEN", ""), os.environ.get("GITHUB_EVENT_NAME", "")
    patterns = ns.guarded_patterns(Path(ns.CODEOWNERS).read_text() if Path(ns.CODEOWNERS).exists() else "")
    gone = set(ns.fetch_heads([(e["number"], e["head"]) for e in prs]))
    kept = []
    for e in prs:
        if e["number"] in gone:
            left_out.append(f"#{e['number']} {e['title']}: its head {e['head'][:7]} is gone; ask for it again")
        elif bad := [f for f in ns.changed_files(main.sha, e["head"]) if ns.guarded(f, patterns)]:
            left_out.append(f"#{e['number']} {e['title']}: changes `{bad[0]}`, which only Ludovic merges")
        else:
            kept.append(e)
    try:
        sha = ns.assemble(main.sha, [(e["number"], e["head"]) for e in kept])
    except ns.Conflict as c:
        state, _, _ = ns.load_state()
        asked_in = issue or next((x["issue"] for x in reversed(state.get("log", [])) if x.get("issue")), "")
        stop(repo, asked_in, f"the build stopped: {c.text()}")
    if repo == "sunrise-org/sunrise" and sha != main.sha:
        stop(repo, issue, f"the build stopped: final source {sha} is not the gate-checked main commit.")
    previous = git("ls-remote", "--tags", "origin", "refs/tags/nightly").split("\t")[0] or None
    build = force or previous != sha
    # A game build is never published without its SDK, published first on the same commit (owner, 2026-11-17).
    sdk = repo != "sunrise-org/sunrise" or sdk_ready(sha)
    if build and not sdk:
        if not dry_run:
            stop(repo, issue, f"the build stopped: {sha} has no complete published sdk-{sha}; a Nightly is published "
                              "only after its SDK, on the same commit.")
        print(f"::warning::{sha} has no published SDK: a real Nightly of it would stop here; this dry run carries no "
              "sdk.json")
    name = "(unchanged)"
    if build:
        if dry_run:
            run, attempt = os.environ.get("GITHUB_RUN_ID", ""), os.environ.get("GITHUB_RUN_ATTEMPT", "")
            if not re.fullmatch(r"[0-9]+", run) or not re.fullmatch(r"[0-9]+", attempt):
                stop(repo, "", "a dry run requires its workflow run and attempt identity")
            # Temporary checkout transport, never a real build or saved selection.
            # The workflow cleans this isolated ref even if pick fails after pushing it.
            name = f"nightly/dry-run-{run}-{attempt}"
        else:
            done = ns.builds_today(today)
            if len(done) >= ns.MAX_BUILDS_PER_DAY and event != "schedule" and os.environ.get("NIGHTLY_URGENT") != "true":
                stop(repo, issue, f"{len(done)} Nightly builds today already, the most a day takes: the next scheduled "
                                  "one (20:17 UTC) builds the set.")
            name = ns.next_build(today, done)
        if not ns.push(sha, name, token):
            stop(repo, issue, f"could not push {name}")
        record = {"ref": name, "sha": sha, "main": main.sha, "at": dt.datetime.now(dt.timezone.utc).isoformat(
            timespec="seconds"), "event": event, "dry_run": dry_run, "run": os.environ.get("GITHUB_RUN_ID", ""),
            "prs": [{k: e[k] for k in ("number", "head", "title", "author", "asked_by")} for e in kept],
            "left_out": left_out}
        if not dry_run:
            ns.update_state(lambda st, builds: (st, builds + json.dumps(record) + "\n", f"build {name}"), token)
    chosen = Ref(name, sha, dt.datetime.now(dt.timezone.utc))
    if force and previous == sha:
        # A forced rebuild of the published Nightly's commit (the macOS legs it lacked, 2026-11-17) replaces that
        # Nightly, so its notes list what the replaced one did: since the newest Nightly kept on an older commit.
        older = [c for c in remote_refs(f"refs/tags/{SOURCE_TAG}*").values() if c != sha and is_ancestor(c, sha)]
        previous = max(older, key=lambda c: int(git("rev-list", "--count", c)), default=None)
    with open("nightly-notes.md", "w") as f:
        f.write(notes(today, chosen, main, kept, left_out, merged_since(previous, main.sha), train.known_issues()))
    output(build=str(build).lower(), sha=sha, short=sha[:7], source=name, date=today, version=f"nightly-{today}",
           nightly=f"{today} {sha[:7]}", source_tag=source_tag(today, sha), sdk=str(sdk).lower())
    print(f"{'build' if build else 'skip (unchanged since the last nightly)'}: {name} {sha}, main {main.sha[:7]}"
          + "".join(f" + #{e['number']}" for e in kept))


def package_format() -> int:
    """The prebuilt package's layout number, from its one place; anything but a number fails."""
    return int((Path(__file__).resolve().parents[2] / "crates/sunrise-mod-package/src/sdk/package-format").read_text().rstrip())


# The SDK's packing legs (prebuilt-game-build.yml's matrix): each target, and the Linux server's side.
SDK_LEGS = (("x86_64-unknown-linux-gnu", ""), ("x86_64-unknown-linux-gnu", "-server"),
            ("x86_64-pc-windows-gnu", ""), ("aarch64-apple-darwin", ""))
SDK_BUILD_DATA = "build-data.tar.xz"
SDK_SURFACE = "surface.json"
# Every asset sdk-publish requires: each leg's slices, each client target's tool, the gathered data and the API source.
SDK_ARCHIVES = (tuple(f"{kind}-{target}{side}.tar.xz" for target, side in SDK_LEGS
                      for kind in ("runtime", "api", "engine"))
                + tuple(f"tool-{target}.tar.xz" for target, side in SDK_LEGS if not side)
                + ("api-source.tar.xz", SDK_BUILD_DATA, SDK_SURFACE))
# Beside them in the release: the index and the signed sums (`sunrise_mod_package::sdk::BESIDE_ASSETS`).
SDK_INDEX = "sdk.json"
SDK_ASSETS = SDK_ARCHIVES + (SDK_INDEX, "SHA256SUMS", "SHA256SUMS.sig")


def published_releases():
    pages = train.gh_json("api", "--paginate", "repos/sunrise-org/sunrise/releases?per_page=100")
    return (release for page in pages for release in page if not release["draft"])


def ready_sdk_shas() -> set[str]:
    """Snapshot of the complete published `sdk-<sha>` releases; API failures fail closed."""
    return {release["tag_name"].removeprefix("sdk-")
            for release in published_releases() if re.fullmatch(r"sdk-[0-9a-f]{40}", release["tag_name"])
            and set(SDK_ASSETS) <= {asset["name"] for asset in release["assets"]}}


def sdk_ready(sha: str) -> bool:
    """A fresh check, never cached across selection and publication."""
    return sha in ready_sdk_shas()


REPOSITORY = Path(__file__).resolve().parents[2]
# What a cargo build or a built tool never inherits from the job: its tokens and the files that change later steps
# (a build script or proc macro runs whatever its crate says), another target folder, or a wrapper that would run in
# place of the compiler.
INHERITED_BUILD = ("GH_TOKEN", "GITHUB_TOKEN", "ACTIONS_RUNTIME_TOKEN", "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
                   "ACTIONS_ID_TOKEN_REQUEST_URL", "GITHUB_ENV", "GITHUB_PATH", "GITHUB_OUTPUT", "GITHUB_STATE",
                   "GITHUB_STEP_SUMMARY", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "RUSTC_WRAPPER",
                   "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER")


def isolated_env() -> dict[str, str]:
    return {name: value for name, value in os.environ.items() if name not in INHERITED_BUILD}


# The release keys launchers trust, read from main, never from the checkout a release builds: a candidate's own copy
# could list a key of its own.
KEYS_FILE = "crates/sunrise-release-sign/release-keys.txt"
# An Ed25519 public key's DER: this SubjectPublicKeyInfo header, then its 32 bytes.
ED25519_SPKI = bytes.fromhex("302a300506032b6570032100")
# What Rust's `str::trim` strips (Unicode White_Space), which the native verifier uses. Python's own strip() takes
# U+001C to U+001F too.
RUST_WHITESPACE = "\t\n\v\f\r \x85\xa0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a" \
                  "\u2028\u2029\u202f\u205f\u3000"


def text_lines(text: str) -> list[str]:
    """`text`'s lines as Rust's `str::lines` splits them: at LF or CRLF, never at a bare CR."""
    return text.replace("\r\n", "\n").split("\n")


def canonical_base64(text: str) -> bytes | None:
    """The bytes of `text` when it is base64 as the native verifier's strict decoder takes it (padded, no stray
    characters, no unused bits set), else None."""
    try:
        decoded = base64.b64decode(text, validate=True)
    except ValueError:
        return None
    return decoded if base64.b64encode(decoded).decode() == text else None


def release_keys() -> list[str]:
    """main's release-keys.txt (one base64 key per line, `#` comments): sunrise-org/sunrise's keys, none elsewhere.
    Read as the native verifier reads it: a line that is no 32-byte key makes it trust none, so that refuses here."""
    if os.environ.get("GITHUB_REPOSITORY", "sunrise-org/sunrise") != "sunrise-org/sunrise":
        return []
    subprocess.run(["git", "fetch", "--quiet", "--no-tags", "origin", "+refs/heads/main:refs/remotes/origin/main"],
                   cwd=REPOSITORY, check=True)
    data = subprocess.run(["git", "show", f"refs/remotes/origin/main:{KEYS_FILE}"], cwd=REPOSITORY, check=True,
                          capture_output=True).stdout
    try:
        text = data.decode()
    except UnicodeDecodeError:
        raise SystemExit(f"::error::main's {KEYS_FILE} is not UTF-8 text: launchers would trust no key")
    keys = [line.strip(RUST_WHITESPACE) for line in text_lines(text)]
    keys = [key for key in keys if key and not key.startswith("#")]
    for key in keys:
        if len(canonical_base64(key) or b"") != 32:
            raise SystemExit(f"::error::main's {KEYS_FILE} line {key!r} is no Ed25519 public key: launchers would "
                             "trust no key")
    return keys


def sums_rows(tag: str, sums: bytes) -> list[tuple[str, str]]:
    """The `(file, sha256)` rows of a SHA256SUMS, read as sunrise-release-sign's parse_sums reads them: UTF-8 text,
    blank lines skipped, each line `<64 hex digits> <file>` with one ` ` or `*` before the file stripped. A file is a
    name in the release's folder: one with a `/` or `\\`, or `.` or `..`, refuses."""
    try:
        text = sums.decode()
    except UnicodeDecodeError:
        raise SystemExit(f"::error::{tag}'s SHA256SUMS is not UTF-8 text")
    rows = []
    for line in text_lines(text):
        if not line.strip(RUST_WHITESPACE):
            continue
        digest, space, name = line.partition(" ")
        name = name[1:] if name[:1] in (" ", "*") else name
        if (not space or len(digest) != 64 or not all(c in string.hexdigits for c in digest) or not name
                or "/" in name or "\\" in name or name in (".", "..")):
            raise SystemExit(f"::error::{tag}'s SHA256SUMS row {line!r} is not `<sha256>  <file>`")
        rows.append((name, digest.lower()))
    return rows


def verify_sums(folder: Path, tag: str, only=(), signature: str = "SHA256SUMS.sig", keys=None,
                publishing: bool = False) -> None:
    """Checks a release folder as launchers do (sunrise-release-sign's `verify`): this script, from the workflow's
    checkout, with the runner's OpenSSL, compiling and running nothing of the release: a key of `keys` (main's release-keys.txt by default) signed SHA256SUMS for `tag` (the
    message `horizon release <tag>` and a newline, then SHA256SUMS), and each file of `only` (every file SHA256SUMS
    lists when empty) is listed and matches. `publishing`: while no key is listed, a release of sunrise-org/sunrise
    (no other repository) goes out with a warning, signed or not, as before any key existed. The signature file holds one base64 signature per line, and one line that
    verifies is enough, as for the native verifier: a line that is no canonical base64 is skipped. A repeated name
    is checked against each of its rows. Reports on stderr (stdout is its callers' result), and exits naming what
    failed."""
    keys = release_keys() if keys is None else list(keys)
    if "/" in signature or "\\" in signature or signature in ("", ".", ".."):
        raise SystemExit(f"::error::--signature names a file of the folder, not {signature!r}")
    sums = (folder / "SHA256SUMS").read_bytes()
    rows = sums_rows(tag, sums)
    if not only and not rows:
        raise SystemExit(f"::error::{tag}'s SHA256SUMS lists no file")
    # Before any key exists, only Sunrise's own releases may go out unchecked, as with the native verifier.
    unkeyed = publishing and not keys and os.environ.get("GITHUB_REPOSITORY", "sunrise-org/sunrise") == "sunrise-org/sunrise"
    try:
        lines = text_lines((folder / signature).read_bytes().decode())
    except (OSError, UnicodeDecodeError):
        lines = None
    if lines is None:
        if unkeyed:
            print(f"::warning::{tag}: published with SHA256SUMS but unsigned: release-keys.txt lists no key yet",
                  file=sys.stderr)
        else:
            raise SystemExit(f"::error::{tag} has no {signature}: launchers would refuse it")
    elif not keys:
        if not unkeyed:
            raise SystemExit(f"::error::{tag}: no key to check the signature with")
        print(f"::warning::{tag}: signed, but release-keys.txt lists no key yet, so launchers can't check it",
              file=sys.stderr)
    else:
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            message = work / "message"
            message.write_bytes(f"horizon release {tag}\n".encode() + sums)
            signed = False
            for line in lines:
                decoded = canonical_base64(line.strip(RUST_WHITESPACE))
                if decoded is None or len(decoded) != 64:
                    continue
                (work / "signature").write_bytes(decoded)
                for key in keys:
                    (work / "key.der").write_bytes(ED25519_SPKI + base64.b64decode(key))
                    checked = subprocess.run(
                        ["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", str(work / "key.der"), "-keyform",
                         "DER", "-rawin", "-in", str(message), "-sigfile", str(work / "signature")],
                        cwd=work, env=isolated_env(), capture_output=True)
                    signed = signed or checked.returncode == 0
            if not signed:
                raise SystemExit(f"::error::{tag}: SHA256SUMS isn't signed for `horizon release {tag}` by a release key")
    for name in only:
        if name not in {listed for listed, _ in rows}:
            raise SystemExit(f"::error::{name} is not in {tag}'s SHA256SUMS")
    checked = [(name, digest) for name, digest in rows if not only or name in only]
    for name, digest in checked:
        with (folder / name).open("rb") as file:
            if hashlib.file_digest(file, "sha256").hexdigest() != digest:
                raise SystemExit(f"::error::{tag}'s {name} does not match its SHA256SUMS")
    print(f"{tag}: {len(checked)} files match SHA256SUMS", file=sys.stderr)


def release_assets(tag: str, *names: str) -> dict[str, bytes]:
    """Assets of the release `tag`, once their bytes match its SHA256SUMS and its signature verifies for that tag."""
    with tempfile.TemporaryDirectory() as directory:
        folder = Path(directory)
        gh("release", "download", tag, "--repo", "sunrise-org/sunrise", "--dir", directory,
           *(argument for name in (*names, "SHA256SUMS", "SHA256SUMS.sig") for argument in ("--pattern", name)))
        verify_sums(folder, tag, names)
        return {name: (folder / name).read_bytes() for name in names}


def released_assets(sha: str, *names: str) -> dict[str, bytes]:
    """Assets of sdk-<sha>, once their bytes match its SHA256SUMS and its signature verifies for that tag."""
    return release_assets(f"sdk-{sha}", *names)


def released(sha: str, name: str) -> bytes:
    return released_assets(sha, name)[name]


def checked_index(sha: str, index: bytes) -> dict:
    """sdk-<sha>'s sdk.json, once it names this build, this package format and every asset the release must hold;
    anything else stops the Nightly, naming why."""
    try:
        document = json.loads(index)
        commit, layout, assets = document["build"]["commit"], document["package_format"], set(document["assets"])
    except (ValueError, KeyError, TypeError) as error:
        raise SystemExit(f"sdk-{sha}'s {SDK_INDEX} is not an SDK index: {error}")
    if commit != sha:
        raise SystemExit(f"sdk-{sha}'s {SDK_INDEX} is for build {commit}")
    if layout != package_format():
        raise SystemExit(f"sdk-{sha}'s {SDK_INDEX} has package format {layout}, not this source's {package_format()}")
    missing = sorted(set(SDK_ARCHIVES) - assets)
    if missing:
        raise SystemExit(f"sdk-{sha}'s {SDK_INDEX} does not list {', '.join(missing)}")
    return document


def published_index(sha: str) -> bytes:
    """sdk-<sha>'s sdk.json as published, checked ([`released`], [`checked_index`])."""
    index = released(sha, SDK_INDEX)
    checked_index(sha, index)
    return index


def previous_sdk(sha: str) -> str:
    """The build id of the newest complete SDK published before `sha` on its history (the fewest commits behind
    it), or "" when there is none: what this build's surface is compared with."""
    behind = {}
    for other in ready_sdk_shas() - {sha}:
        if is_ancestor(other, sha):
            behind[other] = int(git("rev-list", "--count", f"{other}..{sha}"))
    return min(behind, key=behind.get) if behind else ""


def require_sdk(sha: str) -> None:
    """Refuse before rotating/publishing Nightly if its SDK disappeared or its index is not this build's."""
    if os.environ.get("GITHUB_REPOSITORY") == "sunrise-org/sunrise":
        if not sdk_ready(sha):
            raise SystemExit(f"Nightly publication refused: {sha} has no complete published sdk-{sha} release")
        published_index(sha)


def green_base(repo: str, issue: str) -> Ref:
    """NIGHTLY_BASE, else a green main commit (the dyna-checks gate on Sunrise, the green daily tag elsewhere)."""
    explicit = os.environ.get("NIGHTLY_BASE", "").strip()
    if repo == "sunrise-org/sunrise":
        sha = train.channel_head("nightly", explicit, os.environ.get("NIGHTLY_EXCEPTION", "").strip(),
                                 os.environ.get("NIGHTLY_DAILY_RUN", "").strip())
    else:
        sha = explicit or train.daily_green_commit()
        if not sha:
            stop(repo, issue, "the build stopped: no fully green daily run yet.")
        if not is_ancestor(sha, "refs/remotes/origin/main"):
            stop(repo, issue, f"the build stopped: {sha[:9]} is not on main.")
        sha = git("rev-parse", sha)
    return Ref("main", sha, dt.datetime.fromisoformat(git("show", "-s", "--format=%cI", sha)))


def require_ref(kind: str) -> None:
    """A real run publishes `kind` only from a ref that may (release_train.PUBLISHES); a dry run from any."""
    if os.environ.get("DRY_RUN") != "true":
        train.require_ref(os.environ.get("GITHUB_REF", ""), kind)


def pick(force: bool) -> None:
    require_ref("nightly")
    refs = ["+refs/heads/main:refs/remotes/origin/main"]
    # A new repository has no Nightly yet. Only fetch the tag when the remote advertises it;
    # main's fetch and every network error still fail normally.
    if git("ls-remote", "--tags", "origin", "refs/tags/nightly").strip():
        refs.append("+refs/tags/nightly:refs/tags/nightly")
    git("fetch", "--quiet", "origin", *refs)
    today = dt.datetime.now(dt.timezone.utc).date().isoformat()
    repo, token = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("GH_TOKEN", "")
    main = green_base(repo, os.environ.get("NIGHTLY_ISSUE", ""))
    dry_run = os.environ.get("DRY_RUN") == "true"
    if repo == "sunrise-org/sunrise":
        if os.environ.get("NIGHTLY_PRS", "").strip():
            stop(repo, os.environ.get("NIGHTLY_ISSUE", ""), "Sunrise Nightlies require a gate-checked main commit, not PR picks")
        return build_set(main, [], [], today, force, dry_run)
    if spec := os.environ.get("NIGHTLY_PRS", "").strip():  # a dry run's own set; the saved one stays
        return build_set(main, requested(spec, ns.lookup(repo)), [], today, force, dry_run)
    state, _, _ = ns.load_state()
    prs, left_out = state.get("prs", []), []
    if prs and os.environ.get("GITHUB_EVENT_NAME") == "schedule" and not dry_run:
        patterns = ns.guarded_patterns(Path(ns.CODEOWNERS).read_text() if Path(ns.CODEOWNERS).exists() else "")
        get = ns.lookup(repo)

        def fresh(st: dict, builds: str):
            kept, out = refresh(st.get("prs", []), get, patterns)
            left_out[:] = out
            if kept == st.get("prs", []):
                return None
            return {**st, "prs": kept}, builds, "the scheduled nightly drops the merged, closed and refused"
        prs = ns.update_state(fresh, token).get("prs", [])
    build_set(main, prs, left_out, today, force, dry_run)


def pick_dev() -> None:
    """A Dev build (release.yml kind dev, docs/release-flow.md "Dev"): a commit of main that recognizes the Dev
    version prefix, passed the gate and has a published SDK. It builds alone: no picks, build branch, nightly-state
    record or daily cap. If dev-<date>-<short> already has a release, skip it (a tag is never replaced)."""
    require_ref("dev")
    git("fetch", "--quiet", "origin", "+refs/heads/main:refs/remotes/origin/main")
    repo, dry_run = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("DRY_RUN") == "true"
    sha = train.channel_head("dev", os.environ.get("NIGHTLY_BASE", "").strip())
    sdk = repo != "sunrise-org/sunrise" or sdk_ready(sha)
    if not sdk:
        if not dry_run:
            stop(repo, "", f"the Dev build stopped: {sha} has no complete published SDK "
                 "(release.yml publishes it first).")
        print(f"::warning::{sha} has no published SDK yet: a real Dev build publishes sdk-{sha} first; this dry run "
              "carries no sdk.json")
    today = dt.datetime.now(dt.timezone.utc).date().isoformat()
    tag = train.build_tag("dev", today, sha)
    # A tag is never replaced: skip a published build of the same commit on the same day (a dry run still builds).
    build = dry_run or not gh("release", "view", tag, "--json", "tagName", check=False)
    with open("nightly-notes.md", "w") as f:
        f.write(f"Dev {today} · {sha[:7]}: main at {sha}, built to test it after merge.\n\n"
                "Starting with Sunrise Launcher 1.1.3, install Dev builds by exact tag "
                f"(`sunrise-launcher install {tag}`); until then, download and verify the release assets by hand. "
                f"Run `sunrise-release-sign verify --tag {tag} --signature SHA256SUMS.v2.sig <folder>`. "
                f"The matching SDK is `sdk-{sha}`.\n\n{MUSL_NOTE}\n\n{train.DISCLAIMER}\n")
    output(build=str(build).lower(), sha=sha, short=sha[:7], source="main", date=today, version=f"dev-{today}",
           nightly=f"{today} {sha[:7]}", source_tag="", tag=tag, sdk=str(sdk).lower())
    print(f"{'build' if build else f'skip ({tag} is already published)'}: {tag} {sha}")


def pick_stable() -> None:
    """A stable release (docs/release-flow.md "Release branches"): NIGHTLY_BASE, the full SHA approved in the
    release-candidate check, which must be the release branch's head with a real daily run that passed every leg on
    it (release_train.release_head), with its SDK published first, built alone and published as
    `<target>-<RELEASE_VERSION>`, which a real run must name (26.3-0.1.0 first; a dry run defaults to the next patch).
    RELEASES_PAUSED and an open release-blocker hold it, as they hold the Nightly; a dry run only warns, and takes any
    NIGHTLY_BASE while the branch does not exist yet. The publish job checks it all again (reserve_stable_tag)."""
    require_ref("stable")
    git("fetch", "--quiet", "origin", "+refs/heads/main:refs/remotes/origin/main")
    repo, dry_run = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("DRY_RUN") == "true"
    held = (["RELEASES_PAUSED is set"] if os.environ.get("RELEASES_PAUSED") == "1" else []) + [
        f"release-blocker #{number}" for number in train.open_blockers()]
    if held:
        if not dry_run:
            stop(repo, "", f"the release stopped: held by {', '.join(held)}.")
        print(f"::warning::held by {', '.join(held)}: a real release would stop here")
    # A break-glass patch release (docs/release-flow.md "Break-glass patch releases"): why, in the dispatcher's words.
    reason = " ".join(os.environ.get("BREAK_GLASS", "").split())
    sha = train.release_head(os.environ.get("NIGHTLY_BASE", "").strip(), dry_run, daily=not reason)
    sdk = repo != "sunrise-org/sunrise" or sdk_ready(sha)
    if not sdk:
        if not dry_run:
            stop(repo, "", f"the release stopped: {sha} has no complete published sdk-{sha}; a release is published "
                           "only after its SDK, on the same commit.")
        print(f"::warning::{sha} has no published SDK: a real release of it would stop here; this dry run carries no "
              "sdk.json")
    version = os.environ.get("RELEASE_VERSION", "").strip()
    if not dry_run and not version:
        stop(repo, "", "the release stopped: a real stable release names its version (`release_version`, 0.1.0 first), "
                       "as approved in the release-candidate check.")
    tags = train.origin_tags()
    wanted = f"{train.TARGET}-{version}"
    own = wanted if version and tags.get(wanted) == sha else ""
    tag = train.next_stable_tag(tags, version=version, own=own)
    # The release it follows, the one parent every stable check compares with: an orphan stable tag above it stops a
    # real release here, before anything is built or signed.
    if dry_run:
        try:
            parent = stable_parent(tag, verify=True)
        except SystemExit as refused:
            print(f"::warning::a real release would stop here: {refused}")
            parent = None
    else:
        parent = stable_parent(tag, verify=True)
    if wrong := stable_step(tag, parent):
        if not dry_run:
            stop(repo, "", f"the release stopped: {wrong}.")
        print(f"::warning::a real release would stop here: {wrong}")
    legs = []
    if reason:
        problems, legs = train.break_glass_problems(sha, tag, parent)
        attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "1")
        if attempt != "1":
            # Only a run's first attempt's approvals count (release_train.break_glass_approvals): refuse before
            # either person is asked.
            problems.append(f"a break-glass release runs only as its run's first attempt, not attempt {attempt}: "
                            "dispatch a new run")
        # What the two approvers approve, shown before their environments ask.
        line = (f"### Break-glass release {tag} at {sha}\n"
                + (f"Patches {parent.tag} ({parent.sha}) on the daily legs {', '.join(legs) or '(none)'}.\n"
                   if parent else "No published stable release to patch.\n"))
        print(line)
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
                summary.write(line)
        if problems:
            text = "the break-glass release stopped: " + "; ".join(problems) + "."
            if not dry_run:
                stop(repo, "", text)
            print(f"::warning::{text}")
    today = dt.datetime.now(dt.timezone.utc).date().isoformat()
    parts = [f"Sunrise {tag}: the stable release for Minecraft {train.TARGET}, commit {sha} of "
             f"`{train.RELEASE_BRANCH}`. sunrise-source.tar.gz is its tree; native mods built for it pin that commit, "
             f"and its SDK is the release sdk-{sha}.", MUSL_NOTE]
    if parent:
        picked = git("log", "--reverse", "--format=- %s", f"{parent.sha}..{sha}")
        parts.append(f"### Since {parent.tag}\n" + (picked or "No change since then."))
    else:
        parts.append(f"The first stable release for Minecraft {train.TARGET}: what the Nightly of this commit holds.")
    if issues := train.known_issues():
        parts.append("### Known issues\n" + "\n".join(issues))
    parts.append(train.DISCLAIMER)
    with open("nightly-notes.md", "w") as f:
        f.write("\n\n".join(parts) + "\n")
    output(build="true", sha=sha, short=sha[:7], source=train.RELEASE_BRANCH, date=today, version=tag, nightly="",
           source_tag="", tag=tag, sdk=str(sdk).lower(), break_glass=str(bool(reason)).lower(),
           parent_sha=parent.sha if parent else "", parent_tag=parent.tag if parent else "",
           break_glass_legs=" ".join(legs))
    print(f"build: {tag} {sha}")


# The facts a launcher reads from a game build, in every release's signed sums (tools/release_file.py).
RELEASE_FILE = "sunrise-release.json"
# A break-glass release's own section of its notes (break_glass_notes): a record for readers, never read back.
BREAK_GLASS_SECTION = "### Break-glass release"


def stable_parent(tag: str = "", verify: bool = True) -> train.Release | None:
    """The newest published (not draft, not pre-release) stable release on the target before `tag`, or the newest
    without one: the one parent every stable check compares with (stable-mod-api, the break-glass rules, the legs a
    break-glass daily runs). Refuses while a stable tag above it, other than `tag`, has no published release (a
    failed publish's reservation, or one pushed by hand): the next version would be judged against the wrong
    release. Its commit is fetched, so a shallow checkout can diff against it. With `verify` (every caller but tests),
    the commit its tag names must be the one its own signed sunrise-release.json names: a
    tag moved since the release was published refuses."""
    bound = train.stable_version(tag)[1] if tag else None
    published = {release["tag_name"]: release for release in published_releases() if not release["prerelease"]
                 and (found := train.stable_version(release["tag_name"])) and found[0] == train.TARGET}
    newest = max((name for name in published if bound is None or train.stable_version(name)[1] < bound),
                 key=train.stable_key, default=None)
    floor = train.stable_version(newest)[1] if newest else None
    tags = train.origin_tags()
    unpublished = sorted(name for name in tags if name != tag and name not in published
                         and (found := train.stable_version(name)) and found[0] == train.TARGET
                         and (floor is None or found[1] > floor))
    if unpublished:
        raise SystemExit(f"::error::refused: {', '.join(unpublished)} {'is a tag' if len(unpublished) == 1 else 'are tags'} "
                         f"with no published release above {newest or 'every published release'}: publish or delete "
                         "it first")
    if not newest:
        return None
    if not (sha := tags.get(newest)):
        raise SystemExit(f"::error::refused: the published release {newest} has no tag")
    if verify:
        signed = json.loads(release_assets(newest, RELEASE_FILE)[RELEASE_FILE]).get("commit")
        if signed != sha:
            raise SystemExit(f"::error::refused: the tag {newest} names {sha}, but its signed {RELEASE_FILE} names "
                             f"{signed}")
    git("fetch", "--quiet", "--no-tags", "origin", sha)
    return train.Release(newest, sha, train.github_time(published[newest].get("published_at")))


def stable_step(tag: str, parent: train.Release | None) -> str:
    """Why `tag` can't follow `parent` (the newest published stable release before it), or "": the first release is
    0.1.0, a patch keeps the major and minor, a minor resets the patch, a major resets both."""
    version = train.stable_version(tag)[1]
    if parent and version <= train.stable_version(parent.tag)[1]:
        return f"{tag} is not newer than {parent.tag}"
    if not parent:
        return "" if version == train.FIRST_STABLE else (
            f"the first stable release on {train.TARGET} is {train.TARGET}-0.1.0, not {tag}")
    major, minor, patch = train.stable_version(parent.tag)[1]
    if version[:2] == (major, minor) or (version[0] == major and version[2] == 0) or version[1:] == (0, 0):
        return ""
    return (f"{tag} does not follow {parent.tag}: a patch keeps {major}.{minor}, a minor is {major}.N.0 and a major "
            "N.0.0")


def break_glass_legs(sha: str) -> None:
    """The legs a break-glass daily run of `sha` runs (daily.yml's resolve: `legs`, a JSON list), against the newest
    published stable release. Refuses a commit that is not a new
    descendant of it, or whose changes need the full daily."""
    parent = stable_parent(verify=True)
    if not parent:
        raise SystemExit(f"::error::no published stable release on {train.TARGET}: run the full daily")
    if sha == parent.sha or not is_ancestor(parent.sha, sha):
        raise SystemExit(f"::error::{sha} is not a new commit descending from {parent.tag} ({parent.sha})")
    plain, special = train.changed_since(parent.sha, sha)
    legs, unmapped = train.break_glass_legs(plain)
    if unmapped or special:
        raise SystemExit("::error::these changes need the full daily: "
                         + ", ".join(f"`{path!r}`" for path in unmapped + special))
    line = f"Break-glass legs of {sha[:9]} against {parent.tag}: {', '.join(legs)}"
    print(line)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
            summary.write(f"### {line}\n")
    output(legs=json.dumps(legs))


def break_glass_notes(notes: Path) -> None:
    """Adds a break-glass release's own section to its notes, above the closing disclaimer: the tag and commit, its
    parent, the legs it passed, why (BREAK_GLASS_REASON) and the approvals the break-glass job verified
    (BREAK_GLASS_APPROVALS), which must be there."""
    env = os.environ
    approvals = env.get("BREAK_GLASS_APPROVALS", "")
    if not approvals:
        raise SystemExit("::error::refused: a break-glass release without its verified approvals")
    text = notes.read_text()
    head, disclaimer, tail = text.rpartition(train.DISCLAIMER)
    if not disclaimer:
        raise SystemExit("::error::the notes end without their disclaimer")
    section = (f"{BREAK_GLASS_SECTION}\n{env['TAG']} (commit {env['SHA']}) patches {env['PARENT_TAG']} on the daily "
               f"legs its changes select ({', '.join(env['LEGS'].split())}) and dyna-checks' gate on its commit, "
               "instead of the full daily, which runs on it after publishing.\n\n"
               f"Why: {' '.join(env.get('BREAK_GLASS_REASON', '').split())}\n\nApproved by: {approvals}\n\n")
    notes.write_text(head + section + disclaimer + tail)


# The build-data header shared with sunrise_mod_surface::build_data.
BUILD_DATA_SCHEMA = 1
MixinFingerprints = dict[str, tuple[str, str, tuple[str, str] | None]]


def stable_sdk_inputs(sha: str, surface: Path | None) -> tuple[str, MixinFingerprints]:
    """Read the verified content version and mixin fingerprints; save the SDK surface for same-line comparisons."""
    names = (SDK_BUILD_DATA, SDK_SURFACE) if surface is not None else (SDK_BUILD_DATA,)
    files = released_assets(sha, SDK_INDEX, *names)
    try:
        index = json.loads(files[SDK_INDEX])
        schema = index["schema"]
        if type(schema) is not int or schema != 1:
            raise ValueError(f"sdk.json has schema {schema!r}; expected 1")
        if index["build"]["commit"] != sha:
            raise ValueError("sdk.json names the wrong build")
        content_api = index["content_api"]
        if not isinstance(content_api, str):
            raise ValueError("sdk.json has no content_api version")
        for name in names:
            asset, data = index["assets"][name], files[name]
            if asset["size"] != len(data) or asset["sha256"] != hashlib.sha256(data).hexdigest():
                raise ValueError(f"{name} does not match sdk.json")
        with tarfile.open(fileobj=io.BytesIO(files[SDK_BUILD_DATA]), mode="r:xz") as archive:
            member = archive.getmember("./functions.json")
            if not member.isfile():
                raise ValueError("functions.json is not a file")
            with archive.extractfile(member) as file:
                document = json.load(file)
        schema = document["schema"]
        if type(schema) is not int or schema != BUILD_DATA_SCHEMA:
            raise ValueError(f"functions.json has build-data schema {schema!r}; expected {BUILD_DATA_SCHEMA}")
        described = document["build"]["commit"]
        if described != sha:
            raise ValueError(f"functions.json describes game build {described}, not {sha}")
        records = document["functions"]
        if not isinstance(records, dict):
            raise ValueError("functions.json has no function map")
        functions: MixinFingerprints = {}
        for name, record in records.items():
            signature, source = record["signature_hash"], record["sha256"]
            if not isinstance(signature, str) or not isinstance(source, str):
                raise ValueError(f"functions.json: {name} has invalid fingerprints")
            # Builds from before the split digests carry none.
            digests = record.get("digests")
            if digests is not None:
                digests = digests["declaration"], digests["body"]
                if not all(isinstance(digest, str) for digest in digests):
                    raise ValueError(f"functions.json: {name} has invalid digests")
            functions[name] = signature, source, digests
    except (KeyError, TypeError, ValueError, tarfile.TarError) as error:
        raise SystemExit(f"sdk-{sha}: cannot read stable comparison inputs: {error}")
    if surface is not None:
        surface.write_bytes(files[SDK_SURFACE])
    return content_api, functions


def append_mixin_notes(notes: Path, tag: str, previous: str, old: MixinFingerprints, new: MixinFingerprints) -> None:
    """Compare the hashes the native fit reader uses, not lookup metadata: the signature and source
    hashes, and the declaration and body digests where both builds carry them."""
    def changed(old: tuple, new: tuple) -> bool:
        split = old[2] is not None and new[2] is not None and old[2] != new[2]
        return old[:2] != new[:2] or split

    changes = []
    for name in sorted(old.keys() | new.keys()):
        if name not in old:
            changes.append(f"- Added: `{name}`\n")
        elif name not in new:
            changes.append(f"- Removed: `{name}`\n")
        elif changed(old[name], new[name]):
            changes.append(f"- Changed: `{name}`\n")
    intro = f"\n### Changed mixin targets\n\nCompared with `{previous}`:\n\n"
    section = "".join(changes) or "No mixin targets changed.\n"
    # Keep the full list as an advisory release asset if it would exceed GitHub's release body limit.
    if notes.stat().st_size + len((intro + section).encode()) > 125_000:
        listing = notes.with_name("changed-mixin-targets.md")
        listing.write_text(intro + section, encoding="utf-8")
        section = ("[Full list of changed mixin targets]"
                   f"(https://github.com/sunrise-org/sunrise/releases/download/{tag}/{listing.name}).\n")
    with notes.open("a", encoding="utf-8") as out:
        out.write(intro + section)


def require_stable_lineage(sha: str, tag: str, old_sha: str, old_tag: str) -> None:
    """Backports may be merged by PR; new commits from main's first-parent line start a new major."""
    main = "refs/remotes/origin/main"
    # The publish job normally has a shallow checkout. Both sides' full ancestry is needed;
    # only fetch the candidate, its previous stable build and main, not every branch and tag.
    unshallow = ["--unshallow"] if git("rev-parse", "--is-shallow-repository") == "true" else []
    git("fetch", "--quiet", "--no-tags", *unshallow, "origin", f"+refs/heads/main:{main}", sha, old_sha)
    if not is_ancestor(old_sha, sha):
        raise SystemExit(f"::error::refused: {tag} does not descend from {old_tag}")
    # A backport merged back into main is reachable from main, but not on its first-parent line.
    candidate_commits = set(git("rev-list", sha, f"^{old_sha}").splitlines())
    main_line = git("rev-list", "--first-parent", main, f"^{old_sha}").splitlines()
    if imported := next((commit for commit in main_line if commit in candidate_commits), None):
        raise SystemExit(f"::error::refused: {tag} brings in main history ({imported}) absent from {old_tag}; "
                         "cherry-pick fixes on this line or use a new major")


def unpack(sha: str, into: Path) -> None:
    """The tree of `sha`, extracted into the new folder `into`: no checkout, worktree or repository of its own."""
    into.mkdir()
    archive = subprocess.Popen(["git", "archive", sha], cwd=REPOSITORY, stdout=subprocess.PIPE)
    subprocess.run(["tar", "-x", "-C", str(into)], stdin=archive.stdout, check=True)
    archive.stdout.close()
    if archive.wait():
        raise SystemExit(f"::error::git archive {sha} failed")


def parent_stage(sha: str, work: Path) -> str:
    """`sha`'s own sunrise-stage, built from its tree alone in `work`: cargo runs there, so no `.cargo/config.toml` of
    the candidate's checkout applies, into its own target folder, with no inherited wrapper."""
    tree = work / "judge"
    unpack(sha, tree)
    subprocess.run(["cargo", "build", "--locked", "--quiet", "--bin", "sunrise-stage",
                    "--target-dir", str(work / "judge-target")], cwd=tree, env=isolated_env(), check=True)
    return str(work / "judge-target" / "debug" / "sunrise-stage")


def json_text(value) -> str:
    """`value` as canonical JSON: its objects' keys sorted, its lists in order. Equal exactly when the JSON is, which
    Python's == is not (`true == 1`)."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def signed_surface(name: str, data: bytes) -> dict:
    """A signed surface.json as a JSON object, refused when it is not one, names a key twice in any object (a later
    value would hide the earlier one from one reader and not another) or nests deeper than this reader goes."""
    def unique(pairs: list) -> dict:
        keys = [key for key, _ in pairs]
        if len(set(keys)) != len(keys):
            raise ValueError(f"an object names {sorted(key for key in set(keys) if keys.count(key) > 1)} twice")
        return dict(pairs)
    try:
        surface = json.loads(data, object_pairs_hook=unique)
    except (ValueError, RecursionError) as error:
        raise SystemExit(f"::error::refused: {name}'s signed {SDK_SURFACE} is not JSON this reader takes: "
                         f"{type(error).__name__}: {error}")
    if not isinstance(surface, dict):
        raise SystemExit(f"::error::refused: {name}'s signed {SDK_SURFACE} is not a JSON object")
    return surface


# The member lists of each collection of a surface.json, by its schema: a member is a record of its own, under its
# owner.
SURFACE_MEMBERS = {"hooks": ("fields",), "traits": ("methods", "associated"), "types": ("fields", "variants")}


def surface_entries(surface: dict) -> dict[str, str]:
    """Each record of a surface.json as canonical JSON of its own fields, by where it is: `<collection>:<id>` (an
    extension tag's id is `<registry>/<id>`), and a member by its owner's place, its list and its id. An owner's
    member lists (SURFACE_MEMBERS) are left out of it, empty or not."""
    entries = {}
    for collection, records in surface.items():
        for record in records if isinstance(records, list) else ():
            lists = SURFACE_MEMBERS.get(collection, ())
            place = f"{collection}:" + (f"{record['registry']}/{record['id']}" if collection == "extension_tags"
                                        else record["id"])
            entries[place] = json_text({name: value for name, value in record.items() if name not in lists})
            for name in lists:
                for member in record.get(name, []):
                    entries[f"{place}/{name}:{member['id']}"] = json_text(member)
    return entries


def canonical_surface(surface: dict) -> dict:
    """A surface.json as compared: every top-level key kept, each value as canonical JSON (so `true` is not `1`),
    a list as its items' canonical JSON sorted (a repeated item kept), so two surfaces compare equal exactly when
    they differ at most in their lists' order."""
    return {key: sorted(map(json_text, value)) if isinstance(value, list) else json_text(value)
            for key, value in surface.items()}


def stable_mod_api(sha: str, tag: str, notes: Path) -> None:
    """Compare to the latest published stable release (stable_parent, its tag checked against its signed release
    file), not sdk.json's previous SDK or a reserved tag; judged by the parent's own sunrise-stage."""
    parsed = train.stable_version(tag)
    if not parsed or parsed[0] != train.TARGET:
        raise SystemExit(f"::error::refused: {tag} is not a stable tag on {train.TARGET}")
    target, version = parsed
    break_glass = os.environ.get("BREAK_GLASS") == "true"
    parent = stable_parent(tag, verify=True)
    if wrong := stable_step(tag, parent):
        raise SystemExit(f"::error::refused: {wrong}")
    if parent is None:
        if break_glass:
            raise SystemExit(f"::error::refused: a break-glass release needs a published stable release to compare with")
        with notes.open("a", encoding="utf-8") as out:
            out.write("\n### Changed mixin targets\n\nNo earlier stable release to compare.\n")
        return
    old_tag, old_sha = parent.tag, parent.sha
    old_version = train.stable_version(old_tag)[1]
    same_line = old_version[0] == version[0]
    if break_glass and old_version[:2] != version[:2]:
        raise SystemExit(f"::error::refused: a break-glass release is a patch of {old_tag}, not {tag}")
    if same_line:
        require_stable_lineage(sha, tag, old_sha, old_tag)
    with tempfile.TemporaryDirectory() as directory:
        work = Path(directory)
        before, after = work / "before.json", work / "after.json"
        old_content, old_functions = stable_sdk_inputs(old_sha, before if same_line else None)
        new_content, new_functions = stable_sdk_inputs(sha, after if same_line else None)
        if same_line:
            report = work / "diff.json"
            published = {}
            for name, file in ((old_tag, before), (tag, after)):
                published[name] = signed_surface(name, file.read_bytes())
            if break_glass and json_text(published[old_tag]) != json_text(published[tag]):
                raise SystemExit(f"::error::refused: {tag} changes surface.json from {old_tag}'s; a break-glass "
                                 "release never does")
            # No candidate code judges the candidate: the parent's own sunrise-stage, built from the parent's tree
            # alone, exports both trees and diffs them. Its export of the parent must be the parent's signed surface.
            stage = parent_stage(old_sha, work)
            exported = {}
            for name, commit in (("parent", old_sha), ("candidate", sha)):
                tree, out = work / f"{name}-tree", work / f"{name}.json"
                unpack(commit, tree)
                done = subprocess.run([stage, "surface", str(tree), str(out)], cwd=work, env=isolated_env(),
                                      capture_output=True, text=True)
                if done.returncode:
                    raise SystemExit(f"::error::refused: {old_tag}'s sunrise-stage cannot export {name} {commit}: "
                                     f"{done.stderr.strip()}")
                exported[name] = json.loads(out.read_bytes())
            if canonical_surface(exported["parent"]) != canonical_surface(published[old_tag]):
                raise SystemExit(f"::error::refused: {old_tag}'s own sunrise-stage does not export its published "
                                 "surface.json from its tree")
            if break_glass and json_text(exported["candidate"]) != json_text(exported["parent"]):
                raise SystemExit(f"::error::refused: {old_tag}'s sunrise-stage exports a different surface from "
                                 f"{tag}'s tree; a break-glass release never changes it")
            # Both files are plain SDK surfaces: no native internals, loops, or change-note waivers.
            subprocess.run([stage, "surface-diff", str(work / "parent.json"), str(work / "candidate.json"), "--json",
                            str(report)], cwd=work, env=isolated_env(), check=True)
            verdict = json.loads(report.read_text())["verdict"]
            minor = old_version[1] != version[1]
            allowed = {"identical", "additive"} if minor else {"identical"}
            problems = []
            if verdict not in allowed:
                problems.append(f"mod API is {verdict} (see the changed records above)")
            if old_content != new_content:
                problems.append(f"content_api changed from {old_content!r} to {new_content!r}")
            # The signed surface.json is what modders build against, written by the candidate's own exporter. The
            # parent's tool reads it as the SDK's readers do, refusing a stored hash its contract doesn't give
            # (surface-read), and diffs it against its own export of the candidate's tree: the step allows what it
            # allows there too. A minor's exporter may add records, fields and collections the parent's reader
            # doesn't know, but every record the parent's tool exports must read back with each known field equal
            # (the diff trusts an unchanged hash, and some fields are outside it). A patch's is that export exactly,
            # whatever the order of its lists.
            signed, export = published[tag], exported["candidate"]
            known = work / "signed-known.json"
            read = subprocess.run([stage, "surface-read", str(after), str(known)], cwd=work, env=isolated_env(),
                                  capture_output=True, text=True)
            if not read.returncode:
                read = subprocess.run([stage, "surface-diff", str(work / "candidate.json"), str(after), "--json",
                                       str(work / "signed-diff.json")], cwd=work, env=isolated_env(),
                                      capture_output=True, text=True)
            if read.returncode:
                problems.append(f"the parent's tool cannot read the signed {SDK_SURFACE} for {tag}: "
                                f"{read.stderr.strip()}")
            else:
                if (read_as := json.loads((work / "signed-diff.json").read_text())["verdict"]) not in allowed:
                    problems.append(f"the parent's tool reads the signed {SDK_SURFACE} for {tag} as {read_as} "
                                    "against its export of the candidate's tree")
                entries = surface_entries(json.loads(known.read_bytes()))
                if changed := sorted(key for key, record in surface_entries(export).items()
                                     if entries.get(key) != record):
                    problems.append(f"the signed {SDK_SURFACE} for {tag} changes or lacks {len(changed)} records the "
                                    "parent's tool exports from its tree: " + ", ".join(changed[:5]))
            if signed.get("engine_items"):
                problems.append(f"the signed {SDK_SURFACE} for {tag} lists native internals, which an SDK surface "
                                "never holds")
            if signed.get("schema") != export.get("schema"):
                problems.append(f"the signed {SDK_SURFACE} for {tag} has schema {signed.get('schema')!r}, the parent "
                                f"tool's export {export.get('schema')!r}")
            if not minor and canonical_surface(signed) != canonical_surface(export):
                problems.append(f"the signed {SDK_SURFACE} for {tag} is not what the parent's tool exports from its "
                                "tree")
            if problems:
                step = "minor" if minor else "patch"
                raise SystemExit(f"::error::refused: {tag} is a {step} release after {old_tag}: {'; '.join(problems)}")
    append_mixin_notes(notes, tag, old_tag, old_functions, new_functions)


def reserve_stable_tag(sha: str, tag: str) -> None:
    """Right before a stable release is created, a publish-only retry too: the approved commit is still the release
    branch's head with its passed daily run (release_head), `tag` still comes next on its target, and the tag is
    created at that commit in one write, which GitHub refuses when the tag exists. A tag found at another commit is
    refused, never reused; one already at this commit (an earlier attempt's) is kept."""
    train.require_ref(os.environ.get("GITHUB_REF", ""), "stable")
    found = train.stable_version(tag)
    if not found or found[0] != train.TARGET:
        raise SystemExit(f"::error::refused: {tag} is not a stable tag on {train.TARGET}")
    break_glass = os.environ.get("BREAK_GLASS") == "true"
    if train.release_head(sha, daily=not break_glass) != sha:
        raise SystemExit(f"::error::refused: {sha} is no longer the approved release head")
    tags = train.origin_tags()
    # The parent the pick judged it against and its version step again; its surface was checked by the stable-mod-api
    # step just before this.
    parent = stable_parent(tag, verify=True)
    found = (parent.tag, parent.sha) if parent else ("", "")
    if found != (os.environ.get("PARENT_TAG", ""), os.environ.get("PARENT_SHA", "")):
        raise SystemExit(f"::error::refused: the parent release changed since the pick ({parent})")
    if wrong := stable_step(tag, parent):
        raise SystemExit(f"::error::refused: {wrong}")
    if break_glass:
        # Its rules and both approvals again.
        problems, _ = train.break_glass_problems(sha, tag, parent)
        if problems:
            raise SystemExit("::error::refused: " + "; ".join(problems))
        train.break_glass_approvals(os.environ["GITHUB_RUN_ID"], os.environ["GITHUB_RUN_ATTEMPT"])
    there = tags.get(tag)
    if there and there != sha:
        raise SystemExit(f"::error::refused: the tag {tag} already names {there}, not {sha}")
    train.next_stable_tag(tags, version=tag.split("-", 1)[1], own=tag if there else "")
    if there:
        print(f"`{tag}` already names {sha[:9]}")
        return
    created = subprocess.run(["gh", "api", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs", "-f",
                              f"ref=refs/tags/{tag}", "-f", f"sha={sha}"], capture_output=True, text=True)
    if created.returncode:
        # Made by someone else since the lookup: only the same commit is acceptable.
        if train.origin_commit(f"refs/tags/{tag}") != sha:
            raise SystemExit(f"::error::refused: could not create {tag} at {sha}: {created.stderr.strip()}")
    print(f"tagged {sha[:9]} `{tag}`")


def rotate() -> None:
    """The current `nightly` becomes nightly-<its date>, so tonight's can take the tag."""
    current = gh("release", "view", "nightly", "--json", "name", check=False)
    if not current:
        return
    m = TITLE_DATE.search(json.loads(current)["name"])
    dated = f"nightly-{m.group(1)}" if m else f"nightly-{dt.date.today() - dt.timedelta(days=1)}"
    if gh("release", "view", dated, "--json", "tagName", check=False):
        gh("release", "delete", dated, "--cleanup-tag", "--yes")  # a second nightly the same day replaces the first
    gh("release", "edit", "nightly", "--tag", dated)
    gh("api", "-X", "DELETE", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs/tags/nightly", check=False)
    print(f"nightly -> {dated}")


def stale(tags: list[dict], today: dt.date, days: int) -> list[str]:
    """The dated nightly pre-releases older than `days`: never a release, never `nightly` itself."""
    cutoff = today - dt.timedelta(days=days)
    return [r["tagName"] for r in tags if r["isPrerelease"] and (m := DATED.match(r["tagName"]))
            and dt.date.fromisoformat(m.group(1)) < cutoff]


def prune(days: int) -> None:
    tags = json.loads(gh("release", "list", "--limit", "200", "--json", "tagName,isPrerelease"))
    for tag in stale(tags, dt.datetime.now(dt.timezone.utc).date(), days):
        gh("release", "delete", tag, "--cleanup-tag", "--yes")
        print(f"deleted {tag}")


DEV = re.compile(r"^dev-(\d{4}-\d{2}-\d{2})-[0-9a-f]{7,40}$")


def stale_dev_builds(tags: list[dict], today: dt.date, days: int) -> list[str]:
    """The Dev pre-releases older than `days`, never the newest one, never anything not named
    dev-<date>-<short> (an sdk-<sha> SDK above all)."""
    dev_builds = [r["tagName"] for r in tags if r["isPrerelease"] and DEV.match(r["tagName"])]
    if not dev_builds:
        return []
    newest = max(dev_builds, key=lambda tag: DEV.match(tag).group(1))
    cutoff = today - dt.timedelta(days=days)
    return [tag for tag in dev_builds
            if tag != newest and dt.date.fromisoformat(DEV.match(tag).group(1)) < cutoff]


def prune_dev(days: int) -> None:
    tags = json.loads(gh("release", "list", "--limit", "200", "--json", "tagName,isPrerelease"))
    for tag in stale_dev_builds(tags, dt.datetime.now(dt.timezone.utc).date(), days):
        gh("release", "delete", tag, "--cleanup-tag", "--yes")
        print(f"deleted {tag}")


BUILD = re.compile(r"^nightly/build-(\d{4}-\d{2}-\d{2})-(\d+)$")


def stale_builds(branches: dict[str, str], published: set[str], today: dt.date, days: int) -> list[str]:
    """The nightly/build-<date>-<n> branches (name -> commit) to delete: older than `days`, never the newest build,
    never one whose commit a published nightly release points to, never a branch not named like a build."""
    builds = {name: sha for name, sha in branches.items() if BUILD.match(name)}
    if not builds:
        return []
    order = lambda name: (BUILD.match(name).group(1), int(BUILD.match(name).group(2)))  # noqa: E731
    newest = max(builds, key=order)
    cutoff = today - dt.timedelta(days=days)
    return sorted(name for name, sha in builds.items() if name != newest and sha not in published
                  and dt.date.fromisoformat(BUILD.match(name).group(1)) < cutoff)


def remote_refs(pattern: str) -> dict[str, str]:
    """origin's refs matching `pattern`: short name -> commit (annotated tags peeled)."""
    refs: dict[str, str] = {}
    for line in git("ls-remote", "origin", pattern).splitlines():
        sha, ref = line.split("\t", 1)
        name = ref.removeprefix("refs/heads/").removeprefix("refs/tags/")
        if name.endswith("^{}"):
            refs[name[:-3]] = sha  # the peeled commit wins over the tag object
        else:
            refs.setdefault(name, sha)
    return refs


def prune_builds(days: int) -> None:
    """Deletes old nightly/build-* branches on origin through the API, one line per deletion (the run summary)."""
    branches = remote_refs("refs/heads/nightly/build-*")
    tags = {name: sha for name, sha in remote_refs("refs/tags/nightly*").items()
            if name == "nightly" or DATED.match(name)}
    today = dt.datetime.now(dt.timezone.utc).date()
    doomed = stale_builds(branches, set(tags.values()), today, days)
    for name in doomed:
        gh("api", "-X", "DELETE", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs/heads/{name}")
        print(f"- deleted `{name}` ({branches[name][:7]})")
    print(f"{len(doomed)} of {len(branches)} nightly/build-* branches deleted (kept: the newest, the published "
          f"nightlies' and the last {days} days')")


def keep_source(sha: str, today: str) -> None:
    """Tags the published nightly's commit for good (source_tag); a rerun of the same build finds it there."""
    tag = source_tag(today, sha)
    there = remote_refs(f"refs/tags/{tag}").get(tag)
    if there == sha:
        print(f"`{tag}` already names {sha[:7]}")
        return
    if there:
        raise SystemExit(f"`{tag}` names {there[:7]}, not {sha[:7]}")
    gh("api", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs", "-f", f"ref=refs/tags/{tag}", "-f", f"sha={sha}")
    print(f"tagged {sha[:7]} `{tag}`")


def report_musl_failure(kind: str, step: str) -> None:
    """One open bug per channel and failure step, like the daily's deduped leg failures."""
    title = f"{kind.capitalize()}: musl {step} failed"
    repo = os.environ["GITHUB_REPOSITORY"]
    run = f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{os.environ['GITHUB_RUN_ID']}"
    print(f"::warning::{title}: {run}")
    if os.environ.get("DRY_RUN") != "false" or not train.publishes(os.environ.get("GITHUB_REF", ""), kind):
        return
    pages = train.gh_json("api", "--paginate", f"repos/{repo}/issues?state=open&labels=bug&per_page=100")
    if any(issue["title"] == title and "pull_request" not in issue for page in pages for issue in page):
        return
    body = ("> 🤖 Filed by Aria's agent swarm, Dyna, not by Aria.\n\n"
            f"The optional musl Linux `{step}` step failed on the {kind} channel. "
            f"See [the workflow run]({run}/attempts/{os.environ['GITHUB_RUN_ATTEMPT']}). "
            "The regular release does not depend on this step.")
    gh("issue", "create", "--repo", repo, "--label", "bug", "--title", title, "--body", body)


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def download_asset(repo: str, asset_id: int) -> bytes:
    return subprocess.run(["gh", "api", "-H", "Accept: application/octet-stream",
                           f"repos/{repo}/releases/assets/{asset_id}"], check=True, capture_output=True).stdout


def asset_sha256(repo: str, asset: dict) -> str:
    """GitHub's own digest of an uploaded asset, or the hash of its downloaded bytes where it gives none."""
    digest = asset.get("digest") or ""
    if digest.startswith("sha256:"):
        return digest.removeprefix("sha256:")
    return hashlib.sha256(download_asset(repo, asset["id"])).hexdigest()


def bound_release(repo: str, release_id: str, tag: str, sha: str) -> dict:
    """Release `release_id`, refused unless it is still `tag` and that tag still names `sha`. The check holds when it
    is made, not after: writes go to the release by id, so a tag moved later can not redirect them."""
    release = json.loads(gh("api", f"repos/{repo}/releases/{release_id}"))
    commit = gh("api", f"repos/{repo}/git/ref/tags/{tag}", "--jq", ".object.sha")
    if release["tag_name"] != tag or commit != sha:
        sys.exit(f"Release {release_id} is no longer {tag} at {sha} (it is {release['tag_name']}, "
                 f"and {tag} names {commit or 'nothing'}); nothing was attached")
    return release


def release_identity(tag: str, sha: str, sums: Path) -> None:
    repo = os.environ["GITHUB_REPOSITORY"]
    release_id = str(json.loads(gh("api", f"repos/{repo}/releases/tags/{tag}"))["id"])
    release = bound_release(repo, release_id, tag, sha)
    published = [asset for asset in release["assets"] if asset["name"] == sums.name]
    if not published or asset_sha256(repo, published[0]) != file_sha256(sums):
        sys.exit(f"Release {release_id} ({tag}) does not hold this run's {sums.name}")
    output(id=release_id)


def upload_asset(upload_url: str, path: Path) -> dict:
    """One POST to the release's own upload address, which names the release by id."""
    request = urllib.request.Request(f"{upload_url}?name={urllib.parse.quote(path.name)}", data=path.read_bytes(),
                                     method="POST", headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}",
                                                             "Accept": "application/vnd.github+json",
                                                             "Content-Type": "application/octet-stream"})
    with urllib.request.urlopen(request, timeout=600) as response:
        return json.load(response)


def attach(release_id: str, tag: str, sha: str, files: list[Path]) -> None:
    """Uploads `files` in order. GitHub has no all-or-nothing upload of several assets, so a failure can leave the
    first ones attached: a rerun keeps each already there with the same bytes, refuses one with other bytes without
    replacing it, and deletes only an empty `starter` placeholder, an upload GitHub never began storing. Any other
    asset of that name that is not `uploaded` is refused and left in place."""
    repo = os.environ["GITHUB_REPOSITORY"]
    release = bound_release(repo, release_id, tag, sha)
    upload_url = release["upload_url"].split("{", 1)[0]
    existing = {asset["name"]: asset for asset in release["assets"]}
    for path in files:
        asset = existing.get(path.name)
        if asset and asset["state"] == "uploaded":
            if asset_sha256(repo, asset) != file_sha256(path):
                sys.exit(f"{path.name} is already on release {release_id} with other bytes; left as it is")
            print(f"{path.name}: already attached with these bytes")
            continue
        if asset and (asset["state"] != "starter" or asset["size"]):
            sys.exit(f"{path.name} is on release {release_id} as {asset['state']} with {asset['size']} bytes; "
                     "left as it is")
        if asset:
            print(f"{path.name}: removing an empty upload placeholder")
            gh("api", "--method", "DELETE", f"repos/{repo}/releases/assets/{asset['id']}")
        uploaded = upload_asset(upload_url, path)
        if uploaded["state"] != "uploaded" or uploaded["size"] != path.stat().st_size:
            sys.exit(f"{path.name}: the upload ended {uploaded['state']} with {uploaded['size']} bytes")
        print(f"{path.name}: attached")
    # Read the release back by id: every file is there, complete, with these bytes.
    final = {asset["name"]: asset for asset in bound_release(repo, release_id, tag, sha)["assets"]}
    wrong = [path.name for path in files if path.name not in final or final[path.name]["state"] != "uploaded"
             or asset_sha256(repo, final[path.name]) != file_sha256(path)]
    if wrong:
        sys.exit(f"Release {release_id} does not hold these bytes for: {', '.join(wrong)}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    picker = sub.add_parser("pick")
    picker.add_argument("--force", action="store_true")
    picker.add_argument("--kind", choices=["nightly", "dev", "stable"], default="nightly")
    sub.add_parser("rotate")
    sub.add_parser("require-sdk").add_argument("sha")
    reserve = sub.add_parser("reserve-stable-tag")
    reserve.add_argument("sha")
    reserve.add_argument("tag")
    sub.add_parser("sdk-archives")
    sub.add_parser("sdk-assets")
    index = sub.add_parser("sdk-index")
    index.add_argument("sha")
    index.add_argument("out", type=Path)
    previous = sub.add_parser("sdk-previous")
    previous.add_argument("sha")
    previous.add_argument("out", type=Path)
    sums = sub.add_parser("verify-sums")
    sums.add_argument("--tag", required=True)
    sums.add_argument("--signature", default="SHA256SUMS.sig")
    sums.add_argument("--only", action="append", default=[])
    sums.add_argument("--key", action="append")
    sums.add_argument("--publishing", action="store_true")
    sums.add_argument("folder", type=Path)
    sub.add_parser("break-glass-legs").add_argument("sha")
    sub.add_parser("break-glass-notes").add_argument("notes", type=Path)
    stable_api = sub.add_parser("stable-mod-api")
    stable_api.add_argument("sha")
    stable_api.add_argument("tag")
    stable_api.add_argument("notes", type=Path)
    sub.add_parser("prune").add_argument("--days", type=int, default=7)
    sub.add_parser("prune-dev").add_argument("--days", type=int, default=7)
    sub.add_parser("prune-builds").add_argument("--days", type=int, default=7)
    keep = sub.add_parser("keep-source")
    keep.add_argument("sha")
    keep.add_argument("date")
    musl = sub.add_parser("report-musl-failure")
    musl.add_argument("kind", choices=["nightly", "dev", "stable"])
    musl.add_argument("step", choices=["build", "sign", "verify", "attach", "sign or attach"])
    identity = sub.add_parser("release-identity")
    identity.add_argument("tag")
    identity.add_argument("sha")
    identity.add_argument("sums", type=Path)
    attaching = sub.add_parser("attach")
    attaching.add_argument("release_id")
    attaching.add_argument("tag")
    attaching.add_argument("sha")
    attaching.add_argument("files", type=Path, nargs="+")
    args = ap.parse_args()
    if args.cmd == "pick":
        if args.kind == "dev":
            pick_dev()
        elif args.kind == "stable":
            pick_stable()
        else:
            pick(args.force)
    elif args.cmd == "require-sdk":
        require_sdk(args.sha)
    elif args.cmd == "reserve-stable-tag":
        reserve_stable_tag(args.sha, args.tag)
    elif args.cmd == "sdk-archives":
        print("\n".join(SDK_ARCHIVES))
    elif args.cmd == "sdk-assets":
        print("\n".join(SDK_ASSETS))
    elif args.cmd == "sdk-index":
        args.out.write_bytes(published_index(args.sha))
    elif args.cmd == "sdk-previous":
        if build := previous_sdk(args.sha):
            args.out.write_bytes(released(build, SDK_SURFACE))
            print(build)
    elif args.cmd == "verify-sums":
        verify_sums(args.folder, args.tag, args.only, args.signature, args.key, args.publishing)
    elif args.cmd == "break-glass-legs":
        break_glass_legs(args.sha)
    elif args.cmd == "break-glass-notes":
        break_glass_notes(args.notes)
    elif args.cmd == "stable-mod-api":
        if not train.FULL_SHA.fullmatch(args.sha):
            stable_api.error("sha must be a full lowercase commit SHA")
        stable_mod_api(args.sha, args.tag, args.notes)
    elif args.cmd == "rotate":
        rotate()
    elif args.cmd == "prune-builds":
        prune_builds(args.days)
    elif args.cmd == "prune-dev":
        prune_dev(args.days)
    elif args.cmd == "keep-source":
        keep_source(args.sha, args.date)
    elif args.cmd == "report-musl-failure":
        report_musl_failure(args.kind, args.step)
    elif args.cmd == "release-identity":
        release_identity(args.tag, args.sha, args.sums)
    elif args.cmd == "attach":
        attach(args.release_id, args.tag, args.sha, args.files)
    else:
        prune(args.days)


if __name__ == "__main__":
    main()
