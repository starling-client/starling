import tempfile
import unittest
from pathlib import Path

from tools import state_digest_compare


def line(tick, dim="minecraft:overworld", run="1", entities="3:aa"):
    return (f"tick={tick} dim={dim} run={run} entities={entities} randoms=1:bb block_entities=0:cc "
            f"level_random=dd/None/0 counters=5/6 scheduled=0/0:ee\n")


BASE = [line(100, run="0"), line(101), line(102), line(103)]


class CompareTests(unittest.TestCase):
    def test_the_same_run_is_identical(self):
        report, same = state_digest_compare.compare(BASE, list(BASE))
        self.assertTrue(same)
        self.assertEqual(report[-1], "IDENTICAL")

    def test_a_shifted_game_time_is_different(self):
        shifted = [line(100, run="0"), line(101), line(103), line(104)]
        report, same = state_digest_compare.compare(BASE, shifted)
        self.assertFalse(same)
        self.assertIn("tick            DIFFERS on 2 steps, first step 2 (tick 102): 102 vs 103", report)

    def test_a_step_in_another_level_is_different(self):
        moved = [line(100, run="0"), line(101), line(102, dim="minecraft:the_nether"), line(103)]
        report, same = state_digest_compare.compare(BASE, moved)
        self.assertFalse(same)
        self.assertIn("dim             DIFFERS on 1 steps, first step 2 (tick 102): minecraft:overworld vs "
                      "minecraft:the_nether", report)

    def test_a_missing_step_is_different(self):
        _, same = state_digest_compare.compare(BASE, BASE[:-1])
        self.assertFalse(same)

    def test_a_run_without_stepped_ticks_is_different(self):
        _, same = state_digest_compare.compare(BASE, [line(100, run="0")])
        self.assertFalse(same)


class CoverageTests(unittest.TestCase):
    def runs(self, a, b):
        with tempfile.TemporaryDirectory() as tmp:
            dirs = [Path(tmp) / "a", Path(tmp) / "b"]
            for d, text in zip(dirs, (a, b)):
                d.mkdir()
                if text is not None:
                    (d / "coverage.txt").write_text(text)
            return state_digest_compare.compare_coverage(*dirs)

    def test_runs_without_coverage_have_nothing_to_compare(self):
        self.assertIsNone(self.runs(None, None))

    def test_a_different_saved_trader_delay_is_different(self):
        _, same = self.runs("trader spawn_delay=22800\n", "trader spawn_delay=24000\n")
        self.assertFalse(same)

    def test_coverage_in_one_run_only_is_different(self):
        _, same = self.runs("fox_spit reached\n", None)
        self.assertFalse(same)

    def test_the_same_coverage_is_identical(self):
        _, same = self.runs("fox_spit reached\n", "fox_spit reached\n")
        self.assertTrue(same)


if __name__ == "__main__":
    unittest.main()
