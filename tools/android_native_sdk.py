#!/usr/bin/env python3
"""Native mods on Android, as libraries of their own (branch android-native-sdk, the android-sdk workflow).

    python3 tools/android_native_sdk.py prepare               # the game and the mod API as Rust dylibs
    RUSTFLAGS="-C prefer-dynamic" python3 packaging/android/build_apk.py --abi arm64-v8a ...
    RUSTFLAGS="-C prefer-dynamic" python3 tools/android_native_sdk.py mods OUT MOD.smod... [--profile P] [--abi A]

`prepare` rewrites this checkout's manifests (a CI runner's or a build copy's, never a working tree someone edits)
the way tools/prebuilt_game.py rewrites its staged copy: the API crates (sunrise-mod-api's and sunrise-mod-points'
closure), sunrise-mod-host and sunrise-mod-wasm become `rlib` and `dylib`. Built with `-C prefer-dynamic`, the APK
then carries them as libraries of their own beside the entry library (build_apk.py `rust_dylibs`); the std the APK
builds from source (build_apk.py TLS_FLAGS) sits inside the lowest of them, once for the game and every mod.

`mods` builds each native `.smod` (the source package Sunrise's catalog publishes: sunrise-mod.toml with a
`[native]` table, Cargo.toml, src/) as a library linked against those, in the same Cargo build as the game and with
the same flags and features (build_apk.py `build_library` with more packages), so the game's crates are the ones the
APK ships and only the mods compile. OUT gets what the game reads in its `native-libs` folder (android_lib.rs):
`native-mods.json`, each `lib<id>.so`, and each mod's package in `sunrise-native-mods/<id>/`.

Mods with `[native.*]` tables (mixins, modules, bridges, loops) need this checkout to be the staged game
(tools/prebuilt_game.py `stage`, whose `functions.json` it holds) and the commit's unstaged source in
`SUNRISE_UNSTAGED_SOURCE` (the stage's `stage-in`). For such a mod, sunrise-android-patch (crates/sunrise-native-build)
copies the functions its rows target out of that source and applies them with horizon-mixins, into a patch crate;
the mod's own crate becomes an rlib the patch crate's helpers call, and a third crate, the library, holds the entry,
whose `installs()` puts each copy in its function's slot when the game opens the library. One such mod a build.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT / "packaging" / "android"))
import prebuilt_game  # noqa: E402  (api_closure: the crates an API-only mod's build reads)
import build_apk  # noqa: E402  (build_library: the APK's own Cargo build)

# Beside the game's crates, in the workspace (`members` gets this folder's glob): the mods as packages.
MODS_DIR = "native-mods-build"
# What becomes a dylib besides the API closure: the game (the mod host above the engine) and the wasm runtime.
GAME_LIBRARIES = ("sunrise-mod-host", "sunrise-mod-wasm")
# The entry module each mod library carries: its hooks, build records and build identity, which the game's
# loader reads (crates/sunrise-mod-host/src/native_libraries.rs).
TEMPLATE = ROOT / "crates/sunrise-client/src/native_mod_entry.rs.template"
SIDES = {"both": "Both", "client": "Client", "server": "Server"}
ID = re.compile(r"[a-z0-9][a-z0-9_]{0,63}")


def run(command, **kwargs) -> str:
    return subprocess.run(command, check=True, capture_output=True, text=True, cwd=ROOT, **kwargs).stdout


def prepare() -> None:
    metadata = json.loads(run(["cargo", "metadata", "--format-version", "1"]))
    closure = prebuilt_game.api_closure(metadata)
    changed = []
    for package in metadata["packages"]:
        if package["id"] not in closure and package["name"] not in GAME_LIBRARIES:
            continue
        if not any(target["kind"] == ["lib"] for target in package["targets"]):
            continue
        manifest = Path(package["manifest_path"])
        if not manifest.is_relative_to(ROOT):
            continue  # Registry dependencies are Cargo's, not rewritten.
        text = manifest.read_text()
        if "crate-type" in text:
            sys.exit(f"{manifest.relative_to(ROOT)} names its crate types already")
        if "\n[lib]\n" in text:
            text = text.replace("\n[lib]\n", '\n[lib]\ncrate-type = ["rlib", "dylib"]\n', 1)
        else:
            text += '\n[lib]\ncrate-type = ["rlib", "dylib"]\n'
        manifest.write_text(text)
        changed.append(package["name"])
    print(f"prepare: rlib and dylib: {', '.join(sorted(changed))}")
    # The mods' packages join the workspace from the start, so adding them later changes only Cargo.lock.
    root = ROOT / "Cargo.toml"
    text = root.read_text()
    if f'"{MODS_DIR}/*"' not in text:
        if text.count("members = [\n") != 1:
            sys.exit("prepare: Cargo.toml has no single `members = [` to add the mods' folder to")
        root.write_text(text.replace("members = [\n", f'members = [\n    "{MODS_DIR}/*",\n', 1))
    placeholder()
    # sunrise-world's build script hashes Cargo.lock into its snapshot key and reruns when it changes: the mods'
    # packages change it, and the whole game above sunrise-world would build again, unlike the APK's. Here it
    # watches its own sources and manifests only.
    # Sunrise's sunrise-world has no build script (no snapshot key to keep from Cargo.lock).
    script = ROOT / "crates/sunrise-world/build.rs"
    if script.is_file():
        text = script.read_text()
        watched = 'for file in [manifest.join("Cargo.toml"), core.join("Cargo.toml"), lock] {'
        lock_line = '    let lock = manifest.join("../../Cargo.lock");\n'
        if watched not in text or lock_line not in text:
            sys.exit("prepare: crates/sunrise-world/build.rs no longer reads Cargo.lock as this script expects")
        text = text.replace(lock_line, "").replace(watched, watched.replace(", lock]", "]"))
        script.write_text(text)
        print("prepare: sunrise-world's build script leaves Cargo.lock out")
    # One heap for the game and every mod. The phone opens a mod library on its own (RTLD_LOCAL), so its
    # `__rust_alloc` binds to the std's library it needs (the system heap), never to the game's mimalloc: a block
    # a mod allocated and the game freed crashed the first start (SIGSEGV in `native_libraries::open`, 2026-11-19).
    # tools/prebuilt_game.py `cargo_features` builds every desktop but Linux this way for the same reason.
    manifest = ROOT / "crates/sunrise-android/Cargo.toml"
    text = manifest.read_text()
    client = 'features = ["sunrise-extras"] }'
    if text.count(client) != 1:
        sys.exit("prepare: crates/sunrise-android/Cargo.toml no longer names sunrise-client's features as expected")
    manifest.write_text(text.replace(client, 'features = ["sunrise-extras", "system-allocator"] }'))
    print("prepare: the game on the system heap (sunrise-client's system-allocator)")


def placeholder() -> None:
    """An empty package in the mods' folder: Cargo takes a members glob that matches nothing for a path."""
    folder = ROOT / MODS_DIR / "_placeholder"
    (folder / "src").mkdir(parents=True, exist_ok=True)
    (folder / "Cargo.toml").write_text('[package]\nname = "starling-native-mods-placeholder"\nversion = "0.0.0"\n'
                                       'edition = "2024"\npublish = false\n')
    (folder / "src" / "lib.rs").write_text("//! Keeps the workspace's native-mods-build/* glob matching a package.\n")


