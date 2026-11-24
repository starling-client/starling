"""The mechanical part of porting a Horizon mod to Sunrise's API, on a mod's folder.

    python autoport.py MOD_DIR

What it does (each only where the old shape is found; the compiler shows what is left):
- a block entity tick `fn block_entity_tick(&self, host, event)` runs its old body, or the helper it calls with
  `(host, event)`, on a copy of the block entity (starling_tick.rs, added to the crate);
- `run_mod_command` goes: the function it called becomes the executor of every usage the mod declares;
- `ModContainer { .. }` gains `insert_rules`, `state_for_placement` / `analog_output` / config / client-setup hooks
  take their new host types, `event.types.push(x)` registers, `cx.refuse` is the registrar's;
- an item's id read (`.item().id(host)`) is a plain `&str` again (`unwrap_or_default`);
- the gone names leave the `use` lists.
"""
import pathlib
import re
import shutil
import sys

# Starling's Android store: `autoport.py MOD_DIR`, a Horizon mod's folder (tools/android_native_sdk.py); its shims
# (starling_tick.rs and the rest) are beside this file.
SHIMS = pathlib.Path(__file__).resolve().parent
work = pathlib.Path(sys.argv[1])
mod = work.name
src = work / "src"
files = {p: p.read_text(encoding="utf-8") for p in src.rglob("*.rs")}
notes = []


