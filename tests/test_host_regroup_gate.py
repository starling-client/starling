"""The host regroup gate permits name ordering, never contract changes."""
import contextlib
import importlib.util
import io
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    'wit_order', Path(__file__).resolve().parents[1] / 'tools/refactors/api-pr1/wit_order.py')
wit_order = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wit_order)

WIT = '''package horizon:mod-api@0.1.0;
interface types {
  record first {
    x: u32,
    y: u32,
  }
  type second = string;
}
interface host {
  use types.{first, second};
  a: func(x: first);
  b: func() -> second;
}
interface hooks {
  before: func();
  after: func();
}
world mod {
  import host;
  export hooks;
}
'''


class HostRegroupGateTests(unittest.TestCase):
    def compare(self, other):
        with contextlib.redirect_stdout(io.StringIO()):
            return wit_order.compare_wit(WIT, other)

    def test_only_name_resolved_sets_may_reorder(self):
        moved = WIT.replace('  a: func(x: first);\n  b: func() -> second;\n',
                            '  b: func() -> second;\n  a: func(x: first);\n')
        moved = moved.replace('  record first {\n    x: u32,\n    y: u32,\n  }\n  type second = string;\n',
                              '  type second = string;\n  record first {\n    x: u32,\n    y: u32,\n  }\n')
        moved = moved.replace('use types.{first, second}', 'use types.{second, first}')
        self.assertTrue(self.compare(moved))

    def test_no_contract_edit_is_accepted(self):
        for before, after in [
            ('a: func(x: first)', 'a: func(x: second)'),
            ('  b: func() -> second;\n', ''),
            ('  b: func() -> second;\n', '  b: func() -> second;\n  b: func() -> second;\n'),
            ('type second = string', 'type second = u32'),
            ('    x: u32,\n    y: u32,', '    y: u32,\n    x: u32,'),
            ('use types.{first, second}', 'use types.{first, third}'),
            ('  before: func();\n  after: func();', '  after: func();\n  before: func();'),
            ('  import host;\n  export hooks;', '  export hooks;\n  import host;'),
            ('package horizon:mod-api@0.1.0;', 'package horizon:mod-api@0.2.0;'),
        ]:
            with self.subTest(before=before):
                self.assertFalse(self.compare(WIT.replace(before, after)))


if __name__ == '__main__':
    unittest.main()
