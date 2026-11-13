#!/usr/bin/env python3
"""Prove the move's Rust lines, allowing only the reviewed seam in seam.json.

Usage: moved_lines_check.py BASE [--show-unmatched]

Both sides are rustfmt-normalized *after* normalizing crate paths and visibility,
so path length/line wrapping cannot hide changes. Imports, module declarations,
and the new crate's lib.rs are structural, not moved implementation. New tests
are excluded explicitly. No other changed implementation line may disappear.
The immutable seam.json lists the exact unmatched old/new lines for review.
"""
from collections import Counter
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).parent
MOVED = ["attachments.rs", "attachments/index.rs", "attachments/lifetime.rs",
         "attachments/observer.rs", "attachments/resident.rs"]


def original(base, path):
    result = subprocess.run(["git", "show", f"{base}:{path}"], cwd=ROOT, capture_output=True, text=True)
    return result.stdout if result.returncode == 0 else ""


def lines(source):
    source = source.replace("horizon_mod_runtime::attachments", "horizon_world::attachments")
    source = source.replace("horizon_mod_runtime::impl_attachment_lifetime", "horizon_world::impl_attachment_lifetime")
    source = source.replace("horizon_mod_runtime::registry_keys::", "super::registry_keys::")
    # Strip whole imports, including grouped multiline imports, not implementation.
    source = re.sub(r"(?m)^\s*(?:pub(?:\([^)]*\))? )?use [^;]+;", "\nconst _: () = ();", source)
    source = re.sub(r"(?m)^\s*(?:pub(?:\([^)]*\))? )?mod \w+;", "\nconst _: () = ();", source)
    source = re.sub(r"\bpub(?:\([^)]*\))? ", "", source)
    result = subprocess.run(["rustfmt", "--edition", "2024", "--config", "skip_children=true", "--emit", "stdout"], input=source, text=True, capture_output=True, check=True)
    return Counter(line.strip() for line in result.stdout.splitlines() if line.strip() and line.strip() != "const _: () = ();")


def main():
    base = sys.argv[1]
    manifest = tomllib.loads((ROOT / "crates/horizon-mod-runtime/Cargo.toml").read_text())
    assert manifest["dependencies"] == {"horizon-core": {"path": "../horizon-core"}}
    assert "target" not in manifest and "build-dependencies" not in manifest
    paths = set(subprocess.check_output(["git", "diff", "--no-renames", "--name-only", base], cwd=ROOT, text=True).splitlines())
    paths |= {str(p.relative_to(ROOT)) for p in (ROOT / "crates/horizon-mod-runtime/src").rglob("*.rs")}
    paths = sorted(p for p in paths if p.endswith(".rs") and not p.startswith("tools/") and p not in {"crates/horizon-mod-runtime/src/lib.rs", "crates/horizon-mod-runtime/tests/registry_keys.rs"})
    assert (ROOT / "crates/horizon-mod-runtime/tests/registry_keys.rs").read_text() == (HERE / "registry_tests.rs").read_text()
    before, after = Counter(), Counter()
    for path in paths:
        before.update(lines(original(base, path)))
        file = ROOT / path
        after.update(lines(file.read_text() if file.exists() else ""))
    removed, added = before - after, after - before
    if "--show-unmatched" in sys.argv:
        print(json.dumps({"removed": dict(sorted(removed.items())), "added": dict(sorted(added.items()))}, indent=2))
        return
    seam = json.loads((HERE / "seam.json").read_text())
    assert removed == Counter(seam["removed"]), f"unreviewed removed lines: {removed - Counter(seam['removed'])}; missing: {Counter(seam['removed']) - removed}"
    assert added == Counter(seam["added"]), f"unreviewed added lines: {added - Counter(seam['added'])}; missing: {Counter(seam['added']) - added}"
    # Independently prove that the complete attachment files (including all tests)
    # moved byte-for-byte, except for the one declared generic bind method.
    total = 0
    for name in MOVED:
        old = original(base, "crates/horizon-world/src/" + name)
        if not old:
            assert name == "attachments/resident.rs"
            continue
        new = (ROOT / "crates/horizon-mod-runtime/src" / name).read_text()
        if name == "attachments/lifetime.rs":
            new = re.sub(r"    /// Bind an authoritative entity's map.*?(?=    /// Whether this entity owns)", "", new, flags=re.S)
        assert old == new, f"attachment implementation changed: {name}"
        total += len(old.splitlines())
    changed = sum(removed.values()) + sum(added.values())
    assert changed < 400, f"seam grew beyond brief: {changed}"
    print(f"PASS: {total} attachment lines moved unchanged (excluding declared bind seam)")
    print(f"PASS: removed/added Rust-line multisets match across {len(paths)} files after path, visibility, import/mod normalization and the declared seam")
    print(f"Declared seam: {sum(removed.values())} removed + {sum(added.values())} added = {changed} normalized lines; new registry tests excluded")
    print("New runtime depends only on horizon-core; manifests, documentation and refactor tooling are reviewed separately.")


if __name__ == "__main__":
    main()
