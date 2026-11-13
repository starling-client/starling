"""Keep conflict reporting aligned with the area export ownership contract."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('sync_policies', Path(__file__).resolve().parents[1] / 'tools/sync/resolve_policies.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SyncAreaTests(unittest.TestCase):
    def test_areas(self):
        cases = {
            'launcher/lang/fr.json': 'translations',
            'crates/horizon-client/assets/horizon/lang/fr_fr.json': 'translations',
            'crates/horizon-client/src/game/entities/entity.rs': 'game',
            'crates/horizon-client/src/game.rs': 'clientroot',
            'crates/horizon-client-render/src/gfx.rs': 'renderlauncher',
            'launcher/src/main.rs': 'renderlauncher',
            'crates/horizon-world/src/lib.rs': 'crates',
            'docs/api.md': 'toolsdocs',
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assertEqual(module.area(path), expected)
