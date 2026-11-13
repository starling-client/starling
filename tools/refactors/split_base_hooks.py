#!/usr/bin/env python3
"""Split the legacy catalog, preserving declaration text and stable catalog order.

Run from any directory. Rerunning after the split is a no-op. The per-file
catalog-order comment preserves existing HookId values without a shared registry;
new hooks need no ordinal and retain alphabetical discovery order.
"""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
DIRECTORY = ROOT / "crates/horizon-mod-api/src/hooks"


def main():
    source = DIRECTORY / "base.rs"
    if not source.exists():
        return
    text = source.read_text()
    draw_start = text.index("pub trait DrawContexts")
    hook_start = text.index("pub trait Hooks")
    hook_end = text.index("\n}\n", hook_start)
    body = text[hook_start + len("pub trait Hooks {\n"):hook_end + 1]
    outputs = {}
    start = 0
    for ordinal, match in enumerate(re.finditer(r"^    fn (\w+)\(\);\n", body, re.M), 1):
        name = match[1]
        outputs[f"{name}.rs"] = (
            f"//! The `{name}` hook: declarations the mod API generator reads (see `hooks/mod.rs`).\n"
            f"// catalog-order: {ordinal:04d}\n\npub trait Hooks {{\n"
            + body[start:match.end()] + "}\n"
        )
        start = match.end()
    assert not body[start:].strip(), "unparsed declaration"
    outputs["draw_contexts.rs"] = text[draw_start:hook_start].rstrip() + "\n"
    outputs["upstream_notes.rs"] = text[hook_end + 3:].lstrip()
    for name in outputs:
        assert not (DIRECTORY / name).exists(), f"refusing to overwrite {name}"
    for name, content in outputs.items():
        (DIRECTORY / name).write_text(content)
    source.unlink()
    print(f"Moved {len(outputs) - 2} hooks; created {len(outputs)} files.")


if __name__ == "__main__":
    main()
