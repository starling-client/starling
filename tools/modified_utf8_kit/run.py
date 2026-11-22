#!/usr/bin/env python3
"""Times sunrise_core::modified_utf8 against HotSpot's DataOutputStream.writeUTF and DataInputStream.readUTF, with the
method of Isloka's math kits: placement-averaged copies, fresh JVMs and bootstrap confidence intervals. See README.md.
"""
import argparse
import json
import os
from pathlib import Path
import random
import re
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
MODULE = ROOT / "crates/sunrise-core/src/modified_utf8.rs"
BUILD = ROOT / ".local/modified-utf8-kit"
OPS = ["encode", "decode"]
SETS = [f"{mix}-{n}" for mix in ("ascii", "latin", "cjk", "emoji") for n in (8, 64, 1024)]


def log(message):
    print(message, flush=True)


def run(*args, **kw):
    return subprocess.run([str(a) for a in args], check=True, **kw)


def median(values):
    s = sorted(values)
    n = len(s)
    return s[n // 2] if n % 2 else (s[n // 2 - 1] + s[n // 2]) / 2


def bootstrap_median_ci(groups, rng, resamples=4000):
    """The median of the pooled values of `groups` (one list per fresh JVM start) and a 95% percentile-bootstrap
    interval for it. Rounds of one JVM share its state and are not independent, so each resample draws whole starts
    with replacement, then rounds with replacement within each drawn start."""
    k = len(groups)

    def draw():
        return [g[rng.randrange(len(g))] for g in (groups[rng.randrange(k)] for _ in range(k)) for _ in g]

    meds = sorted(median(draw()) for _ in range(resamples))
    return median([v for g in groups for v in g]), meds[int(0.025 * resamples)], meds[int(0.975 * resamples) - 1]


def cpu_list(text):
    out = set()
    for part in text.strip().split(","):
        if "-" in part:
            a, b = part.split("-")
            out.update(range(int(a), int(b) + 1))
        elif part:
            out.add(int(part))
    return out


def pick_cpu():
    """The core to pin to, and why: on a hybrid processor a performance core (Intel's P-cores from
    /sys/devices/cpu_core/cpus, or the cores whose cpuinfo_max_freq is clearly the highest), else CPU 2 if allowed,
    else the last allowed CPU."""
    allowed = sorted(os.sched_getaffinity(0))
    default = 2 if 2 in allowed else allowed[-1]
    try:
        p_cores = [c for c in allowed if c in cpu_list(Path("/sys/devices/cpu_core/cpus").read_text())]
    except (OSError, ValueError):
        p_cores = []
    if p_cores:
        c = default if default in p_cores else p_cores[min(2, len(p_cores) - 1)]
        return c, f"a P-core (from /sys/devices/cpu_core/cpus)"
    freqs = {}
    for c in allowed:
        try:
            freqs[c] = int(Path(f"/sys/devices/system/cpu/cpu{c}/cpufreq/cpuinfo_max_freq").read_text())
        except (OSError, ValueError):
            pass
    if freqs and max(freqs.values()) > 1.1 * min(freqs.values()):
        top = max(freqs.values())
        fast = [c for c, f in freqs.items() if f == top]
        c = default if default in fast else fast[min(2, len(fast) - 1)]
        return c, f"one of the fastest cores by cpuinfo_max_freq ({top / 1e6:.2f} GHz)"
    return default, "the default rule (CPU 2 if allowed, else the last allowed CPU)"


def filler(c, op, n):
    """Copy c's filler, before its first function (`c<k>_<op>_h`): `c<k>_<op>_g`, a name of the same length that sorts
    just before it, since the compiler emits a module's functions in the order of their length-prefixed symbol names.
    One per copy: two would keep the copies a multiple of 32 bytes apart. `n` statements, and a constant of its own,
    so that no two fillers are the same function (which the compiler merges)."""
    uid = c + 1
    body = [f"    let mut v = x ^ {uid};",
            *(f"    v = black_box(v).wrapping_mul({2 * k + 3}).rotate_left({k % 7 + 1});" for k in range(n)),
            "    v"] if n else [f"    black_box(x ^ {uid})"]
    return ["#[inline(never)]", f"pub fn c{c}_{op}_g(x: u64) -> u64 {{", *body, "}"]


def generate(copies, fill):
    """The bench crate in BUILD/rust: main.rs, and `copies` byte-identical copies of the module (copies/c<k>/), each
    with its own encode and decode loops and, before the loops of every copy but the first, a filler of `fill`
    statements: so consecutive copies are the same distance apart, which `fill` changes."""
    rust = BUILD / "rust"
    shutil.rmtree(rust, ignore_errors=True)
    (rust / "src").mkdir(parents=True)
    shutil.copyfile(HERE / "rust/Cargo.toml", rust / "Cargo.toml")
    shutil.copyfile(HERE / "rust/src/main.rs", rust / "src/main.rs")
    lines = ["//! Written by run.py: byte-identical copies of sunrise_core::modified_utf8, each with its own loops.",
             "use std::hint::black_box;", ""]
    for c in range(copies):
        d = rust / "src/copies" / f"c{c}"
        d.mkdir(parents=True)
        shutil.copyfile(MODULE, d / "modified_utf8.rs")
        (rust / "src/copies" / f"c{c}.rs").write_text("pub mod modified_utf8;\n")
        lines += [f"pub mod c{c};", *filler(c, "dec", fill if c else 0),
                  "#[inline(never)]", f"pub fn c{c}_enc_h(strings: &[Vec<u16>], out: &mut Vec<u8>) -> usize {{",
                  "    out.clear();",
                  "    for s in strings {",
                  f"        let len = c{c}::modified_utf8::encoded_len(s);",
                  "        let Ok(prefix) = u16::try_from(len) else { return usize::MAX };",
                  "        out.extend_from_slice(&prefix.to_be_bytes());",
                  f"        c{c}::modified_utf8::encode(s, len, out);",
                  "    }",
                  "    out.len()",
                  "}",
                  "#[inline(never)]", f"pub fn c{c}_dec_h(bytes: &[u8], out: &mut [Vec<u16>]) -> usize {{",
                  "    let mut at = 0;",
                  "    for slot in out.iter_mut() {",
                  "        let len = usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]));",
                  f"        *slot = c{c}::modified_utf8::decode(&bytes[at + 2..at + 2 + len]).unwrap_or_default();",
                  "        at += 2 + len;",
                  "    }",
                  "    at",
                  "}", ""]
    lines += ["pub type Encode = fn(&[Vec<u16>], &mut Vec<u8>) -> usize;",
              "pub type Decode = fn(&[u8], &mut [Vec<u16>]) -> usize;",
              "pub const ENCODE: &[(usize, Encode)] = &[" + ", ".join(f"({c}, c{c}_enc_h)" for c in range(copies)) + "];",
              "pub const DECODE: &[(usize, Decode)] = &[" + ", ".join(f"({c}, c{c}_dec_h)" for c in range(copies)) + "];",
              "/// The fillers, so that the linker keeps them.",
              "pub const FILLERS: &[fn(u64) -> u64] = &["
              + ", ".join(f"c{c}_dec_g" for c in range(copies)) + "];", ""]
    (rust / "src/copies.rs").write_text("\n".join(lines))


def placements(binary, copies):
    """{function: [offset in its 64-byte line, per copy]} for each copy's loops and the module's functions emitted
    out of line, read from the symbol table; the functions whose copies share an address (merged); and the functions
    missing a copy (incomplete), which include the timed wrappers `enc_h` and `dec_h` when any copy of them is absent."""
    out = run("nm", "-C", "--defined-only", binary, capture_output=True, text=True).stdout
    found = {}
    for line in out.splitlines():
        parts = line.split(maxsplit=2)
        if len(parts) != 3 or parts[1] not in "tT":
            continue
        m = re.search(r"copies::c(\d+)(?:::modified_utf8::|_)([a-z_]+?)(?:::h[0-9a-f]{16})?$", parts[2])
        if not m or int(m.group(1)) >= copies or m.group(2).endswith("_g"):
            continue
        found.setdefault(m.group(2), {})[int(m.group(1))] = int(parts[0], 16)
    offsets, merged = {}, []
    incomplete = [n for n in ("enc_h", "dec_h") if n not in found]
    for name, by_copy in sorted(found.items()):
        if len(by_copy) != copies:
            incomplete.append(name)
            continue
        addresses = [by_copy[c] for c in range(copies)]
        offsets[name] = [a % 64 for a in addresses]
        if len(set(addresses)) < copies:
            merged.append(name)
    return offsets, merged, incomplete


def build(copies, attempts=16):
    """Builds with growing fillers until every function's copies sit at min(copies, 4) distinct offsets."""
    env = {**os.environ, "CARGO_TARGET_DIR": str(BUILD / "target")}
    binary = BUILD / "target/release/modified-utf8-kit"
    need = min(copies, 4)
    for fill in range(1, attempts + 1):
        generate(copies, fill)
        run("cargo", "build", "--release", "--quiet", "--manifest-path", BUILD / "rust/Cargo.toml", env=env)
        offsets, merged, incomplete = placements(binary, copies)
        distinct = {k: len(set(o)) >= need and k not in merged for k, o in offsets.items()}
        placed = not incomplete and all(distinct.values())
        if placed:
            break
    log(f"placement: {fill} build(s); " + ("every function's copies at distinct offsets" if placed else
        "NOT placed: " + ", ".join([f"{k} not distinct" for k, d in distinct.items() if not d]
                                   + [f"{k} missing a copy" for k in incomplete])))
    for name, o in offsets.items():
        log(f"  {name:20} offsets mod 64: {o}")
    return binary, offsets, placed


def confidence(kit, java, classes, pin, inputs, starts, per_start, rows, rng):
    """For each row: Rust (every copy) and HotSpot in alternating rounds, over `starts` fresh JVMs of `per_start`
    rounds each (the JVM waits between its rounds while Rust runs, on the same core). Each round's mean over the
    copies is divided by HotSpot's round; the row reports the median of those ratios with a bootstrap 95% interval."""
    out = {}
    for op, set_name in rows:
        rounds = []

        def rust():
            lines = run(*pin, kit, "bench", op, set_name, inputs, capture_output=True, text=True).stdout.splitlines()
            return [json.loads(line) for line in lines]

        for start in range(starts):
            jvm = subprocess.Popen([str(a) for a in (*pin, java, "-cp", classes, "dev.sunriselab.parity.Mutf8Bench", op, set_name, inputs,
                                                     per_start, "stepped")],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

            def hotspot():
                jvm.stdin.write("\n")
                jvm.stdin.flush()
                return float(re.search(r"hotspot median\s+([\d.]+) ns", jvm.stdout.readline()).group(1))

            for i in range(per_start):
                # HotSpot goes first in every other round, so that neither always runs right after the other.
                h = hotspot() if i % 2 else None
                copies = rust()
                if h is None:
                    h = hotspot()
                rounds.append({"copies": copies, "hotspot_ns": h, "jvm_start": start})
            jvm.stdin.close()
            jvm.wait()
        mean = lambda r: sum(c["median_ns"] for c in r["copies"]) / len(r["copies"])
        med, lo, hi = bootstrap_median_ci(
            [[mean(r) / r["hotspot_ns"] for r in rounds if r["jvm_start"] == s] for s in range(starts)], rng)
        per_copy = [round(median([r["copies"][c]["median_ns"] / r["hotspot_ns"] for r in rounds]), 4)
                    for c in range(len(rounds[0]["copies"]))]
        per_start_hs = [median([r["hotspot_ns"] for r in rounds if r["jvm_start"] == s]) for s in range(starts)]
        row = {"ratio_median": round(med, 4), "ratio_ci95": [round(lo, 4), round(hi, 4)],
               "status": "faster" if hi < 0.98 else "slower" if lo > 1.0 else "inside the noise",
               "copies": per_copy, "rust_ns": round(median([mean(r) for r in rounds]), 4),
               "hotspot_ns": round(median([r["hotspot_ns"] for r in rounds]), 4),
               "hotspot_start_spread": round((max(per_start_hs) - min(per_start_hs)) / median(per_start_hs), 4),
               "allocations": sum(c["allocations"] for r in rounds for c in r["copies"]),
               "rounds": len(rounds), "jvm_starts": starts}
        out[f"{op} {set_name}"] = row
        log(f"{op:6} {set_name:10} rust {row['rust_ns']:8.2f} ns  hotspot {row['hotspot_ns']:8.2f} ns  ratio "
            f"{row['ratio_median']:.3f} [{lo:.3f}, {hi:.3f}]  copies {min(per_copy):.3f} to {max(per_copy):.3f}  "
            f"{row['status']}")
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--copies", type=int, default=4, help="byte-identical copies of the module (default 4)")
    parser.add_argument("--confidence-starts", type=int, default=8, help="fresh JVMs per row (default 8)")
    parser.add_argument("--confidence-rounds", type=int, default=5, help="rounds per JVM (default 5)")
    parser.add_argument("--rows", default=None, help="comma-separated 'op set' filters, e.g. 'encode ascii-8'")
    parser.add_argument("--java", default="java", help="the java to time (default: java on PATH)")
    args = parser.parse_args()
    BUILD.mkdir(parents=True, exist_ok=True)
    kit, offsets, placed = build(args.copies)
    inputs = BUILD / "inputs"
    inputs.mkdir(exist_ok=True)
    run(kit, "gen", inputs)
    log(run(kit, "check", inputs, capture_output=True, text=True).stdout.strip())
    classes = BUILD / "classes"
    classes.mkdir(exist_ok=True)
    java = args.java
    javac = str(Path(java).with_name("javac")) if os.sep in java else "javac"
    run(javac, "-d", classes, ROOT / "mods/sunrise/src/test/java/dev/sunriselab/parity/Mutf8Bench.java")
    cpu, why = pick_cpu()
    pin = ["taskset", "-c", str(cpu)] if shutil.which("taskset") else []
    log(f"pinned to CPU {cpu}: {why}" if pin else "not pinned (no taskset)")
    rows = [(op, s) for op in OPS for s in SETS]
    if args.rows is not None:
        wanted = [part for raw in args.rows.split(",") if (part := raw.strip())]
        rows = [r for r in rows if any(f"{r[0]} {r[1]}".startswith(w) for w in wanted)]
    if not rows:
        sys.exit(f"--rows {args.rows!r} selects no cases")
    rng = random.Random(20261119)
    result = confidence(kit, java, classes, pin, inputs, args.confidence_starts, args.confidence_rounds, rows, rng)
    cpu_model = next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                      if line.startswith("model name")), "unknown")
    jdk = subprocess.run([java, "-version"], capture_output=True, text=True).stderr.strip().splitlines()
    rustc = subprocess.run(["rustc", "-V"], capture_output=True, text=True).stdout.strip()
    report = {"cpu": cpu_model, "pinned_cpu": cpu if pin else None, "jdk": jdk, "rustc": rustc,
              "copies": args.copies, "placement": offsets, "placement_averaged": placed, "rows": result}
    (BUILD / "report.json").write_text(json.dumps(report, indent=1) + "\n")
    log("=== summary ===")
    log(f"{cpu_model}; {jdk[0] if jdk else 'java ?'}; {rustc}; {args.copies} copies"
        + ("" if placed else " (NOT placement-averaged)"))
    missing = [k for k, r in result.items() if r["status"] != "faster"]
    allocating = [k for k, r in result.items() if k.startswith("encode") and r["allocations"]]
    verdict = "PASS" if placed and not missing and not allocating else "FAIL"
    log(f"rows faster than HotSpot by more than 2%: {len(result) - len(missing)} of {len(result)}")
    if missing:
        log("rows that miss the bar: " + ", ".join(missing))
    if allocating:
        log("encode rows that allocate: " + ", ".join(allocating))
    log(f"verdict: {verdict} ({BUILD / 'report.json'})")
    sys.exit(0 if verdict == "PASS" else 1)


if __name__ == "__main__":
    main()
