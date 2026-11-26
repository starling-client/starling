"""Inventory is discovered, never a hand-maintained enumeration of source types."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = str(Path(__file__).resolve().parents[1] / "tools")
sys.path.insert(0, TOOLS)
from scan_recursive_wire import ROOT, recursive_types
sys.path.remove(TOOLS)

# Every discovered cycle needs an explicit policy. New types fail this test until
# their codec and depth coverage have been reviewed.
POLICIES = {
    "Compound": "iterative Tag codec; vanilla 512; wire/depth_tests.rs",
    "Tag": "iterative Tag codec; vanilla 512; wire/depth_tests.rs",
    "Ingredient": "generic 32; wire/depth_tests.rs",
    "ItemPlace": "no Wire; process-local item address",
    "RecipeSlotDisplay": "generic 32; wire/depth_tests.rs",
    "Text": "iterative TextForm; vanilla 512; wire/depth_tests.rs",
    "TextContent": "cycle broken by TextForm; generic envelope; wire/depth_tests.rs",
    "TextHover": "cycle broken by TextForm; generic envelope; wire/depth_tests.rs",
    "TextObject": "cycle broken by TextForm; generic envelope; wire/depth_tests.rs",
    "TextStyle": "cycle broken by TextForm; generic envelope; wire/depth_tests.rs",
    "ModelPart": "no Wire; local client model tree",
    "serde_json::Value": "iterative preflight before serde; wire/depth_tests.rs",
    "serde_json::Map": "iterative preflight before serde; wire/depth_tests.rs",
    "WalkedIngredient": "no Wire; private parser work tree",
}


class RecursiveWire(unittest.TestCase):
    def test_every_cycle_has_a_policy(self):
        policies = dict(POLICIES)
        topics = Path(__file__).with_name("recursive_wire_policies")
        for path in sorted(topics.glob("*.json")):
            topic = json.loads(path.read_text(encoding="utf-8"))
            self.assertFalse(policies.keys() & topic.keys(), f"duplicate policy: {path.name}")
            policies.update(topic)
        self.assertEqual(set(policies), set(recursive_types(extra_paths=[ROOT / "crates/sunrise-core/src/nbt.rs"])))

    def test_discovers_direct_and_indirect_cycles_not_variant_names(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lib.rs"
            path.write_text('''
                pub struct A { pub children: Option<(u8, Vec<Self>)> }
                pub enum B { Other(Box<C>), A }
                pub struct C { pub value: B }
                pub struct Leaf { pub b: u8 }
                pub struct Wrapped { children: PrivateChildren }
                struct PrivateChildren(Vec<Wrapped>);
                pub struct Generic<T: Fn()> { children: Vec<Generic<T>>, marker: T }
                // pub struct Fake { self: Fake }
                #[cfg(test)] mod tests { pub struct Fake { x: Fake } }
            ''')
            self.assertEqual(recursive_types(Path(directory)), ["A", "B", "C", "Generic", "Wrapped"])
