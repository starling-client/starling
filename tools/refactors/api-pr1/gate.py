#!/usr/bin/env python3
"""The gate for the host split: nothing mods see about the host may change.

Two comparisons between a base commit and this checkout, both built from scratch:

1. `mod-api.wit` byte for byte except the name-resolved sets affected by host regrouping:
   host/Registrar import function lines, whole type blocks, host/Registrar/types use-list items,
   with exact membership checks. Enum cases retain their order: Wasmtime component typechecking
   requires the same case order. Every other byte (including hooks and the world) stays ordered.
   Hook IDs and dispatch are additionally compared as byte-identical generated Rust.
2. The host manifest (`host-manifest`): one line per host method with its owner trait, signature,
   receiver mutability, rustdoc and hidden flag. The base's generated host is read as it is; this
   checkout's is the generated unions plus the area files, with each area trait counted under the union
   it is a part of. This is what catches a lost or changed doc.

Usage: gate.py BASE_COMMIT [SCRATCH_DIR]
Each resolved BASE_COMMIT gets its own worktree and target folder under SCRATCH_DIR (default
`target/api-pr1-gate`); delete the folder afterwards.
"""
import filecmp
import json
import os
import subprocess
import sys
from pathlib import Path

from wit_order import HOST_IMPORTS, compare_wit

# Registrar is another name-resolved import interface. Hooks remain byte-ordered.
HOST_IMPORTS.add('registrar')


def build(directory, target, tool=False):
    """Builds `horizon-mod-api` in `directory` and returns its build output folder (the manifest tool
    is built only here: it reads text, so one tool serves both sides)."""
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    if tool:
        subprocess.run(["cargo", "build", "-p", "horizon-mod-api-gen", "--bin", "host-manifest"], cwd=directory, env=env, check=True)
    built = subprocess.run(["cargo", "build", "-p", "horizon-mod-api", "--message-format=json"],
                           cwd=directory, env=env, check=True, capture_output=True, text=True)
    outputs = [message["out_dir"] for line in built.stdout.splitlines()
               if (message := json.loads(line)).get("reason") == "build-script-executed"
               and "/horizon-mod-api#" in message["package_id"]]
    if len(outputs) != 1:
        sys.exit(f"expected one mod API build-script output, got {outputs}")
    return Path(outputs[0])


def manifest(tool, files):
    return subprocess.run([str(tool), *map(str, files)], check=True, capture_output=True, text=True).stdout


def main():
    root = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
    base = subprocess.check_output(["git", "rev-parse", sys.argv[1]], cwd=root, text=True).strip()
    scratch = (Path(sys.argv[2]) if len(sys.argv) > 2 else root / "target/api-pr1-gate").resolve() / base[:12]
    tree = scratch / "base"
    if not tree.exists():
        subprocess.run(["git", "worktree", "add", "--detach", str(tree), base], cwd=root, check=True)
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=tree, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=all"], cwd=tree, text=True)
    if actual != base or dirty:
        sys.exit(f"gate refused scratch worktree {tree}: expected clean {base}, found {actual}"
                 + (f"\n{dirty}" if dirty else ""))
    old_out = build(tree, scratch / "target")
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    new_out = build(root, target, tool=True)
    tool = target / "debug/host-manifest"
    failed = False
    if not compare_wit((old_out / "mod-api.wit").read_text(), (new_out / "mod-api.wit").read_text()):
        subprocess.run(["diff", str(old_out / "mod-api.wit"), str(new_out / "mod-api.wit")])
        failed = True
    for name in ("mod_api_generated.rs", "native_render_generated.rs"):
        if not filecmp.cmp(old_out / name, new_out / name, shallow=False):
            print(f"gate failed: hook IDs or dispatch differ in {name}")
            failed = True
        else:
            print(f"hook IDs and dispatch byte-identical: {name}")
    old_areas = sorted((tree / "crates/horizon-mod-api/src/host").glob("*.rs"))
    old = manifest(tool, [old_out / "mod_api_generated_host.rs", *old_areas])
    areas = sorted((root / "crates/horizon-mod-api/src/host").glob("*.rs"))
    new = manifest(tool, [new_out / "mod_api_generated_host.rs", *areas])
    if old != new:
        old_lines, new_lines = set(old.splitlines()), set(new.splitlines())
        for line in sorted(old_lines - new_lines):
            print("only in base:", line[:300])
        for line in sorted(new_lines - old_lines):
            print("only here:   ", line[:300])
        print("gate failed: the host manifest differs")
        failed = True
    if failed:
        sys.exit(1)
    print(f"gate passed against {base[:12]}: identical WIT after name-set ordering and {len(new.splitlines())} identical host methods")


if __name__ == '__main__':
    main()
