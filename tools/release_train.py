#!/usr/bin/env python3
"""Decides when Sunrise releases, names the release and writes its notes (.github/workflows/release.yml).

    python3 tools/release_train.py decide --kind game [--head SHA] [--force]
    python3 tools/release_train.py decide --kind game --channel nightly|dev [--head SHA] \\
        [--force] [--daily] [--exception TEXT]
    python3 tools/release_train.py notes --kind game --tag v2026.10.15 --out dist/notes
    python3 tools/release_train.py gate --kind game --smoke dist/smoke-test.json [--blockers]   # before publishing

The game ships only the Nightly since 2026-10-28 (Ludovic): when `decide --kind game` says so, release.yml starts
`nightly build` on the commit it decided (tools/ci/nightly.py), and "the last release" is the published Nightly.
On the fork (CI tiers, owner 2026-11-13) `--channel` names the kind of build: `nightly` takes `daily-green` and
refuses any other --head (an older commit had no daily run of its own, and no SDK) unless --exception names the
owner's one-time grant for it (owner, 2026-11-17: an exception never changes the rule); `dev` takes main's head (or
a --head on main) and refuses a commit whose gate jobs (dyna-checks' main-push lint, verify, verify-static)
did not all pass (pending is not passed). The next push cancels a main push run (dyna-ci.yml), so a publishing run
(GATE_RERUN=1: release.yml's decide and nightly-build.yml's pick, real runs) waits for the commit's run and starts it again once when it was cancelled, in a
group no push cancels (gate_again); otherwise it always builds, as dev-<date>-<short> (no batch rules,
cap, pause or release-blocker: a Dev build is for testing main, never for players).

Release branches (docs/release-flow.md "Release branches"): once an owner creates `nightly/<target>`, the daily
fast-forwards it to each fully green commit and a Nightly builds its head instead of `daily-green`. A stable release,
`<target>-<semver>` (26.3-0.1.0 first), builds the approved head of `release/<target>`, which must have a completed
real daily run that passed every leg on exactly that commit, on main or the branch (daily_passed). Only main and
those two branches publish (PUBLISHES): `authorize` decides it exactly for a workflow, and `--ref` (a real run's
GITHUB_REF) refuses a kind its ref may not publish. A break-glass patch release takes the daily legs its changes select
instead (break_glass_problems) and two people's approvals (`break-glass-approvals`).

A kind releases when a pull request merged into main since its last published release changed one of its
player-facing paths (GAME_PATHS), once BATCH of them are waiting and the newest merged at least
DEBOUNCE ago (a burst of merges becomes one release), or once the oldest has waited MAX_WAIT (a lone fix still ships),
or at once when one of those pull requests is labelled release-now. Pull requests labelled skip-release (work players don't see) don't count
towards a release and aren't listed in the notes. RELEASES_PAUSED=1 (a repository variable) stops automatic releases,
and an open issue labelled release-blocker stops game releases, even forced ones, until it is closed.

Tags (docs/versioning.md): with a release line in .github/release-line.txt (MAJOR.MINOR, e.g. 0.2), game releases
are vMAJOR.MINOR.PATCH, each the patch after the last tag on the line (MAJOR.MINOR.0 first: a line's first release goes out without waiting for a pull request). Without the
file, the tags from before 0.2: vYYYY.MM.DD, then .01, .02 the same UTC day. Every 0.2 tag is newer than
every tag from before (tag_key). Notes:
"### New" lists the pull requests labelled feature, "### Fixed" the others, "### Known issues" the bullets of
docs/known-issues.md. Every release is published as the release, GitHub's latest, which launchers, servers and
install.sh take at once; one channel, no promotion. A game release is published only after `gate`: its smoke test
passed and no release-blocker is open. Notes cover everything since the last published release; the game
pre-releases of the old Alpha channel (v0.2.1 on) don't count, so the first release after them lists all of
it since v0.2.0. See docs/release-flow.md.

Prints the decision as JSON and writes it to $GITHUB_OUTPUT when set. Needs git history and the gh CLI.
"""

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import posixpath
import re
import subprocess
import sys
import time
import tomllib
import typing

ROOT = Path(__file__).resolve().parents[1]
REPO = os.environ.get("GITHUB_REPOSITORY", "sunrise-org/sunrise")
# The maintainer, left out of the notes' thanks (the repository's owner is the horizon-rust organization).
MAINTAINER = "ludovic111"
# Releases ship in batches, so players get a few releases a day rather than one per merge: BATCH player-facing pull
# requests, settled for DEBOUNCE, or whatever is waiting once the oldest has waited MAX_WAIT.
BATCH = 5
DEBOUNCE = dt.timedelta(minutes=5)
MAX_WAIT = dt.timedelta(hours=6)
# Kept as typed in release titles, where the clauses after the first are lowercased.
PROPER = {"Sunrise", "Sunrise's", "Minecraft", "Mojang", "Microsoft", "Java", "Xbox", "PlayStation", "Steam", "Windows",
          "Linux", "LAN", "Kira", "Discord", "Wayland", "Hyprland", "F3", "GUI", "UI"}
EXPORTERS = "mods/sunrise/src/test/java/dev/sunriselab/parity/"
# The release line (docs/versioning.md): MAJOR.MINOR. Without the file, releases keep the tags from before 0.2.
LINE_FILE = ROOT / ".github" / "release-line.txt"
SEMVER = r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
# Semver's majors stay below this; a date tag's first number is a year.
FIRST_YEAR = 1000
GAME_PREFIX = "v"

# What players run. Documentation (*.md), tests, benchmarks and harness probes (examples/) never count.
GAME_PATHS = ["crates/", "xtask/", "packaging/", "Cargo.toml", "Cargo.lock", "research-lock.json"]
# A game release is GitHub's latest (servers and install.sh follow it), so it is the first release people land on:
# its notes open with the way to the launcher (#1647). Raw HTML on one line, which GitHub shows and every launcher's
# notes view and one-line summary skip (pulldown-cmark's HTML events), so launchers never tell players to get
# themselves. The site's Releases page leaves it out too.
# The same warning closes the notes, above the disclaimer, for whoever scrolls past the changes to the files.
PLAYERS_FOOTER = ('<h1>Want to play? Download the <a href="https://playsunrise.net">Sunrise launcher</a>.</h1>'
                  '<h1>Do NOT run the files below. They will not work on their own.</h1>')
PLAYERS = (PLAYERS_FOOTER + '<p>The launcher installs this version and keeps it up to date. You need your own copy of '
           'Minecraft: Java Edition. The files below are what the launcher installs, and the dedicated server for '
           'people who host one.</p>')
DISCLAIMER = "Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft."
# An open issue with this label holds every game release (docs/release-flow.md). Only people who can triage the
# repository can add it, and the issue's text is never read: its author could edit it afterwards.
BLOCKER = "release-blocker"


def run(*command, check=True):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if check and result.returncode:
        raise SystemExit(f"{' '.join(command)}: {result.stderr.strip()}")
    return result.stdout


def json_pages(text):
    """The JSON values `gh api --paginate` prints, one per page, as a list: what `--slurp` returns, which the runners'
    gh (Ubuntu's 2.45) lacks."""
    decoder, pages, at = json.JSONDecoder(), [], len(text) - len(text.lstrip())
    while at < len(text):
        page, end = decoder.raw_decode(text, at)
        pages.append(page)
        at = len(text) - len(text[end:].lstrip())
    return pages


def gh_json(*arguments):
    """gh's JSON answer; with --paginate, the list of its pages."""
    out = run("gh", *arguments)
    return json_pages(out) if "--paginate" in arguments else json.loads(out or "null")


def exporters():
    """The exporter and oracle sources sunrise-prepare compiles: those its sources name."""
    names = set()
    for source in (ROOT / "crates" / "sunrise-prepare" / "src").glob("*.rs"):
        names.update(re.findall(r'"([A-Z]\w+\.java)"', source.read_text()))
    return {EXPORTERS + name for name in names}


def player_facing(path, kind, exporter_paths=frozenset()):
    """Whether a change to path ships to players in a release of this kind."""
    if path.endswith(".md") or re.match(r"crates/([\w-]+/)?(examples|tests|benches)/", path):
        return False
    return any(path == p or path.startswith(p) for p in GAME_PATHS) or path in exporter_paths


def semver(tag, prefix):
    """(major, minor, patch, pre-release or None) of a 0.2 tag `<prefix>X.Y.Z[-PRE]`, else None."""
    match = re.fullmatch(re.escape(prefix) + SEMVER, tag)
    if not match or int(match[1]) >= FIRST_YEAR:
        return None
    return int(match[1]), int(match[2]), int(match[3]), match[4]


