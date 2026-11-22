#!/usr/bin/env python3
"""Horizon -> Sunrise in the tracked files' contents and paths, and the package extensions .hmod, .hpack and
.hshader -> .smod, .spack and .sshader; then run cargo fmt and let Cargo re-sort lockfiles. Usage: tools/rebrand.py [--dry-run]"""
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

DRY = "--dry-run" in sys.argv
os.chdir(Path(__file__).resolve().parent.parent)

# Files whose strings name an old URL on purpose: the modding book's stale-URL guard and its fixtures. A run
# that rewrote them would leave a guard that passes because it no longer recognises what it refuses.
SKIP_FILES = re.compile(
    r"(^|/)(LICENSE[^/]*|LICENSES/.*|NOTICE[^/]*|COPYING[^/]*|CLA\.md|GOVERNANCE\.md)$"
    r"|^tools/rebrand\.py$"
    r"|^tools/modding_concept_pages\.py$|^tests/test_modding_concept_pages\.py$"
    r"|^tests/test_modding_book_links\.py$")

PROTECT = [
    # Real repositories and their pages. Horizon's own projects (the machine sample, the mixins tool,
    # the registry) keep their links. Historical references to the old mod API repository
    # also stay pinned: their issue numbers belong to that tracker, not this fork.
    r"(?:https?://)?(?:www\.)?github\.com/horizon-rust\b[\w./#?=&-]*",
    r"\bhorizon-rust\.github\.io[\w./#-]*",
    r"@?\bhorizon-rust(?:/[\w.-]+)?",
    r"(?:https?://)?(?:www\.)?github\.com/arismoko/horizon[\w./#?=&-]*",
    r"\barismoko/horizon[\w.-]*(?:/(?:issues|pull)/\d+)?(?:#\d+)?",
    r"(?:https?://)?(?:www\.)?github\.com/arismoko/horizon-machine-sample[\w./#?=&-]*",
    r"\barismoko/horizon-machine-sample[\w.-]*",
    # Vanilla's sky horizon, in prose and in its own names.
    r"\b(?:the|The|below|above|at|near|to|over|on|from|toward|towards|under|past|beyond) horizon\b(?![-_:]|\s+(?:client|repo|main|`))",
    r"\bhorizon_height\b|\b[gs]etHorizonHeight\b|\bhorizonHeight\b|\bHorizonHeight\b|\bnear_horizon\b",
    # Rust's name for the Nintendo Switch OS.
    r"target_os\s*=\s*\"horizon\"",
    # Bytes that signatures cover, checked by verifiers outside this repository.
    r"\bhorizon (?=release\b|native build\b)",
    # A world generation salt: renaming it would change what a seed generates.
    r"\"horizon:biome_region\"",
    # The earlier launchers' folders and keychain service, which a Sunrise build reads to carry data over.
    r"\bhorizon-fork-arismoko[\w/.-]*",
    r"\bdev\.playhorizon\.launcher\b",
    # horizon-rust/horizon-mixins: its crates, binary, generated names and the files it reads.
    r"(?i:horizon[-_ ]mixin)[\w-]*",
    r"\.horizon-revision\b|\bhorizon_revision\b",
]
PROTECT_RE = re.compile("|".join(f"(?:{p})" for p in PROTECT))
BRAND = re.compile(r"horizon(?!tal)", re.IGNORECASE)
LICENCE_LINE = re.compile(r"(?i)copyright|MIT[- ]licen[cs]e|MIT licensed|MIT-Horizon|Horizon contributors|came from Horizon|Horizon's MIT|SPDX-License")
# Lines that explain or encode a kept name; left as they are.
KEEP_LINE = re.compile(r'through the rename|Horizon\'s name, kept|before the Sunrise rename|"playhorizon\.dev": "playsunrise\.net"|^\s+"mods/horizon/src/", "mods/horizon-lab/src/"\)\):|sunriselab/" in path or "/dev/horizonlab/"')
LICENCE_PATH = re.compile(r"(^|/)(LICENSES/[^/]*|[^/]*LICEN[CS]E[^/]*)$")
# Sunrise's package extensions, as a whole word in any spelling (`.hmod`, `"hmod"`, `hmod_path`, `HMOD`; never `chmod`).
PACKAGE = re.compile(r"(?<![A-Za-z0-9])([Hh])(mod|pack|shader|MOD|PACK|SHADER)(?![A-Za-z0-9])")
# The journal is history, and roadmap.json is generated from it.
PACKAGE_SKIP = re.compile(r"^roadmap/journal/|^roadmap/roadmap\.json$")


def package(text):
    return PACKAGE.sub(lambda m: ("S" if m.group(1) == "H" else "s") + m.group(2), text)


