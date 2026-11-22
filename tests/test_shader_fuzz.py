import json
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from pathlib import Path
from unittest.mock import patch

from tools import shader_fuzz as fuzz

GROUP = {'group': 'panic|declaration|translate/mod.rs:12: boom', 'count': 3, 'replay': 99, 'dir': '/w/groups/01-panic',
         'invariant': 'panic', 'step': 'declaration', 'lines': 1}


class DriverTests(unittest.TestCase):
    def run_driver(self, report, extra=()):
        """The driver with a fake fuzzer that writes `report`; its exit code, output and the fuzzer's arguments."""
        seen = {}
        with tempfile.TemporaryDirectory() as temp:
            work = Path(temp) / 'work'
            binary = Path(temp) / 'shader_fuzz'
            binary.write_text('')
            work.mkdir()
            (work / 'report.json').write_text('previous')

            def fake(args, **kwargs):
                seen['args'] = args[1:]
                run_dir = Path(args[args.index('--work-dir') + 1])
                self.assertEqual(run_dir.parent, work / 'runs')
                self.assertEqual(list(run_dir.iterdir()), [])
                (run_dir / 'report.json').write_text(json.dumps(report))
                return subprocess.CompletedProcess(args, 1 if report['groups'] else 0)

            out = StringIO()
            with patch.object(fuzz.subprocess, 'run', side_effect=fake), redirect_stdout(out):
                code = fuzz.main(['--binary', str(binary), '--work-dir', str(work), *extra])
            self.assertEqual((work / 'report.json').read_text(), 'previous')
        return code, out.getvalue(), seen['args']

    def test_groups_print_their_replay_and_fail_the_run(self):
        report = {'cases': 40, 'failing': 3, 'corpus_dir': '/packs', 'groups': [GROUP]}
        code, out, args = self.run_driver(report, ['--seed', '7', '--replay', '99'])
        self.assertEqual(code, 1)
        self.assertEqual(out.splitlines(), [
            '40 cases, 3 failures in 1 groups',
            '     3  panic|declaration|translate/mod.rs:12: boom',
            '        python3 tools/shader_fuzz.py --corpus /packs --replay 99   # shrunk: /w/groups/01-panic'])
        self.assertEqual(args[args.index('--seed') + 1], '7')
        self.assertEqual(args[args.index('--replay') + 1], '99')
        self.assertNotIn('--smoke', args)

    def test_output_directory_error_is_a_driver_error(self):
        with tempfile.TemporaryDirectory() as temp:
            occupied = Path(temp) / 'file'
            occupied.write_text('keep')
            with patch.object(fuzz.subprocess, 'run') as run, redirect_stderr(StringIO()):
                code = fuzz.main(['--binary', str(occupied), '--work-dir', str(occupied)])
            self.assertEqual(code, 2)
            run.assert_not_called()
            self.assertEqual(occupied.read_text(), 'keep')

    def test_unreadable_report_is_a_driver_error(self):
        with tempfile.TemporaryDirectory() as temp:
            work = Path(temp)
            binary = work / 'binary'
            binary.touch()
            def fake(args, **kwargs):
                run_dir = Path(args[args.index('--work-dir') + 1])
                (run_dir / 'report.json').write_text('{')
                return subprocess.CompletedProcess(args, 0)
            with patch.object(fuzz.subprocess, 'run', side_effect=fake), redirect_stderr(StringIO()):
                self.assertEqual(fuzz.main(['--binary', str(binary), '--work-dir', str(work)]), 2)

    def test_a_clean_run_passes(self):
        code, out, args = self.run_driver({'cases': 9, 'failing': 0, 'corpus_dir': None, 'groups': []}, ['--smoke'])
        self.assertEqual(code, 0)
        self.assertEqual(out.splitlines(), ['9 cases, 0 failures in 0 groups'])
        self.assertIn('--smoke', args)
        self.assertNotIn('--corpus', args)


if __name__ == '__main__':
    unittest.main()
