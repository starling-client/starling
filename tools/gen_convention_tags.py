#!/usr/bin/env python3
"""Generate Sunrise's built-in convention tag data and typed `mod_api::tags::c` constants.

With `--from-neoforge <PATH>`, reads NeoForge 26.3's generated tag files
(`<PATH>/src/generated/resources/data/{c,neoforge}/tags/**/*.json`) and `Tags.java`
to regenerate `crates/sunrise-world/data/convention_tags.json` verbatim (preserving
exact tag IDs, `#` references, `{"id": ..., "required": false}` entries, and value
order), then regenerates `crates/sunrise-mod-vocab/src/world/convention_tags.rs`.

Without `--from-neoforge`, reads `crates/sunrise-world/data/convention_tags.json`
and writes (or `--check`s) `crates/sunrise-mod-vocab/src/world/convention_tags.rs`
so the runtime base tag layer and the typed `mod_api::tags::c::*` constants share
one source of truth and cannot drift.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
DATA_PATH = ROOT / "crates/sunrise-world/data/convention_tags.json"
OUT_PATH = ROOT / "crates/sunrise-mod-vocab/src/world/convention_tags.rs"

IDENT_RE = re.compile(r"^[a-z0-9_.-]+:[a-z0-9_./-]+$")
CONST_RE = re.compile(r"^[A-Z][A-Z0-9_]*$")
MOD_RE = re.compile(r"^[a-z][a-z0-9_]*$")

# NeoForge 26.3's generated convention tag registries, in canonical output order.
REGISTRIES: tuple[tuple[str, str, str, str], ...] = (
    ("block", "blocks", "BlockKey", "Built-in `c:` and `neoforge:` block convention tags."),
    ("item", "items", "ItemKey", "Built-in `c:` and `neoforge:` item convention tags."),
    ("fluid", "fluids", "FluidKey", "Built-in `c:` fluid convention tags."),
    ("entity_type", "entity_types", "EntityTypeKey", "Built-in `c:` entity type convention tags."),
    ("worldgen/biome", "biomes", "BiomeKey", "Built-in `c:` biome convention tags."),
    ("enchantment", "enchantments", "EnchantmentKey", "Built-in `c:` enchantment convention tags."),
    ("worldgen/structure", "structures", "StructureKey", "Built-in `c:` structure convention tags."),
    ("potion", "potions", "PotionKey", "Built-in `c:` potion convention tags."),
    ("damage_type", "damage_types", "DamageTypeKey", "Built-in `neoforge:` damage type convention tags."),
)

JAVA_CLASS_TO_REGS: dict[str, tuple[str, ...]] = {
    "Blocks": ("block",),
    "BlockItems": ("block", "item"),
    "Items": ("item",),
    "Fluids": ("fluid",),
    "EntityTypes": ("entity_type",),
    "Biomes": ("worldgen/biome",),
    "Enchantments": ("enchantment",),
    "Structures": ("worldgen/structure",),
    "Potions": ("potion",),
    "DamageTypes": ("damage_type",),
}

JAVA_CLASS_RE = re.compile(r"public static class (\w+)\s*\{")
JAVA_TAG_RE = re.compile(
    r'public static final (?:TagKey<.*?>|BlockItemTagId)\s+([A-Z0-9_]+)\s*=\s*(\w+)\("([^"]+)"\);'
)


def parse_neoforge_const_names(neoforge_root: pathlib.Path) -> dict[tuple[str, str], str]:
    tags_java = neoforge_root / "src/main/java/net/neoforged/neoforge/common/Tags.java"
    if not tags_java.is_file():
        raise FileNotFoundError(f"NeoForge Tags.java not found at {tags_java}")
    cur_regs: tuple[str, ...] | None = None
    const_map: dict[tuple[str, str], str] = {}
    for line in tags_java.read_text(encoding="utf-8").splitlines():
        cls = JAVA_CLASS_RE.search(line)
        if cls:
            cur_regs = JAVA_CLASS_TO_REGS.get(cls.group(1))
            continue
        if cur_regs is None:
            continue
        match = JAVA_TAG_RE.search(line)
        if not match:
            continue
        const_name, helper, path = match.groups()
        ns = "neoforge" if "neoforge" in helper.lower() else "c"
        for reg in cur_regs:
            const_map[(reg, f"{ns}:{path}")] = const_name
    return const_map


def validate_tag_value(entry: object, where: str) -> None:
    if isinstance(entry, str):
        if not entry:
            raise ValueError(f"{where}: empty tag value string")
        return
    if isinstance(entry, dict):
        if set(entry.keys()) - {"id", "required"}:
            raise ValueError(f"{where}: unexpected keys in tag value object {entry!r}")
        if not isinstance(entry.get("id"), str) or not entry["id"]:
            raise ValueError(f"{where}: tag value object missing non-empty 'id': {entry!r}")
        if "required" in entry and not isinstance(entry["required"], bool):
            raise ValueError(f"{where}: tag value object 'required' must be bool: {entry!r}")
        return
    raise ValueError(f"{where}: expected string or object tag value, got {entry!r}")


def import_from_neoforge(neoforge_root: pathlib.Path) -> dict:
    data_root = neoforge_root / "src/generated/resources/data"
    if not data_root.is_dir():
        raise FileNotFoundError(f"NeoForge generated data directory not found at {data_root}")

    const_map = parse_neoforge_const_names(neoforge_root)
    known_regs = {reg for reg, _, _, _ in REGISTRIES}
    by_reg: dict[str, list[dict]] = {reg: [] for reg in known_regs}

    for ns in ("c", "neoforge"):
        tags_dir = data_root / ns / "tags"
        if not tags_dir.is_dir():
            continue
        for json_path in sorted(tags_dir.rglob("*.json")):
            rel = json_path.relative_to(tags_dir).as_posix()
            parts = rel.split("/")
            if parts[0] == "worldgen":
                if len(parts) < 3:
                    raise ValueError(f"{json_path}: unexpected worldgen tag path {rel!r}")
                reg = f"{parts[0]}/{parts[1]}"
                tag_path = "/".join(parts[2:])[:-5]
            else:
                if len(parts) < 2:
                    raise ValueError(f"{json_path}: unexpected tag path {rel!r}")
                reg = parts[0]
                tag_path = "/".join(parts[1:])[:-5]
            if reg not in known_regs:
                raise ValueError(f"{json_path}: unknown convention tag registry {reg!r}")
            raw = json.loads(json_path.read_text(encoding="utf-8"))
            if not isinstance(raw, dict) or set(raw.keys()) != {"values"}:
                raise ValueError(f"{json_path}: expected object with only 'values' key")
            values = raw["values"]
            if not isinstance(values, list):
                raise ValueError(f"{json_path}: 'values' must be a list")
            tag_id = f"{ns}:{tag_path}"
            for entry in values:
                validate_tag_value(entry, f"{json_path} ({tag_id})")
            const_name = const_map.get((reg, tag_id), tag_path.upper().replace("/", "_"))
            by_reg[reg].append(
                {
                    "const": const_name,
                    "id": tag_id,
                    "values": values,
                }
            )

    registries_out = []
    for reg, mod, key_type, doc in REGISTRIES:
        tags = sorted(by_reg[reg], key=lambda item: item["id"])
        if not tags:
            raise ValueError(f"NeoForge generated data produced no tags for registry {reg!r}")
        registries_out.append(
            {
                "registry": reg,
                "module": mod,
                "key_type": key_type,
                "doc": doc,
                "tags": tags,
            }
        )
    return {"registries": registries_out}


def load_spec(path: pathlib.Path) -> list[dict]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    registries = raw.get("registries")
    if not isinstance(registries, list) or not registries:
        raise ValueError(f"{path}: expected non-empty 'registries' array")
    seen_modules: set[str] = set()
    seen_registries: set[str] = set()
    for entry in registries:
        reg = entry["registry"]
        mod = entry["module"]
        key_type = entry["key_type"]
        tags = entry["tags"]
        if not MOD_RE.match(mod):
            raise ValueError(f"{path}: invalid module name {mod!r}")
        if mod in seen_modules:
            raise ValueError(f"{path}: duplicate module {mod!r}")
        if reg in seen_registries:
            raise ValueError(f"{path}: duplicate registry {reg!r}")
        seen_modules.add(mod)
        seen_registries.add(reg)
        if not key_type:
            raise ValueError(f"{path}: empty key_type for {mod}")
        seen_consts: set[str] = set()
        seen_ids: set[str] = set()
        for tag in tags:
            const_name = tag["const"]
            tag_id = tag["id"]
            values = tag.get("values")
            if not CONST_RE.match(const_name):
                raise ValueError(f"{path}: invalid const name {const_name!r} in {mod}")
            if not IDENT_RE.match(tag_id):
                raise ValueError(f"{path}: invalid tag id {tag_id!r} in {mod}")
            if const_name in seen_consts:
                raise ValueError(f"{path}: duplicate const {const_name} in {mod}")
            if tag_id in seen_ids:
                raise ValueError(f"{path}: duplicate tag id {tag_id} in {mod}")
            if not isinstance(values, list):
                raise ValueError(f"{path}: missing list 'values' for {tag_id} in {mod}")
            for value in values:
                validate_tag_value(value, f"{path} ({reg} {tag_id})")
            seen_consts.add(const_name)
            seen_ids.add(tag_id)
    return registries


def render(registries: list[dict]) -> str:
    lines: list[str] = [
        "// Generated by tools/gen_convention_tags.py from crates/sunrise-world/data/convention_tags.json.",
        "// Do not edit by hand; re-run `python3 tools/gen_convention_tags.py`.",
        "",
        "//! Built-in `c:` and `neoforge:` convention tag constants, grouped by registry.",
        "//!",
        "//! Each constant is typed by its target registry key (`TagKey<ItemKey>`,",
        "//! `TagKey<BlockKey>`, and so on) so a tag from one registry cannot be passed to",
        "//! a query for another. Membership comes from Sunrise's built-in convention tag",
        "//! layer, merged beneath data packs and mods.",
        "",
        "/// Built-in `c:` and `neoforge:` convention tag constants, grouped by registry.",
        "pub mod c {",
    ]
    for reg in registries:
        mod = reg["module"]
        key_type = reg["key_type"]
        doc = reg["doc"]
        lines.append(f"    /// {doc}")
        lines.append(f"    pub mod {mod} {{")
        lines.append(f"        use crate::{{{key_type}, TagKey}};")
        lines.append("")
        for tag in reg["tags"]:
            const_name = tag["const"]
            tag_id = tag["id"]
            lines.append(f"        /// Convention tag `{tag_id}`.")
            lines.append(
                f'        pub const {const_name}: TagKey<{key_type}> = TagKey::from_static("{tag_id}");'
            )
        lines.append("    }")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


def format_rust(source: str) -> str:
    result = subprocess.run(
        ["rustfmt", "--edition", "2024"],
        input=source,
        text=True,
        capture_output=True,
        check=True,
    )
    return result.stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify the generated file matches the data file without writing.",
    )
    parser.add_argument(
        "--from-neoforge",
        type=pathlib.Path,
        metavar="PATH",
        help="Regenerate convention_tags.json from a NeoForge 26.3 checkout before generating Rust constants.",
    )
    args = parser.parse_args()

    if args.from_neoforge is not None:
        if args.check:
            parser.error("--check cannot be combined with --from-neoforge")
        imported = import_from_neoforge(args.from_neoforge)
        DATA_PATH.parent.mkdir(parents=True, exist_ok=True)
        DATA_PATH.write_text(json.dumps(imported, indent=2) + "\n", encoding="utf-8")
        imported_total = sum(len(r["tags"]) for r in imported["registries"])
        print(
            f"Imported {DATA_PATH.relative_to(ROOT)} "
            f"({imported_total} tags across {len(imported['registries'])} registries)"
        )

    registries = load_spec(DATA_PATH)
    rendered = format_rust(render(registries))

    if args.check:
        if not OUT_PATH.exists():
            print(f"error: {OUT_PATH} does not exist", file=sys.stderr)
            return 1
        current = OUT_PATH.read_text(encoding="utf-8")
        if current != rendered:
            print(
                f"error: {OUT_PATH} is out of date; run `python3 tools/gen_convention_tags.py`",
                file=sys.stderr,
            )
            return 1
        return 0

    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(rendered, encoding="utf-8")
    total = sum(len(r["tags"]) for r in registries)
    print(f"Generated {OUT_PATH.relative_to(ROOT)} ({total} constants across {len(registries)} modules)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
