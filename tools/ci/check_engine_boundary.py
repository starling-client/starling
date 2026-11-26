#!/usr/bin/env python3
"""Ratchet the engine modules server-side gameplay may not reach.

The game's engine layer owns rendering, audio and the window (`gfx`, `audio`, `window`). Server-side gameplay lives
in the same crate or in `sunrise-gameplay`, and code there must not call those modules: a server build has no
window, no GPU and no audio device, and the layer model says gameplay reaches the platform's public API, not the
engine (docs/layers.md). The plan's audit counted the references already there; this check freezes that list and
refuses every new one, so the count only falls.

`tools/ci/engine_boundary.json` holds the scan roots, the engine modules and the exact files that still reference
them (with the modules each file touches). An entry that disappears is a note: delete it in the same pull request
that removes the last reference. Adding a file or a module is a failure.

The crate graph comes from `tools/check_layers.py` (`load_config`), not a second copy of the layer model.

    python3 tools/ci/check_engine_boundary.py                # the ratchet
    python3 tools/ci/check_engine_boundary.py --base origin/main  # also refuse new entries vs the base
    python3 tools/ci/check_engine_boundary.py --write        # record today's offenders (first baseline only)
    python3 tools/ci/check_engine_boundary.py --list         # every offender and the modules it takes
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
CONFIG = ROOT / "tools" / "ci" / "engine_boundary.json"
CONFIG_PATH = "tools/ci/engine_boundary.json"
DOC = "docs/layers.md"


def load_config(path=CONFIG):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def _module_roots(tokens, start):
    """The immediate path segments of `crate::...` / `sunrise_client::...`, or an empty set."""
    i = start
    if i >= len(tokens):
        return set()
    if tokens[i] != "{":
        return {tokens[i]}
    roots, i = set(), i + 1
    while i < len(tokens) and tokens[i] != "}":
        if tokens[i] in (",",):
            i += 1
            continue
        if tokens[i] == "{":
            nested, i = _module_roots(tokens, i)
            roots |= nested
            continue
        roots.add(tokens[i])
        # Skip the rest of this entry up to the next comma or the closing brace.
        depth = 0
        while i < len(tokens) and not (depth == 0 and tokens[i] in (",", "}")):
            depth += tokens[i] == "{"
            depth -= tokens[i] == "}"
            i += 1
    return roots


# The crate roots an engine reference may start from: the current crate, the client's public crate, and the
# split client crates a gameplay file reaches by path (`use sunrise_client_lib::audio::Audio`).
CRATE_HEADS = (
    "crate",
    "sunrise_client",
    "sunrise_client_lib",
    "sunrise_client_render",
    "sunrise_client_input",
)


def referenced_modules(source, modules):
    """Engine modules this source imports or namespaces through a crate head in `CRATE_HEADS`."""
    try:
        from tools.rust_tokens import tokens
    except ImportError:
        from rust_tokens import tokens

    values = [value for value, _ in tokens(source)]
    found = set()
    for i, value in enumerate(values[:-3]):
        if value in CRATE_HEADS and values[i + 1:i + 3] == [":", ":"]:
            roots = _module_roots(values, i + 3)
            found |= roots & set(modules)
    return found


def offenders(config):
    modules = set(config["modules"])
    found = {}
    for root in config["scan"]:
        base = ROOT / root
        if not base.exists():
            raise SystemExit(f"Engine boundary scan root is missing: {root}")
        files = [base] if base.is_file() else sorted(base.rglob("*.rs"))
        for path in files:
            if path.suffix != ".rs":
                continue
            hit = referenced_modules(path.read_text(encoding="utf-8", errors="replace"), modules)
            if hit:
                found[path.relative_to(ROOT).as_posix()] = sorted(hit)
    return found


def base_config(base):
    """The baseline as `base` has it, or None when the ref has no file (a first commit)."""
    out = subprocess.run(["git", "show", f"{base}:{CONFIG_PATH}"], capture_output=True, text=True, cwd=ROOT)
    if out.returncode != 0:
        return None
    return json.loads(out.stdout)


def check(config, current, base=None):
    """(problems, notes); `current` is offenders(config)."""
    allowed = {path: list(modules) for path, modules in config.get("allowed", {}).items()}
    problems, notes = [], []
    for path in sorted(set(current) - set(allowed)):
        problems.append(f"{path}: references {', '.join(current[path])}. Server-side gameplay may not reach the "
                        f"engine's {', '.join(config['modules'])} modules; move the call behind the platform API, "
                        f"or keep it in client-only code ({DOC})")
    for path in sorted(set(current) & set(allowed)):
        added = sorted(set(current[path]) - set(allowed[path]))
        if added:
            problems.append(f"{path}: now also references {', '.join(added)}. The ratchet only shrinks "
                            f"({CONFIG_PATH})")
        removed = sorted(set(allowed[path]) - set(current[path]))
        if removed:
            notes.append(f"{path}: no longer references {', '.join(removed)}; narrow its entry in {CONFIG_PATH}")
    for path in sorted(set(allowed) - set(current)):
        notes.append(f"{path}: no longer references an engine module; remove it from {CONFIG_PATH}")
    if base is not None:
        base_allowed = {path: list(modules) for path, modules in base.get("allowed", {}).items()}
        for path in sorted(set(allowed) - set(base_allowed)):
            problems.append(f"{path}: added to {CONFIG_PATH}. The exceptions only shrink")
        for path in sorted(set(allowed) & set(base_allowed)):
            if set(allowed[path]) - set(base_allowed[path]):
                problems.append(f"{path}: widened in {CONFIG_PATH}. The exceptions only shrink")
        if config.get("scan") != base.get("scan") or config.get("modules") != base.get("modules"):
            problems.append(f"{CONFIG_PATH}: scan roots and modules are fixed; change them in their own pull request")
    return problems, notes


def write(config, current):
    config = dict(config, allowed={path: modules for path, modules in sorted(current.items())})
    CONFIG.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    print(f"Recorded {len(current)} offenders in {CONFIG_PATH}:")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="refuse entries added since this git ref")
    parser.add_argument("--write", action="store_true", help="record today's offenders (first baseline only)")
    parser.add_argument("--list", action="store_true", help="print every offender and its modules")
    args = parser.parse_args(argv)

    sys.path.insert(0, str(ROOT / "tools"))
    import check_layers

    layers = check_layers.load_config()
    for crate in ("sunrise-client", "sunrise-gameplay"):
        if layers["crates"].get(crate) != "engine":
            print(f"Engine boundary check failed: {crate} is not an engine crate in tools/layers.json")
            return 1

    config = load_config()
    current = offenders(config)
    if args.list:
        for path, modules in sorted(current.items()):
            print(f"{path}: {', '.join(modules)}")
        print(f"\n{len(current)} files still reach the engine from server-side gameplay")
        return 0
    if args.write:
        write(config, current)
        return 0
    problems, notes = check(config, current, base_config(args.base) if args.base else None)
    for note in notes:
        print(f"note: {note}")
    if problems:
        print("Engine boundary check failed (docs/layers.md):")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(f"Engine boundary check passed: {len(current)} files still reach the engine "
          f"({len(config.get('allowed', {}))} recorded, {DOC}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
