"""The surface report step: which base it compares, when it passes, when it fails."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("surface_report", ROOT / "tools/ci/surface_report.py")
surface_report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(surface_report)

MERGE = "tree t\nparent aaaa\nparent bbbb\nauthor x\n\nMerge bbbb into aaaa\n"
SINGLE = "tree t\nparent aaaa\nauthor x\n\nA commit\n"


class BaseCommitTests(unittest.TestCase):
    def test_a_pull_request_compares_against_its_merge_commits_first_parent(self):
        sha, _ = surface_report.base_commit("pull_request", "", lambda *a: MERGE)
        self.assertEqual(sha, "aaaa")
        sha, _ = surface_report.base_commit("pull_request", "", lambda *a: SINGLE)
        self.assertIsNone(sha)

    def test_a_dispatch_compares_against_its_test_merges_first_parent(self):
        sha, _ = surface_report.base_commit("workflow_dispatch", "12", lambda *a: MERGE)
        self.assertEqual(sha, "aaaa")
        # A dispatch without `pr` checks main's own commit: there is no base to compare.
        sha, _ = surface_report.base_commit("workflow_dispatch", "", lambda *a: MERGE)
        self.assertIsNone(sha)

    def test_a_push_has_no_base(self):
        sha, why = surface_report.base_commit("push", "", None)
        self.assertIsNone(sha)
        self.assertEqual(why, "no base for a push run")


class GateTests(unittest.TestCase):
    def test_the_tracked_switch_is_read_and_must_be_a_boolean(self):
        self.assertIs(surface_report.gate(ROOT / "tools/ci/surface.toml"), True)
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory, "surface.toml")
            config.write_text("[surface]\ngate = true\n")
            self.assertIs(surface_report.gate(config), True)
            config.write_text("[surface]\ngate = \"yes\"\n")
            with self.assertRaises(SystemExit):
                surface_report.gate(config)


class FetchTests(unittest.TestCase):
    """fetch_commit(): shallow only where the checkout already is."""

    def calls(self, shallow, present):
        made = []

        def git(*args):
            made.append(args)
            if args[0] == "rev-parse":
                return "true\n" if shallow else "false\n"
            if args[0] == "cat-file" and not present:
                raise subprocess.CalledProcessError(128, ["git", *args])
            return ""

        surface_report.fetch_commit("abcd", 2, git)
        return [args for args in made if args[0] == "fetch"]

    def test_a_shallow_checkout_fetches_shallow(self):
        self.assertEqual(self.calls(shallow=True, present=False),
                         [("fetch", "-q", "--no-tags", "--depth", "2", "origin", "abcd")])

    def test_a_full_clone_is_never_fetched_shallow(self):
        self.assertEqual(self.calls(shallow=False, present=True), [])
        self.assertEqual(self.calls(shallow=False, present=False), [("fetch", "-q", "--no-tags", "origin", "abcd")])


class ArchiveTests(unittest.TestCase):
    def test_a_base_archive_keeps_sources_for_members_outside_crates(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, base = Path(directory, "repo"), Path(directory, "base")
            base.mkdir()
            member = "docs/examples"
            workspace = f'[workspace]\nmembers = ["{member}"]\n'
            manifest = '[package]\nname = "example-package"\n[lib]\nname = "example_library"\n'
            for name, content in {
                "Cargo.toml": workspace,
                f"{member}/Cargo.toml": manifest,
                f"{member}/src/lib.rs": "pub struct Example;\n",
                f"{member}/src/nested.rs": "pub struct Nested;\n",
                "crates/game/src/lib.rs": "",
                "tools/layering/config.toml": "",
            }.items():
                file = repo / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(content)
            with mock.patch.object(surface_report, "ROOT", repo):
                surface_report.git("init", "-q")
                surface_report.git("add", ".")
                surface_report.git("-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                                   "-c", "commit.gpgsign=false", "commit", "-qm", "archive fixture")
                surface_report.unpack("HEAD", base)
            self.assertEqual((base / "Cargo.toml").read_text(), workspace)
            self.assertEqual((base / member / "Cargo.toml").read_text(), manifest)
            self.assertEqual((base / member / "src/lib.rs").read_text(), "pub struct Example;\n")
            self.assertEqual((base / member / "src/nested.rs").read_text(), "pub struct Nested;\n")


@unittest.skipUnless(shutil.which("bash"), "needs bash for the stub tool")
class ReportTests(unittest.TestCase):
    """report() against a stub sunrise-stage whose behaviour each test sets."""

    def run_report(self, base_export=0, diff_exit=0, gate_on=False, head_export=0, diff_err="",
                   with_internals=False, own_export=None):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            tool = work / "sunrise-stage"
            tool.write_text(f"""#!/bin/bash
