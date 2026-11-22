#!/usr/bin/env python3
"""Code areas and the tests that cover them, from tools/test_areas.toml.

  python3 tools/test_areas.py list                           # every area and what it runs
  python3 tools/test_areas.py list --changed origin/main     # the areas a branch touches
  python3 tools/test_areas.py list --changed-files paths.txt # the same for listed paths
  python3 tools/test_areas.py run explosions                 # its Rust tests, then its harnesses
  python3 tools/test_areas.py run explosions --daily --lan   # also its daily tests and its LAN checks
  python3 tools/test_areas.py run --changed origin/main      # the touched areas; harnesses as parity.py --changed picks them
  python3 tools/test_areas.py coverage                       # tracked crate files no area claims

Picking from a diff stays with tools/parity.py --changed (harnesses) and tools/ci/test_scope.py (crates): `run`
from a diff also tests the crates of the paths no area claims, as test_scope.py picks them. An area names a set of
tests, adds the daily and LAN tests nothing picks from a diff, and is how to ask for "the explosion tests". `--daily`
needs the prepared data (SUNRISE_TEST_RESOURCES), as the daily run does. `--lan` builds and starts two games (tools/lan_e2e.py); the
network area holds every LAN check. An opt-in LAN check (LAN_OPT_IN) runs only on a game built for it: `--lan-bin-dir DIR`.
"""

import argparse
import fnmatch
import re
import subprocess
import sys
import tomllib
from collections import defaultdict
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]
# The repository root, appended so a script run finds `tools` without shadowing anything already importable.
sys.path.append(str(ROOT))

MAP = ROOT / "tools" / "test_areas.toml"
LAN = ROOT / "tools" / "lan_e2e.py"
# lan_e2e.py's check names: `CHECKS = [...]` and the `CHECKS += [...]` beside each later check.
CHECK_LISTS = re.compile(r"^CHECKS\s*\+?=\s*\[(.*?)\]", re.M | re.S)
QUOTED = re.compile(r"'([^']+)'")
# The checks lan_e2e.py's default run leaves out, and the game each needs: its own build doesn't have it.
LAN_OPT_IN = {"recipes-smod": "a game built with the native Recipes mod"}
HARNESS = re.compile(r"^tools/test_(\w+)_parity\.py$")
TARGET = re.compile(r"^(?:lib|(?:bin|test):[\w-]+)$")


class Cargo(NamedTuple):
    package: str
    target: str  # `lib`, `bin:NAME` or `test:NAME`, as the daily rows name targets
    filter: str  # a libtest filter: a substring of the test's path; empty for all of the target's tests


class Area(NamedTuple):
    name: str
    about: str
    paths: tuple[str, ...]
    cargo: tuple[Cargo, ...]
    harnesses: tuple[str, ...]
    lan: tuple[str, ...]


class Map(NamedTuple):
    areas: tuple[Area, ...]
    escalate: tuple[str, ...]
    unassigned: tuple[str, ...]  # harness names no area claims yet


class Selection(NamedTuple):
    areas: tuple[Area, ...]
    unmapped: tuple[str, ...]  # changed paths no area claims
    escalated: tuple[str, ...]  # changed paths that can reach every area


def load(path=MAP):
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    areas = tuple(Area(
        area["name"], area.get("about", ""), tuple(area.get("paths", ())),
        tuple(Cargo(entry["package"], entry["target"], entry.get("filter", "")) for entry in area.get("cargo", ())),
        tuple(area.get("harnesses", ())), tuple(area.get("lan", ())),
    ) for area in data["area"])
    return Map(areas, tuple(data.get("escalate", ())), tuple(data.get("unassigned", {}).get("harnesses", ())))


def matching(patterns, names):
    """The names any pattern matches, in their own order (fnmatch: `*` also crosses `/`)."""
    return [name for name in names if any(fnmatch.fnmatchcase(name, pattern) for pattern in patterns)]


def harness_names():
    """The harnesses tools/parity.py runs by name, and the proofs it refuses: {name: how to run it by hand}."""
    from tools import parity
    runnable = sorted({parity.base_name(name) for name in parity.harnesses()})
    return runnable, dict(parity.MANUAL)


def lan_checks():
    """Every tools/lan_e2e.py check, read from its source: importing it would load the whole driver."""
    text = LAN.read_text(encoding="utf-8")
    return [name for block in CHECK_LISTS.findall(text) for name in QUOTED.findall(block)]


def tracked():
    """Tracked files, and new ones not yet added (a patch applied but not committed)."""
    return subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT,
                          capture_output=True, text=True, check=True).stdout.split("\0")[:-1]


def daily_rows():
    """Every daily row (tools/ci/ignored-tests/): what `run --daily` chooses from."""
    from tools.ci import ignored_tests
    return [row for row in ignored_tests.listed_entries() if row.mark == "daily"]


def covers(entry, package, target, test):
    return entry.package == package and entry.target == target and entry.filter in test


