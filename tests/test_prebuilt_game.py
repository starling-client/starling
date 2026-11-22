import contextlib
import io
import json
import os
import shutil
import sys
import struct
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from tools import prebuilt_game
from tests.test_ci_concurrency import yaml

COMMIT = "c" * 40


def write_build_data(root: Path, identity: dict | None = None, commit: str = COMMIT) -> None:
    """The build-data files with their header for `commit`, and an identity naming it, with the smallest
    slices `check` accepts (each required member, as an empty file) unless `identity` gives its own."""
    for name in prebuilt_game.BUILD_DATA:
        (root / name).write_text(json.dumps({"schema": 1, "build": {"commit": commit}}))
    so, exe = prebuilt_game.dylib_suffix(), ".exe" if sys.platform == "win32" else ""
    lib = prebuilt_game.dylib_prefix()
    members = {"runtime": [f"bin/sunrise-client{exe}",
                           f"deps/{lib}sunrise_mod_host{so}",
                           *[f"deps/{lib}{crate}{so}" for crate in ("mod_api", "mod_vocab")]],
               "api": [prebuilt_game.NATIVE_MODS_TEMPLATE, "deps/libmod_api-1.rmeta"]}
    for name in members["runtime"] + members["api"]:
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_bytes(b"")
    (root / prebuilt_game.RUNTIME / prebuilt_game.EXECUTABLE).write_bytes(b"\0" + prebuilt_game.holder_mark() + b"\0")
    slices = {"runtime": [prebuilt_game.IDENTITY, *members["runtime"]],
              "api": [*prebuilt_game.API_BUILD_DATA, *members["api"]]}
    (root / prebuilt_game.IDENTITY).write_text(json.dumps({"slices": slices, **(identity or {}), "commit": commit}))