echo "$@" >> {work}/calls
if [ "$1" = surface ]; then
  case "$2" in *base) [ {base_export} = 0 ] || {{ echo 'unclassified: x' >&2; echo 'sunrise-stage surface: unknown key `access`' >&2; exit 1; }};; esac
  case "$2" in *head) [ {head_export} = 0 ] || {{ echo 'sunrise-stage surface: a.rs: parse error' >&2; exit 1; }};; esac
  echo '{{}}' > "$3"; exit 0
fi
[ "$1" = surface-catalogs ] && exit 0
echo "Surface: report"; [ -z "{diff_err}" ] && [ {diff_exit} != 0 ] && echo "breaking change without a change note: fn:a" >&2
[ -n "{diff_err}" ] && echo "sunrise-stage: $2: {diff_err}" >&2
exit {diff_exit}
""")
            tool.chmod(0o755)
            # The base's own tool, when a test gives one: it exports with `own_export` as its exit status.
            own = None
            if own_export is not None:
                own_tool = work / "own-stage"
                own_tool.write_text(f"""#!/bin/bash
echo "own $@" >> {work}/calls
[ "$#" = 3 ] || exit 2
[ {own_export} = 0 ] || {{ echo 'sunrise-stage surface: base.rs: older form' >&2; exit 1; }}
echo '{{}}' > "$3"
""")
                own_tool.chmod(0o755)
                own = lambda: str(own_tool)  # noqa: E731
            (work / "base").mkdir()
            code, text = surface_report.report(str(tool), work / "base", work / "head", work, gate_on,
                                               with_internals=with_internals, own_base_tool=own)
            return code, text, (work / "calls").read_text()

    def test_the_bases_own_export_is_augmented_with_the_heads_catalog_reader(self):
        code, text, calls = self.run_report(own_export=0, with_internals=True)
        self.assertEqual(code, 0)
        self.assertIn("The base was exported by its own sunrise-stage.\n", text)
        self.assertIn("Surface: report\n", text)
        own, catalogs = (next(call.split() for call in calls.splitlines() if call.startswith(prefix))
                         for prefix in ("own ", "surface-catalogs "))
        self.assertEqual(own[:2], ["own", "surface"])
        self.assertEqual(catalogs[1:], [*own[2:], "--with-internals"])
        self.assertLess(calls.index("surface-catalogs "), calls.index("surface-diff "))

    def test_an_unreadable_base_fails_naming_the_selected_exporter(self):
        code, text, calls = self.run_report(own_export=1)
        self.assertEqual(code, 1)
        self.assertEqual(text, "No surface report: the base's own sunrise-stage could not export its tree "
                               "(sunrise-stage surface: base.rs: older form).\n")
        self.assertNotIn("surface-diff", calls)
        code, text, calls = self.run_report(base_export=1)
        self.assertEqual(code, 1)
        self.assertEqual(text, "No surface report: the head's sunrise-stage cannot read the base tree "
                               "(sunrise-stage surface: unknown key `access`).\n")
        self.assertNotIn("surface-diff", calls)

    def test_a_base_surface_of_another_schema_fails_with_one_line(self):
        code, text, _ = self.run_report(diff_exit=2, diff_err="surface schema 1, this tool reads 2")
        self.assertEqual(code, 1)
        self.assertTrue(text.startswith("No surface report: the head's sunrise-stage cannot read the base surface "), text)
        self.assertIn("surface schema 1, this tool reads 2", text)

    def test_the_schema_reset_checks_only_the_audited_base_and_still_gates_later_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            base, head = work / "old", work / "new"
            reset = head / surface_report.SCHEMA_RESET
            reset.parent.mkdir(parents=True)
            audit = {
                "checkpoint": "checked",
                "plain": {"old": "old-hash", "new": "checkpoint-hash"},
                "diff": {"old": "old-with-catalogs", "new": "checkpoint-with-catalogs",
                         "changes": [{"id": "fn:removed", "class": "breaking", "notes": ["native-only.toml"]}]},
            }
            reset.write_text(json.dumps(audit))

            def export(command, **_kwargs):
                tool, action, root, path, *flags = command
                if action == "surface-catalogs":
                    self.assertEqual(Path(root), work / "schema-reset")
                    self.assertEqual(json.loads(Path(path).read_text())["hash"], "checkpoint-hash")
                    self.assertEqual(flags, [])
                    Path(path).write_text(json.dumps({"schema": 3, "hash": "checkpoint-with-loops"}))
                    return subprocess.CompletedProcess(command, 0, "", "")
                self.assertEqual(flags, ["--with-loops"] if root == str(head) else [])
                schema, digest = {
                    ("old", "old-tool"): (2, "old-hash"),
                    ("new", "stage"): (3, "some-new-change"),
                    ("schema-reset", "checkpoint-tool"): (3, "checkpoint-hash"),
                }[(Path(root).name, tool)]
                Path(path).write_text(json.dumps({"schema": schema, "hash": digest}))
                return subprocess.CompletedProcess(command, 0, "", "")

            with (mock.patch.object(surface_report.subprocess, "run", side_effect=export),
                  mock.patch.object(surface_report, "unpack") as unpack,
                  mock.patch.object(surface_report, "tree_tool", return_value="checkpoint-tool") as tree_tool,
                  mock.patch.object(surface_report, "compare", return_value=(1, "later change refused")) as compare):
                code, text = surface_report.report("stage", base, head, work, True, own_base_tool=lambda: "old-tool")
                self.assertEqual(code, 1)
                self.assertIn("Audited schema 2 → 3 reset", text)
                self.assertIn("later change refused", text)
                unpack.assert_called_once_with("checked", work / "schema-reset", ())
                tree_tool.assert_called_once_with(work / "schema-reset", work)
                self.assertEqual(compare.call_args.args[1:5], (work / "schema-reset", head, work, True))
                self.assertEqual(json.loads((work / "base.json").read_text())["hash"], "checkpoint-with-loops")

                audit["plain"]["old"] = "unrelated-base"
                reset.write_text(json.dumps(audit))
                compare.reset_mock()
                code, text = surface_report.report("stage", base, head, work, True, own_base_tool=lambda: "old-tool")
                self.assertEqual(code, 1)
                self.assertIn("is not the audited schema-reset base", text)
                compare.assert_not_called()

    def test_this_tree_failing_to_export_fails_the_step(self):
        for base_export in (0, 1):
            code, text, calls = self.run_report(head_export=1, base_export=base_export)
            self.assertEqual(code, 1)
            self.assertEqual(text, "The surface export of this tree failed: sunrise-stage surface: a.rs: parse error.\n")
            self.assertNotIn("surface-diff", calls)

    def test_the_diff_decides_and_the_gate_is_passed_on(self):
        code, text, calls = self.run_report()
        self.assertEqual(code, 0)
        self.assertIn("CI does not compare listed internals", text)
        self.assertIn("--with-internals", text)
        self.assertIn("Surface: report\n", text)
        self.assertNotIn("--gate", calls)
        self.assertNotIn("--with-internals", calls)
        exports = [call.split() for call in calls.splitlines() if call.startswith("surface ")]
        self.assertEqual(len(exports), 2)
        for call in exports:
            self.assertIn("--with-loops", call)
        self.assertIn("--notes ", calls)
        code, text, calls = self.run_report(diff_exit=1, gate_on=True)
        self.assertEqual(code, 1)
        self.assertIn("fn:a", text)
        self.assertIn("--gate", calls)
        # An unreadable note (exit 2) fails too.
        code, _, _ = self.run_report(diff_exit=2)
        self.assertEqual(code, 1)

    def test_the_audit_exports_internals_on_both_sides(self):
        code, text, calls = self.run_report(with_internals=True)
        self.assertEqual((code, text), (0, "Surface: report\n"))
        exports = [call for call in calls.splitlines() if call.startswith("surface ")]
        self.assertEqual(len(exports), 2)
        for call in exports:
            self.assertIn("--with-internals", call.split())


@unittest.skipUnless(os.environ.get("SUNRISE_STAGE_TOOL"), "set SUNRISE_STAGE_TOOL to test the real gate")
class EngineCatalogGateTests(unittest.TestCase):
    ITEMS = [
        {"id": "loop:entities.tick", "kind": "loop", "hash": "loop-1", "entry": "Game::tick_entities",
         "generic": "tick_entities_with", "slot": "TickSlot", "side": "server"},
        {"id": "internal:fn:sunrise_client::game::Game::tick [feature = \"fixture\"]",
         "kind": "internal", "hash": "internal-1", "signature": "fn tick(&self)",
         "signature_hash": "old-internal"},
    ]

    def compare(self, old, new, *, notes=()):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for name, records in (("base", old), ("head", new)):
                (work / f"{name}.json").write_text(json.dumps({"schema": 3, "hash": name, "engine_items": records}))
            if notes:
                changes = work / "head" / surface_report.NOTES
                changes.mkdir(parents=True)
                (changes / "engine-change.toml").write_text(f'ids = {json.dumps(notes)}\nwhy = "Use the new sites."\n')
            return surface_report.compare(os.environ["SUNRISE_STAGE_TOOL"], work / "base", work / "head", work, True,
                                          with_internals=True)

    def test_undeclared_loop_removal_fails_and_declared_removal_passes(self):
        loop = self.ITEMS[0]
        record = loop["id"]
        code, text = self.compare([loop], [])
        self.assertEqual(code, 1, text)
        self.assertIn(f"- {record} (removed)", text)
        self.assertIn(f"breaking change without a change note: {record}", text)
        code, text = self.compare([loop], [], notes=[record])
        self.assertEqual(code, 0, text)
        self.assertNotIn("breaking change without a change note:", text)

    def test_undeclared_internal_removal_is_advisory_and_listed(self):
        internal = self.ITEMS[1]
        code, text = self.compare([internal], [])
        self.assertEqual(code, 0, text)
        self.assertIn("Surface: advisory (0 breaking, 0 conditional, 0 additive, 1 advisory;", text)
        self.assertIn("Listed internals (advisory): 0 added, 1 removed, 0 changed", text)
        self.assertIn(f"- {internal['id']} (removed)", text)
        self.assertNotIn("breaking change without a change note:", text)

    def test_additions_pass_and_each_kind_has_a_count(self):
        code, text = self.compare([], self.ITEMS)
        self.assertEqual(code, 0, text)
        self.assertIn("0 breaking, 0 conditional, 1 additive, 1 advisory", text)
        for kind in ("Published loops", "Listed internals (advisory)"):
            self.assertIn(f"{kind}: 1 added, 0 removed, 0 changed", text)

    def test_changed_contracts_keep_their_ids(self):
        loop, internal = self.ITEMS
        code, text = self.compare(self.ITEMS, [
            dict(loop, hash="loop-2", slot="NewTickSlot"),
            dict(internal, hash="internal-2", signature="fn tick(&mut self)", signature_hash="new-internal"),
        ])
        self.assertEqual(code, 1, text)
        self.assertIn("1 breaking, 0 conditional, 0 additive, 1 advisory", text)
        for item, detail in zip(self.ITEMS, ("`TickSlot` → `NewTickSlot`", "`fn tick(&self)` → `fn tick(&mut self)`")):
            self.assertIn(f"- {item['id']} (changed)", text)
            self.assertIn(detail, text)
        self.assertNotIn("(removed)", text)
        self.assertNotIn("(added)", text)


class SummaryTests(unittest.TestCase):
    def size(self, out: str) -> int:
        return len(out.encode())

    def test_a_long_report_is_cut_at_a_line_end(self):
        text = "".join(f"- fn:item{i} (added)\n" for i in range(20_000))
        out = surface_report.summary(text)
        self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
        self.assertRegex(out, r"\(added\)\n… cut here; \d+ more lines in the step log\n```\n$")
        self.assertEqual(surface_report.summary("short\n"), "### Mod API surface\n\n```text\nshort\n```\n")

    def test_a_cut_inside_a_multi_byte_character_keeps_whole_characters(self):
        # "é" is two bytes, so the byte budget is half the characters and lands inside one.
        for pad in ("", "x"):
            out = surface_report.summary(pad + "é" * 200_000 + "\nnext\n")
            self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
            self.assertGreater(self.size(out), surface_report.SUMMARY_LIMIT - 200)
            self.assertNotIn("\ufffd", out)
            self.assertIn("é\n… cut here; 2 more lines in the step log\n", out)

    def test_one_line_longer_than_the_cap_is_cut(self):
        out = surface_report.summary("a" * 300_000 + "\n")
        self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
        self.assertGreater(out.count("a"), 199_000)
        self.assertTrue(out.endswith("a\n… cut here; 1 more line in the step log\n```\n"))


class ScopedOs:
    """`os` for the module under test, with its own environment."""

    def __init__(self, environ: dict[str, str]):
        self.environ = environ

    def __getattr__(self, name):
        return getattr(os, name)


class MainTests(unittest.TestCase):
    """main(): the base it fetches, what it prints, what it writes to the job summary."""

    def run_main(self, event="pull_request", pr="", argv=(), tool="/no/tool", **patches):
        with tempfile.TemporaryDirectory() as directory:
            summary_file = Path(directory) / "summary.md"
            env = {"SUNRISE_STAGE_TOOL": tool, "EVENT": event, "PR": pr,
                   "GITHUB_STEP_SUMMARY": str(summary_file), "RUNNER_TEMP": directory}
            # The module reads its own environment: the fixture never writes the process's.
            with mock.patch.object(surface_report, "os", ScopedOs({**os.environ, **env})), \
                    contextlib.redirect_stdout(io.StringIO()) as out, contextlib.ExitStack() as stack:
                for name, value in patches.items():
                    stack.enter_context(mock.patch.object(surface_report, name, value))
                code = surface_report.main(argv)
            return code, out.getvalue(), summary_file.read_text()

    def test_an_unfetchable_base_reports_and_passes(self):
        def git(*args):
            raise subprocess.CalledProcessError(128, ["git", *args])

        code, out, summary = self.run_main(git=git)
        self.assertEqual(code, 0)
        self.assertTrue(out.startswith("No surface report: the base could not be fetched: "), out)
        self.assertIn("No surface report", summary)

    def test_a_dispatch_whose_checkout_is_not_a_merge_reports_and_passes(self):
        unpacked = []
        code, _, _ = self.run_main("workflow_dispatch", "7", git=lambda *a: SINGLE,
                                   unpack=lambda sha, into: unpacked.append(sha))
        # No base is selected, so nothing is archived, and the report passes.
        self.assertEqual((code, unpacked), (0, []))

    def test_a_report_is_printed_written_and_its_code_returned(self):
        fetched = []
        code, out, summary = self.run_main(
            base_commit=lambda *a: ("abcd", ""),
            unpack=lambda sha, into: fetched.append(sha),
            report=lambda *a, **kw: (1, "Surface: breaking\n"),
            # The checkout's own change notes may carry `landed`; LandedTests covers that pass.
            landed_notes=lambda head, base: {})
        self.assertEqual((code, fetched), (1, ["abcd"]))
        self.assertRegex(out, r"^Surface: breaking\nBase abcd; fetch \d+\.\ds, total \d+\.\ds\n$")
        self.assertTrue(summary.startswith("### Mod API surface\n\n```text\nSurface: breaking\nBase abcd"))

    def test_local_audit_uses_the_supplied_trees_without_fetching(self):
        with mock.patch.object(surface_report, "report", return_value=(0, "Surface: advisory\n")) as report, \
             mock.patch.object(surface_report, "base_commit") as find_base:
            code, out, _ = self.run_main(argv=["--with-internals", "--trees", "before", "after"])
        self.assertEqual((code, out), (0, "Surface: advisory\n"))
        self.assertEqual(report.call_args.args[1:3], (Path("before"), Path("after")))
        self.assertTrue(report.call_args.kwargs["with_internals"])
        find_base.assert_not_called()

    def local_audit(self, *flags, readable=True):
        compared = []

        def stage(command, **_):
            tool, action, first, second, *_ = command
            if action == "surface":
                if not readable and (tool, first) == ("head-stage", "before"):
                    return subprocess.CompletedProcess(command, 1, "", "unsupported older source")
                # Both tools can read the same source but generate different records for it.
                Path(second).write_text(json.dumps({"tree": first, "wire": tool == "base-stage"}))
            elif action == "surface-diff":
                compared.extend(json.loads(Path(path).read_text()) for path in (first, second))
            return subprocess.CompletedProcess(command, 0, "", "")

        tree_tool = mock.Mock(return_value="base-stage")
        with mock.patch.object(surface_report.subprocess, "run", side_effect=stage):
            code, out, _ = self.run_main(argv=["--trees", "before", "after", *flags], tool="head-stage",
                                         tree_tool=tree_tool)
        return code, out, tree_tool, compared

    def test_local_audit_does_not_build_without_opt_in(self):
        code, out, tree_tool, compared = self.local_audit()
        self.assertEqual(code, 0, out)
        tree_tool.assert_not_called()
        self.assertEqual(compared, [{"tree": "before", "wire": False}, {"tree": "after", "wire": False}])
        code, out, tree_tool, compared = self.local_audit(readable=False)
        self.assertEqual(code, 1, out)
        self.assertIn("unsupported older source", out)
        tree_tool.assert_not_called()
        self.assertEqual(compared, [])

    def test_requested_base_tool_preserves_the_record_even_if_the_head_can_export_the_base(self):
        code, out, tree_tool, compared = self.local_audit("--build-base-tool")
        self.assertEqual(code, 0, out)
        self.assertEqual(compared, [{"tree": "before", "wire": True}, {"tree": "after", "wire": False}])
        tree_tool.assert_called_once()
        self.assertEqual(tree_tool.call_args.args[0], Path("before"))

    @unittest.skipUnless(shutil.which("bash"), "needs bash for the stub tools")
    def test_a_commit_both_comparisons_need_is_built_once(self):
        """A landed note may name the CI base itself: the base comparison and the landed one share its tool."""
        sha = "6" * 40
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            head_tool, own_tool = folder / "head-stage", folder / "own-stage"
            # The head's tool succeeds on the base, but produces the checkout's record instead of the base's.
            head_tool.write_text(f"""#!/bin/bash
