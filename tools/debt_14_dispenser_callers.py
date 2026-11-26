#!/usr/bin/env python3
"""Migrate internal test consumers and journal producers for debt item 14."""

from pathlib import Path


GAME = Path("crates/sunrise-client/src/game")
sources = {}


def replace(file, old, new, count=1):
    path = GAME / file
    source = sources.get(path, path.read_text())
    assert source.count(old) == count, (file, old)
    sources[path] = source.replace(old, new)


replace(
    "blocks/dispense_container_tests.rs",
    "game.dispense_item(at, Direction::East, stack).unwrap()",
    "game.dispense_item(at, Direction::East, stack)\n        .unwrap()\n        .continue_value()\n        .unwrap()",
)
replace(
    "blocks/empty_contents_tests.rs",
    "        ItemStack::new(\"minecraft:stick\", 1),\n    )\n    .unwrap()\n}",
    "        ItemStack::new(\"minecraft:stick\", 1),\n    )\n    .unwrap()\n    .continue_value()\n    .unwrap()\n}",
)
replace(
    "blocks/growth.rs",
    'ItemStack::new("minecraft:bone_meal", 3),\n                    )\n                    .unwrap();',
    'ItemStack::new("minecraft:bone_meal", 3),\n                    )\n                    .unwrap()\n'
    '                    .continue_value()\n                    .unwrap();',
)
replace(
    "items/item_abilities_tests.rs",
    "game.dispense_item((7, 64, 0)",
    "let _ = game.dispense_item((7, 64, 0)",
    4,
)
replace(
    "native_hooks/block_entity_on_load_routes_tests.rs",
    "            assert!(left.is_none());",
    "            assert_eq!(left.continue_value(), Some(None));",
)
replace(
    "native_hooks/on_block_placed_by_tests.rs",
    "Direction::East, shulker_stack())\n            .unwrap()\n            .unwrap();",
    "Direction::East, shulker_stack())\n            .unwrap()\n            .continue_value()\n            .unwrap()\n            .unwrap();",
    3,
)
for indent in (" " * 16, " " * 20):
    replace(
        "native_hooks/placement_context_tests.rs",
        f'"minecraft:shulker_box", 2),\n{indent})\n{indent}.unwrap()\n{indent}.unwrap();',
        f'"minecraft:shulker_box", 2),\n{indent})\n{indent}.unwrap()\n'
        f"{indent}.continue_value()\n{indent}.unwrap()\n{indent}.unwrap();",
    )

replace(
    "modding/disabled_block_tests.rs",
    "                        .unwrap(),\n                    Some(stack.clone())",
    "                        .unwrap(),\n                    std::ops::ControlFlow::Break(())",
)
replace(
    "modding/disabled_block_tests.rs",
    "                    left,\n                    Some(stack),",
    "                    left.continue_value(),\n                    Some(Some(stack)),",
)
replace(
    "modding/disabled_block_tests.rs",
    'for (input, remaining) in [(3, 3), (4, 2)] {',
    'for (input, remaining) in [(3, None), (4, Some(2))] {',
)
replace(
    "modding/disabled_block_tests.rs",
    '                            Some(ItemStack::new("minecraft:stick", remaining))',
    '                            remaining.map_or(std::ops::ControlFlow::Break(()), |count| {\n'
    '                                std::ops::ControlFlow::Continue(Some(ItemStack::new("minecraft:stick", count)))\n'
    '                            })',
)

for file, receiver, pos in (
    ("blocks/dispense_container_tests.rs", "game", "SLOT_POS"),
    ("blocks/machines/native_items.rs", "self", "pos"),
    ("modding/native_book_apply.rs", "self", "pos"),
    ("modding/native_menu_refs.rs", "self", "pos"),
    ("modding/slot_takes.rs", "self", "pos"),
    ("network/lan_crafting.rs", "game", "pos"),
    ("network/lan_crafting.rs", "game", "at"),
):
    replace(file, f"After::Changed({pos})", f"After::Changed({receiver}.dim, {pos})")
replace(
    "modding/slot_takes.rs",
    "After::LostMenuContents(pos, Box::new(entity))",
    "After::LostMenuContents(self.dim, pos, Box::new(entity))",
)
replace("blocks/dispenser.rs", "After::DispenserChanged", "After::Changed")
replace(
    "native_hooks/block_entity_tick.rs",
    "journal_block_entity_loan(\n            self.pos,",
    "journal_block_entity_loan(\n            (self.dim, self.pos),",
)
for effect, args in (
    ("FluidWrite", "WrittenFluid"),
    ("FluidEffects", "FluidEffects"),
):
    replace(
        "blocks/world_fluids.rs",
        f"After::{effect}(Box::new({args}",
        f"After::{effect}(self.dim, Box::new({args}",
    )

for path, source in sources.items():
    path.write_text(source)
