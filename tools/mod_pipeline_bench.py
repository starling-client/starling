#!/usr/bin/env python3
"""Mod render pipeline benchmark: load time and frame time with and without a mod's pipelines.

Builds nothing. Build the benchmark's game and the control tool first:

    cargo build --release -p sunrise-mod-host --example mod_pipeline_bench
    cargo build --release -p sunrise-client-lib --bin sunrise-ctl

(two commands: built together, Cargo unifies `sunrise-client-lib`'s test-only features into the
game, and its integrated server stops at startup).

The game is Sunrise with `tools/fixtures/mod-pipeline-bench` linked in. This script stages the
mod's package beside it (with its generated shaders) and runs three rows on `vscreen`'s virtual
screen, or on the X display it was started with (`xvfb-run`) when `vscreen` isn't installed,
each in a fresh seeded creative world:

- `no mod`: the same binary with no mod loaded;
- `mod, cold`: the driver's shader cache off and a fresh game home, so every shader compiles;
- `mod, warm`: a second run on the cache a prior cached mod run left.

Each row records the time from launch to the control socket answering and to the world being
ready, then flies the first graphics_bench scene and records its frame times. Run it at the
base and at the head, on the same host and driver, and compare with `--compare`.

    python3 tools/mod_pipeline_bench.py --label before --offscreen
    python3 tools/mod_pipeline_bench.py --compare before after
"""
import argparse
import datetime
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from tools import graphics_bench as gb, lab

FIXTURE = ROOT / "tools/fixtures/mod-pipeline-bench"
OUT = ROOT / "benchmarks/mod-pipelines"
# Kept in step with `PIPELINES` in the fixture's src/lib.rs.
PIPELINES = 32
COLD = {"MESA_SHADER_CACHE_DISABLE": "true", "__GL_SHADER_DISK_CACHE": "0"}


VERTEX_SHADER = """#version 330
#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>

in vec3 Position;
in vec2 UV0;
in vec4 Color;

out vec2 texCoord0;
out vec4 vertexColor;

void main() {
    gl_Position = ProjMat * ModelViewMat * vec4(Position, 1.0);
    texCoord0 = UV0;
    vertexColor = Color;
}
"""


def fragment_shader(index):
    """Distinct per pipeline, so no cache shares one compile between two of them."""
    k = f"{index + 1}.0 / {PIPELINES}.0"
    if index >= PIPELINES // 2:
        return (
            "#version 330\n"
            "#include <sunrise:pipeline_sprite.glsl>\n"
            "in vec2 texCoord0;\n"
            "in vec4 vertexColor;\n"
            "out vec4 fragColor;\n"
            "void main() {\n"
            f"    float k = {k};\n"
            "    fragColor = sampleSprite(texCoord0) * vertexColor * vec4(k, 1.0 - k, 1.0, 1.0);\n"
            "}\n"
        )
    return (
        "#version 330\n"
        "#include <minecraft:globals.glsl>\n"
        "in vec2 texCoord0;\n"
        "in vec4 vertexColor;\n"
        "out vec4 fragColor;\n"
        "void main() {\n"
        f"    float k = {k};\n"
        "    float pulse = 0.5 + 0.5 * sin(GameTime * 24000.0 * 0.2 * k + texCoord0.x * 6.2831853);\n"
        "    fragColor = vec4(vertexColor.rgb * pulse, vertexColor.a * (1.0 - texCoord0.y * k));\n"
        "}\n"
    )


def stage(game):
    target = game.parent / "sunrise-native-mods/pipeline_bench"
    shutil.rmtree(target, ignore_errors=True)
    shutil.copytree(FIXTURE, target, ignore=shutil.ignore_patterns("target", "Cargo.lock"))
    shaders = target / "assets/pipeline_bench/shaders/core"
    shaders.mkdir(parents=True)
    for index in range(PIPELINES):
        (shaders / f"p{index:02}.vsh").write_text(VERTEX_SHADER)
        (shaders / f"p{index:02}.fsh").write_text(fragment_shader(index))


def run_row(args, ctl, game_exe, name, extra_env, home):
    cwd = Path(tempfile.mkdtemp(prefix="run-", dir=args.scratch))
    (cwd / "options.txt").write_text(gb.OPTIONS.format(distance=args.distance))
    env = gb.vscreen_env() if shutil.which("vscreen") else lab.game_env()
    env.update(extra_env)
    env["SUNRISE_HOME"] = str(home)
    env["XDG_CACHE_HOME"] = str(home / "cache")
    env["__GL_SHADER_DISK_CACHE_PATH"] = str(home / "cache" / "nvidia")
    socket = cwd / "control.sock"
    game = gb.Game(ctl, socket, env, cwd)
    scene, start, end, pitch = gb.SCENES[0]
    launch = ["launch", "--game", game_exe, "--world", gb.WORLD, "--seed", gb.SEED, "--creative",
              "--render-distance", args.distance, "--resources", args.resources,
              "--no-pause", "--weather", "clear", "--time", "6000"]
    if args.offscreen:
        launch.append("--offscreen")
    began = time.monotonic()
    game.run(*launch)
    answered = time.monotonic() - began
    row = {"row": name, "answered_s": round(answered, 3)}
    try:
        game.run("wait-ready", 300)
        row["ready_s"] = round(time.monotonic() - began, 3)
        row["renderer"] = game.run("status").get("renderer")
        game.run("tp", *start)
        game.run("look", 0, pitch)
        game.run("wait-ready", 300)
        time.sleep(args.settle)
        flight = game.run("fly", *start, *end, args.seconds, pitch)
        row.update(avg_fps=flight.get("avg_fps"), low_1pct_fps=flight.get("low_1pct_fps"),
                   p99_ms=flight.get("p99_ms"), cpu_ms=(flight.get("cpu") or {}).get("avg_ms"),
                   gpu_ms=(flight.get("gpu") or {}).get("avg_ms"))
    finally:
        game.run("quit", check=False)
        for _ in range(100):
            if not socket.exists():
                break
            time.sleep(0.2)
        log = socket.with_suffix(".log")
        if log.exists():
            shutil.copy(log, args.scratch / f"{args.label}-{name.replace(', ', '-').replace(' ', '-')}.log")
        shutil.rmtree(cwd, ignore_errors=True)
    print(f"  {name:<10} answered {row['answered_s']:6.2f} s  ready {row.get('ready_s', 0):6.2f} s"
          f"  {row.get('avg_fps') or 0:7.1f} fps  p99 {row.get('p99_ms') or 0:6.2f} ms"
          f"  gpu {row.get('gpu_ms') or 0:6.2f} ms", flush=True)
    return row


