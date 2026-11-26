#!/usr/bin/env python3
"""Replace live upload-and-draw calls at the six standard world stages."""
import re
from pathlib import Path

path = Path('crates/sunrise-client-render/src/gfx/draw_frame.rs')
source = path.read_text()
source, changes = re.subn(
    r'(oit_on,\n                    \);\n                    self\.world_primitives\n)'
    r'(\s+)\.as_mut\(\)(\n\s+\.expect\("world primitives"\)\n\s+)'
    r'\.upload_and_draw_stage\(&self\.device, &self\.queue, &mut pass, &data\)',
    r'\1\2.as_ref()\3.draw_prepared(&mut pass, None, data)',
    source,
)
assert changes == 6, changes
path.write_text(source)
