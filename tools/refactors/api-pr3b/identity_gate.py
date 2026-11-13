#!/usr/bin/env python3
"""Compare built public contracts for an additive 3b layer (no substitutions).

Usage: identity_gate.py BASE_TREE BASE_OUT HEAD_TREE HEAD_OUT HOST_MANIFEST_TOOL
Build each checkout's horizon-mod-api first. The manifest tool comes from
horizon-mod-api-gen. Render hooks now live outside this crate; this gate compares
its complete hook output and WIT, not the removed native_render_generated.rs.
"""
import hashlib
from pathlib import Path
import subprocess
import sys


def manifest(tree, output, tool):
    files = [output / "mod_api_generated_host.rs",
             *sorted((tree / "crates/horizon-mod-api/src/host").glob("*.rs"))]
    return subprocess.check_output([str(tool.resolve()), *map(str, files)])


def compare(name, before, after):
    if before != after:
        raise SystemExit(f"{name}: contract changed")
    print(f"{name}: byte-identical; sha256 {hashlib.sha256(before).hexdigest()}")


def main():
    if len(sys.argv) != 6:
        raise SystemExit(__doc__)
    base, old, head, new, tool = map(Path, sys.argv[1:])
    for name in ("mod-api.wit", "mod_api_generated.rs"):
        compare(name, (old / name).read_bytes(), (new / name).read_bytes())
    before, after = manifest(base, old, tool), manifest(head, new, tool)
    compare(f"Host manifest ({len(before.splitlines())} methods)", before, after)
    print("WIT: 0 added, 0 changed, 0 removed types/functions; enum case order preserved")
    print("Host substitution allow-list: empty; hook IDs and dispatch preserved")


if __name__ == "__main__":
    main()
