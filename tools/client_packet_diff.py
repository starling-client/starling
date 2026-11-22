#!/usr/bin/env python3
"""Compares the serverbound packets of two clients' runs (tools/client_packet_parity.py) route by route.

  client_packet_diff.py .local/packet-parity/runs                    # vanilla-N against sunrise-N
  client_packet_diff.py RUNS --a vanilla --b sunrise --json report.json
  client_packet_diff.py RUNS --strict                                # exit 1 unless bit for bit

The verdict is `strict`: run N of one client against run N of the other (by the manifest's run number), every
serverbound packet of every route in order, compared by its payload bytes as they were on the wire, and every
difference listed, timing ones included. Each row also shows the packet as tools/wire_stream.py decodes it, for
reading only: that form renames entity ids (`self`, `e1`...), turns a click's state id into `stale` and leaves out
bytes it doesn't decode. keep_alive and pong carry ids the server picks, so they are left out of the rows and
checked per connection instead. It counts only finished runs whose manifests (run.json) agree on the recording,
the settings, the course and the vanilla server jar, with one client build per client, and that aren't diagnostic;
each route runs from its own numbered marker (/wire-mark NN) to the client tick the harness recorded at its end, and
must start from the same fixture in both runs of a pair. A run that is missing, unpaired, unfinished, mismatched or
cut short, a marker missing or out of place, and a keep-alive answered with no request, twice, with another id, or
not before the next request are listed as problems, and any problem or difference means not bit for bit. A request
still open when the capture ends is a note: nothing shows it was dropped.

The rest is a summary of what every run reproduces, for reading past real-time input (a key can land a tick
earlier in one run than in another), never a verdict: nothing in it compares tick i with tick i. Per run, each
route gives:
- shapes: the packet names of each client tick, in order (player_input, move_player_pos, client_tick_end...);
- actions: the sequence of packets that aren't moves, with their fields (block sequence numbers as the step
  from the previous one, entity and container ids as wire_stream names them);
- motion: the per-tick vertical and horizontal deltas of the moves, the yaw and pitch steps, and where the
  route ends (position, yaw, pitch);
- counts: how many of each packet the route sent.
A value is a finding when every run of one client has it and no run of the other does (sets), or when the two
clients' ranges over their runs don't overlap (numbers).
"""
import argparse
from collections import Counter
import difflib
import json
import math
from pathlib import Path

try:
    from . import client_packet_routes, wire_stream
except ImportError:
    import client_packet_routes
    import wire_stream

MOVES = wire_stream.MOVES
# Packets the server's timing decides, or that only the network thread sends: never per tick.
UNTIMED = (wire_stream.FRAME | {"chunk_batch_received", "pong", "keep_alive"}) - {"accept_teleportation"}
POSITION_TOLERANCE = 1e-3
# Counts of a packet differ by a few between runs of one client: where a key lands decides a move or two.
COUNT_SLACK = 2
# What a manifest must agree on for two runs to be compared.
MANIFEST_KEYS = ("recording", "settings", "course", "routes", "server_jar")
# Serverbound packets whose payload is an id the server picked: checked by keep_alive_problems, not as rows.
SERVER_IDS = {"keep_alive", "pong"}