def target_flags(target):
    kind, _, name = target.partition(":")
    return ["--lib"] if kind == "lib" else [f"--{kind}", name]


def select(table, paths):
    """The areas changed paths touch, in the map's order, and the paths no area claims."""
    chosen, unmapped, escalated = set(), [], []
    for path in paths:
        if matching(table.escalate, [path]):
            escalated.append(path)
            continue
        hits = {area.name for area in table.areas if matching(area.paths, [path])}
        # A harness script belongs to the areas that list the harness.
        if harness := HARNESS.match(path):
            hits |= {area.name for area in table.areas if matching(area.harnesses, [harness[1]])}
        # Documentation nobody builds from selects nothing, as in tools/ci/test_scope.py.
        inert = (path.startswith("docs/") or path.endswith(".md")) and not path.startswith(".github/")
        if not hits and not inert:
            unmapped.append(path)
        chosen |= hits
    return Selection(tuple(area for area in table.areas if area.name in chosen), tuple(unmapped), tuple(escalated))


def plan(area, harnesses, checks, rows, daily=False, lan=False, by_hand=(), lan_bins=None):
    """The commands that run one area's tests, cheapest first, and what each tier holds. `harnesses` are the ones
    tools/parity.py runs, `by_hand` the proofs it refuses; `lan_bins` is a game built for the opt-in LAN checks."""
    from tools.ci import ignored_tests
    commands = []
    for entry in area.cargo:
        commands.append(["cargo", "test", "-p", entry.package, *target_flags(entry.target),
                         *(["--", entry.filter] if entry.filter else [])])
    chosen = [row for row in rows if any(covers(entry, row.package, row.reference, row.test) for entry in area.cargo)]
    if daily:
        groups = defaultdict(list)
        for row in chosen:
            groups[row.package, row.reference].append(row.test)
        for (package, target), tests in sorted(groups.items()):
            # As the daily run builds them: the workspace's features where the package's own shard doesn't run them.
            features = [] if ignored_tests.shard_runs(package, target) else \
                ignored_tests.workspace_features().get(package, [])
            commands.append(["cargo", "test", "-p", package, *(["--features", ",".join(features)] if features else []),
                             *target_flags(target), "--", "--ignored", "--exact", *sorted(tests)])
    named = matching(area.harnesses, harnesses)
    if named:
        commands.append([sys.executable, "tools/parity.py", *named])
    picked = matching(area.lan, checks)
    runnable = [check for check in picked if lan_bins or check not in LAN_OPT_IN]
    if lan and runnable:
        commands.append([sys.executable, "tools/lan_e2e.py", *(["--bin-dir", lan_bins] if lan_bins else []),
                         *(flag for check in runnable for flag in ("--check", check))])
    return commands, {"daily": chosen, "harnesses": named,
                      "by_hand": matching(area.harnesses, by_hand), "lan": picked}


def describe(area, tiers):
    reasons = defaultdict(int)
    for row in tiers["daily"]:
        reasons[row.reason] += 1
    lines = [f"{area.name}: {area.about}"]
    lines += [f"  rust       cargo test -p {entry.package} {' '.join(target_flags(entry.target))}"
              + (f" -- {entry.filter}" if entry.filter else "") for entry in area.cargo]
    lines.append(f"  daily      {len(tiers['daily'])} tests"
                 + (f" ({', '.join(f'{reason} {count}' for reason, count in sorted(reasons.items()))})" if reasons else ""))
    lines.append(f"  harnesses  {len(tiers['harnesses'])}: {' '.join(tiers['harnesses'])}")
    if tiers["by_hand"]:
        lines.append(f"  by hand    {len(tiers['by_hand'])}: {' '.join(tiers['by_hand'])}")
    lines.append(f"  lan        {len(tiers['lan'])}: {' '.join(tiers['lan'])}")
    return "\n".join(lines)


def fallback(selection, workspace):
    """The crate tests for a diff's paths no area claims, as tools/ci/test_scope.py picks them for CI's verify (their
    packages and those depending on them), or the whole workspace's when a path reaches every area."""
    packages = None if selection.escalated else workspace.scope(selection.unmapped)
    chosen = ["--workspace"] if packages is None else [flag for name in sorted(packages) for flag in ("-p", name)]
    return ["cargo", "test", *chosen, "--lib", "--bins", "--tests"]


def changed(args):
    """The changed paths --changed or --changed-files names, or None for neither."""
    if args.changed:
        from tools import parity
        paths = parity.changed_files(args.changed)
        if paths is None:
            sys.exit(f"no merge base with {args.changed}")
        return sorted(paths)
    if args.changed_files:
        return [line.strip() for line in Path(args.changed_files).read_text(encoding="utf-8").splitlines() if line.strip()]
    return None


