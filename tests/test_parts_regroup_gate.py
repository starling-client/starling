"""The Parts gate relaxes name-resolved declarations, never enum case order."""
import contextlib
import importlib.util
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

DIRECTORY = Path(__file__).resolve().parents[1] / 'tools/refactors/api-pr1'
spec = importlib.util.spec_from_file_location('parts_gate', DIRECTORY / 'gate.py')
gate = importlib.util.module_from_spec(spec)
with patch.object(sys, 'path', [str(DIRECTORY), *sys.path]):
    spec.loader.exec_module(gate)

WIT = '''interface types {
  enum registration-error { invalid, duplicate }
  enum saved-state { active, stopped }
}
interface registrar {
  first: func() -> result<_, registration-error>;
  second: func(id: u32);
}
interface hooks {
  before: func();
  after: func();
}
'''


class PartsGateTests(unittest.TestCase):
    def compare(self, other):
        with contextlib.redirect_stdout(io.StringIO()):
            return gate.compare_wit(WIT, other)

    def test_registrar_methods_are_name_resolved(self):
        self.assertTrue(self.compare(WIT.replace(
            '  first: func() -> result<_, registration-error>;\n  second: func(id: u32);',
            '  second: func(id: u32);\n  first: func() -> result<_, registration-error>;'
        )))

    def test_result_error_cases_remain_byte_ordered(self):
        self.assertFalse(self.compare(WIT.replace('invalid, duplicate', 'duplicate, invalid')))

    def test_case_addition_removal_or_duplication_is_not_a_move(self):
        for cases in ('invalid, other', 'invalid', 'invalid, duplicate, invalid'):
            with self.subTest(cases=cases):
                self.assertFalse(self.compare(WIT.replace('invalid, duplicate', cases)))

    def test_method_signature_changes_and_duplicate_methods_are_refused(self):
        self.assertFalse(self.compare(WIT.replace('id: u32', 'id: u64')))
        self.assertFalse(self.compare(WIT.replace('  second: func(id: u32);', '  second: func(id: u32);\n  second: func(id: u32);')))

    def test_hook_order_remains_byte_ordered(self):
        self.assertFalse(self.compare(WIT.replace('  before: func();\n  after: func();', '  after: func();\n  before: func();')))

    def test_other_enums_remain_byte_ordered(self):
        self.assertFalse(self.compare(WIT.replace('active, stopped', 'stopped, active')))
