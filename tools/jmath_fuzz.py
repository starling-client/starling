#!/usr/bin/env python3
"""Differential fuzz of sunrise_core::jmath against the JVM's java.lang.Math: the same inputs, compared bit by bit.

    python3 tools/jmath_fuzz.py                         # every function, 4 chunks of 1,000,000 inputs
    python3 tools/jmath_fuzz.py sin cos --chunks 1000   # a billion inputs each, in chunks
    python3 tools/jmath_fuzz.py --count 10000000 --seed 7
    python3 tools/jmath_fuzz.py view_vector              # a game function against vanilla's own (needs the jars)

The JVM side (tools/jmath_fuzz/JmathFuzz.java, on the `java` the environment gives: JAVA_HOME or PATH) writes its
results for a seeded SplitMix64 stream of inputs; crates/sunrise-core/examples/jmath_fuzz.rs regenerates the same
inputs and compares each Rust variant: `jmath` as built for this platform, `platform` (Rust's own call, glibc on
Linux), and where they exist `exact` (correctly rounded) and `fdlibm` (StrictMath's algorithm). The inputs mix a
function's usual range, random exponents, any bit pattern and edge values (zeros, subnormals, infinities, NaN,
multiples of pi, the limits of exp). Each chunk's file is deleted once compared.

Writes .local/jmath-fuzz/report.json: per function and variant, the inputs compared, the mismatches (also per input
class, in the order above, each a quarter of the inputs), the largest
distance in ulps and sample inputs with both results' bits. Exits 1 when `jmath` itself differs anywhere.
"""

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import test_rust_parity as base  # noqa: E402

FUNCTIONS = ["sin", "cos", "tan", "exp", "log", "log10", "pow", "cbrt", "atan2", "atan", "asin", "acos", "hypot",
             "mthtable"]
# Vanilla's own game functions (tools/jmath_fuzz/VanillaMathFuzz.java against
# crates/sunrise-world/examples/probe/vanilla_math_fuzz.rs); they need the prepared 26.3 jars.
GAME = ["view_vector", "vec3_yrot", "projectile_rot", "input_vector", "blocking_angle", "mth_rounding"]
WORK = lab.LOCAL / "jmath-fuzz"


def java_tool(name):
    home = os.environ.get("JAVA_HOME")
    if home and (Path(home) / "bin" / name).exists():
        return str(Path(home) / "bin" / name)
    found = shutil.which(name)
    if not found:
        sys.exit(f"{name} not found: set JAVA_HOME to the pinned JDK")
    return found


def build(game):
    """The JVM program's class path and command, and the Rust comparer's command, for one family."""
    classes = WORK / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    examples = lab.cargo_target() / "fast" / "examples"
    if not game:
        subprocess.run([java_tool("javac"), "-d", str(classes), str(lab.ROOT / "tools/jmath_fuzz/JmathFuzz.java")],
                       check=True)
        subprocess.run(["cargo", "build", "--locked", "--profile", "fast", "-p", "sunrise-core", "--example",
                        "jmath_fuzz"], cwd=lab.ROOT, check=True)
        return [java_tool("java"), "-cp", str(classes), "JmathFuzz"], [str(examples / "jmath_fuzz")]
    _, jars, libraries = base.verify_artifacts()
    path = os.pathsep.join(map(str, [jars["server"], *libraries]))
    subprocess.run([java_tool("javac"), "-proc:none", "-nowarn", "-cp", path, "-d", str(classes),
                    str(lab.ROOT / "tools/jmath_fuzz/VanillaMathFuzz.java")], check=True)
    subprocess.run(["cargo", "build", "--locked", "--profile", "fast", "-p", "sunrise-world", "--example",
                    "world_probe"], cwd=lab.ROOT, check=True)
    return ([java_tool("java"), "-cp", str(classes) + os.pathsep + path, "VanillaMathFuzz"],
            [str(examples / "world_probe"), "vanilla_math_fuzz"])


def run(functions, chunks, count, seed):
    commands = {}
    version = subprocess.run([java_tool("java"), "-version"], capture_output=True, text=True).stderr.splitlines()
    report = {"jvm": version[0] if version else "?", "arch": platform.machine(), "system": platform.platform(),
              "count_per_chunk": count, "chunks": chunks, "results": {}}
    for f in functions:
        totals = {}
        # Mth's sine table has 65,536 entries: one pass compares each exactly once, whatever --chunks and --count say.
        f_chunks, f_count = (1, 65536) if f == "mthtable" else (chunks, count)
        for chunk in range(f_chunks):
            chunk_seed = (seed * 1_000_003 + chunk) & 0xFFFF_FFFF_FFFF_FFFF
            out = WORK / f"{f}-{chunk}.bin"
            game = f in GAME
            if game not in commands:
                commands[game] = build(game)
            jvm, rust = commands[game]
            subprocess.run([*jvm, f, str(chunk_seed), str(f_count), str(out)], check=True,
                           stdout=subprocess.DEVNULL)
            lines = subprocess.run([*rust, f, str(chunk_seed), str(f_count), str(out)], check=True,
                                   capture_output=True, text=True).stdout.splitlines()
            out.unlink()
            for row in map(json.loads, lines):
                total = totals.setdefault(row["variant"], {"inputs": 0, "mismatches": 0, "by_class": [0, 0, 0, 0],
                                                          "worst_ulps": 0, "samples": []})
                total["inputs"] += row["count"]
                total["mismatches"] += row["mismatches"]
                total["by_class"] = [a + b for a, b in zip(total["by_class"], row["by_class"])]
                worst = row["worst_ulps"]
                total["worst_ulps"] = None if worst is None or total["worst_ulps"] is None else max(total["worst_ulps"], worst)
                total["samples"] = (total["samples"] + row["samples"])[:16]
        if f == "mthtable":
            # Table entries, not drawn inputs: no input classes.
            for total in totals.values():
                total["table_entries"] = total.pop("inputs")
                del total["by_class"]
        report["results"][f] = totals
        for variant, total in totals.items():
            if f == "mthtable":
                print(f"{f:6} {variant:9} {total['mismatches']:>12} / {total['table_entries']} table entries"
                      f"  worst {total['worst_ulps']} ulps", flush=True)
                continue
            rate = total["mismatches"] / total["inputs"]
            print(f"{f:6} {variant:9} {total['mismatches']:>12} / {total['inputs']:<12} ({rate:.6%})"
                  f"  worst {total['worst_ulps']} ulps  by class (usual range, exponents, any bits, edges) "
                  f"{total['by_class']}", flush=True)
    WORK.mkdir(parents=True, exist_ok=True)
    (WORK / "report.json").write_text(json.dumps(report, indent=1) + "\n", encoding="utf-8")
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("functions", nargs="*", metavar="FUNCTION", help="default: " + " ".join(FUNCTIONS))
    parser.add_argument("--chunks", type=int, default=4)
    parser.add_argument("--count", type=int, default=1_000_000, help="inputs per chunk")
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args(argv)
    unknown = sorted(set(args.functions) - set(FUNCTIONS) - set(GAME))
    if unknown:
        parser.error("unknown function: " + ", ".join(unknown))
    report = run(args.functions or FUNCTIONS, args.chunks, args.count, args.seed)
    shipped = ("jmath", "table", "sunrise")
    differs = [f for f, totals in report["results"].items()
               if any(totals.get(name, {}).get("mismatches") for name in shipped)]
    if differs:
        print("Sunrise's function differs from the JVM in: " + ", ".join(differs), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
