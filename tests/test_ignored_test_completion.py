"""Ignored-test CI must distinguish completed targets from child fixture output."""
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch

from tools.ci import ignored_tests

# What precedes a test run's own command: GNU timeout and the run's limit, where the platform has them.
LIMIT = len(ignored_tests.time_limit())


def target(count, results, summary):
    return f"     Running unittests src/lib.rs (target/debug/deps/example)\nrunning {count} tests\n{results}{summary}"


def summary(passed=0, failed=0, ignored=0):
    verdict = "FAILED" if failed else "ok"
    return (f"test result: {verdict}. {passed} passed; {failed} failed; "
            f"{ignored} ignored; 0 measured; 12 filtered out; finished in 0.1s\n")


class IgnoredTests(unittest.TestCase):
    def run_output(self, output, code=0):
        with patch.object(ignored_tests.subprocess, "run", return_value=subprocess.CompletedProcess([], code, output)), contextlib.redirect_stdout(io.StringIO()) as log:
            result = ignored_tests.run("example", [])
        return result, log.getvalue()

    def test_test_threads_get_the_harnesses_stack(self):
        # A server Game held by value overflowed libtest's 2 MiB test thread (#3744).
        output = target(1, "test first ... ok\n", summary(passed=1))
        with patch.object(ignored_tests.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)) as run, \
                contextlib.redirect_stdout(io.StringIO()):
            ignored_tests.run("sunrise-client", [])
        stacks = [call.kwargs["env"].get("RUST_MIN_STACK") for call in run.call_args_list if call.args[0][LIMIT:LIMIT + 2] == ["cargo", "test"]]
        self.assertTrue(stacks)
        self.assertEqual(set(stacks), {str(16 * 1024 * 1024)})

    def test_daily_tests_require_a_gpu_adapter_in_cargo_and_prebuilt_binaries(self):
        output = target(1, "test gpu_case ... ok\n", summary(passed=1))
        result = subprocess.CompletedProcess([], 0, output)
        with patch.dict(os.environ, {"SUNRISE_GPU_TESTS_REQUIRE_ADAPTER": "0"}), \
                patch.object(ignored_tests.subprocess, "run", return_value=result) as run, \
                patch.object(ignored_tests.ignored_shards, "workspace_members",
                             return_value={"example": ("crates/example", ".")}), \
                patch.object(ignored_tests, "workspace_features", return_value={}), \
                contextlib.redirect_stdout(io.StringIO()):
            ignored_tests.run("example", [])
            ignored_tests.run("example", [], only=(["lib"], ["gpu_case"]),
                              binaries={"lib": "/fake/example-tests"})
        commands = {tuple(call.args[0][LIMIT:])[:2]: call.kwargs["env"]
                    for call in run.call_args_list if len(call.args[0]) > LIMIT}
        self.assertEqual(commands[("cargo", "test")]["SUNRISE_GPU_TESTS_REQUIRE_ADAPTER"], "1")
        self.assertEqual(commands[("/fake/example-tests", "--ignored")]["SUNRISE_GPU_TESTS_REQUIRE_ADAPTER"], "1")

    def test_success_and_multiple_targets(self):
        output = target(1, "test first ... ok\n", summary(passed=1))
        output += target(2, "test second ... ok\ntest third ... ok\n", summary(passed=2))
        result, log = self.run_output(output)
        self.assertEqual(result, {"first": True, "second": True, "third": True})
        self.assertIn("target 2 started 2 tests", log)
        self.assertIn("cargo exit 0", log)

    def test_a_target_whose_source_is_in_another_package_folder_completes(self):
        # sunrise-client-lib's `[[test]]` targets point at ../sunrise-client/tests/, as the daily-rows leg runs them.
        output = ("     Running ../sunrise-client/tests/lan_bot.rs (target/debug/deps/lan_bot-0123)\nrunning 1 test\n"
                  "test synced ... ok\n" + summary(passed=1))
        result, _ = self.run_output(output)
        self.assertEqual(result, {"synced": True})

    def test_known_failure_is_still_available_to_policy(self):
        result, _ = self.run_output(target(2, "test good ... ok\ntest bad ... FAILED\n", summary(passed=1, failed=1)), 101)
        self.assertEqual(result, {"good": True, "bad": False})
        with patch.object(ignored_tests, "known_failures", return_value={("example", "bad"): None}), patch.object(ignored_tests, "run", return_value=result), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(ignored_tests.main(["example"]), 0)

    def test_plain_known_failure_that_passes_requires_removal(self):
        result, _ = self.run_output(target(1, "test repaired ... ok\n", summary(passed=1)))
        with patch.object(ignored_tests, "known_failures", return_value={("example", "repaired"): None}), patch.object(ignored_tests, "run", return_value=result), contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ignored_tests.main(["example"]), 1)
        self.assertIn("repaired: passes now; take it off", log.getvalue())

    def test_the_report_names_problems_and_passes_without_output(self):
        def run(package, skip, number=None, count=None):
            ignored_tests._panic_locations["new::broken"] = "crates/example/src/lib.rs:4:9"
            return {"new::broken": False, "fine": True, "repaired": True, "accepted": False}

        known = {("example", "repaired"): None, ("example", "accepted"): None, ("example", "gone"): None}
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(ignored_tests, "known_failures", return_value=known), \
                patch.object(ignored_tests, "run", side_effect=run), contextlib.redirect_stdout(io.StringIO()):
            path = Path(folder) / "report.json"
            self.assertEqual(ignored_tests.main(["example"], str(path)), 1)
            report = json.loads(path.read_text())
        kinds = {(item["test"], item["kind"], item["panic"]) for item in report["problems"]}
        self.assertEqual(kinds, {("new::broken", "failed", "crates/example/src/lib.rs:4:9"),
                                 ("repaired", "passes-now", None), ("gone", "no-longer-runs", None)})
        self.assertIn({"package": "example", "test": "fine"}, report["passed"])
        self.assertNotIn("accepted", json.dumps(report["problems"]))
        self.assertEqual(report["shards"], ["example"])

    def test_a_shard_that_did_not_complete_is_reported(self):
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(ignored_tests, "known_failures", return_value={}), \
                patch.object(ignored_tests, "run", return_value=None), contextlib.redirect_stdout(io.StringIO()):
            path = Path(folder) / "report.json"
            self.assertEqual(ignored_tests.main(["example"], str(path)), 1)
            self.assertEqual(json.loads(path.read_text())["incomplete"], [{"shard": "example"}])

    def test_the_tests_a_stopped_run_reported_keep_their_verdicts(self):
        def run(package, skip, number=None, count=None):
            return ignored_tests.stopped("a test process crashed", 0.0, None, results={"fine": True, "broken": False})

        known = {("example", "gone"): None}
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(ignored_tests, "CGROUP", Path(folder)), \
                patch.object(ignored_tests, "known_failures", return_value=known), \
                patch.object(ignored_tests, "run", side_effect=run), contextlib.redirect_stdout(io.StringIO()):
            path = Path(folder) / "report.json"
            self.assertEqual(ignored_tests.main(["example"], str(path)), 1)
            report = json.loads(path.read_text())
        self.assertEqual(report["passed"], [{"package": "example", "test": "fine"}])
        self.assertEqual({(item["test"], item["kind"]) for item in report["problems"]}, {("broken", "failed")})
        self.assertEqual([item["shard"] for item in report["incomplete"]], ["example"])

    def test_a_client_half_runs_as_batches_of_fresh_processes_and_a_killed_one_stops_none_of_the_others(self):
        calls = []

        def run(package, skip, number=None, count=None):
            calls.append((number, count))
            if number == 3:
                return ignored_tests.stopped("a test process crashed", 0.0, None, results={"third::passed": True})
            return {f"batch{number}::test": True}

        with tempfile.TemporaryDirectory() as folder, \
                patch.object(ignored_tests, "CGROUP", Path(folder)), \
                patch.object(ignored_tests, "known_failures", return_value={}), \
                patch.object(ignored_tests, "run", side_effect=run), contextlib.redirect_stdout(io.StringIO()):
            path = Path(folder) / "report.json"
            self.assertEqual(ignored_tests.main(["sunrise-client#1of2"], str(path)), 1)
            report = json.loads(path.read_text())
        # Half 1 of 2 is every test whose leg of 8 is odd: legs 1, 3, 5 and 7, one process each, in that order.
        self.assertEqual(calls, [(1, 8), (3, 8), (5, 8), (7, 8)])
        self.assertEqual({item["test"] for item in report["passed"]},
                         {"batch1::test", "third::passed", "batch5::test", "batch7::test"})
        self.assertEqual([item["shard"] for item in report["incomplete"]], ["sunrise-client#1of2"])

    def test_a_test_process_killed_for_memory_reports_signal_memory_time_and_what_still_ran(self):
        # The shape of the first daily's sunrise-client#1of2 (run 43499359519): the library's test process killed by
        # the out-of-memory killer with one test past 60 s; the later targets finished; cargo exits 101.
        stuck = "game::native_datafix_tests::a_mods_host_upgrades_old_data_and_refuses_by_name"
        output = ("     Running unittests src/lib.rs (target/debug/deps/sunrise_client-0123)\nrunning 3 tests\n"
                  "test game::a ... ok\ntest game::b ... ok\n"
                  f"test {stuck} has been running for over 60 seconds\n"
                  "error: test failed, to rerun pass `--lib`\n\nCaused by:\n  process didn't exit successfully: "
                  "`/w/target/debug/deps/sunrise_client-0123 --include-ignored --exact` (signal: 9, SIGKILL: kill)\n"
                  + target(1, "test other::c ... ok\n", summary(passed=1)))
        gib = 32 * 1024 ** 3
        with tempfile.TemporaryDirectory() as cgroup:
            Path(cgroup, "memory.peak").write_text(f"{gib}\n")
            Path(cgroup, "memory.max").write_text(f"{gib}\n")
            events = Path(cgroup, "memory.events")
            events.write_text("low 0\nhigh 0\nmax 812\noom 1\noom_kill 2\n")

            def execute(command, **_):
                events.write_text("low 0\nhigh 0\nmax 990\noom 2\noom_kill 3\n")
                return subprocess.CompletedProcess(command, 101, output)

            with patch.object(ignored_tests, "CGROUP", Path(cgroup)), \
                    patch.object(ignored_tests.subprocess, "run", side_effect=execute), \
                    contextlib.redirect_stdout(io.StringIO()):
                self.assertIsNone(ignored_tests.run("sunrise-client", []))
        why = dict(ignored_tests._stopped)
        self.assertGreaterEqual(why.pop("wall_seconds"), 0)
        self.assertEqual(why, {
            "reason": "a test process crashed", "ended_by": "signal 9 (SIGKILL)", "cargo_exit": 101,
            "memory_peak_bytes": gib, "memory_max_bytes": gib, "oom_kills": 1,
            "still_running": [stuck], "still_running_more": 0, "never_reported": None})

    def test_an_identity_is_bounded_in_length(self):
        self.assertEqual(ignored_tests.safe_identity("a::" + "b" * 297), "a::" + "b" * 297)
        self.assertEqual(ignored_tests.safe_identity("a" * 100_000),
                         f"identity withheld (longer than {ignored_tests.IDENTITY_CHARS} characters)")
        # A long-running notice with a huge name never reaches the stopped run's report as it is.
        output = (f"     Running unittests src/lib.rs (target/debug/deps/example)\nrunning 1 tests\n"
                  f"test {'x' * 5000} has been running for over 60 seconds\n"
                  "error: test failed, to rerun pass `--lib`\n\nCaused by:\n  process didn't exit successfully: "
                  "`example` (signal: 9, SIGKILL: kill)\n")
        with patch.object(ignored_tests.subprocess, "run", return_value=subprocess.CompletedProcess([], 101, output)), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertIsNone(ignored_tests.run("example", []))
        self.assertEqual(ignored_tests._stopped["still_running"],
                         [f"identity withheld (longer than {ignored_tests.IDENTITY_CHARS} characters)"])

    def test_a_run_stopped_at_its_time_limit_names_the_tests_still_running(self):
        # A hung test held both client halves until the job limit cancelled them, naming nothing (daily 44814607973).
        output = (target(3, "test done ... ok\ntest game::dragon::hangs has been running for over 60 seconds\n"
                            "test game::wind::finished_late has been running for over 60 seconds\n"
                            "test game::wind::finished_late ... ok\n", ""))
        with patch.object(ignored_tests.subprocess, "run",
                          return_value=subprocess.CompletedProcess([], ignored_tests.TIMED_OUT, output)) as run, \
                contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertIsNone(ignored_tests.run("example", []))
        if LIMIT:
            self.assertEqual(run.call_args.args[0][:LIMIT], ["timeout", f"{ignored_tests.TEST_RUN_LIMIT}s"])
        self.assertEqual(ignored_tests._stopped["reason"], "tests still running at the time limit (a hung test)")
        self.assertEqual(ignored_tests._stopped["ended_by"], f"the {ignored_tests.TEST_RUN_LIMIT} s limit")
        self.assertEqual(ignored_tests._stopped["still_running"], ["game::dragon::hangs"])
        # What finished before the limit still counts.
        self.assertEqual(ignored_tests._reported, {"done": True, "game::wind::finished_late": True})
        self.assertIn("still running past 60 s: game::dragon::hangs", log.getvalue())

    def test_one_executable_stopped_at_the_limit_stops_the_daily_rows_run_but_not_the_next_executable(self):
        ran = []

        def execute(command, **_):
            executable = command[LIMIT]
            ran.append(Path(executable).name)
            code = ignored_tests.TIMED_OUT if executable.endswith("/a") else 0
            return subprocess.CompletedProcess(command, code, "")

        with tempfile.TemporaryDirectory() as folder, \
                patch.object(ignored_tests.ignored_shards, "workspace_members", return_value={"pkg": (folder, ".")}), \
                patch.object(ignored_tests.subprocess, "run", side_effect=execute):
            done = ignored_tests.run_binaries("pkg", {"lib": f"{folder}/a", "test:b": f"{folder}/b"}, ["--ignored"], {})
        self.assertEqual(ran, ["a", "b"])
        self.assertEqual(done.returncode, ignored_tests.TIMED_OUT)

    def test_the_report_names_the_run_in_progress_before_it_ends(self):
        # A job stopped at its time limit kills this script too: what it wrote before the run is what is uploaded.
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "report.json"

            def run(package, skip, number=None, count=None):
                pending = json.loads(path.read_text())["incomplete"]
                self.assertEqual([(item["shard"], item["in_progress"]) for item in pending], [("example", True)])
                return {"fine": True}

            with patch.object(ignored_tests, "known_failures", return_value={}), \
                    patch.object(ignored_tests, "run", side_effect=run), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(ignored_tests.main(["example"], str(path)), 0)
            self.assertEqual(json.loads(path.read_text())["incomplete"], [])

    def test_client_lib_halves_run_serially_and_preserve_failure_policy(self):
        package = "sunrise-client-lib"
        names = [f"suite::case_{n}" for n in range(20)]
        seen = []

        def run(package_arg, skip, number=None, count=None):
            self.assertEqual(package_arg, package)
            self.assertEqual(count, 2)
            selected = [name for name in names if ignored_tests.in_leg(name, number, count)]
            seen.extend(selected)
            return {name: name != names[0] for name in selected}

        with patch.object(ignored_tests, "known_failures", return_value={}), patch.object(ignored_tests, "run", side_effect=run) as execute, contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ignored_tests.main([package]), 1)
        self.assertEqual([call.args[2:] for call in execute.call_args_list], [(1, 2), (2, 2)])
        self.assertCountEqual(seen, names)
        self.assertIn(f"{names[0]}: failed, and isn't a known failure", log.getvalue())

    def test_a_test_needing_a_harness_input_is_refused_by_name_without_it(self):
        inputs = {("example", "needs_input"): "SUNRISE_EXAMPLE_INPUT"}
        with patch.dict(os.environ), patch.object(ignored_tests, "known_failures", return_value={}), \
                patch.object(ignored_tests, "harness_inputs", return_value=inputs), \
                patch.object(ignored_tests, "run", return_value={"other": True}) as execute, \
                contextlib.redirect_stdout(io.StringIO()) as log:
            os.environ.pop("SUNRISE_EXAMPLE_INPUT", None)
            self.assertEqual(ignored_tests.main(["example"]), 0)
        self.assertEqual(execute.call_args.args, ("example", ["needs_input"]))
        self.assertIn("example needs_input: refused, needs SUNRISE_EXAMPLE_INPUT; run by the parity harness\n",
                      log.getvalue())

    def test_a_test_needing_a_harness_input_runs_and_must_pass_with_it(self):
        inputs = {("example", "needs_input"): "SUNRISE_EXAMPLE_INPUT"}
        with patch.dict(os.environ, {"SUNRISE_EXAMPLE_INPUT": "/input"}), \
                patch.object(ignored_tests, "known_failures", return_value={}), \
                patch.object(ignored_tests, "harness_inputs", return_value=inputs), \
                patch.object(ignored_tests, "run", return_value={"needs_input": False}) as execute, \
                contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ignored_tests.main(["example"]), 1)
        self.assertEqual(execute.call_args.args, ("example", []))
        self.assertNotIn("refused", log.getvalue())
        self.assertIn("example needs_input: failed, and isn't a known failure", log.getvalue())

    def test_needs_entries_name_a_variable_and_are_not_known_failures(self):
        inputs = ignored_tests.harness_inputs()
        self.assertTrue(inputs)
        for (package, test), variable in inputs.items():
            self.assertRegex(variable, r"^[A-Z][A-Z0-9_]*$")
            self.assertNotIn((package, test), ignored_tests.known_failures())

    def test_explicit_client_lib_selection_is_not_split_again(self):
        with patch.object(ignored_tests, "listed_entries", return_value=[]), \
                patch.object(ignored_tests, "run", return_value={}) as execute:
            self.assertEqual(ignored_tests.main(["sunrise-client-lib#2of3"]), 0)
        self.assertEqual(execute.call_args.args, ("sunrise-client-lib", [], 2, 3))

    def daily_main(self, shards, rows, run, binaries=None):
        report = Path(tempfile.mkdtemp()) / "report.json"
        with patch.object(ignored_tests, "known_failures", return_value={}), \
                patch.object(ignored_tests, "harness_inputs", return_value={}), \
                patch.object(ignored_tests, "listed_entries", return_value=rows), \
                patch.object(ignored_tests, "built_test_binaries", return_value=binaries or {}) as build, \
                patch.object(ignored_tests, "run", side_effect=run) as execute, \
                contextlib.redirect_stdout(io.StringIO()) as log:
            code = ignored_tests.main(shards, str(report))
        self.build_calls = build.call_count
        return code, execute, log.getvalue(), json.loads(report.read_text())

    def test_the_daily_rows_leg_builds_the_workspace_tests_once_and_runs_what_that_build_made(self):
        rows = [ignored_tests.Row("sunrise-relay", "a", "test:tunnel", "daily", "daily-sunrise-relay.txt", "slow"),
                ignored_tests.Row("sunrise-core", "c", "lib", "daily", "daily-sunrise-core.txt", "slow"),
                ignored_tests.Row("sunrise-world", "e", "test:integration", "daily", "daily-sunrise-world.txt", "slow"),
                ignored_tests.Row("sunrise-client-lib", "f", "test:lan_bot", "daily", "daily-sunrise-client-lib.txt",
                                  "slow")]
        # sunrise-world's target didn't build: its rows build and report through cargo test, as before.
        binaries = {("sunrise-relay", "test:tunnel"): "/t/tunnel-1", ("sunrise-core", "lib"): "/t/sunrise_core-2",
                    ("sunrise-client-lib", "test:lan_bot"): "/t/lan_bot-3"}
        code, execute, _, _ = self.daily_main(
            ["daily-rows"], rows, lambda package, skip, only, binaries: {name: True for name in only[1]}, binaries)
        self.assertEqual(code, 0)
        self.assertEqual(self.build_calls, 1)
        self.assertEqual({call.args[0]: call.kwargs["binaries"] for call in execute.call_args_list}, {
            "sunrise-client-lib": None,  # its rows build at opt-level 3 (OWN_BUILD)
            "sunrise-core": {"lib": "/t/sunrise_core-2"},
            "sunrise-relay": {"test:tunnel": "/t/tunnel-1"},
            "sunrise-world": None,
        })

    def test_the_workspace_test_build_maps_each_member_target_to_its_executable(self):
        relay = str((ignored_tests.ROOT / ignored_tests.ignored_shards.workspace_members()["sunrise-relay"][0]
                     / "Cargo.toml"))
        records = [
            {"reason": "compiler-artifact", "manifest_path": relay, "executable": "/t/sunrise_relay-1",
             "profile": {"test": True}, "target": {"kind": ["lib"], "name": "sunrise_relay"}},
            {"reason": "compiler-artifact", "manifest_path": relay, "executable": "/t/it-2",
             "profile": {"test": True}, "target": {"kind": ["test"], "name": "it"}},
            {"reason": "compiler-artifact", "manifest_path": relay, "executable": "/t/relay-3",
             "profile": {"test": False}, "target": {"kind": ["bin"], "name": "relay"}},
            {"reason": "compiler-artifact", "manifest_path": "/elsewhere/Cargo.toml", "executable": "/t/dep-4",
             "profile": {"test": True}, "target": {"kind": ["lib"], "name": "dep"}},
            {"reason": "build-finished", "success": True},
        ]
        out = "\n".join(map(json.dumps, records)) + "\n"
        with patch.object(ignored_tests.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, out)) as build, \
                contextlib.redirect_stdout(io.StringIO()):
            binaries = ignored_tests.built_test_binaries()
        self.assertEqual(binaries, {("sunrise-relay", "lib"): "/t/sunrise_relay-1",
                                    ("sunrise-relay", "test:it"): "/t/it-2"})
        command = build.call_args.args[0]
        self.assertEqual(command[:command.index("--message-format=json")], ignored_tests.WORKSPACE_TEST_BUILD)

    def test_prebuilt_executables_report_like_cargo_test_and_a_killed_one_is_a_crash(self):
        with tempfile.TemporaryDirectory() as folder:
            passing, killed = Path(folder, "passes"), Path(folder, "killed")
            passing.write_text("#!/bin/sh\necho 'running 1 tests'\necho 'test a ... ok'\n"
                               "echo 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out'\n")
            killed.write_text("#!/bin/sh\necho 'running 1 tests'\nkill -9 $$\n")
            for script in (passing, killed):
                script.chmod(0o755)
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(ignored_tests.run("sunrise-relay", [], only=(["lib"], ["a"]),
                                                   binaries={"lib": str(passing)}), {"a": True})
                self.assertIsNone(ignored_tests.run("sunrise-relay", [], only=(["lib", "test:it"], ["a", "b"]),
                                                    binaries={"lib": str(passing), "test:it": str(killed)}))
            self.assertEqual(ignored_tests._stopped["reason"], "a test process crashed")

    def test_daily_rows_of_a_shards_targets_must_run_and_pass_in_that_shard(self):
        rows = [ignored_tests.Row("sunrise-world", f"slow::{name}", "lib", "daily", "daily-sunrise-world.txt", "slow")
                for name in ("passes", "fails", "gone")]
        rows.append(ignored_tests.Row("sunrise-world", "it::slow", "test:integration", "daily", "daily-sunrise-world.txt", "slow"))
        code, execute, log, report = self.daily_main(
            ["sunrise-world"], rows, lambda *args, **kwargs: {"slow::passes": True, "slow::fails": False})
        self.assertEqual(code, 1)
        self.assertEqual(execute.call_args.args, ("sunrise-world", []))
        self.assertEqual(sorted((problem["test"], problem["kind"]) for problem in report["problems"]),
                         [("slow::fails", "failed"), ("slow::gone", "no-longer-runs")])
        self.assertNotIn("accepted failure", log)
        # The integration test is not the shard's target: the daily-rows leg runs it, so the shard does not miss it.
        self.assertNotIn("it::slow", log)

    def test_the_daily_rows_leg_runs_every_other_daily_row_by_package_and_target(self):
        rows = [
            ignored_tests.Row("sunrise-relay", "b", "test:tunnel", "daily", "daily-sunrise-relay.txt", "slow"),
            ignored_tests.Row("sunrise-relay", "a", "test:tunnel", "daily", "daily-sunrise-relay.txt", "slow"),
            ignored_tests.Row("sunrise-core", "c", "lib", "daily", "daily-sunrise-core.txt", "slow"),
            ignored_tests.Row("sunrise-world", "d", "lib", "daily", "daily-sunrise-world.txt", "slow"),
            ignored_tests.Row("sunrise-world", "e", "test:integration", "daily", "daily-sunrise-world.txt", "slow"),
        ]
        rows.append(ignored_tests.Row("sunrise-relay", "f", "lib", "daily", "daily-sunrise-relay.txt", "slow"))
        results = {"sunrise-relay": {"a": True, "b": True, "f": True}, "sunrise-core": {},
                   "sunrise-world": {"e": False}}
        code, execute, _, report = self.daily_main(
            ["daily-rows"], rows, lambda package, skip, only, binaries: results[package])
        self.assertEqual(code, 1)
        # One run per package, every target of it together.
        self.assertEqual([(call.args[0], call.kwargs["only"]) for call in execute.call_args_list], [
            ("sunrise-core", (["lib"], ["c"])),
            ("sunrise-relay", (["lib", "test:tunnel"], ["f", "a", "b"])),
            ("sunrise-world", (["test:integration"], ["e"])),
        ])
        self.assertEqual(sorted((problem["package"], problem["test"], problem["kind"]) for problem in report["problems"]),
                         [("sunrise-core", "c", "no-longer-runs"), ("sunrise-world", "e", "failed")])
        self.assertEqual(sorted(entry["test"] for entry in report["passed"]), ["a", "b", "f"])

    def test_a_daily_rows_run_selects_exactly_its_ignored_tests_of_one_target(self):
        output = target(2, "test a ... ok\ntest b ... ok\n", summary(passed=2))
        with patch.object(ignored_tests.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)) as execute, \
                patch.object(ignored_tests, "workspace_features", return_value={"sunrise-relay": ["fixture", "index"]}), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(ignored_tests.run("sunrise-relay", [], only=(["lib", "test:tunnel"], ["a", "b"])),
                             {"a": True, "b": True})
        command = execute.call_args.args[0][LIMIT:]
        # The features the workspace build turns on, as verify compiled the test.
        self.assertEqual(command[:command.index("--")],
                         ["cargo", "test", "-p", "sunrise-relay", "--features", "fixture,index", "--lib", "--test", "tunnel",
                          "--no-fail-fast"])
        self.assertEqual(command[command.index("--") + 1:], ["--ignored", "--exact", "--test-threads=4", "a", "b"])

    def test_the_daily_rows_leg_runs_its_workspaces_groups(self):
        rows = [ignored_tests.Row("sunrise-relay", "a", "test:tunnel", "daily", "daily-sunrise-relay.txt", "slow")]
        _, execute, _, _ = self.daily_main(["daily-rows"], rows, lambda package, skip, only, binaries: {only[1][0]: True})
        self.assertEqual([(call.args[0], call.kwargs["only"]) for call in execute.call_args_list],
                         [("sunrise-relay", (["test:tunnel"], ["a"]))])

    def test_a_package_that_needs_something_built_gets_it_before_its_daily_tests(self):
        output = target(1, "test holder::tests::slow ... ok\n", summary(passed=1))
        prepare = ignored_tests.ignored_shards.PREPARE["sunrise-mod-package"]
        self.assertEqual(prepare[-2:], ["--example", "holder_actor"])
        with patch.object(ignored_tests.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)) as execute, \
                patch.object(ignored_tests, "workspace_features", return_value={}), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(ignored_tests.run("sunrise-mod-package", [], only=(["lib"], ["holder::tests::slow"])),
                             {"holder::tests::slow": True})
        self.assertEqual(execute.call_args_list[0].args[0], prepare)
        self.assertEqual(execute.call_args_list[1].args[0][LIMIT:LIMIT + 2], ["cargo", "test"])
        # A failed build stops the run, saying why, before any test runs.
        with patch.object(ignored_tests.subprocess, "run", return_value=subprocess.CompletedProcess([], 101, "error: x\n")) as execute, \
                patch.object(ignored_tests, "workspace_features", return_value={}), contextlib.redirect_stdout(io.StringIO()):
            self.assertIsNone(ignored_tests.run("sunrise-mod-package", [], only=(["lib"], ["holder::tests::slow"])))
        self.assertEqual(execute.call_count, 1)
        self.assertEqual(ignored_tests._stopped["reason"], "what the tests need could not be built")

    def test_a_parent_that_crashes_after_its_childs_same_count_summary_did_not_complete(self):
        # A fixture reruns its own test in a child, which reports FAILED and a one-test summary;
        # the parent then aborts before its own summary. The child's summary has the parent's count.
        output = target(1, "test fixture::case ... FAILED\n", summary(failed=1))
        output += ("error: test failed, to rerun pass `-p example --lib`\n\nCaused by:\n"
                   "  process didn't exit successfully: `target/debug/deps/example` (signal: 6, SIGABRT: process abort signal)\n")
        result, log = self.run_output(output, 101)
        self.assertIsNone(result)
        self.assertIn("example: a test process crashed (cargo exit 101)", log)
        with patch.object(ignored_tests, "known_failures", return_value={("example", "fixture::case"): None}), \
                patch.object(ignored_tests, "harness_inputs", return_value={}), \
                patch.object(ignored_tests.subprocess, "run", return_value=subprocess.CompletedProcess([], 101, output)), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ignored_tests.main(["example"]), 1)
        self.assertIn("example: the test run did not complete", log.getvalue())

    def test_a_target_completes_on_its_own_summary_after_a_childs(self):
        # The child's one-test summary comes first; the parent's two-test summary closes the target.
        output = target(2, "test fixture::case ... ok\n" + summary(passed=1) + "test fixture::case ... ok\ntest other ... ok\n",
                        summary(passed=2))
        result, log = self.run_output(output)
        self.assertEqual(result, {"fixture::case": True, "other": True})
        self.assertIn("example: test result: ok. 2 passed", log)
        self.assertNotIn("example: test result: ok. 1 passed", log)

    def test_a_target_whose_last_summary_is_a_childs_did_not_complete(self):
        # The parent started two tests; only the child's one-test summary follows.
        result, log = self.run_output(target(2, "test fixture::case ... ok\n", summary(passed=1)), 101)
        self.assertIsNone(result)
        self.assertIn("test target did not finish", log)

    def test_a_failed_target_exiting_101_is_not_a_crash(self):
        output = target(1, "test bad ... FAILED\n", summary(failed=1))
        output += ("error: test failed, to rerun pass `-p example --lib`\n\nCaused by:\n"
                   "  process didn't exit successfully: `target/debug/deps/example` (exit status: 101)\n")
        result, log = self.run_output(output, 101)
        self.assertEqual(result, {"bad": False})
        self.assertNotIn("crashed", log)

    def test_unexplained_nonzero_status(self):
        result, _ = self.run_output(target(1, "test good ... ok\n", summary(passed=1)), 101)
        self.assertIsNone(result)

    def test_missing_summary_even_after_parsed_failure(self):
        result, _ = self.run_output(target(2, "test bad ... FAILED\n", ""), 101)
        self.assertIsNone(result)

    def test_child_summary_cannot_hide_aborted_parent(self):
        output = target(2401, "running 1 test\ntest child ... ok\n", summary(passed=1))
        result, _ = self.run_output(output, 101)
        self.assertIsNone(result)

    def test_small_summary_rejected_even_on_zero_exit(self):
        result, _ = self.run_output(target(2401, "test child ... ok\n", summary(passed=1)))
        self.assertIsNone(result)

    def test_missing_first_summary_not_hidden_by_next_target(self):
        output = target(10, "test bad ... FAILED\n", "")
        output += target(1, "test child ... ok\n", summary(passed=1))
        result, _ = self.run_output(output, 101)
        self.assertIsNone(result)

    def test_target_without_count_and_build_failure(self):
        for output in ["     Running unittests src/lib.rs (target/example)\n", "error: build failed\n"]:
            result, _ = self.run_output(output, 101)
            self.assertIsNone(result)

    def test_signal_diagnostic_does_not_echo_adjacent_output(self):
        output = target(2, "", "") + "error: process failed (signal: 9, private fixture data)\n"
        result, log = self.run_output(output, 101)
        self.assertIsNone(result)
        self.assertIn("(signal: 9)", log)
        self.assertNotIn("private fixture data", log)

    def test_ignored_tests_count_toward_completion(self):
        result, _ = self.run_output(target(2, "test good ... ok\ntest skipped ... ignored\n", summary(passed=1, ignored=1)))
        self.assertEqual(result, {"good": True})


