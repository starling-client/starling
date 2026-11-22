#!/usr/bin/env python3
"""Compare sunrise_core::modified_utf8 with JDK 25's DataOutputStream.writeUTF and DataInputStream.readUTF.

Both sides generate the same inputs from the same seed (VanillaModifiedUtf8Oracle.java and sunrise-core's
src/bin/modified_utf8_parity.rs) and print one digest per chunk; a chunk that differs is dumped record by record on
both sides and its first difference written to .local/modified-utf8-parity/first-difference.txt.

Modes (encode): every single unit; every surrogate pair, lone surrogate and reversed pair; every ordered pair of
units (2^32 with --full, a sample otherwise); the 65,535-byte edge, with the NBT writers' handling of a string past
it; random strings. (Decode): every 1-, 2- and 3-byte sequence; random byte strings; random strings' encodings.
--full runs every pair and 2^32 random inputs per random mode.

Prints the harness name and pass/fail counts only.
"""
import argparse
import os
from pathlib import Path
import subprocess
import time

try:
    from . import lab
except ImportError:
    import lab

ORACLE = "dev.sunriselab.parity.VanillaModifiedUtf8Oracle"
SOURCE = lab.ROOT / "mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaModifiedUtf8Oracle.java"
BIN = "sunrise-modified-utf8-parity"
PAIR_CHUNKS = 1024  # 64 first units each
RANDOM_CHUNK = 1 << 20


def modes(full, random_chunks):
    """(mode, first chunk, chunk count)."""
    pairs = PAIR_CHUNKS if full else 8
    random = 4096 if full else random_chunks
    return [("enc-singles", 0, 1), ("enc-surrogates", 0, 1024), ("enc-edge", 0, 1), ("enc-pairs", 0, pairs),
            ("enc-random", 0, random), ("dec-exhaustive", 0, 257), ("dec-random", 0, random),
            ("dec-valid", 0, random)]


def chunks(output):
    """{chunk: (records, digest)} from one side's output."""
    out = {}
    for line in output.splitlines():
        chunk, records, digest = line.split()
        out[int(chunk)] = (int(records), digest)
    return out


def first_difference(java, probe, mode, chunk, build):
    """Dumps `chunk` on both sides and writes their first differing record to first-difference.txt."""
    jdk = subprocess.run([*java, "dump", mode, str(chunk)], capture_output=True, text=True, check=True).stdout
    rust = subprocess.run([probe, "dump", mode, str(chunk)], capture_output=True, text=True, check=True).stdout
    path = build / "first-difference.txt"
    for i, (a, b) in enumerate(zip(jdk.splitlines(), rust.splitlines())):
        if a != b:
            path.write_text(f"mode {mode} chunk {chunk} record {i}\njdk  {a}\nrust {b}\n")
            return
    path.write_text(f"mode {mode} chunk {chunk}: jdk {len(jdk.splitlines())} records, rust {len(rust.splitlines())}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--full", action="store_true", help="every pair of units and 2^32 inputs per random mode")
    parser.add_argument("--random-chunks", type=int, default=4, help=f"chunks of {RANDOM_CHUNK:,} per random mode")
    parser.add_argument("--threads", type=int, default=os.cpu_count() or 1)
    args = parser.parse_args()
    build = lab.LOCAL / "modified-utf8-parity"
    classes = build / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    (build / "first-difference.txt").unlink(missing_ok=True)
    subprocess.run(["javac", "-d", str(classes), str(SOURCE)], check=True)
    subprocess.run(["cargo", "build", "--release", "--quiet", "-p", "sunrise-core", "--bin", BIN], cwd=lab.ROOT,
                   check=True)
    probe = str(lab.binary(lab.cargo_target() / "release", BIN))
    java = ["java", "-Xmx1G", "-cp", str(classes), ORACLE]
    failed = 0
    started = time.monotonic()
    timings = {}
    for mode, first, count in modes(args.full, args.random_chunks):
        began = time.monotonic()
        command = [mode, str(first), str(count), str(args.threads)]
        jdk = subprocess.Popen([*java, *command], stdout=subprocess.PIPE, text=True)
        rust = subprocess.run([probe, *command], capture_output=True, text=True, check=True).stdout
        jdk_out = jdk.communicate()[0]
        if jdk.returncode:
            raise SystemExit(f"modified_utf8: the oracle failed on {mode}")
        expected, actual = chunks(jdk_out), chunks(rust)
        passed = fail = 0
        bad = None
        for chunk in range(first, first + count):
            records, digest = expected.get(chunk, (0, None))
            if digest is not None and actual.get(chunk) == (records, digest):
                passed += records
            else:
                fail += max(records, actual.get(chunk, (0, None))[0], 1)
                bad = chunk if bad is None else bad
        if bad is not None and not (build / "first-difference.txt").exists():
            first_difference(java, probe, mode, bad, build)
        failed += fail
        timings[mode] = round(time.monotonic() - began, 1)
        print(f"modified_utf8 {mode}: {passed:,} pass, {fail:,} fail", flush=True)
    expected = subprocess.run([*java, "callers"], capture_output=True, text=True, check=True).stdout.splitlines()
    actual = subprocess.run([probe, "callers"], capture_output=True, text=True, check=True).stdout.splitlines()
    # 6 lengths x 3 unit widths: every record must be there, and equal to the bytes writeUTF gives.
    differs = (len(expected) != 18 or len(actual) != 18) + sum(1 for a, b in zip(expected, actual) if a != b)
    print(f"modified_utf8 nbt-writers-edge: {len(actual) - differs:,} pass, {differs:,} fail", flush=True)
    failed += differs
    lab.write_json(build / "report.json", {"full": args.full, "threads": args.threads, "seconds": timings,
                                           "total_seconds": round(time.monotonic() - started, 1), "failed": failed})
    if failed:
        raise SystemExit(f"modified_utf8: FAIL ({build / 'first-difference.txt'})")
    print("modified_utf8: PASS", flush=True)


if __name__ == "__main__":
    main()
