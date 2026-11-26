#!/usr/bin/env python3
"""Rename OtherEntity's display field in its constructors and consumers."""

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
for path in (ROOT / "crates/sunrise-client/src/game").rglob("*.rs"):
    before = path.read_text()
    after = re.sub(r"\bitem_transformation\b", "display", before)
    if after != before:
        path.write_text(after)
