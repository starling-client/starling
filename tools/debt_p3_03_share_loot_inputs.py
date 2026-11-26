#!/usr/bin/env python3
"""Share the prepared modifier map and borrow JSON during item loot rolls."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORLD = "crates/sunrise-world/src/"
ITEMS = "crates/sunrise-items/src/items.rs"
REPLACEMENTS = {
    WORLD + "loot.rs": [
        (
            "    modifiers: HashMap<String, Value>,\n"
            "    predicates: Arc<HashMap<String, Value>>,",
            "    modifiers: Arc<HashMap<String, Value>>,\n"
            "    predicates: Arc<HashMap<String, Value>>,",
        ),
        (
            "            modifiers,\n            predicates: Arc::new(predicates),",
            "            modifiers: Arc::new(modifiers),\n"
            "            predicates: Arc::new(predicates),",
        ),
    ],
    WORLD + "loot/holder_sets.rs": [
        (
            '            ("item_modifier", &mut data.modifiers),',
            '            ("item_modifier", Arc::make_mut(&mut data.modifiers)),',
        ),
    ],
    WORLD + "loot_types.rs": [
        (
            "    pub fn modifiers(&self) -> &HashMap<String, Value> {\n"
            "        &self.modifiers",
            "    pub fn modifiers(&self) -> &Arc<HashMap<String, Value>> {\n"
            "        &self.modifiers",
        ),
    ],
    WORLD + "loot_types/normalize.rs": [
        (
            "        for value in self.modifiers.values_mut() {",
            "        for value in Arc::make_mut(&mut self.modifiers).values_mut() {",
        ),
    ],
    WORLD + "loot_query_tests.rs": [
        (
            "        modifiers: HashMap::new(),\n        predicates: Arc::default(),",
            "        modifiers: Arc::default(),\n        predicates: Arc::default(),",
        ),
    ],
    ITEMS: [
        (
            "    loot_modifiers: HashMap<String, Value>,",
            "    loot_modifiers: Arc<HashMap<String, Value>>,",
        ),
        (
            "    /// Bind tables, predicates and prepared instances from the same candidate.",
            "    /// Bind tables, predicates, modifiers and prepared instances from the same candidate.",
        ),
        (
            "        self.loot_modifiers = data.modifiers().clone();",
            "        self.loot_modifiers = Arc::clone(data.modifiers());",
        ),
        (
            "    fn expand(\n"
            "        &self,\n"
            "        e: &Value,\n"
            "        random: &mut R,\n"
            "        out: &mut Vec<(i32, ItemStack, Value, bool)>,",
            "    fn expand<'v>(\n"
            "        &self,\n"
            "        e: &'v Value,\n"
            "        random: &mut R,\n"
            "        out: &mut Vec<(i32, ItemStack, &'v Value, bool)>,",
        ),
        (
            '                    e["modifier"].clone(),\n                    true,',
            '                    &e["modifier"],\n                    true,',
        ),
        (
            "                    Value::Null,\n"
            "                    false,\n"
            "                ));\n"
            "                true\n"
            "            }\n"
            "            // NestedLootTable: chosen by its weight like an item; `choose`",
            "                    &Value::Null,\n"
            "                    false,\n"
            "                ));\n"
            "                true\n"
            "            }\n"
            "            // NestedLootTable: chosen by its weight like an item; `choose`",
        ),
        (
            "                    e.clone(),\n                    true,",
            "                    e,\n                    true,",
        ),
        (
            '                        item: Arc::from("minecraft:air"),\n'
            "                        count: 0,\n"
            "                        damage: 0,\n"
            "                        extra: None,\n"
            "                    },\n"
            "                    Value::Null,\n"
            "                    false,",
            '                        item: Arc::from("minecraft:air"),\n'
            "                        count: 0,\n"
            "                        damage: 0,\n"
            "                        extra: None,\n"
            "                    },\n"
            "                    &Value::Null,\n"
            "                    false,",
        ),
        (
            "        let (_, mut stack, modifiers, emitted) = candidates.swap_remove(pick);\n"
            "        if &*stack.item == NESTED {\n"
            "            self.nested(&modifiers, random, produced);\n"
            "            return;\n"
            "        }\n"
            "        self.modifiers(&modifiers, &mut stack, random);",
            "        let (_, mut stack, entry_or_modifier, emitted) = candidates.swap_remove(pick);\n"
            "        if &*stack.item == NESTED {\n"
            "            self.nested(entry_or_modifier, random, produced);\n"
            "            return;\n"
            "        }\n"
            "        self.modifiers(entry_or_modifier, &mut stack, random);",
        ),
        (
            "        let list: Vec<&Value> = match m {\n"
            "            Value::Array(a) => a.iter().collect(),\n"
            "            Value::Object(_) => vec![m],",
            "        let list = match m {\n"
            "            Value::Array(a) => a.as_slice(),\n"
            "            Value::Object(_) => std::slice::from_ref(m),",
        ),
    ],
}


def main():
    updated = {}
    for relative, edits in REPLACEMENTS.items():
        path = ROOT / relative
        source = path.read_text(encoding="utf-8")
        for old, new in edits:
            assert source.count(old) == 1, (relative, old)
            source = source.replace(old, new, 1)
        updated[path] = source
    for path, source in updated.items():
        path.write_text(source, encoding="utf-8")


if __name__ == "__main__":
    main()
