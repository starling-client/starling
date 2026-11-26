#!/usr/bin/env python3
"""Move engine menu constructors into the shared close/replacement sequence."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src/game/network"


def main() -> None:
    for name, expected in (
        ("lan_actions/menus.rs", 2),
        ("lan_crafting.rs", 1),
        ("lan_mount_menu.rs", 1),
        ("lan_stations.rs", 2),
        ("lan_trading.rs", 1),
    ):
        path = ROOT / name
        source = path.read_text()
        # Saving closes the UI without constructing a replacement.
        marker = re.compile(
            r"(?:        // ServerPlayer\.openMenu closes the menu that was open\.\n)?"
            r"        self\.guest_close_all\(conn\);\n"
            r"(?!        self\.lan_crafting_removed)"
        )
        matches = list(marker.finditer(source))
        assert len(matches) == expected, name
        for match in reversed(matches):
            start, body = match.span()
            end = source.index("\n    }", body)
            tail = re.sub(r"\bself\b", "game", source[body:end])
            indented = "\n".join("    " + line if line else line for line in tail.split("\n"))
            source = (
                source[:start]
                + "        self.guest_replace_menu(conn, false, |game| {\n"
                + indented
                + "\n        })"
                + source[end:]
            )
        path.write_text(source)


if __name__ == "__main__":
    main()
