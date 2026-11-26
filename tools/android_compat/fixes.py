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
    # Sunrise's `insert` / `extract` say when a handler answered more than asked; Horizon's moved nothing then.
    ("u32", "Result<u32, Overanswered>"): lambda e: f"{_paren(e)}.unwrap_or(0)",
    ("u32", "Result<u32, mod_api::Overanswered>"): lambda e: f"{_paren(e)}.unwrap_or(0)",
    ("[i32; 3]", "&[i32; 3]"): lambda e: f"*{_paren(e)}",
    ("&[i32; 3]", "[i32; 3]"): lambda e: f"&{_paren(e)}",
}

# Fields Horizon's types had that Sunrise's compute (starling_compat.rs): (type, field) -> the call that reads it.
FIELD_CALLS = {
    ("RespawnData", "pos"): "starling_respawn_pos()",
}

# Names starling_compat.rs gives every file of an adopted mod: an import of them from `mod_api` that no longer
# resolves is dropped, and the prelude's glob supplies them.
PRELUDE = {"move_resources", "MovementInputEvent"}

# Methods Sunrise renamed whose old arity tells them apart: name -> (arguments of the old call, new name).
RENAMED_BY_ARITY = {"insert": (5, "insert_at"), "extract": (5, "extract_at")}

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


def _shorthand(source: bytes, start: int, end: int) -> bool:
    """Whether the identifier at `start..end` is a struct literal's shorthand field (`Path { a, id }`): it sits
    between `{` or `,` and `,` or `}`, and the brace that opens its group follows a type's name."""
    before = source[:start].rstrip()
    after = source[end:].lstrip()
    if not (before.endswith((b"{", b",")) and after.startswith((b",", b"}"))):
        return False
    depth = 0
    for i in range(start - 1, -1, -1):
        c = source[i:i + 1]
        if c in (b")", b"]", b"}"):
            depth += 1
        elif c in (b"(", b"["):
            if depth == 0:
                return False
            depth -= 1
        elif c == b"{":
            if depth == 0:
                head = source[:i].rstrip()
                return bool(re.search(rb"[A-Z]\w*(?:::)?(?:<[^<>]*>)?$", head))
            depth -= 1
    return False


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
                new = rule(expr)
                if re.fullmatch(r"[A-Za-z_]\w*", expr) and _shorthand(source, start, end):
                    # `Foo { id }`: the field keeps its name, the value is converted.
                    new = f"{expr}: {new}"
                edits.append((path, start, end, new))
        elif code in ("E0061", "E0050"):
            # Horizon's per-index `insert(host, tx, index, resource, amount)` is Sunrise's `insert_at` (its
            # `insert` is the old `insert_any`): a call with the old arity, or a handler's method with the old
            # parameters, takes the new name.
            if code == "E0061":
                m = re.search(r"takes (\d+) arguments? but (\d+) arguments? (?:were|was) supplied", diag["message"])
                name, given, at = expr, int(m.group(2)) if m else -1, (start, end)
            else:
                m = re.search(r"method `(\w+)` has (\d+) parameters?", diag["message"])
                name, given = (m.group(1), int(m.group(2)) - 1) if m else ("", -1)
                fn = source.rfind(f"fn {name}".encode(), 0, start + 1)
                at = (fn + 3, fn + 3 + len(name)) if fn >= 0 else None
            if name in RENAMED_BY_ARITY and at and given == RENAMED_BY_ARITY[name][0]:
                edits.append((path, *at, RENAMED_BY_ARITY[name][1]))
        elif code == "E0609":
            m = re.search(r"no field `(\w+)` on type `&?(?:mut )?(?:[\w:]+::)?(\w+)", diag["message"])
            call = FIELD_CALLS.get((m.group(2), m.group(1))) if m else None
            if call and expr == m.group(1):
                edits.append((path, start, end, call))
        elif code == "E0432":
            m = re.search(r"unresolved import `(?:mod_api::)?(\w+)`", diag["message"])
            if m and m.group(1) in PRELUDE:
                if expr.startswith("mod_api::"):
                    # `use mod_api::name;`: the prelude's copy, by its own path.
                    edits.append((path, start, end, f"crate::starling_compat::{m.group(1)}"))
                elif expr == m.group(1):
                    # One name of a `use mod_api::{..}` list: out, with the comma after it (or before it).
                    after = len(source[end:]) - len(source[end:].lstrip())
                    if source[end + after:end + after + 1] == b",":
                        edits.append((path, start, end + after + 1, ""))
                    else:
                        before = source[:start].rstrip()
                        if before.endswith(b","):
                            edits.append((path, len(before) - 1, end, ""))
                        else:
                            edits.append((path, start, end, "self as _"))
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