def game_build() -> tuple[str, str]:
    """The game build's identity as sunrise-mod-host's build script records it: the commit and rustc's release."""
    commit = os.environ.get("SUNRISE_GAME_BUILD_ID") or run(["git", "rev-parse", "HEAD"]).strip()
    release = next(line.removeprefix("release: ") for line in run(["rustc", "-vV"]).splitlines()
                   if line.startswith("release: "))
    return commit, release


def unpack(path: Path, into: Path) -> dict:
    """The package's files under `into`, refusing any name that would leave it; its manifest."""
    with zipfile.ZipFile(path) as z:
        manifest = tomllib.loads(z.read("sunrise-mod.toml").decode("utf-8"))
        for info in z.infolist():
            name = info.filename
            parts = Path(name).parts
            if info.is_dir() or not name or name.startswith(("/", "\\")) or ".." in parts or ":" in name:
                continue
            target = into / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(z.read(info))
    return manifest


def hooks_path(src: Path) -> str:
    """Where `#[sunrise_mod]` puts the mod's `hooks()`: the module of the file that has the attribute."""
    for file in sorted(src.rglob("*.rs")):
        if re.search(r"#\[\s*(mod_api\s*::\s*)?sunrise_mod\s*\]", file.read_text(encoding="utf-8", errors="replace")):
            module = file.relative_to(src).with_suffix("").parts
            module = [part for part in module if part not in ("lib", "mod")]
            return "::".join(["crate", *module, "hooks"])
    sys.exit(f"{src.parent.name}: no #[sunrise_mod] in its source")


