#!/usr/bin/env python3
"""Remove the producer-side observers left behind by the chunk disk worker move."""

import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CLIENT = ROOT / "crates/sunrise-client/src"
STREAMING = ROOT / "crates/sunrise-client-render/src/streaming.rs"
OBSERVERS = (
    "native_full_chunk_snapshots",
    "native_late_full_chunk_snapshot",
    "native_proto_snapshots",
)


def remove_ranges(path, ranges):
    source = path.read_text()
    for first, after_last in ranges:
        start = source.index(first)
        end = source.index(after_last, start + len(first))
        source = source[:start] + source[end:]
    return source


def main():
    edits = {
        CLIENT / "game/modding/native_chunk_save.rs": remove_ranges(
            CLIENT / "game/modding/native_chunk_save.rs",
            [
                ("use sunrise_client::chunk::Chunk;", "\nimpl Game<'_> {"),
                ("    /// Observe the exact full-chunk", "    /// Chunks whose subscribed save"),
            ],
        ),
        STREAMING: remove_ranges(
            STREAMING,
            [
                ("/// Borrowed, read-only generation state", "/// Chunk generation in vanilla's stages"),
                ("    /// Generation status of a proto", "    /// What to save of a proto"),
                ("/// Read-only access to checked-out generation state.", "trait Saves:"),
                ("    /// A read-only view of checked-out protos", "    /// Check out proto chunks"),
                ("        struct Access;", "        let mut protos = FxHashMap::default();"),
                ("        let snapshot = ProtoSnapshot {", "        let Job::Save(saved)"),
            ],
        ).replace("trait Saves: LodSource + ProtoAccess {", "trait Saves: LodSource {", 1),
        CLIENT / "save.rs": remove_ranges(
            CLIENT / "save.rs", [("    fn proto_status(", "    fn light_view(")]
        ),
        CLIENT / "vanillagen.rs": remove_ranges(
            CLIENT / "vanillagen.rs", [("    fn proto_status(", "    fn light_view(")]
        ),
    }
    calls = re.compile(rf"^ +self\.(?:{'|'.join(OBSERVERS)})\(&\w+\);\n", re.MULTILINE)
    for name, expected in [("saving.rs", 6), ("world_update.rs", 3)]:
        path = CLIENT / "game/world" / name
        new, count = calls.subn("", path.read_text())
        if count != expected:
            raise ValueError(f"{path}: expected {expected} observer calls, found {count}")
        edits[path] = new

    for path, source in edits.items():
        path.write_text(source)


if __name__ == "__main__":
    main()
