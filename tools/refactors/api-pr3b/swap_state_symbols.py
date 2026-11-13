#!/usr/bin/env python3
"""Log the mechanical API-owned state/key symbol cut after handwritten prerequisites.

Dry-run by default. Public symbols change only inside mod_api. Two Game-only imports
move off the deleted alias without changing numeric engine storage. Raw API constructor
fixtures are refused unless explicitly deferred to the following handwritten commit.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--root", type=Path, default=Path.cwd())
parser.add_argument("--report", type=Path, required=True)
parser.add_argument("--apply", action="store_true")
parser.add_argument("--defer-constructor-fixtures", action="store_true", help="rename the two known constructor fixtures but defer semantic repairs to handwritten commits")
parser.add_argument("--format", action="store_true", help="include touched-file rustfmt output in the logged transformation")
args = parser.parse_args()
root = args.root.resolve()
api = root / "crates/horizon-mod-api/src"
if not (root / "Cargo.toml").is_file() or not api.is_dir():
    parser.error("root must be the Horizon checkout")

symbols = re.compile(r"\b(BlockStateId|BlockId)\b")
constructors = re.compile(r"\b(?:BlockStateId|BlockId)\s*\(")
old_export = "    horizon_world::{level::BlockId, registry::BlockStateId},\n"
changes = []
pending = []
refused = []
hand_fixes = []


def record_change(path, source, replacement):
    if args.format:
        replacement = subprocess.run(
            ["rustfmt", "--edition", "2024", "--config", "skip_children=true", "--emit", "stdout"],
            input=replacement, text=True, capture_output=True, check=True,
        ).stdout
    changes.append({
        "file": path.relative_to(root).as_posix(),
        "before_sha256": hashlib.sha256(source.encode()).hexdigest(),
        "after_sha256": hashlib.sha256(replacement.encode()).hexdigest(),
        "states": len(re.findall(r"\bBlockStateId\b", source)),
        "keys": len(re.findall(r"\bBlockId\b", source)),
        "states_after": len(re.findall(r"\bBlockStateId\b", replacement)),
        "keys_after": len(re.findall(r"\bBlockId\b", replacement)),
        "removed_lines": len(source.splitlines()) - len(replacement.splitlines()),
    })
    pending.append((path, replacement))


for path in sorted(api.rglob("*.rs")):
    source = path.read_text()
    if not symbols.search(source):
        continue
    relative = path.relative_to(root).as_posix()
    if relative == "crates/horizon-mod-api/src/exports/horizon_world.rs":
        if source.count(old_export) != 1:
            refused.append({"file": relative, "reason": "raw export shape changed; inspect it"})
            continue
        replacement = source.replace(old_export, "")
    else:
        if constructors.search(source):
            if args.defer_constructor_fixtures and relative in {
                "crates/horizon-mod-api/src/wasm_key_tests.rs",
                "crates/horizon-mod-api/src/parts/blocks.rs",
            }:
                hand_fixes.append({"file": relative, "reason": "raw numeric wire fixture renamed only; replace with owned record and field-tuple comparison by hand"})
            else:
                refused.append({"file": relative, "reason": "numeric constructor fixture must be migrated by hand"})
        if re.search(r"horizon_world::[^;\n]*\b(?:BlockStateId|BlockId)\b", source):
            refused.append({"file": relative, "reason": "qualified engine import needs boundary ownership review"})
        replacement = symbols.sub(lambda match: "BlockState" if match[0] == "BlockStateId" else "BlockKey", source)
    record_change(path, source, replacement)

# These private native methods consume engine states, not API payload values.
for relative in [
    "crates/horizon-client/src/game/modding/native_entity_refs.rs",
    "crates/horizon-client/src/game/modding/native_block_sprites.rs",
]:
    path = root / relative
    if not path.is_file():
        continue
    source = path.read_text()
    replacement = source.replace("mod_api::BlockStateId", "horizon_world::registry::BlockStateId")
    if replacement != source:
        record_change(path, source, replacement)

# Remove only the two transitional explicit-raw lookup arms. Inferred type
# normalization and the numeric engine carrier conversion must remain untouched.
dispatch = root / "crates/horizon-mod-api-gen/src/dispatch.rs"
if dispatch.is_file():
    source = dispatch.read_text()
    first = '                    "BlockStateId" if table == "block" => {'
    following = '                    "& ItemStack" => {'
    if source.count(first) != 1 or source.count(following) != 1:
        refused.append({"file": dispatch.relative_to(root).as_posix(), "reason": "transitional dispatch arm shape changed"})
    else:
        start, end = source.index(first), source.index(following)
        removed = source[start:end]
        if removed.count('"BlockStateId"') != 2:
            refused.append({"file": dispatch.relative_to(root).as_posix(), "reason": "expected exactly two raw lookup arms"})
        else:
            record_change(dispatch, source, source[:start] + source[end:])

report = {"applied": args.apply and not refused, "files": changes, "refused": refused, "hand_fixes": hand_fixes, "formatted": args.format}
args.report.parent.mkdir(parents=True, exist_ok=True)
args.report.write_text(json.dumps(report, indent=2) + "\n")
if args.apply:
    if refused:
        raise SystemExit("handwritten prerequisites remain; no source files changed")
    for path, replacement in pending:
        path.write_text(replacement)
print(json.dumps({"files": len(changes), "refused": refused, "applied": report["applied"]}, indent=2))
