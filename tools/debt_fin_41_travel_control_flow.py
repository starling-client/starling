#!/usr/bin/env python3
"""Represent borrowed travel suspension as control flow, not an error."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src/mobs"
FILES = (
    "ai.rs", "allay.rs", "aquatic.rs", "dolphin.rs", "frog.rs", "goals.rs", "guardian.rs",
    "living.rs", "living/travel.rs", "nautilus.rs", "turtle.rs", "undead.rs",
)
SCOPED = {
    "ai.rs": (
        ("match travel.resume(m, w, level, events) {\n            Ok(())", "match travel.resume(m, w, level, events) {\n            ControlFlow::Continue(())"),
        ("Err(travel) => Some(MovementPhase::Travel(travel)),", "ControlFlow::Break(travel) => Some(MovementPhase::Travel(travel)),"),
        (".err()\n                    .map(MovementPhase::Travel)", ".break_value()\n                    .map(MovementPhase::Travel)"),
        (".err()\n        .map(MovementPhase::Travel)", ".break_value()\n        .map(MovementPhase::Travel)"),
        ("    Ok(())\n}\n\n/// The end of `LivingEntity.aiStep`", "    ControlFlow::Continue(())\n}\n\n/// The end of `LivingEntity.aiStep`"),
    ),
    "dolphin.rs": (("if let Err(movement) = m.travel_move(", "if let ControlFlow::Break(movement) = m.travel_move("),),
    "goals.rs": (("if let Err(movement) = paused.movement.resume(", "if let ControlFlow::Break(movement) = paused.movement.resume("),),
    "living.rs": (
        (") -> Result<(), Self> {\n        let TravelStop::", ") -> ControlFlow<Self> {\n        let TravelStop::"),
        ("Err(TravelPause(TravelStop::", "ControlFlow::Break(TravelPause(TravelStop::"),
        ("Err(Self(TravelStop::", "ControlFlow::Break(Self(TravelStop::"),
        ("        Ok(())\n    }\n}\n\n/// Travel stopped", "        ControlFlow::Continue(())\n    }\n}\n\n/// Travel stopped"),
        ("            return Ok(());\n        };\n        if let Some(fall) = mob.resume_fall", "            return ControlFlow::Continue(());\n        };\n        if let Some(fall) = mob.resume_fall"),
        ("        Ok(())\n    }\n}\n\n#[derive(Debug)]\nstruct MovePause", "        ControlFlow::Continue(())\n    }\n}\n\n#[derive(Debug)]\nstruct MovePause"),
    ),
    "living/travel.rs": (("Ok(())", "ControlFlow::Continue(())"),),
}

for name in FILES:
    path = ROOT / name
    original = path.read_text(encoding="utf-8")
    updated = original.replace("Result<(), TravelPause>", "ControlFlow<TravelPause>")
    updated = updated.replace("Result<(), super::living::TravelPause>", "ControlFlow<super::living::TravelPause>")
    for old, new in SCOPED.get(name, ()):
        updated = updated.replace(old, new)
    if updated != original:
        lines = updated.splitlines(keepends=True)
        insertion = next(i for i, line in enumerate(lines) if line.startswith("use "))
        lines.insert(insertion, "use std::ops::ControlFlow;\n")
        path.write_text("".join(lines), encoding="utf-8")