def leg_names(count=2):
    """Test names that each leg of `count` owns, found by the same hash the runner uses."""
    owned = {number: [] for number in range(1, count + 1)}
    index = 0
    while any(len(names) < 2 for names in owned.values()):
        name = f"suite::test_{index}"
        for number in owned:
            if ignored_tests.in_leg(name, number, count):
                owned[number].append(name)
        index += 1
    return owned


class Legs(unittest.TestCase):
    def test_worldgen_shard_optimizes_listing_and_execution_without_changing_parent(self):
        output = target(1, "test suite::a ... ok\n", summary(passed=1))
        for package, optimization in [("sunrise-client-lib", "3"), ("example", "0")]:
            with self.subTest(package=package), patch.dict(os.environ, CARGO_PROFILE_TEST_OPT_LEVEL="0"):
                calls = []

                def execute(command, **kwargs):
                    calls.append((command, kwargs["env"]["CARGO_PROFILE_TEST_OPT_LEVEL"]))
                    return subprocess.CompletedProcess(command, 0, "suite::a: test\n" if "--list" in command else output)

                with patch.object(ignored_tests.subprocess, "run", side_effect=execute), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(ignored_tests.run(package, [], 1, 1), {"suite::a": True})
                self.assertEqual(len(calls), 2)
                self.assertIn("--list", calls[0][0])
                self.assertNotIn("--list", calls[1][0])
                self.assertEqual([level for _, level in calls], [optimization, optimization])
                self.assertEqual(os.environ["CARGO_PROFILE_TEST_OPT_LEVEL"], "0")

    def test_shard_names(self):
        self.assertEqual(ignored_tests.parse_shard("sunrise-client#2of3"), ("sunrise-client", 2, 3))
        self.assertEqual(ignored_tests.parse_shard("sunrise-client-lib"), ("sunrise-client-lib", None, None))
        with self.assertRaises(SystemExit):
            ignored_tests.parse_shard("sunrise-client#3of2")

    def test_every_test_is_in_exactly_one_leg(self):
        for index in range(500):
            name = f"module::test_{index}"
            self.assertEqual(sum(ignored_tests.in_leg(name, number, 3) for number in (1, 2, 3)), 1)
        self.assertTrue(ignored_tests.in_leg("anything", None, None))

    def test_a_leg_skips_every_test_another_leg_owns(self):
        owned = leg_names()
        listing = "".join(f"{name}: test\n" for names in owned.values() for name in names) + "a_bench: benchmark\n"
        output = target(2, "".join(f"test {name} ... ok\n" for name in owned[1]), summary(passed=2))
        calls = []

        def fake(command, **_):
            calls.append(command)
            if "--list" in command:
                return subprocess.CompletedProcess(command, 0, listing)
            return subprocess.CompletedProcess(command, 0, output)

        with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()):
            result = ignored_tests.run("example", ["marked"], 1, 2)
        self.assertEqual(set(result), set(owned[1]))
        command = calls[1]
        skipped = {command[i + 1] for i, arg in enumerate(command) if arg == "--skip"}
        self.assertEqual(skipped, {"marked", *owned[2]})

    def test_should_panic_results_complete_a_leg_and_preserve_known_failures(self):
        good, bad = leg_names()[1][:2]
        listing = f"{good}: test\n{bad}: test\n"
        for should_panic_passes in (True, False):
            verdict = "ok" if should_panic_passes else "FAILED"
            output = target(2, f"test {good} - should panic ... {verdict}\n"
                            f"test {bad} ... FAILED\n",
                            summary(passed=int(should_panic_passes), failed=2-int(should_panic_passes)))

            def fake(command, **_):
                return subprocess.CompletedProcess(command, 0 if "--list" in command else 101,
                                                   listing if "--list" in command else output)

            with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()):
                result = ignored_tests.run("example", [], 1, 2)
            self.assertEqual(result, {good: should_panic_passes, bad: False})
            code, _ = self.judge({("example", bad): None}, result)
            self.assertEqual(code, 0 if should_panic_passes else 1)

    def test_a_listed_test_that_never_reports_fails_the_leg(self):
        owned = leg_names()[1][:2]
        listing = "".join(f"{name}: test\n" for name in owned)
        output = target(1, f"test {owned[0]} ... ok\n", summary(passed=1))

        def fake(command, **_):
            if "--list" in command:
                return subprocess.CompletedProcess(command, 0, listing)
            return subprocess.CompletedProcess(command, 0, output)

        with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertIsNone(ignored_tests.run("example", [], 1, 2))
        self.assertIn(owned[1], log.getvalue())

    def test_interleaved_listed_result_is_recovered_without_payload(self):
        name = leg_names()[1][0]
        for status in ("ok", "FAILED"):
            with self.subTest(status=status):
                def fake(command, **_):
                    text = (f"{name}: test\n" if "--list" in command else
                            target(1, f"test fixture_peer ... PRIVATE_PAYLOAD test {name} ... {status}\n",
                                   summary(passed=int(status == "ok"), failed=int(status == "FAILED"))))
                    return subprocess.CompletedProcess(command, 0 if "--list" in command or status == "ok" else 101, text)
                with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()) as log:
                    self.assertEqual(ignored_tests.run("example", [], 1, 2), {name: status == "ok"})
                self.assertNotIn("PRIVATE_PAYLOAD", log.getvalue())

    def test_prefixed_first_marker_with_trailing_unlisted_record(self):
        name = leg_names()[1][0]
        record = f"PRIVATE_PAYLOAD test {name} ... ok test other ... ok\n"
        def fake(command, **_):
            text = f"{name}: test\n" if "--list" in command else target(1, record, summary(passed=1))
            return subprocess.CompletedProcess(command, 0, text)
        with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ignored_tests.run("example", [], 1, 2), {name: True})
        self.assertNotIn("PRIVATE_PAYLOAD", log.getvalue())

    def test_multiple_distinct_listed_records_on_one_line(self):
        first, second = leg_names()[1][:2]
        record = f"test {first} ... ok test {second} - should panic ... ok\n"
        def fake(command, **_):
            text = (f"{first}: test\n{second}: test\n" if "--list" in command else
                    target(2, record, summary(passed=2)))
            return subprocess.CompletedProcess(command, 0, text)
        with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(ignored_tests.run("example", [], 1, 2), {first: True, second: True})

    def test_slow_notices_are_not_results_even_when_interleaved(self):
        name = "join_seq::visibility::tests::unbound_passes_block_until_restore_consumption_and_unwind"
        notice = f"test {name} has been running for over 60 seconds"
        expected = {name}
        for text in (
            f"{notice}\ntest {name} ... ok",
            f"test fixture_peer ... {notice}\ntest {name} ... ok",
            f"{notice} test {name} ... ok",
        ):
            with self.subTest(text=text):
                records = [record for line in text.splitlines()
                           for record in ignored_tests.listed_result_records(line, expected, len(name))]
                self.assertEqual(records, [(name, "ok")])
        for text in (notice, f"failures: {name}", f"test fixture_peer ... {notice}"):
            with self.subTest(text=text):
                self.assertEqual(list(ignored_tests.listed_result_records(text, expected, len(name))), [])

    def test_ambiguous_listed_results_fail_closed(self):
        name = leg_names()[1][0]
        for record in (
                f"test fixture_peer ... PRIVATE_PAYLOAD test {name} ... ok EXTRA\n",
                f"PRIVATE_PAYLOAD test {name} ... ok EXTRA\n",
                f"test {name} ... ok\ntest fixture_peer ... PRIVATE_PAYLOAD test {name} ... ok EXTRA\n"):
            with self.subTest(record=record.count("\n")):
                def fake(command, **_):
                    text = f"{name}: test\n" if "--list" in command else target(1, record, summary(passed=1))
                    return subprocess.CompletedProcess(command, 0, text)
                with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()) as log:
                    self.assertIsNone(ignored_tests.run("example", [], 1, 2))
                self.assertIn(f"{name}; category=ambiguous-result", log.getvalue())
                self.assertNotIn("PRIVATE_PAYLOAD", log.getvalue())
                self.assertNotIn("EXTRA", log.getvalue())

    def test_a_test_that_reruns_itself_completes_and_fails_if_any_of_its_records_failed(self):
        # A fixture that reruns its own test executable prints the child's record beside the parent's.
        name = leg_names()[1][0]
        for record, passed in (
                (f"test {name} ... ok\ntest {name} ... ok\n", True),
                (f"test {name} ... ok\n\x1b[0mPRIVATE_PAYLOAD test {name} ... FAILED\n", False),
                (f"test fixture_peer ... PRIVATE_PAYLOAD test {name} ... ok test {name} ... ok\n", True),
                (f"test {name} ... FAILED\ntest fixture_peer ... PRIVATE_PAYLOAD test {name} ... ok\n", False)):
            with self.subTest(passed=passed, record=record.count("\n")):
                def fake(command, **_):
                    text = f"{name}: test\n" if "--list" in command else target(1, record, summary(passed=1))
                    return subprocess.CompletedProcess(command, 0, text)
                with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()) as log:
                    self.assertEqual(ignored_tests.run("example", [], 1, 2)[name], passed)
                self.assertIn("example: 1 listed tests reported more than one result", log.getvalue())
                self.assertNotIn("category=", log.getvalue())
                self.assertNotIn("PRIVATE_PAYLOAD", log.getvalue())

    def test_recovery_requires_exact_listed_identity_and_complete_status(self):
        name = leg_names()[1][0]
        for record in (
                f"test fixture_peer ... PRIVATE_PAYLOAD test {name}_different ... ok\n",
                f"test fixture_peer ... PRIVATE_PAYLOAD test {name} ...\nok\n",
                f"test fixture_peer ... PRIVATE_PAYLOAD test {name} ... ignored\n",
                "test fixture_peer ... PRIVATE_PAYLOAD\n"):
            with self.subTest(record_kind=record.endswith("ignored\n")):
                def fake(command, **_):
                    text = f"{name}: test\n" if "--list" in command else target(1, record, summary(passed=1))
                    return subprocess.CompletedProcess(command, 0, text)
                with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()) as log:
                    self.assertIsNone(ignored_tests.run("example", [], 1, 2))
                self.assertIn("category=missing-result", log.getvalue())
                self.assertNotIn("PRIVATE_PAYLOAD", log.getvalue())

    def test_complete_prefixed_records_are_recovered_without_payload(self):
        name = leg_names()[1][0]
        for prefix in ("PRIVATE_PAYLOAD ", "\x1b[0mPRIVATE_PAYLOAD ",
                       f"\x1b[0m{'P' * 503}", f"\x1b[0m{'P' * 504}"):
            def fake(command, **_):
                text = (f"{name}: test\n" if "--list" in command else
                        target(1, f"{prefix}test {name} ... ok\n", summary(passed=1)))
                return subprocess.CompletedProcess(command, 0, text)
            with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()) as log:
                self.assertEqual(ignored_tests.run("example", [], 1, 2), {name: True})
            self.assertNotIn("PRIVATE_PAYLOAD", log.getvalue())
            self.assertNotIn("\x1b", log.getvalue())
            self.assertNotIn("framing=", log.getvalue())

    def test_missing_identity_framing_is_safe_and_still_fails(self):
        name = leg_names()[1][0]
        # Expected counts are independent fixture facts, not classifier output.
        cases = [
            (f"PRIVATE_PAYLOAD test {name} ...\nok\n", "split-status", 1, 0, 0, 1),
            ("test different ... ok PRIVATE_PAYLOAD\n", "absent-intact-identity", 0, 0, 0, 0),
            (f"PRIVATE_PAYLOAD test {name} ... UNKNOWN\n", "unclassified-candidate", 1, 0, 0, 0),
        ]
        for record, framing, candidates, ansi, interleaved, split in cases:
            with self.subTest(framing=framing):
                def fake(command, **_):
                    text = f"{name}: test\n" if "--list" in command else target(1, record, summary(passed=1))
                    return subprocess.CompletedProcess(command, 0, text)
                with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()) as log:
                    self.assertIsNone(ignored_tests.run("example", [], 1, 2))
                header = "example: 1 listed tests of this leg never reported a result:\n"
                expected = (
                    header
                    + f"  {name}; category=missing-result; framing={framing}; "
                    + f"candidate_lines={candidates}; ansi-prefix={ansi}; "
                    + f"interleaved-record={interleaved}; split-status={split}; "
                    + f"sampled_matches={candidates}; sample_limit=8\n"
                    + "0 missing identities not shown (shared total byte budget)\n"
                )
                # Exact allowlist rejects arbitrary record/status fragments too,
                # not just a private marker or terminal control sequence.
                self.assertEqual(log.getvalue()[log.getvalue().index(header):], expected)
                self.assertNotIn("PRIVATE_PAYLOAD", log.getvalue())
                self.assertNotIn("\x1b", log.getvalue())

    def test_framing_is_sample_bounded_and_not_printed_for_success(self):
        name = leg_names()[1][0]
        detail = ignored_tests.missing_result_framing(f"\x1b[0mtest {name} ... ok\n" * 100, name)
        self.assertIn("candidate_lines=8", detail)
        self.assertIn("sampled_matches=8; sample_limit=8", detail)
        result, log = IgnoredTests().run_output(target(1, f"test {name} ... ok\n", summary(passed=1)))
        self.assertEqual(result, {name: True})
        self.assertNotIn("framing=", log)

    def test_a_marked_test_is_not_expected_to_report(self):
        owned = leg_names()[1][:2]
        listing = "".join(f"{name}: test\n" for name in owned)
        output = target(1, f"test {owned[0]} ... ok\n", summary(passed=1))

        def fake(command, **_):
            if "--list" in command:
                return subprocess.CompletedProcess(command, 0, listing)
            return subprocess.CompletedProcess(command, 0, output)

        with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(ignored_tests.run("example", [owned[1]], 1, 2), {owned[0]: True})

    def test_every_target_runs_despite_a_failing_one(self):
        calls = []

        def fake(command, **_):
            calls.append(command)
            return subprocess.CompletedProcess(command, 0, target(1, "test a ... ok\n", summary(passed=1)))

        with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()):
            ignored_tests.run("example", [])
        self.assertIn("--no-fail-fast", calls[0])

    def test_a_leg_that_cannot_list_its_tests_fails(self):
        def fake(command, **_):
            return subprocess.CompletedProcess(command, 101, "", "error: synthetic compile failure")

        with patch.object(ignored_tests.subprocess, "run", side_effect=fake), contextlib.redirect_stdout(io.StringIO()):
            self.assertIsNone(ignored_tests.run("example", [], 1, 2))

    def judge(self, known, results, leg=1):
        with patch.object(ignored_tests, "known_failures", return_value=known), patch.object(ignored_tests, "run", return_value=results), contextlib.redirect_stdout(io.StringIO()) as log:
            code = ignored_tests.main([f"example#{leg}of2"])
        return code, log.getvalue()

    def test_an_entry_in_the_other_leg_is_not_judged_here(self):
        mine, other = leg_names()[1][0], leg_names()[2][0]
        code, log = self.judge({("example", other): None}, {mine: True})
        self.assertEqual(code, 0, log)

    def test_an_entry_in_this_leg_that_passes_or_vanished_still_fails(self):
        mine = leg_names()[1]
        code, log = self.judge({("example", mine[0]): None}, {mine[0]: True})
        self.assertEqual(code, 1)
        self.assertIn("passes now", log)
        code, log = self.judge({("example", mine[1]): None}, {mine[0]: True})
        self.assertEqual(code, 1)
        self.assertIn("no longer runs", log)

    def test_a_windows_row_is_not_expected_by_a_linux_leg(self):
        windows = ignored_tests.Row("example", "exporters_log", "test:hidden", "daily", "daily-example.txt", "windows")
        mine = leg_names()[1][0]
        with patch.object(ignored_tests, "listed_entries", return_value=[windows]):
            self.assertEqual(ignored_tests.daily_rows(), [])
            self.assertEqual(ignored_tests.daily_rows_groups([windows]), {})
            code, log = self.judge({}, {mine: True})
        self.assertEqual(code, 0, log)
        self.assertNotIn("no longer runs", log)

    def test_a_new_failure_and_an_incomplete_leg_fail(self):
        mine = leg_names()[1][0]
        code, log = self.judge({}, {mine: False})
        self.assertEqual(code, 1)
        self.assertIn("isn't a known failure", log)
        code, log = self.judge({}, None)
        self.assertEqual(code, 1)
        self.assertIn("example#1of2: the test run did not complete", log)


if __name__ == "__main__":
    unittest.main()
