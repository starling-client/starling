#!/usr/bin/env python3
"""Mechanically lend the world alongside the LAN host and items at P3-10's original callers."""
from pathlib import Path
import re

GAME = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src/game"
TURNS = GAME / "network/lan_actions/turns.rs"
SIGNATURE = "f: impl FnOnce(&mut LanHost, &sunrise_client::items::Items) -> Option<R>,"
if SIGNATURE in (source := TURNS.read_text()):
    TURNS.write_text(source.replace(
        SIGNATURE,
        "f: impl FnOnce(&mut LanHost, &sunrise_client::items::Items, "
        "&sunrise_client::world::World) -> Option<R>,",
    ).replace("f(self.lan.as_mut()?, world_items!(self))", "f(self.lan.as_mut()?, world_items!(self), &self.world)"))
    for relative in (
        "modding/native_menus.rs", "network/lan_actions/interaction.rs",
        "network/lan_actions/menus.rs", "network/lan_actions/packets.rs", "network/lan_crafting.rs",
    ):
        path = GAME / relative
        source = path.read_text()
        path.write_text(re.sub(r"(\.with_own_record\(gi, \|\w+, \w+)\|", r"\1, _|", source))
