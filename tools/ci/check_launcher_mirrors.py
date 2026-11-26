#!/usr/bin/env python3
"""Keep the launcher's copies of game crates honest.

The launcher (sunrise-org/sunrise-launcher) does not depend on this repository by path: it carries copies of
fifteen crates, each marked "Copied from the game's crate so the launcher can leave the game repository". Nothing
checked that a copy still matched its source. `tools/launcher_mirrors.toml` names every mirror, its source paths
and the hash of the source's public surface; this check recomputes the hash and fails when it moved.

A surface change is allowed, but it must be announced: the pull request body carries a line starting with
`launcher-mirror:` saying what the launcher copy must follow, and the manifest is regenerated with `--write` in the
same commit. With `--base`, the check compares the base manifest too, so a stale manifest cannot hide a change.

    python3 tools/ci/check_launcher_mirrors.py                    # hashes match the manifest
    python3 tools/ci/check_launcher_mirrors.py --base origin/main --event "$GITHUB_EVENT_PATH"  # CI
    python3 tools/ci/check_launcher_mirrors.py --write            # after an announced surface change
    python3 tools/ci/check_launcher_mirrors.py --list             # every mirror and its source paths
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tools" / "launcher_mirrors.toml"
MANIFEST_PATH = "tools/launcher_mirrors.toml"
MARKER = "launcher-mirror:"
PUBLIC = {"struct", "enum", "trait", "fn", "type", "const", "static", "mod", "use", "union", "macro_rules"}
KEEP_ATTRIBUTES = ("cfg", "non_exhaustive", "doc(hidden)")


def load_manifest(path=MANIFEST):
    with open(path, "rb") as f:
        return tomllib.load(f)


def _attributes(values, i):
    """The attribute blocks at `i`, as one normalized string."""
    out = []
    while i + 1 < len(values) and values[i] == "#" and values[i + 1] == "[":
        i += 1
        depth, start = 0, i
        while i < len(values):
            depth += values[i] == "["
            depth -= values[i] == "]"
            i += 1
            if depth == 0:
                break
        text = " ".join(values[start:i])
        if any(part in text for part in KEEP_ATTRIBUTES):
            out.append(text)
    return out, i


def _declaration(values, i):
    """The tokens of one public declaration from `pub` at `i`, and the index after it."""
    i += 1
    if i < len(values) and values[i] == "(":
        # `pub(crate)`, `pub(super)`: not a public surface.
        return None, i
    start = i
    while i < len(values) and values[i] in ("async", "unsafe", "const", "extern", "default"):
        i += 1
    keyword = values[i] if i < len(values) else ""
    terminators = (";",) if keyword == "use" else ("{", ";", "=", ",")
    header, depth = [], 0
    while i < len(values):
        value = values[i]
        if depth == 0 and value in terminators:
            if value == "=" and keyword in ("const", "static", "type"):
                # The initializer is implementation; the name and type are the surface.
                header.append("=")
                i += 1
                while i < len(values) and not (depth == 0 and values[i] == ";"):
                    depth = max(0, depth + (values[i] in ("(", "[", "{")) - (values[i] in (")", "]", "}")))
                    i += 1
                return " ".join(values[start:i + 1]), i + 1
            header.append(value)
            i += 1
            break
        depth = max(0, depth + (value in ("(", "[", "{")) - (value in (")", "]", "}")))
        header.append(value)
        i += 1
    if keyword in ("enum", "trait") and header and header[-1] == "{":
        depth, body = 1, []
        while i < len(values) and depth:
            depth += values[i] == "{"
            depth -= values[i] == "}"
            body.append(values[i])
            i += 1
        header += body
    return " ".join(header), i


def _load_code_tokens():
    """`tools/rust_tokens.code_tokens`, by path: importing this module must not edit `sys.path`."""
    try:
        from tools.rust_tokens import code_tokens
    except ImportError:
        spec = importlib.util.spec_from_file_location("rust_tokens", ROOT / "tools" / "rust_tokens.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        code_tokens = module.code_tokens
    return code_tokens


code_tokens = _load_code_tokens()


def public_surface(source):
    """Public declarations of one Rust source file, normalized token by token."""
    values = [value for value, _ in code_tokens(source)]
    surface, i, pending = [], 0, []
    while i < len(values):
        if values[i] == "#" and i + 1 < len(values) and values[i + 1] == "[":
            attributes, i = _attributes(values, i)
            pending += attributes
            continue
        if values[i] == "pub":
            declaration, i = _declaration(values, i)
            if declaration is not None:
                surface.append(" ".join(pending + [declaration]))
            pending = []
            continue
        pending = []
        i += 1
    return surface


def source_files(sources):
    """Every Rust file under the mirror's sources, in a stable order."""
    files = []
    for source in sources:
        path = ROOT / source
        if not path.exists():
            raise SystemExit(f"Launcher mirror source is missing: {source}")
        files += [path] if path.is_file() else sorted(path.rglob("*.rs"))
    return files


