import contextlib
import io
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import oracle_harness


def message(level, rendered):
    return json.dumps({'reason': 'compiler-message', 'message': {'level': level, 'rendered': rendered}})


class CargoFailure(unittest.TestCase):
    def run_failing(self, command, output):
        def run(args, stdout, **kwargs):
            stdout.write(output)
            return subprocess.CompletedProcess(args, 101)

        with tempfile.TemporaryDirectory() as build, patch.object(oracle_harness.subprocess, 'run', side_effect=run), \
                contextlib.redirect_stderr(io.StringIO()) as said:
            with self.assertRaises(subprocess.CalledProcessError) as raised:
                oracle_harness.run_logged(command, Path(build), 'compile-rust')
            self.assertEqual(raised.exception.returncode, 101)
        return said.getvalue()

    def test_a_failed_cargo_build_says_its_errors_and_its_last_lines(self):
        log = '\n'.join([
            message('warning', 'warning: unused import'),
            message('error', 'error[E0308]: mismatched types\n --> src/game.rs:1:1\n'),
            *(f'   Compiling crate{n}' for n in range(40)),
            'error: could not compile `sunrise-client` (lib test) due to 1 previous error',
            json.dumps({'reason': 'build-finished', 'success': False}),
        ]) + '\n'
        said = self.run_failing(['cargo', 'test', '--no-run', '--message-format=json'], log)
        self.assertIn('error[E0308]: mismatched types\n --> src/game.rs:1:1', said)
        self.assertNotIn('unused import', said)
        self.assertIn('could not compile `sunrise-client`', said)
        # Only the last 30 lines of cargo's own text.
        self.assertNotIn('Compiling crate10\n', said)
        self.assertIn('Compiling crate39', said)

    def test_another_programs_failure_prints_nothing_of_its_output(self):
        said = self.run_failing(['java', '-cp', 'x', 'Oracle'], 'some output of the program\n')
        self.assertEqual(said, '')


if __name__ == '__main__':
    unittest.main()