def load(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def marks(entries):
    """[(route number or None, client ticks ended before it)] of each marker in the serverbound play stream."""
    out, ticks = [], 0
    for e in entries:
        if e.get("dir") != "c2s" or e.get("state") != "play":
            continue
        if e.get("name") == "client_tick_end":
            ticks += 1
        elif e.get("name") == "chat_command":
            command = wire_stream.decode("chat_command", bytes.fromhex(e.get("data", ""))).get("command")
            if wire_stream.is_mark(command):
                number = command.split(" ", 1)[1] if " " in command else None
                out.append((int(number) if number and number.isdigit() else None, ticks))
    return out


def cut_routes(entries, rows):
    """{route: ticks} from each played row's marker to its end_tick, and the problems found on the way."""
    scenarios, found, problems, out = wire_stream.scenarios(entries), marks(entries), [], {}
    if [n for n, _ in found] != [r["mark"] for r in rows]:
        problems.append({"kind": "markers", "played": [r["mark"] for r in rows], "seen": [n for n, _ in found]})
    by_mark = {n: (i, before) for i, (n, before) in enumerate(found) if n is not None}
    for row in rows:
        if row["mark"] not in by_mark:
            problems.append({"kind": "route_missing", "route": row["route"]})
            continue
        i, before = by_mark[row["mark"]]
        length = row["end_tick"] - before
        if length <= 0 or length > len(scenarios[i]):
            problems.append({"kind": "route_cut", "route": row["route"], "ticks": length, "seen": len(scenarios[i])})
            continue
        out[row["route"]] = scenarios[i][:length]
    return out, problems


def value(v):
    return round(v, 6) if isinstance(v, float) else v


def signature(name, fields, last_sequence):
    """One action as comparable text: a sequence number becomes the step from the previous one (0, which a
    release or a drop always sends, stays 0)."""
    fields = dict(fields)
    if fields.get("sequence"):
        seq = fields.pop("sequence")
        fields["seq_step"] = None if last_sequence[0] is None else seq - last_sequence[0]
        last_sequence[0] = seq
    if name == "use_item":
        fields["yaw"], fields["pitch"] = round(fields["yaw"], 2), round(fields["pitch"], 2)
    if name == "use_item_on":
        fields["cursor"] = [round(c, 4) for c in fields["cursor"]]
    text = ", ".join(f"{k}={value(v)}" for k, v in fields.items())
    return f"{name}{{{text}}}" if text else name


def features(ticks):
    """What one route of one run is compared by."""
    shapes, actions, dys, speeds = Counter(), [], Counter(), Counter()
    first_tick = {}
    counts = Counter()
    last_sequence = [None]
    pos = yaw = pitch = None
    flags = Counter()
    for i, tick in enumerate(ticks):
        names = tuple(n for n, _ in tick["packets"])
        if names:
            shapes[names] += 1
            first_tick.setdefault(("shapes", names), i)
        for name, fields in tick["packets"]:
            counts[name] += 1
            if name in MOVES:
                if "x" in fields:
                    new = (fields["x"], fields["y"], fields["z"])
                    if pos is not None:
                        dy = round(new[1] - pos[1], 5)
                        if dy:
                            dys[dy] += 1
                            first_tick.setdefault(("dy", dy), i)
                        h = round(math.hypot(new[0] - pos[0], new[2] - pos[2]), 4)
                        if h:
                            speeds[h] += 1
                            first_tick.setdefault(("speed", h), i)
                    pos = new
                if "yaw" in fields:
                    yaw, pitch = fields["yaw"], fields["pitch"]
                flags[(fields["on_ground"], fields["horizontal_collision"])] += 1
            elif name not in ("player_input", "client_tick_end"):
                sig = signature(name, fields, last_sequence)
                actions.append(sig)
                first_tick.setdefault(("action", sig), i)
            elif name == "player_input":
                sig = "player_input" + str(fields["keys"])
                actions.append(sig)
                first_tick.setdefault(("action", sig), i)
        for name, _ in tick["frame"]:
            counts[name] += 1
        for name in tick["misplaced"]:
            first_tick.setdefault(("misplaced", name), i)
    # A speed seen once is a transition whose value depends on the tick a key landed on; steady ones repeat.
    speeds = Counter({h: n for h, n in speeds.items() if n >= 2})
    return {"shapes": shapes, "actions": actions, "dy": dys, "speed": speeds, "counts": counts,
            "end": {"pos": pos, "yaw": yaw, "pitch": pitch}, "flags": flags, "first": first_tick,
            "ticks": len(ticks), "misplaced": Counter(n for t in ticks for n in t["misplaced"])}


def keep_alive_problems(entries):
    """Keep-alives in connection order: each request answered once, with its id, before the next one comes; one
    still open when the capture ends is listed as `keep_alive_pending`."""
    problems, waiting = [], None
    for e in entries:
        if e.get("name") != "keep_alive" or e.get("state") != "play":
            continue
        if e["dir"] == "s2c":
            if waiting is not None:
                problems.append({"kind": "keep_alive_unanswered", "id": waiting})
            waiting = e.get("data")
        elif waiting is None or e.get("data") != waiting:
            problems.append({"kind": "keep_alive_unasked", "id": e.get("data"), "waiting": waiting})
        else:
            waiting = None
    if waiting is not None:
        problems.append({"kind": "keep_alive_pending", "id": waiting})
    return problems


def run_number(path, client):
    """The run number of a folder named `<client>-<n>`, or None for any other name (`vanilla-old`, `x-vanilla-1`)."""
    rest = path.name[len(client) + 1:]
    return int(rest) if path.name.startswith(client + "-") and rest.isdigit() else None


def expectation_problems(run, cut):
    """What each played route's own `expect` (client_packet_routes.route) found missing in this run, as problems."""
    out = []
    for name, ticks in cut.items():
        expect = client_packet_routes.ROUTES.get(name, {}).get("expect")
        if expect is None:
            continue
        shots = {label: run / f"{name}-{label}.png" for _, label in client_packet_routes.ROUTES[name]["shots"]
                 if (run / f"{name}-{label}.png").exists()}
        out += [{"kind": "expectation", "route": name, "detail": detail} for detail in expect(ticks, shots)]
    return out


def runs_of(folder, client, indices=None):
    """Every run folder of `client` (a prefix: `vanilla`, `x-sunrise`), in number order, with what it played and the
    problems that keep it out; only the run numbers in `indices` when given."""
    out = []
    numbered = [(run_number(p, client), p) for p in folder.glob(f"{client}-*")]
    for number, run in sorted((n, p) for n, p in numbered if n is not None and (indices is None or n in indices)):
        manifest_path, capture = run / "run.json", run / "capture.jsonl"
        manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}
        entry = {"run": run.name, "number": number, "manifest": manifest, "routes": {}, "ticks": {}, "keep_alive": [],
                 "problems": []}
        if not manifest.get("complete") or not capture.exists():
            entry["problems"].append({"kind": "unfinished" if manifest else "no_manifest"})
        else:
            entries = load(capture)
            cut, entry["problems"] = cut_routes(entries, manifest["played"])
            entry.update(routes={n: features(t) for n, t in cut.items()}, ticks=cut,
                         keep_alive=keep_alive_problems(entries))
            entry["problems"] += expectation_problems(run, cut)
            if manifest.get("diagnostic"):
                entry["problems"].append({"kind": "diagnostic", "mouse_scale": manifest.get("mouse_scale")})
        out.append(entry)
    return out


