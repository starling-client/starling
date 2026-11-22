#!/usr/bin/env python3
"""Robustness fuzz of the shader-pack translator: mutated packs through the game's load path, checked for invariants.

    python3 tools/shader_fuzz.py --smoke                     # the seed programs unmutated: the tool works
    python3 tools/shader_fuzz.py --seed 1 --count 500        # 500 cases from the committed seed pack
    python3 tools/shader_fuzz.py --corpus ~/shaderpacks --budget 1200 --jobs 4
    python3 tools/shader_fuzz.py --corpus ~/shaderpacks --replay 1234567   # one case again, by its seed

crates/sunrise-render-shaderpack/examples/shader_fuzz/ does the work. A case is a pack of the corpus (the seed pack
under examples/shader_fuzz/seeds/, and each zip or folder of --corpus, which is read and never copied into the
repository) cut down to one or two programs (both stages, or compute shaders only) and what they include, then
mutated by one to six named steps (declaration forms, #include chains and cycles, preprocessor nesting and macros,
array sizes, names that collide with the game's inputs or later keywords, number extremes, truncation, encodings,
duplicated or reordered lines, shaders.properties values) drawn from a SplitMix64 stream seeded per case. Each case
loads through load_post as the game loads a pack, with the game's inputs, in a worker process, and must keep every
invariant: no panic, within --time-limit and --memory-limit, every stage file within --size-limit once preprocessed,
every loaded program valid in naga, every note starting with a subject (a `key` or a word before its first ": ") and
a program the unmutated case loaded loaded or named by a note, the same output twice.

Failures are grouped by invariant, first failing step and detail. Each group's smallest case is shrunk while the
same failure stays (steps, then files, then runs of lines) and written as a pack folder under
the run's groups/ folder; its report.json lists the groups with the seed that replays each. Exits 1
when any case fails.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

WORK = lab.LOCAL / "shader-fuzz"


def build():
    subprocess.run(["cargo", "build", "--locked", "--profile", "fast", "-p", "sunrise-render-shaderpack",
                    "--example", "shader_fuzz"], cwd=lab.ROOT, check=True)
    return lab.binary(lab.cargo_target() / "fast" / "examples", "shader_fuzz")


def arguments(args):
    """The fuzzer's command line for these options."""
    out = ["--seed", str(args.seed), "--count", str(args.count), "--jobs", str(args.jobs),
           "--time-limit", str(args.time_limit), "--memory-limit", str(args.memory_limit),
           "--size-limit", str(args.size_limit), "--shrink-seconds", str(args.shrink_seconds),
           "--work-dir", str(args.work_dir.resolve())]
    if args.budget is not None:
        out += ["--budget", str(args.budget)]
    if args.corpus is not None:
        out += ["--corpus", str(args.corpus.resolve(strict=True))]
    if args.replay is not None:
        out += ["--replay", str(args.replay)]
    if args.smoke:
        out.append("--smoke")
    return out


def summary(report):
    """One line per group, most cases first, with its replay command."""
    lines = [f"{report['cases']} cases, {report['failing']} failures in {len(report['groups'])} groups"]
    for group in sorted(report["groups"], key=lambda g: (-g["count"], g["group"])):
        corpus = f" --corpus {report['corpus_dir']}" if report.get("corpus_dir") else ""
        lines.append(f"  {group['count']:4d}  {group['group']}")
        lines.append(f"        python3 tools/shader_fuzz.py{corpus} --replay {group['replay']}"
                     f"   # shrunk: {group['dir']}")
    return lines


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--seed", type=int, default=1, help="the run's seed; each case's own seed derives from it")
    parser.add_argument("--count", type=int, default=200, help="cases to generate")
    parser.add_argument("--budget", type=int, help="stop generating cases after this many seconds")
    parser.add_argument("--corpus", type=Path, help="a folder of packs (zips or folders) to take programs from")
    parser.add_argument("--jobs", type=int, default=1, help="worker processes")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--replay", type=int, metavar="SEED", help="run the case of this seed (from a report)")
    source.add_argument("--smoke", action="store_true", help="the seed programs unmutated, which must pass")
    parser.add_argument("--time-limit", type=int, default=20, help="seconds a case may take")
    parser.add_argument("--memory-limit", type=int, default=2048, help="MiB a worker may hold")
    parser.add_argument("--size-limit", type=int, default=16, help="MiB a stage file may grow to, preprocessed")
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
    return 1 if report["groups"] else 0


if __name__ == "__main__":
    sys.exit(main())
