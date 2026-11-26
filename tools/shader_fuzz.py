#!/usr/bin/env python3
"""Robustness fuzz of the shader-pack translator: mutated packs through the game's load path, checked for invariants.

    python3 tools/shader_fuzz.py --smoke                     # the seed programs unmutated: the tool works
    python3 tools/shader_fuzz.py --seed 1 --count 500        # 500 cases from the committed seed pack
    python3 tools/shader_fuzz.py --corpus ~/shaderpacks --budget 1200 --jobs 4   # as many cases as 20 minutes make
    python3 tools/shader_fuzz.py --replay 1234567 --corpus ~/shaderpacks ...      # a group's case again, as printed

crates/sunrise-render-shaderpack/examples/shader_fuzz/ does the work. A case is a pack of the corpus (the seed pack
under examples/shader_fuzz/seeds/, and each zip or folder of --corpus, which is read and never copied into the
repository) whole and read by a case process within --memory-limit, with one or two of its programs (both stages, or
compute shaders only) mutated by one to six named steps (declaration forms, #include chains and cycles, preprocessor
nesting and macros, array sizes, names that collide with the game's inputs or later keywords, number extremes,
truncation, encodings, duplicated or reordered lines, shaders.properties values) drawn from a SplitMix64 stream
seeded per case. Each case loads through load_post as the game loads a pack, with the game's inputs, in an eval
worker of its case process, and must keep every invariant: no panic, within --time-limit and --memory-limit, every
stage file of the dimension's folder within --size-limit once preprocessed, every loaded program valid in naga,
every note starting with a subject (a `key` or a word before its first ": ") and a program the unmutated case loaded
loaded or named by a note, the same output twice.

Failures are grouped by invariant, first failing step and detail. Each group's smallest case is shrunk while the
same failure stays (steps, then files, then runs of lines) and written as a pack folder under
the run's groups/ folder; its report.json lists the groups, each with the arguments that replay
its smallest case (`--replay` with its seed, or `smoke:N` for a fixed smoke case, the corpus of a
generated case, the seed pack when the run was given one, and the run's time, memory and size
limits), printed as a command, then the cases run and skipped (each skipped one is printed with
why), and how each case process ended. Exits 1 when any case fails, or when no case ran.

--budget alone generates cases until it ends, --count alone that many, both until the first is
reached, neither 200.
"""

import argparse
import json
import re
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from tools import lab

WORK = lab.LOCAL / "shader-fuzz"


def build():
    subprocess.run(["cargo", "build", "--locked", "--profile", "fast", "-p", "sunrise-render-shaderpack",
                    "--example", "shader_fuzz"], cwd=lab.ROOT, check=True)
    return lab.binary(lab.cargo_target() / "fast" / "examples", "shader_fuzz")


def arguments(args):
    """The fuzzer's command line for these options."""
    out = ["--seed", str(args.seed), "--jobs", str(args.jobs),
           "--time-limit", str(args.time_limit), "--memory-limit", str(args.memory_limit),
           "--size-limit", str(args.size_limit), "--shrink-seconds", str(args.shrink_seconds),
           "--work-dir", str(args.work_dir.resolve())]
    if args.count is not None:
        out += ["--count", str(args.count)]
    if args.budget is not None:
        out += ["--budget", str(args.budget)]
    if args.corpus is not None:
        out += ["--corpus", str(args.corpus.resolve(strict=True))]
    if args.seed_pack is not None:
        out += ["--seed-pack", str(args.seed_pack.resolve(strict=True))]
    if args.replay is not None:
        out += ["--replay", str(args.replay)]
    if args.smoke:
        out.append("--smoke")
    return out


def case_id(text):
    """A case `--replay` takes: a generated case's seed, or `smoke:N` for a fixed smoke case."""
    if not re.fullmatch(r"(smoke:)?[0-9]+", text, re.ASCII):
        raise argparse.ArgumentTypeError(f"not a seed or smoke:N: {text!r}")
    return text


def summary(report):
    """The cases run and skipped, then one line per group, most cases first, with the command that replays its case."""
    lines = [f"{report['cases']} cases ({report.get('skipped', 0)} skipped), {report['failing']} failures"
             f" in {len(report['groups'])} groups"]
    if not report["cases"]:
        lines.append("no case ran")
    for group in sorted(report["groups"], key=lambda g: (-g["count"], g["group"])):
        lines.append(f"  {group['count']:4d}  {group['group']}")
        lines.append(f"        python3 tools/shader_fuzz.py {shlex.join(group['replay_args'])}"
                     f"   # shrunk: {group['dir']}")
    return lines


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--seed", type=int, default=1, help="the run's seed; each case's own seed derives from it")
    parser.add_argument("--count", type=int, help="cases to generate (200 when --budget isn't given either)")
    parser.add_argument("--budget", type=int, help="stop generating cases after this many seconds")
    parser.add_argument("--corpus", type=Path,
                        help="a folder of packs (zips or folders) to take cases from; only case processes read them")
    parser.add_argument("--jobs", type=int, default=1,
                        help="case processes, each with its own eval worker")
    parser.add_argument("--seed-pack", type=Path,
                        help="a pack folder in place of the committed seed pack (the first pack, and --smoke's)")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--replay", type=case_id, metavar="SEED|smoke:N",
                        help="run this one case (from a report's replay command)")
    source.add_argument("--smoke", action="store_true", help="the seed programs unmutated, which must pass")
    parser.add_argument("--time-limit", type=int, default=20,
                        help="seconds a load of a case (in its eval worker) or a pack's reading may take")
    parser.add_argument("--memory-limit", type=int, default=2048,
                        help="MiB each case process and each eval worker may hold")
    parser.add_argument("--size-limit", type=int, default=16,
                        help="MiB a stage file of the dimension's folder may grow to, preprocessed")
    parser.add_argument("--shrink-seconds", type=int, default=120, help="per group")
    parser.add_argument("--binary", type=Path, help="an already built shader_fuzz")
    parser.add_argument("--work-dir", type=Path, default=WORK, help="parent directory for separately stored runs")
    args = parser.parse_args(argv)
    binary = args.binary.resolve(strict=True) if args.binary else build()
    runs = args.work_dir.resolve() / "runs"
    try:
        runs.mkdir(parents=True, exist_ok=True)
        args.work_dir = Path(tempfile.mkdtemp(prefix="run-", dir=runs))
        done = subprocess.run([str(binary), *arguments(args)], cwd=lab.ROOT)
        if done.returncode not in (0, 1):
            print(f"shader_fuzz stopped (exit {done.returncode})", file=sys.stderr)
            return 2
        report_path = args.work_dir / "report.json"
        report = json.loads(report_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        print(f"shader_fuzz: {args.work_dir}: {error}", file=sys.stderr)
        return 2
    print("\n".join(summary(report)))
    # A run that ran no case checked nothing: not a pass.
    return 1 if report["groups"] or not report["cases"] else 0


if __name__ == "__main__":
    sys.exit(main())
