#!/usr/bin/env python3
"""Migrate resource-handler and transfer signatures to scoped transaction results (debt 2r)."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
paths = (
    "sunrise-mod-vocab/src/content/resource.rs",
    "sunrise-mod-vocab/src/content/resource_util.rs",
    "sunrise-mod-vocab/src/entity/entity_capability.rs",
    "sunrise-mod-vocab/src/item/capability.rs",
    "sunrise-mod-vocab/src/item/energy_capability.rs",
    "sunrise-mod-api/src/parts/fluids.rs",
    "sunrise-client/src/game/blocks/machines/storage_attempt_tests.rs",
)
# These low-level checks do not open a scope and still only return Overanswered.
keep = {
    "pub(crate) fn checked_transfer",
}
for path in paths:
    file = root / "crates" / path
    lines = file.read_text().splitlines(keepends=True)
    for index, line in enumerate(lines):
        if any(line.startswith(prefix) for prefix in keep):
            continue
        lines[index] = (line.replace(", Overanswered>", ", TransactionError<Overanswered>>")
                        if "Overanswered>" in line else line)
    file.write_text("".join(lines))
