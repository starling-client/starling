#!/usr/bin/env python3
"""Attach item model evaluators to CallbackValue and update their engine consumers."""

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src"


def replace(source, old, new, count=1):
    if source.count(old) != count:
        raise SystemExit(f"Expected {count} occurrence(s) of {old!r}")
    return source.replace(old, new)


def consumers(source, invocations, prechecks, dispatches):
    source = replace(source, "(bound.eval)(", "(bound.eval.value())(", invocations)
    source = replace(source, "bound.owner.is_enabled()", "bound.eval.owner().is_enabled()", prechecks)
    source, actual = re.subn(r"(\bbound\n( +))\.owner\n", r"\1.eval\n\2.owner()\n", source)
    if actual != dispatches:
        raise SystemExit(f"Expected {dispatches} dispatches, found {actual}")
    return source


def providers(source):
    source = replace(source, "CallbackOwner, ItemModelRegistration", "CallbackOwner, CallbackValue, ItemModelRegistration")
    for callback in ("ItemTintSourceFn", "ItemModelFn", "RangeSelectPropertyFn", "SelectPropertyFn"):
        visibility = "" if callback == "SelectPropertyFn" else "pub "
        source = replace(
            source,
            f"    pub owner: Arc<CallbackOwner>,\n    {visibility}eval: Arc<{callback}>,",
            f"    pub eval: CallbackValue<Arc<{callback}>>,",
        )
    source = replace(source, "        self.owner\n", "        self.eval.owner()\n")
    source = replace(source, "(self.eval)(host, input, &mut keys)", "(self.eval.value())(host, input, &mut keys)")
    source = replace(source, "Option<(Arc<CallbackOwner>, Arc<F>)>", "Option<CallbackValue<Arc<F>>>")
    source = replace(
        source,
        '''    match entry
        .owner
        .call(callback_kind, "decode", || (entry.decode)(&config))
    {
        Ok(Ok(eval)) => Ok(Some((Arc::clone(&entry.owner), eval))),''',
        '''    match entry.owner.call(callback_kind, "decode", || {
        (entry.decode)(&config).map(|eval| {
            CallbackValue::new((*entry.owner).clone(), callback_kind, "drop_evaluator", eval)
        })
    }) {
        Ok(Ok(eval)) => Ok(Some(eval)),''',
    )
    source = replace(source, "bound.map(|(owner, eval)| {", "bound.map(|eval| {", 2)
    source = replace(source, "                owner,\n                eval,", "                eval,", 2)
    source = replace(
        source,
        "        (entry.decode)(&config, &mut children)",
        '''        (entry.decode)(&config, &mut children).map(|eval| {
            CallbackValue::new((*entry.owner).clone(), "item model", "drop_evaluator", eval)
        })''',
    )
    source = replace(source, "                owner: Arc::clone(&entry.owner),\n", "")
    source = replace(
        source,
        "            (entry.decode)(&config)\n",
        '''            (entry.decode)(&config).map(|(decode_case, eval)| {
                (decode_case, CallbackValue::new((*entry.owner).clone(), "item model select property", "drop_evaluator", eval))
            })
''',
    )
    return replace(source, "        owner: Arc::clone(&owner),\n", "")


edits = {
    "item_render/registered_providers.rs": lambda source: consumers(providers(source), 2, 0, 0),
    "item_render/tint_tests.rs": lambda source: consumers(source, 1, 0, 1),
    "game/interface/item_draw.rs": lambda source: consumers(source, 3, 4, 3),
}
rewritten = {ROOT / path: edit((ROOT / path).read_text()) for path, edit in edits.items()}
for path, source in rewritten.items():
    path.write_text(source)
