"""The rolling mod tool: the folders that rebuild it come from cargo metadata, and its workflow mirrors the SDK's signing."""
import json
import re
import shutil
import unittest
from pathlib import Path

from tests.test_ci_concurrency import ROOT
from tools.ci import mod_tool
from tools.ci import vendored_yaml

yaml = vendored_yaml.load()


def package(name, root="/w"):
    return {"id": name, "name": name, "manifest_path": f"{root}/crates/{name}/Cargo.toml"}


def node(name, deps):
    return {"id": name, "deps": [{"pkg": dep, "dep_kinds": [{"kind": kind}]} for dep, kind in deps]}


class ToolFoldersTests(unittest.TestCase):
    def test_the_folders_are_the_workspace_crates_the_tool_builds_with_and_no_others(self):
        metadata = {
            "workspace_root": "/w",
            "workspace_members": ["cargo-sunrise-mod", "lib", "build-helper", "only-tests", "unrelated"],
            "packages": [package(n) for n in ("cargo-sunrise-mod", "lib", "build-helper", "only-tests", "unrelated",
                                              "registry-crate")],
            "resolve": {"nodes": [
                node("cargo-sunrise-mod", [("lib", None), ("build-helper", "build"), ("only-tests", "dev"),
                                           ("registry-crate", None)]),
                node("lib", [("registry-crate", None)]),
                node("build-helper", []), node("only-tests", []), node("unrelated", []), node("registry-crate", []),
            ]},
        }
        self.assertEqual(mod_tool.crates(metadata),
                         ["crates/build-helper", "crates/cargo-sunrise-mod", "crates/lib"])

    @unittest.skipUnless(shutil.which("cargo"), "needs cargo")
    def test_the_real_workspace_names_the_tool_and_its_package_crates(self):
        folders = mod_tool.crates()
        self.assertIn("crates/cargo-sunrise-mod", folders)
        self.assertIn("crates/sunrise-native-build", folders)
        self.assertIn("crates/sunrise-mod-package", folders)
        self.assertNotIn("crates/sunrise-client", folders)

    def test_the_build_inputs_follow_the_imports_of_the_program_that_builds_the_tool(self):
        import tempfile
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as folder:
            tools = Path(folder) / "tools"
            tools.mkdir()
            (tools / "prebuilt_game.py").write_text("import os\nfrom tools import lab, unused_name\nfrom tools.release_file import LAUNCHER_MIN\n")
            (tools / "lab.py").write_text("from tools import file_lock\nfrom tools.ci import ids\n")
            (tools / "ci").mkdir()
            (tools / "ci/ids.py").write_text("")
            (tools / "file_lock.py").write_text("import time\n")
            (tools / "release_file.py").write_text("")
            (tools / "other.py").write_text("")
            with patch.object(mod_tool, "ROOT", Path(folder)):
                inputs = mod_tool.build_inputs()
        self.assertEqual([path for path in inputs if path.startswith("tools/") and path not in mod_tool.BUILD_FILES],
                         ["tools/ci/ids.py", "tools/file_lock.py", "tools/lab.py", "tools/prebuilt_game.py",
                          "tools/release_file.py"])
        self.assertIn("Cargo.lock", inputs)

    def test_the_real_build_inputs_hold_the_modules_prebuilt_game_imports(self):
        self.assertTrue({"tools/prebuilt_game.py", "tools/lab.py", "tools/file_lock.py"} <= set(mod_tool.build_inputs()))

    def test_the_contract_number_in_tool_json_is_the_sources(self):
        import tempfile
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "tool.json"
            mod_tool.tool_json("HEAD", out)
            document = json.loads(out.read_text())
        self.assertEqual(document["tag"], "mod-tool")
        self.assertIsInstance(document["native_contract"], int)
        self.assertIsInstance(document["release_contract"], int)
        number = (ROOT / "crates/sunrise-mod-package/src/sdk/package-format").read_text().split()[0]
        self.assertEqual(document["package_format"], int(number))
        self.assertEqual(document["schema"], 1)


class WorkflowTests(unittest.TestCase):
    def load(self, name):
        return yaml.load((ROOT / ".github/workflows" / name).read_text(), Loader=yaml.BaseLoader)

    def test_only_the_sign_job_holds_the_key_and_only_publish_writes(self):
        workflow = self.load("mod-tool.yml")
        jobs = workflow["jobs"]
        text = (ROOT / ".github/workflows/mod-tool.yml").read_text()
        self.assertEqual(sorted(re.findall(r"secrets\.\w+", text)), ["secrets.RELEASE_SIGNING_KEY",
                                                                    "secrets.RELEASE_SIGNING_KEY_NEXT"])
        self.assertIn("RELEASE_SIGNING_KEY", json.dumps(jobs["sign"]))
        self.assertIn("'release'", jobs["sign"]["environment"])
        for name, job in jobs.items():
            self.assertEqual(job.get("permissions", {}).get("contents") == "write", name == "publish", name)
            self.assertEqual("environment" in job, name == "sign", name)
        self.assertNotIn("pull_request", workflow["on"])
        # It follows the gate it reads: a run started by the push itself would look before the gate had finished.
        self.assertNotIn("push", workflow["on"])
        self.assertEqual(workflow["on"]["workflow_run"]["workflows"], ["dyna-checks"])

    def test_signing_mirrors_the_sdks(self):
        mine = self.load("mod-tool.yml")["jobs"]["sign"]
        sdk = self.load("sdk-publish.yml")["jobs"]["sign"]
        self.assertEqual(mine["runs-on"], sdk["runs-on"])
        checkout = lambda job: next(step for step in job["steps"] if step.get("uses", "").startswith("actions/checkout"))
        self.assertEqual(checkout(mine)["with"], checkout(sdk)["with"])
        signing = lambda job: next(step for step in job["steps"] if step.get("name", "").startswith("Sign SHA256SUMS"))
        self.assertEqual(signing(mine)["run"].replace('"$TAG"', "TAG"), signing(sdk)["run"].replace('"$TAG"', "TAG"))


if __name__ == "__main__":
    unittest.main()