def case(m):
    s = m.group(0)
    if s.isupper():
        return "SUNRISE"
    return "Sunrise" if s[0].isupper() else "sunrise"


def stashed(pattern, text, convert, mark):
    keep = []

    def stash(m):
        keep.append(m.group(0))
        return f"{mark}{len(keep) - 1}{mark}"

    text = convert(pattern.sub(stash, text))
    return re.sub(f"{mark}(\\d+){mark}", lambda m: keep[int(m.group(1))], text)


def convert_line(text):
    def brand(t):
        # The fork's own repository and Pages site moved with the rename; the upstream projects stay
        # protected above, and a later run of this tool names the new owner instead of `arismoko`.
        t = re.sub(r"\barismoko/horizon(?![\w.-])", "sunrise-org/sunrise", t)
        t = t.replace("arismoko.github.io/sunrise", "sunrise-org.github.io/sunrise")
        t = BRAND.sub(case, t.replace("playhorizon.dev", "playsunrise.net"))
        t = re.sub(r"([Pp]lay[Ss]unrise)\.dev\b", r"\1.net", t)
        return t.replace("upstream Sunrise", "upstream Horizon").replace("fork of Sunrise", "fork of Horizon")

    return stashed(PROTECT_RE, text, brand, "\x00")


def convert(text):
    return "".join(
        line if KEEP_LINE.search(line)
        else line.replace("crates/horizon-", "crates/sunrise-") if LICENCE_LINE.search(line)
        else convert_line(line)
        for line in text.splitlines(keepends=True)
    )


SPELLED = [(", ".join(f"b'{c}'" for c in "horizon"), ", ".join(f"b'{c}'" for c in "sunrise"))]
for old, new in [("horizon", "sunrise"), ("Horizon", "Sunrise"), ("HORIZON", "SUNRISE")]:
    SPELLED.append(("".join(f"\\x{b:02x}" for b in old.encode()), "".join(f"\\x{b:02x}" for b in new.encode())))
    SPELLED.append((old.encode().hex(), new.encode().hex()))
    SPELLED.append((old.encode().hex().upper(), new.encode().hex().upper()))


# The biome region salt as a saved world file spells it, in hex: kept like the salt itself.
SALT_HEX = "horizon:biome_region".encode().hex()


def convert_rust(text):
    text = text.replace(SALT_HEX, "\x03")
    for old, new in SPELLED:
        text = text.replace(old, new)
    return text.replace("\x03", SALT_HEX)


def convert_path(path):
    if not PACKAGE_SKIP.search(path):
        path = package(path)
    parts = path.split("/")
    keep_last = bool(LICENCE_PATH.search(path))
    return "/".join(
        p if keep_last and i == len(parts) - 1 or path.startswith("LICENSES/") else BRAND.sub(case, p)
        for i, p in enumerate(parts)
    )


def main():
    tracked = [f for f in subprocess.run(["git", "ls-files", "-z"], capture_output=True, check=True).stdout.decode().split("\0") if f]
    files = [f for f in tracked if not os.path.islink(f)]
    links = [f for f in tracked if os.path.islink(f)]
    renamed = {f: convert_path(f) for f in tracked if convert_path(f) != f and f != "tools/rebrand.py"}
    clash = [(a, b) for a, b in renamed.items() if b in set(tracked) and b not in renamed]
    targets = defaultdict(list)
    for a, b in renamed.items():
        targets[b].append(a)
    dup = {b: a for b, a in targets.items() if len(a) > 1}
    if clash or dup:
        sys.exit(f"path clashes: {clash[:20]} {list(dup.items())[:20]}")
    changed = 0
    for f in files:
        if SKIP_FILES.search(f):
            continue
        raw = Path(f).read_bytes()
        if b"\0" in raw[:8192]:
            continue
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
        out = convert(text)
        if f.endswith(".rs"):
            out = convert_rust(out)
        if not PACKAGE_SKIP.search(f):
            out = package(out)
        if out != text:
            changed += 1
            if not DRY:
                Path(f).write_text(out, encoding="utf-8", newline="")
    if not DRY:
        for f in links:
            target = os.readlink(f)
            if convert_path(target) != target:
                os.unlink(f)
                os.symlink(convert_path(target), f)
        for a, b in sorted(renamed.items(), key=lambda kv: -kv[0].count("/")):
            os.makedirs(os.path.dirname(b) or ".", exist_ok=True)
            os.rename(a, b)
        for root, _, _ in os.walk(".", topdown=False):
            if ".git" not in root.split(os.sep) and root != "." and not os.listdir(root):
                os.rmdir(root)
    print(f"contents changed: {changed}, paths renamed: {len(renamed)}")


if __name__ == "__main__":
    main()
