#!/usr/bin/env python3
"""Rename the shared movement continuations without changing their trample payload."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src/mobs"
FILES = (
    "ai.rs",
    "allay.rs",
    "aquatic.rs",
    "dolphin.rs",
    "frog.rs",
    "goals.rs",
    "guardian.rs",
    "living.rs",
    "living/travel.rs",
    "nautilus.rs",
    "turtle.rs",
    "undead.rs",
)
WORDS = {
    "TravelTrample": "TravelPause",
    "travel_until_trample": "travel_until_pause",
    "move_until_trample": "move_until_pause",
    "GoalTrample": "GoalMovementPause",
    "tick_until_trample": "tick_until_movement_pause",
}
SCOPED = {
    "ai.rs": (("goals.resume_after_trample(", "goals.resume_movement("),),
    "dolphin.rs": (("fn after_trample(", "fn after_movement("),),
    "goals.rs": (
        ("fn after_trample(", "fn after_movement("),
        ("g.goal.after_trample(", "g.goal.after_movement("),
        ("fn resume_after_trample(", "fn resume_movement("),
    ),
}


def main():
    words = re.compile(r"\b(?:" + "|".join(WORDS) + r")\b")
    for name in FILES:
        path = ROOT / name
        original = path.read_text(encoding="utf-8")
        renamed = words.sub(lambda match: WORDS[match[0]], original)
        for old, new in SCOPED.get(name, ()):
            renamed = renamed.replace(old, new)
        if renamed != original:
            path.write_text(renamed, encoding="utf-8")


if __name__ == "__main__":
    main()
