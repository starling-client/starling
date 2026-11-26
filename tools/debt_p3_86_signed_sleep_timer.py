#!/usr/bin/env python3
"""Migrate Sunrise's sleep counter storage to vanilla's signed int."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "crates"
CLIENT = ROOT / "sunrise-client/src"
VOCAB = ROOT / "sunrise-mod-vocab/src"
WRITES = {
    CLIENT / "game.rs": [("sleep_timer: u32,", "sleep_timer: i32,"),
                ("self.sleep_timer as i32", "self.sleep_timer"),
                ("const MAX_SLEEP_TIMER: u32", "const MAX_SLEEP_TIMER: i32"),
                ("fn tick_sleep_timer(timer: u32, sleeping: bool) -> u32",
                 "fn tick_sleep_timer(timer: i32, sleeping: bool) -> i32")],
    CLIENT / "game/modding/native_living_flags.rs": [("timer: u32,", "timer: i32,")],
    CLIENT / "game/modding/native_server.rs": [
        ("fn player_sleep_timer(&self, player: PlayerRef) -> Option<u32>",
         "fn player_sleep_timer(&self, player: PlayerRef) -> Option<i32>"),
        ("fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: u32)",
         "fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: i32)"),
    ],
    CLIENT / "game/network/lan_actions/shared.rs": [("sleep_timer: u32,", "sleep_timer: i32,")],
    CLIENT / "game/network/lan_actions/sleep.rs": [("timer: u32|", "timer: i32|")],
    CLIENT / "lan/guest.rs": [("pub sleep_timer: u32,", "pub sleep_timer: i32,")],
    CLIENT / "mobs.rs": [("pub sleep_timer: u32,", "pub sleep_timer: i32,")],
    VOCAB / "host/player_sleep.rs": [
        ("fn player_sleep_timer(&self, player: PlayerRef) -> Option<u32>",
         "fn player_sleep_timer(&self, player: PlayerRef) -> Option<i32>"),
        ("fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: u32)",
         "fn player_set_sleep_timer(&mut self, player: PlayerRef, ticks: i32)"),
    ],
    VOCAB / "hooks_tests.rs": [
        ("let _: Option<u32> = player.sleep_timer(read);",
         "let _: Option<i32> = player.sleep_timer(read);"),
    ],
}

outputs = {}
for path, changes in WRITES.items():
    source = path.read_text()
    for old, new in changes:
        if source.count(old) != 1:
            raise SystemExit(f"{path}: expected exactly one {old!r}")
        source = source.replace(old, new)
    outputs[path] = source
for path, source in outputs.items():
    path.write_text(source)