class StageToolTests(unittest.TestCase):
    def test_stage_sync_only_rewrites_different_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, stage = root / "next", root / "stage"
            before = {"same": b"unchanged\r\n", "nested/changed": b"new", "stale/removed": b"old",
                      "file-to-dir": b"old", "dir-to-file/old": b"old"}
            # A revert can have older source mtimes and the same byte count as the previous build's sources.
            after = {"same": b"unchanged\r\n", "nested/changed": b"old", "nested/added": b"new",
                     "file-to-dir/new": b"new", "dir-to-file": b"new"}
            for folder, files, mtime in ((stage, before, 2000), (source, after, 1000)):
                for name, data in files.items():
                    path = folder / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(data)
                    os.utime(path, (mtime, mtime))
            same, changed = stage / "same", stage / "nested/changed"
            same_mtime, changed_mtime = same.stat().st_mtime_ns, changed.stat().st_mtime_ns

            prebuilt_game.sync_stage(source, stage)

            self.assertEqual(same.stat().st_mtime_ns, same_mtime)
            self.assertGreater(changed.stat().st_mtime_ns, changed_mtime)
            self.assertEqual(sorted(prebuilt_game.tree(stage)), sorted(prebuilt_game.tree(source)))
            self.assertEqual({path.relative_to(stage).as_posix(): path.read_bytes()
                              for path in stage.rglob("*") if path.is_file()}, after)

    @unittest.skipUnless(os.name == "posix", "POSIX executable bits and symlinks")
    def test_sync_mirrors_execute_bits_without_rewriting_bytes_or_following_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, stage = root / "next", root / "stage"
            for folder in (source, stage):
                folder.mkdir()
                (folder / "script").write_bytes(b"unchanged")
                (folder / "link").symlink_to("script")
            incoming, staged = source / "script", stage / "script"
            staged.chmod(0o644)
            os.utime(staged, (1000, 1000))
            mtime = staged.stat().st_mtime_ns
            for mode in (0o755, 0o644):
                with self.subTest(mode=mode):
                    incoming.chmod(mode)
                    prebuilt_game.sync_stage(source, stage)
                    self.assertEqual(staged.stat().st_mode & 0o777, mode)
                    self.assertEqual(staged.stat().st_mtime_ns, mtime)
                    self.assertEqual(staged.read_bytes(), b"unchanged")
                    self.assertFalse((stage / "link").is_symlink())
                    self.assertFalse((stage / "link").exists())

    def test_stage_tool_failure_cleans_scratch_and_keeps_previous_stage(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            stage = target / "stage"
            stage.mkdir()
            marker = stage / "old"
            marker.write_bytes(b"previous build")
            with patch.object(prebuilt_game, "head", return_value=COMMIT), \
                 patch.object(prebuilt_game, "tracked_files", return_value=[("new", b"new input", 0o644)]), \
                 patch.object(prebuilt_game, "run", side_effect=subprocess.CalledProcessError(1, "sunrise-stage")):
                with self.assertRaises(subprocess.CalledProcessError):
                    prebuilt_game.stage(target, stage_tool=target / "sunrise-stage")
            self.assertEqual(list(target.iterdir()), [stage])
            self.assertEqual(list(stage.iterdir()), [marker])
            self.assertEqual(marker.read_bytes(), b"previous build")

    def test_sdk_build_rejects_a_second_writer_and_releases_the_lock_after_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def build_then_fail(out, server, target_dir):
                self.assertEqual(target_dir, root.resolve() / "prebuilt")
                with self.assertRaises(SystemExit):
                    prebuilt_game.build(root / "other-sdk", server=False)
                raise RuntimeError("build failed")

            with patch.dict(os.environ, CARGO_TARGET_DIR=directory), \
                 patch.object(prebuilt_game, "_build_locked", side_effect=build_then_fail) as build_locked:
                with self.assertRaisesRegex(RuntimeError, "build failed"):
                    prebuilt_game.build(root / "sdk", server=True)
                build_locked.assert_called_once()
                build_locked.side_effect = None
                prebuilt_game.build(root / "other-sdk", server=False)
                build_locked.assert_called_with(root / "other-sdk", False, root.resolve() / "prebuilt")

    def test_prebuilt_tool_stages_fresh_tracked_copy_without_cargo(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            committed = b"fn first() {}\nfn second() {}\n"
            (source / "tracked.rs").write_bytes(committed)
            manifest = source / "crates/sunrise-mod-api/Cargo.toml"
            manifest.parent.mkdir(parents=True)
            manifest.write_text("[package]\nname = 'fixture'\n")
            git = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.autocrlf=false"]
            subprocess.run([*git, "init", "-q"], cwd=source, check=True)
            subprocess.run([*git, "add", "tracked.rs", "crates"], cwd=source, check=True)
            subprocess.run([*git, "update-index", "--chmod=+x", "tracked.rs"], cwd=source, check=True)
            subprocess.run([*git, "commit", "-qm", "fixture"], cwd=source, check=True)
            commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=source, check=True, text=True,
                                    stdout=subprocess.PIPE).stdout.strip()
            # A Windows checkout's line endings, and edits never committed: neither reaches the stage.
            (source / "tracked.rs").write_bytes(committed.replace(b"\n", b"\r\n") + b"fn edited() {}\r\n")
            (source / "untracked.rs").write_text("not copied")
            target = root / "output"
            tool = root / "unused" / ".." / "sunrise-stage"
            calls = []
            metadata_calls = []
            real_run = prebuilt_game.run

            def capture(command, **kwargs):
                if command[0] == "git":
                    return real_run(command, **kwargs)
                scratch = kwargs["cwd"]
                self.assertEqual(scratch.parent, target)
                self.assertNotEqual(scratch, target / "stage")
                if command[:2] == ["cargo", "metadata"]:
                    metadata_calls.append(scratch)
                    package = {"id": "api", "name": "sunrise-mod-api",
                               "manifest_path": str(scratch / manifest.relative_to(source)),
                               "targets": [{"kind": ["lib"]}]}
                    return json.dumps({"packages": [package, {"id": "core", "name": "sunrise-core", "targets": []}],
                                       "resolve": {"nodes": [{"id": "api", "deps": [
                                           {"pkg": "core", "dep_kinds": [{"kind": None}]}]},
                                           {"id": "core", "deps": []}]}})
                calls.append(scratch)
                self.assertEqual(command, [str(tool.resolve()), str(scratch), str(scratch / "internals.json"), commit])
                self.assertIsNone(kwargs["env"])
                self.assertEqual((scratch / "tracked.rs").read_bytes(), committed)
                if os.name == "posix":
                    self.assertTrue((scratch / "tracked.rs").stat().st_mode & 0o100)
                self.assertFalse((scratch / "untracked.rs").exists())
                (scratch / "tracked.rs").write_text("staged")
                return "staged fixture\n"

            with patch.object(prebuilt_game, "ROOT", source), \
                 patch.object(prebuilt_game, "rustc_host", return_value="x86_64-unknown-linux-gnu"), \
                 patch.object(prebuilt_game, "run", side_effect=capture):
                staged = prebuilt_game.stage(target, stage_tool=tool)
                self.assertEqual(staged, target / "stage")
                staged_manifest = staged / manifest.relative_to(source)
                self.assertEqual(staged_manifest.read_bytes(), manifest.read_bytes())
                self.assertEqual(metadata_calls, [])

                self.assertEqual(prebuilt_game.stage(target, stage_tool=tool, sdk_dylibs=True), staged)
                self.assertEqual(staged_manifest.read_text(),
                                 "[package]\nname = 'fixture'\n\n[lib]\ncrate-type = [\"rlib\", \"dylib\"]\n")
                # Both the stage tool and the manifest rewrite start over, but their output stays in place.
                mtimes = {}
                for file in (staged / "tracked.rs", staged_manifest):
                    os.utime(file, (1000, 1000))
                    mtimes[file] = file.stat().st_mtime_ns
                self.assertEqual(prebuilt_game.stage(target, stage_tool=tool, sdk_dylibs=True), staged)
                self.assertEqual({file: file.stat().st_mtime_ns for file in mtimes}, mtimes)

                self.assertEqual(prebuilt_game.stage(target, stage_tool=tool, sdk_dylibs=False), staged)
                self.assertEqual(staged_manifest.read_bytes(), manifest.read_bytes())
                if os.name == "posix":
                    self.assertTrue((staged / "tracked.rs").stat().st_mode & 0o100)
            self.assertEqual(len(calls), 4)
            self.assertEqual(metadata_calls, calls[1:3])
            self.assertTrue(all(not scratch.exists() for scratch in calls))
            self.assertTrue((source / "tracked.rs").read_bytes().endswith(b"fn edited() {}\r\n"))


class RegistryPatchTests(unittest.TestCase):
    def test_a_crate_beside_the_base_library_stays_inside_the_library_above_it(self):
        """`call` and `core` both use `digest` and neither links the other: as two libraries each would hold a
        copy, which rustc refuses. `call` is not split out, so the library above it holds it. A derive both
        use is no copy: it runs in the compiler."""
        def package(name, *dependencies, kind="lib"):
            return ({"id": name, "name": name, "manifest_path": "", "targets": [{"kind": [kind]}]},
                    {"id": name, "deps": [{"pkg": dependency, "dep_kinds": [{"kind": None}]}
                                          for dependency in dependencies]})
        packages, nodes = zip(
            package("sunrise-mod-api", "sunrise-core", "vocab", "call"),
            package("vocab", "sunrise-core"), package("call", "sha2", "derive"), package("sha2", "digest"),
            package("sunrise-core", "md-5", "derive"), package("md-5", "digest"), package("digest"),
            package("derive", "syn", kind="proc-macro"), package("syn"),
            package("sunrise-mod-host", "sunrise-mod-api", "sha2"))
        metadata = {"packages": list(packages), "resolve": {"nodes": list(nodes)}}
        split = prebuilt_game.sdk_dylibs(metadata)
        self.assertEqual(split, {"sunrise-core", "vocab", "sunrise-mod-api"})
        self.assertIsNone(prebuilt_game.shared_twice(metadata, split | {"sunrise-mod-host"}))
        clash = prebuilt_game.shared_twice(metadata, split | {"sunrise-mod-host", "call"})
        for crate in ("call", "sunrise-core", "digest"):
            self.assertIn(crate, clash)
        for crate in ("derive", "syn", "sha2", "md-5"):
            self.assertNotIn(crate, clash)

    def test_sdk_carries_only_patched_crate_and_resolution_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            staged, sdk = root / "stage", root / "sdk"
            patched = staged / "third_party" / "allocator-sys"
            patched.mkdir(parents=True)
            sdk.mkdir()
            (staged / "Cargo.toml").write_text("[workspace]\nmembers=[]\n[patch.crates-io]\nallocator-sys={path='third_party/allocator-sys'}\n")
            (patched / "Cargo.toml").write_text("[package]\nname='allocator-sys'\nversion='0.1.49'\n")
            (patched / "src").mkdir()
            (patched / "src/lib.rs").write_text("pub fn patched() {}")
            (staged / "game.rs").write_text("must not ship")
            patches = prebuilt_game.carry_patches(staged, sdk)
            self.assertEqual(patches["crates-io"]["allocator-sys"]["path"], "patches/allocator-sys")
            self.assertEqual((sdk / "patches/allocator-sys/src/lib.rs").read_text(), "pub fn patched() {}")
            self.assertFalse((sdk / "game.rs").exists())
            self.assertNotIn("workspace", (sdk / "Cargo.toml").read_text())

    def test_source_symlink_is_refused_without_copying_its_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, sdk = root / "source", root / "sdk"
            patched = source / "patch"
            patched.mkdir(parents=True)
            sdk.mkdir()
            (source / "Cargo.toml").write_text("[patch.crates-io]\nallocator-sys={path='patch'}\n")
            outside = root / "private.txt"
            outside.write_text("must not ship")
            (patched / "leak").symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "symlink"):
                prebuilt_game.carry_patches(source, sdk)
            self.assertFalse((sdk / "patches").exists())

    def test_output_inventory_rejects_extra_files_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            sdk = Path(directory)
            patch = sdk / "patches/allocator-sys"
            patch.mkdir(parents=True)
            declared = patch / "Cargo.toml"
            declared.write_text("manifest")
            (sdk / "deps").mkdir()
            for crate in ("sunrise_official", "sunrise_mod_host"):
                (sdk / "deps" / f"lib{crate}.so").write_bytes(b"")
            write_build_data(sdk, {"patch_files": {
                "patches/allocator-sys/Cargo.toml": prebuilt_game.sha256(declared)}})
            prebuilt_game.check(sdk)
            extra = patch / "extra.rs"
            extra.write_text("not inventoried")
            with self.assertRaisesRegex(SystemExit, "aren't compiler outputs"):
                prebuilt_game.check(sdk)
            extra.unlink()
            extra.symlink_to(declared)
            with self.assertRaisesRegex(SystemExit, "symlinks"):
                prebuilt_game.check(sdk)
            extra.unlink()
            declared.write_text("modified")
            with self.assertRaisesRegex(SystemExit, "inventory mismatch"):
                prebuilt_game.check(sdk)


