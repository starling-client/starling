#!/usr/bin/env python3
"""Thread the existing scoped loot place through the evaluators (P3-03, step 2)."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1] / "crates"


def edit(path, old, new, count=1):
    file = ROOT / path
    before = file.read_text()
    after, found = re.subn(old, new, before, flags=re.MULTILINE)
    if found != count:
        raise ValueError(f"{path}: expected {count} matches, got {found}")
    file.write_text(after)


world = "sunrise-world/src/"
items = "sunrise-items/src/"
for file, count in [("loot.rs", 2), ("loot/fill.rs", 1), ("loot_query.rs", 3)]:
    edit(world + file, r"^( +)query: query::current_query\(\),$",
         r"\g<0>\n\1place: current_native_loot_place(),", count)
edit(world + "loot.rs", r"^    query: std::rc::Rc<query::LootQuery>,$",
     r"\g<0>\n    place: Option<Arc<LootPlace>>,")
edit(world + "loot_query.rs", r"^            place: current_native_loot_place\(\),\n        }$",
     "            place: self.place.clone(),\n        }")
for file, query, count in [("items.rs", "sunrise_world::loot::query::", 7),
                           ("loot_query_tests.rs", "query::", 1)]:
    edit(items + file, rf"^( +)query: {query}current_query\(\),$",
         r"\g<0>\n\1place: sunrise_world::loot::current_native_loot_place(),", count)
edit(items + "items.rs", r"^    query: std::rc::Rc<sunrise_world::loot::query::LootQuery>,$",
     r"\g<0>\n    place: Option<Arc<sunrise_world::loot::LootPlace>>,")
edit(items + "loot_query.rs", r"^            query: context.query.clone\(\),$",
     r"\g<0>\n            place: context.place.clone(),")
edit(items + "loot_query.rs", r"^            place: sunrise_world::loot::current_native_loot_place\(\),$",
     "            place: self.place.clone(),")
