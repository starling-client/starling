#!/usr/bin/env python3
"""Remove the LAN border diff and its per-level cache before wiring the real setters."""
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src"


def remove(file: str, blocks: list[tuple[str, int]]) -> None:
    path = ROOT / file
    source = path.read_text()
    for block, count in blocks:
        assert source.count(block) == count, (file, block)
        source = source.replace(block, "")
    path.write_text(source)


remove("game/network/lan/mod.rs", [
    ("    /// The world border as the guests were last told of it (lan/border.rs).\n"
     "    pub border_sent: Option<lan::border::Sent>,\n", 1),
    ("\n            border_sent: None,", 1),
    ("\n                border_sent: None,", 2),
    ("        self.lan_sync_border();\n", 1),
])
remove("game/world/levels.rs", [
    (", and what its players were\n"
     "    /// last told of it (`LanHost::border_sent`)", 1),
    ("    border_sent: Option<sunrise_client::lan::border::Sent>,\n", 1),
    ("\n            border_sent: None,", 1),
    ("            std::mem::swap(&mut h.border_sent, &mut p.border_sent);\n", 1),
])

path = ROOT / "game/world/world_border.rs"
source = path.read_text()
start = source.index("    /// Every tick:")
end = source.index("    pub(in crate::game) fn tick_worldborder", start)
path.write_text(source[:start] + source[end:])

path = ROOT / "lan/border.rs"
source = path.read_text()
path.write_text(source.split("\n/// The border as the players were last told of it.", 1)[0]
                .replace("::play::{self, Clientbound as C}", "::play"))