def report(selection):
    names = " ".join(area.name for area in selection.areas) or "none"
    print(f"areas: {names}")
    if selection.escalated:
        print(f"these reach every area, so `run` tests the whole workspace: {' '.join(selection.escalated)}")
    if selection.unmapped:
        print(f"in no area ({len(selection.unmapped)}), so `run` tests their crates as tools/ci/test_scope.py picks "
              f"them: {' '.join(selection.unmapped)}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("list", "run"):
        command = commands.add_parser(name)
        command.add_argument("areas", nargs="*", help="area names (default: all for list, none for run)")
        source = command.add_mutually_exclusive_group()
        source.add_argument("--changed", metavar="REF", help="the areas the changes since the merge base with REF touch")
        source.add_argument("--changed-files", metavar="FILE", help="the areas these paths (one per line) touch")
        command.add_argument("--daily", action="store_true", help="also the area's daily tests (needs prepared data)")
        command.add_argument("--lan", action="store_true", help="also the area's LAN checks (starts two games)")
    commands.choices["run"].add_argument("--lan-bin-dir", metavar="DIR", help="prebuilt sunrise-client and sunrise-ctl "
                                         "for lan_e2e.py, built for the opt-in LAN checks, which run only on them")
    coverage = commands.add_parser("coverage")
    coverage.add_argument("--files", action="store_true", help="list each tracked file no area claims")
    args = parser.parse_args(argv)
    table = load()

    if args.command == "coverage":
        files = [path for path in tracked() if path.startswith("crates/")]
        loose = [path for path in files if not any(matching(area.paths, [path]) for area in table.areas)]
        by_crate = defaultdict(int)
        for path in loose:
            by_crate["/".join(path.split("/")[:2])] += 1
        print(f"{len(files) - len(loose)} of {len(files)} files under crates/ are in an area")
        for crate, count in sorted(by_crate.items(), key=lambda item: -item[1]):
            print(f"  {count:5}  {crate}")
        if args.files:
            print("\n".join(loose))
        return 0

    known = {area.name: area for area in table.areas}
    if unknown := [name for name in args.areas if name not in known]:
        parser.error(f"no area {', '.join(unknown)}; the areas: {', '.join(known)}")
    paths = changed(args)
    if paths is not None:
        if args.areas:
            parser.error("name areas, or pass --changed/--changed-files, not both")
        selection = select(table, paths)
        report(selection)
        areas = selection.areas
    elif args.areas:
        areas = tuple(known[name] for name in args.areas)
    elif args.command == "list":
        areas = table.areas
    else:
        parser.error("run needs area names, --changed or --changed-files")

    (harnesses, manual), checks, rows = harness_names(), lan_checks(), daily_rows()
    lan_bins = args.lan_bin_dir if args.command == "run" else None
    plans = [(area, *plan(area, harnesses, checks, rows, args.daily, args.lan, manual, lan_bins)) for area in areas]
    if args.command == "list":
        for area, _, tiers in plans:
            print(describe(area, tiers))
        return 0

    steps = []
    for _, area_commands, _ in plans:
        steps += [command for command in area_commands if command[1:2] != ["tools/parity.py"]]
    if paths is not None:
        if selection.unmapped or selection.escalated:
            from tools.ci import test_scope
            steps.append(fallback(selection, test_scope.Workspace(test_scope.cargo_metadata())))
        # From a diff, harnesses come from the runner's traced dependencies, not the areas' lists.
        source = ["--changed", args.changed] if args.changed else ["--changed-files", str(Path(args.changed_files).resolve())]
        steps.append([sys.executable, "tools/parity.py", *source])
    else:
        named = sorted({name for _, _, tiers in plans for name in tiers["harnesses"]})
        if named:
            steps.append([sys.executable, "tools/parity.py", *named])
    # One LAN run for every chosen check: each run starts two games.
    lan = [command for command in steps if command[1:2] == ["tools/lan_e2e.py"]]
    if lan:
        wanted = {flag for command in lan for flag in command[2:] if flag != "--check"}
        checks_wanted = [check for check in checks if check in wanted]  # the driver's own order
        steps = [command for command in steps if command not in lan]
        steps.append([sys.executable, "tools/lan_e2e.py", *(["--bin-dir", lan_bins] if lan_bins else []),
                      *(flag for check in checks_wanted for flag in ("--check", check))])
    # What the areas hold that this run leaves to another.
    notes = [f"{name} is run by hand: {manual[name]}"
             for name in sorted({name for _, _, tiers in plans for name in tiers["by_hand"]})]
    if args.lan and not lan_bins:
        opt_in = sorted({check for _, _, tiers in plans for check in tiers["lan"] if check in LAN_OPT_IN})
        notes += [f"{check} needs {LAN_OPT_IN[check]}: pass it with --lan-bin-dir" for check in opt_in]

    failed = []
    for command in dict.fromkeys(tuple(command) for command in steps):
        print("$ " + " ".join(command), flush=True)
        if subprocess.run(command, cwd=ROOT).returncode != 0:
            failed.append(" ".join(command))
    if notes:
        print("not run here:\n  " + "\n  ".join(notes))
    if failed:
        print("failed:\n  " + "\n  ".join(failed))
        return 1
    print("passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
