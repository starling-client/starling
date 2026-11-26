#!/usr/bin/env python3
"""Migrate the internal equipment draw tuple and final atlas binding for p3-27."""
from pathlib import Path

root = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src"
draw_tuple = "(&Part, [f32; 4], u32, u32, bool)"
for name, replacements in {
    "mobs/render.rs": [
        (draw_tuple, "WornLayer<'_>"),
        ("pub equipment_texture_final: bool,", "pub equipment_uv: Option<[f32; 4]>,"),
        ("equipment_texture_final: false,", "equipment_uv: None,"),
        ("[(&*armour, [", "[(WornModel::Vanilla(&armour), ["),
        ("&[(part, [0.5, 0.5, 0.75, 0.75]", "&[(WornModel::Vanilla(part), [0.5, 0.5, 0.75, 0.75]"),
        ("(part.as_ref(), [0.5,", "(WornModel::Vanilla(&part), [0.5,"),
    ],
    "mobs/invisible_body.rs": [
        ("FrameDraw, Mob, Part, RenderDataSource, Species,", "FrameDraw, Mob, RenderDataSource, Species, WornLayer,"),
        (draw_tuple, "WornLayer<'_>"),
    ],
    "mobs/render_pose_tests.rs": [
        (draw_tuple, "WornLayer<'_>"),
        ("&*cow.kind.model,", "WornModel::Vanilla(&cow.kind.model),"),
        ("&*villager.kind.model,", "WornModel::Vanilla(&villager.kind.model),"),
    ],
    "game/entities/armor_stands.rs": [
        (draw_tuple, "WornLayer<'_>"),
    ],
}.items():
    file = root / name
    source = file.read_text()
    for old, new in replacements:
        if old not in source:
            raise ValueError(f"{file}: missing {old!r}")
        source = source.replace(old, new)
    file.write_text(source)
