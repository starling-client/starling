#!/usr/bin/env python3
"""Writes crates/sunrise-parity-fixtures/Cargo.toml from crates/sunrise-client/Cargo.toml (#4896).

The `sunrise-parity-fixtures` program compiles sunrise-client's sources with `test` set, so the test-only code its
fixtures read compiles as in the lib test, without the lib's tests. That code takes sunrise-client's
[dev-dependencies], which a shipped crate's normal dependencies may not include (docs/layers.md): this package, a
dev tool, takes them instead. It has sunrise-client's dependencies and features, every dependency optional and
switched on by `fixtures`, so a workspace build that doesn't ask for `fixtures` builds and unifies nothing from it.
`client-defaults` is sunrise-client's default features.

Usage: parity_fixtures_manifest.py [--check]   (--check: fail when the manifest isn't what this writes)
"""
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLIENT = ROOT / "crates/sunrise-client/Cargo.toml"
MANIFEST = ROOT / "crates/sunrise-parity-fixtures/Cargo.toml"
# build.rs reads sunrise-client's modules to list the fixtures.
TABLE_BUILD_DEPENDENCIES = {
    "sunrise-mod-source": {"path": "../sunrise-mod-source"},
    "syn": {"version": "2", "features": ["full"]},
}


def value(item):
    if isinstance(item, bool):
        return "true" if item else "false"
    if isinstance(item, str):
        return '"' + item.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if isinstance(item, list):
        return "[" + ", ".join(value(entry) for entry in item) + "]"
    if isinstance(item, dict):
        return "{ " + ", ".join(f"{key} = {value(entry)}" for key, entry in item.items()) + " }"
    raise TypeError(f"no TOML form for {item!r}")


def table(rows):
    return "".join(f"{name if name.replace('-', '').replace('_', '').isalnum() else value(name)} = {value(entry)}\n"
                   for name, entry in rows.items())


def spec(entry):
    return dict(entry) if isinstance(entry, dict) else {"version": entry}


def merged(normal, dev):
    """One table of both: a dependency in both takes the union of their features."""
    out = {name: spec(entry) for name, entry in normal.items()}
    for name, entry in dev.items():
        entry = spec(entry)
        if name in out:
            features = out[name].get("features", []) + [f for f in entry.get("features", [])
                                                        if f not in out[name].get("features", [])]
            if features:
                out[name]["features"] = features
        else:
            out[name] = entry
    return out


def render():
    client = tomllib.loads(CLIENT.read_text(encoding="utf-8"))
    package = client["package"]
    enabled = []
    sections = []

    def optional(rows):
        for name, entry in rows.items():
            if not entry.get("optional") and f"dep:{name}" not in enabled:
                enabled.append(f"dep:{name}")
            entry["optional"] = True
        return rows

    sections.append(("dependencies", optional(merged(client.get("dependencies", {}),
                                                     client.get("dev-dependencies", {})))))
    for target, tables in client.get("target", {}).items():
        rows = merged(tables.get("dependencies", {}), tables.get("dev-dependencies", {}))
        if rows:
            sections.append((f"target.{value(target)}.dependencies", optional(rows)))
    features = {name: entry for name, entry in client.get("features", {}).items() if name != "default"}
    features["client-defaults"] = client.get("features", {}).get("default", [])
    features["fixtures"] = enabled
    build = {name: spec(entry) for name, entry in client.get("build-dependencies", {}).items()}
    build.update(TABLE_BUILD_DEPENDENCIES)
    text = (
        "# Written by tools/ci/parity_fixtures_manifest.py from crates/sunrise-client/Cargo.toml: rerun it after a\n"
        "# change there, never edit this file. verify-static checks it is current.\n"
        "[package]\n"
        'name = "sunrise-parity-fixtures"\n'
        f"version = {value(package['version'])}\n"
        "edition.workspace = true\n"
        "license.workspace = true\n"
        "rust-version.workspace = true\n"
        'description = "The parity harnesses\' sunrise-client fixtures, without the lib test (CI only)"\n'
        "publish = false\n"
        "autolib = false\n"
        "autobins = false\n"
        "autoexamples = false\n"
        "autotests = false\n"
        "autobenches = false\n"
        "\n"
        "# sunrise-client's own lib.rs is the program's root: its `main` is cfg(sunrise_parity_fixtures).\n"
        "[[bin]]\n"
        'name = "sunrise-parity-fixtures"\n'
        'path = "../sunrise-client/src/lib.rs"\n'
        'required-features = ["fixtures"]\n'
        "test = false\n"
        "bench = false\n"
        "doc = false\n"
        "\n"
        "[lints]\n"
        "workspace = true\n"
        "\n"
        "[build-dependencies]\n" + table(build)
    )
    for name, rows in sections:
        text += f"\n[{name}]\n" + table(rows)
    text += "\n[features]\n" + table(features)
    return text


def main(argv):
    text = render()
    if argv == ["--check"]:
        if MANIFEST.read_text(encoding="utf-8") != text:
            print(f"{MANIFEST.relative_to(ROOT)} is out of date: run python3 tools/ci/parity_fixtures_manifest.py")
            return 1
        return 0
    if argv:
        print(__doc__)
        return 2
    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    MANIFEST.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
