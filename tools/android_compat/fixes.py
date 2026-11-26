"""Compiler-guided fixes for mods made for Starling 0.9.0's (Horizon's) mod API, on Sunrise's.

`horizon_compat` and autoport give a Horizon mod the names and hooks Sunrise dropped. What they can't reach is a
method of Sunrise's own types whose value changed shape: `block.id(host)` was the id and is `Option<&str>` now, a
position that was lent by reference is a value. The compiler says exactly where each one is, with the type it
expected and the one it found, so the fix is chosen by that pair, never by the mod:

    edits = fixes(diagnostics, mod_folder)    # rustc's JSON messages of one `cargo check` of the mod
    apply(edits)                              # then check again, until nothing changes

Only spans inside the mod's own folder are edited, and only by rules that keep the old meaning: a missing id reads
as empty (what Horizon returned for an unregistered key), a value is used where a reference was, and rustc's own
suggestions where rustc marks them MachineApplicable.
"""
import json
import re
from pathlib import Path


def _types(message: str) -> tuple[str, str] | None:
    m = re.search(r"expected `([^`]+)`, found `([^`]+)`", message)
    if not m:
        m = re.search(r"expected `([^`]+)`\s*found `([^`]+)`", message)
    return (m.group(1), m.group(2)) if m else None


def _strip_ref(text: str) -> str:
    return text[1:].lstrip() if text.startswith("&") and not text.startswith("&&") else text


def _paren(text: str) -> str:
    """`text` as a receiver: wrapped unless it is already a path, call, field or index chain."""
    return text if re.fullmatch(r"[\w.:()\[\]&<>' ]*?[\w)\]]", text) and " " not in text.strip() else f"({text})"


# (expected, found) -> how the found expression becomes the expected one. `{e}` is the expression's text.
TYPE_RULES = {
    ("&str", "&Option<&str>"): lambda e: f"{_paren(_strip_ref(e))}.unwrap_or_default()",
    ("&str", "Option<&str>"): lambda e: f"{_paren(e)}.unwrap_or_default()",
    ("&str", "&Option<String>"): lambda e: f"{_paren(_strip_ref(e))}.as_deref().unwrap_or_default()",
    ("String", "Option<&str>"): lambda e: f"{_paren(e)}.unwrap_or_default().to_string()",
    ("String", "&Option<&str>"): lambda e: f"{_paren(_strip_ref(e))}.unwrap_or_default().to_string()",
    ("String", "Option<String>"): lambda e: f"{_paren(e)}.unwrap_or_default()",
    ("String", "&str"): lambda e: f"{_paren(e)}.to_string()",
    ("&str", "String"): lambda e: f"&{_paren(e)}",
    ("Option<&str>", "&str"): lambda e: f"Some({e})",
    ("Option<&str>", "&String"): lambda e: f"Some({_strip_ref(e)}.as_str())",
    ("Option<&str>", "String"): lambda e: f"Some({_paren(e)}.as_str())",
    ("Option<String>", "Option<&str>"): lambda e: f"{_paren(e)}.map(str::to_string)",
    ("Option<String>", "Option<Option<&str>>"): lambda e: f"{_paren(e)}.flatten().map(str::to_string)",
    ("Option<&str>", "Option<Option<&str>>"): lambda e: f"{_paren(e)}.flatten()",
    ("[i32; 3]", "Option<[i32; 3]>"): lambda e: f"{_paren(e)}.unwrap_or_default()",
    ("[i32; 3]", "&[i32; 3]"): lambda e: f"*{_paren(e)}",
    ("&[i32; 3]", "[i32; 3]"): lambda e: f"&{_paren(e)}",
}

# A method the old type had and Sunrise's value doesn't need: `x.unwrap_or(d)` on what is no longer an Option.
GONE_OPTION_METHODS = {"unwrap_or", "unwrap_or_default", "unwrap_or_else", "unwrap", "as_deref", "copied", "cloned"}
PLAIN = re.compile(r"(?:i8|i16|i32|i64|u8|u16|u32|u64|f32|f64|bool|usize|isize|String|&str|\[i32; 3\])")


