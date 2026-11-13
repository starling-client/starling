"""Regression checks for actual import declarations versus function-local imports."""
import unittest
from compare_items import items
from moved_lines_check import lines


class ImportBoundaryTests(unittest.TestCase):
    def test_function_with_local_import_is_still_compared(self):
        source = 'fn example() { use std::mem::size_of; let _ = size_of::<u32>() + 1; }'
        changed = source.replace('+ 1', '+ 2')
        self.assertNotEqual(items(source), items(changed))
        self.assertNotEqual(lines(source), lines(changed))

    def test_only_actual_imports_are_omitted(self):
        source = '#[cfg(test)] use std::mem::size_of;\nfn example() {}'
        self.assertEqual(items(source), items('fn example() {}'))
        self.assertEqual(lines(source), lines('fn example() {}'))


if __name__ == '__main__':
    unittest.main()
