"""Dev builds: release.yml kind dev builds a commit of main with the gate green and publishes it under its
own tag, never as the Nightly and never moving `daily-green`."""
import datetime as dt
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests.test_ci_concurrency import ROOT, yaml
from tools.ci import nightly

WORKFLOWS = ROOT / ".github/workflows"


def load(name):
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


class ReleaseDevTests(unittest.TestCase):
    def setUp(self):
        self.workflow = load("release.yml")
        self.jobs = self.workflow["jobs"]
        self.dev = {name: job for name, job in self.jobs.items() if name.startswith("dev")}

    def test_only_main_without_dry_run_publishes(self):
        mode = self.jobs["decide"]["steps"][1]
        self.assertEqual(mode["run"], 'python3 tools/release_train.py authorize --publish-kind "$KIND" --dry-run "$DRY_INPUT"')
        self.assertEqual(mode["env"], {"KIND": "${{ inputs.kind }}", "DRY_INPUT": "${{ inputs.dry_run }}"})
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["dry_run"]["default"], "true")
        publish = self.dev["dev-sdk-publish"]
        self.assertEqual(publish["with"]["publish"], "${{ needs.decide.outputs.publish == 'true' }}")
        self.assertEqual(publish["uses"], "./.github/workflows/sdk-publish.yml")
        # The game is built by `nightly build`, which publishes only when its own dry_run is false: it gets ours.
        run = self.dev["dev"]["steps"][0]["run"]
        self.assertIn('-f dry_run="$DRY_RUN"', run)
        self.assertEqual(self.dev["dev"]["steps"][0]["env"]["DRY_RUN"], "${{ needs.decide.outputs.dry_run }}")

    def test_a_dev_build_never_moves_daily_green_or_writes_the_nightly(self):
        text = yaml.safe_dump(self.dev)
        self.assertNotIn("daily-green", text)
        self.assertNotIn("git tag", text)
        self.assertNotIn("gh release", text)
        run = self.dev["dev"]["steps"][0]["run"]
        self.assertIn("gh workflow run nightly-build.yml --repo \"$GITHUB_REPOSITORY\" --ref main -f kind=dev", run)
        for name, job in self.dev.items():
            self.assertIn("inputs.kind == 'dev'", job["if"], name)
        self.assertIn("inputs.kind == 'nightly'", self.jobs["nightly"]["if"])

    def test_the_sdk_is_built_and_published_in_this_run_before_the_game(self):
        self.assertEqual(self.dev["dev-sdk"]["uses"], "./.github/workflows/prebuilt-game-build.yml")
        self.assertEqual(self.dev["dev-sdk"]["with"], {"commit": "${{ needs.decide.outputs.head }}"})
        self.assertEqual(self.dev["dev-sdk-publish"]["with"],
                         {"commit": "${{ needs.decide.outputs.head }}",
                          "publish": "${{ needs.decide.outputs.publish == 'true' }}"})
        game = self.dev["dev"]
        self.assertEqual(game["needs"], ["decide", "dev-sdk", "dev-sdk-publish"])
        self.assertIn("needs.dev-sdk.result == 'success'", game["if"])
        self.assertIn("(needs.dev-sdk-publish.result == 'success' || needs.dev-sdk-publish.result == 'skipped')",
                      game["if"].replace("\n", " "))

    def test_dev_builds_never_queue_with_nightlies(self):
        self.assertIn("inputs.kind == 'dev' && '-dev'", self.workflow["concurrency"]["group"])