def is_game_tag(tag):
    return re.fullmatch(r"v\d{4}\.\d{2}\.\d{2}(\.\d+)?", tag) is not None or semver(tag, GAME_PREFIX) is not None


def tag_key(tag):
    """A tag's place among releases (docs/versioning.md): the tags from before 0.2 (vYYYY.MM.DD[.N],
    launcher-vX.Y.Z, which the launcher's old releases here carry) by their numbers, trailing zeros dropped (v2026.10.19 is v2026.10.19.0); then every 0.2 tag
    (vX.Y.Z) in semver's order, a pre-release (v0.3.0-rc.1) before its release."""
    version = semver(tag, GAME_PREFIX)
    if version is None:
        numbers = [int(part) for part in re.findall(r"\d+", tag)]
        while numbers and numbers[-1] == 0:
            numbers.pop()
        return 0, tuple(numbers), ()
    *numbers, pre = version
    # A release follows its pre-releases; their identifiers compare one by one, numbers before words.
    rank = (1,) if pre is None else (0, tuple((0, int(i), "") if i.isdigit() else (1, 0, i) for i in pre.split(".")))
    return 1, tuple(numbers), rank


def release_line(path=LINE_FILE):
    """(major, minor) of the release line, or None without the file (the tags from before 0.2)."""
    if not path.exists():
        return None
    text = path.read_text().strip()
    match = re.fullmatch(r"(\d+)\.(\d+)", text)
    if not match:
        raise SystemExit(f"{path}: a release line is MAJOR.MINOR, like 0.2 (got {text!r})")
    return int(match[1]), int(match[2])


def next_on_line(tags, prefix, line):
    """<prefix>MAJOR.MINOR.PATCH: the patch after the newest release tagged on the line (drafts and rolled-back
    releases included; a hand-made pre-release such as v0.3.0-rc.1 is not one), or .0. Never a line below one that
    has tags: GitHub's latest release only moves forward."""
    major, minor = line
    versions = [v for v in (semver(tag, prefix) for tag in tags) if v]
    above = [v for v in versions if (v[0], v[1]) > line and v[3] is None]
    if above:
        raise SystemExit(f"{LINE_FILE.name} says {major}.{minor}, but {prefix}{'.'.join(map(str, above[0][:3]))} exists")
    patches = [v[2] for v in versions if (v[0], v[1]) == line and v[3] is None]
    return f"{prefix}{major}.{minor}.{max(patches) + 1 if patches else 0}"


def first_on_line(tags, prefix, line):
    """Whether no release of this kind is tagged on the line yet (its first goes out at once)."""
    return not any(v and (v[0], v[1]) == line and v[3] is None for v in (semver(tag, prefix) for tag in tags))


def next_game_tag(tags, today, line=None):
    """vMAJOR.MINOR.PATCH on a release line; before 0.2, vYYYY.MM.DD, or .01, .02... when that day already has a
    tag (drafts and rolled-back releases included).

    The counter has two digits because GitHub's releases page sorts a day's tags as text: .9 went above .18.
    Every reader compares the numbers as integers, so the unpadded tags from before still order correctly."""
    if line:
        return next_on_line(tags, GAME_PREFIX, line)
    day = f"v{today:%Y.%m.%d}"
    taken = [tag for tag in tags if tag == day or tag.startswith(day + ".")]
    if not taken:
        return day
    return f"{day}.{max(int(tag.split('.')[3]) if tag.count('.') == 3 else 0 for tag in taken) + 1:02d}"


def merged_pull_requests(base, head):
    """(number, merge commit) of the pull requests merged into main between base and head, oldest first."""
    found = []
    for line in run("git", "log", "--first-parent", "--reverse", "--format=%H%x09%s", f"{base}..{head}").splitlines():
        commit, _, subject = line.partition("\t")
        match = re.match(r"Merge pull request #(\d+) ", subject) or re.search(r"\(#(\d+)\)$", subject)
        if match:
            found.append((int(match.group(1)), commit))
    return found


def changed_paths(commit):
    """The files a pull request changed: its merge commit against main before it (first parent)."""
    return run("git", "diff", "--name-only", f"{commit}^1", commit).split()


def merged_pull_request_metadata():
    """Repository-owned merge identities and note fields, without one network call per PR."""
    owner, name = REPO.split("/")
    query = """
    query($owner: String!, $name: String!, $endCursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequests(first: 100, states: MERGED, after: $endCursor) {
          nodes {
            number title url mergedAt mergeCommit { oid } author { login }
            labels(first: 100) { nodes { name } }
            closingIssuesReferences(first: 100) { nodes { number } }
          }
          pageInfo { hasNextPage endCursor }
        }
      }
    }
    """
    pages = gh_json("api", "graphql", "--paginate",
                    "-F", f"owner={owner}", "-F", f"name={name}", "-f", f"query={query}")
    owned = {}
    for page in pages:
        for data in page["data"]["repository"]["pullRequests"]["nodes"]:
            # A historical merge commit can have been removed by a force-push. It cannot
            # match a reachable candidate; GraphQL explicitly permits a null commit.
            if data["mergeCommit"] is None:
                continue
            owned[(data["number"], data["mergeCommit"]["oid"])] = {
                "number": data["number"],
                "title": data["title"].strip(),
                "url": data["url"],
                "labels": sorted(label["name"] for label in data["labels"]["nodes"]),
                "issues": sorted(issue["number"] for issue in data["closingIssuesReferences"]["nodes"]),
                "author": (data["author"] or {}).get("login", ""),
                "merged_at": dt.datetime.fromisoformat(data["mergedAt"].replace("Z", "+00:00")),
            }
    return owned


def published_releases():
    """Published releases with their tags and dates: no drafts, and no pre-releases (the old Alpha channel's)."""
    releases = gh_json("api", f"repos/{REPO}/releases?per_page=100", "--paginate")
    flat = [release for page in releases for release in page]
    return [release for release in flat
            if not release["draft"] and not release["prerelease"]]


def open_blockers():
    """The numbers of the open issues labelled release-blocker (pull requests left out)."""
    issues = gh_json("api", f"repos/{REPO}/issues?labels={BLOCKER}&state=open&per_page=100", "--paginate")
    return sorted(issue["number"] for page in issues for issue in page if "pull_request" not in issue)


def smoke_problem(report):
    """Why a release smoke test's report (the game's --smoke-report) fails, or None when the game drew a world."""
    if report is None:
        return "no smoke-test.json: the release smoke test did not run"
    if report.get("error") is not None:
        return f"the smoke run failed: {report['error']}"
    if report.get("frames", 0) < 1500 or not report.get("chunks") or not report.get("meshed_sections") \
            or not report.get("player"):
        return "the smoke run did not draw a world"
    return None


def gate(report, blockers):
    """Why a built game release must not be published, or None: it is published as the latest at once, so its
    smoke test must have passed and no release-blocker may be open."""
    problems = [problem for problem in [smoke_problem(report)] if problem]
    problems += [f"release-blocker #{number} is open" for number in blockers]
    return "; ".join(problems) or None


def last_release(releases, kind):
    ours = [release for release in releases if is_game_tag(release["tag_name"])]
    return max(ours, key=lambda release: tag_key(release["tag_name"]), default=None)


def decision(kind, prs, now, paused=False, force=False, new_line=None, blockers=(), daily=False):
    """Whether to release now, and why. prs: the merged pull requests since the last release, each with the
    files it changed under "paths" (player-facing ones only) and when it merged ("merged_at"). new_line: the
    release line ("0.2") when it has no release of this kind yet; its first goes out at once. blockers: the open
    release-blocker issues; they hold every release, forced ones too. daily: the daily run's call for its
    daily-green commit, published without the batch-size and age rules (it comes once a day, with no retry)."""
    counted = [pr for pr in prs if pr["paths"] and "skip-release" not in pr["labels"]]
    urgent = [pr for pr in counted if "release-now" in pr["labels"]]
    merged = [pr["merged_at"] for pr in counted if pr.get("merged_at")]
    if blockers:
        return False, "held by " + ", ".join(f"release-blocker #{number}" for number in blockers)
    if force:
        return True, "started by hand with force"
    if paused:
        return False, "RELEASES_PAUSED is set"
    if new_line:
        return True, f"the first release of {new_line}"
    if not counted:
        return False, "no merged pull request changed what players run"
    if urgent:
        return True, "release-now: " + ", ".join(f"#{pr['number']}" for pr in urgent)
    if daily:
        return True, f"{len(counted)} pull requests changed what players run; the daily run passed this commit whole"
    if merged and now - min(merged) >= MAX_WAIT:
        return True, f"{len(counted)} pull requests changed what players run; the oldest has waited {MAX_WAIT.seconds // 3600} hours"
    if len(counted) < BATCH:
        wait = MAX_WAIT - (now - min(merged)) if merged else MAX_WAIT
        return False, (f"{len(counted)} of {BATCH} pull requests: waiting for more, or {wait.seconds // 60 + 1} more "
                       "minutes for the oldest")
    if merged and now - max(merged) < DEBOUNCE:
        wait = DEBOUNCE - (now - max(merged))
        return False, f"{len(counted)} pull requests: waiting {wait.seconds // 60 + 1} more minutes for merges to settle"
    return True, f"{len(counted)} pull requests changed what players run"


