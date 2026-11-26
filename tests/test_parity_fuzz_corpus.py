import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import parity_fuzz as fuzz
from tools import parity_fuzz_corpus as corpus

SCENARIO = {'family': 'persist', 'seed': 1, 'steps': [
    {'k': 'block', 'block': 'chest', 'at': [7, 0, 7]}, {'k': 'read_block', 'at': [7, 0, 7]}]}
ORIGINAL = {**SCENARIO, 'steps': [*SCENARIO['steps'], {'k': 'tick', 'n': 3}]}
PASS = {'failures': [], 'invalid': None, 'vanilla-unstable': []}
FAIL = {'failures': [('persist|chest|live|Items', 'read', [])], 'invalid': None}


class StepKindTests(unittest.TestCase):
    SAMPLES = {
        'summon': {'type': 'pig', 'at': [1, 0, 1], 'n': 0, 'nbt': {}},
        'block': {'block': 'chest', 'at': [7, 0, 7]},
        'cmd': {'cmd': '/say hi'},
        'tick': {'n': 1},
        'dispense': {'at': [1, 0, 1]},
        'reload': {},
        'read_entity': {'n': 0},
        'read_near': {'type': 'pig', 'at': [1, 0, 1]},
        'read_items': {},
        'read_block': {'at': [7, 0, 7]},
    }

    def test_every_corpus_step_kind_renders_and_other_kinds_are_refused(self):
        self.assertEqual(self.SAMPLES.keys(), corpus.STEP_FIELDS.keys())
        for kind, fields in self.SAMPLES.items():
            with self.subTest(kind=kind):
                scenario = {'family': 'persist', 'seed': 1, 'steps': [{'k': kind, **fields}]}
                self.assertTrue(fuzz.render(scenario, 0))
        with self.assertRaises(ValueError):
            fuzz.render({'family': 'persist', 'seed': 1, 'steps': [{'k': 'unknown'}]}, 0)


class CorpusTests(unittest.TestCase):
    def run_cli(self, directory, args, results):
        argv = ['parity_fuzz.py', '--work-dir', str(directory / 'run'), *args]
        with (patch.object(sys, 'argv', argv), patch.object(fuzz, 'CORPUS', directory / 'cases'),
              patch.object(fuzz.Runner, 'run', return_value=results) as run,
              contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()) as err):
            code = fuzz.main()
        return code, run, err.getvalue()

    def test_promote_checks_original_and_shrunk_before_writing_only_replay_inputs(self):
        group = {'signature': 'persist|chest|live|Items', 'scenario': SCENARIO, 'original': ORIGINAL,
                 'differences': ['private capture'], 'commands': ['/data get block 7 300 7']}
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            source = directory / 'group.json'
            source.write_text(json.dumps(group), encoding='utf-8')
            args = ['--promote', str(source), '--name', 'chest-reload']
            path = directory / 'cases/chest-reload.json'
            code, run, _ = self.run_cli(directory, args, [PASS, FAIL])
            run.assert_called_once_with([SCENARIO, ORIGINAL], single_batch=True)
            self.assertEqual(code, 1)
            self.assertFalse(path.exists())
            code, _, _ = self.run_cli(directory, args, [PASS, PASS])
            self.assertEqual(code, 0)
            self.assertEqual(json.loads(path.read_text(encoding='utf-8')), {
                'name': 'chest-reload', 'family': 'persist', 'origin_seed': 1, 'signature': group['signature'],
                'scenario': SCENARIO, 'original': ORIGINAL,
            })

    def test_corpus_discovers_both_forms_together_and_names_the_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            corpus.write(directory / 'cases', corpus.Case.from_input('a-pass', SCENARIO))
            corpus.write(directory / 'cases', corpus.Case.from_input('b-fail', {
                'scenario': SCENARIO, 'original': ORIGINAL}))
            code, run, error = self.run_cli(directory, ['--corpus', '--batch', '1'], [PASS, PASS, PASS, FAIL])
        run.assert_called_once_with([SCENARIO, SCENARIO, SCENARIO, ORIGINAL], single_batch=True)
        self.assertEqual(code, 1)
        self.assertEqual(error, 'Cases did not match: b-fail\n')

    def test_promote_rejects_report_fields_inside_a_scenario(self):
        bad = {**SCENARIO, 'steps': [{'k': 'read_block', 'at': [7, 0, 7], 'vanilla': 'private capture'}]}
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            source = directory / 'scenario.json'
            source.write_text(json.dumps(bad), encoding='utf-8')
            with patch.object(fuzz.Runner, 'run') as run:
                # Argument errors are raised before the fixture boots.
                with (patch.object(sys, 'argv', ['parity_fuzz.py', '--work-dir', folder,
                                                '--promote', str(source), '--name', 'bad']),
                      contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit)):
                    fuzz.main()
                run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