class NightlyBuildDevTests(unittest.TestCase):
    def setUp(self):
        self.workflow = load("nightly-build.yml")
        self.steps = {step.get("name"): step for step in self.workflow["jobs"]["publish"]["steps"]}

    def test_a_dev_build_publishes_only_its_own_tag(self):
        nightly_step = self.steps["Publish as the `nightly` pre-release"]
        dev_step = self.steps["Publish as the dev-<date>-<short> pre-release"]
        self.assertIn("inputs.kind == 'nightly'", nightly_step["if"])
        self.assertIn("inputs.kind == 'dev'", dev_step["if"])
        self.assertIn("needs.pick.outputs.publish == 'true'", dev_step["if"])
        run = dev_step["run"]
        self.assertIn('gh release create "$TAG"', run)
        for never in ("rotate", "keep-source", "daily-green", "release create nightly", "prune --days", "release edit"):
            self.assertNotIn(never, run)
        self.assertIn('--title "Dev $DATE · $SHORT"', run)
        self.assertLess(run.index("require-sdk"), run.index("gh release create"))

    def test_the_channel_signed_and_checked_is_the_kind(self):
        sign = self.workflow["jobs"]["sign"]["steps"][2]
        self.assertEqual(sign["env"]["KIND"], "${{ inputs.kind }}")
        self.assertEqual(sign["env"]["ALSO_FOR"], "${{ inputs.kind == 'stable' && needs.pick.outputs.tag || "
                         "format('{0}-{1}-{2}', inputs.kind, needs.pick.outputs.date, needs.pick.outputs.short) }}")
        verify = self.steps["The signatures, checked as launchers check them"]["run"]
        self.assertIn('--tag "$SIGN_FOR" --publishing', verify)
        self.assertIn('version="${KIND}-${DATE}-${SHORT}"', verify)
        self.assertIn('--tag "$version"', verify)
        self.assertEqual(self.workflow["on"]["workflow_dispatch"]["inputs"]["kind"]["default"], "nightly")

    def test_optional_platforms_follow_repository_and_channel(self):
        game = load("game-build.yml")
        build = game["jobs"]["build"]
        expression = build["continue-on-error"].removeprefix("${{").removesuffix("}}").strip()
        self.assertEqual(game["on"]["workflow_call"]["inputs"]["kind"]["default"], "release")
        self.assertEqual(self.workflow["jobs"]["build"]["with"]["kind"], "${{ inputs.kind }}")
        for repo in ("sunrise-org/sunrise", "example/test"):
            for kind in ("dev", "nightly", "release"):
                optional = set()
                for row in build["strategy"]["matrix"]["include"]:
                    condition = (expression.replace("matrix.name", repr(row["name"]))
                                 .replace("inputs.kind", repr(kind)).replace("github.repository", repr(repo))
                                 .replace("matrix.mac", str(row.get("mac") == "true")).replace("true", "True"))
                    if eval(condition.replace("||", "or").replace("&&", "and"), {"__builtins__": {}}):
                        optional.add(row["name"])
                macs = {"macos-x64"} if kind == "dev" else set()
                if repo == "sunrise-org/sunrise":
                    macs = {"macos", "macos-x64"}
                self.assertEqual(optional, {"windows-arm64"} | macs)
        self.assertEqual(game["jobs"]["linux-smoke"]["needs"], "legs")
        # A reusable workflow that ends failed or cancelled (an optional leg) passes back no outputs: signing and
        # publishing follow the legs job's build-complete marker, read by an ordinary job.
        self.assertNotIn("outputs", game["on"]["workflow_call"])
        marker = [step for step in game["jobs"]["legs"]["steps"] if step.get("with", {}).get("name") == "build-complete"]
        self.assertEqual([step["if"] for step in marker], ["steps.decide.outputs.complete == 'true'"])
        complete = self.workflow["jobs"]["complete"]
        self.assertEqual(complete["steps"][0]["with"]["name"], "build-complete")
        self.assertEqual(complete["outputs"]["complete"], "${{ steps.marker.outcome == 'success' }}")
        self.assertEqual(self.workflow["jobs"]["sign"]["needs"], ["pick", "build", "complete", "break-glass"])
        self.assertEqual(self.workflow["jobs"]["publish"]["needs"], ["pick", "build", "complete", "sign", "break-glass"])
        for job in ("sign", "publish"):
            self.assertIn("needs.complete.outputs.complete == 'true'", self.workflow["jobs"][job]["if"])

    @unittest.skipIf(os.name == "nt", "executes the POSIX signing step")
    def test_missing_intel_mac_is_signed_only_for_dev_and_notes_name_missing_assets(self):
        sign = self.workflow["jobs"]["sign"]["steps"][2]["run"]
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "dist").mkdir()
            (root / "tools/ci").mkdir(parents=True)
            (root / "tools/ci/sign_release.sh").write_text(
                'for file in "${@:2}"; do test -f "$file" || exit 9; done\nprintf "%s\\n" "$@" > signed\n')
            for name in ("sunrise-linux-x86_64.tar.gz", "sunrise-windows-x64.zip", "sunrise-macos-arm64.zip",
                         "sunrise-server-linux-x86_64.tar.gz", "sunrise-source.tar.gz", "sunrise-release.json"):
                (root / "dist" / name).touch()
            for kind, code in (("dev", 0), ("nightly", 1)):
                result = subprocess.run(["bash", "-e", "-o", "pipefail", "-c", sign], cwd=root,
                                        env=dict(os.environ, KIND=kind, GITHUB_REPOSITORY="example/test"),
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, code, result.stdout + result.stderr)
            published = self.steps["Publish as the dev-<date>-<short> pre-release"]["run"]
            notes = published[published.index("# Archive presence"):].split("# Exactly the files", 1)[0]
            subprocess.run(["bash", "-e", "-o", "pipefail", "-c", notes], cwd=root, check=True)
            body = (root / "dist/nightly-notes.md").read_text()
            self.assertIn("Built: `sunrise-macos-arm64.zip`", body)
            for missing in ("sunrise-macos-x64.zip", "sunrise-windows-arm64.zip"):
                self.assertIn(f"Failed or unavailable; not published: `{missing}`", body)
            self.assertNotIn("musl", body)  # The platform table is for archives published before attachment.

    def test_dev_builds_never_replace_a_waiting_nightly(self):
        group = self.workflow["concurrency"]["group"]
        self.assertIn("'nightly-build'", group)
        self.assertIn("format('dev-build{0}'", group)


