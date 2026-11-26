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
    """(expected, found) from a label (`expected `A`, found `B``) or a note (`expected tuple `a::A`` / `found
    struct `b::B``), with paths dropped (`std::string::String` is `String`)."""
    m = re.search(r"expected (?:[\w ]+? )?`([^`]+)`,?\s*found (?:[\w ]+? )?`([^`]+)`", message)
    if not m:
        return None
    def bare(t):
        return re.sub(r"\b(?:[a-z_][a-z0-9_]*::)+(?=[A-Za-z_])", "", t)
    return bare(m.group(1)), bare(m.group(2))


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
    # Tooltip lines are a text with its colour now (`u32::MAX` keeps the text's own); Horizon's were strings.
    ("(Text, u32)", "String"): lambda e: f"(mod_api::Text::literal({e}), u32::MAX)",
    ("(Text, u32)", "&str"): lambda e: f"(mod_api::Text::literal({e}), u32::MAX)",
    ("(Text, u32)", "&String"): lambda e: f"(mod_api::Text::literal({_paren(_strip_ref(e))}.clone()), u32::MAX)",
    ("&[String]", "&EntityTags<'_>"):
        lambda e: f"&{_paren(_strip_ref(e))}.iter_utf16().map(String::from_utf16_lossy).collect::<Vec<String>>()",
    # A text read made owned (starling_owned) where a borrowed one is read.
    ("Option<&str>", "Option<String>"): lambda e: f"{_paren(e)}.as_deref()",
    ("&str", "Option<String>"): lambda e: f"{_paren(e)}.as_deref().unwrap_or_default()",
    ("Option<&str>", "&Option<String>"): lambda e: f"{_paren(_strip_ref(e))}.as_deref()",
    ("Option<bool>", "bool"): lambda e: f"Some({e})",
    ("bool", "Option<bool>"): lambda e: f"{_paren(e)}.unwrap_or(false)",
    ("Option<&ItemResource>", "&ItemResource"): lambda e: f"Some({e})",
    ("&ItemStack", "&(usize, ItemStack)"): lambda e: f"&{_paren(_strip_ref(e))}.1",
    ("&ItemStack", "&&(usize, ItemStack)"): lambda e: f"&{_paren(_strip_ref(e))}.1",
    ("Result<i32, _>", "i32"): lambda e: f"Ok({e})",
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

# Shapes by pattern, after the exact pairs: (expected, found) regexes -> rule.
SHAPE_RULES = [
    # A write Horizon answered with whether it took, Sunrise with why it didn't.
    ((r"bool", r"Result<\(\), [\w:]+>"), lambda e: f"{_paren(e)}.is_ok()"),
    # A value Horizon handed plain that Sunrise may refuse: the old value or the type's default.
    ((r"String|i32|u32|i64|u64|f32|f64|usize", r"Result<_, _>|Result<\w+, [\w:]+>"),
     lambda e: f"{_paren(e)}.unwrap_or_default()"),
]

# Reads whose text Sunrise lends from the host (Horizon's handed a String): made owned where the host is wanted
# mutably while the text is alive (E0502).
TEXT_READS = {b"id", b"name", b"display_name", b"dimension", b"key", b"custom_name", b"translation_key"}

# Methods Sunrise renamed whose old arity tells them apart: name -> (arguments of the old call, new name).
RENAMED_BY_ARITY = {"insert": (5, "insert_at"), "extract": (5, "extract_at")}

# A method the old type had and Sunrise's value doesn't need: `x.unwrap_or(d)` on what is no longer an Option.
GONE_OPTION_METHODS = {"unwrap_or", "unwrap_or_default", "unwrap_or_else", "unwrap", "as_deref", "copied", "cloned"}
PLAIN = re.compile(r"(?:i8|i16|i32|i64|u8|u16|u32|u64|f32|f64|bool|usize|isize|String|&str|\[i32; 3\])")


