"""The mace harness's exact cases: record framing, comparison, and equipment proofs."""
import unittest

from tools import test_mace_parity as mace

SMASH = 'S 0,0,0 minecraft:item.mace.smash_ground'
THORNS = 'S 0,0,0 minecraft:enchant.thorns.hit'


class RecordTests(unittest.TestCase):
    def test_records_are_numbered_in_order_and_kept_whole(self):
        text = 'case 0\nV 0 20.0 0 0 0\nD minecraft:stone\ncase 1\ncase 2\nrng 5\n'
        self.assertEqual(mace.records(text), [['V 0 20.0 0 0 0', 'D minecraft:stone'], [], ['rng 5']])
        for bad in ('case 0\ncase 2\n', 'case 1\n', 'rng 5\ncase 0\n'):
            with self.subTest(text=bad), self.assertRaises(SystemExit):
                mace.records(bad)

    def test_an_event_stream_keeps_its_order_and_state_does_not(self):
        record = ['stat minecraft:mace minecraft:used 1', 'V 0 18.0 0 0 0', 'H 20.0', SMASH, THORNS]
        self.assertEqual(mace.exact_record(record), mace.exact_record([*record[1:3][::-1], record[0], *record[3:]]))
        self.assertNotEqual(mace.exact_record(record), mace.exact_record([*record[:3], THORNS, SMASH]))
        self.assertNotEqual(mace.exact_record(record), mace.exact_record(record + [SMASH]))


class ArmorProofTests(unittest.TestCase):
    def test_health_must_differ_between_two_unambiguous_target_rows(self):
        proof = ('V 0 ', None)
        equipped = ['V 0 18.0 0 0 0']
        bare = ['V 0 16.0 0 0 0']
        self.assertTrue(mace.proven(equipped, bare, proof))
        self.assertFalse(mace.proven(equipped, equipped, proof))
        for case, twin in ((equipped, []), ([], bare), (equipped * 2, bare), (equipped, bare * 2)):
            with self.subTest(case=case, twin=twin):
                self.assertFalse(mace.proven(case, twin, proof))


class ThornsProofTests(unittest.TestCase):
    PROOF = ('W chest ', 240)

    def worn(self, damage, enchanted):
        components = [f'minecraft:damage:{damage}'] if damage else []
        if enchanted:
            components.append('minecraft:enchantments:{"minecraft:thorns":7}')
        return f'W chest {{components:{{{",".join(components)}}},count:1,id:"minecraft:iron_chestplate"}}'

    def test_the_attempt_wears_the_piece_beyond_its_twin(self):
        self.assertTrue(mace.proven([self.worn(5, True)], [self.worn(3, False)], self.PROOF))
        self.assertTrue(mace.proven([self.worn(2, True)], [self.worn(0, False)], self.PROOF))

    def test_a_different_stack_alone_is_no_attempt(self):
        # The enchantment makes the lines differ before any wear.
        self.assertFalse(mace.proven([self.worn(3, True)], [self.worn(3, False)], self.PROOF))
        self.assertFalse(mace.proven([self.worn(4, True)], [self.worn(3, False)], self.PROOF))

    def test_a_break_counts_only_when_the_wear_reaches_the_durability(self):
        self.assertTrue(mace.proven([], [self.worn(238, False)], self.PROOF))
        self.assertFalse(mace.proven([], [self.worn(100, False)], self.PROOF))
        self.assertFalse(mace.proven([self.worn(5, True)], [], self.PROOF))

    def test_the_durability_is_the_slots_base_times_the_material(self):
        self.assertEqual(mace.durability('chest', 'iron_chestplate'), 240)
        self.assertEqual(mace.durability('head', 'leather_helmet'), 55)


if __name__ == '__main__':
    unittest.main()
