#!/usr/bin/env python3
"""Mechanically migrate the native criterion test signatures and simple callers."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

for name in (
    "crates/sunrise-mod-vocab/src/world/criteria.rs",
    "crates/sunrise-mod-vocab/src/host/player_progress.rs",
    "crates/sunrise-client/src/game/modding/native_criteria.rs",
    "crates/sunrise-client/src/game/modding/native_guest_turn_tests.rs",
    "crates/sunrise-client/src/game/modding/native_block_use_criteria_tests.rs",
    "crates/sunrise-client/src/game/network/lan_actions/guest_body/tests.rs",
):
    path = ROOT / name
    text = path.read_text()
    text = text.replace("test: Arc<dyn CriterionTest>", "test: &mut dyn CriterionTest")
    text = text.replace("test: &dyn CriterionTest", "test: &mut dyn CriterionTest")
    text = text.replace("fn test(&self,", "fn test(&mut self,")
    text = text.replace("fn test(\n                &self,", "fn test(\n                &mut self,")
    text = text.replace("test.as_ref()", "test")
    for constructor in ("Never", "Accept", "Granted"):
        text = text.replace(f"Arc::new({constructor})", f"&mut {constructor}")
    if name.endswith("/world/criteria.rs"):
        text = text.replace("    use std::sync::Arc;\n", "")
    if name.endswith("/modding/native_criteria.rs"):
        text = text.replace("use std::{cell::Cell, sync::Arc};", "use std::cell::Cell;")
        text = text.replace("&pipe(&", "&mut pipe(&")
        text = text.replace("&any)", "&mut any)")
        text = text.replace("let any = ItemTest", "let mut any = ItemTest")
        text = text.replace("&Unavailable)", "&mut Unavailable)")
    path.write_text(text)