def rows(ticks):
    """A route's serverbound packets as (wire, shown) rows in order: `wire` is tick, place (tick or frame), name and
    payload hex, what strict compares; `shown` adds the decoded fields for reading. SERVER_IDS are left out."""
    out = []
    for i, tick in enumerate(ticks):
        decoded = {}
        for where in ("packets", "frame"):
            for name, fields in tick[where]:
                decoded.setdefault((where, name), []).append(fields)
        for where, name, payload in tick["raw"]:
            queue = decoded.get((where, name))
            fields = queue.pop(0) if queue else None
            if name in SERVER_IDS:
                continue
            wire = f"{i} {where} {name} {payload}"
            out.append((wire, f"{wire} | {json.dumps(fields, sort_keys=True)}"))
    return out


def strict(a, b, names=("vanilla", "sunrise")):
    """Run N against run N, packet for packet; every problem and every difference, and whether there are none."""
    problems, differences = [], []
    for runs, name in ((a, names[0]), (b, names[1])):
        if not runs:
            problems.append({"kind": "no_runs", "client": name})
        for r in runs:
            problems += [dict(p, run=r["run"]) for p in r["problems"]]
            problems += [dict(p, run=r["run"]) for p in r["keep_alive"]]
    notes = [p for p in problems if p["kind"] == "keep_alive_pending"]
    problems = [p for p in problems if p["kind"] != "keep_alive_pending"]
    manifests = [r["manifest"] for r in a + b if r["manifest"]]
    for key in MANIFEST_KEYS:
        if len({json.dumps(m.get(key), sort_keys=True) for m in manifests}) > 1:
            problems.append({"kind": "manifests_differ", "key": key})
    for runs, name in ((a, names[0]), (b, names[1])):
        if len({r["manifest"].get("client_build") for r in runs if r["manifest"]}) > 1:
            problems.append({"kind": "client_builds_differ", "client": name})
    pairs = ({r["number"]: r for r in a}, {r["number"]: r for r in b})
    for number in sorted(set(pairs[0]) ^ set(pairs[1])):
        problems.append({"kind": "unpaired", "number": number})
    for number in sorted(set(pairs[0]) & set(pairs[1])):
        ra, rb = pairs[0][number], pairs[1][number]
        fixtures = [{p["route"]: p.get("fixture") for p in r["manifest"].get("played", [])} for r in (ra, rb)]
        for route in sorted(set(ra["ticks"]) | set(rb["ticks"])):
            if route not in ra["ticks"] or route not in rb["ticks"]:
                continue
            if fixtures[0].get(route) != fixtures[1].get(route):
                problems.append({"kind": "fixture_differs", "route": route, "runs": [ra["run"], rb["run"]]})
            xs, ys = rows(ra["ticks"][route]), rows(rb["ticks"][route])
            if [wire for wire, _ in xs] == [wire for wire, _ in ys]:
                continue
            lines = list(difflib.unified_diff([shown for _, shown in xs], [shown for _, shown in ys], ra["run"],
                                              rb["run"], n=0, lineterm=""))
            changed = sum(1 for line in lines if line[:1] in "+-" and not line.startswith(("+++", "---")))
            differences.append({"route": route, "runs": [ra["run"], rb["run"]], "rows": [len(xs), len(ys)],
                                "changed": changed, "diff": lines[2:]})
    return {"bit_exact": not problems and not differences, "problems": problems, "differences": differences,
            "notes": notes}


