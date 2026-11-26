#!/usr/bin/env python3
"""One-off p3-60 migration: guard whole-vector writes in mob travel."""

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src/mobs"
# Limit the rewrite to Entity.move and the selected travel branch and its tail.
REGIONS = {
    "living.rs": {
        "impl TravelTail {": 1,
        "    fn move_until_trample(": 1,
        "    fn finish_move(": 1,
        "    fn travel_in_air(": 1,
        "    fn finish_air_travel(": 2,
        "    fn finish_fluid_travel(": 4,
    },
    "living/travel.rs": {"    pub(in crate::mobs) fn travel_until_trample(": 1},
    "aquatic.rs": {"pub(super) fn travel_in_water(": 2},
    "frog.rs": {"pub(super) fn travel(": 1},
    "guardian.rs": {"pub(super) fn travel(": 1},
    "nautilus.rs": {"pub(super) fn travel(": 1},
    "turtle.rs": {"pub(super) fn travel(": 1},
    "undead.rs": {"pub(super) fn drowned_travel(": 1},
    "vex.rs": {"pub fn travel(": 1},
}
WRITE = re.compile(r"(?m)^( +)(self|mob|m)\.velocity =\s*([^;]+);")


def main():
    changes = {}
    for name, regions in REGIONS.items():
        path = ROOT / name
        source = path.read_text()
        for marker, expected in regions.items():
            start = source.index(marker)
            indent = marker[: len(marker) - len(marker.lstrip())]
            end = source.index(f"\n{indent}}}", start)
            region, count = WRITE.subn(r"\1\2.set_velocity(\3);", source[start:end])
            if count != expected:
                raise SystemExit(f"{name}: {marker} had {count} writes, expected {expected}")
            source = source[:start] + region + source[end:]
        changes[path] = source
    for path, source in changes.items():
        path.write_text(source)


if __name__ == "__main__":
    main()
