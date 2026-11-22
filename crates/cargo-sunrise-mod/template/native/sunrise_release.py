#!/usr/bin/env python3
"""Build, sign and stage a native mod release from the tool's authenticated plan."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib


REGISTRY = "sunrise-org/sunrise-mods"
LINUX = "x86_64-unknown-linux-gnu"
KEY = "SUNRISE_MOD_SIGNING_KEY"
PASSPHRASE = "SUNRISE_MOD_SIGNING_PASSPHRASE"
SHA = re.compile(r"[0-9a-f]{40}\Z")
PLAIN = re.compile(r"[a-zA-Z0-9_][a-zA-Z0-9_+.-]{0,127}\Z")
# Match the registry's valid_repo, including its exclusion of dot-only path components of length one or two.
REPO = re.compile(r"(?!\.{1,2}/)[a-zA-Z0-9_.-]{1,39}/(?!\.{1,2}\Z)[a-zA-Z0-9_.-]{1,100}\Z")


def require(condition, message):
    if not condition:
        raise SystemExit(message)


def call(*command, cwd=None, env=None, capture=False):
    return subprocess.run(command, cwd=cwd, env=env, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None).stdout


def tool(*args, **kwargs):
    return call(os.environ.get("SUNRISE_MOD_TOOL", "cargo-sunrise-mod"), *args, **kwargs)


def sha(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def json_read(path):
    return json.loads(path.read_text(encoding="utf-8"))


def json_write(path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def files(directory):
    entries = list(directory.iterdir())
    require(all(file.is_file() and not file.is_symlink() for file in entries), f"{directory}: only regular files are allowed")
    return {file.name for file in entries}


def read_plan(path):
    envelope = json_read(path)
    schema = envelope.get("schema")
    if type(schema) is not int or schema != 1:
        commit = envelope.get("tool_commit", "unknown")
        raise SystemExit(f"unsupported plan.json schema {schema!r} from tool commit {commit}; expected schema 1")
    return envelope


class Batch:
    """The plan artifact is written by a job that never runs mod code."""

    def __init__(self, work):
        self.work = work.resolve()
        self.shared = self.work / "plan"
        self.sdk = self.shared / "sdk-data"
        self.releases = self.shared / "releases.json"
        path = self.shared / "plan.json"
        envelope = read_plan(path)
        self.tool_commit = envelope["tool_commit"]
        intent = json_read(self.shared / "intent.json")
        self.plan, self.commit, self.repo = envelope["plan"], intent["commit"], intent["repo"]
        require(self.plan, "expected a release plan, not a no-op")
        require(SHA.fullmatch(self.commit) and REPO.fullmatch(self.repo), "invalid release commit or repository")
        self.intent = {**intent, "plan": sha(path), "releases": sha(self.releases)}
        require(SHA.fullmatch(self.tool_commit), "invalid planning tool commit")
        self.id = self.plan["id"]
        self.tag = self.plan["author_tag"]
        self.descriptor = self.plan["descriptor"]
        self.source = self.plan["source_asset"]
        self.hosts = {host["name"]: host for host in self.plan["hosts"]}
        self.initial = self.plan["mode"] != "rebuild"
        names = [*self.hosts, self.id, self.source, self.tag]
        names.extend(host["archive"] for host in self.hosts.values() if host["archive"] is not None)
        if self.descriptor is not None:
            names.append(self.descriptor)
        require(all(PLAIN.fullmatch(n) for n in names), "an artifact, project or release name is not a plain name")
        require(len(self.hosts) == len(self.plan["hosts"]), "duplicate SDK hosts in the plan")
        self.signed = self.work / "signed"
        self.assets = self.signed / "assets"

    def check_tool(self, env=None):
        match = re.fullmatch(r"cargo-sunrise-mod commit ([0-9a-f]{40}) native-contract [0-9]+\n?",
                             tool("--version", capture=True, env=env))
        require(match and match[1] == self.tool_commit, "the mod tool changed since planning; use the verified planning tool commit")

    @property
    def reviewed(self):
        return self.shared / "reviewed" / self.source

    def prepare(self, directory, *, build=False, env=None):
        args = ["release", "prepare-rebuild", "--plan", self.shared / "plan.json",
                "--package", self.reviewed, "--out", directory]
        if build:
            args.append("--build")
        tool(*args, env=env)
        return directory

    def outputs(self, host):
        return ({host["archive"]} if host["archive"] else set()) | ({self.source} if host["source_producer"] else set())

    def release_names(self):
        names = {host["archive"] for host in self.hosts.values() if host["archive"]}
        if self.initial:
            names.update([self.source, f"{self.source}.minisig"])
        if self.descriptor:
            names.update([self.descriptor, f"{self.descriptor}.minisig"])
        return names

    def check_signed(self):
        record = json_read(self.signed / "batch.json")
        require(record["intent"] == self.intent, "the saved signed batch is not for this plan; do not re-sign a partial release")
        expected = self.release_names()
        require(files(self.assets) == expected == set(record["sha256"]), "the signed batch has missing or unexpected assets")
        for name, expected in record["sha256"].items():
            require(sha(self.assets / name) == expected, f"{name}: the saved signed bytes changed")
        return record["sha256"]


def plan(work, project, commit, repo, tag, releases, *, registry=None, published=None, version=None, target=None):
    shared = work / "plan"
    shared.mkdir(parents=True)
    if releases:
        shutil.copyfile(releases, shared / "releases.json")
    else:
        tool("registry", "releases", "--out", shared / "releases.json")
    args = ["release", "plan", "--releases", shared / "releases.json", "--sdk-data", shared / "sdk-data",
            "--out", shared / "plan.json"]
    if tag is not None:
        args.extend(["--tag", tag])
    else:
        manifest = tomllib.loads((project / "sunrise-mod.toml").read_text(encoding="utf-8"))
        args.extend(["--registry", registry, "--index", published, "--repo", repo, "--id", manifest["id"]])
        if version or target:
            require(version and target, "dispatch requires both a mod version and a stable Sunrise target")
            args.extend(["--version", version, "--target", target])
    tool(*args, cwd=project)
    json_write(shared / "intent.json", {"commit": commit, "repo": repo})
    if read_plan(shared / "plan.json")["plan"] is None:
        print("No eligible reviewed version needs a binary rebuild on its stable line's tip.")
        return None
    batch = Batch(work)
    batch.check_tool()
    if tag is not None:
        plan_outputs(batch)
    return batch


def plan_outputs(batch):
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as file:
            print("selected=true", file=file)
            print(f"author_tag={batch.tag}", file=file)
            print(f"commit={batch.commit}", file=file)
            print(f"tool_commit={batch.tool_commit}", file=file)
            print("matrix=" + json.dumps({"include": batch.plan["hosts"]}, separators=(",", ":")), file=file)


def build(batch, project, name):
    batch.check_tool()
    host = batch.hosts[name]
    release, triple = batch.plan["rust"], host["triple"]
    requested = {f"release: {release}", f"host: {triple}"}
    version = subprocess.run(["rustc", "-vV"], text=True, capture_output=True, check=False)
    env = os.environ.copy()
    # A daily runner can already have exactly this Rust. Otherwise select it for cargo check and
    # tests too, not only the native-build path that always matches the SDK.
    if version.returncode != 0 or not requested.issubset(version.stdout.splitlines()):
        rust = f"{release}-{triple}"
        call("rustup", "toolchain", "install", rust, "--profile", "minimal")
        env["RUSTUP_TOOLCHAIN"] = rust
    if not batch.initial:
        project = batch.prepare(batch.work / "build-project", build=True, env=env)
    lock = project / "Cargo.lock"
    reviewed = lock.read_bytes() if lock.is_file() else None
    tool("check", cwd=project, env=env)
    if reviewed is not None:
        require(lock.is_file() and lock.read_bytes() == reviewed, "cargo check would change the reviewed Cargo.lock; fix it before releasing")
    call("cargo", "test", "--locked", cwd=project, env=env)
    out = batch.work / "host-results" / name
    out.mkdir(parents=True)
    args = ["build"]
    if host["archive"]:
        args.extend(["--binaries", out, "--sunrise-release", batch.plan["target"]["tag"], "--releases", batch.releases])
        if host["side"] == "server":
            args.append("--server")
    tool(*args, cwd=project, env=env)
    if reviewed is not None:
        require(lock.read_bytes() == reviewed, "the release build changed the reviewed Cargo.lock")
    if host["source_producer"]:
        shutil.copyfile(project / "target" / "sunrise-mod" / f"{batch.id}.smod", out / batch.source)
    require(files(out) == batch.outputs(host), f"{name}: build did not produce the planned files")


def sign(batch, project):
    env = {k: v for k, v in os.environ.items() if k not in (KEY, PASSPHRASE)}
    batch.check_tool(env=env)
    unsigned = batch.work / "host-results"
    require({p.name for p in unsigned.iterdir()} == set(batch.hosts), "the unsigned batch has missing or unexpected host results")
    combined = {}
    for name, host in batch.hosts.items():
        folder = unsigned / name
        require(not folder.is_symlink() and files(folder) == batch.outputs(host), f"{name}: missing or unexpected artifacts")
        for asset in batch.outputs(host):
            require(asset not in combined, f"{asset}: produced by more than one host")
            combined[asset] = folder / asset
    if not batch.initial:
        project = batch.prepare(batch.work / "sign-project", env=env)
    batch.assets.mkdir(parents=True)
    for name, path in sorted(combined.items()):
        dest = batch.assets / name
        shutil.copyfile(path, dest)
        tool("sign", dest, "--key-env", cwd=project)
    if batch.descriptor:
        # Descriptor generation reads metadata and signatures, not mod code, and does not need the key.
        tool("build", "--describe-binaries", batch.assets, "--sunrise-release", batch.plan["target"]["tag"],
             "--releases", batch.releases, "--sdk-data", batch.sdk, cwd=project, env=env)
        tool("sign", batch.assets / batch.descriptor, "--key-env", cwd=project)
        for host in batch.hosts.values():
            (batch.assets / f'{host["archive"]}.minisig').unlink()
    expected = batch.release_names()
    require(files(batch.assets) == expected, "signing did not produce the exact release asset set")
    json_write(batch.signed / "batch.json", {"intent": batch.intent, "sha256": {name: sha(batch.assets / name) for name in expected}})


def confirmed_rebuild(batch, registry, published, hashes):
    """Refresh review and availability before touching a release or submitting an append."""
    record = batch.plan["reviewed_source"]
    source_tag, target = batch.plan["source"]["tag"], batch.plan["target"]
    entry = registry / "mods" / f"{batch.id}.toml"
    require(entry.is_file() and not entry.is_symlink(), "the reviewed mod entry is missing or linked")
    current = tomllib.loads(entry.read_text(encoding="utf-8"))
    key = os.environ.get("SUNRISE_MOD_PUBLIC_KEY", "").strip()
    require(key == record["key"] and current.get("key", "").strip() == key,
            "SUNRISE_MOD_PUBLIC_KEY or the reviewed key changed; replan the rebuild")
    require(current["owners"] == record["owners"] and current.get("owner_ids", []) == record["owner_ids"],
            "the reviewed owners changed; replan the rebuild")
    source = [row for row in current.get("versions", []) if row["version"] == batch.plan["version"]]
    require(len(source) == 1 and current["id"] == batch.id and current["kind"] == "native-mod",
            "the reviewed native source row is no longer present")
    row = source[0]
    require(all(row.get(field) == want for field, want in [
        ("file", record["url"]), ("sha256", record["sha256"]), ("signature", record["signature"]), ("sunrise", source_tag),
    ]) and row.get("builds"), "the reviewed source row changed; replan the rebuild")
    revoked = registry / "revoked.toml"
    local_revoked = []
    if revoked.exists():
        require(not revoked.is_symlink(), "the registry revocation list is linked")
        local_revoked = tomllib.loads(revoked.read_text(encoding="utf-8")).get("revoked", [])
    index = json_read(published)
    require(index.get("schema") == 1, "unsupported published registry index schema")
    for item in local_revoked + index["revoked"]:
        if item["id"] == batch.id:
            require("sha256" in item and item["sha256"].lower() not in {record["sha256"], *hashes.values()},
                    "the reviewed source or a build asset was taken down; stop the rebuild")
    projects = [item for item in index["projects"] if item["id"] == batch.id]
    require(len(projects) == 1 and projects[0].get("key") == key and projects[0]["kind"] == "native-mod",
            "the published mod or its signing key changed; replan the rebuild")
    versions = [item for item in projects[0]["versions"] if item["version"] == row["version"]]
    require(len(versions) == 1, "the reviewed source version is no longer published")
    listed = versions[0]
    require(listed.get("sunrise") == source_tag and listed["game"] == row["game"] and listed.get("native_binaries")
            and all(listed["file"].get(field) == want for field, want in [
                ("name", batch.source), ("size", record["size"]), ("sha256", record["sha256"]), ("signature", record["signature"]),
            ]), "the published source identity or binary eligibility changed; replan the rebuild")
    commits = {release["tag"]: release["commit"] for release in index.get("releases", [])}
    require(commits.get(source_tag) == batch.plan["source"]["commit"]
            and commits.get(target["tag"], target["commit"]) == target["commit"],
            "the published stable pin changed; replan the rebuild")
    if nightly := index.get("nightly_build"):
        commits["nightly"] = nightly["commit"]
    require(not any(verdict["verdict"] in {"breaks", "rebuild"} and commits.get(tag) == target["commit"]
                    for tag, verdict in listed.get("verdicts", {}).items()),
            "the reviewed source now has a published fit break on this commit")
    releases = {release["tag"]: release["commit"] for release in json_read(batch.releases)}
    matches = [build for build in row["builds"] if releases.get(build["sunrise"]) == target["commit"]]
    if matches:
        signature = (batch.assets / f"{batch.descriptor}.minisig").read_bytes().decode()
        require(matches == [{"sunrise": target["tag"], "sha256": hashes[batch.descriptor], "signature": signature,
                             "binaries": f"{record['url'].rsplit('/', 1)[0]}/{batch.descriptor}"}],
                "the registry already has a different build for this commit")
        return True
    require(not any(build["build"] == target["commit"] for build in listed["native_binaries"]),
            "the published index already covers this commit; replan the rebuild")
    return False


def stage(batch, project, registry, out, *, source_only=False, published=None):
    batch.check_tool()
    hashes = batch.check_signed()
    if not batch.initial:
        if confirmed_rebuild(batch, registry, published, hashes):
            print(f"{batch.id} {batch.plan['version']} already lists the exact rebuild")
            out.mkdir(parents=True)
            (out / "ALREADY_LISTED").touch()
            return
        project = batch.prepare(batch.work / f"{out.name}-project")
        base = batch.plan["reviewed_source"]["url"].rsplit("/", 1)[0]
        tool("publish", "--out", out, "--registry", registry, "--asset-base", base, "--releases", batch.releases,
             "--package", batch.reviewed, "--binaries", batch.assets, "--sdk-data", batch.sdk, cwd=project)
        for name, expected in hashes.items():
            require(sha(out / name) == expected, f"{name}: publish staged different bytes")
        return
    base = f"https://github.com/{batch.repo}/releases/download/{batch.tag}"
    key = os.environ.get("SUNRISE_MOD_PUBLIC_KEY", "").strip()
    require(key, "set SUNRISE_MOD_PUBLIC_KEY to the public half of the author signing key")
    entry = registry / "mods" / f"{batch.id}.toml"
    previous = tomllib.loads(entry.read_text(encoding="utf-8")) if entry.exists() else None
    signature = batch.assets / f"{batch.source}.minisig"
    version = batch.plan["version"]
    if previous:
        for row in previous.get("versions", []):
            if row["version"] != version:
                continue
            require(previous.get("key", "").strip() == key and row["sha256"] == hashes[batch.source]
                    and row.get("signature") == signature.read_text(encoding="utf-8")
                    and row.get("sunrise") == batch.plan["source"]["tag"] and row["file"] == f"{base}/{batch.source}",
                    "the registry already has a different source for this version")
            if batch.descriptor and not source_only:
                require(any(build["sunrise"] == batch.plan["target"]["tag"] and
                            build["sha256"] == hashes[batch.descriptor]
                            for build in row.get("builds", [])), "this version is already listed; use the rebuild flow to append a target")
            print(f"{batch.id} {version} is already listed with these assets")
            out.mkdir(parents=True)
            (out / "ALREADY_LISTED").touch()
            return
    args = ["publish", "--out", out, "--registry", registry, "--asset-base", base,
            "--releases", batch.releases, "--package", batch.assets / batch.source,
            "--sunrise", batch.plan["source"]["tag"], "--key", key, "--signature", signature]
    for flag, variable in [("--owner", "SUNRISE_MOD_REGISTRY_OWNER"), ("--summary", "SUNRISE_MOD_SUMMARY"),
                           ("--categories", "SUNRISE_MOD_CATEGORIES")]:
        if value := os.environ.get(variable):
            args.extend([flag, value])
    if batch.descriptor and not source_only:
        args.extend(["--binaries", batch.assets, "--sdk-data", batch.sdk])
    tool(*args, cwd=project)
    for name in ({batch.source, signature.name} if source_only else hashes):
        require(sha(out / name) == hashes[name], f"{name}: publish staged different bytes")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["plan", "build", "sign", "stage"])
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--project", type=Path, default=Path.cwd())
    parser.add_argument("--host")
    parser.add_argument("--tag")
    parser.add_argument("--commit")
    parser.add_argument("--repo")
    parser.add_argument("--releases", type=Path)
    parser.add_argument("--registry", type=Path)
    parser.add_argument("--published", type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    work, project = args.work.resolve(), args.project.resolve()
    if args.command == "plan":
        plan(work, project, args.commit, args.repo, args.tag, args.releases)
    else:
        batch = Batch(work)
        if args.command == "build":
            build(batch, project, args.host)
        elif args.command == "sign":
            sign(batch, project)
        else:
            stage(batch, project, args.registry.resolve(), args.out.resolve(), published=args.published)


if __name__ == "__main__":
    main()
