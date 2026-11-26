#!/usr/bin/env python3
"""Exercise TNT's transient transfer state through the actual isolated Game loaders.

The contracts follow pinned PrimedTnt.teleport and its save/load methods. This
runs no Java server, world generation or cross-dimensional travel transcript.
"""
import os
from pathlib import Path
import subprocess
import tempfile
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab, test_rust_parity as base


def child_env(scratch, resources=None):
    env = dict(os.environ)
    env.pop("SUNRISE_TEST_RESOURCES", None)
    for key, child in [("SUNRISE_HOME", "home"), ("SUNRISE_LAUNCHER_HOME", "launcher"),
                       ("XDG_DATA_HOME", "xdg-data"), ("XDG_CONFIG_HOME", "xdg-config"),
                       ("SUNRISE_LAUNCHER_TRASH", "trash"),
                       ("SUNRISE_LAUNCHER_CLIPBOARD", "clipboard"),
                       ("SUNRISE_LAUNCHER_DESKTOP", "desktop")]:
        env[key] = str(scratch / child)
    env = lab.game_env(env)
    if resources is not None:
        env["SUNRISE_TEST_RESOURCES"] = str(resources)
    return env


def main():
    resources = Path(os.environ.get("SUNRISE_TEST_RESOURCES", lab.RESOURCES)).resolve()
    if not (resources / "movement-properties.nbt").exists():
        raise RuntimeError("Set SUNRISE_TEST_RESOURCES to prepared pinned resources")
    build = lab.LOCAL / "tnt-portal-state-parity"
    build.mkdir(parents=True, exist_ok=True)
    report = build / "report.json"
    report.unlink(missing_ok=True)
    sources = [Path(__file__), lab.ROOT / "research-lock.json", lab.ROOT / "Cargo.toml",
               lab.ROOT / "Cargo.lock", lab.ROOT / "tools/lab.py",
               *sorted((lab.ROOT / "crates/sunrise-client/src").rglob("*.rs")),
               *sorted((lab.ROOT / "crates/sunrise-client-render/src").rglob("*.rs"))]
    hashes = base.source_hashes(sources)
    binary = lab.parity_fixtures()
    binary_hash = lab.digest(binary, "sha256")
    with tempfile.TemporaryDirectory(prefix="sunrise-tnt-state-game-") as directory:
        scratch = Path(directory)
        with (build / "rust.log").open("w", encoding="utf-8") as log:
            subprocess.run([str(binary), "portal_transfer_", "--include-ignored", "--color", "never"],
                           cwd=scratch, env=child_env(scratch, resources), check=True,
                           stdout=log, stderr=subprocess.STDOUT)
    output = (build / "rust.log").read_text(encoding="utf-8")
    if "running 3 tests" not in output or "3 passed; 0 failed" not in output:
        raise AssertionError("All three transfer-state regressions must execute")
    if binary_hash != lab.digest(binary, "sha256"):
        raise RuntimeError("Test executable changed during validation")
    base.ensure_sources_unchanged(sources, hashes)
    lab.write_json(report, {"minecraft": lab.VERSION, "sources_sha256": hashes,
                           "rust_binary_sha256": binary_hash,
                           "rust_log_sha256": lab.digest(build / "rust.log", "sha256"),
                           "rust_tests": 3, "scope": __doc__})
    print("TNT transfer state: three actual Game contracts passed")


if __name__ == "__main__":
    main()