# A title's `kind(area): ` prefix (CONTRIBUTING.md "Pull requests") labels the pull request; release names and notes
# read the words after it.
KIND_PREFIX = re.compile(r"^\s*(?:parity|bug|fix|perf|feature|feat|extra|tooling|docs)(?:\([^)]*\))?\s*:\s*", re.I)


def plain(title):
    """A title without its `kind(area): ` prefix, capitalised: "parity(mobs): ghasts fly" -> "Ghasts fly"."""
    rest = KIND_PREFIX.sub("", title.strip(), count=1)
    return rest if rest == title.strip() else rest[:1].upper() + rest[1:]


def clause(title):
    """The start of a pull request title, for a release's name: "Nether portals: vanilla timing" -> "Nether portals"."""
    title = re.sub(r"^(?:fix|fixes|fixed)\s+(?:the\s+)?", "", plain(title), flags=re.I)
    return re.split(r"[:;(]| — | - ", title, maxsplit=1)[0].strip().rstrip(".")


def summary(new, fixed, limit=72):
    """A short name for the release, from its pull request titles, features first."""
    titles = []
    for pr in new[::-1] + fixed[::-1]:  # several pull requests of one feature ("Controllers: ...") name it once
        title = clause(pr["title"])
        if title.lower() not in (t.lower() for t in titles):
            titles.append(title)
    shown, text = [], ""
    for title in titles:
        first = title.split(" ", 1)[0]
        if shown and title[1:2].islower() and first not in PROPER:
            title = title[0].lower() + title[1:]
        candidate = ", ".join(shown + [title])
        if shown and len(candidate) > limit - 20:
            break
        shown.append(title)
        text = candidate
    rest = len(titles) - len(shown)
    if rest:
        noun = ("change" if new else "fix") + ("" if rest == 1 else "es" if not new else "s")
        text += f" and {rest} more {noun}"
    return text


def bullet(pr):
    """A pull request as a line of notes: its title (written for people), its number and the issues it closes."""
    title = re.sub(r"\s*\((?:fixes|closes) #\d+\)|,? stacked on #\d+", "", plain(pr["title"])).strip().rstrip(".")
    issues = "".join(f", fixes #{issue}" for issue in pr["issues"])
    return f"- {title} (#{pr['number']}{issues})"


def known_issues(path=ROOT / "docs" / "known-issues.md"):
    if not path.exists():
        return []
    issues = []
    in_bullet = False
    for line in path.read_text().splitlines():
        if line.startswith("- "):
            issues.append(line.rstrip())
            in_bullet = True
        elif in_bullet and line.startswith((" ", "\t")) and line.strip():
            issues[-1] += " " + line.strip()
        else:
            in_bullet = False
    return issues


def line_intro(kind, version, previous):
    """The paragraph a line's first release (X.Y.0) opens with; the first after the dated tags says what changed."""
    found = semver(version, GAME_PREFIX)
    if not found or found[2] != 0 or found[3] is not None:
        return None
    line = f"{found[0]}.{found[1]}"
    if previous and semver(previous, GAME_PREFIX):
        return f"The first release of Sunrise {line}."
    before = (previous or "").removeprefix(GAME_PREFIX)
    return (f"Sunrise is numbered from now on: this is Sunrise {line}.0" + (f", the release after {before}" if before else "")
            + ". Releases used to be named after the day they came out; from now on they count up from "
            f"{line}.0, and the launcher's too. The launcher installs it by itself once it has updated itself. A server "
            "installed with install.sh that still says it is up to date takes it with "
            f"`sunrise-server update --version {version}`.")


def notes(kind, version, prs, issues, previous=None, repository="horizon-rust/horizon"):
    """(title, body) of a release. previous: the release before it, for a line's first release."""
    shown = [pr for pr in prs if pr["paths"] and "skip-release" not in pr["labels"]]
    new = [pr for pr in shown if "feature" in pr["labels"]]
    fixed = [pr for pr in shown if "feature" not in pr["labels"]]
    title = f"Sunrise {version.removeprefix(GAME_PREFIX)}" + (f": {summary(new, fixed)}" if shown else "")
    intro = line_intro(kind, version, previous)
    parts = [PLAYERS]
    if intro:
        parts.append(intro)
    if not shown:
        parts.append("Rebuilt from the latest main.")
    if new:
        parts.append("### New\n" + "\n".join(map(bullet, new)))
    if fixed:
        parts.append("### Fixed\n" + "\n".join(map(bullet, fixed)))
    if issues and kind == "game":
        parts.append("### Known issues\n" + "\n".join(issues))
    contributors = sorted({pr["author"] for pr in shown if pr["author"] and pr["author"] != MAINTAINER})
    if contributors:
        parts.append("### Contributors\nThanks to " + ", ".join(f"@{name}" for name in contributors) + ".")
    if kind == "game":
        parts.append(PLAYERS_FOOTER)
    parts.append("---\n" + DISCLAIMER)
    return title, "\n\n".join(parts) + "\n"


# The workflow that builds and publishes a Nightly. NIGHTLY_CAP is tools/ci/nightly_set.py's MAX_BUILDS_PER_DAY: past
# it, the train waits for the next UTC day unless a pull request is labelled release-now or the run is forced.
NIGHTLY_WORKFLOW = "nightly-build.yml"
NIGHTLY_CAP = 12


def published_nightly():
    """The published Nightly (the `nightly` pre-release) as {tag_name, published_at}, or None."""
    out = run("gh", "api", f"repos/{REPO}/releases/tags/nightly", "--jq", "{tag_name, published_at}", check=False)
    return json.loads(out) if out.strip() else None


DEV_RUN = "dev build"
STABLE_RUN = "stable build"


def nightly_building():
    """The Nightly's run of `nightly build` under way or waiting, if any: the train starts no second one. A
    Dev or Stable run (named `dev build <commit>` or `stable build <ref>` by
    nightly-build.yml's run-name) has its own concurrency group and never holds a Nightly back; every other run of
    the workflow is a Nightly's."""
    runs = gh_json("run", "list", "--repo", REPO, "--workflow", NIGHTLY_WORKFLOW, "--limit", "20",
                   "--json", "databaseId,status,displayTitle")
    others = (DEV_RUN.split(), STABLE_RUN.split())
    return next((entry["databaseId"] for entry in runs
                 if entry["status"] in ("queued", "in_progress", "waiting", "pending", "requested")
                 and entry["displayTitle"].split(" ", 2)[:2] not in others), None)


def nightly_builds_today(today):
    """The nightly/build-<date>-<n> branches of `today` (every Nightly build pushes one)."""
    out = run("git", "ls-remote", "origin", f"refs/heads/nightly/build-{today}-*")
    return [line for line in out.splitlines() if line.strip()]


def nightly_decision(release, reason, prs, building, built_today, force=False, cap=NIGHTLY_CAP):
    """The train's decision held back by the Nightly build: one at a time, and at most `cap` a day unless a pull
    request is labelled release-now or the run is forced. Returns (release, reason, urgent)."""
    urgent = force or any("release-now" in pr["labels"] for pr in prs if pr["paths"])
    if not release:
        return False, reason, urgent
    if building:
        return False, f"a Nightly is being built (run {building}): the next decision takes what it leaves", urgent
    if built_today >= cap and not urgent:
        return False, f"{built_today} Nightlies today already, the most a day takes: the next goes out after 00:00 UTC", urgent
    return True, reason, urgent


def gather(kind, head, releases=None):
    """The last published release of this kind and the pull requests merged since, with their player-facing
    paths."""
    releases = published_releases() if releases is None else releases
    last = last_release(releases, kind)
    base = last["tag_name"] if last else run("git", "rev-list", "--max-parents=0", head).split()[0]
    return last, merged_between(kind, base, head)


