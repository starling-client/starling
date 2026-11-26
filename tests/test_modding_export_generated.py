"""The site export writes the generated chapters and real SDK tables without a Rust build in tests."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import modding_export_generated as exporter


class ExportGenerated(unittest.TestCase):
    def test_writes_both_generated_chapters_and_all_sdk_tables(self):
        chapters = {
            "notes": "# Hook notes\n\nNotes from the catalog.\n",
            "counterparts": "# NeoForge counterparts\n\nCounterparts from the catalog.\n",
            "reference": "# Mod API reference\n\nHooks and Host reference.\n",
        }
        surface = {
            "schema": 2,
            "records": [{"id": record} for record in (
                "hook:tick", "hook:tick.phase", "trait:Host", "method:Host::level", "assoc:Host::Side",
                "handle:Pos::new", "fn:log", "macro:hook", "type:Pos", "type:Pos.x", "type:Pos.y",
                "part:items", "capability:net", "behaviour:pool_merging",
            )],
            "extension_tags": [],
        }
        with (
            tempfile.TemporaryDirectory() as folder,
            patch.object(exporter.notes, "generated", side_effect=lambda _, name: chapters[name]),
            patch.object(exporter.tables, "export_surface", return_value=surface),
        ):
            out = Path(folder) / "generated"
            exporter.export(out)
            self.assertEqual((out / "reference/hook-notes.md").read_text(), chapters["notes"])
            self.assertEqual((out / "reference/neoforge-counterparts.md").read_text(), chapters["counterparts"])
            self.assertEqual((out / "reference/api-reference.md").read_text(), chapters["reference"])
            sdk = json.loads((out / "sdk-tables.json").read_text())
            self.assertEqual(set(sdk), {"sdk-assets", "sdk-routes", "sdk-hosts", "sdk-surface"})
            self.assertIn("`SHA256SUMS.sig`", sdk["sdk-assets"])
            self.assertIn("`runtime`, `api`, `engine`", sdk["sdk-routes"])
            self.assertIn("| Linux, x86-64 | `x86_64-unknown-linux-gnu` | yes | yes |", sdk["sdk-hosts"])
            self.assertIn("| `type:<path>.<field or variant>` | 2 |", sdk["sdk-surface"])


if __name__ == "__main__":
    unittest.main()
