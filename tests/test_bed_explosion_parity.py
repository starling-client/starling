import unittest

from tools import test_bed_explosion_parity as bed


class BedExplosionSamplingTests(unittest.TestCase):
    def test_item_samples_compare_exact_motion_at_the_same_elapsed_ticks(self):
        case = 'south-behind-foot'

        def transcript(age, push='0.0625', after='0.05', elapsed=10):
            return [
                f'Alice step={case} chat Stone has the following entity data: '
                f'{{Age: {age}s, Motion: [{push}d, 0.0d, -0.03125d], Health: 1s}}',
                f'Alice step={case} chat Stone has the following entity data: '
                f'{{Age: {age + elapsed}s, Motion: [{after}d, 0.0d, -0.025d], Health: 1s}}',
            ]

        reference = bed.compared(transcript(27))
        self.assertEqual(reference, bed.compared(transcript(41)))
        self.assertNotEqual(reference, bed.compared(transcript(41, push='0.06250000000000001')))
        self.assertNotEqual(reference, bed.compared(transcript(41, after='0.05000000000000001')))
        with self.assertRaises(AssertionError):
            bed.compared(transcript(41, elapsed=11))
        with self.assertRaises(AssertionError):
            bed.compared(transcript(41)[:1])


if __name__ == '__main__':
    unittest.main()