def merged_between(kind, base, head):
    """The pull requests merged into main between base and head, with their player-facing paths ("paths")."""
    exporter_paths = exporters()
    candidates = merged_pull_requests(base, head)
    if not candidates:
        return []
    # Merge subjects survive imports and PR numbers are repository-local. Fetch this
    # repository's merge identities AND note fields in one fully paginated query.
    owned = merged_pull_request_metadata()
    prs = []
    for number, commit in candidates:
        if (number, commit) not in owned:
            continue
        pr = owned[(number, commit)].copy()
        pr["paths"] = [path for path in changed_paths(commit) if player_facing(path, kind, exporter_paths)]
        prs.append(pr)
    return prs


def gate_jobs_passed(run_id, required):
    """Require every named gate on the latest attempt, regardless of non-gate failures."""
    pages = gh_json("api", "--paginate",
                    f"repos/{REPO}/actions/runs/{run_id}/jobs?filter=latest&per_page=100")
    jobs = [job for page in pages for job in page["jobs"] if job["name"] in required]
    return (required == {job["name"] for job in jobs}
            and all(job["status"] == "completed" and job["conclusion"] == "success" for job in jobs))


DAILY_GREEN = "daily-green"
GATE = {"lint", "verify", "verify-static"}


def origin_commit(ref):
    """The commit origin's `ref` (a full ref name) points to, an annotated tag peeled, or "" when origin has no such
    ref. Read from origin every time, never from a local ref a checkout may hold; a failed lookup stops."""
    refs = {name: sha for sha, name in (line.split("\t") for line in run("git", "ls-remote", "origin", ref, ref + "^{}").splitlines())}
    return refs.get(ref + "^{}", refs.get(ref, ""))


def origin_tags():
    """Every tag on origin and the commit it names (annotated tags peeled)."""
    refs = {}
    for line in run("git", "ls-remote", "--tags", "origin").splitlines():
        sha, name = line.split("\t")
        name = name.removeprefix("refs/tags/")
        if name.endswith("^{}"):
            refs[name[:-3]] = sha
        else:
            refs.setdefault(name, sha)
    return refs


def daily_green_commit():
    """The commit origin's tag `daily-green` points to (the newest fully green daily run), or "" when origin has none."""
    return origin_commit(f"refs/tags/{DAILY_GREEN}")


# The release branches, named for the game version they target (docs/release-flow.md "Release branches").
TARGET = "26.3"
NIGHTLY_BRANCH = f"nightly/{TARGET}"
RELEASE_BRANCH = f"release/{TARGET}"
# What a real run on each ref may publish. Any other ref is a dry run: it signs with a throwaway key.
PUBLISHES = {
    "refs/heads/main": {"daily", "nightly", "dev", "sdk"},
    f"refs/heads/{NIGHTLY_BRANCH}": {"nightly"},
    f"refs/heads/{RELEASE_BRANCH}": {"daily", "stable", "sdk"},
}
# The only repository and event a publishing run comes from: every real run is a dispatch (the schedule dispatches).
PUBLISHING_REPOSITORY = "sunrise-org/sunrise"
PUBLISHING_EVENT = "workflow_dispatch"


def publishes(ref, kind):
    """Whether a real run on `ref` (GITHUB_REF, compared exactly: Actions' own comparisons ignore case) may publish
    `kind` (PUBLISHES)."""
    return kind in PUBLISHES.get(ref, ())


def require_ref(ref, kind):
    """Exits unless a real run on `ref` may publish `kind`, naming the refs that may."""
    if not publishes(ref, kind):
        allowed = [name.removeprefix("refs/heads/") for name, kinds in PUBLISHES.items() if kind in kinds]
        raise SystemExit(f"::error::refused: a real {kind} runs only on {' or '.join(allowed)}, not {ref}; "
                         "any other ref is a dry run")


def authorized(kind, dry_run, ref, repository, event):
    """Whether this run publishes `kind`: only when its dry_run input is exactly "false" and its repository, event
    and full ref match exactly. Anything else, a case variant, a tag, a pull request's ref or an unset value, is a
    dry run. Workflows take credentials and write only on this answer (`authorize`), never on their own expressions."""
    return (dry_run == "false" and repository == PUBLISHING_REPOSITORY and event == PUBLISHING_EVENT
            and publishes(ref, kind))


def nightly_branch_head():
    """The head of the nightly branch on origin, or "" until an owner creates it (then the Nightly follows
    `daily-green`, as before the branches)."""
    return origin_commit(f"refs/heads/{NIGHTLY_BRANCH}")


DAILY_WORKFLOW = ".github/workflows/daily.yml"
DAILY_SDK_WORKFLOW = ".github/workflows/prebuilt-game-build.yml"
DAILY_BRANCHES = ("main", RELEASE_BRANCH)
DAILY_EVIDENCE_DAYS = 30
FULL_SHA = re.compile(r"[0-9a-f]{40}", re.ASCII)
RUN_ATTEMPT = re.compile(r"[1-9][0-9]*/[1-9][0-9]*", re.ASCII)


def is_ancestor(older, newer):
    return subprocess.run(["git", "merge-base", "--is-ancestor", older, newer], cwd=ROOT,
                          capture_output=True).returncode == 0


def daily_workflow_source(entry):
    """The trusted immutable workflow SHA of a daily run, or "" when its provenance cannot be proved."""
    branch, source = entry.get("head_branch"), entry.get("head_sha")
    title = entry.get("display_title")
    if (entry.get("path") != DAILY_WORKFLOW or entry.get("event") != PUBLISHING_EVENT
            or branch not in DAILY_BRANCHES or not isinstance(source, str) or not FULL_SHA.fullmatch(source)
            or isinstance(title, str) and title.endswith("(dry run)")):
        return ""
    for field in ("repository", "head_repository"):
        repo = entry.get(field)
        if not isinstance(repo, dict) or repo.get("full_name") != PUBLISHING_REPOSITORY:
            return ""

    # head_branch is only a short name. The SDK is a local reusable workflow in every full daily and
    # every break-glass daily that can prove the required SDK legs, so its ref and SHA witness the caller's.
    workflows = entry.get("referenced_workflows")
    if not isinstance(workflows, list):
        return ""
    sdk = f"{PUBLISHING_REPOSITORY}/{DAILY_SDK_WORKFLOW}"
    ref = f"refs/heads/{branch}"
    found = False
    for workflow in workflows:
        if not isinstance(workflow, dict) or not isinstance(path := workflow.get("path"), str):
            return ""
        if path.partition("@")[0] == sdk:
            if path != f"{sdk}@{source}" or workflow.get("ref") != ref or workflow.get("sha") != source:
                return ""
            found = True
    if not found or run("git", "cat-file", "-t", source, check=False).strip() != "commit":
        return ""

    # The release jobs check out full history from origin. Require the exact tracking ref (no Git name
    # guessing when it's absent), and never fetch an object named by an Actions record.
    tip = run("git", "show-ref", "--verify", "--hash", f"refs/remotes/origin/{branch}", check=False).strip()
    return source if tip and is_ancestor(source, tip) else ""


def attempt_jobs(entry):
    """{name: conclusion} of the completed jobs of the run's own attempt."""
    attempt = entry["run_attempt"]
    pages = gh_json("api", "--paginate", f"repos/{REPO}/actions/runs/{entry['id']}/attempts/{attempt}/jobs?per_page=100")
    return {job["name"]: job["conclusion"] for page in pages for job in page["jobs"]
            if job["status"] == "completed" and job["run_id"] == entry["id"] and job["run_attempt"] == attempt}


def full_daily_verdict(sha, source, jobs):
    """Job evidence for `sha` from one attempt of a trusted workflow source: None without a full verdict.

    The exact summary job names the tested commit, which the immutable workflow source must contain.
    Dry runs and break-glass runs have suffixed summary names.
    """
    summary = jobs.get(f"summary {sha}")
    if summary not in ("success", "failure") or not is_ancestor(sha, source):
        return None
    return summary == "success" and jobs.get(f"green {sha}") == "success"


def daily_run_green(entry, sha, running=False):
    """The URL of an admitted daily run whose own attempt has green job evidence for `sha`, or ""."""
    finished = entry["status"] == "completed" and entry["conclusion"] == "success"
    underway = running and entry["status"] == "in_progress" and entry["conclusion"] is None
    if (finished or underway) and (source := daily_workflow_source(entry)):
        if full_daily_verdict(sha, source, attempt_jobs(entry)):
            return entry["html_url"]
    return ""


