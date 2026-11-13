#!/usr/bin/env python3
"""Bisect real Text conversion stack demand in isolated abort-safe processes.

Run after cargo build --bin stack-need in this crate. Pass the binary and the
prepared vanilla en_us.json. Output contains measurements, never game data.
"""
import argparse
import json
import resource
import signal
import subprocess


def attempt(binary, language, size, relation):
    result = subprocess.run(
        [binary, str(size), language, str(relation)], capture_output=True, text=True
    )
    if result.returncode == 0:
        assert "complete" in result.stderr
        return True
    if result.returncode == -signal.SIGABRT and "stack overflow" in result.stderr:
        return False
    raise RuntimeError(f"Unexpected failure: {result.returncode}: {result.stderr}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("language")
    args = parser.parse_args()
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    results = []
    for relation, name in enumerate([
        "extra", "translation argument", "hover text", "selector separator",
        "atlas fallback", "entity hover name", "NBT separator", "player fallback",
    ]):
        low, high = 4, 2048  # Units of 4 KiB; 8 MiB upper bound.
        assert attempt(args.binary, args.language, high * 4096, relation)
        assert not attempt(args.binary, args.language, low * 4096, relation)
        while high - low > 1:
            middle = (low + high) // 2
            if attempt(args.binary, args.language, middle * 4096, relation):
                high = middle
            else:
                low = middle
        # Repeat either side to detect unstable thresholds.
        for _ in range(3):
            assert not attempt(args.binary, args.language, low * 4096, relation)
            assert attempt(args.binary, args.language, high * 4096, relation)
        results.append({"relation": name, "overflow_bytes": low * 4096,
                        "pass_bytes": high * 4096})
    print(json.dumps({"depth": 512, "resolution_bytes": 4096, "results": results}, indent=2))


if __name__ == "__main__":
    main()
