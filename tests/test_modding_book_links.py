"""The book's absolute links and tool install source name this repository and its site."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BOOK = ROOT / "docs" / "modding"
AUTHORING_GUIDE = ROOT / "docs" / "modding-book.md"

GAME_REPO = re.compile(r"https://github\.com/(?:arismoko|horizon-rust)/horizon(?![\w.-])")
# The book is published at playsunrise.net/docs/modding; `sunrise-org.github.io/sunrise-mods` is the registry's index.
STALE_SITE = re.compile(r"https://(?:arismoko|horizon-rust|sunrise-org)\.github\.io/sunrise(?![\w.-])")
INSTALL = re.compile(r"cargo\s+(?:install|binstall)\b[^\n]*\bcargo-sunrise-mod\b")
GIT_SOURCE = re.compile(r"--git\s+(\S+)")
UPSTREAM_PROJECTS = (
    "https://github.com/sunrise-org/sunrise-machine-sample",
    "https://github.com/horizon-rust/horizon-mixins",
    "https://github.com/sunrise-org/sunrise-mods",
)


class BookLinkTests(unittest.TestCase):
    def pages(self):
        return sorted((BOOK / "src").rglob("*.md")) + sorted((BOOK / "src").rglob("*.txt")) + [AUTHORING_GUIDE]

    def lines(self):
        for page in self.pages():
            for number, line in enumerate(page.read_text(encoding="utf-8").splitlines(), 1):
                yield page, number, line

    def test_game_repository_links_name_sunrise_org(self):
        for page, number, line in self.lines():
            for url in GAME_REPO.findall(line):
                with self.subTest(page=page.relative_to(ROOT), line=number):
                    self.assertTrue(url.startswith("https://github.com/sunrise-org/sunrise"), url)

    def test_book_site_links_name_sunrise_org(self):
        for page, number, line in self.lines():
            for url in STALE_SITE.findall(line):
                with self.subTest(page=page.relative_to(ROOT), line=number):
                    self.fail(f"stale book site URL {url}")

    def test_the_registry_page_beside_the_book_is_not_the_book_site(self):
        for url in ("https://sunrise-org.github.io/sunrise-mods/", "https://playsunrise.net/docs/modding/start/"):
            with self.subTest(url=url):
                self.assertIsNone(STALE_SITE.search(url))
        for url in ("https://arismoko.github.io/sunrise/start/", "https://sunrise-org.github.io/sunrise/start/"):
            with self.subTest(url=url):
                self.assertIsNotNone(STALE_SITE.search(url))

    def test_cargo_sunrise_mod_installs_from_sunrise_org(self):
        found = 0
        for page, number, line in self.lines():
            if not INSTALL.search(line):
                continue
            found += 1
            source = GIT_SOURCE.search(line)
            with self.subTest(page=page.relative_to(ROOT), line=number):
                self.assertIsNotNone(source, "cargo-sunrise-mod install names no --git source")
                self.assertEqual(source.group(1), "https://github.com/sunrise-org/sunrise")
        self.assertGreater(found, 0, "the book would stop showing how to install cargo-sunrise-mod")

    def test_explicit_upstream_projects_are_kept(self):
        text = "\n".join(page.read_text(encoding="utf-8") for page in self.pages())
        for url in UPSTREAM_PROJECTS:
            with self.subTest(url=url):
                self.assertIn(url, text)


if __name__ == "__main__":
    unittest.main()