def completed_dailies(now=None):
    """The completed daily.yml dispatches of the last DAILY_EVIDENCE_DAYS, newest first."""
    since = ((now or dt.datetime.now(dt.timezone.utc)) - dt.timedelta(days=DAILY_EVIDENCE_DAYS)).date().isoformat()
    pages = gh_json("api", "--paginate", f"repos/{REPO}/actions/workflows/daily.yml/runs?event={PUBLISHING_EVENT}"
                    f"&status=completed&created=%3E%3D{since}&per_page=100")
    return [entry for page in pages for entry in page["workflow_runs"]]


def daily_passed(sha, now=None, this_run=""):
    """The URL of a successful real daily that tested `sha` and has green evidence in its own attempt, or "".
    Only an explicitly named `this_run` ("<id>/<attempt>") may still be in progress; it selects evidence, not a caller.
    """
    if not FULL_SHA.fullmatch(sha or ""):
        return ""
    if this_run:
        if not RUN_ATTEMPT.fullmatch(this_run):
            raise SystemExit(f"::error::refused: a daily run is named <run id>/<attempt>, not {this_run!r}")
        run_id, attempt = this_run.split("/")
        entry = gh_json("api", f"repos/{REPO}/actions/runs/{run_id}/attempts/{attempt}")
        if entry and str(entry["id"]) == run_id and str(entry["run_attempt"]) == attempt:
            return daily_run_green(entry, sha, running=True)
        return ""
    for entry in completed_dailies(now):
        if url := daily_run_green(entry, sha):
            return url
    return ""


# A stable tag `<target>-<semver>` (26.3-0.1.0) is not semver as a whole: read as two parts, ordered by the semver.
STABLE_TAG = re.compile(r"([0-9]+\.[0-9]+(?:\.[0-9]+)?)-(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", re.ASCII)
FIRST_STABLE = (0, 1, 0)


def stable_version(tag):
    """(target, (major, minor, patch)) of a stable release's tag `<target>-<semver>`, else None."""
    match = STABLE_TAG.fullmatch(tag)
    return (match[1], (int(match[2]), int(match[3]), int(match[4]))) if match else None


def stable_key(tag):
    """A stable tag's order: by target, then by the semver within it."""
    target, version = stable_version(tag)
    return tuple(int(part) for part in target.split(".")), version


def next_stable_tag(tags, target=TARGET, version="", own=""):
    """The tag of the next stable release on `target`: `version` (MAJOR.MINOR.PATCH) when given, which must be newer
    than every release on it (0.1.0 for the first), else the patch after the newest, or 0.1.0. `own` is this release's
    tag when it already names the approved commit (a retry), which it need not be newer than."""
    taken = [found[1] for tag in tags if tag != own and (found := stable_version(tag)) and found[0] == target]
    newest = max(taken, default=None)
    if version:
        wanted = stable_version(f"{target}-{version}")
        if not wanted:
            raise SystemExit(f"::error::refused: a stable version is MAJOR.MINOR.PATCH, like 0.1.0, not {version!r}")
        if newest and wanted[1] <= newest:
            raise SystemExit(f"::error::refused: {target}-{version} is not newer than {target}-{'.'.join(map(str, newest))}")
        if not newest and wanted[1] != FIRST_STABLE:
            raise SystemExit(f"::error::refused: the first stable release on {target} is {target}-0.1.0, not {target}-{version}")
        return f"{target}-{version}"
    major, minor, patch = (newest[0], newest[1], newest[2] + 1) if newest else FIRST_STABLE
    return f"{target}-{major}.{minor}.{patch}"


def release_head(head="", dry_run=False, daily=True):
    """The commit a stable release builds, or exits naming the check it failed. A real release takes the full SHA
    approved in the release-candidate check, which must be the head of the release branch (refused while the branch
    is missing) and have a real daily run that passed every leg on exactly it (daily_passed). A dry run takes the
    branch's head or any `head`, and only warns when that run is missing, so the build can be tried first.
    `daily=False` is a break-glass release's, which checks its own evidence instead (break_glass_problems)."""
    if not dry_run and not re.fullmatch(r"[0-9a-f]{40}", head or ""):
        raise SystemExit("::error::refused: a real stable release names the full 40-character commit approved in the "
                         f"release-candidate check (`base`), not {head or 'nothing'}")
    branch = origin_commit(f"refs/heads/{RELEASE_BRANCH}")
    if not branch and (not dry_run or not head):
        raise SystemExit(f"::error::refused: {RELEASE_BRANCH} does not exist; an owner cuts it from a green daily's "
                         "commit")
    sha = run("git", "rev-parse", "--verify", "--quiet", f"{head or branch}^{{commit}}", check=False).strip()
    if not sha:
        raise SystemExit(f"::error::refused: {head or branch} is not a commit of this repository")
    if branch and sha != branch:
        raise SystemExit(f"::error::refused: {sha} is not the head of {RELEASE_BRANCH} ({branch}); a release builds "
                         "only that head")
    if not daily:
        return sha
    evidence = daily_passed(sha)
    if not evidence:
        problem = (f"{sha} has no completed real daily run that passed every leg on it (main or {RELEASE_BRANCH}); "
                   "a stable release ships only such a commit")
        if not dry_run:
            raise SystemExit(f"::error::refused: {problem}")
        print(f"::warning::a real release would stop here: {problem}", file=sys.stderr)
    else:
        print(f"{sha[:9]} passed its daily run: {evidence}", file=sys.stderr)
    return sha


# Break-glass patch releases (docs/release-flow.md "Break-glass patch releases"): a stable patch whose cherry-picks
# need only some daily legs, approved by two people through GitHub environments. The daily legs each changed path
# selects (daily.yml's job names), the first row matching it deciding: a row ending in `/` is that folder (or a file
# or link replacing it), any other an exact path. A path no row matches needs the full daily (fail closed), and so
# does any change to the code that judges a release (judge_graph) or to what the root manifest sets for every
# crate ([patch], [replace], profiles, [workspace.package], [workspace.lints], the resolver; changed_since). The SDK legs (BREAK_GLASS_ALWAYS) always run: sdk-publish waits for them
# (daily_summary.SDK_LEGS), and a release ships only after its SDK. A game crate's change runs every leg that tests
# the game (the ignored-test shards, the Windows rows and the parity harnesses) and is booted headless by the release
# build's own smoke test, which publishing requires.
BREAK_GLASS_ALWAYS = ("sdk", "machine-sample", "fn-replace-check")
BREAK_GLASS_LEGS = (
    ("docs/modding/", ("book-quickstart",)),
    ("tools/book_quickstart.py", ("book-quickstart",)),
    ("docs/", ()),
    ("crates/", ("cargo-sunrise-mod", "parity", "shards", "windows-runtime-leases")),
    ("Cargo.toml", ("cargo-sunrise-mod", "parity", "shards", "windows-runtime-leases")),
    ("Cargo.lock", ("cargo-sunrise-mod", "parity", "shards", "windows-runtime-leases")),
)
# Who approves a break-glass release: each environment's one required reviewer, by GitHub's numeric user id (a login
# can be renamed or taken over), UMCEKO and arismoko. nightly-build.yml binds one job to each environment: GitHub lets
# any one of an environment's reviewers approve, so two people take two environments. On the release branch this
# table changes only through a reviewed pull request, like the code that reads it.
BREAK_GLASS_APPROVERS = {"break-glass-umceko": 11256132, "break-glass-aria": 19542195}


class Release(typing.NamedTuple):
    """A published stable release: its tag, commit and publication time."""
    tag: str
    sha: str
    published_at: dt.datetime


def github_time(text: str) -> dt.datetime:
    """A GitHub API timestamp (`2026-11-20T09:00:00Z`) as an aware datetime; anything else refuses."""
    try:
        found = dt.datetime.fromisoformat(text)
    except (TypeError, ValueError):
        found = None
    if found is None or found.tzinfo is None:
        raise SystemExit(f"::error::refused: {text!r} is not a GitHub timestamp")
    return found


def break_glass_legs(paths):
    """The daily legs a break-glass release changing `paths` needs, sorted, and the paths that need the full daily."""
    legs, unmapped = set(BREAK_GLASS_ALWAYS), []
    for path in paths:
        row = next((selected for prefix, selected in BREAK_GLASS_LEGS
                    if path == prefix or prefix.endswith("/") and (path.startswith(prefix) or path == prefix[:-1])),
                   None)
        if row is None:
            unmapped.append(path)
        else:
            legs.update(row)
    return sorted(legs), unmapped


# The crates that judge a release: its mod API surface (sunrise-stage), its packages and its signature.
JUDGES = ("crates/sunrise-stage", "crates/sunrise-mod-api-gen", "crates/sunrise-mod-surface", "crates/sunrise-mod-package",
          "crates/sunrise-release-sign")
