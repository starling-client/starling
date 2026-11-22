"""The API's file list stands for git's diff only when it can't differ from it (tools/ci/pr_files.py)."""
import contextlib
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from tools.ci import pr_files
from tools.ci.rust_changes import changed_entries, listed_entries

BASE, HEAD = "b" * 40, "c" * 40
ENV = {"GITHUB_REPOSITORY": "o/r", "BASE_SHA": BASE, "HEAD_SHA": HEAD}


def api(files, base=BASE):
    """A stand-in for GitHub's comparison of the run's base and head: every file on its first page."""
    def get(path):
        assert path == f"compare/{BASE}...{HEAD}?per_page=1", path
        return {"base_commit": {"sha": base}, "files": files}
    return get


class PrFilesTests(unittest.TestCase):
    def run_main(self, get, changed, **env):
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.dict(os.environ, dict(ENV, CHANGED_FILES=str(changed), **env)), \
                mock.patch.object(pr_files, "get", side_effect=get), \
                contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()):
            listing = Path(directory, "files")
            pr_files.main(str(listing))
            count = int(dict(line.split("=", 1) for line in out.getvalue().splitlines()).get("entries", 0))
            return out.getvalue(), listed_entries(listing, count) if listing.exists() else None

    def test_the_api_list_matches_gits_diff_without_renames(self):
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, "-c", "user.name=t", "-c", "user.email=t@t",
                                                "-c", "commit.gpgsign=false", *args], text=True).strip()
            git("init", "-q")
            for name in ["modified.md", "deleted.md", "old.rs", "kept.txt"]:
                Path(directory, name).write_text(f"{name}\n" * 20)
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            Path(directory, "modified.md").write_text("changed\n")
            Path(directory, "deleted.md").unlink()
            Path(directory, "docs").mkdir()
            Path(directory, "old.rs").rename(Path(directory, "docs/new.rs"))
            Path(directory, "added.md").write_text("added\n")
            Path(directory, "copy.txt").write_text("kept.txt\n" * 20)
            git("add", "-A")
            git("commit", "-qm", "head")
            previous = Path.cwd()
            try:
                os.chdir(directory)
                expected = changed_entries(base, "HEAD")
            finally:
                os.chdir(previous)
        # What GitHub lists for the same change: renames folded, a copy marked as one.
        files = [{"status": "modified", "filename": "modified.md"}, {"status": "removed", "filename": "deleted.md"},
                 {"status": "renamed", "filename": "docs/new.rs", "previous_filename": "old.rs"},
                 {"status": "added", "filename": "added.md"}, {"status": "copied", "filename": "copy.txt"}]
        out, listed = self.run_main(api(files), len(files))
        self.assertEqual(out, "listed=true\nentries=6\n")
        self.assertEqual(sorted(listed), sorted(expected))

    def test_the_list_is_read_for_this_runs_commits_only(self):
        # The comparison names both commit ids: a branch that moves and comes back between reads can't change it.
        files = [{"status": "added", "filename": f"docs/{n}.md"} for n in range(pr_files.API_LIMIT - 1)]
        out, listed = self.run_main(api(files), len(files))
        self.assertEqual(out, f"listed=true\nentries={pr_files.API_LIMIT - 1}\n")
        self.assertEqual(len(listed), pr_files.API_LIMIT - 1)

    def test_a_same_count_branch_round_trip_cannot_replace_the_pinned_listing(self):
        paths = []
        def get(path):
            paths.append(path)
            if path.startswith("pulls/"):
                # The branch briefly shows only a doc edit, then returns to the original head.
                return [{"status": "modified", "filename": "docs/setup.md"}] if "/files" in path else {
                    "head": {"sha": HEAD}, "base": {"sha": BASE}, "changed_files": 1,
                }
            return api([{"status": "modified", "filename": "src/lib.rs"}])(path)
        out, listed = self.run_main(get, 1)
        self.assertEqual((out, listed), ("listed=true\nentries=1\n", [("M", "src/lib.rs")]))
        self.assertEqual(paths, [f"compare/{BASE}...{HEAD}?per_page=1"])

    def test_a_record_with_no_usable_path_or_a_repeated_path_falls_back(self):
        for files in ([{"status": "modified", "filename": "docs/../src/lib.rs"}],
                      [{"status": "modified", "filename": "./src/lib.rs"}],
                      [{"status": "renamed", "filename": "docs/new.md", "previous_filename": None}],
                      [{"status": "modified", "filename": "docs/a.md"}, {"status": "removed", "filename": "docs/a.md"}]):
            with self.subTest(files=files):
                self.assertEqual(self.run_main(api(files), len(files)), ("listed=false\n", None))

    def test_any_doubt_falls_back_to_the_history(self):
        one = [{"status": "modified", "filename": "docs/setup.md"}]
        for name, get, changed in [
            ("a count other than the event's", api(one), 2),
            ("an empty event diff", api([]), 0),
            ("a missing list", lambda _: {"base_commit": {"sha": BASE}}, 1),
            ("a truncated path", api([{"status": "added", "filename": "docs/\0setup.md"}]), 1),
            ("the comparison's limit", api(one), pr_files.API_LIMIT),
            ("a comparison of another base", api(one, base="n" * 40), 1),
            ("a status git has no exact twin for", api([{"status": "changed", "filename": "docs/a.md"}]), 1),
            ("an API error", mock.Mock(side_effect=OSError("HTTP 502")), 1),
        ]:
            with self.subTest(name):
                self.assertEqual(self.run_main(get, changed), ("listed=false\n", None))
        # A name instead of a commit id could move between this run's reads.
        for name in ("BASE_SHA", "HEAD_SHA"):
            with self.subTest(name):
                self.assertEqual(self.run_main(api(one), 1, **{name: "main"}), ("listed=false\n", None))


if __name__ == "__main__":
    unittest.main()