class SdkAssetTests(unittest.TestCase):
    RUNTIME = ["deps/libsunrise_mod_host.so", "bin/sunrise-client",
               "deps/libmod_api.so", "deps/libmod_vocab.so"]
    API = ["deps/libmod_api-1.rmeta", "host/libserde_derive.so"]
    ENGINE = ["deps/libsunrise_client-1.rmeta", "native/libwindows.a"]

    def out(self, root: Path, mtime: int) -> Path:
        out = root / "prebuilt"
        for name in self.RUNTIME + self.API + self.ENGINE + [prebuilt_game.NATIVE_MODS_TEMPLATE]:
            (out / name).parent.mkdir(parents=True, exist_ok=True)
            (out / name).write_bytes(name.encode())
        slices = {"runtime": [prebuilt_game.IDENTITY, *self.RUNTIME],
                  "api": [*self.API, *prebuilt_game.API_BUILD_DATA, prebuilt_game.NATIVE_MODS_TEMPLATE]}
        with patch.object(sys, "platform", "linux"):
            write_build_data(out, {"package_format": 6, "side": "server", "target": "x86_64-unknown-linux-gnu",
                                   "slices": slices})
        for path in out.rglob("*"):
            os.utime(path, (mtime, mtime))
        return out

    def pack(self, out: Path, dist: Path, **options) -> None:
        def run(command, **kwargs):
            self.assertEqual(command[-3:], ["surface", str(prebuilt_game.ROOT), str(dist / "surface.json")])
            (dist / "surface.json").write_text('{"hash": "ab"}')
        built = out.parent / "tool"
        built.mkdir(exist_ok=True)
        (built / prebuilt_game.TOOL_PROGRAM).write_bytes(b"tool")
        with patch.object(prebuilt_game, "head", return_value=COMMIT), patch.object(prebuilt_game, "run", side_effect=run), \
                patch.object(prebuilt_game, "tool", return_value=built):
            prebuilt_game.pack(out, dist, stage_tool=Path("sunrise-stage"), **options)

    def test_pack_writes_three_archives_that_split_the_package_and_the_same_every_run(self):
        import lzma
        import tarfile
        archives = []
        for mtime in (1000, 2000):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                dist = root / "dist"
                out = self.out(root, mtime)
                with patch.object(sys, "platform", "linux"):
                    self.pack(out, dist)
                names = sorted(path.name for path in dist.iterdir())
                self.assertEqual(names, ["api-x86_64-unknown-linux-gnu-server.tar.xz", "build-data.tar.xz",
                                         "engine-x86_64-unknown-linux-gnu-server.tar.xz",
                                         "runtime-x86_64-unknown-linux-gnu-server.tar.xz", "surface.json"])
                with tarfile.open(dist / "build-data.tar.xz") as tar:
                    self.assertEqual(tar.getnames(), [".", "./functions.json", "./internals.json", "./loops.json"])
                files = {}
                for kind in ("runtime", "api", "engine"):
                    with lzma.open(dist / f"{kind}-x86_64-unknown-linux-gnu-server.tar.xz") as xz, \
                            tarfile.open(fileobj=xz) as tar:
                        self.assertEqual({member.mtime for member in tar.getmembers()}, {0})
                        files[kind] = sorted(m.name[2:] for m in tar.getmembers() if m.isfile())
                self.assertEqual(files["runtime"], sorted([prebuilt_game.IDENTITY, *self.RUNTIME]))
                self.assertEqual(files["api"], sorted([*self.API, *prebuilt_game.API_BUILD_DATA,
                                                      prebuilt_game.NATIVE_MODS_TEMPLATE]))
                # Everything not listed is the engine's, so the three together are the whole package, once each. The
                # function index is the engine route's alone.
                self.assertEqual(files["engine"], sorted([*self.ENGINE, prebuilt_game.FUNCTIONS]))
                every = sorted(p.relative_to(out).as_posix() for p in out.rglob("*") if p.is_file())
                self.assertEqual(sorted(files["runtime"] + files["api"] + files["engine"]), every)
                archives.append([(dist / name).read_bytes() for name in names])
        self.assertEqual(archives[0], archives[1])

    def test_parallel_pack_matches_serial_with_spawn(self):
        import multiprocessing
        from concurrent.futures import ProcessPoolExecutor
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = self.out(root, 1000)
            serial, parallel = root / "serial", root / "parallel"
            serial.mkdir()
            identity = json.loads((out / prebuilt_game.IDENTITY).read_text())
            for kind, names in prebuilt_game.archives(out, identity["slices"]).items():
                prebuilt_game.tar_xz(out, names, serial / f"{kind}-{identity['target']}-server.tar.xz")
            prebuilt_game.tar_xz(out, [".", *prebuilt_game.BUILD_DATA], serial / prebuilt_game.BUILD_DATA_ARCHIVE)
            with patch.object(prebuilt_game, "ProcessPoolExecutor", side_effect=lambda **kwargs:
                              ProcessPoolExecutor(mp_context=multiprocessing.get_context("spawn"), **kwargs)):
                self.pack(out, parallel)
            for archive in serial.iterdir():
                self.assertEqual(archive.read_bytes(), (parallel / archive.name).read_bytes(), archive.name)

    def test_worker_failure_names_archive_and_stops_surface_export(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = self.out(root, 1000)
            dist = root / "dist"
            (dist / prebuilt_game.BUILD_DATA_ARCHIVE).mkdir(parents=True)
            with self.assertRaisesRegex(SystemExit, "build-data.tar.xz"):
                self.pack(out, dist)
            self.assertFalse((dist / "surface.json").exists())

    def test_check_refuses_slices_naming_absent_or_shared_files(self):
        api = [*self.API, *prebuilt_game.API_BUILD_DATA, prebuilt_game.NATIVE_MODS_TEMPLATE]
        for change, why in (({"api": [*api, "deps/libgone-1.rmeta"]}, "absent files"),
                            ({"api": [*api, "deps/libsunrise_mod_host.so"]}, "overlap"),
                            # Each slice holds the members the required inputs name (so neither is empty).
                            ({"api": []}, "lack required members"),
                            ({"runtime": [prebuilt_game.IDENTITY, *self.RUNTIME[:2]]}, "lack required members"),
                            ({"api": api[1:]}, "lack required members"),
                            ({"api": api[:-1]}, "lack required members")):
            with tempfile.TemporaryDirectory() as directory, self.subTest(change), \
                    patch.object(sys, "platform", "linux"):
                out = self.out(Path(directory), 1000)
                identity = json.loads((out / prebuilt_game.IDENTITY).read_text())
                identity["slices"].update(change)
                (out / prebuilt_game.IDENTITY).write_text(json.dumps(identity))
                with self.assertRaisesRegex(SystemExit, why):
                    prebuilt_game.check(out)
        # The API executable carries the holder marker in its own bytes, or the launcher plays it the wrong way.
        with tempfile.TemporaryDirectory() as directory, patch.object(sys, "platform", "linux"):
            out = self.out(Path(directory), 1000)
            prebuilt_game.check(out)
            (out / prebuilt_game.RUNTIME / prebuilt_game.EXECUTABLE).write_bytes(b"an executable")
            with self.assertRaisesRegex(SystemExit, "holder marker"):
                prebuilt_game.check(out)

    def test_slices_come_from_the_build_records_with_proc_macros_as_leaves(self):
        def package(name, kind="lib", deps=()):
            return ({"id": name, "name": name, "targets": [{"kind": [kind]}]},
                    {"id": name, "deps": [{"pkg": dep, "dep_kinds": [{"kind": dep_kind}]} for dep, dep_kind in deps]})
        packages, nodes = zip(
            package("sunrise-mod-api", deps=[("vocab", None), ("macros", None), ("tool", "build")]),
            package("vocab", deps=[("serde", None)]),
            package("serde"),
            package("macros", "proc-macro", deps=[("syn", None)]),
            package("syn"),
            package("tool"),
            package("sunrise-mod-host", "dylib", deps=[("sunrise-mod-api", None), ("engine", None)]),
            package("sunrise-official", "dylib"),
            package("engine"),
        )
        metadata = {"packages": list(packages), "resolve": {"nodes": list(nodes)}}
        self.assertEqual(prebuilt_game.api_closure(metadata),
                         {"sunrise-mod-api", "vocab", "serde", "macros"})
        copied = {name: [f"deps/lib{name}-1.rmeta"] for name in ("sunrise-mod-api", "vocab", "serde",
                                                                  "syn", "tool", "engine")}
        copied["macros"] = ["host/libmacros-1.so"]
        copied["sunrise-mod-host"] = ["deps/libsunrise_mod_host.so"]
        copied["sunrise-official"] = ["deps/libsunrise_official.so"]
        sliced = prebuilt_game.slices(metadata, copied, ["bin/sunrise-client"])
        self.assertEqual(sliced["runtime"], sorted([prebuilt_game.IDENTITY, "bin/sunrise-client",
                                                    "deps/libsunrise_mod_host.so", "deps/libsunrise_official.so"]))
        self.assertEqual(sliced["api"], sorted([
            "deps/libsunrise-mod-api-1.rmeta", "deps/libvocab-1.rmeta",
            "deps/libserde-1.rmeta", "host/libmacros-1.so", *prebuilt_game.API_BUILD_DATA,
            prebuilt_game.NATIVE_MODS_TEMPLATE]))
        # Each slice unpacks to its own folder, so a game library inside the API closure is refused.
        nodes[1]["deps"].append({"pkg": "sunrise-official", "dep_kinds": [{"kind": None}]})
        with self.assertRaisesRegex(SystemExit, "sunrise-official"):
            prebuilt_game.slices(metadata, copied, ["bin/sunrise-client"])

    def test_fixed_client_links_and_installs_official_layer(self):
        for platform, host, expected_link_args in (
            ("linux", "x86_64-unknown-linux-gnu", ["link-arg=-Wl,-rpath,$ORIGIN"]),
            ("darwin", "aarch64-apple-darwin", ["link-arg=-Wl,-rpath,@executable_path"]),
            ("win32", "aarch64-pc-windows-msvc", ["link-arg=/STACK:8388608"]),
            ("win32", "x86_64-pc-windows-gnu", ["link-arg=-Wl,--stack,8388608"]),
        ):
            with self.subTest(host=host), tempfile.TemporaryDirectory() as directory, \
                    patch.object(sys, "platform", platform), patch.object(prebuilt_game, "run") as rustc:
                out = Path(directory)
                deps = out / "deps"
                deps.mkdir()
                for crate in ("sunrise_client", "mod_api", "sunrise_mod_package"):
                    (deps / f"lib{crate}.rmeta").touch()
                prefix, suffix = prebuilt_game.dylib_prefix(), prebuilt_game.dylib_suffix()
                (deps / f"{prefix}sunrise_mod_host{suffix}").touch()
                official = deps / f"{prefix}sunrise_official{suffix}"
                official.touch()
                files = prebuilt_game.api_route(out, host)
                command = rustc.call_args.args[0]
                self.assertIn('feature="official"', command)
                self.assertIn(f"sunrise_official={official}", command)
                self.assertEqual([arg for arg in command if arg.startswith("link-arg=")], expected_link_args)
                metadata = {"packages": [{"id": "official", "name": "sunrise-official"}], "resolve": {"nodes": []}}
                compiled = official.relative_to(out).as_posix()
                self.assertIn(compiled, prebuilt_game.slices(metadata, {"official": [compiled]}, files)["runtime"])

    def test_a_server_leg_packs_the_tool_only_when_asked(self):
        for with_tool in (False, True):
            with self.subTest(with_tool=with_tool), tempfile.TemporaryDirectory() as directory, \
                    patch.object(sys, "platform", "linux"):
                root = Path(directory)
                out = self.out(root / "server", 1000)
                identity = json.loads((out / prebuilt_game.IDENTITY).read_text())
                identity["side"] = "server"
                (out / prebuilt_game.IDENTITY).write_text(json.dumps(identity))
                self.pack(out, root / "dist", with_tool=with_tool)
                names = {path.name for path in (root / "dist").iterdir()}
                self.assertEqual("tool-x86_64-unknown-linux-gnu.tar.xz" in names, with_tool)

    def test_gather_takes_every_legs_archives_once_and_one_build_data_that_all_copies_match(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(sys, "platform", "linux"):
            root = Path(directory)
            legs = root / "legs"
            for side in ("server", "client"):
                out = self.out(root / side, 1000)
                identity = json.loads((out / prebuilt_game.IDENTITY).read_text())
                identity["side"] = side
                (out / prebuilt_game.IDENTITY).write_text(json.dumps(identity))
                self.pack(out, legs / f"sdk-linux-{side}")
            prebuilt_game.gather(legs, root / "dist")
            # The client leg alone packs the target's tool: it serves both sides.
            self.assertEqual(sorted(path.name for path in (root / "dist").iterdir()), sorted(
                [f"{kind}-x86_64-unknown-linux-gnu{suffix}.tar.xz" for kind in ("api", "engine", "runtime")
                 for suffix in ("", "-server")] + ["build-data.tar.xz", "surface.json",
                                                   "tool-x86_64-unknown-linux-gnu.tar.xz"]))
            with tarfile.open(root / "dist" / "tool-x86_64-unknown-linux-gnu.tar.xz") as tar:
                self.assertEqual(tar.getnames(), [".", "./" + prebuilt_game.TOOL_PROGRAM])

            def refused(why, change):
                broken = root / "broken"
                shutil.rmtree(broken, ignore_errors=True)
                shutil.copytree(legs, broken)
                change(broken / "sdk-linux-server")
                with self.assertRaisesRegex(SystemExit, why):
                    prebuilt_game.gather(broken, root / "broken-dist")
                shutil.rmtree(root / "broken-dist", ignore_errors=True)

            def repack(leg, archive, name, text):
                # The archive again, with one of its files' text changed.
                folder = root / "repack"
                shutil.rmtree(folder, ignore_errors=True)
                folder.mkdir()
                with tarfile.open(leg / archive, "r:xz") as tar:
                    tar.extractall(folder, filter="data")
                (folder / name).write_text(text)
                prebuilt_game.tar_xz(folder, prebuilt_game.tree(folder), leg / archive)

            refused("surface.json differs", lambda leg: (leg / "surface.json").write_text('{"hash": "cd"}'))
            refused("internals.json differ",
                    lambda leg: repack(leg, "build-data.tar.xz", prebuilt_game.INTERNALS, "{}"))
            # The package's own copy, in the api slice, is checked too.
            refused("api-x86_64-unknown-linux-gnu-server.tar.xz: loops.json differ",
                    lambda leg: repack(leg, "api-x86_64-unknown-linux-gnu-server.tar.xz", prebuilt_game.LOOPS, "{}"))
            refused("comes from two legs", lambda leg: shutil.copy2(
                legs / "sdk-linux-client/engine-x86_64-unknown-linux-gnu.tar.xz", leg))
            refused("no build-data.tar.xz", lambda leg: (leg / "build-data.tar.xz").unlink())

    def test_pack_refuses_another_commits_package(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            out = self.out(root, 1000)
            with patch.object(prebuilt_game, "head", return_value="d" * 40), \
                    self.assertRaisesRegex(SystemExit, "not this source's package"):
                prebuilt_game.pack(out, root / "dist")
            self.assertFalse((root / "dist").exists())

    def test_the_rust_reader_fixture_is_what_pack_writes(self):
        # crates/sunrise-mod-package/src/sdk/fixture.tar.xz is unpacked by the Rust reader's test: the two agree.
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "deps").mkdir()
            (root / "deps/a.rmeta").write_text("metadata")
            for path in (root / "deps", root / "deps/a.rmeta"):
                os.utime(path, (0, 0))
            prebuilt_game.tar_xz(root, prebuilt_game.tree(root), root / "fixture.tar.xz")
            fixture = prebuilt_game.ROOT / "crates/sunrise-mod-package/src/sdk/fixture.tar.xz"
            self.assertEqual((root / "fixture.tar.xz").read_bytes(), fixture.read_bytes())


class ToolBuildTests(unittest.TestCase):
    def built_with(self, environment: dict, binary: bytes = b"tool") -> dict:
        calls = []
        with tempfile.TemporaryDirectory() as directory, \
                patch.dict(os.environ, {"CARGO_TARGET_DIR": directory}), \
                patch.object(prebuilt_game, "rustc_host", return_value="x86_64-unknown-linux-gnu"), \
                patch.object(prebuilt_game, "head", return_value=COMMIT), \
                patch.object(prebuilt_game, "run", side_effect=lambda *args, **kwargs: calls.append(kwargs)):
            os.environ.pop("SUNRISE_TOOL_COMMIT", None)
            os.environ.update(environment)
            release = Path(directory) / "release"
            release.mkdir()
            (release / prebuilt_game.TOOL_PROGRAM).write_bytes(binary)
            prebuilt_game.tool("x86_64-unknown-linux-gnu")
        return calls[0]["env"]

    def test_the_tool_is_built_naming_the_sources_commit_unless_the_workflow_names_one(self):
        self.assertEqual(self.built_with({})["SUNRISE_TOOL_COMMIT"], COMMIT)
        self.assertEqual(self.built_with({"SUNRISE_TOOL_COMMIT": "d" * 40})["SUNRISE_TOOL_COMMIT"], "d" * 40)

    def test_a_tool_that_reads_the_developer_sdk_key_is_never_packed(self):
        # The release build of the real tool holds no such name; one built with debug assertions does.
        with self.assertRaisesRegex(SystemExit, "built with debug assertions"):
            self.built_with({}, b"\0{SUNRISE_SDK_TRUSTED_KEY} ignored: \0")


class TrackedBlobsTests(unittest.TestCase):
    def test_bytes_are_the_commits_whatever_the_line_ending_settings(self):
        # Git for Windows' default (core.autocrlf=true) made the Windows leg's function index differ from the others.
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            git = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-C", folder]
            subprocess.run([*git, "init", "-q"], check=True)
            (root / "a.rs").write_bytes(b"fn a() {}\nfn b() {}\n")
            subprocess.run([*git, "add", "a.rs"], check=True)
            subprocess.run([*git, "commit", "-q", "-m", "a"], check=True)
            subprocess.run([*git, "config", "core.autocrlf", "true"], check=True)
            blobs = dict(prebuilt_game.tracked_blobs(root, "HEAD"))
        self.assertEqual(blobs, {"a.rs": b"fn a() {}\nfn b() {}\n"})


class AllocatorSelectionTests(unittest.TestCase):
    def test_package_format_comes_from_its_one_place(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            number = root / "crates/sunrise-mod-package/src/sdk/package-format"
            number.parent.mkdir(parents=True)
            for good in ("7\n", "7\r\n", "7"):
                number.write_bytes(good.encode())
                self.assertEqual(prebuilt_game.package_format(root), 7, repr(good))
            for broken in ("", "\n", "v7\n", "7\n8\n"):
                number.write_text(broken)
                with self.subTest(broken), self.assertRaisesRegex(ValueError, "one number"):
                    prebuilt_game.package_format(root)
        # The repository's own number is the one the archive gate compares against.
        self.assertEqual(prebuilt_game.package_format(prebuilt_game.ROOT), 6)

    def test_windows_and_macos_archives_use_the_system_allocator(self):
        for platform in ("win32", "darwin"):
            self.assertEqual(prebuilt_game.cargo_features(False, platform), ["--features", "system-allocator"])
            self.assertEqual(prebuilt_game.cargo_features(True, platform),
                             ["--no-default-features", "--features", "system-allocator"])

    def test_sdk_build_refuses_an_output_that_overlaps_its_target_folder(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "target" / "prebuilt"
            with patch.dict(os.environ, CARGO_TARGET_DIR=str(target.parent)), \
                    patch.object(prebuilt_game, "_build_locked") as build_locked:
                for out in (root, target, target / "stage"):
                    with self.subTest(out=out), self.assertRaisesRegex(SystemExit, "E_SDK_OUT_OVERLAPS_TARGET"):
                        prebuilt_game.build(out, server=False)
                build_locked.assert_not_called()
            self.assertFalse(target.exists())

    def test_sdk_build_preserves_a_stage_under_a_linked_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "sdk"
            work = output / "work"
            stage = work / "stage"
            stage.mkdir(parents=True)
            source = stage / "keep.rs"
            source.write_text("// staged source\n")
            target = root / "target" / "prebuilt"
            target.parent.mkdir()
            try:
                target.symlink_to(work, target_is_directory=True)
            except OSError:
                self.skipTest("directory symlinks unavailable")
            with patch.dict(os.environ, CARGO_TARGET_DIR=str(target.parent)), \
                    patch.object(prebuilt_game, "_build_locked") as build_locked:
                with self.assertRaisesRegex(SystemExit, "E_SDK_OUT_OVERLAPS_TARGET"):
                    prebuilt_game.build(output, server=False)
                build_locked.assert_not_called()
            self.assertEqual(source.read_text(), "// staged source\n")
            self.assertFalse((work / ".sdk-build.lock").exists())

    def test_linux_keeps_mimalloc(self):
        self.assertEqual(prebuilt_game.cargo_features(False, "linux"), [])
        self.assertEqual(prebuilt_game.cargo_features(True, "linux"), ["--no-default-features"])

    def test_build_uses_explicit_host_target_and_target_dynamic_flags(self):
        class Captured(Exception):
            pass

        def capture(command, **kwargs):
            if command[:2] == ["cargo", "metadata"]:
                return json.dumps({"packages": [], "resolve": {"nodes": []}})
            self.assertEqual(command[command.index("--target") + 1], "fixture-host")
            self.assertEqual(kwargs["env"]["SUNRISE_GAME_BUILD_ID"], "fixture-commit")
            self.assertIn("-C prefer-dynamic", kwargs["env"]["RUSTFLAGS"])
            self.assertNotIn("CARGO_ENCODED_RUSTFLAGS", kwargs["env"])
            self.assertEqual(kwargs["env"]["CARGO_PROFILE_RELEASE_CODEGEN_UNITS"], "1")
            raise Captured

        with tempfile.TemporaryDirectory() as target, \
             patch.object(prebuilt_game, "stage", return_value=Path("staged")), \
             patch.object(prebuilt_game, "head", return_value="fixture-commit"), \
             patch.object(prebuilt_game, "rustc_host", return_value="fixture-host"), \
             patch.object(prebuilt_game, "run", side_effect=capture), \
             patch.object(prebuilt_game, "os", ScopedOs({
                 "CARGO_ENCODED_RUSTFLAGS": "-Cprefer-dynamic=no",
                 # An inherited unit count must not reach the SDK game.
                 "CARGO_PROFILE_RELEASE_CODEGEN_UNITS": "16",
                 "CARGO_TARGET_DIR": target})):
            # Its own target folder: the build takes that folder's lock, never the checkout's.
            with self.assertRaises(Captured):
                prebuilt_game.build(Path(target) / "unused-sdk", False)


class ScopedOs:
    """`os` for the module under test, with its own environment: a fixture never writes the process's."""

    def __init__(self, environ: dict[str, str]):
        self.environ = environ

    def __getattr__(self, name):
        return getattr(os, name)


class PrebuiltContentsTests(unittest.TestCase):
    def folder(self, *names: str) -> Path:
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        for name in names:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_bytes(b"")
        return root

    def test_only_target_artifacts_and_host_proc_macros_enter_the_archive(self):
        root = self.folder(
            "cache/release/deps/libshared-host.rmeta",
            "cache/release/deps/libderive.so",
            "cache/host/release/deps/libshared-target.rmeta",
            "cache/host/release/deps/libshared-second-version.rmeta",
            "cache/host/release/deps/libshared-target.rlib",
            "cache/host/release/deps/libsunrise_client.so",
        )
        output, macros = root / "sdk/deps", root / "sdk/host"
        output.mkdir(parents=True)
        macros.mkdir()
        def artifact(path, kind):
            return {"reason": "compiler-artifact", "target": {"kind": [kind]}, "package_id": Path(path).name,
                    "filenames": [str(root / path)]}
        copied = prebuilt_game.copy_artifacts([
            artifact("cache/release/deps/libshared-host.rmeta", "lib"),
            artifact("cache/release/deps/libderive.so", "proc-macro"),
            artifact("cache/host/release/deps/libshared-target.rmeta", "lib"),
            artifact("cache/host/release/deps/libshared-second-version.rmeta", "lib"),
            artifact("cache/host/release/deps/libshared-target.rlib", "lib"),
            artifact("cache/host/release/deps/libsunrise_client.so", "dylib"),
            {"reason": "build-finished", "success": True},
        ], output, macros, root / "cache/host")
        self.assertEqual({path.name for path in output.iterdir()}, {
            "libshared-target.rmeta", "libshared-second-version.rmeta", "libsunrise_client.so",
        })
        self.assertEqual({path.name for path in macros.iterdir()}, {"libderive.so"})
        # Each copied file is recorded under its package, as the slices read them.
        self.assertEqual(copied, {"libderive.so": ["host/libderive.so"],
                                  "libshared-target.rmeta": ["deps/libshared-target.rmeta"],
                                  "libshared-second-version.rmeta": ["deps/libshared-second-version.rmeta"],
                                  "libsunrise_client.so": ["deps/libsunrise_client.so"]})
        # Multiple legitimate target versions remain available to rustc's transitive lookup.
        # Only explicit extern/facade lookup must reject ambiguous names.
        with self.assertRaises(ValueError):
            prebuilt_game.extern_args(output, "shared")

    def test_dylib_metadata_is_collected_from_the_rlib_sidecar(self):
        root = self.folder("cache/host/release/deps/libmod_api.rlib",
                           "cache/host/release/deps/libmod_api.rmeta",
                           "cache/host/release/libmod_api.so")
        deps, host = root / "sdk/deps", root / "sdk/host"
        deps.mkdir(parents=True)
        host.mkdir()
        copied = prebuilt_game.copy_artifacts([{
            "reason": "compiler-artifact", "target": {"kind": ["lib"]}, "package_id": "api",
            "filenames": [str(root / "cache/host/release/deps/libmod_api.rlib"),
                          str(root / "cache/host/release/libmod_api.so")],
        }], deps, host, root / "cache/host")
        self.assertEqual(set(copied["api"]), {"deps/libmod_api.so", "deps/libmod_api.rmeta"})

    def test_facades_exclude_build_only_and_ambiguous_runtime_crates(self):
        def package(name, kind="lib"):
            return {"id": name, "name": name, "targets": [{"name": name.replace('-', '_'), "kind": [kind]}]}

        def node(name, *dependencies):
            return {"id": name, "deps": [
                {"pkg": target, "dep_kinds": [{"kind": kind}]} for target, kind in dependencies
            ]}

        metadata = {
            "packages": [package("sunrise-mod-host", "dylib"), package("sunrise-client"),
                         package("sunrise-items", "rlib"), package("mod_vocab"), package("sunrise_core"),
                         package("build_tool"), package("derive", "proc-macro"), package("macro_helper")],
            "resolve": {"nodes": [
                node("sunrise-mod-host", ("sunrise-client", None), ("sunrise-items", None),
                     ("build_tool", "build")), node("sunrise-items"),
                node("sunrise-client", ("mod_vocab", None), ("sunrise_core", None), ("derive", None)),
                node("mod_vocab"), node("sunrise_core"), node("build_tool"),
                node("derive", ("macro_helper", None)), node("macro_helper"),
            ]},
        }
        runtime = prebuilt_game.runtime_crates(metadata, "sunrise-mod-host")
        self.assertEqual(runtime, {"sunrise_mod_host", "sunrise_items", "sunrise_client", "mod_vocab", "sunrise_core"})
        deps = self.folder("libmod_vocab-host.rmeta", "libmod_vocab-target.rmeta",
                           "libsunrise_core-one.rmeta", "libbuild_tool-one.rmeta", "libmacro_helper-one.rmeta",
                           "libsunrise_mod_host.so", "libsunrise_items-one.rmeta", "libsunrise_client-one.rmeta")
        eligible = prebuilt_game.facade_crates(deps, runtime)
        self.assertEqual(eligible, ["sunrise_client", "sunrise_core", "sunrise_items", "sunrise_mod_host"])

    def test_native_link_inventory_uses_package_identity_not_a_same_named_host_variant(self):
        def package(package_id, version, library=None):
            return {"id": package_id, "name": "native-sys" if library else "sunrise-mod-host",
                    "version": version, "links": library,
                    "targets": [{"name": "native_sys" if library else "sunrise_client", "kind": ["lib"]}]}
        metadata = {
            "packages": [package("game", "1.0.0"), package("target", "1.0.0", "native"),
                         package("host", "2.0.0", "native")],
            "resolve": {"nodes": [
                {"id": "game", "deps": [{"pkg": "target", "dep_kinds": [{"kind": None}]},
                                          {"pkg": "host", "dep_kinds": [{"kind": "build"}]}]},
                {"id": "target", "deps": []}, {"id": "host", "deps": []},
            ]},
        }
        self.assertEqual(prebuilt_game.native_links(metadata, "sunrise-mod-host"), [
            {"name": "native-sys", "version": "1.0.0", "library": "native"},
        ])

    def test_sdk_assertion_refuses_a_duplicate_eligible_facade_on_every_os(self):
        root = self.folder("deps/libsunrise_core-one.rmeta", "deps/libmod_api-one.rmeta",
                           "deps/libsunrise_mod_package-one.rmeta")
        prebuilt_game.assert_externs(root, ["sunrise_core"])
        (root / "deps/libsunrise_core-two.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "expected exactly one SDK metadata for sunrise_core"):
            prebuilt_game.assert_externs(root, ["sunrise_core"])

    def test_native_extern_lookup_refuses_a_second_package_variant(self):
        root = self.folder("libsunrise_mod_package-runtime.rmeta")
        self.assertEqual(prebuilt_game.extern_args(root, "sunrise_mod_package"), [
            "--extern", f"sunrise_mod_package={root / 'libsunrise_mod_package-runtime.rmeta'}",
        ])
        (root / "libsunrise_mod_package-build.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "expected exactly one SDK metadata for sunrise_mod_package"):
            prebuilt_game.extern_args(root, "sunrise_mod_package")

    def test_host_macros_pass_but_host_dependency_metadata_is_refused(self):
        root = self.folder("deps/libsunrise_client.so", "deps/libsunrise_official.so", "deps/libsunrise_mod_host.so",
                           "host/libderive.so", "host/derive.dll", "host/libderive.dylib")
        write_build_data(root)
        prebuilt_game.check(root)
        (root / "host/libshared-host.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(SystemExit, "aren't compiler outputs"):
            prebuilt_game.check(root)

    def test_unrelated_multiversion_dependencies_do_not_break_named_externs(self):
        root = self.folder("deps/libmod_api-one.rmeta", "deps/libsunrise_mod_package-one.rmeta",
                           "deps/libshared-v1.rmeta", "deps/libshared-v2.rmeta",
                           "host/libmod_api-host.so")
        prebuilt_game.assert_externs(root, [])

    def test_compiler_outputs_pass(self):
        root = self.folder(
            "sunrise-prebuilt.json", "Cargo.lock", "native_main.rs.template", "deps/libsunrise_client.so", "deps/libmod_api-1.rmeta",
            "native/libwindows.0.52.0.a",
            "deps/libserde_derive-2.so", "deps/libsunrise_client.dll.a", "deps/libsunrise_official.so",
            "deps/sunrise_official.dll", "deps/libsunrise_official.dylib", "deps/libsunrise_mod_host.so",
        )
        write_build_data(root)
        prebuilt_game.check(root)

    def test_build_data_without_its_header_for_this_commit_is_refused(self):
        root = self.folder("deps/libsunrise_mod_host.so", "deps/libsunrise_official.so")
        for name in prebuilt_game.BUILD_DATA:
            for broken in ('{"build": {"commit": "%s"}}' % COMMIT, '{"schema": 2, "build": {"commit": "%s"}}' % COMMIT,
                           '{"schema": 1, "build": {"commit": "%s"}}' % ("d" * 40), '{"schema": 1}', "[]", None):
                with self.subTest(name=name, broken=broken):
                    write_build_data(root)
                    if broken is None:
                        (root / name).unlink()
                    else:
                        (root / name).write_text(broken)
                    with self.assertRaisesRegex(SystemExit, name):
                        prebuilt_game.check(root)

    def test_anything_else_is_refused(self):
        for stray in ("assets/minecraft/lang/en_us.json", "deps/Cargo.lock", "deps/client.jar", "deps/libmod_api-3.rlib", "native/libfoo.so", "deps/internals.json", "notes.txt"):
            with self.subTest(stray), self.assertRaises(SystemExit):
                prebuilt_game.check(self.folder("deps/libsunrise_client.so", "deps/libsunrise_official.so", stray))

    def test_host_carries_engine_code_with_engine_metadata_for_mods(self):
        root = self.folder("deps/libsunrise_mod_host.so", "deps/libsunrise_official.so", "deps/libsunrise_client-123.rmeta")
        write_build_data(root)
        prebuilt_game.check(root)

    def test_engine_without_host_is_refused(self):
        with self.assertRaisesRegex(SystemExit, "sunrise-mod-host"):
            prebuilt_game.check(self.folder("deps/libsunrise_client.so", "deps/libsunrise_official.so"))

    def dll(self, root: Path, name: str, count: int, pe64: bool = True) -> None:
        # One PE section: RVA 0x1000 maps to raw offset 0x200, holding the export directory.
        data = bytearray(0x400)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 0x80)
        data[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<H", data, 0x86, 1)  # NumberOfSections
        optional_size = 240 if pe64 else 224
        struct.pack_into("<H", data, 0x94, optional_size)
        struct.pack_into("<H", data, 0x98, 0x20B if pe64 else 0x10B)
        struct.pack_into("<II", data, 0x98 + (112 if pe64 else 96), 0x1000, 40)
        struct.pack_into("<III", data, 0x98 + optional_size + 12, 0x1000, 0x200, 0x200)
        struct.pack_into("<I", data, 0x200 + 24, count)  # NumberOfNames
        (root / "deps" / name).write_bytes(data)

    def test_exports_reports_every_dll(self):
        root = self.folder("deps/libsunrise_client.so")
        self.dll(root, "sunrise_client.dll", 45000)
        self.dll(root, "sunrise_official.dll", 12000)
        self.dll(root, "serde_derive.dll", 1, pe64=False)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            prebuilt_game.exports(root)
        for name, count in (("sunrise_client", 45000), ("sunrise_official", 12000), ("serde_derive", 1)):
            self.assertIn(f"{name}.dll: {count} exported names", output.getvalue())

    def test_exports_warns_past_80_percent_without_failing(self):
        for large in ("sunrise_client", "sunrise_official"):
            root = self.folder("deps/libsunrise_client.so")
            for name in ("sunrise_client", "sunrise_official"):
                self.dll(root, f"{name}.dll", 52429 if name == large else 52428)
            output = io.StringIO()
            with self.subTest(large), contextlib.redirect_stdout(output):
                prebuilt_game.exports(root)
            warnings = [line for line in output.getvalue().splitlines() if line.startswith("::warning::")]
            self.assertEqual(len(warnings), 1)
            self.assertIn(f"{large}.dll exports 52,429 names", warnings[0])
            self.assertEqual(output.getvalue().count("exported names (Windows' limit"), 2)

    def test_exports_fails_past_90_percent_after_reporting_every_dll(self):
        root = self.folder("deps/libsunrise_client.so")
        self.dll(root, "sunrise_client.dll", 58982)
        self.dll(root, "sunrise_official.dll", 58981)
        self.dll(root, "mod_api.dll", 171)
        output = io.StringIO()
        with contextlib.redirect_stdout(output), self.assertRaises(SystemExit) as failed:
            prebuilt_game.exports(root)
        self.assertIn("sunrise_client.dll (58,982)", str(failed.exception.code))
        self.assertNotIn("sunrise_official", str(failed.exception.code))
        self.assertEqual(output.getvalue().count("exported names (Windows' limit"), 3)

    def test_exports_is_silent_without_dlls(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            prebuilt_game.exports(self.folder("deps/libsunrise_client.so", "deps/libsunrise_official.so"))
        self.assertEqual(output.getvalue(), "")


if __name__ == "__main__":
    unittest.main()


class LauncherPinTests(unittest.TestCase):
    def test_only_a_full_lowercase_sha_is_accepted_by_the_script_and_the_smoke(self):
        script = prebuilt_game.ROOT / "tools/ci/launcher_pin.sh"
        pin = prebuilt_game.ROOT / "tools/ci/launcher-pin"
        original = pin.read_text()
        try:
            for value, accepted in (("a" * 40, True), ("A" * 40, False), ("a" * 39, False), ("main", False), ("a" * 41, False)):
                pin.write_text(value + "\n")
                shell = subprocess.run([str(script)], capture_output=True, text=True)
                self.assertEqual(shell.returncode == 0, accepted, value)
                if accepted:
                    self.assertEqual(prebuilt_game.launcher_pin(), value)
                else:
                    with self.assertRaises(SystemExit):
                        prebuilt_game.launcher_pin()
        finally:
            pin.write_text(original)
        self.assertEqual(subprocess.run([str(script)], capture_output=True, text=True).stdout.strip(), original.strip())


class LauncherCheckoutTests(unittest.TestCase):
    def test_checks_out_the_pinned_commit_detached_and_refuses_an_unfetchable_pin(self):
        script = prebuilt_game.ROOT / "tools/ci/launcher_checkout.sh"
        pin_file = prebuilt_game.ROOT / "tools/ci/launcher-pin"
        original = pin_file.read_text()
        git = ["git", "-c", "user.name=t", "-c", "user.email=t@example.com"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            subprocess.run(git + ["init", "-q", str(source)], check=True)
            commits = []
            for name in ("one", "two"):
                (source / "file").write_text(name)
                subprocess.run(git + ["-C", str(source), "add", "file"], check=True)
                subprocess.run(git + ["-C", str(source), "commit", "-q", "-m", name], check=True)
                commits.append(subprocess.run(["git", "-C", str(source), "rev-parse", "HEAD"], check=True,
                                              capture_output=True, text=True).stdout.strip())
            env = {**os.environ, "LAUNCHER_URL": source.as_uri(), "GIT_CONFIG_GLOBAL": os.devnull}
            try:
                pin_file.write_text(commits[0] + "\n")
                done = subprocess.run([str(script), str(root / "out")], env=env, capture_output=True, text=True)
                self.assertEqual(done.returncode, 0, done.stderr)
                self.assertEqual((root / "out/file").read_text(), "one")
                state = subprocess.run(["git", "-C", str(root / "out"), "symbolic-ref", "-q", "HEAD"], capture_output=True)
                self.assertNotEqual(state.returncode, 0)  # detached
                pin_file.write_text("f" * 40 + "\n")
                done = subprocess.run([str(script), str(root / "out2")], env=env, capture_output=True, text=True)
                self.assertNotEqual(done.returncode, 0)
            finally:
                pin_file.write_text(original)


class SmokeStartTests(unittest.TestCase):
    def test_launcher_files_fetched_at_the_pin_keep_the_layout_that_includes_the_icon(self):
        urls = []
        with tempfile.TemporaryDirectory() as directory:
            root = prebuilt_game.fetch_launcher_files(Path(directory), "a" * 40, lambda url: urls.append(url) or b"x")
            self.assertTrue((root / "src/game_app.rs").is_file())
            self.assertTrue((root / "src/../assets/icons/sunrise.icns").is_file())
        self.assertTrue(all("/sunrise-launcher/" + "a" * 40 + "/" in url for url in urls))

    def test_start_requires_the_expected_missing_resources_exit_for_each_side(self):
        from types import SimpleNamespace
        for side, code, launcher, refusal in (
            ("client", 1, True, None),
            ("server", 1, False, None),
            ("client", 1, False, "client omitted the launcher download URL"),
            ("server", 1, True, "server showed the launcher download URL"),
            ("client", 0, True, "stopped before the prepared-files check"),
            ("client", 2, True, "stopped before the prepared-files check"),
            ("client", 3221225477, True, "stopped before the prepared-files check"),
        ):
            with self.subTest(side=side, code=code, launcher=launcher), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                out = root / "out"
                out.mkdir()
                (out / prebuilt_game.IDENTITY).write_text(json.dumps({"side": side}))
                stdout = prebuilt_game.NO_RESOURCES
                if launcher:
                    stdout += "\nSunrise is started from its launcher: download it at https://example.test (Download)."
                result = subprocess.CompletedProcess([], code, stdout=stdout)
                with patch.dict(sys.modules, {"lab": SimpleNamespace(game_env=lambda **_: {"SUNRISE_HOME": "/inherited"})}), \
                     patch.object(prebuilt_game.sys, "platform", "linux"), \
                     patch.object(prebuilt_game.subprocess, "run", return_value=result) as run, \
                     contextlib.redirect_stdout(io.StringIO()):
                    if refusal is None:
                        prebuilt_game.start(out)
                    else:
                        with self.assertRaisesRegex(SystemExit, refusal):
                            prebuilt_game.start(out)
                environment = run.call_args.kwargs["env"]
                for key in ("SUNRISE_HOME", "SUNRISE_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
                    self.assertTrue(Path(environment[key]).is_relative_to(root))