if [ "$1" = surface ]; then
  echo '{{"by": "checkout"}}' > "$3"; exit 0
fi
[ "$1" = surface-catalogs ] && exit 0
cat "$2" "$3" >> {folder}/compared
""")
            own_tool.write_text("""#!/bin/bash
echo "{\\"schema\\": 3, \\"by\\": \\"$(cat "$2/marker")\\"}" > "$3"
""")
            head_tool.chmod(0o755)
            own_tool.chmod(0o755)
            built = []

            def base_tool(commit, work):
                built.append(commit)
                return str(own_tool)

            def unpack(commit, into, paths=()):
                (into / "marker").write_text(commit)

            def git(*args):
                return MERGE if args[0] == "cat-file" else ""

            code, out, _ = self.run_main(tool=str(head_tool), base_commit=lambda *a: (sha, ""), unpack=unpack,
                                         git=git, base_tool=base_tool, landed_notes=lambda head, base: {sha: ["n.toml"]})
            self.assertEqual(code, 0, out)
            self.assertEqual(built, [sha, "aaaa"])
            exports = [json.loads(line)["by"] for line in (folder / "compared").read_text().splitlines()]
            # The base against this checkout, then the landed merge against its first parent.
            self.assertEqual(exports, [sha, "checkout", "aaaa", sha])

    @unittest.skipUnless(importlib.util.find_spec("fcntl"), "needs POSIX file locking")
    def test_each_base_tool_build_gets_its_own_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            work = folder / "work"
            work.mkdir()

            def cargo(command, cwd, **_):
                binary = folder / "target/surface-base/debug/sunrise-stage"
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_text(str(cwd))
                return subprocess.CompletedProcess(command, 0, "", "")

            with mock.patch.object(surface_report, "ROOT", folder), \
                 mock.patch.object(surface_report.subprocess, "run", cargo):
                first = surface_report.tree_tool(Path("one"), work)
                second = surface_report.tree_tool(Path("two"), work)
            # A later build of another tree replaces the shared binary, never a copy an audit already holds.
            self.assertNotEqual(first, second)
            self.assertEqual((Path(first).read_text(), Path(second).read_text()), ("one", "two"))


@unittest.skipUnless(shutil.which("bash"), "needs bash for the stub tool")
class LandedTests(unittest.TestCase):
    SHA = "6" * 40

    def check(self, diff_exit, git_text=None, without_own_tool=False, own_tool=None, parent_schema=3):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for side in ("head", "base"):
                notes = work / side / surface_report.NOTES
                notes.mkdir(parents=True)
            (work / "head" / surface_report.NOTES / "r61.toml").write_text(
                f'ids = ["loop:a"]\nwhy = "w"\nlanded = "{self.SHA}"\n')
            (work / "head" / surface_report.NOTES / "own.toml").write_text('ids = ["hook:b"]\nwhy = "w"\n')
            tool = work / "sunrise-stage"
            tool.write_text(f"""#!/bin/bash
