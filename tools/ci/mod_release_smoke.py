#!/usr/bin/env python3
"""Run the generated native release steps offline on the daily run's signed Linux SDK."""

import argparse
import base64
from contextlib import contextmanager
from io import BytesIO
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from unittest.mock import patch
import zipfile
import zlib


ROOT = Path(__file__).resolve().parents[2]
HISTORICAL = ROOT / "crates/sunrise-mod-package/tests/index/stable-native"


def icon(path):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    size = 128
    header = struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)
    pixels = (b"\0" + b"\x76\x91\xab" * size) * size
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))


def linux_sdk(batch, base, work, linux, release):
    # A daily rerun may use an already-published SDK with more than the Linux legs. Project the
    # already authenticated index onto the two Linux hosts in a debug-only signed scratch copy.
    original = base / release
    dist = work / "linux-sdk" / release
    dist.mkdir(parents=True)
    index = release_api.json_read(batch.sdk / batch.plan["target"]["commit"] / "sdk.json")
    index["assets"] = {name: asset for name, asset in index["assets"].items() if asset.get("target", linux) == linux}
    for name in index["assets"]:
        (dist / name).symlink_to((original / name).resolve())
    release_api.json_write(dist / "sdk.json", index)
    release_api.call("bash", ROOT / "tools/ci/sign_release.sh", release, dist / "sdk.json",
                     cwd=ROOT, env={**os.environ, "EPHEMERAL": "yes"})
    os.environ["SUNRISE_SDK_TRUSTED_KEY"] = (dist / "ephemeral.pub").read_text().strip()
    os.environ["SUNRISE_SDK_BASE"] = str(dist.parent)


def entry(from_stage, to_checkout, id):
    for directory, name in [("mods", f"{id}.toml"), ("icons", f"{id}.png")]:
        if (from_stage / directory / name).is_file():
            (to_checkout / directory).mkdir(parents=True, exist_ok=True)
            shutil.copyfile(from_stage / directory / name, to_checkout / directory / name)
    return tomllib.loads((to_checkout / "mods" / f"{id}.toml").read_text(encoding="utf-8"))


@contextmanager
def signing_key(private):
    with patch.dict(os.environ, {release_api.KEY: (private / "signing.key").read_text(encoding="utf-8"),
                                 release_api.PASSPHRASE: ""}):
        yield


def rebuild_sdks(work, batch, old):
    """Keep the daily's real target assets; supply a signed metadata-only SDK for the historical fixture."""
    current = batch.plan["target"]["commit"]
    prior = old["build"]
    assert prior != current
    source = Path(os.environ["SUNRISE_SDK_BASE"]) / f"sdk-{current}"
    base = work / "rebuild-sdk"
    target = base / source.name
    previous = base / f"sdk-{prior}"
    target.mkdir(parents=True)
    previous.mkdir()
    # Both indices must verify under the one extra SDK key debug builds accept.
    key = subprocess.check_output(["openssl", "genpkey", "-algorithm", "ed25519", "-outform", "DER"])
    public = subprocess.run(["openssl", "pkey", "-inform", "DER", "-pubout", "-outform", "DER"],
                            input=key, capture_output=True, check=True).stdout[-32:]
    env = {name: value for name, value in os.environ.items()
           if name not in ("EPHEMERAL", "RELEASE_SIGNING_KEY_NEXT", "ALSO_FOR", "LAUNCHER_FOR")}
    env["RELEASE_SIGNING_KEY"] = base64.b64encode(key).decode("ascii")
    index = release_api.json_read(batch.sdk / current / "sdk.json")
    for name in [*index["assets"], "sdk.json"]:
        (target / name).symlink_to((source / name).resolve())
    (previous / "surface.json").symlink_to((target / "surface.json").resolve())
    index["build"]["commit"] = prior
    index["build"].pop("nightly", None)
    index["toolchain"]["rust"] = old["rust"]
    data = previous / "build-data.tar.xz"
    with tarfile.open(data, "w:xz") as archive:
        for name, contents in [("functions", {"crates": {}, "functions": {}, "refused": {}}),
                               ("internals", {"internals": []}), ("loops", {"loops": []})]:
            payload = json.dumps({"schema": 1, "build": {"commit": prior}, **contents}).encode()
            member = tarfile.TarInfo(f"{name}.json")
            member.size = len(payload)
            archive.addfile(member, BytesIO(payload))
    index["assets"][data.name].update(sha256=release_api.sha(data), size=data.stat().st_size)
    release_api.json_write(previous / "sdk.json", index)
    for folder in (target, previous):
        release_api.call("bash", ROOT / "tools/ci/sign_release.sh", folder.name, folder / "sdk.json",
                         cwd=ROOT, env=env)
    os.environ.update(SUNRISE_SDK_BASE=str(base), SUNRISE_SDK_TRUSTED_KEY=base64.b64encode(public).decode("ascii"))


