"""tools/book_quickstart.py: which blocks of a page run, and how a failing one is named."""
import os
from pathlib import Path
import tempfile
import unittest

from tools import book_quickstart

PAGE = """# A page

```sh
echo one > one.txt
cd "$(mktemp -d -p .)"
```

```text
not a command
```

<!-- quickstart-test: skip, needs the game -->
```sh
exit 9
```

```sh
test -f ../one.txt
echo here > here.txt
false
```

```sh
echo never > never.txt
```
"""


class BookQuickstart(unittest.TestCase):
    def test_sh_blocks_are_found_with_their_line_and_skip_reason(self):
        self.assertEqual(
            [(line, skip) for line, _, skip, _ in book_quickstart.blocks(PAGE)],
            [(3, None), (13, "needs the game"), (17, None), (23, None)])

    def test_blocks_share_one_shell_and_the_first_failing_block_is_named(self):
        with tempfile.TemporaryDirectory() as folder:
            page = Path(folder, "page.md")
            page.write_text(PAGE)
            with open(Path(folder, "log"), "w") as log:
                problem = book_quickstart.run_page(page, folder, dict(os.environ), log)
            # The third block runs in the folder the first moved to, fails on its last line, and the fourth never runs.
            self.assertTrue(problem.startswith(f"{page}:17: this block failed"), problem)
            self.assertEqual([path.parent.parent for path in Path(folder).rglob("here.txt")], [Path(folder)])
            self.assertTrue(Path(folder, "one.txt").exists())
            self.assertEqual(list(Path(folder).rglob("never.txt")), [])

    def test_a_shell_that_ends_early_with_success_names_the_block_it_skipped(self):
        with tempfile.TemporaryDirectory() as folder:
            page = Path(folder, "page.md")
            page.write_text("```sh\nexit 0\n```\n\n```sh\ntrue\n```\n")
            with open(Path(folder, "log"), "w") as log:
                problem = book_quickstart.run_page(page, folder, dict(os.environ), log)
            self.assertTrue(problem.startswith(f"{page}:5: this block never started"), problem)

    def test_a_block_marked_in_a_folder_runs_in_that_folder_of_the_run(self):
        with tempfile.TemporaryDirectory() as scratch:
            Path(scratch, "earlier/project").mkdir(parents=True)
            Path(scratch, "page").mkdir()
            page = Path(scratch, "page.md")
            page.write_text("<!-- quickstart-test: in earlier/project -->\n```sh\ntouch made\n```\n")
            with open(Path(scratch, "log"), "w") as log:
                problem = book_quickstart.run_page(page, Path(scratch, "page"), dict(os.environ), log)
            self.assertIsNone(problem)
            self.assertTrue(Path(scratch, "earlier/project/made").exists())


if __name__ == "__main__":
    unittest.main()
