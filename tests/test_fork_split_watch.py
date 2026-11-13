"""The fork's game.rs split watch (tools/starling/fork_split_watch.py)."""
import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "tools" / "starling" / "fork_split_watch.py"
spec = importlib.util.spec_from_file_location("fork_split_watch", SCRIPT)
watch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(watch)

MANIFEST = """\
# a comment, and then the header
upstream_path\tfork_path\tfunctions\titems\tfork_copies
game/saving.rs\tgame/world/saving.rs\tsave_level\t\t
game/weather.rs\tgame/world/weather.rs\train_height,tick_weather_effects\t\t
game/runtime/launch_options.rs\tgame/runtime/launch_options.rs\t\t\t
"""


class Progress(unittest.TestCase):
    def test_a_function_still_in_game_rs_holds_its_destination_back(self):
        game = "pub(super) fn save_level(&mut self) {}\nfn rain_height<T>(x: T) {}\n"
        moves = watch.progress(MANIFEST, game, exists=lambda _: True)
        self.assertEqual([m[0] for m in moves],
                         ["game/world/saving.rs", "game/world/weather.rs",
                          "game/runtime/launch_options.rs"])
        self.assertEqual([m[2] for m in moves],
                         [["save_level"], ["rain_height"], []])
        self.assertEqual([m[3] for m in moves], [False, False, True])

    def test_a_destination_is_moved_once_its_file_is_there_and_game_rs_is_clear(self):
        moves = watch.progress(MANIFEST, "fn something_else() {}", exists=lambda _: True)
        self.assertTrue(all(m[3] for m in moves))
        self.assertIn("destinations: 3 of 3 moved", watch.lines(moves))
        self.assertIn("functions: 3 of 3 out of game.rs", watch.lines(moves))

    def test_a_destination_without_its_file_is_not_moved(self):
        moves = watch.progress(MANIFEST, "", exists=lambda _: False)
        self.assertFalse(any(m[3] for m in moves))
        self.assertIn("destinations: 0 of 3 moved", watch.lines(moves))

    def test_a_name_inside_another_name_does_not_count(self):
        # `save_level_backup` is not `save_level`.
        game = "fn save_level_backup() {}\nfn also_rain_height() {}"
        moves = watch.progress(MANIFEST, game, exists=lambda _: True)
        self.assertEqual([m[2] for m in moves], [[], [], []])

    def test_the_summary_says_whether_to_wait(self):
        waiting = watch.lines(watch.progress(MANIFEST, "fn save_level() {}", lambda _: True))
        self.assertTrue(waiting[-1].startswith("Wait for this"))
        clear = watch.lines(watch.progress(MANIFEST, "", lambda _: True))
        self.assertTrue(clear[-1].startswith("The fork's split is done"))


if __name__ == "__main__":
    unittest.main()