def everywhere(runs, route, kind, key):
    return all(key in r["routes"][route][kind] for r in runs)


def nowhere(runs, route, kind, key):
    return not any(key in r["routes"][route][kind] for r in runs)


def example(runs, route, kind, key):
    r = runs[0]
    return {"run": r["run"], "tick": r["routes"][route]["first"].get((kind, key))}


def excerpt(run, route, tick, around=1):
    ticks = run["ticks"][route]
    if tick is None:
        return []
    lines = []
    for i in range(max(0, tick - around), min(len(ticks), tick + around + 1)):
        parts = []
        for name, fields in ticks[i]["packets"]:
            text = ", ".join(f"{k}={value(v)}" for k, v in fields.items())
            parts.append(f"{name}{{{text}}}" if text else name)
        lines.append(f"tick {i}: " + "; ".join(parts))
    return lines


def compare_sets(a, b, route, kind, names):
    found = []
    keys = set().union(*(r["routes"][route][kind] for r in a + b))
    for key in sorted(keys, key=str):
        for this, other, side in ((a, b, "only_" + names[0]), (b, a, "only_" + names[1])):
            if everywhere(this, route, kind, key) and nowhere(other, route, kind, key):
                ex = example(this, route, kind, key)
                found.append({"route": route, "kind": kind, "side": side, "value": list(key)
                              if isinstance(key, tuple) else key, "example": ex,
                              "excerpt": excerpt(this[0], route, ex["tick"])})
    return found


def disjoint(xs, ys, tolerance=0.0):
    return max(xs) + tolerance < min(ys) or max(ys) + tolerance < min(xs)