def surface(sources):
    digest = hashlib.sha256()
    for path in source_files(sources):
        relative = path.relative_to(ROOT).as_posix()
        digest.update(relative.encode() + b"\0")
        for declaration in public_surface(path.read_text(encoding="utf-8", errors="replace")):
            digest.update(declaration.encode() + b"\n")
    return "sha256:" + digest.hexdigest()


def measured(manifest):
    """{launcher crate: hash}, plus a note for a mirror whose source is not in this repository."""
    hashes, notes = {}, []
    for mirror in manifest["mirror"]:
        if not mirror.get("sources"):
            notes.append(f"{mirror['launcher']}: no source in this repository; skipped")
            continue
        hashes[mirror["launcher"]] = surface(mirror["sources"])
    return hashes, notes


def pr_body(args):
    if args.pr_body_file:
        return Path(args.pr_body_file).read_text(encoding="utf-8")
    if args.event:
        event = json.loads(Path(args.event).read_text(encoding="utf-8"))
        return (event.get("pull_request") or {}).get("body") or ""
    return ""


def base_manifest(base):
    out = subprocess.run(["git", "show", f"{base}:{MANIFEST_PATH}"], capture_output=True, text=True, cwd=ROOT)
    return tomllib.loads(out.stdout) if out.returncode == 0 else None


def announced(body):
    return any(line.strip().startswith(MARKER) for line in body.splitlines())


def check(manifest, hashes, base=None, body=""):
    problems = []
    recorded = {mirror["launcher"]: mirror for mirror in manifest["mirror"]}
    for name, digest in sorted(hashes.items()):
        recorded_digest = recorded[name].get("surface")
        if recorded_digest != digest:
            problems.append(f"{name}: public surface is {digest}, {MANIFEST_PATH} records {recorded_digest or 'nothing'}. "
                            f"Run --write, and say {MARKER} what the launcher copy must follow")
    if base is not None:
        before = {mirror["launcher"]: mirror.get("surface") for mirror in base["mirror"]}
        changed = sorted(name for name, digest in hashes.items() if before.get(name) != digest)
        if changed and not announced(body):
            problems.append(f"launcher mirrors changed ({', '.join(changed)}) without a line starting "
                            f"{MARKER} in the pull request body")
    return problems


def _toml_string(value):
    return json.dumps(value, ensure_ascii=False)


def write(manifest, hashes):
    rows = []
    for mirror in manifest["mirror"]:
        lines = ["[[mirror]]", f"launcher = {_toml_string(mirror['launcher'])}"]
        lines.append("sources = [" + ", ".join(_toml_string(s) for s in mirror.get("sources", [])) + "]")
        if mirror.get("note"):
            lines.append(f"note = {_toml_string(mirror['note'])}")
        if mirror["launcher"] in hashes:
            lines.append(f"surface = {_toml_string(hashes[mirror['launcher']])}")
        rows.append("\n".join(lines))
    MANIFEST.write_text(
        '# docs/layers.md: the launcher copies these crates; check_launcher_mirrors.py hashes their public surface.\n'
        '# Run --write after an announced `launcher-mirror:` change, never to hide an unannounced one.\n'
        'version = 1\n\n' + "\n\n".join(rows) + "\n",
        encoding="utf-8",
    )
    print(f"Recorded {len(hashes)} launcher mirror surfaces in {MANIFEST_PATH}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="also compare the manifest at this git ref")
    parser.add_argument("--event", help="the pull request event JSON (GitHub Actions GITHUB_EVENT_PATH)")
    parser.add_argument("--pr-body-file", help="a file holding the pull request body, for a local check")
    parser.add_argument("--write", action="store_true", help="record the current public surface")
    parser.add_argument("--list", action="store_true", help="print every mirror and its sources")
    args = parser.parse_args(argv)
    manifest = load_manifest()
    if args.list:
        for mirror in manifest["mirror"]:
            print(f"{mirror['launcher']}: {', '.join(mirror.get('sources') or ['(no source in this repository)'])}")
        return 0
    hashes, notes = measured(manifest)
    for note in notes:
        print(f"note: {note}")
    if args.write:
        write(manifest, hashes)
        return 0
    problems = check(manifest, hashes, base_manifest(args.base) if args.base else None, pr_body(args))
    if problems:
        print("Launcher mirror check failed:")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(f"Launcher mirror check passed: {len(hashes)} mirrored crates, {MANIFEST_PATH}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
