"""The automatic parity runner must be able to invoke the special-attack harness."""
import unittest
from unittest.mock import patch

from tools import test_special_attack_parity as harness


class SpecialAttackInvocationTests(unittest.TestCase):
    def test_no_arguments_reaches_resource_validation(self):
        with patch('sys.argv', ['test_special_attack_parity.py']), \
                patch.object(harness.base, 'verify_artifacts', side_effect=RuntimeError('resources checked')):
            with self.assertRaisesRegex(RuntimeError, 'resources checked'):
                harness.main()
