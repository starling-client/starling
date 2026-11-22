#!/usr/bin/env python3
"""Resume an author release or submit its staged registry files."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib
from urllib.parse import quote, urlencode

from sunrise_release import Batch, REGISTRY, REPO, SHA, call, json_read, plan, plan_outputs, require, sha, stage, tool


SAVED = "sunrise-release-signed"
INDEX = "https://sunrise-org.github.io/sunrise-mods/index.json"


def pages(endpoint):
    return json.loads(call("gh", "api", "--paginate", "--slurp", endpoint, capture=True))


def api(endpoint):
    return json.loads(call("gh", "api", endpoint, capture=True))


def locate_saved():
    repo, run_id = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"]
    found = [str(row["id"]) for page in pages(f"repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100&name={SAVED}")
             for row in page["artifacts"] if row["name"] == SAVED and not row["expired"]]
    require(len(found) <= 1 and all(i.isascii() and i.isdecimal() for i in found), "ambiguous saved signed batch")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as file:
        print(f"id={next(iter(found), '')}", file=file)


def tag_commit(batch):
    ref = api(f"repos/{batch.repo}/git/ref/tags/{quote(batch.tag, safe='')}")["object"]
    seen = set()
    while ref["type"] == "tag":
        value = ref["sha"]
        require(SHA.fullmatch(value) and value not in seen and len(seen) < 8, "invalid annotated author tag chain")
        seen.add(value)
        ref = api(f"repos/{batch.repo}/git/tags/{value}")["object"]
    require(ref["type"] == "commit" and SHA.fullmatch(ref["sha"]), "author tag does not resolve to a commit")
    return ref["sha"]


def release(batch):
    # List also finds drafts, so a rerun resumes the draft rather than creating another one.
    found = [row for page in pages(f"repos/{batch.repo}/releases?per_page=100") for row in page if row["tag_name"] == batch.tag]
    require(len(found) <= 1, "more than one author release uses this tag")
    return next(iter(found), None)


def missing_assets(expected, remote, digest):
    """Compare *all* existing assets before allowing the first upload."""
    present = set()
    for row in remote:
        name = row["name"]
        if name not in expected:
            continue
        require(name not in present and row["state"] == "uploaded", f"{name}: ambiguous or incomplete remote asset; repair it in GitHub")
        require(digest(row) == expected[name], f"{name}: the author release already has different bytes; recover the original signed batch, never overwrite")
        present.add(name)
    return sorted(set(expected) - present)


def remote_files(batch, released, expected):
    rows = [row for page in pages(f'repos/{batch.repo}/releases/{released["id"]}/assets?per_page=100') for row in page]
    with tempfile.TemporaryDirectory(dir=batch.work) as scratch:
        def digest(row):
            given = row.get("digest") or ""
            if re.fullmatch(r"sha256:[0-9a-f]{64}", given):
                return given[7:]
            path = Path(scratch) / row["name"]
            call("gh", "release", "download", batch.tag, "--repo", batch.repo, "--pattern", row["name"], "--dir", scratch)
            return sha(path)
        return missing_assets(expected, rows, digest)


def recheck_stable_tags(batch):
    """Registry Pages can still describe a release that has moved or been withdrawn."""
    with tempfile.TemporaryDirectory(dir=batch.work) as scratch:
        output = Path(scratch) / "releases.json"
        tool("registry", "releases", "--out", output)
        commits = {row["tag"]: row["commit"] for row in json_read(output)}
    for role in ("source", "target"):
        tag = batch.plan[role]["tag"]
        require(commits.get(tag) == batch.plan[role]["commit"],
                f"Sunrise {role} tag {tag} moved or was withdrawn since planning; replan the rebuild")


def upload(batch):
    batch.check_tool()
    expected = batch.check_signed()
    original = {}
    if not batch.initial:
        checkout, published = batch.work / "registry", batch.work / "published.json"
        registry(checkout, published)
        out = batch.work / "upload-preflight"
        stage(batch, None, checkout, out, published=published)
        if (out / "ALREADY_LISTED").exists():
            return
        original[batch.source] = batch.plan["reviewed_source"]["sha256"]
    tagged = tag_commit(batch)
    if batch.initial:
        require(tagged == batch.commit, "the author tag no longer names the planned mod commit")
    released = release(batch)
    if released is None:
        require(batch.initial, "there is no author release to append to")
        call("gh", "release", "create", batch.tag, "--repo", batch.repo, "--verify-tag", "--draft", "--title", batch.tag, "--notes", "")
        released = release(batch)
        require(released, "could not find the draft just created")
    require(batch.initial or not released["draft"], "a rebuild must append to a published author release")
    remote_expected = {**expected, **original}
    pending = remote_files(batch, released, remote_expected)
    require(not set(pending).intersection(original), "the original reviewed source is missing from the author release")
    if not batch.initial:
        recheck_stable_tags(batch)
    if pending:
        call("gh", "release", "upload", batch.tag, *(batch.assets / name for name in pending), "--repo", batch.repo)
    require(not remote_files(batch, released, remote_expected), "the author release is missing assets after upload")
    require(tag_commit(batch) == tagged, "the author tag moved during upload; verify the release before publishing it")
    if batch.initial and released["draft"]:
        call("gh", "release", "edit", batch.tag, "--repo", batch.repo, "--draft=false")


def download(url, path, limit):
    call("curl", "--disable", "--proto", "=https", "--proto-redir", "=https", "--max-filesize", str(limit),
         "-fsSL", "-o", path, "--", url)


def registry(checkout, published=None):
    call("git", "clone", "--depth=1", f"https://github.com/{REGISTRY}", checkout)
    if published is not None:
        download(INDEX, published, 16 << 20)


def rebuild_plan(work, project, commit, repo, version, target):
    if os.environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch":
        require(version and target, "dispatch requires both a mod version and a stable Sunrise target")
    work.mkdir(parents=True, exist_ok=True)
    checkout, published = work / "registry", work / "published.json"
    registry(checkout, published)
    batch = plan(work, project, commit, repo, None, None, registry=checkout, published=published,
                 version=version, target=target)
    if batch is None:
        return
    source = batch.plan["reviewed_source"]
    require(os.environ.get("SUNRISE_MOD_PUBLIC_KEY", "").strip() == source["key"],
            "SUNRISE_MOD_PUBLIC_KEY does not match the reviewed source key")
    canonical = f"https://github.com/{batch.repo}/releases/download/{batch.tag}/{batch.source}"
    require(source["url"].lower() == canonical.lower() and 0 < source["size"] <= 64 << 20,
            "the reviewed source is not a bounded author release asset")
    batch.reviewed.parent.mkdir()
    download(source["url"], batch.reviewed, source["size"])
    with tempfile.TemporaryDirectory(dir=work) as scratch:
        batch.prepare(Path(scratch) / "manifest")
    plan_outputs(batch)


def apply_files(source, checkout, relative):
    for path in relative:
        to = checkout / path
        require(not to.parent.is_symlink() and not to.is_symlink(), f"registry submission path {path} is linked")
        to.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source / path, to)


def submit(batch, checkout, staged, fork):
    batch.check_tool()
    if (staged / "ALREADY_LISTED").exists():
        return
    if not batch.initial:
        recheck_stable_tags(batch)
    draft = (staged / "PULL_REQUEST.md").read_text(encoding="utf-8")
    relative = [Path("mods") / f"{batch.id}.toml"]
    icon = Path("icons") / f"{batch.id}.png"
    if (staged / icon).is_file():
        relative.append(icon)
    require(all((staged / path).is_file() and not (staged / path).is_symlink() for path in relative), "staged submission files are absent or linked")
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a", encoding="utf-8") as file:
            file.write("## Registry submission\n\nOnce this registry job succeeds, the staged entry and any staged icon are attached to this run as `sunrise-registry-submission`.\n\n" + draft)
    if not fork or not os.environ.get("GH_TOKEN"):
        print("No registry fork token configured. Once this job succeeds, open the pull request yourself from the `sunrise-registry-submission` artifact.")
        return
    require(REPO.fullmatch(fork), "SUNRISE_MOD_REGISTRY_FORK must be owner/name")
    repository = api(f"repos/{fork}")
    require(repository["fork"] and repository["parent"]["full_name"].lower() == REGISTRY,
            "the configured repository is not a fork of the Sunrise registry")
    user = api("user")
    login, user_id = user["login"], user["id"]
    owner = fork.split("/")[0]
    require(owner.lower() == login.lower(), "the registry token must belong to the owner of the configured fork")
    entry = tomllib.loads((staged / relative[0]).read_text(encoding="utf-8"))
    ids = entry.get("owner_ids", [])
    owns_entry = user_id in ids if ids else any(login.lower() == name.lower() for name in entry["owners"])
    require(owns_entry, "the registry fork token does not belong to a listed owner")
    with tempfile.TemporaryDirectory(dir=batch.work) as scratch:
        candidate = Path(scratch) / "candidate"
        shutil.copytree(checkout, candidate, symlinks=True, ignore=shutil.ignore_patterns(".git"))
        apply_files(staged, candidate, relative)
        try:
            tool("registry", "review", checkout, candidate, "--author", login, "--releases", batch.releases)
        except subprocess.CalledProcessError:
            print("The registry has required changes. Use the attached submission and complete it manually.")
            return
    branch = f'sunrise-release/{batch.id}-{batch.plan["version"]}'
    if not batch.initial:
        branch += f'-{batch.plan["target"]["tag"]}'
    base = api(f"repos/{REGISTRY}")["default_branch"]
    if not batch.initial:
        base_tip = call("git", "rev-parse", "HEAD", cwd=checkout, capture=True).strip()
        remote_tip = call("git", "ls-remote", "origin", "HEAD", cwd=checkout, capture=True).split()
        require(remote_tip == [base_tip, "HEAD"], "the registry changed after staging; rerun from its new head")
    call("gh", "auth", "setup-git", "--hostname", "github.com")
    url = f"https://github.com/{fork}"
    listed = call("git", "ls-remote", "--heads", url, f"refs/heads/{branch}", cwd=checkout, capture=True)
    if listed:
        call("git", "fetch", "--depth=1", url, f"refs/heads/{branch}", cwd=checkout)
        head = call("git", "rev-parse", "FETCH_HEAD", cwd=checkout, capture=True).strip()
        compare = quote(f"{base}...{owner}:{head}", safe=":.")
        diff = api(f"repos/{REGISTRY}/compare/{compare}")["files"]
        allowed = {path.as_posix() for path in relative}
        require(all(row["filename"] in allowed and "previous_filename" not in row for row in diff),
                "the registry submission branch has unrelated changes; review it manually")
        call("git", "switch", "-c", branch, head, cwd=checkout)
    else:
        call("git", "switch", "-c", branch, cwd=checkout)
    apply_files(staged, checkout, relative)
    call("git", "add", "--", *relative, cwd=checkout)
    changed = call("git", "diff", "--cached", "--name-only", cwd=checkout, capture=True)
    if changed:
        title = draft.partition("\n")[0]
        call("git", "-c", f"user.name={login}", "-c", f"user.email={user_id}+{login}@users.noreply.github.com",
             "commit", "--signoff", "-m", title, cwd=checkout)
        call("git", "push", url, f"HEAD:refs/heads/{branch}", cwd=checkout)
    query = urlencode({"head": f"{owner}:{branch}", "state": "open", "base": base})
    existing = api(f"repos/{REGISTRY}/pulls?{query}")
    if existing:
        print(existing[0]["html_url"])
    else:
        title, _, body = draft.partition("\n\n")
        call("gh", "pr", "create", "--repo", REGISTRY, "--head", f"{owner}:{branch}", "--base", base,
             "--title", title, "--body", body)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["saved", "registry", "upload", "submit", "rebuild-plan"])
    parser.add_argument("--work", type=Path)
    parser.add_argument("--registry", type=Path)
    parser.add_argument("--submission", type=Path)
    parser.add_argument("--commit")
    parser.add_argument("--repo")
    parser.add_argument("--version")
    parser.add_argument("--target")
    args = parser.parse_args()
    if args.command == "saved":
        locate_saved()
    elif args.command == "registry":
        batch = Batch(args.work) if args.work else None
        registry(args.registry.resolve(), batch.work / "published.json" if batch and not batch.initial else None)
    elif args.command == "rebuild-plan":
        rebuild_plan(args.work.resolve(), Path.cwd(), args.commit, args.repo, args.version, args.target)
    else:
        batch = Batch(args.work)
        if args.command == "upload":
            upload(batch)
        else:
            submit(batch, args.registry, args.submission, os.environ.get("SUNRISE_MOD_REGISTRY_FORK"))


if __name__ == "__main__":
    main()
