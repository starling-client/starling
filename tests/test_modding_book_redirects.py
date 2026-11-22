"""The modding book's redirects file holds, and a page the book loses needs an entry."""
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.append(str(ROOT / "tools"))
import modding_book_redirects as redirects  # noqa: E402

PAGES = ["introduction.md", "SUMMARY.md", "start/first-mod.md", "concepts/native-route.md", "concepts/native-route/7-how-it-works.md"]


def text(entries):
    return json.dumps(dict(sorted(entries.items())), indent=2) + "\n"


class BookRedirectTests(unittest.TestCase):
    def test_the_books_redirects_hold(self):
        found = redirects.problems((ROOT / redirects.REDIRECTS).read_text(encoding="utf-8"), redirects.pages_on_disk())
        self.assertEqual(found, [])

    def test_an_address_drops_order_prefixes_and_the_introduction_is_the_book(self):
        self.assertEqual(redirects.addresses(PAGES), {"", "start/first-mod", "concepts/native-route", "concepts/native-route/how-it-works"})

    def test_an_entry_lands_on_a_page_and_hides_none(self):
        self.assertEqual(redirects.problems(text({"start/first-wasm-mod": "start/first-mod", "concepts/gone": ""}), PAGES), [])
        self.assertEqual(len(redirects.problems(text({"start/first-mod": "concepts/native-route"}), PAGES)), 1)
        self.assertEqual(len(redirects.problems(text({"start/first-wasm-mod": "start/missing"}), PAGES)), 1)
        self.assertEqual(len(redirects.problems(text({"start/first-wasm-mod": "start/8-first-mod"}), PAGES)), 1)

    def test_entries_are_plain_addresses_in_canonical_order(self):
        self.assertEqual(len(redirects.problems(text({"Start/Old": "start/first-mod"}), PAGES)), 1)
        self.assertEqual(len(redirects.problems(text({"start/old\n": "start/first-mod"}), PAGES)), 1)
        self.assertEqual(len(redirects.problems(text({"start/old": ["start/first-mod"]}), PAGES)), 1)
        unsorted = '{\n  "start/zz": "start/first-mod",\n  "start/aa": "start/first-mod"\n}\n'
        self.assertEqual(len(redirects.problems(unsorted, PAGES)), 1)
        twice = '{\n  "start/aa": "",\n  "start/aa": "start/first-mod"\n}\n'
        self.assertEqual(len(redirects.problems(twice, PAGES)), 1)

    def test_a_lost_address_needs_an_entry_and_a_renumbered_page_does_not(self):
        before = PAGES + ["how-to/wasm-mods.md", "start/first-wasm-mod.md"]
        after = [p.replace("7-how-it-works", "8-how-it-works") for p in PAGES]
        self.assertEqual(redirects.unredirected(before, after, {"start/first-wasm-mod": "start/first-mod"}), ["how-to/wasm-mods"])


if __name__ == "__main__":
    unittest.main()
