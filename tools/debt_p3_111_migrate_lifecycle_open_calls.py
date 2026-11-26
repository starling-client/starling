#!/usr/bin/env python3
"""Migrate epoch fixtures to explicit optional server authority for debt p3-111.

Run on the part-2 base before updating NativeLifecycle's implementation.
"""

from pathlib import Path
import re
import subprocess


root = Path(__file__).resolve().parents[1]
paths = subprocess.check_output(
    ["git", "ls-files", "-z", "crates/sunrise-client/src/game"], cwd=root
).decode().split("\0")
server = re.compile(
    r"(\.begin_open\()((?:mod_api::)?Authority::(?:IntegratedServer|DedicatedServer))\)"
)
client = re.compile(r"(\.begin_open\()(?:mod_api::)?Authority::RemoteClient\)")
private_path = re.compile(r"(?:super::)+native_lifecycle::Authority\b")

for path in paths:
    if not path.endswith(".rs") or path.endswith("/native_lifecycle.rs"):
        continue
    source = root / path
    old = source.read_text()
    new = private_path.sub("mod_api::Authority", old)
    new = server.sub(r"\1Some(\2))", new)
    new = client.sub(r"\1None)", new)
    if source.name == "native_block_sprites.rs":
        new = new.replace("Authority, BiomeColorsEvent,", "BiomeColorsEvent,")
    if new != old:
        source.write_text(new)