def exercise_rebuild(work, project, private, initial, empty, old):
    import sunrise_release_github as github

    rebuild_sdks(work, initial, old)
    current = {field: initial.plan["target"][field] for field in ("tag", "commit")}
    repo = initial.repo
    prior = old["build"]
    game = current["tag"].split("-", 1)[0]
    previous = {"tag": f"{game}-0.1.0", "commit": prior}
    releases = work / "rebuild-releases.json"
    release_api.json_write(releases, [previous, current])

    # Give the historical signed archive fixture the same source as today's starter, with
    # just its pin and use-table build changed. Only today's SDK is used to compile.
    historical = work / "historical-project"
    historical.mkdir()
    source = historical / initial.source
    with zipfile.ZipFile(initial.assets / initial.source) as from_zip, zipfile.ZipFile(source, "w") as to_zip:
        for member in from_zip.infolist():
            data = from_zip.read(member)
            if member.filename == "sunrise-mod.toml":
                data = data.replace(current["commit"].encode(), prior.encode())
            elif member.filename == "uses.json":
                uses = json.loads(data)
                uses["made_for"] = prior
                data = json.dumps(uses).encode()
            to_zip.writestr(member, data)
            if member.filename in ("sunrise-mod.toml", "Cargo.toml", "Cargo.lock"):
                (historical / member.filename).write_bytes(data)
    shutil.copytree(project / "src", historical / "src")
    shutil.copyfile(project / "icon.png", historical / "icon.png")
    old_releases = work / "historical-releases.json"
    release_api.json_write(old_releases, [previous])
    seed = release_api.plan(work / "historical-release", historical, current["commit"], repo, initial.tag, old_releases)
    archives = {item["target"]: item["name"] for item in old["archives"]}
    for name, host in seed.hosts.items():
        folder = seed.work / "host-results" / name
        folder.mkdir(parents=True)
        shutil.copyfile(HISTORICAL / archives[name], folder / host["archive"])
        if host["source_producer"]:
            shutil.copyfile(source, folder / seed.source)
    with signing_key(private):
        release_api.sign(seed, historical)
    published = work / "historical-index.json"
    listed = work / "reviewed"
    release_api.stage(seed, historical, empty, listed)
    release_api.tool("registry", "build", listed, "--files", listed / "files", "--out", published,
                     "--releases", seed.releases, "--sdk-data", seed.sdk)
    original = (listed / "mods" / f"{seed.id}.toml").read_bytes()
    source_bytes = (seed.assets / seed.source).read_bytes()
    published_version = release_api.json_read(published)["projects"][0]["versions"][0]
    assert {(b["build"], b["target"]) for b in published_version["native_binaries"]} == {
        (prior, name) for name in seed.hosts
    }

    # Replace the network reads, not the rebuild planner, verification, or build steps.
    def fixture_registry(checkout, index):
        entry(listed, checkout, seed.id)
        shutil.copyfile(published, index)

    def fixture_download(url, dest, size):
        assert url == f"https://github.com/{repo}/releases/download/{initial.tag}/{initial.source}"
        assert size == len(source_bytes)
        dest.write_bytes(source_bytes)

    tool = release_api.tool

    def fixture_tool(*args, **kwargs):
        if args[:3] == ("registry", "releases", "--out"):
            shutil.copyfile(releases, args[3])
            return
        return tool(*args, **kwargs)

    directory = work / "rebuild"
    with (patch.object(github, "registry", fixture_registry), patch.object(github, "download", fixture_download),
          patch.object(release_api, "tool", fixture_tool)):
        github.rebuild_plan(directory, project, current["commit"], repo, initial.plan["version"], current["tag"])
    batch = release_api.Batch(directory)
    assert not batch.initial and batch.plan["source"]["commit"] == prior
    assert batch.plan["target"]["commit"] == current["commit"] and set(batch.hosts) == set(initial.hosts)
    for name in batch.hosts:
        release_api.build(batch, None, name)
        built = batch.work / "build-project"
        assert tomllib.loads((built / "sunrise-mod.toml").read_text(encoding="utf-8"))["native"]["sunrise"] == current["commit"]
        assert (built / "Cargo.lock").read_bytes() == (historical / "Cargo.lock").read_bytes()
        shutil.rmtree(built)  # each hosted matrix leg gets its own clean project
    with signing_key(private):
        release_api.sign(batch, None)
    after = work / "append-rebuild"
    release_api.stage(batch, None, listed, after, published=published)
    amended = after / "mods" / f"{batch.id}.toml"
    before_new_build, marker, _ = amended.read_bytes().rpartition(b"\n[[versions.builds]]\n")
    assert marker and before_new_build.rstrip(b"\n") == original.rstrip(b"\n"), "source or existing builds changed"
    previous_rows = tomllib.loads(original.decode())["versions"][0]["builds"]
    builds = tomllib.loads(amended.read_text(encoding="utf-8"))["versions"][0]["builds"]
    assert builds[:-1] == previous_rows and builds[-1]["sunrise"] == current["tag"]
    assert batch.reviewed.read_bytes() == source_bytes
    assert all(not (after / name).exists() for name in (batch.source, f"{batch.source}.minisig"))
    shutil.copytree(listed / "files", after / "files", dirs_exist_ok=True)
    shutil.copytree(listed / "icons", after / "icons", dirs_exist_ok=True)
    combined = work / "rebuilt-index.json"
    release_api.tool("registry", "build", after, "--files", after / "files", "--out", combined,
                     "--releases", releases, "--sdk-data", batch.sdk)
    indexed = release_api.json_read(combined)["projects"][0]["versions"][0]["native_binaries"]
    assert {(b["build"], b["target"]) for b in indexed} == {
        (pin, name) for pin in (prior, current["commit"]) for name in batch.hosts
    }
    retry = work / "rebuild-retry"
    release_api.stage(batch, None, after, retry, published=combined)
    assert release_api.files(retry) == {"ALREADY_LISTED"}
    print("Generated rebuild: preserved reviewed bytes, appended one distinct stable build, identical retry was a no-op.")


