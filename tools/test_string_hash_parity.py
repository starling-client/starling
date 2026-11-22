#!/usr/bin/env python3
"""`java_string_hash` against JDK 25's String.hashCode, bit for bit.

Every string of one and of two UTF-16 units (all 2^32 pairs, surrogates included), then seeded random
strings of 0 to 600 units mixing ASCII, Latin-1, BMP, surrogate pairs and lone surrogates (`--billions`).
The oracle (VanillaStringHashOracle, JDK only: no game jar) and the Rust probe (sunrise-core's
`sunrise-string-hash-parity`) generate the same strings from the same seed and print a streaming digest
per chunk; a chunk whose digests differ is dumped on both sides and its first difference written to
`.local/rust-string-hash-parity/first_difference.txt`. The probe also hashes every valid UTF-16 string
as a Rust `str` map key (`JavaKey::java_hash`), which must agree with `java_string_hash`. Prints the
harness name and counts only.
"""
import argparse
from itertools import zip_longest
import os
import subprocess
import time
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

NAME = "string_hash"
ORACLE = lab.ROOT / "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaStringHashOracle.java"
SEED = 0x5354_5249_4E47_4853  # "STRINGHS"
PER_CHUNK = 1_000_000
UTF16_VALUES = 1 << 16


def lines(command):
    out = subprocess.run(list(map(str, command)), check=True, capture_output=True, text=True).stdout
    return out.splitlines()


def parse(output, tag, expected, *, rust=False):
    """Require the complete chunk schedule and, for Rust, the UTF-8 check summary."""
    chunks, summary = {}, None
    for line in output:
        parts = line.split()
        if len(parts) == 4 and parts[0] == tag:
            chunk, count = map(int, parts[1:3])
            if chunk in chunks or expected.get(chunk) != count:
                raise ValueError(f"unexpected or duplicate {tag} chunk {chunk} (count {count})")
            chunks[chunk] = parts[3]
        elif rust and len(parts) == 3 and parts[0] == "S" and summary is None:
            summary = tuple(map(int, parts[1:]))
        else:
            raise ValueError(f"unexpected output: {line!r}")
    if missing := expected.keys() - chunks.keys():
        raise ValueError(f"missing {tag} chunk {min(missing)} ({len(missing):,} missing)")
    if rust:
        if summary is None:
            raise ValueError("missing Rust S summary")
        checked, differed = summary
        if not 0 <= differed <= checked <= sum(expected.values()) or checked == 0:
            raise ValueError(f"invalid Rust S summary: {checked} {differed}")
        if tag == "E":
            # Empty, non-surrogate singleton/pair, or one valid surrogate pair.
            utf8_cases = 1 + 63_488 + 63_488**2 + 1_024**2
            if checked != utf8_cases:
                raise ValueError(f"Rust S summary checked {checked:,}; expected {utf8_cases:,}")
    return chunks, summary


def first_difference(build, java, probe, dump_args):
    java_file, rust_file = build / "dump-java.txt", build / "dump-rust.txt"
    lines([*java, "dump", *dump_args, java_file])
    lines([probe, "dump", *dump_args, rust_file])
    with java_file.open() as j, rust_file.open() as r:
        for a, b in zip_longest(j, r):
            if a != b:
                java_row = a.rstrip("\n") if a is not None else "<missing>"
                rust_row = b.rstrip("\n") if b is not None else "<missing>"
                text = f"chunk {' '.join(map(str, dump_args))}\njava {java_row}\nrust {rust_row}\n"
                (build / "first_difference.txt").write_text(text)
                return
    (build / "first_difference.txt").write_text(f"chunk {' '.join(map(str, dump_args))}: dumps agree\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--billions", type=float, default=0.01,
                        help="random strings, in billions (default 0.01; the full run uses 4 or more)")
    parser.add_argument("--threads", type=int, default=os.cpu_count() or 1)
    parser.add_argument("--no-exhaustive", action="store_true", help="skip the 2^32 one- and two-unit strings")
    args = parser.parse_args()
    build = lab.LOCAL / "rust-string-hash-parity"
    build.mkdir(parents=True, exist_ok=True)
    (build / "first_difference.txt").unlink(missing_ok=True)
    classes = build / "classes"
    classes.mkdir(exist_ok=True)
    subprocess.run(["javac", "--release", "25", "-d", classes, ORACLE], check=True)
    subprocess.run(["cargo", "build", "--release", "--quiet", "-p", "sunrise-core", "--bin",
                    "sunrise-string-hash-parity"], check=True, cwd=lab.ROOT)
    probe = lab.binary(lab.cargo_target() / "release", "sunrise-string-hash-parity")
    java = ["java", "-Xmx1G", "-cp", classes, "dev.sunriselab.parity.VanillaStringHashOracle"]
    threads = args.threads

    runs = []
    if not args.no_exhaustive:
        # Chunk zero holds the empty string and every one-unit string; one chunk per first unit thereafter.
        expected = dict.fromkeys(range(1, UTF16_VALUES + 1), UTF16_VALUES)
        expected[0] = UTF16_VALUES + 1
        runs.append(("E", ["exhaustive", threads], lambda c: ["exhaustive", c], expected))
    chunks = max(1, round(args.billions * 1e9 / PER_CHUNK))
    runs.append(("R", ["random", SEED, 0, chunks, PER_CHUNK, threads],
                 lambda c: ["random", SEED, c, PER_CHUNK], dict.fromkeys(range(chunks), PER_CHUNK)))
    start = time.monotonic()
    failed = False
    totals = {"inputs": 0, "chunks matched": 0, "chunks differed": 0, "str checked": 0, "str differed": 0}
    for tag, command, dump, expected in runs:
        for source, cmd in (("Java", [*java, *command]), ("Rust", [probe, *command])):
            try:
                parsed, summary = parse(lines(cmd), tag, expected, rust=source == "Rust")
            except ValueError as error:
                raise SystemExit(f"{NAME}: FAIL: {source}: {error}") from error
            if source == "Java":
                j = parsed
            else:
                r, checked = parsed, summary
        differed = sorted(c for c in expected if j[c] != r[c])
        totals["inputs"] += sum(expected.values())
        totals["chunks matched"] += len(expected) - len(differed)
        totals["chunks differed"] += len(differed)
        totals["str checked"] += checked[0]
        totals["str differed"] += checked[1]
        if differed and not failed:
            first_difference(build, java, probe, dump(differed[0]))
        failed |= bool(differed) or checked[1] > 0
    seconds = time.monotonic() - start
    print(f"{NAME}: {'FAIL' if failed else 'PASS'}: " + ", ".join(f"{k} {v:,}" for k, v in totals.items())
          + f" ({seconds:.0f} s)")
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
