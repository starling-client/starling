#!/usr/bin/env python3
"""What the test builds' performance reports say (mod_build_gateway.py keeps them).

    perf_reports.py [FOLDER]     default: /var/lib/private/starling-gateway/perf

For each GPU, the guided tests' ways of drawing: how many phones ran each, and their median FPS
and median slowest-1% FPS. Reports with schema 0 (checks) are skipped.
"""
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path


def describe(choices):
    """`{"draws": "direct", "late_sky": "auto", ...}` as `direct`, `late sky off`, or `auto`."""
    parts = []
    if choices.get("draws", "auto") != "auto":
        parts.append(choices["draws"])
    sky = choices.get("late_sky", "auto")
    if sky != "auto":
        parts.append(f"late sky {sky}")
    return ", ".join(parts) or "auto"


def guided(folder):
    """{gpu: {way: [(fps, low), ...]}} from every guided report in `folder`."""
    found = defaultdict(lambda: defaultdict(list))
    for path in sorted(Path(folder).glob("*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                report = json.loads(line)["report"]
            except (ValueError, KeyError, TypeError):
                continue
            if report.get("schema", 0) < 1 or report.get("kind") != "guided":
                continue
            gpu = (report.get("device") or {}).get("gpu") or "?"
            for step in report.get("guided") or []:
                summary = step.get("summary") or {}
                if summary.get("fps"):
                    found[gpu][describe(step.get("choices") or {})].append(
                        (summary["fps"], summary.get("low_1pct_fps") or 0)
                    )
    return found


def lines(found):
    out = []
    for gpu in sorted(found):
        out.append(gpu)
        ways = found[gpu]
        for way in sorted(ways, key=lambda w: -statistics.median(f for f, _ in ways[w])):
            runs = ways[way]
            fps = statistics.median(f for f, _ in runs)
            low = statistics.median(l for _, l in runs)
            out.append(f"  {way:<24} {len(runs):>3} runs  {fps:6.1f} fps  1% low {low:6.1f}")
    return out


if __name__ == "__main__":
    folder = sys.argv[1] if len(sys.argv) > 1 else "/var/lib/private/starling-gateway/perf"
    print("\n".join(lines(guided(folder))) or "No guided reports yet.")
