"""docs/security.md's references resolve: the files, sections and links it names exist.

Its issue numbers once pointed at another tracker and silently came to mean unrelated issues here, so the page
cites none: it names files and sections, which this test can check.
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOC = ROOT / "docs" / "security.md"
HEADING = re.compile(r"^#+\s+(.+?)\s*$", re.M)


def sections(path):
    """A page's headings and the bold lead-ins of its bullets ("- **Signed updates.**")."""
    text = path.read_text(encoding="utf-8")
    leads = re.findall(r"^\s*- \*\*(.+?)[.:]?\*\*", text, re.M)
    return set(HEADING.findall(text)) | set(leads)


def anchor(heading):
    return re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-")


class SecurityDocRefsTests(unittest.TestCase):
    text = DOC.read_text(encoding="utf-8")

    def test_cites_no_bare_issue_numbers(self):
        self.assertEqual(re.findall(r"(?<![\w/&])#\d+\b", self.text), [])

    def test_named_repository_paths_exist(self):
        tops = {p.name for p in ROOT.iterdir()}
        for path in sorted(set(re.findall(r"`([\w.\-]+(?:/[\w.\-]+)+)`", self.text))):
            if path.split("/", 1)[0] in tops:
                with self.subTest(path=path):
                    self.assertTrue((ROOT / path).exists(), path)

    def test_named_sections_exist(self):
        # `docs/release-flow.md, "Signed checksums"` names a section of another page; `"Signed updates", below`
        # one of this page.
        for page, name in re.findall(r"(docs/[\w\-/]+\.md),\s+\"([^\"]+)\"", self.text):
            with self.subTest(page=page, section=name):
                self.assertIn(name, sections(ROOT / page))
        own = sections(DOC)
        for name in re.findall(r"\"([^\"]+)\",\s+(?:below|above)", self.text):
            with self.subTest(section=name):
                self.assertIn(name, own)

    def test_relative_links_resolve(self):
        for target in re.findall(r"\]\(([^)\s]+)\)", self.text):
            if "://" in target or target.startswith("mailto:"):
                continue
            path, _, fragment = target.partition("#")
            page = (DOC.parent / path) if path else DOC
            with self.subTest(target=target):
                self.assertTrue(page.exists(), target)
                if fragment:
                    self.assertIn(fragment, {anchor(h) for h in sections(page)}, target)


if __name__ == "__main__":
    unittest.main()