def _span(diag: dict, folder: Path | None = None, root: Path | None = None) -> dict | None:
    """The primary span, or, when it is inside a macro's expansion (`format!` in std), the call that expanded it
    in the mod's own file."""
    span = next((s for s in diag.get("spans", []) if s.get("is_primary")), None)
    while span and folder and not _inside(span, folder, root) and span.get("expansion"):
        span = span["expansion"].get("span")
    return span


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
        span = _span(diag, folder, root)
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
            # The label first (rustc's short display, `&str`), then the message, then a note (`&'static str`,
            # lifetimes and paths spelled out), for a span that followed a macro to its call.
            label = span.get("label") or next((s.get("label") or "" for s in diag["spans"] if s.get("is_primary")), "")
            pair = _types(label) or _types(diag["message"]) or next(
                (_types(c["message"]) for c in diag.get("children", []) if _types(c["message"])), None)
            if pair:
                pair = tuple(re.sub(r"&'static ", "&", t) for t in pair)
            rule = TYPE_RULES.get(pair) if pair else None
            if not rule and pair:
                rule = next((r for (want, got), r in SHAPE_RULES if re.fullmatch(want, pair[0])
                             and re.fullmatch(got, pair[1])), None)
            if not rule and pair == ("Option<RegistryName>", "&str"):
                # `kind.id(keys) != name`: the id is compared as text, as Horizon's was.
                other = next((s for s in diag["spans"] if not s.get("is_primary")
                              and "expected because" in (s.get("label") or "")), None)
                if other and _inside(other, folder, root) == path:
                    lhs = source[other["byte_start"]:other["byte_end"]].decode("utf-8", "replace")
                    edits.append((path, other["byte_start"], other["byte_end"], f"{_paren(lhs)}.as_deref()"))
                    edits.append((path, start, end, f"Some({expr})"))
                continue
            if rule:
                new = rule(expr)
                if re.fullmatch(r"[A-Za-z_]\w*", expr) and _shorthand(source, start, end):
                    # `Foo { id }`: the field keeps its name, the value is converted.
                    new = f"{expr}: {new}"
                edits.append((path, start, end, new))
        elif code == "E0658" and "str_as_str" in diag["message"] and expr == "as_str":
            # `.as_str()` on what is a `&str` now (Horizon handed a String): the value itself.
            dot = source.rfind(b".", 0, start)
            stop = _close(source, end) if source[end:end + 1] == b"(" else -1
            if dot >= 0 and stop > 0 and not source[dot + 1:start].strip():
                edits.append((path, dot, stop, ""))
        elif code == "E0631" and re.fullmatch(r"[\w:]+", expr):
            # `.map(str::to_owned)` over Sunrise's `RegistryName` (which derefs to `str`): the function on its text.
            note = " ".join(c["message"] for c in diag.get("children", [])) + diag["message"]
            if re.search(r"expected function signature `fn\((?:[\w:]+::)?RegistryName\) -> _`", note) \
                    and "found function signature `fn(&str) -> _`" in note:
                edits.append((path, start, end, f"|name| {expr}(&name)"))
        elif code == "E0061" and "expected `Text`, found `(Text, u32)`" in json.dumps(diag):
            # A tooltip's `lines.push((text, colour))`: Sunrise's takes the two as arguments.
            arg = next((s for c in diag.get("children", []) for s in c.get("spans", [])
                        if _inside(s, folder, root) == path and source[s["byte_start"]:s["byte_start"] + 1] == b"("), None)
            if arg:
                inner = source[arg["byte_start"] + 1:arg["byte_end"] - 1].decode("utf-8", "replace")
                edits.append((path, arg["byte_start"], arg["byte_end"], inner))
        elif code == "E0502":
            # `let name = player.name(host).unwrap_or_default();` borrowed the host where Horizon's gave a String,
            # and the host is wanted mutably later: the value is kept as its own.
            borrowed = next((s for s in diag["spans"] if "immutable borrow occurs" in (s.get("label") or "")), None)
            if borrowed and _inside(borrowed, folder, root) == path:
                # The host lent to a text read (`x.id(host)`, `player.name(host)`): Sunrise's text borrows the
                # host where Horizon's was owned, so the read is made owned right there (starling_owned).
                open_at = source.rfind(b"(", 0, borrowed["byte_start"])
                call = re.search(rb"\.(\w+)\s*$", source[:open_at]) if open_at >= 0 else None
                if call and call.group(1) in TEXT_READS and not source[open_at + 1:borrowed["byte_start"]].strip(b" &*"):
                    close = _close(source, open_at)
                    if close > 0 and not source[close:].startswith(b".starling_owned()"):
                        edits.append((path, close, close, ".starling_owned()"))
                        continue
                line_end = source.find(b";", borrowed["byte_end"])
                stmt_start = source.rfind(b"let ", 0, borrowed["byte_start"])
                newline = source.rfind(b"\n", 0, borrowed["byte_start"])
                if line_end > 0 and stmt_start >= 0 and stmt_start > newline:
                    init = source[stmt_start:line_end]
                    if re.search(rb"\.unwrap_or(?:_default\(\)|\(\"\"\))\s*$", init):
                        edits.append((path, line_end, line_end, ".to_owned()"))
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
