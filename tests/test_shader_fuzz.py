import json
import re
import shlex
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
from pathlib import Path
from unittest.mock import patch

from tests.os_support import needs_symlinks
from tools import shader_fuzz as fuzz


def group(name, replay, replay_args, count=3):
    return {'group': name, 'count': count, 'replay': replay, 'replay_args': replay_args, 'dir': '/w/groups/01-x',
            'invariant': name.split('|')[0], 'step': name.split('|')[1], 'lines': 1}


def option(args, flag):
    """The value `args` gives `flag`, None when it isn't there."""
    return args[args.index(flag) + 1] if flag in args else None


def counts(out):
    """The numbers of the output's first line: cases, skipped, failures, groups."""
    return [int(n) for n in re.findall(r'[0-9]+', out.splitlines()[0])]


def replay_commands(out):
    """The commands the output prints, as the shell splits them (comments dropped)."""
    commands = [shlex.split(line, comments=True) for line in out.splitlines()]
    return [c[2:] for c in commands if c[:2] == ['python3', 'tools/shader_fuzz.py']]


class DriverTests(unittest.TestCase):
    def run_driver(self, report, extra=(), work=None):
        """The driver with a fake fuzzer that writes `report`; its exit code, output and the fuzzer's arguments."""
        seen = {}
        with tempfile.TemporaryDirectory() as temp:
            work = Path(temp) / 'work' if work is None else work
            binary = Path(temp) / 'shader_fuzz'
            binary.write_text('')
            work.mkdir(exist_ok=True)
            (work / 'report.json').write_text('previous')

            def fake(args, **kwargs):
                seen['args'] = args[1:]
                run_dir = Path(args[args.index('--work-dir') + 1])
                self.assertEqual(run_dir.parent.resolve(), (work / 'runs').resolve())
                self.assertEqual(list(run_dir.iterdir()), [])
                (run_dir / 'report.json').write_text(json.dumps(report))
                return subprocess.CompletedProcess(args, 1 if report['groups'] or not report['cases'] else 0)

            out = StringIO()
            with patch.object(fuzz.subprocess, 'run', side_effect=fake), redirect_stdout(out):
                code = fuzz.main(['--binary', str(binary), '--work-dir', str(work), *extra])
            self.assertEqual((work / 'report.json').read_text(), 'previous')
        return code, out.getvalue(), seen['args']

    def test_groups_print_the_command_that_replays_their_case(self):
        with tempfile.TemporaryDirectory() as temp:
            corpus = Path(temp) / "my packs 'v2'"
            corpus.mkdir()
            resolved = str(corpus.resolve())
            seeds = Path(temp) / 'my seeds'
            seeds.mkdir()
            own = str(seeds.resolve())
            seed_args = ['--replay', '99', '--seed-pack', own, '--corpus', resolved, '--time-limit', '20',
                         '--memory-limit', '2048', '--size-limit', '3']
            smoke_args = ['--replay', 'smoke:2', '--seed-pack', own, '--time-limit', '20', '--memory-limit', '2048',
                          '--size-limit', '3']
            report = {'cases': 40, 'failing': 4, 'corpus_dir': resolved, 'groups': [
                group('panic|declaration|translate/mod.rs:12: boom', '99', seed_args),
                group('size|base|final.fsh: preprocessed past the limit', 'smoke:2', smoke_args, count=1)]}
            code, out, args = self.run_driver(report, ['--seed', '7', '--corpus', str(corpus), '--size-limit', '3',
                                                       '--seed-pack', str(seeds)])
            self.assertEqual(code, 1)
            self.assertEqual(counts(out), [40, 0, 4, 2])
            self.assertEqual(option(args, '--seed'), '7')
            self.assertEqual(option(args, '--corpus'), resolved)
            self.assertEqual(option(args, '--seed-pack'), own)
            self.assertEqual(option(args, '--size-limit'), '3')
            # Each group's command, run again, asks the fuzzer for that case with the run's packs and limits.
            commands = replay_commands(out)
            self.assertEqual(commands, [seed_args, smoke_args])
            for command, expected in zip(commands, [seed_args, smoke_args]):
                again_code, _, again = self.run_driver({'cases': 1, 'failing': 0, 'groups': []}, command)
                self.assertEqual(again_code, 0)
                for flag in ['--replay', '--seed-pack', '--corpus', '--time-limit', '--memory-limit', '--size-limit']:
                    self.assertEqual(option(again, flag), option(expected, flag), flag)
                self.assertNotIn('--smoke', again)

    def test_budget_alone_is_not_capped_by_a_count(self):
        clean = {'cases': 9, 'failing': 0, 'corpus_dir': None, 'groups': []}
        _, _, args = self.run_driver(clean, ['--budget', '1200'])
        self.assertEqual((option(args, '--budget'), option(args, '--count')), ('1200', None))
        _, _, args = self.run_driver(clean, ['--count', '500'])
        self.assertEqual((option(args, '--budget'), option(args, '--count')), (None, '500'))
        _, _, args = self.run_driver(clean, ['--count', '5', '--budget', '60'])
        self.assertEqual((option(args, '--budget'), option(args, '--count')), ('60', '5'))
        # Neither: the fuzzer's own default applies.
        _, _, args = self.run_driver(clean)
        self.assertEqual((option(args, '--budget'), option(args, '--count')), (None, None))

    def test_replay_takes_a_seed_or_a_smoke_case(self):
        clean = {'cases': 1, 'failing': 0, 'corpus_dir': None, 'groups': []}
        for case in ['0', '18446744073709551615', 'smoke:0', 'smoke:12']:
            _, _, args = self.run_driver(clean, ['--replay', case])
            self.assertEqual(option(args, '--replay'), case)
        for bad in ['', 'smoke:', 'smoke:x', '-1', '0x1', 'smoke:1:2', ' 1', 'seed:1']:
            with self.assertRaises(SystemExit) as stop, redirect_stderr(StringIO()):
                fuzz.main(['--binary', '/nonexistent', '--replay', bad])
            self.assertEqual(stop.exception.code, 2, bad)

    @needs_symlinks
    def test_a_symlinked_work_directory_holds_the_run(self):
        with tempfile.TemporaryDirectory() as temp:
            real = Path(temp) / 'real'
            real.mkdir()
            link = Path(temp) / 'link'
            link.symlink_to(real, target_is_directory=True)
            code, _, args = self.run_driver({'cases': 1, 'failing': 0, 'groups': []}, work=link)
            self.assertEqual(code, 0)
            self.assertEqual(Path(option(args, '--work-dir')).parent, (real / 'runs').resolve())

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
        report = {'cases': 9, 'skipped': 0, 'failing': 0, 'corpus_dir': None, 'groups': []}
        code, out, args = self.run_driver(report, ['--smoke'])
        self.assertEqual(code, 0)
        self.assertEqual(counts(out), [9, 0, 0, 0])
        self.assertEqual(replay_commands(out), [])
        self.assertIn('--smoke', args)
        self.assertNotIn('--corpus', args)

    def test_a_run_in_which_no_case_ran_fails(self):
        report = {'cases': 0, 'skipped': 3, 'failing': 0, 'corpus_dir': '/packs', 'groups': []}
        code, out, _ = self.run_driver(report, ['--corpus', '.'])
        self.assertEqual(code, 1)
        # The counts it prints: cases, skipped, failures, groups.
        self.assertEqual(counts(out), [0, 3, 0, 0])
        self.assertEqual(replay_commands(out), [])


if __name__ == '__main__':
    unittest.main()
