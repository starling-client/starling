"""Mechanical Mob/ArmorStand field move to GroundState (#3728).

Run with --owners once. Feed the JSON output of `cargo check --all-targets
--message-format=json` with --diagnostics, repeating after each dependency
layer, to apply rustc's type-directed field qualifications. Run --position
when living.rs's production supporting fields are all qualified; finish with
--adapters once it is compiled and the downstream field passes are finished.
These adjust array conversions for the shared BlockPos. Semantic changes to
collision sampling are not part of this script.
"""

import argparse
from collections import defaultdict
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/sunrise-client/src"
GROUND = "sunrise_world::ground_support::GroundState"


def change(text: str, old: str, new: str) -> str:
    assert text.count(old) == 1, old
    return text.replace(old, new)


def owners() -> None:
    path = SOURCE / "mobs.rs"
    text = change(
        path.read_text(),
        "    pub on_ground: bool,\n    pub horizontal_collision: bool,\n    pub vertical_collision_below:",
        f"    pub ground: {GROUND},\n    pub vertical_collision_below:",
    )
    text = change(
        text,
        "    /// `Entity.mainSupportingBlockPos`: the block it stood on after its last\n"
        "    /// move.\n    pub supporting: Option<[i32; 3]>,\n",
        "",
    )
    text = change(
        text,
        "            on_ground: false,\n            horizontal_collision: false,\n"
        "            vertical_collision_below:",
        "            ground: Default::default(),\n            vertical_collision_below:",
    )
    path.write_text(change(text, "            supporting: None,\n", ""))

    path = SOURCE / "entities.rs"
    text = change(
        path.read_text(),
        "    pub on_ground: bool,\n    /// `yRot`, which the body follows",
        "    /// `yRot`, which the body follows",
    )
    text = change(text, "    /// `Entity.mainSupportingBlockPos` after the last move.\n"
                  f"    pub support: {GROUND},", f"    pub ground: {GROUND},")
    text = change(text, "            on_ground: false,\n            yaw,", "            yaw,")
    text = change(text, "            support: Default::default(),", "            ground: Default::default(),")
    text = change(text, "match self.support.supporting_block", "match self.ground.supporting_block")
    text = change(text, "            .support\n            .check_support", "            .ground\n            .check_support")
    path.write_text(text)


def apply_diagnostics(path: Path) -> None:
    by_file = defaultdict(dict)
    for line in path.read_text().splitlines():
        record = json.loads(line)
        if record.get("reason") != "compiler-message":
            continue
        message = record["message"]
        if (message.get("code") or {}).get("code") not in {"E0609", "E0615"}:
            continue
        match = re.fullmatch(
            r"(?:no field|attempted to take value of method) "
            r"`(on_ground|horizontal_collision|supporting)` on type `.*\b(?:Mob|ArmorStand)`",
            message["message"],
        )
        if match is None:
            continue
        field = match.group(1).encode()
        spans = [span for span in message["spans"] if span["is_primary"]]
        if len(spans) != 1:
            continue
        span = spans[0]
        start, end = span["byte_start"], span["byte_end"]
        new = b"ground.supporting_block" if field == b"supporting" else b"ground." + field
        source = (ROOT / span["file_name"]).resolve()
        if source.is_relative_to(ROOT / "crates"):
            by_file[source][(start, end)] = (field, new)

    count = 0
    for source, edits in by_file.items():
        data = source.read_bytes()
        for (start, end), (old, new) in sorted(edits.items(), reverse=True):
            assert data[start:end] == old, (source, start, old)
            data = data[:start] + new + data[end:]
            count += 1
        source.write_bytes(data)
    print(f"Qualified {count} living ground field accesses in {len(by_file)} files")


def position() -> None:
    path = SOURCE / "mobs/living.rs"
    text = path.read_text()
    for vector in ("moved", "delta"):
        text = change(text, f"self.ground.supporting_block = self.supporting_block(w, {vector});",
                      f"self.ground.supporting_block = self.supporting_block(w, {vector})"
                      ".map(|[x, y, z]| BlockPos::new(x, y, z));")
    text = text.replace("self.ground.supporting_block.map(|[x, y, z]| BlockPos::new(x, y, z))",
                        "self.ground.supporting_block")
    start = text.index("    fn supported_on_pos(")
    end = text.index("    /// Entity.getOnPos(offset) without", start)
    support = text[start:end].replace("supporting: Option<[i32; 3]>", "supporting: Option<BlockPos>")
    support = support.replace("return p;", "return [p.x, p.y, p.z];").replace("            p\n", "            [p.x, p.y, p.z]\n")
    for index, axis in enumerate("xyz"):
        support = support.replace(f"p[{index}]", f"p.{axis}")
    text = text[:start] + support + text[end:]
    for index, axis in enumerate("xyz"):
        text = text.replace(f"support[{index}]", f"support.{axis}")
    text = text.replace("BlockPos::new(support.x, support.y, support.z)", "support")
    text = text.replace("ground.supporting_block = Some([0, 0, 0]);",
                        "ground.supporting_block = Some(BlockPos::new(0, 0, 0));")
    path.write_text(text)


def adapters() -> None:
    for relative, old, new in (
        ("game/entities/entity.rs", "ground.supporting_block == Some([pos.x, pos.y, pos.z])",
         "ground.supporting_block == Some(pos)"),
        ("game/mobs/mob_landing_support_tests.rs", "pig.ground.supporting_block,",
         "pig.ground.supporting_block.map(|p| [p.x, p.y, p.z]),"),
        ("game/interface/client_ambient.rs", ".and_then(|m| m.ground.supporting_block),",
         ".and_then(|m| m.ground.supporting_block).map(|p| [p.x, p.y, p.z]),"),
        ("game/network/remote/packets_entity.rs", "m.ground.supporting_block);",
         "m.ground.supporting_block.map(|p| [p.x, p.y, p.z]));"),
    ):
        path = SOURCE / relative
        path.write_text(change(path.read_text(), old, new))
    for relative in ("mobs/living.rs", "mobs/living_tests.rs", "mobs/entity_sounds_tests.rs"):
        path = SOURCE / relative
        path.write_text(path.read_text().replace(
            "ground.supporting_block = Some([0, 0, 0]);",
            "ground.supporting_block = Some(sunrise_core::position::BlockPos::new(0, 0, 0));",
        ))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--owners", action="store_true")
    group.add_argument("--diagnostics", type=Path)
    group.add_argument("--position", action="store_true")
    group.add_argument("--adapters", action="store_true")
    args = parser.parse_args()
    if args.owners:
        owners()
    elif args.position:
        position()
    elif args.adapters:
        adapters()
    else:
        apply_diagnostics(args.diagnostics)
