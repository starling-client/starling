#!/usr/bin/env python3
"""Compare the resident view and handle paths, with and without an ambient transaction.

    cargo build --locked --release -p sunrise-hot-paths-game --features sunrise-client/alloc-count
    python3 tools/block_entity_view_bench.py --resources PATH --work target/block-entity-view-run

The work directory must be new. It keeps raw per-process samples and logs. Use
--valgrind PATH to collect Callgrind Ir, or omit it for native time and allocator
counts only. Neither calibration (empty loop or no listener) is subtracted.
"""
import argparse
import json
import os
import platform
from pathlib import Path

import hot_paths


ROUTES = (
    "view_read", "handle_read", "view_write", "handle_write",
    "view_tx_read", "handle_tx_read", "view_tx_write", "handle_tx_write",
    "no_listener", "empty",
)
PREFIX = "mod.block_entity_view_v2."


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game", type=Path, default=Path("target/release/sunrise-hot-paths-game"))
    parser.add_argument("--resources", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    parser.add_argument("--valgrind")
    parser.add_argument("--samples", type=int, default=5)
    args = parser.parse_args()
    work = args.work.resolve()
    work.mkdir(parents=True)
    game = hot_paths.install(args.game.resolve(), work, 0)
    resources = args.resources.resolve()
    metadata = {
        "commit": hot_paths.first_line(["git", "rev-parse", "HEAD"]),
        "cpu": hot_paths.cpu_model(),
        "kernel": platform.release(),
        "rustc": hot_paths.first_line(["rustc", "--version"]),
        "valgrind": hot_paths.first_line([args.valgrind, "--version"]) if args.valgrind else None,
        "instruction_scope": "ticker thread" if args.valgrind else None,
    }
    (work / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    checksum = {}
    with (work / "samples.jsonl").open("w") as samples:
        for instrumented in ((False, True) if args.valgrind else (False,)):
            for repeat in range(args.samples):
                # Rotate the route order rather than measuring every sample of one route first.
                offset = repeat % len(ROUTES)
                for route in ROUTES[offset:] + ROUTES[:offset]:
                    name = PREFIX + route
                    os.environ["SUNRISE_BLOCK_ENTITY_VIEW_BENCH"] = route
                    mods = "none" if route in ("empty", "no_listener") else "native"
                    folder = work / f"{'ir' if instrumented else 'native'}-{repeat}-{route}"
                    if instrumented:
                        counts, rows = hot_paths.callgrind(
                            args.valgrind, game, resources, folder, mods, name, separate_threads=True
                        )
                        if name not in counts:
                            raise SystemExit(f"Callgrind did not report {name}; see {folder}")
                        sample = {"instructions": counts[name][0]}
                    else:
                        rows = hot_paths.run_game(game, resources, folder, mods, name)
                        sample = {k: rows[name][k] for k in ("wall_ns", "allocations", "allocated_bytes")}
                        if sample["allocations"] is None:
                            raise SystemExit("benchmark binary was built without sunrise-client/alloc-count")
                    row = rows[name]
                    group = "write" if route.endswith("_write") else "no_write"
                    observed = (row["checksum"], row["operations"])
                    if checksum.setdefault(group, observed) != observed:
                        raise SystemExit(f"{route} ran different work: {observed} vs {checksum[group]}")
                    sample.update(route=route, repeat=repeat, checksum=row["checksum"], operations=row["operations"])
                    print(json.dumps(sample), file=samples, flush=True)
                    print(json.dumps(sample), flush=True)


if __name__ == "__main__":
    main()
