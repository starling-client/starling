#!/usr/bin/env python3
"""Migrate the worn-equipment callback fixture signatures and stack arguments."""

from pathlib import Path


PATH = (
    Path(__file__).resolve().parents[1]
    / "crates/sunrise-client/src/game/modding/client_item_extensions.rs"
)
marker = "#[cfg(test)]\nmod tests {"
engine, tests = PATH.read_text().split(marker, 1)
for old, new, count in [
    (": &mod_api::ItemStack", ": &mod_api::ItemStackView<'_>", 8),
    ("&EquipmentLayer,", "&EquipmentLayer<'_>,", 7),
    ("_: ArmorTextureRef,", "_: &str,", 2),
    (
        "&mod_api::ItemStack::of(item),",
        "&mod_api::ItemStackView::from_stack(&mod_api::ItemStack::of(item)),",
        3,
    ),
    ("                    &stack,\n", "                    &mod_api::ItemStackView::from_stack(&stack),\n", 1),
]:
    assert tests.count(old) == count, old
    tests = tests.replace(old, new)
PATH.write_text(engine + marker + tests)