def exercise(work, project, commit, old):
    repo, tag = f"example/{old['id']}", f"v{old['version']}"
    manifest = project / "sunrise-mod.toml"
    page = (f'homepage = "https://github.com/{repo}"\nlicense = "MIT"\n'
            'description = "A disposable native sample that checks release builds for both Linux sides."')
    manifest.write_text(manifest.read_text(encoding="utf-8").replace('description = ""', page)
                        .replace('version = "0.1.0"', f'version = "{old["version"]}"'), encoding="utf-8")
    cargo = project / "Cargo.toml"
    cargo.write_text(cargo.read_text(encoding="utf-8").replace('version = "0.1.0"', f'version = "{old["version"]}"')
                     + "\n[workspace]\n", encoding="utf-8")
    icon(project / "icon.png")
    sdk_base = Path(os.environ["SUNRISE_SDK_BASE"])
    game = release_api.json_read(sdk_base / f"sdk-{commit}" / "sdk.json")["build"]["game"]
    stable = f"{game}-0.1.1"
    releases = work / "stable.json"
    release_api.json_write(releases, [{"tag": stable, "commit": commit}])
    release_work = work / "release"
    release_api.plan(release_work, project, commit, repo, tag, releases)
    batch = release_api.Batch(release_work)
    linux = release_api.LINUX
    wanted_hosts = {linux, f"{linux}-server"}
    if set(batch.hosts) != wanted_hosts:
        linux_sdk(batch, sdk_base, work, linux, f"sdk-{commit}")
        release_work = work / "linux-release"
        release_api.plan(release_work, project, commit, repo, tag, releases)
        batch = release_api.Batch(release_work)
    release_api.require(set(batch.hosts) == wanted_hosts, "this smoke needs both Linux SDK sides")
    for host in batch.hosts:
        release_api.build(batch, project, host)

    private = work / "author-key"
    key = release_api.tool("keygen", "--out", private, "--no-password", cwd=project, capture=True).strip()
    with signing_key(private):
        release_api.sign(batch, project)
    os.environ.update(SUNRISE_MOD_PUBLIC_KEY=key, SUNRISE_MOD_REGISTRY_OWNER="example",
                      SUNRISE_MOD_SUMMARY="Native release smoke.", SUNRISE_MOD_CATEGORIES="library")

    empty = work / "empty"
    (empty / "mods").mkdir(parents=True)
    first = work / "first-listing"
    release_api.stage(batch, project, empty, first)
    seeded = work / "source-only"
    release_api.stage(batch, project, empty, seeded, source_only=True)
    before = work / "base"
    held = entry(seeded, before, batch.id)
    append = work / "append"
    release_api.tool("publish", "--out", append, "--registry", before, "--package", batch.assets / batch.source,
                     "--binaries", batch.assets, "--sdk-data", batch.sdk, "--releases", batch.releases,
                     "--asset-base", f"https://github.com/{repo}/releases/download/{tag}", cwd=project)
    shutil.copytree(seeded / "files", append / "files", dirs_exist_ok=True)
    for staged in [first, append]:
        checkout = staged / "registry"
        if staged == append:
            shutil.copytree(before, checkout)
        updated = entry(staged, checkout, batch.id)
        row = updated["versions"][0]
        builds = row.pop("builds")
        assert row == held["versions"][0], "the reviewed source row changed"
        assert len(builds) == 1 and builds[0]["sunrise"] == stable, "expected one appended build"
        assert updated["key"] == held["key"] == key
        index = staged / "index.json"
        release_api.tool("registry", "build", checkout, "--files", staged / "files", "--out", index,
                         "--releases", batch.releases, "--sdk-data", batch.sdk)
        published = release_api.json_read(index)["projects"][0]["versions"][0]["native_binaries"]
        assert {(item["build"], item["target"]) for item in published} == {(commit, host) for host in wanted_hosts}
    print("Generated release: Linux client and server, signed first listing and unchanged-source append passed.")
    exercise_rebuild(work, project, private, batch, empty, old)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tool", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    os.environ["SUNRISE_MOD_TOOL"] = str(args.tool.resolve())
    # Exercise the scripts embedded in the same tool that generated the workflow YAML.
    with tempfile.TemporaryDirectory(dir=args.scratch) as scratch:
        work = Path(scratch)
        os.environ["SUNRISE_MOD_CACHE"] = str(work / "cache")
        old = json.loads((HISTORICAL / "valid.binaries.json").read_bytes())
        project = work / old["id"]
        subprocess.run([os.environ["SUNRISE_MOD_TOOL"], "new", project, "--template", "native", "--sunrise", args.commit], check=True)
        scripts = work / "scripts"
        subprocess.run([os.environ["SUNRISE_MOD_TOOL"], "release", "scripts", "--out", scripts], check=True)
        sys.dont_write_bytecode = True
        sys.path.insert(0, str(scripts))
        global release_api
        import sunrise_release as release_api
        exercise(work, project, args.commit, old)


if __name__ == "__main__":
    main()
