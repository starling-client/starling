import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1] / 'tools'
spec = importlib.util.spec_from_file_location('sync_upstream', TOOLS / 'sync_upstream.py')
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)


class SyncTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        self.hooks = self.root / sync.HOOKS
        self.hooks.mkdir(parents=True)
        (self.hooks / 'mod.rs').write_text('mod base;\nmod zebra;\n')
        (self.hooks / 'base.rs').write_text('')
        (self.hooks / 'zebra.rs').write_text('')
        self.fragments = self.root / sync.FRAGMENTS
        self.fragments.mkdir(parents=True)
        self.patch = patch.object(sync, 'ROOT', self.root)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def test_conversion_stages_doc_and_removes_json(self):
        doc = 'First line\n\n Last line'
        fragment = {'hooks': [{'name': 'example', 'doc': doc, 'side': 'server',
                    'target': "horizon_client::game::native_hooks::Game<'_>::native_example",
                    'mod_api': {'contract': 'listen'}}]}
        (self.fragments / 'example.json').write_text(json.dumps(fragment))
        subprocess.run(['git', 'add', '.'], cwd=self.root, check=True)
        sync.finish()
        text = (self.hooks / 'example.rs').read_text()
        lines = [line.strip() for line in text.splitlines() if line.strip().startswith('///')]
        self.assertEqual(sync.converter.decode(lines), doc)
        self.assertEqual((self.hooks / 'mod.rs').read_text(), 'mod base;\nmod example;\nmod zebra;\n')
        self.assertFalse((self.fragments / 'example.json').exists())
        staged = subprocess.check_output(['git', 'diff', '--cached', '--name-only'], cwd=self.root, text=True)
        self.assertIn('example.rs', staged)

    def test_invalid_fragment_is_left_in_place(self):
        fragment = self.fragments / 'invalid.json'
        fragment.write_text('{"draw_contexts": [1]}')
        with self.assertRaisesRegex(ValueError, 'invalid.json'):
            sync.finish()
        self.assertTrue(fragment.exists())
        self.assertFalse((self.hooks / 'invalid.rs').exists())

    def test_finish_checks_patched_root_for_duplicates(self):
        for name in ('a', 'b'):
            (self.hooks / f'{name}.rs').write_text('pub fn native_example() {}\n')
        with self.assertRaisesRegex(ValueError, 'native_example'):
            sync.finish()

    def test_duplicate_names_are_reported(self):
        for name in ('a', 'b'):
            (self.hooks / f'{name}.rs').write_text('pub fn native_example() {}\n')
        with self.assertRaisesRegex(ValueError, 'native_example'):
            sync.check_duplicates(self.root)


if __name__ == '__main__':
    unittest.main()
