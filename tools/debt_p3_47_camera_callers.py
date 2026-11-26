#!/usr/bin/env python3
"""One-time P3-47 migration of the camera event and renderer fixture call sites."""

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CAMERA_EVENT = [
    ("ComputeCameraAnglesEvent<'_>", "ComputeCameraAnglesEvent"),
    ("*event.yaw", "event.yaw"),
]
CHANGES = {
    "crates/sunrise-client/src/game/native_hooks/camera_type.rs": CAMERA_EVENT
    + [("*event.roll", "event.roll")],
    "crates/sunrise-mod-host/examples/native_gui_overlay_probe.rs": CAMERA_EVENT,
    "crates/sunrise-client-render/fixtures/world_views.rs": [
        ("85.0, 0.0, 0.0, glam::Mat4::IDENTITY", "85.0, 0.0, 0.0, 0.0, glam::Mat4::IDENTITY")
    ],
    "crates/sunrise-client-render/examples/audio_bench.rs": [
        ("            angle,\n            0.0,", "            [-angle.to_radians().sin(), 0.0, angle.to_radians().cos()],\n            [0.0, 1.0, 0.0],"),
        ("set_listener([100.0, 64.0, 200.0], 0.0, 0.0)",
         "set_listener([100.0, 64.0, 200.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0])"),
    ],
}

for name, replacements in CHANGES.items():
    path = ROOT / name
    source = path.read_text()
    for old, new in replacements:
        if old not in source:
            raise SystemExit(f"{name}: missing {old!r}")
        source = source.replace(old, new)
    path.write_text(source)
