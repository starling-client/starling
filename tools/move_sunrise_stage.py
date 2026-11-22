#!/usr/bin/env python3
"""Move the stage binary and its integration tests out of the API generator. Run once from the checkout root."""
from pathlib import Path


GEN = Path("crates/sunrise-mod-api-gen")
STAGE = Path("crates/sunrise-stage")
DAILIES = Path("tools/ci/ignored-tests")


def replace(path, old, new, occurrences=1):
    path = Path(path)
    text = path.read_text()
    if text.count(old) != occurrences:
        raise ValueError(f"{path}: expected {occurrences} occurrences of {old!r}")
    path.write_text(text.replace(old, new))


def main():
    for src, dst in (
        ("src/bin/sunrise-stage.rs", "src/main.rs"),
        ("tests/scan_child.rs", "tests/scan_child.rs"),
        ("tests/surface_diff.rs", "tests/surface_diff.rs"),
    ):
        new_path = STAGE / dst
        new_path.parent.mkdir(parents=True, exist_ok=True)
        (GEN / src).rename(new_path)

    (STAGE / "Cargo.toml").write_text('''[package]
name = "sunrise-stage"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false
description = "Stage prebuilt game sources, export the SDK surface and scan mod uses"

[dependencies]
sunrise-mod-api-gen = { path = "../sunrise-mod-api-gen" }
sunrise-mod-source = { path = "../sunrise-mod-source" }
sunrise-mod-surface = { path = "../sunrise-mod-surface" }
serde_json = "1"

[dev-dependencies]
tempfile = "3.27.0"

[lints]
workspace = true
''')
    replace(GEN / "Cargo.toml", '[[bin]]\nname = "sunrise-stage"\npath = "src/bin/sunrise-stage.rs"\n\n', "")
    replace("Cargo.toml", '    "crates/sunrise-mod-api-gen",\n',
            '    "crates/sunrise-mod-api-gen",\n    "crates/sunrise-stage",\n')
    for profile in ("dev", "release"):
        row = f"[profile.{profile}.package.sunrise-mod-api-gen]\nopt-level = 3\n"
        replace("Cargo.toml", row, row + f"[profile.{profile}.package.sunrise-stage]\nopt-level = 3\n")
    replace("AGENTS.md", "| `sunrise-mod-runtime` |", "| `sunrise-stage` | Prebuilt game staging, SDK surface export and comparison, and standalone mod source scans (`surface`, `surface-diff`, `uses`, `fits`) |\n| `sunrise-mod-runtime` |")
    replace("tools/layers.json", '    "xtask": "tool",', '    "sunrise-stage": "tool",\n    "xtask": "tool",')
    replace("Cargo.lock", '[[package]]\nname = "sunrise-system"', '''[[package]]
name = "sunrise-stage"
version = "0.1.0"
dependencies = [
 "serde_json",
 "sunrise-mod-api-gen",
 "sunrise-mod-source",
 "sunrise-mod-surface",
 "tempfile",
]

[[package]]
name = "sunrise-system"''')

    moved = []
    for path in DAILIES.glob("*.txt"):
        text = path.read_text()
        kept = []
        for line in text.splitlines(keepends=True):
            if line.startswith("sunrise-mod-api-gen ") and any(
                f" test:{target} " in line for target in ("scan_child", "surface_diff")
            ):
                moved.append(line.replace("sunrise-mod-api-gen ", "sunrise-stage ", 1))
            else:
                kept.append(line)
        if "".join(kept) != text:
            if any(line.strip() and not line.startswith("#") for line in kept):
                path.write_text("".join(kept))
            else:
                path.unlink()
    (DAILIES / "daily-sunrise-stage.txt").write_text(
        "# The stage binary's integration tests run child processes.\n" + "".join(sorted(moved))
    )

    replace(".github/workflows/dyna-ci.yml", "-p sunrise-mod-api-gen --bin sunrise-stage", "-p sunrise-stage --bin sunrise-stage")
    for path, count in (("tools/modding_sdk_tables.py", 1), ("tools/prebuilt_game.py", 2)):
        # This checkout's tool is always built from the same revision as the calling script.
        replace(path, '"-p", "sunrise-mod-api-gen",', '"-p", "sunrise-stage",', count)
    for path in ("tools/ci/nightly.py", "tools/ci/surface_report.py"):
        # The binary has the same name even on revisions that build it from the generator package.
        replace(path, ' "-p", "sunrise-mod-api-gen",', "")
    for workflow, count in (("modding-book-pages", 1), ("modding-book", 2)):
        row = '"crates/sunrise-mod-api-gen/**", '
        replace(f".github/workflows/{workflow}.yml", row, row + '"crates/sunrise-stage/**", ', count)
    replace("tools/release_train.py", 'JUDGES = ("crates/sunrise-mod-api-gen",',
            'JUDGES = ("crates/sunrise-stage", "crates/sunrise-mod-api-gen",')
    doc = "docs/modding/src/reference/sdk/02-surface.md"
    replace(doc, '```sh\nSUNRISE_STAGE_TOOL=target/debug/sunrise-stage',
            '```sh\ncargo build --locked -p sunrise-stage\nSUNRISE_STAGE_TOOL=target/debug/sunrise-stage')


if __name__ == "__main__":
    main()
