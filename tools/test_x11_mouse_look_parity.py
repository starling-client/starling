#!/usr/bin/env python3
"""Vanilla 26.3 X11 mouse-look bit-exact parity harness (#3022).

Writes the scripted 4-route XTEST recording (look_x, look_y, look_counts, regrab), the recorded
vanilla 26.3 witness JSON, and meta.json into .local/witness/<version>/x11_mouse_look/ (gitignored).
Replays the recording on Xvfb with unscaled input through tools/client_packet_parity.py and compares
every serverbound rotation's raw IEEE-754 float bits and look-dependent action fields against the
pinned vanilla 26.3 client.

  python3 tools/test_x11_mouse_look_parity.py --record --runs 2
  python3 tools/test_x11_mouse_look_parity.py --bin target/fast --runs 2
  python3 tools/test_x11_mouse_look_parity.py --live --bin target/fast --runs 2
  python3 tools/test_x11_mouse_look_parity.py --compare RUNS --a vanilla --b sunrise
  python3 tools/test_x11_mouse_look_parity.py --compare-witness RUNS --client sunrise --witness FILE
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import client_packet_diff as diff, client_packet_routes as routes, input_recording, lab, packet_parity_runs

WITNESS_ENV = "SUNRISE_X11_MOUSE_LOOK_WITNESS"
WITNESS_DIR_NAME = "x11_mouse_look"
RECORDING_NAME = "x11_mouse_look_recording.jsonl"
WITNESS_NAME = "x11_mouse_look.json"
META_NAME = "meta.json"
DEFAULT_LABEL = "x11-mouse-look-26.3"
ROUTES = list(routes.MOUSE_LOOK)


def default_witness_dir():
    """Gitignored local folder holding the generated recording, vanilla witness, and meta.json."""
    return lab.LOCAL / "witness" / lab.VERSION / WITNESS_DIR_NAME


def default_recording_path():
    """Location of the generated 4-route XTEST input recording under .local."""
    return default_witness_dir() / RECORDING_NAME


def default_witness_path():
    """Location of the recorded vanilla 26.3 X11 mouse-look witness under .local (or WITNESS_ENV)."""
    override = os.environ.get(WITNESS_ENV)
    if override:
        return Path(override)
    return default_witness_dir() / WITNESS_NAME


def write_recording(recording_path=None):
    """Writes the scripted 4-route mouse-look XTEST recording to recording_path (default: .local)."""
    path = Path(recording_path) if recording_path is not None else default_recording_path()
    header = {**input_recording.FORMAT, "minecraft": lab.VERSION, "routes": ROUTES}
    events = sorted(routes.recording(ROUTES), key=lambda e: e["t"])
    content = "".join(json.dumps(row) + "\n" for row in [header, *events])
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists() or path.read_text(encoding="utf-8") != content:
        path.write_text(content, encoding="utf-8")
    return path


def ensure_recording(recording_path=None):
    """Ensures the default .local recording exists and matches client_packet_routes.MOUSE_LOOK."""
    default_path = default_recording_path()
    if recording_path is None:
        return write_recording(default_path)
    path = Path(recording_path)
    if path.resolve() == default_path.resolve():
        return write_recording(path)
    return path


def _load_packet_parity():
    # Imported lazily so importing this module in unit tests never mutates sys.path.
    from tools import client_packet_parity
    return client_packet_parity


def _relative_or_str(path):
    try:
        return Path(path).resolve().relative_to(lab.ROOT.resolve()).as_posix()
    except ValueError:
        return str(path)


def _java_version():
    done = subprocess.run(["java", "-version"], capture_output=True, text=True, check=True)
    text = (done.stderr or done.stdout).strip()
    return text.splitlines()[0] if text else None


def _witness_is_fresh(witness_path, recording_path, runs, *, settings_digest=None, server_jar_sha256=None):
    witness_path = Path(witness_path)
    if not witness_path.is_file():
        return False
    try:
        witness = json.loads(witness_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return False
    if {k: witness.get(k) for k in diff.LOOK_WITNESS_FORMAT} != diff.LOOK_WITNESS_FORMAT:
        return False
    if witness.get("runs_verified", 0) < runs:
        return False
    if witness.get("recording_file_sha256") != lab.digest(recording_path, "sha256"):
        return False
    if settings_digest is not None and witness.get("settings_digest") != settings_digest:
        return False
    if server_jar_sha256 is not None and witness.get("vanilla_server_jar_sha256") != server_jar_sha256:
        return False
    return True


def record_witness(recording_path=None, witness_path=None, runs=2, label=DEFAULT_LABEL, rebuild_world=False):
    """Runs vanilla 26.3 on Xvfb via client_packet_parity and writes the verified witness JSON and meta.json."""
    recording_path = ensure_recording(recording_path)
    witness_path = Path(witness_path) if witness_path is not None else default_witness_path()
    recording = input_recording.read(recording_path)
    route_names = [name for name, _ in input_recording.routes(recording)]
    if route_names != ROUTES:
        raise ValueError(f"{recording_path}: expected routes {ROUTES}, got {route_names}")
    java_ver = _java_version()
    harness = _load_packet_parity()
    if rebuild_world or not harness.course_current():
        harness.build_world()
    indices = set(range(1, runs + 1))
    for i in sorted(indices):
        print(f"vanilla {lab.VERSION} run {i}", flush=True)
        harness.run_once("vanilla", i, recording, bins=None, label=label, mouse_scale=1.0, replace=True)
    prefix = packet_parity_runs.prefix("vanilla", label)
    captured = diff.runs_of(harness.WORK / "runs", prefix, indices)
    witness = diff.build_look_witness(
        captured, ROUTES,
        recording_path=_relative_or_str(recording_path),
        recording_file_sha256=lab.digest(recording_path, "sha256"),
        java_version=java_ver,
    )
    witness_path.parent.mkdir(parents=True, exist_ok=True)
    witness_path.write_text(json.dumps(witness, indent=2) + "\n", encoding="utf-8")
    meta_path = witness_path.parent / META_NAME
    lab.write_json(meta_path, {
        "game_version": lab.VERSION,
        "java_version": java_ver,
        "command": f"python3 tools/test_x11_mouse_look_parity.py --record --runs {runs}",
        "harness": "tools/test_x11_mouse_look_parity.py",
        "witness_env": WITNESS_ENV,
        "recording": _relative_or_str(recording_path),
        "witness": _relative_or_str(witness_path),
        "runs_verified": witness["runs_verified"],
        "input_hashes": {
            "research-lock.json": lab.digest(lab.LOCK, "sha256"),
            "recording_file_sha256": witness["recording_file_sha256"],
            "recording_digest": witness["recording_digest"],
            "settings_digest": witness["settings_digest"],
            "course_digest": witness["course_digest"],
            "vanilla_server_jar_sha256": witness["vanilla_server_jar_sha256"],
            "vanilla_client_jar_sha256": witness["vanilla_client_jar_sha256"],
            "tools/client_packet_routes.py": lab.digest(lab.ROOT / "tools/client_packet_routes.py", "sha256"),
            "tools/client_packet_parity.py": lab.digest(lab.ROOT / "tools/client_packet_parity.py", "sha256"),
            "tools/test_x11_mouse_look_parity.py": lab.digest(lab.ROOT / "tools/test_x11_mouse_look_parity.py", "sha256"),
        },
        "output_sha256": lab.digest(witness_path, "sha256"),
    })
    print(f"recorded {lab.VERSION} witness ({runs} runs verified): {witness_path}", flush=True)
    return witness


def verify_live(recording_path, bins, runs=2, label=DEFAULT_LABEL, rebuild_world=False, out_json=None):
    """Runs vanilla 26.3 and Sunrise on Xvfb with unscaled input and compares raw float bits."""
    recording_path = ensure_recording(recording_path)
    recording = input_recording.read(recording_path)
    route_names = [name for name, _ in input_recording.routes(recording)]
    if route_names != ROUTES:
        raise ValueError(f"{recording_path}: expected routes {ROUTES}, got {route_names}")
    harness = _load_packet_parity()
    if rebuild_world or not harness.course_current():
        harness.build_world()
    indices = set(range(1, runs + 1))
    for client in ("vanilla", "sunrise"):
        for i in sorted(indices):
            print(f"{client} run {i}", flush=True)
            harness.run_once(client, i, recording, bins=bins, label=label, mouse_scale=1.0, replace=True)
    names = tuple(packet_parity_runs.prefix(c, label) for c in ("vanilla", "sunrise"))
    report = diff.compare_look_runs(
        diff.runs_of(harness.WORK / "runs", names[0], indices),
        diff.runs_of(harness.WORK / "runs", names[1], indices),
        ROUTES, names,
    )
    if out_json:
        Path(out_json).write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(diff.describe_look(report), flush=True)
    return report


def verify_against_witness(recording_path, witness_path, bins, runs=2, label=DEFAULT_LABEL,
                           rebuild_world=False, out_json=None):
    """Runs Sunrise on Xvfb with unscaled input and compares raw float bits against the local 26.3 witness."""
    recording_path = ensure_recording(recording_path)
    recording = input_recording.read(recording_path)
    route_names = [name for name, _ in input_recording.routes(recording)]
    if route_names != ROUTES:
        raise ValueError(f"{recording_path}: expected routes {ROUTES}, got {route_names}")
    witness = json.loads(Path(witness_path).read_text(encoding="utf-8"))
    harness = _load_packet_parity()
    if rebuild_world or not harness.course_current():
        harness.build_world()
    indices = set(range(1, runs + 1))
    for i in sorted(indices):
        print(f"sunrise run {i}", flush=True)
        harness.run_once("sunrise", i, recording, bins=bins, label=label, mouse_scale=1.0, replace=True)
    prefix = packet_parity_runs.prefix("sunrise", label)
    report = diff.compare_look_against_witness(
        diff.runs_of(harness.WORK / "runs", prefix, indices), witness, prefix)
    if out_json:
        Path(out_json).write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(diff.describe_look(report), flush=True)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--record", action="store_true",
                        help="run vanilla 26.3 on Xvfb and write the recorded witness JSON and meta.json under .local")
    parser.add_argument("--live", action="store_true",
                        help="run both vanilla 26.3 and Sunrise live on Xvfb instead of using a recorded witness")
    parser.add_argument("--compare", type=Path,
                        help="offline: compare two captured run prefixes in this runs folder (no game launch)")
    parser.add_argument("--compare-witness", type=Path,
                        help="offline: compare captured runs in this runs folder against --witness (no game launch)")
    parser.add_argument("--a", default="vanilla", help="first run prefix for --compare")
    parser.add_argument("--b", default="sunrise", help="second run prefix for --compare")
    parser.add_argument("--client", default="sunrise", help="run prefix for --compare-witness")
    parser.add_argument("--recording", type=Path, default=None,
                        help="26.3 input recording JSONL to replay (default: generated under .local/witness)")
    parser.add_argument("--witness", type=Path, default=None,
                        help=f"recorded vanilla 26.3 witness JSON (default: ${WITNESS_ENV} or .local/witness)")
    parser.add_argument("--bin", type=Path, default=None, help="folder with the built Sunrise binary")
    parser.add_argument("--runs", type=int, default=2, help="runs per client (default: 2)")
    parser.add_argument("--label", default=DEFAULT_LABEL, help="run set label under .local/packet-parity/runs")
    parser.add_argument("--rebuild-world", action="store_true", help="rebuild the world snapshot before running")
    parser.add_argument("--json", type=Path, help="optional JSON report output path")
    args = parser.parse_args(argv)

    if args.runs < 1:
        parser.error("--runs must be at least 1")
    witness_explicit = args.witness is not None
    witness_path = args.witness if witness_explicit else default_witness_path()
    recording_path = args.recording if args.recording is not None else default_recording_path()

    if args.compare:
        rep = diff.compare_look_runs(
            diff.runs_of(args.compare, args.a),
            diff.runs_of(args.compare, args.b),
            ROUTES, (args.a, args.b),
        )
        if args.json:
            args.json.write_text(json.dumps(rep, indent=1), encoding="utf-8")
        print(diff.describe_look(rep))
        return 0 if rep["bit_exact"] else 1
    if args.compare_witness:
        if not witness_path.exists():
            raise SystemExit(
                f"missing recorded {lab.VERSION} witness: {witness_path} "
                f"(run python3 tools/test_x11_mouse_look_parity.py --record or pass --witness FILE)"
            )
        witness = json.loads(witness_path.read_text(encoding="utf-8"))
        rep = diff.compare_look_against_witness(diff.runs_of(args.compare_witness, args.client), witness, args.client)
        if args.json:
            args.json.write_text(json.dumps(rep, indent=1), encoding="utf-8")
        print(diff.describe_look(rep))
        return 0 if rep["bit_exact"] else 1
    if args.record:
        record_witness(recording_path, witness_path, args.runs, args.label, args.rebuild_world)
        return 0
    recording_path = ensure_recording(recording_path)
    bins = args.bin if args.bin is not None else lab.cargo_target() / "fast"
    if args.live:
        rep = verify_live(recording_path, bins, args.runs, args.label, args.rebuild_world, args.json)
    else:
        if witness_explicit:
            if not witness_path.exists():
                raise SystemExit(
                    f"missing recorded {lab.VERSION} witness: {witness_path} "
                    f"(run python3 tools/test_x11_mouse_look_parity.py --record --witness {witness_path})"
                )
        else:
            harness = _load_packet_parity()
            settings = harness.digest([harness.PROPERTIES, harness.OPTIONS, harness.GAMERULES])
            server_sha = lab.digest(harness.server_jar(), "sha256")
            if not _witness_is_fresh(witness_path, recording_path, args.runs,
                                     settings_digest=settings, server_jar_sha256=server_sha):
                record_witness(recording_path, witness_path, args.runs, args.label, args.rebuild_world)
        rep = verify_against_witness(recording_path, witness_path, bins, args.runs, args.label,
                                     args.rebuild_world, args.json)
    return 0 if rep["bit_exact"] else 1


if __name__ == "__main__":
    sys.exit(main())