def own_crate(at: Path, name: str, spec: dict) -> Path:
    """Starling: a crate of the mod's own, by `path` from the crate at `at` (Earshot's `earshot-core`): where it
    is, or exit when the path holds no crate or leaves `at` (callers check the package's own bounds)."""
    path = (at / spec["path"]).resolve()
    if not (path / "Cargo.toml").is_file():
        sys.exit(f"{at.name}: dependency {name} holds no crate at {spec['path']}")
    return path


def package_manifest(mod_id: str, version: str, cargo: dict, work: Path | None = None,
                     crate_type: str = "dylib") -> str:
    """The mod's Cargo manifest as a package of this workspace: its own dependencies, with the pinned SDK's API
    crate (`.sunrise/sdk/api/crates/sunrise-mod-api`) replaced by this checkout's. A crate of the mod's own (a
    `path` inside its package) joins the workspace with it, its API crate replaced the same way."""
    api = (ROOT / "crates" / "sunrise-mod-api").as_posix()
    lines = ["[package]", f'name = "smod-{mod_id}"', f'version = "{version}"', 'edition = "2024"',
             "publish = false", "", "[lib]", f'name = "{mod_id}"', 'path = "src/lib.rs"', f'crate-type = ["{crate_type}"]',
             "", "[dependencies]"]
    for name, spec in cargo.get("dependencies", {}).items():
        if name == "sunrise-mod-api" or (isinstance(spec, dict) and spec.get("package") == "sunrise-mod-api"):
            lines.append(f'{name} = {{ path = "{api}" }}')
        elif isinstance(spec, str):
            lines.append(f'{name} = "{spec}"')
        elif "path" in spec and work is not None and "git" not in spec:
            path = own_crate(work, name, spec)
            if not path.is_relative_to(work.resolve()):
                sys.exit(f"{mod_id}: dependency {name} is a path outside the mod's package")
            fields = ", ".join(f"{key} = {json.dumps(value)}" for key, value in spec.items() if key != "path")
            lines.append(f'{name} = {{ path = "{path.as_posix()}"{", " + fields if fields else ""} }}')
        elif "path" in spec or "git" in spec:
            sys.exit(f"{mod_id}: dependency {name} is a path or git dependency, which a build here cannot take")
        else:
            fields = ", ".join(f"{key} = {json.dumps(value)}" for key, value in spec.items())
            lines.append(f"{name} = {{ {fields} }}")
    return "\n".join(lines) + "\n"


def own_crates(work: Path, cargo: dict) -> None:
    """The mod's own crates (path dependencies inside its package) join this workspace with it: in each, a path
    to the API crate becomes this checkout's, and its own path dependencies must stay inside the package."""
    api = json.dumps((ROOT / "crates" / "sunrise-mod-api").as_posix())
    pending = [(work, name, spec) for name, spec in cargo.get("dependencies", {}).items()
               if isinstance(spec, dict) and "path" in spec and spec.get("package", name) != "sunrise-mod-api"
               and name != "sunrise-mod-api"]
    seen = set()
    while pending:
        at, name, spec = pending.pop()
        path = own_crate(at, name, spec)
        if not path.is_relative_to(work.resolve()):
            sys.exit(f"{work.name}: dependency {name} is a path outside the mod's package")
        if path in seen:
            continue
        seen.add(path)
        manifest = path / "Cargo.toml"
        text = manifest.read_text()
        for dep_name, dep in tomllib.loads(text).get("dependencies", {}).items():
            if not isinstance(dep, dict) or "path" not in dep:
                continue
            if dep_name == "sunrise-mod-api" or dep.get("package") == "sunrise-mod-api":
                text = re.sub(r'(?m)^(\s*' + re.escape(dep_name) + r'\s*=\s*\{[^}\n]*?path\s*=\s*)"[^"]*"',
                              lambda m: m.group(1) + api, text)
            else:
                pending.append((path, dep_name, dep))
        manifest.write_text(text)


