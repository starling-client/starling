#!/usr/bin/env python3
"""Use shared tick-context borrows at the P3-80 join call sites."""
import re
from pathlib import Path


client = Path("crates/sunrise-client")
for path in (*sorted((client / "src/lan").rglob("*.rs")), client / "tests/lan_bot.rs"):
    old = text = path.read_text()
    text = re.sub(r"(\.begin_player_join\([^,\n]+, )&mut ctx\)", r"\1&ctx)", text)
    if text != old:
        path.write_text(text)
