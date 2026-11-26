#!/usr/bin/env python3
"""Remove the P3-18 ticket-validation broadcast when landing its replacement.

Run with --check to validate the inputs without touching the worktree. Run
without --check alongside the controller-owned validation change, not alone.
The catalog and dispatcher fixtures are discovered from the remaining files.
"""

import argparse
import hashlib
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
NATIVE_HOOKS = Path("crates/sunrise-client/src/game/native_hooks")
FIXTURES = Path("crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures")

# Exact bytes at the Part 2 baseline (99c5682ffe6). The API fixture only
# declares and re-exports the broadcast's TicketValidation stand-in.
DELETE = {
    Path("crates/sunrise-mod-vocab/src/hooks/validate_tickets.rs"):
        "ac1b7c18b232dddf9d435820ec91805efe6f894cecaaabe3ccf277faa07c579d",
    NATIVE_HOOKS / "validate_tickets.rs":
        "35bda20415d78ea59b0d3a324dfd0e13ed79e693163b096614f0a67e4d7128a4",
    FIXTURES / "validate_tickets.rs":
        "c19903655efca146384ddb769b668e9fcff4631548787ee8429b83e98c87d6ca",
    FIXTURES / "api/mod_chunk_tickets.rs":
        "a710827b0aa8e4c3916f57b1221f99084e490d9f0f962cf4f18385e0b7be049c",
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="validate inputs without editing")
    args = parser.parse_args()

    module = NATIVE_HOOKS / "mod.rs"
    try:
        for relative in (*DELETE, module):
            path = ROOT / relative
            if not path.is_file() or path.resolve() != path:
                parser.exit(1, f"{relative}: expected a regular file at this path\n")

        # Preflight every deletion and both module edits before writing anything.
        for relative, expected in DELETE.items():
            if hashlib.sha256((ROOT / relative).read_bytes()).hexdigest() != expected:
                parser.exit(1, f"{relative}: contents differ from the expected deletion\n")

        source = (ROOT / module).read_bytes()
        edits = (
            (
                b"TicketController",
                b"\n    SpecialModelRenderers, TicketController, ToolUse, VillagerProfessions, WorldPhase,\n};\n",
                b"\n    SpecialModelRenderers, ToolUse, VillagerProfessions, WorldPhase,\n};\n",
            ),
            (
                b"validate_tickets",
                b"\nmod use_item_on_block;\nmod validate_tickets;\n",
                b"\nmod use_item_on_block;\n",
            ),
        )
        for name, old, new in edits:
            if source.count(old) != 1 or len(re.findall(rb"\b" + name + rb"\b", source)) != 1:
                parser.exit(1, f"{module}: expected a single unchanged {name.decode()} anchor\n")
            source = source.replace(old, new, 1)

        if not args.check:
            (ROOT / module).write_bytes(source)
            for relative in DELETE:
                (ROOT / relative).unlink()
    except OSError as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
