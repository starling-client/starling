#!/usr/bin/env python3
"""Run the real report writer, exporter, timings and tombstone parser on the host.

Only platform UI/version metadata are replaced. FrameStats, logging and timing
implementations are compiled directly from their production files.
"""
from pathlib import Path
import os
import subprocess
import tempfile
import tomllib
import lab

ROOT = Path(__file__).resolve().parents[1]

def main():
    harness = ROOT / ".local/diagnostics-tests"
    (harness / "src").mkdir(parents=True, exist_ok=True)
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))
    zip_version = next(p["version"] for p in lock["package"] if p["name"] == "zip" and p["version"].startswith("9."))
    threads = (ROOT / "crates/sunrise-threads").as_posix()
    (harness / "Cargo.toml").write_text(f'''[package]
name = "starling-diagnostics-tests"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
serde_json = "1"
log = "0.4"
regex = "1"
chrono = "0.4"
flate2 = "1"
rustix = {{ version = "1", features = ["time"] }}
zip = {{ version = "={zip_version}", default-features = false, features = ["deflate"] }}
sunrise-threads = {{ path = "{threads}" }}
''', encoding="utf-8")
    # This data-only type connects FrameStats to the unrelated renderer. Read its
    # exact definition rather than compiling the entire graphics/game workspace.
    source = (ROOT / "crates/sunrise-client-render/src/block_entity_render.rs").read_text(encoding="utf-8")
    start = source.index("pub struct StaticCacheFrameStats {")
    end = source.index("\n}", start) + 2
    struct = "#[derive(Clone, Copy, Debug, Default)]\n" + source[start:end]
    modules = {
        "control": "crates/sunrise-client/src/control.rs",
        "logging": "crates/sunrise-client-render/src/logging.rs",
        "spans": "crates/sunrise-client-render/src/spans.rs",
        "diagnostics": "crates/sunrise-client/src/diagnostics.rs",
        "performance": "crates/sunrise-client/src/performance.rs",
        "preparer_install": "crates/sunrise-client/src/preparer_install.rs",
    }
    code = '''#![allow(dead_code)]
extern crate self as sunrise_client;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
mod crash { pub fn version() -> &'static str { "diagnostics-test" } }
mod platform { pub fn open_folder(_: &std::path::Path) -> Option<bool> { Some(false) } }
'''
    code += "mod block_entity_render {\n" + struct + "\n}\n"
    for name, path in modules.items():
        code += f'#[path = "{(ROOT / path).as_posix()}"] pub mod {name};\n'
    (harness / "src/lib.rs").write_text(code, encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="starling-diagnostics-") as scratch:
        env = lab.game_env(os.environ, **{
            name: str(Path(scratch) / name.lower())
            for name in ("SUNRISE_HOME", "SUNRISE_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME")
        })
        subprocess.run(["cargo", "test", "--manifest-path", str(harness / "Cargo.toml"),
                        "--", "--test-threads=1"], cwd=scratch, env=env, check=True)

if __name__ == "__main__":
    main()
