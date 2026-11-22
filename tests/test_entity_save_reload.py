import argparse
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest import mock

from tools import entity_save_reload as harness


def binary_for(path):
    return harness.sunrise_binary(argparse.Namespace(binary=path))


class SunriseBinaryTests(unittest.TestCase):
    def test_relative_binary_is_resolved_against_the_callers_directory(self):
        # Each server runs with its world as the working directory, so the path must not stay relative.
        before = os.getcwd()
        with tempfile.TemporaryDirectory() as caller:
            os.chdir(caller)
            try:
                found = binary_for('target/fast/sunrise-client')
                expected = Path(os.getcwd()) / 'target' / 'fast' / 'sunrise-client'
            finally:
                os.chdir(before)
        self.assertTrue(found.is_absolute())
        self.assertEqual(found, expected)

    def test_absolute_binary_is_kept(self):
        binary = Path(os.path.abspath('sunrise-client'))
        self.assertEqual(binary_for(str(binary)), binary)

    def test_a_failed_build_says_so_and_leaves_cargos_errors_in_the_log(self):
        failed = subprocess.CompletedProcess([], 101)
        with mock.patch.object(harness.subprocess, 'run', return_value=failed) as run:
            with self.assertRaisesRegex(SystemExit, r'cargo build of sunrise-client failed \(exit 101\)'):
                binary_for(None)
        self.assertNotIn('stdout', run.call_args.kwargs, "cargo writes to the harness's own output")


if __name__ == '__main__':
    unittest.main()
