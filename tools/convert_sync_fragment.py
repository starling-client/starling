#!/usr/bin/env python3
"""Convert one retired fragment with convert(path, out).

The sync tool inserts the module line and removes the JSON. Run rustfmt
--edition 2024 on the generated Rust files before committing the sync.
"""
import json, re
from pathlib import Path

DEFAULT_TARGET = "horizon_client::game::native_hooks::Game<'_>::native_"

def doc_lines(doc):
    out = []
    for line in doc.split("\n"):
        out.append("///" + (" " + line if line else ""))
    return out


def decode(lines):
    out = []
    for l in lines:
        t = l[3:]
        out.append(t[1:] if t.startswith(" ") else t)
    return "\n".join(out)


def lit(value):
    return json.dumps(value, ensure_ascii=False)


def names(value):
    if isinstance(value, list):
        return "[" + ", ".join(lit(v) for v in value) + "]"
    return lit(value)


def hook_attr(h):
    api = h["mod_api"]
    parts = [f'side = {lit(h["side"])}', f'contract = {lit(api["contract"])}']
    if "out" in api:
        parts.append(f'out = {lit(api["out"])}')
    if "scope" in api:
        parts.append(f'scope = {lit(api["scope"])}')
        parts.append(f'key = {lit(api["key"])}')
    if "answered_by" in api:
        parts.append(f'answered_by = {lit(api["answered_by"])}')
    if "host" in h:
        parts.append(f'host = {lit(h["host"])}')
    if "context" in h:
        parts.append(f'context = {lit(h["context"])}')
    for key in ("neoforge", "fabric", "paper"):
        if key in h:
            parts.append(f"{key} = {names(h[key])}")
    if "cancellable" in h:
        parts.append(f'cancellable = {"true" if h["cancellable"] else "false"}')
    if h.get("version", 1) != 1:
        parts.append(f'version = {h["version"]}')
    if h["target"] != DEFAULT_TARGET + h["name"]:
        parts.append(f'target = {lit(h["target"])}')
    return "#[hook(" + ", ".join(parts) + ")]"


HEADER = "//! {title}: declarations the mod API generator reads (see `hooks/mod.rs`).\n\n"
def convert(path, out):
    src = json.loads(path.read_text())
    unknown = set(src) - {"hooks", "draw_contexts"}
    if unknown or src.get("draw_contexts"):
        raise ValueError(
            f"unsupported fragment fields: {sorted(unknown)} or draw contexts; "
            "helpers are host methods now, written by hand in crates/horizon-mod-vocab/src/host/<area>.rs"
        )
    stem = path.stem.replace("-", "_")
    if not re.fullmatch(r"[a-z_][a-z0-9_]*", stem):
        raise ValueError(f"invalid Rust module name: {stem}")
    destination = out / f"{stem}.rs"
    if destination.exists():
        raise ValueError(f"destination already exists: {destination}")
    text = HEADER.format(title=f"The `{stem}` hooks")
    if src.get("hooks"):
        text += "pub trait Hooks {\n"
        for h in src["hooks"]:
            unknown = set(h) - {"name", "doc", "side", "target", "mod_api", "host", "context", "neoforge", "fabric", "paper", "cancellable", "version"}
            unknown_api = set(h["mod_api"]) - {"contract", "out", "scope", "key", "answered_by"}
            if unknown or unknown_api:
                raise ValueError(f"{h['name']}: unsupported fields {sorted(unknown | unknown_api)}")
            lines = doc_lines(h["doc"])
            if decode(lines) != h["doc"]:
                raise ValueError(f"{h['name']}: documentation cannot round-trip")
            text += "".join("    " + line + "\n" for line in lines)
            text += "    " + hook_attr(h) + "\n    fn " + h["name"] + "();\n"
        text += "}\n"
    destination.write_text(text)
    return stem
