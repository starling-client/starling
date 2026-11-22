import unittest

from tools.test_level_environment_reads_parity import passed_resource_tests


class LevelEnvironmentReadsParityTests(unittest.TestCase):
    def test_counts_larger_group_only_when_the_vanilla_vector_test_passed(self):
        names = [f'game::environment_read_tests::other_{i}' for i in range(6)]
        names.append('game::environment_read_tests::level_environment_reads_match_vanilla_vectors')
        summary = 'test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 99 filtered out\n'
        self.assertEqual(passed_resource_tests(
            ''.join(f'test {name} ... ok\n' for name in names) + summary), 7)
        names[-1] = 'game::environment_read_tests::other_6'
        with self.assertRaises(AssertionError):
            passed_resource_tests(''.join(f'test {name} ... ok\n' for name in names) + summary)