class DevToolsTests(unittest.TestCase):
    def test_old_dev_builds_go_but_the_newest_and_everything_else_stay(self):
        tags = [{"tagName": "dev-2026-11-01-aaaaaaa", "isPrerelease": True},
                {"tagName": "dev-2026-11-02-bbbbbbb", "isPrerelease": True},
                {"tagName": "dev-2026-11-12-ccccccc", "isPrerelease": True},
                {"tagName": "prebuilt-" + "d" * 40, "isPrerelease": True},
                {"tagName": "sdk-" + "d" * 40, "isPrerelease": True},
                {"tagName": "nightly-2026-11-01", "isPrerelease": True},
                {"tagName": "dev-2026-11-01", "isPrerelease": True},
                {"tagName": "dev-2026-11-01-eeeeeee", "isPrerelease": False}]
        today = dt.date(2026, 11, 13)
        self.assertEqual(nightly.stale_dev_builds(tags, today, 7),
                         ["dev-2026-11-01-aaaaaaa", "dev-2026-11-02-bbbbbbb"])
        # A long quiet spell: the newest stays however old.
        self.assertEqual(nightly.stale_dev_builds(tags[:2], dt.date(2027, 1, 1), 7), ["dev-2026-11-01-aaaaaaa"])

    def test_pick_checks_the_gate_and_names_the_dev_build(self):
        sha = "0c02067" + "a" * 33
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "out"
            back = os.getcwd()
            os.chdir(folder)
            try:
                env = {"GITHUB_REPOSITORY": "sunrise-org/sunrise", "GITHUB_OUTPUT": str(out), "NIGHTLY_BASE": sha,
                       "GITHUB_REF": "refs/heads/main"}
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha) as gate, \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "gh", return_value=""):
                    nightly.pick_dev()
                gate.assert_called_once_with("dev", sha)
                values = dict(line.split("=", 1) for line in out.read_text().splitlines())
                today = dt.datetime.now(dt.timezone.utc).date().isoformat()
                self.assertEqual(values["tag"], f"dev-{today}-0c02067")
                self.assertEqual(values["version"], f"dev-{today}")
                self.assertEqual(values["build"], "true")
                notes = Path("nightly-notes.md").read_text()
                self.assertIn(f"sunrise-launcher install {values['tag']}", notes)
                self.assertIn(f"sunrise-release-sign verify --tag {values['tag']} --signature SHA256SUMS.v2.sig", notes)
                # Published already (the same commit the same day): skipped, never replaced.
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=True), \
                        mock.patch.object(nightly, "gh", return_value='{"tagName": "x"}'):
                    nightly.pick_dev()
                self.assertIn("build=false", out.read_text().splitlines()[-10:])
                self.assertIn("sdk=true", out.read_text().splitlines()[-10:])
                # No published SDK: a dry run builds without it (sdk=false: no sdk.json), a real Dev build stops.
                with mock.patch.dict(os.environ, {**env, "DRY_RUN": "true"}), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=False):
                    nightly.pick_dev()
                self.assertIn("sdk=false", out.read_text().splitlines()[-10:])
                # No published SDK: a real Dev build stops.
                with mock.patch.dict(os.environ, env), mock.patch.object(nightly, "git"), \
                        mock.patch.object(nightly.train, "channel_head", return_value=sha), \
                        mock.patch.object(nightly, "sdk_ready", return_value=False):
                    with self.assertRaises(SystemExit):
                        nightly.pick_dev()
            finally:
                os.chdir(back)


if __name__ == "__main__":
    unittest.main()
