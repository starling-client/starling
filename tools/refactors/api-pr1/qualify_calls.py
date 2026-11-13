#!/usr/bin/env python3
"""Rewrites `Host::method(..)`, `ServerHost::method(..)`, `ClientHost::method(..)` (and the
backend unions) to the area trait that now declares `method`, for the files given.
The host methods moved out of the union traits, which no longer declare them."""
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from split_doubles import load_methods

table = load_methods()
pattern = re.compile(r"(?<![\w:])((?:mod_api::|crate::)?)(Host|ServerHost|ClientHost|Backend|BackendMut|ClientBackendMut)::(\w+)\(")
for name in sys.argv[1:]:
    path = Path(name)
    text = path.read_text()

    def fix(m):
        entry = table.get((m.group(2), m.group(3)))
        if not entry:
            return m.group(0)
        return f"{m.group(1)}{entry[1]}::{m.group(3)}(" if not m.group(1) else f"mod_api::prelude::{entry[1]}::{m.group(3)}("

    new = pattern.sub(fix, text)
    if new != text:
        path.write_text(new)
        print("qualified", name)
