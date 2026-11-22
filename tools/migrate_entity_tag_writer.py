#!/usr/bin/env python3
"""Migrate mob subclass writers to the shared entity save destination.

Run once from the repository root before applying the save-order changes. The
writers' bodies and ordinary nested Compound operations are left unchanged.
"""

import re
from pathlib import Path


WRITERS = {
    "bee": ("to_nbt",),
    "brain": ("write_frog",),
    "goat": ("write_nbt",),
    "llama": ("to_nbt",),
    "nautilus": ("to_nbt",),
    "nether": ("to_nbt",),
    "piglin": ("write_guarded_anger",),
    "raider": ("write_nbt",),
    "riders": ("write_nbt",),
    "sniffer": ("to_nbt",),
    "sulfur_cube": ("to_nbt",),
    "vex": ("write_nbt",),
    "villager": ("write_nbt",),
    "zombie_villager": ("put_cure", "write_nbt"),
}


def main():
    root = Path("crates/sunrise-client/src/mobs")
    for module, writers in WRITERS.items():
        path = root / f"{module}.rs"
        source = path.read_text()
        for name in writers:
            signature = rf"(^\s*(?:pub )?fn {name}\([^{{]*?map: &mut )(?:sunrise_core::nbt::)?Compound\b"
            source, count = re.subn(signature, r"\g<1>crate::entity_nbt::EntityTagWriter", source, flags=re.M)
            if count != 1:
                raise SystemExit(f"{path}: expected one Compound argument in {name}; found {count}")
        path.write_text(source)


if __name__ == "__main__":
    main()