def median_rows(runs):
    rows = {}
    for run in runs:
        for row in run:
            rows.setdefault(row["row"], []).append(row)
    keys = ("answered_s", "ready_s", "avg_fps", "low_1pct_fps", "p99_ms", "cpu_ms", "gpu_ms")
    return {name: {k: statistics.median(r[k] for r in group if r.get(k) is not None)
                   for k in keys if any(r.get(k) is not None for r in group)}
            for name, group in rows.items()}


def compare(before, after):
    load = lambda label: json.loads((OUT / f"{label}.json").read_text())
    a, b = load(before), load(after)
    print(f"Before {a['commit']}, after {b['commit']}; host {a['host']} / {b['host']}.\n")
    print("| row | launch to socket, s | launch to world ready, s | avg FPS | 1% low FPS | p99 ms | CPU ms | GPU ms |")
    print("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for name, old in a["median"].items():
        new = b["median"].get(name, {})
        cell = lambda k, f: f"{old.get(k, 0):{f}} → {new.get(k, 0):{f}}"
        print(f"| {name} | {cell('answered_s', '.2f')} | {cell('ready_s', '.2f')} | {cell('avg_fps', '.0f')} "
              f"| {cell('low_1pct_fps', '.1f')} | {cell('p99_ms', '.2f')} | {cell('cpu_ms', '.2f')} "
              f"| {cell('gpu_ms', '.2f')} |")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--label", default="run")
    p.add_argument("--runs", type=int, default=3, help="each row this many times; the table takes medians")
    p.add_argument("--distance", default="12")
    p.add_argument("--seconds", type=float, default=12.0)
    p.add_argument("--settle", type=float, default=2.0)
    p.add_argument("--offscreen", action="store_true", help="render offscreen on the discrete GPU")
    p.add_argument("--resources", default=str(lab.RESOURCES))
    p.add_argument("--build", default=os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target")) + "/release")
    p.add_argument("--compare", nargs=2, metavar=("BEFORE", "AFTER"))
    args = p.parse_args()
    if args.compare:
        compare(*args.compare)
        return
    build = Path(args.build).resolve()
    ctl, game_exe = build / "sunrise-ctl", build / "examples/mod_pipeline_bench"
    for path in (ctl, game_exe):
        if not path.exists():
            sys.exit(f"{path} missing: see this script's help for the build command")
    stage(game_exe)
    args.scratch = ROOT / ".local/mod-pipeline-bench"
    args.scratch.mkdir(parents=True, exist_ok=True)
    if shutil.which("vscreen"):
        subprocess.run(["vscreen", "up"], cwd=ROOT, check=True, capture_output=True)
    elif "DISPLAY" not in os.environ:
        sys.exit("no vscreen and no DISPLAY: run under xvfb-run or install vscreen")
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT,
                            capture_output=True, text=True).stdout.strip()
    runs = []
    for number in range(args.runs):
        print(f"run {number + 1} of {args.runs}", flush=True)
        no_mod_home = Path(tempfile.mkdtemp(prefix="home-", dir=args.scratch))
        cold_home = Path(tempfile.mkdtemp(prefix="home-", dir=args.scratch))
        warm_home = Path(tempfile.mkdtemp(prefix="home-", dir=args.scratch))
        no_mod = run_row(args, ctl, game_exe, "no mod", {"SUNRISE_PIPELINE_BENCH": "off"}, no_mod_home)
        cold = run_row(args, ctl, game_exe, "mod, cold", COLD, cold_home)
        # Prime the driver shader cache with the mod loaded before measuring the warm row.
        run_row(args, ctl, game_exe, "mod, warmup", {}, warm_home)
        warm = run_row(args, ctl, game_exe, "mod, warm", {}, warm_home)
        runs.append([no_mod, cold, warm])
    report = {
        "label": args.label, "date": datetime.date.today().isoformat(), "commit": commit,
        "host": os.uname().nodename, "offscreen": args.offscreen, "distance": args.distance,
        "scene": gb.SCENES[0][0], "runs": runs, "median": median_rows(runs),
    }
    OUT.mkdir(parents=True, exist_ok=True)
    out = OUT / f"{args.label}.json"
    out.write_text(json.dumps(report, indent=2) + "\n")
    print(out)


if __name__ == "__main__":
    main()
