#!/usr/bin/env python3
"""Hourly check: did Mojang change a pinned version's JSON (.github/workflows/mojang-republish.yml)?

    python3 tools/ci/mojang_republish.py [--dry-run]

Fetches Mojang's public version manifest and, for each version research-lock.json pins (tools/lab.py
`pinned_versions`), looks at that version's entry. The lock's own `manifest_sha1` needs nothing. Any other sha1 is
downloaded and compared with the lock entry by content (tools/lab.py `pinned_content_differs`: id, client and server
jars, Java major, libraries):
- the same content: Mojang re-issued the file for its asset index alone. tools/lab.py and sunrise-prepare accept it
  as it is, but the launcher accepts a version JSON by its sha1 (sunrise-launcher's src/sources.rs `Pins::read`), so a sha1
  not yet in `accepted_manifest_sha1` gets an issue "Mojang re-issued <version>: <sha8>" with the line to add;
- other content: it opens an issue "Mojang changed <version>: <sha8>" naming what differs, because Sunrise refuses
  that file until the lock pins the new content. An issue with that title, open or closed, is not opened again or
  commented on (no hourly noise).
Nothing from Mojang is stored; only hashes go in the issue. Needs the gh CLI; --dry-run prints what it would do.
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
# tools/lab.py: the pinned versions and the content rule.
sys.path.append(str(ROOT / "tools"))
import lab  # noqa: E402

LOCK = ROOT / "research-lock.json"
MANIFEST_URL = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
TITLE = "Mojang changed {version}: "
REISSUE = "Mojang re-issued {version}: "


def fetch_json(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.load(response)


def plan(game, manifest, fetch, open_issues):
    """What to do about one pinned version (`game`, its lock entry): None when Mojang lists the pinned file,
    or a re-issue with the pinned content the launcher already accepts, else {kind: open|skip, number, title, body,
    changed}; `changed` is empty for a re-issue with the pinned content.

    open_issues: {title: number} of the issues, open or closed (a known one is not reported again). fetch(url) -> the JSON at url."""
    version = game["version"]
    entry = next((v for v in manifest.get("versions", []) if v.get("id") == version), None)
    if entry is None:
        raise ValueError(f"{version} is not in Mojang's version manifest")
    checksum = entry["sha1"]
    accepted = game.get("accepted_manifest_sha1", [])
    if checksum == game["manifest_sha1"] or checksum in accepted:
        return None
    changed = lab.pinned_content_differs(game, fetch(entry["url"]))
    if not changed:
        snippet = json.dumps({"accepted_manifest_sha1": accepted + [checksum]}, indent=2)
        body = (f"Mojang re-issued the {version} version JSON with the pinned content (jars, Java major, libraries), "
                "so sunrise-prepare already accepts it.\n\n"
                f"- manifest sha1: `{checksum}`\n- url: {entry['url']}\n\n"
                "The launcher accepts a version JSON by its sha1 (sunrise-launcher's `src/sources.rs`, `Pins::read`): a player whose "
                "launcher holds this one gets the pinned file downloaded from Mojang instead, so cannot prepare offline. "
                "Add it to `research-lock.json`:\n\n"
                f"```json\n{snippet}\n```\n")
        title = REISSUE.format(version=version) + checksum[:8]
        number = open_issues.get(title)
        return {"kind": "skip" if number else "open", "number": number, "title": title, "body": body, "changed": []}
    body = (f"Mojang's manifest now lists a {version} version JSON whose content differs from the one "
            f"`research-lock.json` pins, so Sunrise refuses it.\n\n"
            f"- manifest sha1: `{checksum}`\n- url: {entry['url']}\n- differs: {', '.join(f'`{c}`' for c in changed)}\n\n"
            "If this is the release Sunrise targets, pin its content in `research-lock.json` "
            "(`python3 tools/port_regen.py` makes the block).\n")
    title = TITLE.format(version=version) + checksum[:8]
    number = open_issues.get(title)
    return {"kind": "skip" if number else "open", "number": number, "title": title, "body": body, "changed": changed}


def gh(*arguments, dry_run=False):
    if dry_run:
        print("would run: gh " + " ".join(arguments[:4]) + " ...")
        return ""
    return subprocess.run(["gh", *arguments], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def open_issues_titled(version, dry_run=False, runner=gh, title=TITLE):
    """{title: number} of issues, open or closed, whose title starts with `title`'s prefix for this version (no label
    needed)."""
    prefix = title.format(version=version)
    listed = json.loads(runner("issue", "list", "--state", "all", "--limit", "200", "--search",
                               f'"{prefix.strip()}" in:title', "--json", "number,title") or "[]")
    return {issue["title"]: issue["number"] for issue in listed if issue["title"].startswith(prefix)}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    lock = json.loads(LOCK.read_text())
    manifest = fetch_json(MANIFEST_URL)
    for game in lab.pinned_versions(lock):
        version = game["version"]
        try:
            listed = {**open_issues_titled(version), **open_issues_titled(version, title=REISSUE)}
        except (subprocess.CalledProcessError, FileNotFoundError):
            if not args.dry_run:
                raise
            listed = {}
        action = plan(game, manifest, fetch_json, listed)
        if action is None:
            print(f"{version}: Mojang lists a version JSON the lock accepts, nothing to do.")
        else:
            what = f"differs: {', '.join(action['changed'])}" if action["changed"] else "the pinned content"
            print(f"{action['kind']}: {action['title']} ({what})")
            if action["kind"] == "open":
                gh("issue", "create", "--title", action["title"], "--body", action["body"], dry_run=args.dry_run)
            if args.dry_run:
                print(action["body"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
