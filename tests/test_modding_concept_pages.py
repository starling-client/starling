"""Focused discovery, nesting and link-check regressions for the modding book."""
import copy
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

from tools import modding_concept_pages as pages


class ConceptPages(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.src = Path(self.tmp.name) / "src"
        self.src.mkdir()
        self.book = {"sections": []}
        for part, folder in pages.FOLDERS.items():
            (self.src / folder).mkdir()
            self.book["sections"].append({"PartTitle": part})
        (self.src / "concepts/parts.md").write_text("# Parts\n\n[block](parts/b-block.md#block-parts)\n")
        (self.src / "concepts/parts").mkdir()
        (self.src / "concepts/parts/b-block.md").write_text("# Block parts\n\n## Landing\n")
        (self.src / "concepts/parts/a-item.md").write_text("# Item parts\n")
        self.book["sections"].insert(1, {"Chapter": dict(name="Parts", path="concepts/parts.md",
            source_path="concepts/parts.md", content=(self.src / "concepts/parts.md").read_text(),
            number=[1], parent_names=[], sub_items=[])})

    def expand(self, book=None):
        return pages.expand(copy.deepcopy(book or self.book), self.src)

    def test_a_folder_beside_a_page_gives_its_sub_chapters_in_file_name_order(self):
        parts = self.expand()["sections"][1]["Chapter"]
        self.assertEqual([c["Chapter"]["name"] for c in parts["sub_items"]], ["Item parts", "Block parts"])
        self.assertEqual([c["Chapter"]["number"] for c in parts["sub_items"]], [[1, 1], [1, 2]])
        self.assertEqual(parts["sub_items"][0]["Chapter"]["parent_names"], ["Parts"])

    def test_section_pages_marker_lists_the_sub_chapters(self):
        (self.src / "concepts/parts.md").write_text("# Parts\n\n<!-- section-pages -->\n")
        book = copy.deepcopy(self.book)
        book["sections"][1]["Chapter"]["content"] = (self.src / "concepts/parts.md").read_text()
        parts = self.expand(book)["sections"][1]["Chapter"]
        self.assertIn("- [Item parts](parts/a-item.md)\n- [Block parts](parts/b-block.md)", parts["content"])

    def test_nested_folders_nest_again(self):
        (self.src / "concepts/parts/a-item").mkdir()
        (self.src / "concepts/parts/a-item/tags.md").write_text("# Tag parts\n")
        item = self.expand()["sections"][1]["Chapter"]["sub_items"][0]["Chapter"]
        self.assertEqual([c["Chapter"]["path"] for c in item["sub_items"]], ["concepts/parts/a-item/tags.md"])

    def test_unlisted_page_joins_its_part_by_title_and_is_numbered(self):
        (self.src / "concepts/zeta.md").write_text("# Zeta\n\nLast by title.\n")
        (self.src / "concepts/alpha.md").write_text("# Alpha\n\n[block](parts/b-block.md#landing)\n")
        result = self.expand()
        flat = [(i.get("PartTitle"), i.get("Chapter", {}).get("name"), i.get("Chapter", {}).get("number"))
                for i in result["sections"]]
        self.assertEqual(flat, [("Concepts", None, None), (None, "Parts", [1]), (None, "Alpha", [2]),
                                (None, "Zeta", [3]), ("How-to", None, None)])

    def test_equal_titles_use_filename_order_not_directory_iteration_order(self):
        for name in ("zeta", "alpha"):
            (self.src / f"how-to/{name}.md").write_text("# Same title\n")
        glob = Path.glob

        def reversed_glob(path, *args, **kwargs):
            return iter(sorted(glob(path, *args, **kwargs), reverse=True))
        with patch.object(Path, "glob", reversed_glob):
            result = self.expand()
        found = [c["path"] for c in pages.chapters(result["sections"]) if c["path"].startswith("how-to/")]
        self.assertEqual(found, ["how-to/alpha.md", "how-to/zeta.md"])

    def test_design_page_dropped_into_concepts_fails(self):
        (self.src / "concepts/next.md").write_text("# Design: what comes next\n\nA proposal.\n")
        with self.assertRaisesRegex(AssertionError, "concepts/next.md: .* issues and PRs"):
            self.expand()

    def test_design_page_dropped_into_a_group_folder_fails(self):
        (self.src / "concepts/parts/c-plan.md").write_text("# Parts roadmap\n")
        with self.assertRaisesRegex(AssertionError, "concepts/parts/c-plan.md: .* issues and PRs"):
            self.expand()

    def test_page_outside_the_discovered_folders_must_be_listed(self):
        (self.src / "reference").mkdir()
        (self.src / "reference/orphan.md").write_text("# Orphan\n")
        with self.assertRaisesRegex(AssertionError, "doesn't render.*reference/orphan.md"):
            self.expand()

    def test_discovered_page_links_are_checked(self):
        (self.src / "how-to/broken.md").write_text("# Broken\n\n[gone](missing.md)\n")
        with self.assertRaisesRegex(AssertionError, "how-to/broken.md: missing link target"):
            self.expand()

    def test_missing_relative_target_or_anchor_fails(self):
        path = self.src / "concepts/parts/a-item.md"
        for target, error in [("missing.md", "missing link target"), ("b-block.md#absent", "missing anchor")]:
            path.write_text(f"# Item parts\n\n[x]({target})\n")
            with self.assertRaisesRegex(AssertionError, error):
                self.expand()

    def test_links_resolve_generated_chapter_and_anchor_without_a_file(self):
        book = copy.deepcopy(self.book)
        book["sections"].append({"Chapter": dict(name="Hook notes", path="reference/hook-notes.md",
            source_path=None, content="# Hook notes\n\n## Example\n", number=[9], parent_names=[], sub_items=[])})
        path = self.src / "concepts/parts/a-item.md"
        path.write_text("# Item parts\n\n[notes](../../reference/hook-notes.md#example)\n")
        self.expand(book)
        path.write_text(path.read_text().replace("#example", "#absent"))
        with self.assertRaisesRegex(AssertionError, "missing anchor"):
            self.expand(book)

    def test_reference_destinations_are_checked(self):
        for target, error in [("missing.md", "missing link target"), ("b-block.md#absent", "missing anchor")]:
            text = f"[x][ref]\n\n[ref]: <{target}>\n"
            with self.assertRaisesRegex(AssertionError, error):
                pages.validate(text, "concepts/parts/a-item.md", self.src, {})

    def test_source_root_relative_links_are_rejected(self):
        for url in ("/missing.html", "%2Fmissing.html"):
            for text in (f"[x]({url})", f"[x][ref]\n\n[ref]: {url}\n"):
                with self.assertRaisesRegex(AssertionError, "root-relative link"):
                    pages.validate(text, "concepts/parts/a-item.md", self.src, {})

    def test_stale_repository_and_book_site_urls_are_refused(self):
        for url in ("https://github.com/arismoko/horizon",
                    "https://github.com/horizon-rust/horizon/blob/main/docs/x.md",
                    "https://arismoko.github.io/sunrise/start/",
                    "https://horizon-rust.github.io/sunrise/",
                    "https://sunrise-org.github.io/sunrise/start/first-mod.html",
                    "https://github.com/horizon-rust/horizon-mods/releases/download/x/y.hmod",
                    "https://horizon-rust.github.io/horizon-mods/icons/coords_hud.png"):
            with self.subTest(url=url):
                with self.assertRaises(AssertionError):
                    pages.validate(f"[x]({url})", "concepts/parts/a-item.md", self.src, {})

    def test_upstream_projects_and_placeholders_keep_their_urls(self):
        for url in ("https://github.com/arismoko/horizon-machine-sample",
                    "https://github.com/horizon-rust/horizon-mixins/blob/main/docs/mixin-api.md",
                    "https://github.com/sunrise-org/sunrise-mods/releases/download/x/y.smod",
                    "https://sunrise-org.github.io/sunrise-mods/icons/coords_hud.png",
                    "https://playsunrise.net/docs/modding/start/first-mod",
                    "https://github.com/OWNER/REPO/releases/download/<tag>/<name>"):
            with self.subTest(url=url):
                pages.validate(f"[x]({url})", "concepts/parts/a-item.md", self.src, {})

    def test_rendered_stale_urls_are_refused(self):
        output = self.src / "html"
        path = output / "concepts/parts.html"
        path.parent.mkdir(parents=True)
        path.write_text('<a href="https://github.com/arismoko/horizon">x</a>')
        with self.assertRaises(AssertionError):
            pages.check_html(output, self.src / "concepts/..")

    def test_links_in_code_are_not_checked(self):
        text = (
            "```markdown\n[example](missing.md)\n```\n\n"
            "1. List item:\n\n"
            "    ```markdown\n"
            "    [x](not-a-book-page.md)\n"
            "    ```\n\n"
            "Inline `[x](gone.md)` code.\n"
        )
        pages.validate(text, "concepts/parts/a-item.md", self.src, {})

    def test_explicit_heading_ids_are_anchors(self):
        (self.src / "concepts/parts/b-block.md").write_text("# Block parts\n\n## Landing {#custom}\n")
        pages.validate("[x](b-block.md#custom)", "concepts/parts/a-item.md", self.src, {})
        with self.assertRaisesRegex(AssertionError, "missing anchor"):
            pages.validate("[x](b-block.md#landing-custom)", "concepts/parts/a-item.md", self.src, {})

    def test_missing_compiled_include_anchor_fails(self):
        (self.src / "code.rs").write_text("// ANCHOR: present\n// ANCHOR_END: present\n")
        text = "{{#include ../../code.rs:missing}}"
        with self.assertRaisesRegex(AssertionError, "missing include anchor"):
            pages.validate(text, "concepts/parts/a-item.md", self.src, {})

    def test_rendered_links_must_stay_in_the_book(self):
        output = self.src / "html"
        path = output / "concepts/parts.html"
        path.parent.mkdir(parents=True)
        for href, error in [("/missing.html", "root-relative link"),
                            ("%2Fmissing.html", "root-relative link"),
                            ("missing.html", "missing rendered link"),
                            ("../../../outside.html", "missing rendered link"),
                            ("#absent", "missing rendered anchor")]:
            path.write_text(f'<a href="{href}">x</a>')
            with self.assertRaisesRegex(AssertionError, error):
                pages.check_html(output, self.src / "concepts/..")

    def test_missing_h1_fails_expand(self):
        (self.src / "concepts/parts/no-h1.md").write_text("## Only h2\n\nBody.\n", encoding="utf-8")
        with self.assertRaisesRegex(AssertionError, r"concepts/parts/no-h1\.md: missing top-level `# ` heading"):
            self.expand()

    def test_comment_only_h1_fails_expand(self):
        (self.src / "concepts/parts/comment-h1.md").write_text(
            "<!--\n# Comment-only title\n-->\n\n## Subheading\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(AssertionError, r"concepts/parts/comment-h1\.md: missing top-level `# ` heading"):
            self.expand()

    def test_h1_after_quoted_fence_closed_without_space_is_found(self):
        (self.src / "concepts/parts/quoted.md").write_text(
            "> ```text\n> # Not a title\n>```\n\n# Quoted examples\n",
            encoding="utf-8",
        )
        self.expand()

    def test_bare_issue_reference_in_prose_is_rejected_while_code_destinations_and_heading_ids_pass(self):
        text = (
            "# Heading\n\n## Section {#1234}\n\nSee [section](#1234), [anchor](b-block.md#landing) and `#4776` and:\n\n"
            "```text\n#1234\n```\n\nTitled: [section](#1234 \"Section\") and [anchor](b-block.md#landing 'Landing').\n\n"
            "> ```text\n> #1234\n> ```\n\n- ```text\n  #1234\n  ```\n"
        )
        pages.validate(
            text,
            "concepts/parts/a-item.md",
            self.src,
            {(self.src / "concepts/parts/a-item.md").resolve(): text},
        )
        for token in ("#42", "#4776", "#100000"):
            with self.subTest(token=token):
                with self.assertRaisesRegex(AssertionError, rf"bare issue reference {re.escape(token)}"):
                    pages.validate(
                        f"# Heading\n\nAdded in {token}.\n",
                        "concepts/parts/a-item.md",
                        self.src,
                        {},
                    )
        with self.subTest("prose after a quoted fence that ends at the quote boundary"):
            with self.assertRaisesRegex(AssertionError, r"bare issue reference #42"):
                pages.validate("# Heading\n\n> ```text\n> code\nSee #42.\n", "concepts/parts/a-item.md", self.src, {})

    def test_inline_plain_rust_fence_and_uncompiled_include_are_rejected(self):
        examples_src = self.src.parent / "examples" / "src"
        examples_src.mkdir(parents=True, exist_ok=True)
        (examples_src / "sample.rs").write_text("// ANCHOR: ok\nfn ok() {}\n// ANCHOR_END: ok\n", encoding="utf-8")
        (self.src / "uncompiled.rs").write_text("// ANCHOR: ok\nfn ok() {}\n// ANCHOR_END: ok\n", encoding="utf-8")
        pages.validate(
            "# Heading\n\n```rust\n{{#include ../../../examples/src/sample.rs:ok}}\n```\n\n```rust,ignore\nlet x = 1;\n```\n",
            "concepts/parts/a-item.md",
            self.src,
            {},
        )
        with self.assertRaisesRegex(AssertionError, r"must target a compiled source"):
            pages.validate(
                "# Heading\n\n```rust\n{{#include ../../uncompiled.rs:ok}}\n```\n",
                "concepts/parts/a-item.md",
                self.src,
                {},
            )
        for fence in (
            "```rust\nlet x = 1;\n```",
            "```rs\nlet x = 1;\n```",
            "```rust,no_run\nlet x = 1;\n```",
            "> ```rust\n> let x = 1;\n> ```",
            "- ```rust\n  let x = 1;\n  ```",
            "1. Step:\n\n   ```rust\n   let x = 1;\n   ```",
            "> - ```rust\n>   let x = 1;\n>   ```",
        ):
            with self.subTest(fence=fence):
                with self.assertRaisesRegex(AssertionError, r"must use \{\{#include\}\}"):
                    pages.validate(f"# Heading\n\n{fence}\n", "concepts/parts/a-item.md", self.src, {})
        # A quoted text fence ends where its block quote does, so the top-level Rust fence after it is checked.
        with self.assertRaisesRegex(AssertionError, r"a-item\.md:5: non-ignored ```rust block"):
            pages.validate(
                "# Heading\n\n> ```text\n> output\n```rust\nlet x = 1;\n```\n",
                "concepts/parts/a-item.md",
                self.src,
                {},
            )
        pages.validate(
            "# Heading\n\n- ```rust,ignore\n  let x = 1;\n  ```\n- ```rust\n  {{#include ../../../examples/src/sample.rs:ok}}\n  ```\n",
            "concepts/parts/a-item.md",
            self.src,
            {},
        )


class BookSourceGuards(unittest.TestCase):
    def setUp(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.src = root / "src"
        self.html = root / "html"

    def write_rendered(self, name, body, *, write_source=True):
        if write_source:
            source = self.src / name
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("# Placeholder\n", encoding="utf-8")
        rendered = (self.html / name).with_suffix(".html")
        rendered.parent.mkdir(parents=True, exist_ok=True)
        rendered.write_text(
            f'<html><body><h1 class="menu-title">Sunrise Modding</h1><main>{body}</main></body></html>',
            encoding="utf-8",
        )

    def test_duplicate_rendered_h1_is_case_insensitive_and_ignores_formatting_and_chrome(self):
        self.write_rendered("concepts/parts.md", '<h1 id="registry-keys"><a class="header" href="#registry-keys">Registry <code>keys</code></a></h1>')
        self.write_rendered("reference/keys.md", '<h1 id="registry-keys"><a class="header" href="#registry-keys">Registry   Keys</a></h1>')
        with self.assertRaisesRegex(AssertionError, r"reference/keys\.md: duplicate H1 .*concepts/parts\.md"):
            pages.check_html(self.html, self.src)

    def test_duplicate_rendered_h1_catches_source_vs_generated_chapter(self):
        self.write_rendered("concepts/hook-notes.md", '<h1 id="hook-notes"><a class="header" href="#hook-notes">Hook notes</a></h1>')
        self.write_rendered("reference/hook-notes.md", '<h1 id="hook-notes"><a class="header" href="#hook-notes">Hook notes</a></h1>', write_source=False)
        with self.assertRaisesRegex(AssertionError, r"reference/hook-notes\.md: duplicate H1 'Hook notes' \(also in concepts/hook-notes\.md\)"):
            pages.check_html(self.html, self.src)

    def test_duplicate_rendered_h1_discovers_any_generated_reference_chapter(self):
        self.write_rendered("concepts/parts.md", '<h1 id="future-ref"><a class="header" href="#future-ref">Future reference</a></h1>')
        self.write_rendered("reference/generated-future.md", '<h1 id="future-ref"><a class="header" href="#future-ref">Future reference</a></h1>', write_source=False)
        with self.assertRaisesRegex(AssertionError, r"reference/generated-future\.md: duplicate H1 'Future reference' \(also in concepts/parts\.md\)"):
            pages.check_html(self.html, self.src)

    def test_menu_title_chrome_and_distinct_titles_pass(self):
        self.write_rendered("concepts/parts.md", '<h1>Parts</h1>')
        self.write_rendered("reference/keys.md", '<pre><code># Parts</code></pre><h1>Registry keys</h1>')
        pages.check_html(self.html, self.src)

    def test_nested_h1_does_not_mask_chapter_title_or_cause_false_duplicate(self):
        self.write_rendered("concepts/parts.md", '<blockquote><h1>Example</h1></blockquote><h1>Parts</h1>')
        self.write_rendered("concepts/example.md", '<h1>Example</h1>')
        pages.check_html(self.html, self.src)

    def test_nested_h1_does_not_hide_real_duplicate_chapter_title(self):
        self.write_rendered("concepts/parts.md", '<blockquote><h1>Example</h1></blockquote><h1>Parts</h1>')
        self.write_rendered("reference/parts.md", '<ul><li><h1>Item</h1></li></ul><h1>Parts</h1>')
        with self.assertRaisesRegex(AssertionError, r"reference/parts\.md: duplicate H1 'Parts' \(also in concepts/parts\.md\)"):
            pages.check_html(self.html, self.src)

    def test_missing_rendered_h1_fails_check_html(self):
        self.write_rendered("concepts/parts.md", '<h2>Only subheading</h2><p>Body</p>')
        with self.assertRaisesRegex(AssertionError, r"concepts/parts\.md: missing rendered H1 heading"):
            pages.check_html(self.html, self.src)

    def test_valueless_class_attribute_on_h1_does_not_crash_and_still_checks_duplicates(self):
        self.write_rendered("concepts/parts.md", '<h1 class>Parts</h1>')
        self.write_rendered("reference/parts.md", '<h1>Parts</h1>')
        with self.assertRaisesRegex(AssertionError, r"reference/parts\.md: duplicate H1 'Parts' \(also in concepts/parts\.md\)"):
            pages.check_html(self.html, self.src)


if __name__ == "__main__":
    unittest.main()
