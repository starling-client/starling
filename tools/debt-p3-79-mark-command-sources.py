#!/usr/bin/env python3
"""Initialize the native-console output marker on existing command source roots."""

from pathlib import Path
import re


game = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src/game"
for file in (
    "administration/command_blocks.rs",
    "administration/command_minecarts.rs",
    "administration/commands/execute.rs",
    "interface/sign_edit.rs",
    "modding/native_entity_commands.rs",
):
    path = game / file
    source = path.read_text()
    source, count = re.subn(
        r"(?m)^( +)origin_player: (?:false|self\.player\.is_some\(\)),\n(?! +native_output:)",
        r"\g<0>\1native_output: None,\n",
        source,
    )
    expected = 2 if file.endswith("/execute.rs") else 1
    if count not in (0, expected):
        raise SystemExit(f"{file}: expected {expected} source constructors, found {count}")
    path.write_text(source)