DEPENDENCY_KINDS = ("dependencies", "build-dependencies")


def manifest(sha, path):
    """The TOML file `path` of commit `sha`, or {} where it has none."""
    text = run("git", "show", f"{sha}:{path}", check=False)
    return tomllib.loads(text) if text else {}


def judge_graph(sha):
    """What JUDGES build from at `sha`: (their folders and those of every local crate they build from, their package
    names, the names they take from the root's [workspace.dependencies]). Local crates are path dependencies of every
    kind but dev and every target, directly or through [workspace.dependencies], and every crate the root's [patch]
    or [replace] puts in place of a registry one, followed through their own."""
    root = manifest(sha, "Cargo.toml")
    shared = root.get("workspace", {}).get("dependencies", {})
    patched = [spec for table in root.get("patch", {}).values() for spec in table.values()]
    patched += list(root.get("replace", {}).values())
    todo = list(JUDGES) + [posixpath.normpath(spec["path"]) for spec in patched if isinstance(spec, dict) and "path" in spec]
    folders, packages, inherited = set(), set(), set()
    while todo:
        folder = todo.pop()
        if folder in folders:
            continue
        folders.add(folder)
        crate = manifest(sha, f"{folder}/Cargo.toml")
        if name := crate.get("package", {}).get("name"):
            packages.add(name)
        tables = [crate.get(kind, {}) for kind in DEPENDENCY_KINDS]
        tables += [target.get(kind, {}) for target in crate.get("target", {}).values() for kind in DEPENDENCY_KINDS]
        for name, spec in ((name, spec) for table in tables for name, spec in table.items()):
            base = folder
            if isinstance(spec, dict) and spec.get("workspace"):
                inherited.add(name)
                base, spec = "", shared.get(name)
            if isinstance(spec, dict) and "path" in spec:
                todo.append(posixpath.normpath(posixpath.join(base, spec["path"])))
    return folders, packages, inherited


def locked_closure(sha, packages):
    """The Cargo.lock entries of the local `packages` at `sha` and of everything they depend on, as comparable
    tuples with the packages each one links: a judge built from another version or source of a crate, or linking
    another set of them, builds other code. None when a dependency does not name exactly one entry, which the caller
    treats as a change (fail closed). A dependency is written as Cargo writes it: `name` when one entry has that
    name, `name version` when one has that version or it is the local one (a path package has no source), and
    `name version (source)` otherwise, the source without its `#<commit>` (a git package's [[package]] source has
    it), so edges are compared by the package they resolve to, never by how the selector is spelled."""
    by_name = {}
    for entry in manifest(sha, "Cargo.lock").get("package", []):
        by_name.setdefault(entry["name"], []).append(entry)

    def resolve(selector):
        name, _, rest = selector.partition(" ")
        version, _, source = rest.partition(" ")
        found = [entry for entry in by_name.get(name, []) if not version or entry["version"] == version]
        if source:
            source = source.removeprefix("(").removesuffix(")")
            found = [entry for entry in found if (entry.get("source") or "").split("#", 1)[0] == source]
        elif version and len(found) > 1:
            found = [entry for entry in found if not entry.get("source")]
        return found[0] if len(found) == 1 else None

    def identity(entry):
        return entry["name"], entry["version"], entry.get("source") or ""

    found, seen = set(), set()
    todo = [entry for name in packages for entry in by_name.get(name, []) if not entry.get("source")]
    while todo:
        entry = todo.pop()
        if identity(entry) in seen:
            continue
        seen.add(identity(entry))
        linked = [resolve(selector) for selector in entry.get("dependencies", [])]
        if None in linked:
            return None
        found.add((*identity(entry), entry.get("checksum"), tuple(sorted({identity(link) for link in linked}))))
        todo += linked
    return found


def changed_since(parent_sha, sha):
    """The paths that differ between the two commits (a rename's old and new names both), and those no row may map:
    a symbolic link or submodule on either side, a name holding a line break, anything in the code that judges a
    release on either side (judge_graph), the root manifest when its [patch], [replace] or a [workspace.dependencies]
    entry the judges take changed, or anything else of it that changes how every crate builds (its profiles,
    [workspace.package], [workspace.lints] and the resolver), and Cargo.lock when it changed what the judges build
    from or no longer says it unambiguously (locked_closure)."""
    out = run("git", "-c", "core.quotePath=false", "diff", "-z", "--raw", "--no-renames", parent_sha, sha)
    graphs = [judge_graph(commit) for commit in (parent_sha, sha)]
    judged = graphs[0][0] | graphs[1][0]
    packages, inherited = graphs[0][1] | graphs[1][1], graphs[0][2] | graphs[1][2]
    pins, locks = [], []
    for commit in (parent_sha, sha):
        root = manifest(commit, "Cargo.toml")
        shared = root.get("workspace", {}).get("dependencies", {})
        workspace = root.get("workspace", {})
        pins.append((root.get("patch"), root.get("replace"), {name: shared.get(name) for name in inherited},
                     root.get("profile"), workspace.get("package"), workspace.get("lints"), workspace.get("resolver")))
        locks.append(locked_closure(commit, packages))
    fields, plain, special = out.split("\0"), [], []
    for meta, path in zip(fields[0::2], fields[1::2]):
        modes = meta.split()[:2]
        odd = (any(mode.lstrip(":") in ("120000", "160000") for mode in modes) or "\n" in path or "\r" in path
               or any(path == folder or path.startswith(folder + "/") for folder in judged)
               or path == "Cargo.toml" and pins[0] != pins[1] or path == "Cargo.lock" and (None in locks or locks[0] != locks[1]))
        (special if odd else plain).append(path)
    return plain, special


def break_glass_runs():
    """[(run, {job name: conclusion} of its own attempt)] for completed daily dispatches with trusted sources,
    newest first. Admit both successes and failures: the latter can give a red full-daily verdict."""
    return [(entry, attempt_jobs(entry)) for entry in completed_dailies()
            if entry["status"] == "completed" and entry["conclusion"] in ("success", "failure")
            and daily_workflow_source(entry)]


def legs_passed(sha, legs, runs):
    """The URL of a successful admitted release-branch daily that passed at least `legs` on sha, else "".
    `legs <sha> <leg>` names each passed leg; the tested sha must be contained in the run's workflow source.
    """
    if not FULL_SHA.fullmatch(sha or ""):
        return ""
    for entry, jobs in runs:
        if (entry["head_branch"] == RELEASE_BRANCH and entry["conclusion"] == "success"
                and all(jobs.get(f"legs {sha} {leg}") == "success" for leg in legs)
                and is_ancestor(sha, entry["head_sha"])):
            return entry["html_url"]
    return ""


def full_dailies(sha, runs):
    """[(run, passed)] of the admitted full daily runs of sha, newest first, omitting runs with no full verdict.
    A failed run is red even when its jobs give green evidence.
    """
    if not FULL_SHA.fullmatch(sha or ""):
        return []
    return [(entry, entry["conclusion"] == "success" and verdict)
            for entry, jobs in runs if entry["conclusion"] in ("success", "failure")
            and (verdict := full_daily_verdict(sha, entry["head_sha"], jobs)) is not None]


def newest_full_daily(sha, runs, since=None):
    """(URL, passed) of the newest real full daily run of sha, created after `since` (a datetime) when given, or
    None. Strictly after: the timestamps have one-second resolution, so a run created in the publication's second
    may have started first."""
    return next(((entry["html_url"], passed) for entry, passed in full_dailies(sha, runs)
                 if since is None or github_time(entry["created_at"]) > since), None)


