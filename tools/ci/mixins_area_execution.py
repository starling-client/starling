#!/usr/bin/env python3
"""Execute synthetic area/native-hook injections; no vanilla data or game run.

Caller supplies horizon-mixins built at d9c83d7096b7ea5b83678da2efdfd6161f74997e.
All disposable sources, logs and build output remain under --repo/target.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--tool", required=True, type=Path)
    args = parser.parse_args()
    repo, tool = args.repo.resolve(strict=True), args.tool.resolve(strict=True)
    sys.path.insert(0, str(repo))
    from tools import lab

    target = repo / "target"
    target.mkdir(exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="mixins-area-execution-", dir=target))
    print(f"Area execution proof: {root}", flush=True)
    source, prepared, output = (root / name for name in ("source", "prepared", "output"))
    mods, preparer = root / "mods/area_check", root / "preparer"
    build = root / "build"

    def put(base, name, text):
        path = base / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def run(name, command, success=True):
        homes = {}
        for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                    "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
            home = root / "homes" / name / key
            home.mkdir(parents=True)
            homes[key] = home
        env = lab.game_env(os.environ, **homes, CARGO_TARGET_DIR=build)
        # Any relative saves/options also stay in disposable scratch, never the checkout.
        result = subprocess.run([str(arg) for arg in command], cwd=root, env=env,
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        log = root / f"{name}.log"
        log.write_text(result.stdout, encoding="utf-8")
        if success and result.returncode != 0:
            raise RuntimeError(f"{name} failed ({result.returncode}); see {log}\n{result.stdout[-5000:]}")
        return result

    put(source, "Cargo.toml", '[workspace]\nmembers=["crates/horizon-client"]\nresolver="3"\n')
    put(source, ".horizon-revision", "0" * 40)
    put(source, "Cargo.lock", 'version = 4\n\n[[package]]\nname = "horizon-client"\nversion = "0.1.0"\n')
    client = source / "crates/horizon-client"
    # Keep the library's target identity, but do not name this synthetic executable after the game.
    put(client, "Cargo.toml", '[package]\nname="horizon-client"\nversion="0.1.0"\nedition="2024"\n[[bin]]\nname="area-execution-proof"\npath="src/main.rs"\n')
    put(client, "src/lib.rs", "pub mod game;\n")
    put(client, "src/main.rs", '''fn main() {
    assert_eq!(horizon_client::game::run(), 211);
    let keys = ["HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME",
                "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP",
                "HORIZON_NO_AUDIO", "HORIZON_LAUNCHER_NO_SELF_UPDATE", "HORIZON_TEST_LOOPBACK"];
    let observed = keys.map(|key| format!("{key}={}", std::env::var(key).unwrap())).join("\\n");
    std::fs::write("execution-env.txt", observed).unwrap();
    println!("AREA_AND_NATIVE_HOOK_HELPERS=211");
}
''')
    put(client, "src/game.rs", '''mod native_hooks;
include!(concat!(env!("OUT_DIR"), "/game_modules.rs"));
struct Game<'a> { value: u32, _marker: &'a () }
pub fn run() -> u32 {
    let marker = ();
    let mut game = Game { value: 0, _marker: &marker };
    assert!(game.start_eating());
    assert!(game.native_right_click_block());
    game.value
}
''')
    put(client, "src/game/items/.game-area", "")
    put(client, "src/game/items/using_item.rs", "use super::*; impl Game<'_> { pub(super) fn start_eating(&mut self) -> bool { self.value += 1; true } }\n")
    put(client, "src/game/native_hooks/mod.rs", 'include!(concat!(env!("OUT_DIR"), "/native_hook_modules.rs"));\n')
    put(client, "src/game/native_hooks/base.rs", "use super::super::*; impl Game<'_> { pub(in crate::game) fn native_right_click_block(&mut self) -> bool { self.value += 10; true } }\n")
    manifest = '''[mod]
id="area_check"
version="0.1.0"
[compatibility]
horizon_revision="0000000000000000000000000000000000000000"
'''
    for ident, path, method, effect in [
        ("area", "using_item", "start_eating", 97),
        ("native", "native_hooks::base", "native_right_click_block", 103),
    ]:
        manifest += f'''
[[injection]]
id="{ident}"
target="horizon_client::game::{path}::Game<'_>::{method}"
kind="HEAD"
helper="src/{ident}.rs"
bindings=[{{name="game",ty="&mut crate::game::Game<'_>",value="&mut *self"}}]
'''
        put(mods, f"src/{ident}.rs", f"fn count(game: &mut crate::game::Game<'_>) {{ game.value += {effect}; }}\n")
    put(mods, "horizon-mixin.toml", manifest)

    def source_files():
        return {path.relative_to(source): path.read_bytes()
                for path in source.rglob("*") if path.is_file()}

    original = source_files()
    raw = run("raw", [tool, "apply", "--workspace", source, "--mods", mods.parent,
                      "--out", root / "raw-output"], success=False)
    if raw.returncode == 0 or "E_TARGET_NOT_FOUND" not in raw.stdout:
        raise RuntimeError(f"Expected raw E_TARGET_NOT_FOUND; see {root / 'raw.log'}")
    generator = json.dumps(str(repo / "crates/horizon-mod-api-gen"))
    put(preparer, "Cargo.toml", f'''[package]
name="area-module-preparer"
version="0.1.0"
edition="2024"
[workspace]
[dependencies]
horizon-mod-api-gen={{path={generator}}}
''')
    put(preparer, "src/main.rs", 'fn main() { horizon_mod_api_gen::materialize_game_modules(std::path::Path::new(&std::env::args().nth(1).unwrap())).unwrap(); }\n')
    shutil.copytree(source, prepared)
    run("prepare", ["cargo", "run", "--manifest-path", preparer / "Cargo.toml", "--", prepared])
    run("apply", [tool, "apply", "--workspace", prepared, "--mods", mods.parent, "--out", output])
    # Absolute includes back into prepared must not make this relocated build pass.
    shutil.rmtree(prepared)
    run("build", ["cargo", "build", "--manifest-path", output / "Cargo.toml", "-p", "horizon-game"])
    executable = "area-execution-proof.exe" if os.name == "nt" else "area-execution-proof"
    executed = run("execute", [build / "debug" / executable])
    if executed.stdout.strip() != "AREA_AND_NATIVE_HOOK_HELPERS=211":
        raise RuntimeError(f"Missing execution proof; see {root / 'execute.log'}")
    if source_files() != original:
        raise RuntimeError("Original download source file map was modified")
    print("Raw rejected; relocated output compiled and executed 1 + 10 + 97 + 103 = 211.")


if __name__ == "__main__":
    main()