def compare_numbers(a, b, route, names):
    found = []
    fa, fb = [r["routes"][route] for r in a], [r["routes"][route] for r in b]
    for field in ("yaw", "pitch"):
        xs, ys = [f["end"][field] for f in fa], [f["end"][field] for f in fb]
        if None not in xs + ys and disjoint(xs, ys, 0.01):
            found.append({"route": route, "kind": "end_" + field, names[0]: xs, names[1]: ys})
    xs, ys = [f["end"]["pos"] for f in fa], [f["end"]["pos"] for f in fb]
    if None not in xs + ys:
        for axis, label in enumerate("xyz"):
            if disjoint([p[axis] for p in xs], [p[axis] for p in ys], POSITION_TOLERANCE):
                found.append({"route": route, "kind": "end_" + label, names[0]: [p[axis] for p in xs],
                              names[1]: [p[axis] for p in ys]})
    for name in sorted(set().union(*(f["counts"] for f in fa + fb)) - UNTIMED - {"chat_command"}):
        xs, ys = [f["counts"][name] for f in fa], [f["counts"][name] for f in fb]
        if disjoint(xs, ys, COUNT_SLACK):
            found.append({"route": route, "kind": "count", "packet": name, names[0]: xs, names[1]: ys})
    for name in sorted(set().union(*(f["misplaced"] for f in fb)) - set().union(*(f["misplaced"] for f in fa))):
        if all(f["misplaced"][name] for f in fb):
            found.append({"route": route, "kind": "misplaced", "packet": name, names[1]: [f["misplaced"][name] for f in fb]})
    return found


def compare_actions(a, b, route, names):
    """Differences of the action sequences that every pair of runs (one of each client) shows."""
    seen = None
    for ra in a:
        for rb in b:
            sa, sb = ra["routes"][route]["actions"], rb["routes"][route]["actions"]
            chunks = set()
            for op, i1, i2, j1, j2 in difflib.SequenceMatcher(None, sa, sb, autojunk=False).get_opcodes():
                if op != "equal":
                    chunks.add((tuple(sa[i1:i2]), tuple(sb[j1:j2])))
            seen = chunks if seen is None else seen & chunks
    return [{"route": route, "kind": "actions", names[0]: list(x), names[1]: list(y)} for x, y in sorted(seen or [])]


def report(a, b, names=("vanilla", "sunrise")):
    verdict = strict(a, b, names)
    a, b = [r for r in a if r["routes"]], [r for r in b if r["routes"]]
    routes = [r for r in a[0]["routes"] if all(r in run["routes"] for run in a + b)] if a and b else []
    findings = []
    for route in routes:
        for kind in ("shapes", "dy", "speed"):
            findings += compare_sets(a, b, route, kind, names)
        findings += compare_actions(a, b, route, names)
        findings += compare_numbers(a, b, route, names)
    return {"strict": verdict, "runs": {names[0]: [r["run"] for r in a], names[1]: [r["run"] for r in b]},
            "routes": routes, "findings": findings}


def report_runs(folder, names=("vanilla", "sunrise"), indices=None):
    return report(runs_of(Path(folder), names[0], indices), runs_of(Path(folder), names[1], indices), names)


def describe(rep):
    verdict = rep["strict"]
    lines = [f"strict: {'bit for bit' if verdict['bit_exact'] else 'NOT bit for bit'}: "
             f"{len(verdict['problems'])} problems, {len(verdict['differences'])} routes differ"]
    for p in verdict["problems"]:
        lines.append(f"- problem: {json.dumps(p)[:300]}")
    for n in verdict["notes"]:
        lines.append(f"- note: {json.dumps(n)[:300]}")
    for d in verdict["differences"]:
        lines.append(f"- [{d['route']}] {d['runs'][0]} vs {d['runs'][1]}: {d['changed']} rows of {d['rows']}")
        for line in d["diff"][:6]:
            lines.append(f"      {line[:300]}")
    lines += ["summary (what every run reproduces; not a verdict):",
              f"runs: {rep['runs']}", f"routes: {len(rep['routes'])}", f"findings: {len(rep['findings'])}"]
    for f in rep["findings"]:
        rest = {k: v for k, v in f.items() if k not in ("route", "kind", "excerpt")}
        lines.append(f"- [{f['route']}] {f['kind']}: {json.dumps(rest)[:400]}")
        for line in f.get("excerpt", [])[:3]:
            lines.append(f"      {line[:300]}")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("runs", type=Path)
    parser.add_argument("--a", default="vanilla")
    parser.add_argument("--b", default="sunrise")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--strict", action="store_true", help="exit 1 unless every packet of every route matches")
    args = parser.parse_args()
    rep = report_runs(args.runs, (args.a, args.b))
    if args.json:
        args.json.write_text(json.dumps(rep, indent=1))
    print(describe(rep))
    if args.strict and not rep["strict"]["bit_exact"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
