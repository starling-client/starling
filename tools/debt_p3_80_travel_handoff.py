#!/usr/bin/env python3
"""Thread the pending level change through the hub-only command send fixtures."""
import re
from pathlib import Path


client = Path("crates/sunrise-client")
for path in (*sorted((client / "src/lan").rglob("*.rs")), client / "tests/lan_bot.rs"):
    old = path.read_text()
    text = re.sub(
        r"(?m)^([ \t]*)((?P<hub>(?:\w+\.)?hub)\.begin_level_change\(0, [^\n]+\);\n"
        r"\s*(?P=hub)\s*\.send_commands\(0, (?P=hub)\.commands_packet\((?P=hub)\.guests\[0\]\.permission\)\);\n"
        r"\s*(?P=hub)\.finish_level_change\(0, )",
        r"\1let change = \2change, ",
        old,
    )
    if text != old:
        path.write_text(text)