def break_glass_problems(sha, tag, parent):
    """Why `sha` may not ship as the break-glass patch release `tag` (empty when it may), and the daily legs its
    changes select. `parent` is the newest published stable release before it (Release), or None: the one parent
    nightly.stable_parent resolves for every check, stable-mod-api's surface comparison included.
    The rules never waived: a patch of that release, descending from it; dyna-checks' gate passed on a release-branch
    push of exactly `sha`; a daily run of `sha` that passed every leg its changed paths select (break_glass_legs), the
    SDK published, and no full daily of `sha` that failed; and the parent's newest full daily passed, run after its
    publication (nightly-build.yml's full-daily dispatches it after every stable release), whatever kind of release
    the parent was: a run created after the publication ran entirely after it. The surface (unchanged
    `surface.json`, an `identical` diff, the content API) is stable-mod-api's, run at the pick and again before the
    tag."""
    if not parent:
        return [f"there is no published stable release on {TARGET} to patch; a first release takes a full daily"], []
    problems, legs, runs = [], [], break_glass_runs()
    if stable_version(tag)[1][:2] != stable_version(parent.tag)[1][:2]:
        problems.append(f"{tag} is not a patch release of {parent.tag}; a minor or major release takes a full daily")
    if sha == parent.sha or not is_ancestor(parent.sha, sha):
        problems.append(f"{sha} is not a new commit descending from {parent.tag} ({parent.sha})")
    else:
        plain, special = changed_since(parent.sha, sha)
        legs, unmapped = break_glass_legs(plain)
        if unmapped or special:
            problems.append("these changes need the full daily: " + ", ".join(f"`{path!r}`" for path in unmapped + special))
        elif not legs_passed(sha, legs, runs):
            problems.append(f"{sha} has no real daily run that passed the legs its changes select ({', '.join(legs)}); "
                            f"run `gh workflow run daily.yml --ref {RELEASE_BRANCH} -f dry_run=false -f break_glass=true`")
    if (own := newest_full_daily(sha, runs)) and not own[1]:
        problems.append(f"the newest full daily run of {sha} failed ({own[0]})")
    found = newest_full_daily(parent.sha, runs, parent.published_at)
    if not found or not found[1]:
        problems.append(f"the newest full daily run of {parent.tag} ({parent.sha}) since its publication "
                        f"({parent.published_at.astimezone(dt.timezone.utc):%Y-%m-%dT%H:%M:%SZ}) "
                        + (f"failed ({found[0]})" if found else f"is missing in the last {DAILY_EVIDENCE_DAYS} days")
                        + f"; run `gh workflow run daily.yml --ref {RELEASE_BRANCH} -f dry_run=false -f commit={parent.sha}`")
    if not fork_gate_passed(sha, RELEASE_BRANCH):
        problems.append(f"dyna-checks' gate jobs ({', '.join(sorted(GATE))}) did not all pass on a {RELEASE_BRANCH} "
                        f"push of {sha}")
    return problems, legs


def break_glass_approvals(run_id, attempt):
    """`@login (environment): comment` for each of BREAK_GLASS_APPROVERS' environments, or exits. Only a run's first
    attempt counts: the approvals API returns every attempt's history with no attempt on a record, so a re-run could
    reuse an earlier approval (a new dispatch is a new approval). Each environment has exactly one record, approved,
    by its own approver, a user: a rejection, a second record or anyone else refuses, and an environment with no
    reviewer configured records nothing, so it refuses too."""
    if attempt != "1":
        raise SystemExit(f"::error::refused: a break-glass release counts only its run's first attempt's approvals, "
                         f"not attempt {attempt}'s: dispatch a new run")
    pages = gh_json("api", "--paginate", f"repos/{REPO}/actions/runs/{run_id}/approvals?per_page=100")
    by = {}
    for record in (record for page in pages for record in page):
        for name in (environment["name"] for environment in record["environments"]):
            if name not in BREAK_GLASS_APPROVERS:
                continue
            if record["state"] != "approved" or name in by:
                raise SystemExit(f"::error::refused: {name} was {record['state']} by @{record['user']['login']}"
                                 + (" after another review" if name in by else ""))
            by[name] = record
    lines = []
    for name, approver in BREAK_GLASS_APPROVERS.items():
        user = by[name]["user"] if name in by else None
        if not user or user["id"] != approver or user["type"] != "User":
            who = f"@{user['login']} ({user['type']} {user['id']})" if user else "nobody"
            raise SystemExit(f"::error::refused: {name} needs the approval of user {approver}, not {who}")
        comment = " ".join((by[name]["comment"] or "").split()) or "no comment"
        lines.append(f"@{user['login']} ({name}): {comment}")
    return lines


GATE_WAIT = 150 * 60  # seconds: a gate run's queue wait and its longest job, lint (about 25 min), with room
GATE_POLL = 60


def push_gate_run(sha, branch="main"):
    """The newest dyna-checks push run of `branch` on sha (its API record), or None."""
    pages = gh_json("api", "--paginate",
                    f"repos/{REPO}/actions/workflows/dyna-ci.yml/runs?branch={branch}&event=push&head_sha={sha}&per_page=100")
    runs = [entry for page in pages for entry in page["workflow_runs"]
            if entry["head_sha"] == sha and entry["head_branch"] == branch and entry["event"] == "push"]
    return runs[0] if runs else None


def fork_gate_passed(sha, branch="main"):
    """Whether the newest dyna-checks push run of `branch` (main for a Dev build) on sha passed every gate job."""
    found = push_gate_run(sha, branch)
    return found is not None and gate_jobs_passed(found["id"], GATE)


def gate_again(sha):
    """Whether sha's gate passes once its main push run ends, the run started again (its cancelled and failed jobs,
    `gh run rerun --failed`) when the next push cancelled it: that run's later attempt has a concurrency group of its
    own (dyna-ci.yml), so it finishes. A gate job that failed is a red gate, never run again. Waits GATE_WAIT at most."""
    deadline, again = time.monotonic() + GATE_WAIT, False
    while True:
        found = push_gate_run(sha)
        if found is None:
            return False
        if found["status"] != "completed":
            if time.monotonic() > deadline:
                return False
            time.sleep(GATE_POLL)
            continue
        if gate_jobs_passed(found["id"], GATE):
            return True
        if again or found["conclusion"] != "cancelled":
            return False
        print(f"dyna-checks' main push run {found['id']} on {sha[:9]} was cancelled by a later push: running its gate "
              "again", file=sys.stderr)
        run("gh", "run", "rerun", str(found["id"]), "--failed", "--repo", REPO)
        again = True
        time.sleep(GATE_POLL)