echo "$@" >> {work}/calls
[ "$1" = surface ] && exit 1 # landed must not ask the current exporter to read a historical tree.
[ "$1" = surface-catalogs ] && exit 0
cat "$2" "$3" > {work}/compared
while [ $# -gt 0 ]; do case "$1" in --notes|--base-notes|--covered-by|--covered-base) ls "$2" > {work}/given${{1#--}};; esac; shift; done
if [ {diff_exit} != 0 ]; then echo "change note r61.toml names loop:a, which did not change" >&2; fi
exit {diff_exit}
""")
            tool.chmod(0o755)
            shas = surface_report.landed_notes(work / "head", work / "base")
            fetched = []

            def fetch(sha, into):
                # Each fetched tree names its commit, for the exporters to read; the merge brings a note of its own.
                fetched.append(sha)
                (into / "marker").write_text(sha)
                if sha == self.SHA:
                    (into / surface_report.NOTES).mkdir(parents=True)
                    (into / surface_report.NOTES / "then.toml").write_text('ids = ["loop:b"]\nwhy = "w"\n')

            git_text = git_text or (lambda *a: MERGE if a[0] == "cat-file" else "")
            if own_tool is None and not without_own_tool:
                exporter = work / "historical-stage"
                exporter.write_text(f"""#!/bin/bash
schema=3
[ "$(cat "$2/marker")" != aaaa ] || schema={parent_schema}
printf '{{"schema": %s, "hash": "from-historical-stage"}}\\n' "$schema" > "$3"
""")
                exporter.chmod(0o755)
                own_tool = lambda _sha, _work: str(exporter)
            code, text = surface_report.landed_check(
                str(tool), work / "head", work / "base", work, shas, git_text=git_text, fetch=fetch,
                own_tool=(lambda sha: own_tool(sha, work)) if own_tool else None)
            calls = (work / "calls").read_text() if (work / "calls").exists() else ""
            for given in ("notes", "base-notes", "covered-by", "covered-base"):
                if (work / f"given{given}").exists():
                    calls += f"{given}: " + (work / f"given{given}").read_text().replace("\n", " ") + "; "
            if (work / "compared").exists():
                calls += "compared " + (work / "compared").read_text().replace("\n", " ")
            return shas, code, text, calls, fetched

    def test_only_new_notes_that_carry_landed_are_found(self):
        shas, *_ = self.check(0)
        self.assertEqual(shas, {self.SHA: ["r61.toml"]})

    def test_a_note_whose_ids_changed_in_that_merge_passes(self):
        _, code, text, calls, fetched = self.check(0)
        self.assertEqual(code, 0, text)
        self.assertEqual(fetched, ["aaaa", self.SHA])
        self.assertIn(f"--landed {self.SHA}", calls)
        self.assertNotIn("--gate", calls)
        # The diff checks this pull request's notes (the late one for this merge among them), and reads the merge's
        # own notes against its first parent's apart from them, as coverage.
        self.assertIn("notes: own.toml r61.toml ;", calls)
        self.assertIn("covered-by: then.toml ;", calls)
        self.assertIn("covered-base: ;", calls)

    def test_a_note_whose_ids_did_not_change_there_fails_naming_it(self):
        _, code, text, _, _ = self.check(1)
        self.assertEqual(code, 1)
        self.assertIn(f"r61.toml (landed {self.SHA})", text)

    def test_a_landed_note_does_not_compare_a_schema_two_first_parent(self):
        _, code, text, calls, _ = self.check(0, parent_schema=2)
        self.assertEqual(code, 1)
        self.assertIn(f"r61.toml (landed {self.SHA})", text)
        self.assertIn("The first parent uses surface schema 2", text)
        self.assertNotIn("surface-catalogs", calls)
        self.assertNotIn("surface-diff", calls)

    def test_old_trees_are_exported_by_their_own_commits_tools(self):
        built = []

        def own_tool(sha, work):
            built.append(sha)
            tool = work / f"own-{sha}"
            # Says which commit's tool exported which tree.
            tool.write_text(f"""#!/bin/bash
echo "{{\\"schema\\": 3, \\"by\\": \\"{sha} tool, $(cat "$2/marker") tree\\"}}" > "$3"
""")
            tool.chmod(0o755)
            return str(tool)

        _, code, text, calls, _ = self.check(0, own_tool=own_tool)
        self.assertEqual(code, 0, text)
        self.assertEqual(built, ["aaaa", self.SHA])
        # The diff compares the first parent, exported by its own tool, with the merge, exported by its own.
        self.assertIn('compared {"schema": 3, "by": "aaaa tool, aaaa tree"} '
                      '{"schema": 3, "by": "%s tool, %s tree"}' % (self.SHA, self.SHA), calls)
        self.assertIn(f"--landed {self.SHA}", calls)
        # Without the commit's exporter the check cannot verify the historical surface.
        _, code, text, calls, _ = self.check(0, without_own_tool=True)
        self.assertEqual(code, 1)
        self.assertIn("the landed base must be exported in its original schema", text)
        self.assertNotIn("surface-diff", calls)

    def test_a_landed_that_is_not_a_full_sha_is_not_returned(self):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for side in ("head", "base"):
                (work / side / surface_report.NOTES).mkdir(parents=True)
            for name, value in (("a", "--upload-pack=x"), ("b", "6597e08"), ("c", "G" * 40)):
                (work / "head" / surface_report.NOTES / f"{name}.toml").write_text(
                    f'ids = ["x"]\nwhy = "w"\nlanded = "{value}"\n')
            self.assertEqual(surface_report.landed_notes(work / "head", work / "base"), {})

    def test_a_commit_that_is_not_a_two_parent_merge_fails(self):
        _, code, text, calls, _ = self.check(
            0, git_text=lambda *a: SINGLE if a[0] == "cat-file" else "")
        self.assertEqual(code, 1)
        self.assertIn("not a merge commit with two parents", text)
        self.assertNotIn("surface-diff", calls)


if __name__ == "__main__":
    unittest.main()