# The crates of this workspace a generated patch crate may name, by their library name.
def workspace_libraries() -> dict[str, Path]:
    found = {}
    for manifest in (ROOT / "crates").glob("*/Cargo.toml"):
        data = tomllib.loads(manifest.read_text())
        package = data.get("package", {}).get("name")
        if package:
            found[data.get("lib", {}).get("name", package.replace("-", "_"))] = manifest.parent
    return found


def patch_tool() -> Path:
    """sunrise-android-patch for this computer: `SUNRISE_ANDROID_PATCH`, or built here (host target). `mods` builds
    it before any mod joins the workspace: a mod's own manifest would stop Cargo reading it."""
    named = os.environ.get("SUNRISE_ANDROID_PATCH")
    if named:
        return Path(named)
    target = ROOT / "target" / "host-tools"
    subprocess.run(["cargo", "build", "--release", "-q", "-p", "sunrise-native-build", "--bin", "sunrise-android-patch"],
                   check=True, cwd=ROOT, env={**os.environ, "CARGO_TARGET_DIR": str(target)})
    return target / "release" / "sunrise-android-patch"


def patched(mod_id: str, work: Path, folder: Path, commit: str, abi: str) -> list[str] | None:
    """The patch crate of `mod_id`'s `[native.*]` rows, as the workspace package `smod-<id>-patched` (library
    `sunrise_patched`, which the install expressions name), and those expressions; None when it has none."""
    source = os.environ.get("SUNRISE_UNSTAGED_SOURCE")
    if not (ROOT / "functions.json").is_file() or not source:
        sys.exit(f"mods: {mod_id} has [native.*] rows: build from the staged game, with SUNRISE_UNSTAGED_SOURCE")
    # Outside `target`, which may be a link (the tool refuses output paths through one), and outside the
    # workspace's member glob.
    scratch = folder.parent / ".android-patch" / mod_id
    if scratch.exists():
        shutil.rmtree(scratch)
    done = subprocess.run([str(patch_tool()), str(ROOT), source, commit, str(scratch), build_apk.TARGETS[abi][0],
                           ",".join(GAME_FEATURES), str(work)], cwd=ROOT, capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(f"mods: {mod_id}: {done.stderr.strip()}")
    result = json.loads(done.stdout.strip().splitlines()[-1])
    if result is None:
        return None
    crate = Path(result["dir"]) / "sunrise_patched"
    dest = folder / f"{mod_id}-patched"
    shutil.copytree(crate, dest)
    # The generator re-exports every module of the copied ones, a test module too (`native_hooks`'s
    # `*_tests`, `#![cfg(test)]` inside), which the game built for a phone doesn't have.
    lib = dest / "src" / "lib.rs"
    kept = [line for line in lib.read_text(encoding="utf-8").splitlines(keepends=True)
            if not re.fullmatch(r"\s*pub use ::sunrise_\w+(::\w+)*_tests;\s*", line)]
    lib.write_bytes("".join(kept).encode("utf-8"))
    text = "".join(f.read_text(encoding="utf-8") for f in (dest / "src").rglob("*.rs"))
    libraries = workspace_libraries()
    # The mod's crate by the name its helpers call it, as the desktop's build names it
    # (`api::Project::crate_name`): its manifest's library name, else its package's (`mod-flair` is `mod_flair`).
    own = tomllib.loads((work / "Cargo.toml").read_text())
    own_name = own.get("lib", {}).get("name") or own["package"]["name"].replace("-", "_")
    lines = ["[package]", f'name = "smod-{mod_id}-patched"', 'version = "0.0.0"', 'edition = "2024"',
             "publish = false", "", "[lib]", 'name = "sunrise_patched"', 'path = "src/lib.rs"',
             'crate-type = ["rlib"]', "", "[dependencies]",
             f'{own_name} = {{ path = "{work.as_posix()}", package = "smod-{mod_id}" }}']
    for name, path in sorted(libraries.items()):
        if re.search(r"\b" + re.escape(name) + r"\s*::", text):
            package = tomllib.loads((path / "Cargo.toml").read_text())["package"]["name"]
            # No default features: the APK's own build (crates/sunrise-android) names the game's, and the
            # defaults would bring the desktop's (controllers: hidapi, libudev).
            features = f', features = {json.dumps(list(GAME_FEATURES))}' if package == "sunrise-client" else ""
            lines.append(f'{name} = {{ path = "{path.as_posix()}", package = "{package}", '
                         f'default-features = false{features} }}')
    # The crates from crates.io the game's own source names (`winit`), as sunrise-client takes them: its Android
    # table's over its own.
    game = tomllib.loads((ROOT / "crates" / "sunrise-client" / "Cargo.toml").read_text())
    external = dict(game.get("dependencies", {}))
    external.update(game.get("target", {}).get('cfg(target_os = "android")', {}).get("dependencies", {}))
    for package, spec in sorted(external.items()):
        name = package.replace("-", "_")
        if name in libraries or not re.search(r"\b" + re.escape(name) + r"\s*::", text):
            continue
        if isinstance(spec, str):
            spec = {"version": spec}
        if "path" in spec or spec.get("workspace"):
            continue
        fields = ", ".join(f"{key} = {json.dumps(value)}" for key, value in spec.items() if key != "optional")
        lines.append(f'{name} = {{ package = "{package}", {fields} }}')
    (dest / "Cargo.toml").write_text("\n".join(lines) + "\n")
    return result["installs"]


HORIZON_CRATE = re.compile(r"\bhorizon_(?=[a-z])")
# Horizon's mod API names over Sunrise's (tools/android_compat/starling_compat.rs), put in a mod made for Horizon.
COMPAT = ROOT / "tools" / "android_compat" / "starling_compat.rs"
INNER = re.compile(r"\A(?:\s*(?://[!/][^\n]*|#!\[[^\]]*\])\n)*")


def body_end(text: str, start: int) -> int:
    """The index just past the `}` closing the block whose `{` is at `start` (strings and comments aside)."""
    depth, i = 0, start
    while i < len(text):
        c = text[i]
        if text.startswith("//", i):
            i = text.find("\n", i)
            if i < 0:
                return len(text)
            continue
        if c == '"':
            i += 1
            while i < len(text) and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return len(text)


MOVEMENT_HOOK = re.compile(
    r"fn\s+movement_input\s*\(\s*&self\s*,\s*(\w+)\s*:\s*&dyn\s+(?:mod_api::)?ClientHost\s*,\s*"
    r"(\w+)\s*:\s*&mut\s+(?:mod_api::)?MovementInputEvent\s*,?\s*\)\s*\{")


def api_rules(text: str) -> str:
    """Horizon hooks Sunrise renamed, rewritten in place: the old body runs on the old event (starling_compat)."""
    m = MOVEMENT_HOOK.search(text)
    while m:
        host, event = m.group(1), m.group(2)
        end = body_end(text, m.end() - 1)
        body = text[m.end():end - 1]
        new = (f"fn movement_input_update(&self, {host}: &dyn mod_api::ClientHost, "
               f"__starling_event: &mut mod_api::MovementInputUpdateEvent<'_>) {{\n"
               f"        let mut __starling_old = crate::starling_compat::MovementInputEvent::of(&*__starling_event.input);\n"
               f"        (|{event}: &mut crate::starling_compat::MovementInputEvent| {{{body}}})(&mut __starling_old);\n"
               f"        __starling_old.write(&mut *__starling_event.input);\n    }}")
        text = text[:m.start()] + new + text[end:]
        m = MOVEMENT_HOOK.search(text, m.start() + len(new))
    text = re.sub(r"\bmod_api::MovementInputEvent\b", "crate::starling_compat::MovementInputEvent", text)
    text = re.sub(r"(use\s+mod_api::\{[^}]*?)\bMovementInputEvent\s*,\s*", r"\1", text, flags=re.S)
    text = re.sub(r"(use\s+mod_api::\{[^}]*?),\s*MovementInputEvent\b", r"\1", text, flags=re.S)
    text = re.sub(r"\bhook_ids::movement_input\b", "hook_ids::movement_input_update", text)
    return text


def compat(work: Path) -> None:
    """A mod made for Horizon (its rows or source name `horizon_`... before `retarget`, or it calls Horizon's
    names): `starling_compat` in its crate, and every file of it `use`s its traits, so the old calls resolve."""
    src = work / "src"
    lib = src / "lib.rs"
    if not lib.is_file() or (src / "starling_compat.rs").exists():
        return
    shutil.copyfile(COMPAT, src / "starling_compat.rs")
    # A row's helper (one function, which the tool inlines into the game's copy) and a support module (a module
    # of the patch crate, where `crate` is the game's) are not the mod's crate: they take nothing.
    manifest = (work / "sunrise-mod.toml").read_text(encoding="utf-8")
    # Any source file a row names (a helper, a module, a bridge), and the files of a crate of the mod's own
    # (a folder of `src` with its Cargo.toml), whose `crate` isn't the mod's.
    elsewhere = {(work / m).resolve() for m in re.findall(r'"(src/[^"]+\.rs)"', manifest)}

    def own_crate_file(path: Path) -> bool:
        return any((parent / "Cargo.toml").is_file() for parent in path.parents
                   if parent != work and work in parent.parents)

    for path in src.rglob("*.rs"):
        if path.name == "starling_compat.rs" or path.resolve() in elsewhere or own_crate_file(path):
            continue
        text = api_rules(path.read_text(encoding="utf-8"))
        head = INNER.match(text).end()
        line = "#[allow(unused_imports)]\nuse crate::starling_compat::*;\n"
        if path == lib:
            line = "mod starling_compat;\n" + line
        path.write_bytes((text[:head] + line + text[head:]).encode("utf-8"))
GENERICS = re.compile(r"<[^<>]*>")


def short(path: str) -> str:
    """A function's path without generic arguments, as the index and a row may differ in them."""
    while GENERICS.search(path):
        path = GENERICS.sub("", path)
    return path


def retarget(mod_id: str, work: Path) -> list[str]:
    """Starling: a Horizon mod's `[native.*]` rows, made for Horizon's game, moved to this one. Its crates are Sunrise's
    (`horizon_client` is `sunrise_client`); a target whose module moved in the sync is found in the staged game's
    function index by its type and name, when exactly one function there has them; a support module's file is the
    same file under the renamed crate, or the game crate's root. The rows' bindings name the same crates. Writes the
    manifest back; returns what it could not place, for the build's log."""
    manifest_path = work / "sunrise-mod.toml"
    text = manifest_path.read_text(encoding="utf-8")
    if "horizon_" not in text and "crates/horizon-" not in text:
        return []
    index = json.loads((ROOT / "functions.json").read_text())["functions"]
    by_short = {}
    for key in index:
        by_short.setdefault(short(key), []).append(key)
    by_tail = {}
    for key in index:
        parts = short(key).split("::")
        by_tail.setdefault("::".join(parts[-2:]), []).append(key)
    unplaced = []

    def place(target: str) -> str:
        renamed = HORIZON_CRATE.sub("sunrise_", target)
        if short(renamed) in by_short:
            return renamed
        tail = "::".join(short(renamed).split("::")[-2:])
        found = [k for k in by_tail.get(tail, []) if k.split("::")[0] == renamed.split("::")[0]]
        if len(found) == 1:
            return found[0]
        unplaced.append(target)
        return renamed

    def target_line(match: re.Match) -> str:
        return f'{match.group(1)}"{place(match.group(2))}"'

    text = re.sub(r'(?m)^(\s*target\s*=\s*)"([^"]+)"', target_line, text)

    def file_line(match: re.Match) -> str:
        path = re.sub(r"crates/horizon-", "crates/sunrise-", match.group(2))
        if not (ROOT / path).is_file():
            unplaced.append(match.group(2))
            path = "crates/sunrise-client/src/game.rs"
        return f'{match.group(1)}"{path}"'

    text = re.sub(r'(?m)^(\s*file\s*=\s*)"(crates/[^"]+)"', file_line, text)
    # Bindings' types and the rest: the crates under their Sunrise names.
    text = HORIZON_CRATE.sub("sunrise_", text)
    manifest_path.write_bytes(text.encode("utf-8"))
    for source in (work / "src").rglob("*.rs"):
        code = source.read_text(encoding="utf-8")
        renamed = HORIZON_CRATE.sub("sunrise_", code)
        if renamed != code:
            source.write_bytes(renamed.encode("utf-8"))
    if unplaced:
        print(f"mods: {mod_id}: not placed in this game: {', '.join(unplaced)}")
    return unplaced


# The game crate's features in the APK (crates/sunrise-android's dependency on sunrise-client and its default):
# a copied function is compiled under the same `cfg`s.
GAME_FEATURES = ("audio", "sunrise-extras")


def mods(out: Path, packages: list[Path], profile: str, abi: str) -> None:
    commit, release = game_build()
    template = TEMPLATE.read_text()
    folder = ROOT / MODS_DIR
    if f'"{MODS_DIR}/*"' not in (ROOT / "Cargo.toml").read_text():
        sys.exit("mods: run `prepare` first, before the APK's build")
    for old in folder.iterdir() if folder.exists() else ():
        if old.name != "_placeholder":
            shutil.rmtree(old)
    placeholder()
    if os.environ.get("SUNRISE_UNSTAGED_SOURCE") and not os.environ.get("SUNRISE_ANDROID_PATCH"):
        os.environ["SUNRISE_ANDROID_PATCH"] = str(patch_tool())
    built = []
    for path in packages:
        work = folder / path.stem
        manifest = unpack(path, work)
        mod_id, version = manifest["id"], manifest["version"]
        if not ID.fullmatch(mod_id):
            sys.exit(f"{path.name}: id {mod_id!r} is not a crate name")
        native = manifest.get("native")
        if not isinstance(native, dict):
            sys.exit(f"{path.name}: not a native mod")
        tables = sorted(key for key, value in native.items() if isinstance(value, (list, dict)))
        if tables and any(t for *_, t in built if t):
            sys.exit(f"mods: {mod_id}: one mod with [native.*] rows a build")
        if work.name != mod_id:
            work.rename(folder / mod_id)
            work = folder / mod_id
        cargo = tomllib.loads((work / "Cargo.toml").read_text())
        # The patch tool reads the mod's own manifest (the pinned SDK's path), before it joins this workspace.
        if tables:
            # Only mods with [native.*] rows for now: the API mods the store builds already have their ports.
            compat(work)
            retarget(mod_id, work)
        installs = patched(mod_id, work, folder, commit, abi) if tables else None
        (work / "Cargo.toml").write_text(package_manifest(mod_id, version, cargo, work,
                                                          "rlib" if tables else "dylib"))
        own_crates(work, cargo)
        # `<build> <mod> <version> <rustc release> <panic strategy>`, as sunrise-mod-host's loader reads it
        # (`identity_parts`): the APK and its mods unwind (build_apk.py BUILD_STD). The entry template adds the
        # panic strategy itself (`sunrise_native_build`), so the library carries the first four words and
        # native-mods.json all five. A four-word record made every library refused on the phone, and five words
        # in the library too gave it "unwind unwind" ("native mods left out").
        built_as = f"{commit} {mod_id} {version} {release}"
        identity = f"{built_as} unwind"
        side = SIDES.get(manifest.get("side", "both"), "Both")
        hooks = hooks_path(work / "src")
        if tables:
            # The library is a crate of its own above the mod's and the patch crate.
            hooks = hooks.replace("crate::", f"{mod_id}::", 1)
        rows = {
            "NATIVE_BUILD": built_as,
            "MOD_ENTRIES": f'"{mod_id}" => {hooks},',
            "BUILT_MODS": f'mod_api::__private::native::BuiltMod {{ id: "{mod_id}", version: "{version}", '
                          f"side: mod_api::Side::{side} }},",
            "MOD_SLOTS": "",
            "MOD_REDIRECTS": "".join(f"{i},\n" for i in installs or ()),
        }
        entry = template
        for marker, value in rows.items():
            entry = entry.replace(f"/* {marker} */", value)
        if not tables:
            (work / "src" / "starling_native_entry.rs").write_text(entry)
            with open(work / "src" / "lib.rs", "a", encoding="utf-8") as lib:
                lib.write('\n#[path = "starling_native_entry.rs"]\nmod starling_native_entry;\n')
        else:
            library = folder / f"{mod_id}-library"
            (library / "src").mkdir(parents=True)
            api = (ROOT / "crates" / "sunrise-mod-api").as_posix()
            host = (ROOT / "crates" / "sunrise-mod-host").as_posix()
            (library / "Cargo.toml").write_text("\n".join([
                "[package]", f'name = "smod-{mod_id}-library"', f'version = "{version}"', 'edition = "2024"',
                "publish = false", "", "[lib]", f'name = "{mod_id}_library"', 'path = "src/lib.rs"',
                'crate-type = ["dylib"]', "", "[dependencies]",
                f'mod_api = {{ path = "{api}", package = "sunrise-mod-api", default-features = false }}',
                # As crates/sunrise-android takes it: the host's defaults are the desktop's (controllers).
                f'sunrise_mod_host = {{ path = "{host}", package = "sunrise-mod-host", default-features = false }}',
                f'{mod_id} = {{ path = "{work.as_posix()}", package = "smod-{mod_id}" }}',
                f'sunrise_patched = {{ path = "{(folder / (mod_id + "-patched")).as_posix()}", '
                f'package = "smod-{mod_id}-patched" }}' if installs is not None else "",
            ]) + "\n")
            (library / "src" / "lib.rs").write_text(
                # After the template: it opens with its inner doc comment.
                entry + f"\nextern crate {mod_id};\n"
                + ("extern crate sunrise_patched;\n" if installs is not None else "")
                + "extern crate sunrise_mod_host as _;\n")
        built.append((mod_id, version, identity, path, bool(tables)))
    if not built:
        sys.exit("mods: nothing to build")

    sdk = build_apk.sdk_dir()
    ndk = Path(os.environ.get("ANDROID_NDK_HOME") or sdk / "ndk" / build_apk.NDK)
    build_apk.build_library(profile, ndk, sdk, abi, ("sunrise-android", *(
        f"smod-{m}-library" if staged else f"smod-{m}" for m, *_, staged in built)))
    triple = build_apk.TARGETS[abi][0]
    deps = ROOT / "target" / triple / ("debug" if profile == "dev" else profile)
    if out.exists():
        shutil.rmtree(out)
    (out / "sunrise-native-mods").mkdir(parents=True)
    records = []
    for mod_id, version, identity, path, staged in built:
        library = out / f"lib{mod_id}.so"
        name = f"lib{mod_id}_library.so" if staged else f"lib{mod_id}.so"
        made = next((p for p in (deps / name, deps / "deps" / name) if p.is_file()), None)
        if made is None:
            sys.exit(f"mods: no {name} under {deps}")
        build_apk.run(build_apk.ndk_tool(ndk, "llvm-strip"), "--strip-unneeded", "-o", library, made)
        records.append({"file": library.name, "sha256": hashlib.sha256(library.read_bytes()).hexdigest(),
                        "mod": mod_id, "identity": identity})
        # The whole package, its source too: the game reads it as the desktop reads the one beside its
        # executable, and a `[native]` manifest without its source is refused ("inconsistent code declaration").
        unpack(path, out / "sunrise-native-mods" / mod_id)
    (out / "native-mods.json").write_text(json.dumps({"libraries": records}, indent=2) + "\n")
    print(f"mods: {', '.join(m for m, *_ in built)} for {commit[:10]} in {out}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("prepare")
    build = sub.add_parser("mods")
    build.add_argument("out", type=Path)
    build.add_argument("packages", type=Path, nargs="+")
    build.add_argument("--profile", default="quick")
    build.add_argument("--abi", default=build_apk.ABI)
    args = parser.parse_args()
    if args.command == "prepare":
        prepare()
    else:
        mods(args.out.resolve(), [p.resolve() for p in args.packages], args.profile, args.abi)


if __name__ == "__main__":
    main()