def channel_head(channel, head, exception="", daily_run=""):
    """The commit a fork channel builds, or exits naming the check it failed. A nightly builds only the nightly
    branch's head once it exists, before that the commit
    `daily-green` points to (its default), unless `exception` names the owner's one-time grant for another commit
    (docs/release-flow.md "Nightly channel"), which names that commit in full; it still needs the gate. A Dev build
    takes no exception."""
    if channel == "nightly" and exception and not re.fullmatch(r"[0-9a-f]{40}", head or ""):
        raise SystemExit("::error::refused: an exception names the full 40-character commit it grants, not "
                         + (head or "main's head"))
    # Once the nightly branch exists, the Nightly builds its head: the daily fast-forwards it to each green commit.
    branch = nightly_branch_head() if channel == "nightly" and not exception else ""
    green = (branch or daily_green_commit()) if channel == "nightly" and not exception else ""
    head = head or green
    sha = run("git", "rev-parse", "--verify", "--quiet", f"{head or 'refs/remotes/origin/main'}^{{commit}}",
              check=False).strip()
    if not sha:
        raise SystemExit(f"::error::refused: {head} is not a commit of this repository")
    if not is_ancestor(sha, "refs/remotes/origin/main"):
        raise SystemExit(f"::error::refused: {sha} is not on main; a {channel} builds only a commit of main")
    # Both release.yml (before building the SDK) and nightly.py pick use this commit check.
    if channel == "dev" and '"dev-"' not in run(
        "git", "show", f"{sha}:crates/sunrise-core/src/release.rs", check=False
    ):
        raise SystemExit(f"::error::refused: {sha} predates Dev build support. "
                         "Choose a commit that recognizes the dev- version prefix.")
    if channel == "nightly" and not exception and sha != green:
        pointer = f"the head of {NIGHTLY_BRANCH}" if branch else f"the commit `{DAILY_GREEN}` points to"
        raise SystemExit(f"::error::refused: {sha} is not {pointer} ({green or 'none yet'}); "
                         "a Nightly ships only the commit of a fully green daily run, or one the owner granted a "
                         "one-time exception (the `exception` input)")
    # The branch is only a pointer: its head needs passed daily evidence, optionally from a named attempt.
    if branch and sha == branch and not daily_passed(sha, this_run=daily_run):
        raise SystemExit(f"::error::refused: {sha}, the head of {NIGHTLY_BRANCH}, has no completed real daily run "
                         "that passed every leg on it")
    if exception and channel == "nightly":
        grant = f"{sha[:9]} is allowed by a one-time exception instead of a green daily run: {exception}"
        print("::warning::" + grant.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A"), file=sys.stderr)
    if not fork_gate_passed(sha) and not (os.environ.get("GATE_RERUN") == "1" and gate_again(sha)):
        raise SystemExit(f"::error::refused: dyna-checks' gate jobs ({', '.join(sorted(GATE))}) "
                         f"did not all pass on a main push of {sha}; a {channel} needs the gate green on its commit")
    return sha


def build_tag(channel, date, sha):
    """A build's tag and the version it installs as: `nightly-<date>-<short>` or `dev-<date>-<short>`
    (sunrise_release_sign::Channel parses it; the launcher installs under it)."""
    return f"{channel}-{date}-{sha[:7]}"


def green_main_head(eligible=None):
    """The newest commit on main whose gate passed: what an automatic release builds."""
    if REPO == "sunrise-org/sunrise":
        # Runs are ordered by creation, not completion: a late old run must not
        # displace a newer push. Paginate beyond bursts of cancelled main runs.
        pages = gh_json("api", "--paginate",
                        f"repos/{REPO}/actions/workflows/dyna-ci.yml/runs?branch=main&event=push&per_page=100")
        seen = set()
        for page in pages:
            for entry in page["workflow_runs"]:
                sha = entry["head_sha"]
                if entry["head_branch"] != "main" or entry["event"] != "push" or sha in seen:
                    continue
                seen.add(sha)
                if eligible is not None and not eligible(sha):
                    continue
                if gate_jobs_passed(entry["id"], GATE):
                    return sha
        return None
    # 50, not 10: a busy hour of merges cancels many runs, which pushes the last finished one out of a short window.
    runs = gh_json("run", "list", "--repo", REPO, "--workflow", "checks", "--branch", "main", "--event", "push",
                   "--limit", "50", "--json", "headSha,status,conclusion")
    for entry in runs:
        if entry["status"] == "completed" and entry["conclusion"] != "cancelled":
            return entry["headSha"] if entry["conclusion"] == "success" else None
    return None


def write_outputs(values):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as out:
            for name, value in values.items():
                out.write(f"{name}={value}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["decide", "notes", "gate", "authorize", "break-glass-approvals"])
    parser.add_argument("--kind", choices=["game"], default="game")
    parser.add_argument("--head", help="the commit to release (default: the newest commit on main whose checks passed)")
    parser.add_argument("--daily", action="store_true",
                        help="decide: the daily run's nightly, without the batch-size and age rules")
    parser.add_argument("--channel", choices=["nightly", "dev"],
                        help="decide: the fork's kind of build, which picks and checks the commit (see above)")
    parser.add_argument("--exception", default="",
                        help="decide --channel nightly: the owner's one-time grant for a commit that is not daily-green")
    parser.add_argument("--ref", help="decide: a real run's GITHUB_REF, which must publish the channel (PUBLISHES)")
    parser.add_argument("--daily-run", default="",
                        help="decide --channel nightly: the daily proof to use, <run id>/<attempt> (may still be running)")
    parser.add_argument("--publish-kind", choices=["daily", "nightly", "dev", "stable", "sdk"],
                        help="authorize: what the run would publish")
    parser.add_argument("--dry-run", default="true", help="authorize: the run's dry_run input; only \"false\" may publish")
    parser.add_argument("--tag", help="notes: the release's tag")
    parser.add_argument("--force", action="store_true", help="release even without player-facing changes, before merges settle or while paused")
    parser.add_argument("--out", type=Path, help="notes: write title.txt and body.md here")
    parser.add_argument("--now", help="the time to decide at (ISO 8601; default now)")
    parser.add_argument("--smoke", type=Path, help="gate: the release smoke test's report (smoke-test.json)")
    parser.add_argument("--blockers", action="store_true", help="gate: also check that no release-blocker is open")
    args = parser.parse_args()
    now = dt.datetime.fromisoformat(args.now) if args.now else dt.datetime.now(dt.timezone.utc)

    if args.command == "authorize":
        allowed = authorized(args.publish_kind, args.dry_run, os.environ.get("GITHUB_REF", ""),
                             os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("GITHUB_EVENT_NAME", ""))
        print(f"{args.publish_kind}: {'publishes' if allowed else 'dry run'} (dry_run={args.dry_run!r}, "
              f"ref {os.environ.get('GITHUB_REF', '')!r}, event {os.environ.get('GITHUB_EVENT_NAME', '')!r})")
        write_outputs({"publish": str(allowed).lower(), "dry_run": str(not allowed).lower()})
        return 0
    if args.command == "break-glass-approvals":
        # The break-glass job of nightly-build.yml, after both environments' jobs: why, and who approved. A dry run
        # binds no environment, so it has nothing to report.
        reason = " ".join(os.environ.get("BREAK_GLASS_REASON", "").split())
        if os.environ.get("PUBLISH") == "true":
            lines = break_glass_approvals(os.environ["GITHUB_RUN_ID"], os.environ["GITHUB_RUN_ATTEMPT"])
            write_outputs({"note": "; ".join(lines)})
        else:
            lines = ["Dry run: no environment is bound and nothing is approved; a real run waits for "
                     + " and ".join(BREAK_GLASS_APPROVERS) + "."]
        text = f"### Break-glass release\nWhy: {reason}\n\n" + "".join(f"- {line}\n" for line in lines)
        print(text)
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
                summary.write(text)
        return 0
    if args.ref and args.channel:
        require_ref(args.ref, args.channel)
    if args.command == "gate":
        report = json.loads(args.smoke.read_text()) if args.smoke and args.smoke.exists() else None
        problem = gate(report, open_blockers() if args.blockers else [])
        line = f"Not publishing: {problem}" if problem else "Ready to publish: the smoke test passed" + (
            " and no release-blocker is open" if args.blockers else "")
        print(json.dumps(report) if report else "", line, sep="\n")
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
                summary.write(f"### {line}\n")
        return 1 if problem else 0

    # The legacy, unchannelled game decision now uses the daily tag instead of a retired push workflow.
    needs_daily = args.kind == "game" and args.command == "decide" and not args.channel and not args.force
    daily_head = daily_green_commit() if needs_daily else ""
    head = channel_head(args.channel, args.head, args.exception.strip(), args.daily_run.strip()) if args.channel else args.head or (
        daily_head if needs_daily else green_main_head())
    if not head:
        result = {"release": False, "reason": "no fully green daily run yet" if needs_daily else "main's latest checks did not pass"}
        print(json.dumps(result, indent=1))
        write_outputs({"release": "false", "reason": result["reason"]})
        return 0
    head = run("git", "rev-parse", head).strip()
    if args.channel == "dev" and args.command == "decide":
        # The Dev commit is eligible without Nightly's batch, cap, pause or release-blocker checks.
        tag = build_tag("dev", now.date().isoformat(), head)
        result = {"release": True, "reason": f"the gate passed on {head[:9]}: {tag}", "tag": tag,
                  "head": head, "urgent": False}
        print(json.dumps(result, indent=1))
        write_outputs({"release": "true", "reason": result["reason"], "tag": tag, "head": head, "urgent": "false"})
        return 0
    nightly = published_nightly() if args.kind == "game" else None
    if nightly:
        run("git", "fetch", "--quiet", "--force", "origin", "refs/tags/nightly:refs/tags/nightly")
        last, prs = nightly, merged_between("game", "refs/tags/nightly", head)
    else:
        last, prs = gather(args.kind, head)
    if args.command == "notes":
        version = args.tag or "unreleased"
        previous = last["tag_name"] if last and last["tag_name"] != version else None
        title, body = notes(args.kind, version, prs, known_issues(), previous=previous, repository=REPO)
        if args.out:
            args.out.mkdir(parents=True, exist_ok=True)
            (args.out / "title.txt").write_text(title + "\n")
            (args.out / "body.md").write_text(body)
        print(title + "\n\n" + body)
        return 0

    tags = run("git", "tag", "--list").split()
    line = release_line()
    new_line = f"{line[0]}.{line[1]}" if line and first_on_line(tags, GAME_PREFIX, line) and not nightly else None
    blockers = open_blockers() if args.kind == "game" else []
    release, reason = decision(args.kind, prs, now, os.environ.get("RELEASES_PAUSED") == "1", args.force, new_line,
                               blockers, daily=args.daily)
    if release and needs_daily and head != daily_head:
        release, reason = False, f"waiting for a fully green daily on {head[:9]}"
    urgent = False
    if args.kind == "game":
        release, reason, urgent = nightly_decision(release, reason, prs, nightly_building(),
                                                   len(nightly_builds_today(now.date().isoformat())), args.force)
    tag = "nightly"
    result = {
        "release": release, "reason": reason, "tag": tag, "head": head, "urgent": urgent,
        "last": last["tag_name"] if last else None, "last_published": last["published_at"] if last else None,
        "pull_requests": [{key: pr[key] for key in ("number", "title", "labels", "paths")} for pr in prs],
    }
    print(json.dumps(result, indent=1))
    write_outputs({"release": str(release).lower(), "reason": reason, "tag": tag, "head": head,
                   "urgent": str(urgent).lower()})
    return 0


if __name__ == "__main__":
    sys.exit(main())
