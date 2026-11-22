"""Dyna's switch routes only supported, same-repository events to its pool."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
PARITY = ROOT / ".github/workflows/parity.yml"
BASH = shutil.which("bash")
POSIX_BASH = BASH is not None and not (
    os.name == "nt" and os.path.dirname(os.path.normcase(BASH)).endswith(os.path.normcase(r"\system32")))


class DynaRoutingTests(unittest.TestCase):
    def route(self, event, same_repo="true", enabled="true", lease=None):
        env = dict(os.environ, EVENT=event, SAME_REPO=same_repo, ENABLED=enabled,
                   LEASE=str(int(time.time()) + 3600) if lease is None else lease)
        return subprocess.check_output(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"], env=env, text=True).strip()

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_only_allowed_events_and_same_repository(self):
        for event in ("push", "workflow_dispatch", "pull_request"):
            self.assertEqual(self.route(event), "self_hosted=true", event)
        for event in ("pull_request_target", "workflow_run", "issue_comment", "schedule", ""):
            self.assertEqual(self.route(event), "self_hosted=false", event)
        self.assertEqual(self.route("pull_request", same_repo="false"), "self_hosted=false")

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_switch_and_lease_fail_closed(self):
        self.assertEqual(self.route("push", enabled="false"), "self_hosted=false")
        for lease in ("", "bad", "0"):
            self.assertEqual(self.route("push", lease=lease), "self_hosted=false")

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_route_reads_the_lease_live_else_judges_the_snapshot_at_the_attempts_creation(self):
        now = int(time.time())
        stamp = lambda age: time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(now - age))
        # live lease, snapshot lease, the run's age in seconds, attempt, its start's age, expected
        cases = (("9999999999", "0", 60, 1, 60, "true"),
                 ("0", "9999999999", 60, 1, 60, "false"),
                 # The token can't read Variables: a run that waited 55 minutes keeps the lease it was created with.
                 ("", str(now - 55 * 60 + 12 * 60), 55 * 60, 1, 60, "true"),
                 ("", str(now - 55 * 60 - 1), 55 * 60, 1, 60, "false"),
                 ("", "9999999999", 3601, 1, 60, "false"),
                 # A re-run two hours after the run was created: its own snapshot, judged at the attempt's start.
                 # Lapsed by now, valid at the attempt's start: judged at the start, not now.
                 ("", str(now - 30), 2 * 3600, 2, 60, "true"),
                 ("", str(now - 61), 2 * 3600, 2, 60, "false"),
                 ("", "9999999999", 2 * 3600, 2, 3601, "false"))
        with tempfile.TemporaryDirectory() as directory:
            gh = Path(directory) / "gh"
            gh.write_text('#!/usr/bin/env bash\ncase "$*" in *SELF_HOSTED*) echo "$LIVE_ENABLED";; '
                          '*POOL_UNTIL*) echo "$LIVE";; *runs/7*.run_started_at) echo "$STARTED_AT";; '
                          '*runs/7*.created_at) echo "$CREATED_AT";; *) exit 1;; esac\n')
            gh.chmod(0o755)
            for live, snapshot, age, attempt, started, expected in cases:
                with self.subTest(live=live, snapshot=snapshot, age=age, attempt=attempt):
                    env = dict(os.environ, PATH=f"{directory}:{os.environ['PATH']}", EVENT="workflow_dispatch",
                               SAME_REPO="true", VARS_ENABLED="true", VARS_LEASE=snapshot, LIVE=live,
                               LIVE_ENABLED="true" if live else "", CREATED_AT=stamp(age), STARTED_AT=stamp(started),
                               GITHUB_REPOSITORY="sunrise-org/sunrise", GITHUB_RUN_ID="7",
                               GITHUB_RUN_ATTEMPT=str(attempt), NOW=str(now))
                    env.pop("GITHUB_OUTPUT", None)
                    result = subprocess.run(["bash", ROOT / "tools/ci/dyna_ci_route.sh"], env=env,
                                            capture_output=True, text=True, check=True)
                    self.assertEqual(result.stdout, f"self_hosted={expected}\n")

    def test_every_workflow_routes_through_the_route_script(self):
        # A job that handed the run's lease snapshot straight to dyna_ci_runner.sh judged it against the time the job
        # started: a daily run that waited for GitHub's runners sent its data shards off the pool (2026-11-16).
        for workflow in sorted((ROOT / ".github/workflows").glob("*.yml")):
            text = workflow.read_text()
            if "vars.DYNA_CI_POOL_UNTIL" not in text:
                continue
            with self.subTest(workflow=workflow.name):
                self.assertNotIn("dyna_ci_runner.sh", text)
                self.assertIn('bash tools/ci/dyna_ci_route.sh >> "$GITHUB_OUTPUT"', text)
                self.assertIn("VARS_LEASE: ${{ vars.DYNA_CI_POOL_UNTIL }}", text)
                self.assertIn("actions: read", text)

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_the_daily_routes_fn_replace_check_at_its_start(self):
        # fn-replace-check starts after the SDK's Linux legs, an hour or more into the run: routing there judged the
        # lease snapshot too late and sent every daily's check to GitHub's runners (runs 44065326913, 44145360054).
        from test_ci_concurrency import yaml
        daily = yaml.load((ROOT / ".github/workflows/daily.yml").read_text(), Loader=yaml.BaseLoader)["jobs"]
        route = next(step for step in daily["resolve"]["steps"] if step.get("id") == "route")
        self.assertIn("dyna_ci_route.sh", route["run"])
        self.assertEqual(daily["resolve"]["outputs"]["self_hosted"], "${{ steps.route.outputs.self_hosted }}")
        self.assertEqual(daily["fn-replace-check"]["with"]["self_hosted"], "${{ needs.resolve.outputs.self_hosted }}")
        check = yaml.load((ROOT / ".github/workflows/fn-replace-check.yml").read_text(), Loader=yaml.BaseLoader)
        script = next(step for step in check["jobs"]["route"]["steps"] if step.get("id") == "runner")["run"]
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            for given, status, expected in (("true", 0, "self_hosted=true\n"), ("false", 0, "self_hosted=false\n"),
                                            ("yes", 1, "")):
                with self.subTest(given=given):
                    output.write_text("")
                    result = subprocess.run(["bash", "-e", "-c", script], cwd=ROOT, capture_output=True, text=True,
                                            env=dict(os.environ, GIVEN=given, GITHUB_OUTPUT=str(output)))
                    self.assertEqual(result.returncode, status, result.stdout)
                    self.assertEqual(output.read_text(), expected)

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_snapshot_lease_is_bounded_by_run_age(self):
        # source, lease, created, now, expected: values independent of wall clock.
        cases = [
            ("live", "1100", "900", "1000", True),
            ("live", "950", "900", "1000", False),
            ("snapshot", "950", "900", "1000", True),
            ("snapshot", "899", "900", "1000", False),
            ("snapshot", "950", "900", "4500", True),
            ("snapshot", "950", "900", "4501", False),
            ("snapshot", "9999", "", "1000", False),
            ("snapshot", "9999", "bad", "1000", False),
            ("snapshot", "9999", "1001", "1000", False),
        ]
        for source, lease, created, now, expected in cases:
            with self.subTest(source=source, lease=lease, created=created, now=now):
                env = dict(os.environ, EVENT="push", SAME_REPO="true", ENABLED="true",
                           LEASE=lease, LEASE_SOURCE=source, CREATED=created, NOW=now)
                result = subprocess.check_output(["bash", ROOT / "tools/ci/dyna_ci_runner.sh"],
                                                 env=env, text=True)
                self.assertEqual(result.strip(), f"self_hosted={str(expected).lower()}")

    @unittest.skipIf(os.name == "nt", "the script installs dependencies on Dyna's Ubuntu runner")
    def test_cached_dependencies_export_persistent_paths(self):
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            prefix = cache / "apt-prefix"
            prefix.mkdir()
            (prefix / ".dyna-hash").write_text(hashlib.sha256(script.read_bytes()).hexdigest() + "\n")
            envfile, pathfile = cache / "env", cache / "path"
            # Dyna sets CARGO_HOME and CARGO_TARGET_DIR on the runner, as in session boxes, so the
            # host's shared compiler cache matches; the script keeps them and only puts Cargo's bin on PATH.
            cargo_home = cache / "dyna-cargo"
            env = dict(os.environ, DYNA_CI_BUILD=directory, GITHUB_ENV=str(envfile), GITHUB_PATH=str(pathfile),
                       CARGO_HOME=str(cargo_home))
            subprocess.run(["bash", script], env=env, check=True)
            exports = envfile.read_text()
            self.assertIn(f"RUSTUP_HOME={cache / 'rustup'}\n", exports)
            for key in ("CARGO_TARGET_DIR", "CARGO_HOME", "CARGO_INCREMENTAL"):
                self.assertNotIn(key, exports)
            self.assertIn(str(cargo_home / "bin"), pathfile.read_text())

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_baked_packages_skip_the_download_and_a_partial_image_falls_back(self):
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        for missing, extraction_error in ((False, False), (True, False), (True, True)):
            with self.subTest(missing=missing, extraction_error=extraction_error), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                stubs = root / "stubs"
                stubs.mkdir()
                (stubs / "dpkg-query").write_text(
                    '#!/bin/bash\nif [ "$MISSING" = 1 ] && [ "${!#}" = libudev-dev ]; then exit 1; fi\nprintf installed\n')
                (stubs / "apt-get").write_text(
                    '#!/bin/bash\nprintf "%s\\n" "$*" >> "$APT_CALLS"\n'
                    'for arg in "$@"; do\n'
                    '  case "$arg" in Dir::Cache::archives=*) archives=${arg#*=} ;; esac\n'
                    'done\n'
                    'if [[ " $* " == *" install "* ]]; then touch "$archives/headers.deb"; fi\n')
                (stubs / "dpkg-deb").write_text(
                    '#!/bin/bash\nset -eu\n'
                    'test -f "$2"\n'
                    '[ "$EXTRACTION_ERROR" = 0 ] || exit 42\n'
                    'mkdir -p "$3/usr/include"\n'
                    'printf "fixture header\\n" > "$3/usr/include/fixture.h"\n'
                    'echo extracted >> "$EXTRACTION_RECEIPT"\n')
                for stub in stubs.iterdir():
                    stub.chmod(0o755)
                (root / "build").mkdir()
                env = dict(os.environ, PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}", HOME=str(root / "home"),
                           DYNA_CI_BUILD=str(root / "build"), CARGO_HOME=str(root / "cargo"),
                           GITHUB_ENV=str(root / "env"), GITHUB_PATH=str(root / "path"),
                           MISSING=str(int(missing)), APT_CALLS=str(root / "apt-calls"), DYNA_CI_APT_TRIES="1",
                           EXTRACTION_ERROR=str(int(extraction_error)), EXTRACTION_RECEIPT=str(root / "extracted"))
                result = subprocess.run(["bash", script], env=env, capture_output=True, text=True)
                calls = (root / "apt-calls").read_text() if (root / "apt-calls").exists() else ""
                if missing:
                    self.assertTrue(calls, "a missing package must run the download fallback")
                    if extraction_error:
                        self.assertEqual(result.returncode, 42, result.stderr)
                        self.assertFalse((root / "env").exists())
                        self.assertFalse((root / "build/apt-prefix/.dyna-hash").exists())
                    else:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        self.assertEqual((root / "extracted").read_text(), "extracted\n")
                        exports = dict(line.split("=", 1) for line in (root / "env").read_text().splitlines())
                        prefix = Path(exports["PKG_CONFIG_SYSROOT_DIR"])
                        self.assertEqual((prefix / "usr/include/fixture.h").read_text(), "fixture header\n")
                else:
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(calls, "")
                    self.assertIn("already installed", result.stdout)

    def run_deps_with_stub_apt(self, failures):
        """Run the script with an apt-get that fails `failures` times per command, then succeeds."""
        script = ROOT / "tools/ci/dyna_ci_deps.sh"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stubs = root / "stubs"
            stubs.mkdir()
            (stubs / "apt-get").write_text(
                '#!/usr/bin/env bash\n'
                f'n=$(cat "{root}/count" 2>/dev/null || echo 0); echo $((n + 1)) > "{root}/count"\n'
                f'if [ "$n" -lt {failures} ]; then echo "E: File has unexpected size (mirror sync)" >&2; exit 100; fi\n')
            (stubs / "dpkg-deb").write_text("#!/usr/bin/env bash\n")
            # No baked packages: the rootless download fallback is what these cases exercise.
            (stubs / "dpkg-query").write_text("#!/usr/bin/env bash\nexit 1\n")
            for stub in stubs.iterdir():
                stub.chmod(0o755)
            home = root / "home"
            home.mkdir()
            env = dict(os.environ, PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}", HOME=str(home),
                       DYNA_CI_BUILD=str(root / "build"), CARGO_HOME=str(root / "cargo"),
                       GITHUB_ENV=str(root / "env"), GITHUB_PATH=str(root / "path"),
                       DYNA_CI_APT_PAUSE="0", DYNA_CI_APT_TRIES="3")
            (root / "build").mkdir()
            result = subprocess.run(["bash", script], env=env, capture_output=True, text=True)
            count = int((root / "count").read_text()) if (root / "count").exists() else 0
            return result, count

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_apt_is_retried_a_bounded_number_of_times_and_then_succeeds(self):
        # The first command (update) fails twice, then both commands pass: 2 failures + 2 successes.
        result, count = self.run_deps_with_stub_apt(failures=2)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(count, 4)
        self.assertEqual(result.stderr.count("retrying in"), 2)

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash")
    def test_apt_that_keeps_failing_fails_with_the_real_error_after_the_last_try(self):
        result, count = self.run_deps_with_stub_apt(failures=99)
        self.assertEqual(result.returncode, 100)
        self.assertEqual(count, 3)
        self.assertEqual(result.stderr.count("E: File has unexpected size"), 3)
        self.assertIn("timeout failed after 3 tries (exit 100)", result.stderr)

    def test_lint_enforces_api_boundary_under_central_selection(self):
        from test_ci_runners import jobs
        lint = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["lint"]
        self.assertIn("if: ${{ always() && !cancelled()", lint)
        # Pin the executable contract, including the selection and source-only mode:
        # deleting the step or making it unconditional must fail this test.
        step = lint.split("- name: Check mod API boundary\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("needs.changes.outputs.rust_required != 'false'", step)
        self.assertIn("python3 tools/ci/check_api_boundary.py --source-only", step)

    def test_workflow_wires_route_and_keeps_privileges_off_pool(self):
        from test_ci_runners import jobs
        workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()
        blocks = dict(jobs(workflow))
        route = blocks["changes"]
        for item in ("vars.DYNA_CI_SELF_HOSTED", "vars.DYNA_CI_POOL_UNTIL", "github.event_name",
                     "github.event.pull_request.head.repo.full_name == github.repository", "dyna_ci_route.sh"):
            self.assertIn(item, route)
        for name, label in (("verify", "dyna-large"), ("verify-static", "dyna-medium")):
            block = blocks[name]
            self.assertIn("needs: changes", block)
            self.assertIn(f"needs.changes.outputs.self_hosted == 'true' && '{label}'", block)
            self.assertNotIn("secrets.", block)
            self.assertNotIn("permissions:", block)
        # The watch for a dead pool runs off the pool, as one scheduled sweep, never as a job of each run.
        self.assertNotIn("pool-watch", blocks)
        self.assertIn("runs-on: ubuntu-latest", (ROOT / ".github/workflows/dyna-pool-watch.yml").read_text())
        self.assertIn("permissions:\n  contents: read", workflow)

    def test_split_jobs_are_independent_fail_closed_and_share_routing(self):
        from test_ci_runners import jobs
        blocks = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))
        runtime = blocks["verify"]
        static = blocks["verify-static"]
        # verify waits for the heavy gate (tools/ci/pr_gate.py); verify-static runs on every pull request.
        conditions = {
            id(runtime): "${{ always() && !cancelled() && needs.changes.result != 'skipped' && "
                         "(needs.changes.result != 'success' || needs.changes.outputs.heavy == 'true') }}",
            id(static): "${{ always() && !cancelled() && needs.changes.result != 'skipped' }}",
        }
        for block in (runtime, static):
            header, steps = block.split("    steps:\n", 1)
            fields = dict(line.strip().split(":", 1) for line in header.splitlines()
                          if line.startswith("    ") and ":" in line and not line.lstrip().startswith("#"))
            self.assertEqual(fields["needs"].strip(), "changes")
            self.assertIn("timeout-minutes", fields)
            self.assertEqual(fields["if"].strip(), conditions[id(block)])
            first = steps.split("\n      - ", 1)[0]
            guard = dict(line.strip().split(":", 1) for line in first.splitlines()
                         if line.startswith("        ") and ":" in line)
            self.assertEqual(guard["if"].strip(), "needs.changes.result != 'success'")
            self.assertEqual(guard["run"].strip(), "exit 1")
        # verify-static routes as verify does, except that a pull request outside the gate and a contributor's
        # pull request stay on GitHub's runners, and our pool gives it a medium runner (it compiles nothing heavy).
        for field, outside, fork in (
                ("runs-on", "needs.changes.outputs.heavy == 'false' && 'ubuntu-latest' || ",
                 "needs.changes.outputs.trusted != 'true' && 'ubuntu-latest' || "),
                ("RUNNER_KIND", "needs.changes.outputs.heavy == 'false' && 'github' || ",
                 "needs.changes.outputs.trusted != 'true' && 'github' || ")):
            shared = runtime.split(f"{field}:", 1)[1].split("\n", 1)[0]
            if field == "runs-on":
                shared = shared.replace("'ubuntu-latest') || ", "'ubuntu-latest') || " + fork, 1)
                shared = shared.replace("needs.changes.outputs.self_hosted == 'true' && 'dyna-large'",
                                        "needs.changes.outputs.self_hosted == 'true' && 'dyna-medium'", 1)
            else:
                shared = shared.replace("${{ ", "${{ " + fork, 1)
            shared = shared.replace("${{ ", "${{ " + outside, 1)
            self.assertEqual(static.split(f"{field}:", 1)[1].split("\n", 1)[0], shared)
        self.assertIn("- name: No Rust work selected", runtime)
        self.assertNotIn("history_fetch.sh", runtime)
        self.assertIn("run: bash tools/ci/history_fetch.sh", static)
        self.assertNotIn("cargo test", static)
        self.assertNotIn("Swatinem/rust-cache", static)
        self.assertNotIn("runner-packages.txt", static)
        self.assertIn("python3 tools/ci/ignored_shards.py --prepare sunrise-mod-package $JOBS", runtime)
        self.assertIn("cc -shared -fPIC -O2 -Wall -Wextra -Werror tools/ci/network_guard.c -o target/network_guard.so", runtime)
        self.assertIn("python3 tools/ci/run_test_binaries.py --jobs 3 --time-limits 2,5 --network-guard target/network_guard.so --no-data-skips -- cargo test $JOBS --profile test --workspace --lib --bins --tests", runtime)
        self.assertIn("cargo test $JOBS --no-fail-fast --profile test --workspace --doc", runtime)
        tools = static.split("- name: Install Rust tools for isolated tooling fixtures\n", 1)[1].split("\n      - ", 1)[0]
        # Dyna's image has no Cargo: a reused main push (nothing selected) still runs the Cargo-using checks there.
        self.assertIn("if: needs.changes.outputs.rust_required != 'false' || needs.changes.outputs.tooling_required != 'false'"
                      " || env.RUNNER_KIND == 'dyna'\n", tools)
        self.assertIn("--component rustfmt --component clippy", tools)
        # GITHUB_PATH, not a shell-local export, carries Cargo into later steps.
        self.assertIn('echo "$CARGO_HOME/bin" >> "$GITHUB_PATH"', tools)
        self.assertLess(static.index("- name: Install Rust tools for isolated tooling fixtures"),
                        static.index("- name: Python tests of the repository's tools"))

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash and POSIX paths")
    def test_pull_requests_select_from_the_api_list_and_fetch_less_history(self):
        sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
        import yaml
        jobs = yaml.load((ROOT / ".github/workflows/dyna-ci.yml").read_text(), Loader=yaml.BaseLoader)["jobs"]
        changes = {step.get("name"): step for step in jobs["changes"]["steps"]}
        listing = changes["List this pull request's files"]
        fetch = changes["Fetch this commit's history for selection"]
        select = changes["Select checks once for this pull request"]
        # Ordinary pull requests only: main's pushes, rollups and syncs (heavy) select nothing, so every gate job keeps
        # running there.
        ordinary = "github.event_name == 'pull_request' && steps.gate.outputs.heavy == 'false'"
        for step in (listing, select):
            self.assertEqual(step["if"], ordinary)
        for name in ("rust_required", "tooling_required"):
            self.assertEqual(jobs["changes"]["outputs"][name], f"${{{{ steps.reuse.outputs.reused == 'true' && 'false' || steps.selection.outputs.{name} || 'true' }}}}")
        # The list is the comparison of this run's two commits, not the pull request's current files.
        self.assertNotIn("PR", listing["env"])
        self.assertEqual(listing["env"]["BASE_SHA"], "${{ github.event.pull_request.base.sha }}")
        self.assertEqual(listing["env"]["HEAD_SHA"], "${{ github.event.pull_request.head.sha }}")
        # A list that fails or times out falls back to the history, never fails the job.
        self.assertEqual(listing["continue-on-error"], "true")
        self.assertEqual(fetch["if"], ordinary + " && steps.files.outputs.listed != 'true'")
        self.assertEqual(fetch["env"]["HISTORY_FILTER"], "tree:0")
        self.assertEqual(select["env"]["FILES"],
                         "${{ steps.files.outputs.listed == 'true' && format('{0}/pr-files', runner.temp) || '' }}")
        self.assertEqual(select["env"]["ENTRIES"], "${{ steps.files.outputs.entries }}")
        # The selection step as written: the API's list when there is one, else git's diff of base...head.
        with tempfile.TemporaryDirectory() as directory:
            def git(*args):
                return subprocess.check_output(["git", "-C", directory, "-c", "user.name=t", "-c", "user.email=t@t",
                                                "-c", "commit.gpgsign=false", *args], text=True).strip()
            git("init", "-q")
            Path(directory, "lib.rs").write_text("fn a() {}\n")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            Path(directory, "lib.rs").write_text("fn b() {}\n")
            git("commit", "-qam", "head")
            head = git("rev-parse", "HEAD")
            listing_file = Path(directory, "pr-files")
            listing_file.write_bytes(b"M\0docs/setup.md\0")
            # A listing that lost a record (the producer printed 2 entries, one remains) selects everything.
            for files, entries, rust in (("", "", "true"), (str(listing_file), "1", "false"),
                                         (str(listing_file), "2", "true")):
                with self.subTest(files=files, entries=entries):
                    output = Path(directory, "output")
                    output.write_text("")
                    env = dict(os.environ, BASE_SHA=base, HEAD_SHA=head, FILES=files, ENTRIES=entries, GITHUB_OUTPUT=str(output))
                    subprocess.run(["bash", "-e", "-c", select["run"].replace("tools/ci/", f"{ROOT}/tools/ci/")],
                                   cwd=directory, env=env, check=True)
                    self.assertIn(f"rust_required={rust}\n", output.read_text())
        # format skips rustfmt when no Rust changed; the repository checks still run.
        format_steps = {step.get("name"): step for step in jobs["format"]["steps"]}
        self.assertNotIn("if", format_steps["Check repository artifacts and workflow YAML"])
        for name in ("Install rustfmt on the pinned Rust", "Check formatting"):
            self.assertEqual(format_steps[name]["if"], "needs.changes.outputs.rust_required != 'false'")
        # verify-static: a pull request off our pool takes the history without file contents; pushes take all of it.
        static = next(step for step in jobs["verify-static"]["steps"] if step.get("name") == "Fetch this commit's history")
        self.assertEqual(static["env"]["HISTORY_FILTER"],
                         "${{ github.event_name == 'pull_request' && env.RUNNER_KIND != 'dyna' && 'blob:none' || '' }}")

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash and POSIX paths")
    def test_verify_reuses_persistent_target_without_replacing_existing_paths(self):
        import textwrap
        from test_ci_runners import jobs
        runtime = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        setup = runtime.split("- name: Use persistent verify build artifacts\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("needs.changes.outputs.rust_required != 'false' && needs.changes.outputs.self_hosted == 'true'", setup)
        script = textwrap.dedent(setup.split("        run: |\n", 1)[1])
        restore = runtime.split("- name: Restore unchanged source timestamps\n", 1)[1].split("\n      - ", 1)[0]
        self.assertIn("needs.changes.outputs.rust_required != 'false'", restore)
        self.assertIn("python3 tools/ci/source_mtimes.py target/ci-source-mtimes-v2.json", restore)
        self.assertLess(runtime.index("- name: Use persistent verify build artifacts"),
                        runtime.index("- name: Restore unchanged source timestamps"))
        self.assertLess(runtime.index("- name: Restore unchanged source timestamps"),
                        runtime.index("- name: Build the runtime holder test actor"))
        for case in ("absent", "correct-link", "directory", "file", "wrong-link", "dangling-link"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                persistent = root / "persistent"
                persistent.mkdir()
                target = root / "target"
                other = root / "other"
                other.mkdir()
                if case == "correct-link":
                    target.symlink_to(persistent)
                elif case == "directory":
                    target.mkdir()
                    (target / "keep").write_text("retained")
                elif case == "file":
                    target.write_text("retained")
                elif case in ("wrong-link", "dangling-link"):
                    target.symlink_to(other if case == "wrong-link" else root / "missing")
                before = target.lstat() if case != "absent" else None
                before_link = os.readlink(target) if case in ("wrong-link", "dangling-link") else None
                env = dict(os.environ, CARGO_TARGET_DIR=str(persistent))
                result = subprocess.run(["bash", "-e", "-c", script], cwd=root, env=env,
                                        capture_output=True, text=True)
                if case in ("absent", "correct-link"):
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(target.resolve(), persistent)
                else:
                    self.assertNotEqual(result.returncode, 0)
                    after = target.lstat()
                    # Following a symlink may update atime without modifying it.
                    fields = ("st_ino", "st_dev", "st_mode", "st_size", "st_mtime_ns")
                    self.assertEqual(tuple(getattr(after, field) for field in fields),
                                     tuple(getattr(before, field) for field in fields))
                    if case in ("wrong-link", "dangling-link"):
                        self.assertEqual(os.readlink(target), before_link)
                    if case == "directory":
                        self.assertEqual((target / "keep").read_text(), "retained")
                    elif case == "file":
                        self.assertEqual(target.read_text(), "retained")

    @unittest.skipUnless(POSIX_BASH and os.name != "nt", "needs the runner's bash and POSIX paths")
    def test_stage_artifact_handoff_selects_normal_binary_and_rejects_unsafe_paths(self):
        import json
        import textwrap
        from test_ci_runners import jobs
        runtime = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        build = runtime.split("- name: Build the source stage generator (test profile)\n", 1)[1].split("\n      - ", 1)[0]
        script = textwrap.dedent(build.split("        run: |\n", 1)[1])
        for case in ("normal-and-harness", "harness-only", "newline-path"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                stubs = root / "bin"
                stubs.mkdir()
                cargo = stubs / "cargo"
                cargo.write_text("#!/usr/bin/env python3\nimport os, sys\nsys.stdout.write(os.environ['STAGE_ARTIFACTS'])\n")
                cargo.chmod(0o755)
                normal = root / ("line\nbreak" if case == "newline-path" else "sunrise-stage")
                normal.touch()
                harness = root / "sunrise-stage-harness"
                harness.touch()
                messages = [{"reason": "compiler-artifact", "target": {"name": "sunrise-stage", "kind": ["bin"]},
                             "profile": {"test": True}, "executable": str(harness)}]
                if case != "harness-only":
                    messages.append({"reason": "compiler-artifact", "target": {"name": "sunrise-stage", "kind": ["bin"]},
                                     "profile": {"test": False}, "executable": str(normal)})
                environment = root / "github-env"
                env = dict(os.environ, PATH=f"{stubs}{os.pathsep}{os.environ['PATH']}",
                           RUNNER_TEMP=str(root), GITHUB_ENV=str(environment), JOBS="",
                           STAGE_ARTIFACTS="".join(json.dumps(message) + "\n" for message in messages))
                result = subprocess.run(["bash", "-e", "-c", script], cwd=ROOT, env=env,
                                        capture_output=True, text=True)
                if case == "normal-and-harness":
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(environment.read_text(), f"SUNRISE_STAGE_TOOL={normal.resolve()}\n")
                else:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(environment.exists())

    def assert_positive_stage_export(self, build):
        command = "python tools/ci/test_stage_catalog.py -v"
        name = "- name: The source stages with this commit's build data\n"
        step = build.split(name, 1)[1].split("\n      - ", 1)[0]
        # Inspect active scalar values in this named step, not command text in
        # comments or another step. Require the exact executable invocation.
        fields = dict(line.strip().split(":", 1) for line in step.splitlines()
                      if line.startswith("        ") and ":" in line and not line.lstrip().startswith("#"))
        self.assertEqual(fields.get("run", "").strip(), command)
        self.assertEqual(fields.get("if", "").strip(), "matrix.name == 'linux'")
        self.assertEqual(fields.get("SUNRISE_STAGE_TOOL", "").strip(), "target/prebuilt/tools/release/sunrise-stage")
        self.assertEqual(fields.get("continue-on-error", "false").strip(), "false")
        self.assertEqual(build.count(command), 1)
        # After the build that compiles the tool, and before the leg's assets upload: a tree that does not stage
        # publishes no SDK.
        self.assertLess(build.index("- name: Build and collect\n"), build.index(name))
        self.assertLess(build.index(name), build.index("uses: actions/upload-artifact@v4"))

    def test_positive_stage_export_runs_in_the_sdk_linux_leg_and_not_in_verify(self):
        from test_ci_runners import jobs
        build = dict(jobs((ROOT / ".github/workflows/prebuilt-game-build.yml").read_text()))["build"]
        self.assert_positive_stage_export(build)
        self.assertNotIn("run: python3 tools/ci/test_stage_catalog.py", (ROOT / ".github/workflows/dyna-ci.yml").read_text())

    def test_the_surface_report_runs_after_the_stage_build_on_pull_requests_and_dispatches(self):
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify"]
        name = "- name: Report the mod API surface change\n"
        self.assertEqual(verify.count(name), 1)
        step = verify.split(name, 1)[1].split("\n      - ", 1)[0]
        fields = {}
        for line in step.splitlines():
            if ":" in line and not line.lstrip().startswith("#"):
                key, value = line.strip().split(":", 1)
                fields[key] = value.strip()
        self.assertEqual(fields.get("run"), "python3 tools/ci/surface_report.py")
        self.assertIn("steps.stage.outcome == 'success'", fields.get("if", ""))
        self.assertIn("github.event_name == 'pull_request' || inputs.pr != ''", fields.get("if", ""))
        self.assertEqual(fields.get("EVENT"), "${{ github.event_name }}")
        self.assertEqual(fields.get("PR"), "${{ inputs.pr }}")
        # The base is the checkout's own merge parent: the step reads no API.
        self.assertNotIn("GH_TOKEN", fields)
        self.assertNotIn("continue-on-error", fields)
        # It needs the tool the stage build puts in SUNRISE_STAGE_TOOL.
        self.assertLess(verify.index("- name: Build the source stage generator (test profile)"), verify.index(name))

    def test_positive_stage_guard_rejects_commented_command_noop_and_ignored_failure(self):
        from test_ci_runners import jobs
        build = dict(jobs((ROOT / ".github/workflows/prebuilt-game-build.yml").read_text()))["build"]
        command = "        run: python tools/ci/test_stage_catalog.py -v"
        for replacement in (
            "        # run: python tools/ci/test_stage_catalog.py -v\n        run: true",
            "        run: '# python tools/ci/test_stage_catalog.py -v'",
            command + "\n        continue-on-error: true",
        ):
            with self.subTest(replacement=replacement), self.assertRaises(AssertionError):
                self.assert_positive_stage_export(build.replace(command, replacement, 1))

    def test_optional_map_upload_is_nonblocking_and_reports_its_failure(self):
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify-static"]
        upload = verify.split("        id: mod_host_map_upload\n", 1)[1].split("\n      - ", 1)[0]
        fields = dict(line.strip().split(":", 1) for line in upload.splitlines()
                      if line.startswith("        ") and ":" in line and not line.lstrip().startswith("#"))
        self.assertEqual(fields["continue-on-error"].strip(), "true")
        self.assertIn("uses: actions/upload-artifact@", upload)
        report = verify.split("- name: Report optional mod-host map upload failure\n", 1)[1].split("\n      - ", 1)[0]
        condition = report.split("if:", 1)[1].split("\n", 1)[0]
        self.assertIn("!cancelled()", condition)
        self.assertIn("steps.mod_host_map_upload.outcome == 'failure'", condition)
        self.assertIn("::warning::", report)
        self.assertIn('>> "$GITHUB_STEP_SUMMARY"', report)

    def test_tooling_tests_run_only_after_static_setup_succeeded(self):
        # Failed setup must not produce secondary failures from compiler fixtures (#2920).
        from test_ci_runners import jobs
        verify = dict(jobs((ROOT / ".github/workflows/dyna-ci.yml").read_text()))["verify-static"]
        step = verify.split("- name: Python tests of the repository's tools\n", 1)[1].split("\n      - ", 1)[0]
        condition = step.split("if:", 1)[1].split("\n", 1)[0]
        self.assertNotIn("success()", condition)
        self.assertIn("!cancelled()", condition)
        self.assertIn("steps.setup.outcome == 'success'", condition)
        self.assertNotIn("always()", condition)

    @unittest.skipUnless(POSIX_BASH, "needs a bash that is not WSL's")
    def test_missing_prepared_data_fails_the_leg(self):
        from test_ci_runners import jobs
        import textwrap
        workflow = (ROOT / ".github/workflows/ignored-tests.yml").read_text()
        shard = dict(jobs(workflow))["ignored-tests-shards"]
        script = textwrap.dedent(shard.split("        run: |\n          for dir", 1)[1]
                                .split("      - name: Generate", 1)[0])
        script = "for dir " + script
        with tempfile.TemporaryDirectory() as directory:
            script = script.replace("/cache/sunrise-local", directory + "/missing")
            summary = Path(directory) / "summary"
            for event in ("push", "workflow_dispatch"):
                env = dict(os.environ, EVENT=event, GITHUB_STEP_SUMMARY=str(summary))
                run = subprocess.run(["bash", "-e", "-c", script], env=env,
                                     capture_output=True, text=True)
                self.assertEqual(run.returncode, 1)
                self.assertIn(f"Prepared data unavailable: minecraft ({event})", summary.read_text())
                summary.unlink()

    def test_telemetry_ingest_checks_require_manual_dispatch(self):
        from test_ci_runners import triggers
        workflow = (ROOT / ".github/workflows/telemetry-server.yml").read_text()
        self.assertEqual(triggers(workflow), {"workflow_dispatch"})

    def test_schedule_only_dispatches_and_parity_has_no_gate_entry_point(self):
        from test_ci_runners import jobs, triggers
        workflow = (ROOT / ".github/workflows/dyna-ci.yml").read_text()
        self.assertEqual(triggers(workflow), {"pull_request", "push", "workflow_dispatch", "schedule"})
        blocks = dict(jobs(workflow))
        dispatcher = blocks["nightly-dispatch"]
        self.assertIn("github.event_name == 'schedule'", dispatcher)
        self.assertIn("runs-on: ubuntu-latest", dispatcher)
        self.assertIn('gh workflow run dyna-ci.yml --repo "$GITHUB_REPOSITORY" --ref main', dispatcher)
        self.assertIn("if: github.event_name != 'schedule'", blocks["changes"])
        self.assertNotIn("vanilla-parity", blocks)
        workflows = ROOT / ".github/workflows"
        gate = (workflows / "dyna-ci.yml").read_text()
        self.assertNotIn("inputs.parity", gate)
        self.assertNotIn("inputs.harnesses", gate)
        callers = [path.name for path in workflows.glob("*.yml") if "uses: ./.github/workflows/parity.yml" in path.read_text()]
        self.assertEqual(callers, ["daily.yml"])
        self.assertNotIn("vanilla-parity", dict(jobs((workflows / "ci.yml").read_text())))
        parity = dict(jobs(PARITY.read_text()))["vanilla-parity"]
        self.assertIn("runs-on: dyna-large", parity)
        self.assertIn("persist-credentials: false", parity)
        self.assertIn("ref: ${{ inputs.commit }}", parity)

    def test_daily_harnesses_client_lib_tests_are_built_before_the_harnesses(self):
        from test_ci_runners import jobs
        parity = dict(jobs(PARITY.read_text()))["vanilla-parity"]
        step = parity.split("- name: Build the daily harnesses' game probes\n", 1)[1]
        script = textwrap.dedent(step.split("        run: |\n", 1)[1].split("\n      - ", 1)[0])
        with tempfile.TemporaryDirectory() as temporary:
            tree, bin_dir = Path(temporary, "tree"), Path(temporary, "bin")
            (tree / "tools").mkdir(parents=True)
            bin_dir.mkdir()
            calls = Path(temporary, "calls")
            cargo = bin_dir / "cargo"
            cargo.write_text(f"#!/bin/sh\necho \"$*\" >> {calls}\n")
            cargo.chmod(0o755)
            # Two packages, one Cargo run each, as harnesses write them; a profile of its own builds elsewhere.
            (tree / "tools" / "test_lib_parity.py").write_text(
                'run(["cargo", "test", "--locked", "-p", "sunrise-client-lib", "--lib", "x"])\n')
            (tree / "tools" / "test_game_parity.py").write_text(
                "run(['cargo', 'test', '-p', 'sunrise-client', '--lib', 'y'])\n")
            (tree / "tools" / "test_fast_parity.py").write_text(
                'run(["cargo", "test", "--profile", "fast", "-p", "sunrise-world", "--lib", "z"])\n')
            # A package beside the client's, as dragon_contacts runs sunrise-gameplay's lib tests.
            (tree / "tools" / "test_gameplay_parity.py").write_text(
                "run(['cargo', 'test', '--locked', '-p', 'sunrise-gameplay', '--lib', 'w'])\n")
            env = {**os.environ, "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                   "PARITY_LOG_DIR": temporary}
            subprocess.run(["bash", "-e", "-c", script], cwd=tree, env=env, check=True)
            self.assertEqual(calls.read_text().splitlines(), ["test --locked --no-run -p sunrise-client --lib",
                                                             "test --locked --no-run -p sunrise-client-lib --lib",
                                                             "test --locked --no-run -p sunrise-gameplay --lib"])

class PersistentTargetTests(unittest.TestCase):
    def test_every_job_that_can_build_on_a_self_hosted_runner_keeps_the_source_snapshot_first(self):
        # A job that builds in the runner's persistent target without tools/ci/source_mtimes.py leaves artifacts of
        # its tree under a snapshot of another, which a later job then judges fresh: R55's verify linked a
        # sunrise-mod-surface from a tree before 09339ddb31 (2026-11-19). The first step that can compile (cargo, or a
        # repository tool that runs it) must come after the snapshot.
        import re
        from test_ci_concurrency import yaml
        builds = re.compile(r"\bcargo (build|test|run|clippy|check|doc|fetch)\b|tools/ci/check_ignored_test_rows\.py|"
                            r"tools/ci/check_machine_sample\.py|tools/parity\.py|tools/prebuilt_game\.py|xtask")
        checked = 0
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            for name, job in yaml.load(path.read_text(), Loader=yaml.BaseLoader)["jobs"].items():
                if not re.search(r"dyna|DYNA", str(job.get("runs-on", ""))):
                    continue
                steps = [str(step.get("run", "")) for step in job.get("steps", [])]
                first = next((index for index, run in enumerate(steps) if builds.search(run)), None)
                if first is None:
                    continue
                checked += 1
                with self.subTest(workflow=path.name, job=name):
                    self.assertTrue(any("tools/ci/source_mtimes.py" in run for run in steps[:first]))
        self.assertGreater(checked, 8)


class ParityWorkflowTests(unittest.TestCase):
    def setUp(self):
        from test_ci_concurrency import yaml
        self.steps = {step.get("name"): step for step in
                      yaml.load(PARITY.read_text(), Loader=yaml.BaseLoader)["jobs"]["vanilla-parity"]["steps"]}
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.env = {**os.environ, "DYNA_CI_BUILD": str(self.root), "RUNNER_TEMP": str(self.root),
                    "GITHUB_ENV": str(self.root / "env"), "GITHUB_STEP_SUMMARY": str(self.root / "public-summary"),
                    "GITHUB_RUN_ID": "77", "GITHUB_RUN_ATTEMPT": "2", "COMMIT": "current"}
        ci = self.root / "tools/ci"
        ci.mkdir(parents=True)
        shutil.copy(ROOT / "tools/ci/parity_verdict.py", ci)
        self.slot = self.root / "parity-keep/77-2-bucket-0"

    def step(self, name):
        return subprocess.run(["bash", "-e", "-c", self.steps[name]["run"]], cwd=self.root, env=self.env,
                              capture_output=True, text=True)

    def initialise(self):
        result = self.step("Start a fresh private log directory for this attempt")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.env.update([line.split("=", 1) for line in (self.root / "env").read_text().splitlines()])
        self.assertEqual(Path(self.env["PARITY_LOG_DIR"]), self.slot)

    def test_a_failed_probe_build_shows_its_error_headings_not_its_text(self):
        step = self.steps["Show a failed probe build's compiler errors"]
        self.assertEqual(self.steps["Build the daily harnesses' game probes"]["id"], "game-probes")
        self.assertEqual(self.steps["Build parity probes"]["id"], "probes")
        self.assertIn("steps.game-probes.outcome == 'failure'", step["if"])
        self.initialise()
        (self.slot / "game-probes.log").write_text(
            "   Compiling sunrise-client-lib\nerror[E0425]: cannot find value `PRIVATE` in this scope\n"
            "  --> crates/sunrise-client/src/lib.rs:1:1\nprivate source excerpt\n"
            "error[E0609]: no field `PRIVATE`\n  --> crates/sunrise-client/src/untracked_generated_probe.rs:3:9\n"
            "error: could not compile\n")
        result = subprocess.run(["bash", "-e", "-c", step["run"]], cwd=ROOT, capture_output=True, text=True,
                                env=dict(self.env, FAILED="game-probes"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("game-probes failed:", result.stdout)
        self.assertIn("compiler: error[E0425]: custom diagnostic payload withheld", result.stdout)
        self.assertIn("compiler:   --> crates/sunrise-client/src/lib.rs:1:1", result.stdout)
        self.assertIn("compiler:   --> location withheld (not a tracked Rust source)", result.stdout)
        for private in ("PRIVATE", "private source excerpt", "Compiling", "untracked_generated_probe"):
            self.assertNotIn(private, result.stdout)

    def test_a_failed_shares_logs_are_kept_beyond_the_slot(self):
        step = self.steps["Keep a failed share's logs beyond this slot"]
        self.assertEqual(step["if"], "failure()")
        self.initialise()
        (self.slot / "parity.log").write_text("private mismatch")
        keep = self.root / "keep"
        self.env["DYNA_CI_KEEP"] = str(keep)
        result = self.step("Keep a failed share's logs beyond this slot")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((keep / "parity-failed-logs/77-2-bucket-0/parity.log").read_text(), "private mismatch")
        self.assertEqual([path.name for path in (keep / "parity-failed-logs").iterdir()], ["77-2-bucket-0"])
        self.assertNotIn("private", result.stdout)

    def test_every_share_prunes_expired_kept_logs_and_keeps_fresh_ones(self):
        cleanup = self.steps["Delete this job's vanilla-derived scratch"]
        self.assertEqual(cleanup["if"], "always()")
        keep = self.root / "keep"
        stale, fresh = keep / "parity-failed-logs/70-1-bucket-0", keep / "parity-failed-logs/77-2-bucket-0"
        for path in (stale, fresh):
            path.mkdir(parents=True)
        os.utime(stale, (0, 0))
        # A green share: nothing captured, the expired copy still goes.
        result = subprocess.run(["bash", "-e", "-c", cleanup["run"]], cwd=self.root, capture_output=True, text=True,
                                env=dict(self.env, DYNA_CI_KEEP=str(keep)))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(stale.exists())
        self.assertTrue(fresh.exists())
        # A host whose keep holds no captured logs is left as it is.
        result = subprocess.run(["bash", "-e", "-c", cleanup["run"]], cwd=self.root, capture_output=True, text=True,
                                env=dict(self.env, DYNA_CI_KEEP=str(self.root / "empty-keep")))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "empty-keep").exists())

    def test_an_early_failure_cannot_report_a_previous_slots_result(self):
        # Both an older attempt and stale state in the current directory survive until the first workflow step.
        for path in (self.slot.with_name("77-1-bucket-0"), self.slot):
            path.mkdir(parents=True)
            (path / "parity.log").write_text("FAIL loot_old 3s\n0 passed, 1 failed in 1.0 min.\n")
            (path / "selection.json").write_text('["loot_old"]')
            (path / "exit").write_text("1\n")
        self.initialise()
        self.assertFalse((self.slot / "parity.log").exists())
        self.assertTrue((self.slot.with_name("77-1-bucket-0") / "parity.log").exists())
        # A setup step failed before any harness could run. The always() reporter still runs.
        self.assertEqual(self.steps["Write the harnesses' verdict"]["if"], "always()")
        result = self.step("Write the harnesses' verdict")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        report = json.loads((self.root / "parity.json").read_text())
        self.assertEqual(report, {"passed": [], "failed": [], "complete": False, "selected": [],
                                  "bucket": None, "of": 1,
                                  "scope": {"run_id": "77", "run_attempt": "2", "commit": "current"}})
        self.assertEqual(self.steps["Upload the harnesses' verdict"]["with"]["name"],
                         "daily-report-parity-${{ github.run_attempt }}-${{ matrix.bucket }}")

    def test_parity_public_output_is_names_and_results_only(self):
        self.initialise()
        (self.root / "tools/parity.py").write_text(
            "import os, sys\nfrom pathlib import Path\n"
            "assert {'--all', '--skip-named-only'} <= set(sys.argv)\n"
            "Path(sys.argv[-1]).write_text('[\"loot_a\", \"loot_b\"]')\n"
            "Path(os.environ['GITHUB_STEP_SUMMARY']).write_text('private retry reason')\n"
            "print('PASS loot_a 3s')\nprint('FAIL loot_b 2s  (private path)')\n"
            "print('PASS private_value 2s')\nprint('RETRIED loot_b: private retry reason')\n"
            "print('1 passed, 1 failed in 1.0 min.')\nprint('private mismatch', file=sys.stderr)\nsys.exit(1)\n")
        logs = self.root / ".local/parity-runner/logs"
        logs.mkdir(parents=True)
        (logs / "loot_b.log").write_text("private mismatch")
        (logs / "loot_a.first-run.log").write_text("private retry reason")
        result = self.step("Run every automatic harness")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout.splitlines(), ["Parity exited 1"])
        self.assertEqual(result.stderr, "")
        result = self.step("Write the harnesses' verdict")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["PASS loot_a", "FAIL loot_b"])
        self.assertEqual(result.stderr, "")
        self.assertFalse((self.root / "public-summary").exists())
        self.assertIn("private mismatch", (self.slot / "parity.log").read_text())
        self.assertEqual((self.slot / "loot_b.log").read_text(), "private mismatch")
        self.assertEqual((self.slot / "loot_a.first-run.log").read_text(), "private retry reason")
        self.assertEqual((self.root / "parity-private-summary").read_text(), "private retry reason")

    def test_merging_verdicts_keeps_current_failures_beside_bad_evidence(self):
        from test_ci_concurrency import expression_value, yaml
        verdict = yaml.load(PARITY.read_text(), Loader=yaml.BaseLoader)["jobs"]["verdict"]
        step = next(step for step in verdict["steps"] if step.get("name") == "Merge the shares' verdicts")
        scope = {"run_id": "77", "run_attempt": "2", "commit": "selected-main-commit"}
        expected_commit = expression_value(step["env"]["COMMIT"].strip("${} "), {}, inputs={"commit": scope["commit"]})
        env = dict(self.env, SHARDED="true", BUCKETS="3", COMMIT=expected_commit, GITHUB_SHA="workflow-source")
        plan = self.root / "plan/selection.json"
        plan.parent.mkdir()
        plan.write_text('["loot_a", "loot_b", "loot_c"]')
        for bucket, name in enumerate(("loot_a", "loot_b", "loot_c")):
            report = self.root / f"shares/artifact-{bucket}/parity.json"
            report.parent.mkdir(parents=True)
            run = scope | {"run_attempt": "1"} if bucket == 1 else scope
            report.write_text(json.dumps({"bucket": bucket, "of": 3, "selected": [name], "passed": [],
                                          "failed": [name], "complete": True, "scope": run}) if bucket != 2 else "{")
        result = subprocess.run(["bash", "-e", "-c", step["run"]], cwd=self.root, env=env,
                                capture_output=True, text=True, check=True)
        self.assertEqual(result.stdout.splitlines(), ["FAIL loot_a"])
        merged = json.loads((self.root / "parity.json").read_text())
        self.assertEqual((merged["passed"], merged["failed"], merged["complete"], merged["scope"]),
                         ([], ["loot_a"], False, scope))
        self.assertEqual(merged["shares"], {"reported": 1, "expected": 3})

    def test_vanilla_derived_scratch_is_deleted_when_the_job_ends(self):
        self.initialise()
        build = self.root
        target = build / "target/parity"
        target.mkdir(parents=True)
        (target / "probe").write_text("binary")
        for path in ("parity-out/parity-runner/oracle-cache/a.stdout", "parity-tmp/world/level.dat",
                     "vanilla/job/sunrise-client/x", "vanilla/prepared/key/sunrise-client/x"):
            (build / path).parent.mkdir(parents=True, exist_ok=True)
            (build / path).write_text("vanilla")
        (build / "parity-out/parity-target").symlink_to(target)
        (self.slot / "loot_b.log").write_text("private mismatch")
        step = self.steps["Delete this job's vanilla-derived scratch"]
        self.assertEqual(step["if"], "always()")
        result = self.step("Delete this job's vanilla-derived scratch")
        self.assertEqual(result.returncode, 0, result.stderr)
        for gone in ("parity-out", "parity-tmp", "vanilla/job"):
            self.assertFalse((build / gone).exists(), gone)
        # The probes' build, the prepared cache other workflows read and the kept triage logs stay.
        self.assertEqual((target / "probe").read_text(), "binary")
        self.assertTrue((build / "vanilla/prepared/key/sunrise-client/x").exists())
        self.assertTrue((self.slot / "loot_b.log").exists())

    def test_failed_datafix_seed_prepare_only_fails_datafix_and_runs_other_harnesses(self):
        self.initialise()
        keep = self.root / "keep"
        datafix_root = self.root / "parity-out/datafix"
        plan = self.root / "plan/bucket-0.json"
        plan.parent.mkdir(parents=True, exist_ok=True)
        plan.write_text('["loot_a", "datafix#1"]')
        self.env.update({
            "DYNA_CI_KEEP": str(keep),
            "SUNRISE_DATAFIX_ROOT": str(datafix_root),
            "BUCKET": "0",
        })
        (self.root / "tools/datafix_seeds.py").write_text(
            "import sys\nprint('private seed generation error', file=sys.stderr)\nsys.exit(1)\n")
        prepare_step = self.steps["Prepare datafix seed worlds from keep"]
        run_step = self.steps["Run every automatic harness"]
        self.assertEqual(prepare_step.get("id"), "datafix-seeds")
        self.assertEqual(prepare_step.get("continue-on-error"), "true")
        self.assertEqual(run_step["env"]["SUNRISE_DATAFIX_SEEDS_OUTCOME"], "${{ steps.datafix-seeds.outcome }}")
        prep = self.step("Prepare datafix seed worlds from keep")
        self.assertEqual(prep.returncode, 1)
        self.assertEqual(prep.stdout, "")
        self.assertEqual(prep.stderr, "")
        self.assertIn("private seed generation error", (self.slot / "datafix-seeds.log").read_text())
        # GitHub Actions records steps.datafix-seeds.outcome=failure and continues to the harness step.
        self.env["SUNRISE_DATAFIX_SEEDS_OUTCOME"] = "failure"
        (self.root / "tools/parity.py").write_text(
            "import os, sys\nfrom pathlib import Path\n"
            "assert os.environ['SUNRISE_DATAFIX_SEEDS_OUTCOME'] == 'failure'\n"
            "Path(sys.argv[-1]).write_text('[\"loot_a\", \"datafix#1\"]')\n"
            "print('PASS loot_a 3s')\n"
            "print('FAIL datafix#1 0s  (private path)')\n"
            "print('1 passed, 1 failed in 0.1 min.')\n"
            "sys.exit(1)\n")
        run = self.step("Run every automatic harness")
        self.assertEqual(run.returncode, 1)
        verdict = self.step("Write the harnesses' verdict")
        self.assertEqual(verdict.returncode, 0, verdict.stderr)
        self.assertEqual(verdict.stdout.splitlines(), ["PASS loot_a", "FAIL datafix"])


class ScheduledDynaJobTests(unittest.TestCase):
    def test_hot_paths_schedule_only_dispatches_and_its_dyna_job_runs_from_the_dispatch(self):
        # Dyna's runners refuse a scheduled event at "Set up runner" (hot-paths' scheduled runs, 2026-11-16 to 18):
        # the schedule starts only a GitHub-hosted job that dispatches the workflow on main, and the chain that
        # reaches the Dyna runner (route, measure, record) runs from that dispatch alone.
        from test_ci_concurrency import yaml
        workflow = yaml.load((ROOT / ".github/workflows/hot-paths.yml").read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(set(workflow["on"]), {"schedule", "workflow_dispatch"})
        jobs = workflow["jobs"]
        self.assertEqual(set(jobs), {"schedule-dispatch", "route", "measure", "record"})
        dispatcher = jobs["schedule-dispatch"]
        self.assertEqual(dispatcher["if"], "github.repository == 'sunrise-org/sunrise' && github.event_name == 'schedule'")
        self.assertEqual(dispatcher["runs-on"], "ubuntu-latest")
        self.assertEqual(dispatcher["permissions"], {"actions": "write"})
        self.assertEqual([step["run"] for step in dispatcher["steps"]],
                         ['gh workflow run hot-paths.yml --repo "$GITHUB_REPOSITORY" --ref main'])
        self.assertEqual(jobs["route"]["if"], "github.event_name == 'workflow_dispatch'")
        self.assertNotIn("needs", jobs["route"])
        self.assertEqual(jobs["measure"]["needs"], "route")
        self.assertEqual(jobs["measure"]["runs-on"], "dyna-medium")
        self.assertEqual(jobs["record"]["needs"], "measure")


if __name__ == "__main__":
    unittest.main()
