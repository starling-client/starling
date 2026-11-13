import importlib.util
from pathlib import Path
import unittest
import tempfile
import subprocess
import sys

spec = importlib.util.spec_from_file_location('merge_lang', Path(__file__).resolve().parents[1] / 'tools/sync/merge_lang.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class TranslationMergeTests(unittest.TestCase):
    def test_upstream_change(self):
        self.assertEqual(module.merge({'a': 'old'}, {'a': 'old'}, {'a': 'new'}), ({'a': 'new'}, []))

    def test_fork_change_and_addition(self):
        self.assertEqual(module.merge({'a': 'old'}, {'a': 'fork', 'b': 'added'}, {'a': 'old'}), ({'a': 'fork', 'b': 'added'}, []))

    def test_deletion(self):
        self.assertEqual(module.merge({'a': 'old'}, {}, {'a': 'old'}), ({}, []))

    def test_fork_deletion_survives_retranslation(self):
        self.assertEqual(module.merge({'a': 'old'}, {}, {'a': 'translated'}), ({}, []))

    def test_conflict(self):
        self.assertEqual(module.merge({'a': 'old'}, {'a': 'fork'}, {'a': 'new'}), ({'a': 'fork'}, ['a']))

    def test_driver_format_and_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ('base', 'ours', 'theirs')]
            for path in paths:
                path.write_text('{"z": "été", "a": "first"}')
            result = subprocess.run([sys.executable, str(spec.origin), *map(str, paths), '7', 'lang.json'], capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(paths[1].read_text(), '{\n  "a": "first",\n  "z": "été"\n}\n')

    def test_order(self):
        self.assertEqual(list(module.merge({}, {'z': 'last', 'a': 'first'}, {})[0]), ['a', 'z'])