def body_end(text, start):
    """The index just past the `}` closing the block whose `{` is the first after `start`."""
    i = text.index("{", start)
    depth = 0
    in_str = False
    while i < len(text):
        c = text[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "/" and text.startswith("//", i):
            i = text.index("\n", i)
            continue
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    raise ValueError("unbalanced")


TICK_SIG = re.compile(
    r"fn block_entity_tick\(\s*&self,\s*(?P<host>_?host): &mut dyn (?:mod_api::)?ServerHost,\s*"
    r"(?P<ev>_?event): &mut (?:mod_api::)?BlockEntityTickEvent<'_>,?\s*\)\s*\{")
HELPER_CALL = re.compile(r"^\s*(?:let _ = )?(?P<path>[\w:]+)\(\s*host,\s*event(?P<rest>(?:,[^;]*?)?)\);?\s*$")
uses_tick = False
helpers = set()
for path, text in list(files.items()):
    pos = 0
    while (m := TICK_SIG.search(text, pos)):
        end = body_end(text, m.start())
        body = text[m.end():end - 1]
        call = HELPER_CALL.match(body.strip("\n")) if body.strip().count("\n") == 0 else None
        if m.group("ev").startswith("_"):
            new = "fn block_entity_tick(&self, _event: &mut BlockEntityTickEvent<'_>) {}"
        elif call:
            helper = call.group("path")
            rest = call.group("rest") or ""
            helpers.add(helper.split("::")[-1])
            new = ("fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {\n"
                   f"        crate::starling_tick::run(event, |host, entity, pos| {helper}(host, entity, pos{rest}));\n    }}")
            uses_tick = True
        else:
            inner = body.replace("&mut *event.entity", "entity")
            inner = re.sub(r"\bevent\s*\.\s*entity\b", "(*entity)", inner)
            inner = re.sub(r"\bevent\s*\.\s*pos\b", "tick_pos", inner)
            inner = re.sub(r"\*event\.reply = (?:mod_api::)?BlockEntityTickReply::\w+;", "// (unchanged: nothing is written back)", inner)
            new = ("fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {\n"
                   "        crate::starling_tick::run(event, |host, entity, tick_pos| {" + inner + "});\n    }")
            uses_tick = True
        text = text[:m.start()] + new + text[end:]
        pos = m.start() + len(new)
    files[path] = text

# The helpers a tick called with (host, event): they take the copy now.
for helper in helpers:
    sig = re.compile(rf"fn {helper}\(\s*host: &mut dyn (?:mod_api::)?ServerHost,\s*event: &mut (?:mod_api::)?BlockEntityTickEvent<'_>")
    for path, text in list(files.items()):
        m = sig.search(text)
        if not m:
            continue
        end = body_end(text, m.start())
        head = text[m.start():text.index("{", m.start())]
        head = sig.sub(f"fn {helper}(host: &mut dyn ServerHost, entity: &mut crate::starling_tick::OldEntity, tick_pos: [i32; 3]", head)
        inner = text[text.index("{", m.start()):end]
        inner = inner.replace("&mut *event.entity", "entity")
        inner = re.sub(r"\bevent\s*\.\s*entity\b", "(*entity)", inner)
        inner = re.sub(r"\bevent\s*\.\s*pos\b", "tick_pos", inner)
        inner = re.sub(r"\*event\.reply = (?:mod_api::)?BlockEntityTickReply::\w+;", "// (unchanged: nothing is written back)", inner)
        files[path] = text[:m.start()] + head + inner + text[end:]

# Every other function that took the tick's event (helpers of helpers) takes the copy too.
HELPER_SIG = re.compile(r"fn (?P<name>\w+)(?P<gen><[^>]*>)?\((?P<before>[^)]*?)\bevent: &mut (?:mod_api::)?BlockEntityTickEvent<'_>")
changed = True
while changed:
    changed = False
    for path, text in list(files.items()):
        m = next((x for x in HELPER_SIG.finditer(text) if x.group("name") != "block_entity_tick"), None)
        if not m:
            continue
        end = body_end(text, m.start())
        brace = text.index("{", m.end())
        head = text[m.start():brace]
        head = re.sub(r"\bevent: &mut (?:mod_api::)?BlockEntityTickEvent<'_>",
                      "entity: &mut crate::starling_tick::OldEntity, tick_pos: [i32; 3]", head)
        inner = text[brace:end]
        inner = inner.replace("&mut *event.entity", "entity")
        inner = re.sub(r"\bevent\s*\.\s*entity\b", "(*entity)", inner)
        inner = re.sub(r"\bevent\s*\.\s*pos\b", "tick_pos", inner)
        inner = re.sub(r"\*event\.reply = (?:mod_api::)?BlockEntityTickReply::\w+;", "// (unchanged: nothing is written back)", inner)
        files[path] = text[:m.start()] + head + inner + text[end:]
        helpers.add(m.group("name"))
        changed = True
for path, text in list(files.items()):
    for helper in helpers:
        text = re.sub(rf"\b{helper}\(\s*host,\s*event\b", f"{helper}(host, entity, tick_pos", text)
    files[path] = text

# run_mod_command: the hook goes; its body runs from every usage's executor on a rebuilt event
# (starling_command.rs), so the mod's own command code stays as written.
RUN = re.compile(r"\n(\s*)(?:///[^\n]*\n\s*)*fn run_mod_command\(\s*&self,\s*(?P<host>_?host): &mut dyn (?:mod_api::)?ServerHost,\s*"
                 r"(?P<ev>_?event): &mut (?:mod_api::)?RunModCommandEvent<'_>,?\s*\)\s*->\s*(?:mod_api::)?Flow\s*\{")
executor = None
uses_command = False
for path, text in list(files.items()):
    m = RUN.search(text)
    if not m:
        continue
    end_at = body_end(text, m.start() + 1)
    body = text[m.end():end_at - 1]
    module = "" if path.name == "lib.rs" else f"{path.stem}::"
    files[path] = (text[:m.start()] + text[end_at:] +
                   "\n/// Starling's port: the old `run_mod_command` hook's body.\n"
                   f"pub(crate) fn starling_run_mod_command(\n    {m.group('host')}: &mut dyn ServerHost,\n"
                   f"    {m.group('ev')}: &mut crate::starling_command::RunModCommandEvent<'_>,\n) -> Flow {{" + body + "}\n"
                   "\n/// Starling's port: every declared usage runs the old hook's body.\n"
                   "pub(crate) fn starling_executor(\n    host: &mut dyn ServerHost,\n    source: &mod_api::CommandSource,\n"
                   "    command: &mod_api::CommandInvocation,\n) -> Option<i32> {\n"
                   "    crate::starling_command::execute(host, source, command, starling_run_mod_command)\n}\n")
    executor = f"crate::{module}starling_executor"
    uses_command = True
if executor:
    for path, text in list(files.items()):
        text = re.sub(r"(Command::(?:op|new)\((?:[^()]|\([^()]*\))*\))(?!\s*\.executes)",
                      lambda m: m.group(1) + f".executes({executor})", text)
        text = text.replace("&mut Vec<Command>", "&mut Vec<mod_api::CommandRegistration>")
        text = text.replace("Vec<mod_api::Command>", "Vec<mod_api::CommandRegistration>")
        text = re.sub(r"-> Vec<Command>", "-> Vec<mod_api::CommandRegistration>", text)
        # The old event's type is the rebuilt one.
        if re.search(r"\bRunModCommandEvent\b", text):
            text = re.sub(r"\bmod_api::RunModCommandEvent\b", "crate::starling_command::RunModCommandEvent", text)
            if re.search(r"use mod_api::\{[^}]*\bRunModCommandEvent\b", text, re.S):
                text = re.sub(r"\bRunModCommandEvent,\s*", "", text, count=1) if re.search(r"\bRunModCommandEvent,", text) else re.sub(r",\s*RunModCommandEvent\b", "", text, count=1)
                text = re.sub(r"^(use mod_api::)", r"use crate::starling_command::RunModCommandEvent;\n\1", text, count=1, flags=re.M)
        files[path] = text

for path, text in list(files.items()):
    # ModContainer's new field.
    text = re.sub(r"(?<!-> )(?<!::)ModContainer \{\n(\s*)(?!insert_rules)", r"ModContainer {\n\1insert_rules: Vec::new(),\n\1", text)
    # A helper that took the component list: the event registers each id.
    text = re.sub(r"(\w+::)?declare_component\(event\.types\)", r"{ let mut types = Vec::new(); \1declare_component(&mut types); for id in types { event.register(id.into()); } }", text)
    # Placement and comparator hooks' level.
    text = re.sub(r"fn state_for_placement\(\s*&self,\s*level: &dyn (?:mod_api::)?LevelHost,([^{]*)\{",
                  r"fn state_for_placement(&self, level: mod_api::LevelSide<'_>,\1{\n        let (mod_api::LevelSide::Server(level) | mod_api::LevelSide::Client(level)) = level;",
                  text)
    text = re.sub(r"fn analog_output\(\s*&self,\s*level: &dyn (?:mod_api::)?LevelHost,([^{]*)\{",
                  r"fn analog_output(&self, level: &mut dyn mod_api::LevelRandomHost,\1{\n        let level: &dyn mod_api::LevelHost = &*level;",
                  text)
    text = re.sub(r"fn can_survive\(\s*&self,\s*level: &dyn (?:mod_api::)?LevelHost,([^{]*)\{",
                  r"fn can_survive(&self, level: mod_api::LevelSide<'_>,\1{\n        let mod_api::LevelSide::Server(level) = level else { return None };",
                  text)
    # Config and client-setup hooks.
    text = re.sub(r"(fn (?:register_configs|config_loading)\(\s*&self,\s*\w+): (?:mod_api::)?HostSide<'_>", r"\1: &dyn mod_api::ConfigContext", text)
    text = re.sub(r"(fn client_setup\(\s*&self,\s*\w+): &mut dyn (?:mod_api::)?ClientHost", r"\1: &mut dyn mod_api::ClientSetupContext", text)
    text = re.sub(r"(fn add_pack_finders\(\s*&self,\s*\w+): (?:mod_api::)?HostSide<'_>", r"\1: &mut dyn mod_api::PackDiscoveryContext", text)
    # Component ids, refusals.
    text = re.sub(r"event\.types\.push\(([^;]*)\);", r"event.register(\1.into());", text)
    reg = re.search(r"(\w+): &mut dyn (?:mod_api::)?Registrar", text)
    text = re.sub(r"\b(cx|context|_cx|_context|ctx)\.refuse\(", (reg.group(1) if reg else "r") + ".refuse(", text)
    # An item's id as a plain str.
    text = re.sub(r"\.item\(\)\.id\((host|&\*host|&\*server|server|client|&\*client|read|h)\)(?!\.)", r".item().id(\1).unwrap_or_default()", text)
    # A block's id as a plain str, and an id passed by reference.
    text = re.sub(r"\.block\(\)\.id\((host|&\*host|&\*server|server|client|&\*client|read|h)\)(?![.)])", r".block().id(\1).unwrap_or_default()", text)
    text = re.sub(r"\(&(\w+(?:\.\w+(?:\(\))?)*\.id\((?:host|&\*host|&\*server|read|h)\))\)", r"(\1.unwrap_or_default())", text)
    # The inventory is lent: the occupied slots' stacks copied out, as the old read gave them.
    text = re.sub(r"(\w+)\s*\.inventory_slots\((?:&\*)?(host|server)\)",
                  r"\1.with_inventory(\2, |view| view.iter().filter_map(|(slot, stack)| stack.project().map(|stack| (slot, stack))).collect::<Vec<_>>())",
                  text)
    # A block capability's provider takes an offer: `entity.map(|_| handler)` offers it.
    text = re.sub(
        r"(block_capability(?:_before_game)?\([^|]*)\|(\w+), (\w+), (\w+), (\w+)\| \{\s*(\w+)\.map\(\|_\| (.+?)\)\s*\}\);",
        r"\1|\2, \3, \4, \5, offer| {\n        if \6.is_some() {\n            offer.offer(|| \7);\n        }\n    });",
        text, flags=re.S)
    # Prepared drops are read and written by index.
    text = re.sub(
        r"for (\w+) in event\.drops\.iter_mut\(\) \{\s*let _ = \1\.stack\.apply_components\(&(\w+)\);\s*\}",
        r"for i in 0..event.drops.len() {\n        let Some(mut stack) = event.drops.get(i).and_then(|d| d.stack()).and_then(|s| s.project()) else {\n            continue;\n        };\n        if stack.apply_components(&\2).is_ok()\n            && let Some(mut drop) = event.drops.get_mut(i)\n        {\n            let _ = drop.set_stack(stack);\n        }\n    }",
        text)
    # An id read that a helper returns as `String`.
    text = re.sub(r"(-> String \{\s*\w+\.item\(\)\.id\(\w+\)\.unwrap_or_default\(\))(\s*\})", r"\1.to_owned()\2", text)
    text = re.sub(r"\.map\(\|key\| key\.id\((host|&\*host)\)\)", r".and_then(|key| key.id(\1).map(str::to_owned))", text)
    # An id in a tuple is kept past the host's borrow: owned, as before.
    text = re.sub(r"\((\w+\.item\(\)\.id\([^)]*\)\.unwrap_or_default\(\)), ", r"(\1.to_owned(), ", text)
    # Gone names out of `use mod_api::{..}`.
    # Ids kept in a list, mapped into one, or returned owned.
    text = re.sub(r"\.push\((\w+\.item\(\)\.id\([^)]*\)\.unwrap_or_default\(\))\)", r".push(\1.to_owned())", text)
    text = re.sub(r"\.map\(\|(\w+)\| \1\.item\(\)\.id\(([^)]*)\)\.unwrap_or_default\(\)\)", r".map(|\1| \1.item().id(\2).unwrap_or_default().to_owned())", text)
    text = re.sub(r"Some\(([\w.]+\([^()]*\)\?)\.id\((host|&\*host)\)\)", r"\1.id(\2).map(str::to_owned)", text)
    # A key's id read that was then borrowed as `Option<&str>`: `and_then` gives that at once.
    text = re.sub(r"\.map\(\|(\w+)\| \1\.id\(([^)]*)\)\)(\s*)\.as_deref\(\)", r".and_then(|\1| \1.id(\2))", text)
    text = re.sub(r"\.map\(\|(\w+)\| \1\.id\((&\*host|host)\)\);", r".and_then(|\1| \1.id(\2).map(str::to_owned));", text)
    # A kind's id compared with a str.
    text = re.sub(r"\.is_some_and\(\|(\w+)\| \1\.id\(([^)]*)\) == (\w+)\)", r".is_some_and(|\1| \1.id(\2) == Some(\3))", text)
    # An id bound to a name outlives the host's borrow: owned, as the old read was.
    text = re.sub(r"(let (?:mut )?\w+ = [\w.()]+?\.id\((?:&\*host|host|&\*server)\)\.unwrap_or_default\(\));", r"\1.to_owned();", text)
    # An id compared with a constant, or held in a struct field.
    text = re.sub(r"\.id\((&\*host|host)\) (!=|==) ([A-Z][A-Z0-9_]*(?:::[A-Z][A-Z0-9_]*)*)\b", r".id(\1) \2 Some(\3)", text)
    text = re.sub(r"\.map\(\|(\w+)\| \1\.id\(([^)]*)\)\) (!=|==) Some\((\w+(?:::\w+)*)\.to_owned\(\)\)", r".and_then(|\1| \1.id(\2)) \3 Some(\4)", text)
    text = re.sub(r"(?m)^(\s*\w+: \w+\.item\(\)\.id\([^)]*\)\.unwrap_or_default\(\)),$", r"\1.to_owned(),", text)
    # A key's id mapped and then defaulted into a String.
    text = re.sub(r"\.map\(\|(\w+)\| \1\.id\(([^)]*)\)\)(\s*)\.unwrap_or_(default\(\)|else\()",
                  r".and_then(|\1| \1.id(\2).map(str::to_owned))\3.unwrap_or_\4", text)
    # A bounded random draw is an i32 now.
    text = re.sub(r"\.next_int_bound\(([^()]*)\)\.unwrap_or\([^()]*\)", r".next_int_bound(\1)", text)
    # A menu opened: its type is `menu_type` now (`menu` is a live handle).
    text = re.sub(r"\bevent\.menu (!=|==) ", r"event.menu_type \1 ", text)
    # A block's id defaulted to an empty String.
    text = re.sub(r"\.map_or_else\(String::new, \|(\w+)\| \1\.id\(([^)]*)\)\)",
                  r".and_then(|\1| \1.id(\2).map(str::to_owned))\n        .unwrap_or_default()", text)
    # Prepared drops patched where their item matches.
    text = re.sub(
        r"for (\w+) in event\.drops\.iter_mut\(\) \{\s*if \1\.stack\.item\(\) == ([*\w]+) \{\s*let _ = \1\.stack\.apply_components\(&(\w+)\);\s*\}\s*\}",
        r"for i in 0..event.drops.len() {\n        let Some(mut stack) = event.drops.get(i).and_then(|d| d.stack()).and_then(|s| s.project()) else {\n            continue;\n        };\n        if stack.item() == \2\n            && stack.apply_components(&\3).is_ok()\n            && let Some(mut drop) = event.drops.get_mut(i)\n        {\n            let _ = drop.set_stack(stack);\n        }\n    }",
        text)
    # A menu slot read gives an owned stack.
    text = re.sub(r"(\.slot\(MenuContainer::\w+, [^)]*\)\s*\.map\(\|(\w+)\| \w+\(\w+, )\2\)", r"\1&\2)", text)
    # Held, worn and dropped stacks are lent: an owned copy where the old read gave one.
    text = re.sub(r"\.hand_item\(([^(),]+), ([^()]+?)\)", r".with_hand_item(\1, \2, |stack| stack.project()).flatten()", text)
    text = re.sub(r"\.equipment\(([^(),]+), ([^()]+?)\)", r".with_equipment(\1, \2, |stack| stack.project()).flatten()", text)
    text = re.sub(r"\.stack\((host|&\*host|server|&\*server)\)", r".with_stack(\1, |stack| stack.project()).flatten()", text)
    # A block's id compared with a str variable.
    text = re.sub(r"\((\w+)\.id\((host|&\*host)\) (==|!=) ([a-z_]\w*)\)", r"(\1.id(\2) \3 Some(\4))", text)
    # Recipe kinds: no codec; matching and assembling get the loop's context.
    text = re.sub(r"\n[ \t]*fn codec\(&self\) -> Option<(?:mod_api::)?CraftingRecipeCodec<Self::Value>> \{[^}]*\}\n", "\n", text)
    text = re.sub(r"\bCraftingRecipeCodec,\s*", "", text)
    text = re.sub(r"fn (matches|assemble)\(&self, (\w+): &Self::Value", r"fn \1(&self, _context: &dyn mod_api::LoopContext, \2: &Self::Value", text)
    # Indexed resource moves are `*_at`.
    text = re.sub(r"\.(extract|insert)\((host|&mut \*host|server), (tx|&tx), ([a-z_]\w*|\d+), ", r".\1_at(\2, \3, \4, ", text)
    # A sound played at a level takes who is left out (nobody).
    def sound(m):
        call = text_ref[0][m.start():]
        depth, i = 0, call.index("(")
        while True:
            c = call[i]
            depth += c == "("
            depth -= c == ")"
            if depth == 0:
                break
            i += 1
        args = call[call.index("(") + 1:i]
        top = 0
        commas = 0
        for c in args:
            top += c in "([{"
            top -= c in ")]}"
            commas += c == "," and top == 0
        if args.rstrip().endswith(","):
            commas -= 1
        return commas
    text_ref = [text]
    out, pos = [], 0
    for m in re.finditer(r"\.play_sound\(", text):
        if sound(m) == 5:
            start = m.end() - 1
            depth, i = 0, start
            while True:
                depth += text[i] == "("
                depth -= text[i] == ")"
                if depth == 0:
                    break
                i += 1
            inner = text[start + 1:i].rstrip()
            sep = "" if inner.endswith(",") else ","
            out.append(text[pos:start + 1] + inner + sep + " None")
            pos = i
    text = "".join(out) + text[pos:]
    # An id `.into()` an Option of an owned String.
    text = re.sub(r"\.id\(([^)]+)\)\.into\(\)", r".id(\1).map(str::to_owned)", text)
    # A tank's capacity for a resource names it as an Option; an id read as a str.
    text = re.sub(r"\.capacity\((host|&\*host), (\w+), &(\w+)\)", r".capacity(\1, \2, Some(&\3))", text)
    text = re.sub(r"\.id\((host|&\*host|server|client)\)\.as_str\(\)", r".id(\1).unwrap_or_default()", text)
    # A key's id read off a `?` and bound: owned, defaulting to "" as an unknown id read before.
    text = re.sub(r"(let \w+ = [^;=]*\?)\.id\((host|&\*host|server|&\*server)\);", r"\1.id(\2).unwrap_or_default().to_owned();", text)
    # A mod's own resource handler: the indexed moves are `*_at`, a capacity names its resource
    # as an Option (one the old code was never asked without answers 0).
    for hm in list(re.finditer(r"impl (?:mod_api::)?ResourceHandler<[^>]+> for \w+ \{", text)):
        hstart = text.find(hm.group(0))
        if hstart < 0:
            continue
        hend = body_end(text, hstart)
        block = text[hstart:hend]
        block = re.sub(r"fn (insert|extract)\((\s*&self,\s*[^,]+,\s*[^,]+,\s*\w+: usize,)", r"fn \1_at(\2", block)
        def capacity(cm):
            name = cm.group(3)
            head = f"fn capacity(&self,{cm.group(1)},{cm.group(2)}, {name}: Option<&{cm.group(4)}>) -> u32 {{"
            if name.startswith("_"):
                return head
            return head + f"\n        let Some({name}) = {name} else {{\n            return 0;\n        }};"
        block = re.sub(r"fn capacity\(&self,([^,]+),([^,]+), (\w+): &(\w+)\) -> u32 \{", capacity, block)
        text = text[:hstart] + block + text[hend:]
    # A block entity's capability provider: the old closure answers, its handler is offered.
    out, pos = [], 0
    for cm in re.finditer(r"\.block_entity_capability(?:_before_game)?\(", text):
        if cm.start() < pos:
            continue
        open_at = cm.end() - 1
        depth, i, top_commas = 0, open_at, []
        while True:
            c = text[i]
            if c in "([{":
                depth += 1
            elif c in ")]}":
                depth -= 1
                if depth == 0:
                    break
            elif c == "," and depth == 1:
                top_commas.append(i)
            i += 1
        starts = [c for c in top_commas if re.match(r"\s*(move\s+)?\|", text[c + 1:])]
        if not starts:
            continue
        last = text[starts[0] + 1:i]
        arm = re.match(r"\s*(move\s+)?\|([^|,]*),([^|,]*)\|", last)
        if not arm:
            continue
        top_commas = [starts[0]]
        old_closure = last.strip().rstrip(",")
        new_closure = ("move |starling_level, starling_side, _, _, offer| {\n        let old = " + old_closure +
                       ";\n        if let Some(handler) = old(starling_level, starling_side) {\n            offer.offer(move || handler);\n        }\n    }")
        out.append(text[pos:top_commas[-1] + 1] + " " + new_closure)
        pos = i
    text = "".join(out) + text[pos:]
    # An id read in the other arm of a `map_or_else` with an owned default.
    text = re.sub(r"(\.map_or_else\(\|\| \"[^\"]*\"\.to_owned\(\), \|(\w+)\| \2\.id\(([^)]*)\))\)",
                  r"\1.unwrap_or_default().to_owned())", text)
    text = re.sub(r"(Some\((\w+)\) => \2\.id\((host|&\*host)\)),", r"\1.unwrap_or_default().to_owned(),", text)
    # The old tick's lent entity is the copy's type.
    text = re.sub(r"&mut (?:mod_api::)?ModBlockEntityView<'_>", "&mut crate::starling_tick::OldEntity", text)
    text = re.sub(r"(&'\w+ mut )(?:mod_api::)?ModBlockEntityView<'\w+>", r"\1crate::starling_tick::OldEntity", text)
    # A struct whose second lifetime only named the old view: the lifetime goes.
    for sm in list(re.finditer(r"struct (\w+)<'(\w+), '(\w+)> \{", text)):
        name, first, second = sm.groups()
        body = text[sm.end():body_end(text, sm.start())]
        if f"'{second}" not in body:
            text = text.replace(sm.group(0), f"struct {name}<'{first}> {{")
            text = re.sub(rf"\b{name}<'_, '_>", f"{name}<'_>", text)
    for gone in ("BlockEntityTickReply", "ModBlockEntityView",
                 "StructureRandom"):
        text = re.sub(rf"\b{gone},\s*", "", text)
        text = re.sub(rf",\s*{gone}\b", "", text)
    text = text.replace("&mut dyn StructureRandom", "&mut dyn mod_api::RandomSource")
    files[path] = text

# Screens: a Part's old screen methods move to its `StarlingScreens` impl (starling_screen.rs),
# with Horizon's types rebuilt there; registering it for its menu type is left to do by hand.
SCREEN_METHODS = ("screen", "screen_draw", "screen_click", "screen_tooltip")
SHIM_TYPES = ("Screen", "ScreenEvent", "ScreenDrawEvent", "ScreenClickEvent", "ScreenTooltipEvent")
uses_screen = False
screen_parts = []
IMPL = re.compile(r"(#\[mod_api::sunrise_part\]\s*)?impl (?:mod_api::)?BlockEntityPart for (?P<part>\w+(?:<[^>]*>)?) \{")
for path, text in list(files.items()):
    pos = 0
    while (m := IMPL.search(text, pos)):
        end_at = body_end(text, m.start())
        block = text[m.end():end_at - 1]
        moved = []
        for name in SCREEN_METHODS:
            fm = re.search(rf"\n(?:[ \t]*///[^\n]*\n)*[ \t]*fn {name}\(", block)
            if not fm:
                continue
            fend = body_end(block, fm.start() + fm.group(0).index("fn"))
            moved.append(block[fm.start():fend])
            block = block[:fm.start()] + block[fend:]
        if moved:
            part = m.group("part")
            extra = f"\n\nimpl crate::starling_screen::StarlingScreens for {part} {{" + "".join(moved) + "\n}"
            text = text[:m.end()] + block + "}" + extra + text[end_at:]
            screen_parts.append(f"{path.stem}::{part}")
            uses_screen = True
            pos = m.end() + len(block) + len(extra)
        else:
            pos = end_at
    if uses_screen or re.search(r"\b(?:mod_api::)?Screen(?:Draw|Click|Tooltip)?Event\b|\bmod_api::Screen\b", text):
        for ty in SHIM_TYPES:
            text = re.sub(rf"\bmod_api::{ty}\b", f"crate::starling_screen::{ty}", text)
        found = [ty for ty in SHIM_TYPES if re.search(rf"use mod_api::\{{[^}}]*\b{ty}\b", text, re.S)]
        for ty in found:
            text = re.sub(rf"\b{ty},\s*", "", text, count=1) if re.search(rf"\b{ty},", text) else re.sub(rf",\s*{ty}\b", "", text, count=1)
        if found:
            text = re.sub(r"^(use mod_api::)", "use crate::starling_screen::{" + ", ".join(found) + "};\n" + r"\1", text, count=1, flags=re.M)
            uses_screen = True
    files[path] = text
if screen_parts:
    menus = sorted({m for t in files.values() for m in re.findall(r"\b(\w*MENU\w*): &str = \"([\w:/]+)\"", t)})
    notes.append("screens: register each Part for its menu with crate::starling_screen::register(r, MENU, Part): "
                 + ", ".join(screen_parts) + " | menu consts: " + ", ".join(f"{a}={b}" for a, b in menus))

# Menu buttons: a Part's old `menu_button` moves to its `StarlingMenuButton` impl (starling_menu.rs).
uses_menu = False
button_parts = []
for path, text in list(files.items()):
    pos = 0
    while (m := IMPL.search(text, pos)):
        end_at = body_end(text, m.start())
        block = text[m.end():end_at - 1]
        fm = re.search(r"\n(?:[ \t]*///[^\n]*\n)*[ \t]*fn menu_button\(", block)
        if not fm:
            pos = end_at
            continue
        fend = body_end(block, fm.start() + fm.group(0).index("fn"))
        method = block[fm.start():fend]
        block = block[:fm.start()] + block[fend:]
        part = m.group("part")
        extra = f"\n\nimpl crate::starling_menu::StarlingMenuButton for {part} {{" + method + "\n}"
        text = text[:m.end()] + block + "}" + extra + text[end_at:]
        button_parts.append(f"{path.stem}::{part}")
        uses_menu = True
        pos = m.end() + len(block) + len(extra)
    if uses_menu and re.search(r"\bMenuButtonEvent\b", text):
        text = re.sub(r"\bmod_api::MenuButtonEvent\b", "crate::starling_menu::MenuButtonEvent", text)
        if re.search(r"use mod_api::\{[^}]*\bMenuButtonEvent\b", text, re.S):
            text = re.sub(r"\bMenuButtonEvent,\s*", "", text, count=1) if re.search(r"\bMenuButtonEvent,", text) else re.sub(r",\s*MenuButtonEvent\b", "", text, count=1)
            text = re.sub(r"^(use mod_api::)", r"use crate::starling_menu::MenuButtonEvent;\n\1", text, count=1, flags=re.M)
    files[path] = text
if button_parts:
    notes.append("buttons: r.menu_part(MENU, crate::starling_menu::Buttons(Box::new(Part))) for " + ", ".join(button_parts))
if uses_menu:
    shutil.copyfile(SHIMS / "starling_menu.rs", src / "starling_menu.rs")
    uses_tick = True
    lib = src / "lib.rs"
    text = files[lib]
    if "mod starling_menu;" not in text:
        m = re.search(r"^(pub )?mod \w+;", text, re.M) or re.search(r"^use ", text, re.M)
        files[lib] = text[:m.start()] + "mod starling_menu;\n" + text[m.start():]

if uses_screen:
    shutil.copyfile(SHIMS / "starling_screen.rs", src / "starling_screen.rs")
    lib = src / "lib.rs"
    text = files[lib]
    if "mod starling_screen;" not in text:
        m = re.search(r"^(pub )?mod \w+;", text, re.M) or re.search(r"^use ", text, re.M)
        files[lib] = text[:m.start()] + "mod starling_screen;\n" + text[m.start():]

if uses_command:
    shutil.copyfile(SHIMS / "starling_command.rs", src / "starling_command.rs")
    lib = src / "lib.rs"
    text = files[lib]
    if "mod starling_command;" not in text:
        m = re.search(r"^(pub )?mod \w+;", text, re.M) or re.search(r"^use ", text, re.M)
        files[lib] = text[:m.start()] + "mod starling_command;\n" + text[m.start():]

if uses_tick or any("crate::starling_tick" in t for t in files.values()):
    shutil.copyfile(SHIMS / "starling_tick.rs", src / "starling_tick.rs")
    lib = src / "lib.rs"
    text = files[lib]
    if "mod starling_tick;" not in text:
        m = re.search(r"^(pub )?mod \w+;", text, re.M) or re.search(r"^use ", text, re.M)
        text = text[:m.start()] + "mod starling_tick;\n" + text[m.start():]
        files[lib] = text
for path, text in files.items():
    path.write_text(text, encoding="utf-8")
print(f"{mod}: autoport done; executor {executor}; tick helpers {sorted(helpers)}")
for note in notes:
    print("NOTE", note)