def _span(diag: dict) -> dict | None:
    return next((s for s in diag.get("spans", []) if s.get("is_primary")), None)


def _inside(span: dict, folder: Path, root: Path) -> Path | None:
    path = (root / span["file_name"]).resolve()
    return path if folder.resolve() in path.parents else None


def _close(text: bytes, open_at: int) -> int:
    depth = 0
    for i in range(open_at, len(text)):
        c = text[i:i + 1]
        if c in (b"(", b"[", b"{"):
            depth += 1
        elif c in (b")", b"]", b"}"):
            depth -= 1
            if depth == 0:
                return i + 1
    return -1


def fixes(diagnostics: list[dict], folder: Path, root: Path) -> list[tuple[Path, int, int, str]]:
    """The edits (file, byte start, byte end, new text) for `diagnostics` inside `folder`."""
    edits = []
    texts = {}

    def text_of(path: Path) -> bytes:
        if path not in texts:
            texts[path] = path.read_bytes()
        return texts[path]

    for diag in diagnostics:
        code = (diag.get("code") or {}).get("code", "")
        span = _span(diag)
        if not span:
            continue
        path = _inside(span, folder, root)
        if not path:
            continue
        start, end = span["byte_start"], span["byte_end"]
        source = text_of(path)
        expr = source[start:end].decode("utf-8", "replace")
        # rustc's own fix, where it is sure of it.
        sure = [s for child in diag.get("children", []) for s in child.get("spans", [])
                if s.get("suggestion_applicability") == "MachineApplicable" and s.get("suggested_replacement") is not None
                and _inside(s, folder, root)]
        if sure:
            for s in sure:
                edits.append((_inside(s, folder, root), s["byte_start"], s["byte_end"], s["suggested_replacement"]))
            continue
        if code == "E0308":
            pair = _types(diag["message"]) or next(
                (_types(c["message"]) for c in diag.get("children", []) if _types(c["message"])), None)
            label = span.get("label") or ""
            pair = pair or _types(label)
            rule = TYPE_RULES.get(pair) if pair else None
            if rule:
                edits.append((path, start, end, rule(expr)))
        elif code == "E0614" and expr.startswith("*"):
            # `*pos` where `pos` is a value now.
            edits.append((path, start, end, expr[1:]))
        elif code == "E0599":
            m = re.search(r"no method named `(\w+)` found for (?:type|reference|mutable reference|struct|enum) `([^`]+)`",
                          diag["message"])
            if m and m.group(1) in GONE_OPTION_METHODS and PLAIN.fullmatch(m.group(2).lstrip("&")):
                # The span is the method's name: drop `.name(...)` (its arguments were the old default).
                dot = source.rfind(b".", 0, start)
                after = source[end:end + 1]
                stop = _close(source, end) if after == b"(" else end
                if dot >= 0 and stop > 0 and not source[dot + 1:start].strip():
                    edits.append((path, dot, stop, ""))
    return edits


def apply(edits: list[tuple[Path, int, int, str]]) -> int:
    """Writes the edits, latest first per file, skipping any that overlaps another; how many were made."""
    made = 0
    by_file = {}
    for path, start, end, new in edits:
        by_file.setdefault(path, []).append((start, end, new))
    for path, items in by_file.items():
        data = path.read_bytes()
        last = len(data) + 1
        for start, end, new in sorted(set(items), key=lambda e: (-e[0], -e[1])):
            if end > last:
                continue
            data = data[:start] + new.encode("utf-8") + data[end:]
            last = start
            made += 1
        path.write_bytes(data)
    return made


def from_cargo(stdout: str, package: str) -> list[dict]:
    """The error messages of `package` in `cargo ... --message-format=json` output."""
    out = []
    for line in stdout.splitlines():
        if not line.startswith("{"):
            continue
        msg = json.loads(line)
        # `path+file:///…#smod-x@0.1.0` (cargo 1.77 on) or `smod-x 0.1.0 (path+file:///…)`.
        if msg.get("reason") == "compiler-message" and msg["message"]["level"] == "error" \
                and re.search(rf"(?:^|[#/ ]){re.escape(package)}(?:[@# ]|$)", msg.get("package_id", "")):
            out.append(msg["message"])
    return out
