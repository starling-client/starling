#!/usr/bin/env python3
"""Give each guest block turn its own phase state and thread a receipt to its server handler."""
from pathlib import Path
import re
import subprocess


root = Path(__file__).resolve().parents[1]
for name in subprocess.check_output(
    ["git", "ls-files", "crates/sunrise-client/src"], cwd=root, text=True
).splitlines():
    if not name.endswith(".rs"):
        continue
    path = root / name
    source = path.read_text()
    text = re.sub(r"GuestUse::OnBlock\(_\)", "GuestUse::OnBlock { .. }", source)
    text = text.replace("if let Some(GuestUse::OnBlock(hand))", "if let Some(GuestUse::OnBlock { hand, .. })")
    text = text.replace("GuestUse::OnBlock(hand) |", "GuestUse::OnBlock { hand, .. } |")
    text = re.sub(r"GuestUse::OnBlock\(([^()]+)\)", r"GuestUse::on_block(\1)", text)
    if path.name == "game.rs" and "enum GuestUse" in text:
        text = text.replace("    OnBlock(Hand),", "    OnBlock { hand: Hand },")
        text = text.replace(
            "/// The entity an interact packet names",
            "impl GuestUse {\n"
            "    fn on_block(hand: Hand) -> Self {\n"
            "        Self::OnBlock { hand }\n"
            "    }\n"
            "}\n"
            "/// The entity an interact packet names",
            1,
        )
    text = re.sub(r"(\.guest_use_item_on\([^()\n]+)\)", r"\1, None)", text)
    if path.name == "mod.rs" and ".guest_use_item_on(\n" in text:
        text, count = re.subn(
            r"(\.guest_use_item_on\(\n.*?\n)( +)(super::Hand::Main)\n",
            r"\1\2\3,\n\2None,\n", text, count=1, flags=re.S,
        )
        assert count == 1
    if path.name == "interaction.rs" and "fn guest_use_item_on(" in text:
        text = text.replace(
            "        hand: Hand,\n    ) -> Result<bool, ActionRefused> {\n        let mut consumed",
            "        hand: Hand,\n        _receipt: Option<i64>,\n    ) -> Result<bool, ActionRefused> {\n        let mut consumed",
            1,
        )
    if text != source:
        path.write_text(text)
