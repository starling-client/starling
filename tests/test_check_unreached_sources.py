"""A crate file that no `mod` item or `include!` reaches is named; declared and included ones are not."""
import tempfile
import unittest
from pathlib import Path

from tools.ci import check_unreached_sources


class UnreachedSources(unittest.TestCase):
    def test_an_undeclared_source_file_is_named_and_declared_or_included_ones_are_not(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)

            def put(path, text):
                file = root / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(text)

            put("Cargo.toml", '[workspace]\nmembers = ["crates/sunrise-client"]\n')
            put("crates/sunrise-client/Cargo.toml", '[package]\nname = "sunrise-client"\n')
            put("crates/sunrise-client/src/lib.rs", "mod game;\n")
            put("crates/sunrise-client/src/game.rs", '#[path = "game/blocks/widget.rs"]\nmod widget;\n')
            put("crates/sunrise-client/src/game/blocks/widget.rs", 'include!("widget_parts/shared.rs");\n')
            put("crates/sunrise-client/src/game/blocks/widget_parts/shared.rs", "fn shared() {}\n")
            put("crates/sunrise-client/src/game/blocks/forgotten.rs", "fn forgotten() {}\n")
            put("crates/sunrise-client/src/mobs/stray_tests.rs", "fn stray() {}\n")
            put("crates/sunrise-client/tests/fixtures/data.rs", "fn data() {}\n")
            self.assertEqual(check_unreached_sources.unreached(root),
                             ["crates/sunrise-client/src/game/blocks/forgotten.rs",
                              "crates/sunrise-client/src/mobs/stray_tests.rs"])


if __name__ == "__main__":
    unittest.main()
