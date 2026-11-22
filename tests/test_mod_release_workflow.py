"""Offline asset collection and resume tests for the generated native workflow's shared steps."""

import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch


TEMPLATES = pathlib.Path(__file__).resolve().parents[1] / "crates/cargo-sunrise-mod/template/native"
sys.path.insert(0, str(TEMPLATES))
from sunrise_release import Batch, LINUX, REPO, confirmed_rebuild, json_read, json_write, sha, sign
from sunrise_release_github import missing_assets, submit, upload
sys.path.pop(0)


class Resume(unittest.TestCase):
    def test_repo_names_accept_registry_components_but_not_dot_paths(self):
        for repo in ("test/_example", "test/-example", "test/.example", f"{'a' * 39}/{'b' * 100}"):
            self.assertIsNotNone(REPO.fullmatch(repo), repo)
        for repo in ("./project", "../project", "test/.", "test/..", f"{'a' * 40}/b", f"a/{'b' * 101}"):
            self.assertIsNone(REPO.fullmatch(repo), repo)

    def test_only_missing_assets_are_uploaded_after_every_overlap_matches(self):
        rows = [
            {"name": "source.smod", "state": "uploaded", "digest": "exact source bytes"},
            {"name": "unrelated.md", "state": "uploaded", "digest": "unrelated bytes"},
        ]
        expected = {"source.smod": "exact source bytes", "source.smod.minisig": "signature bytes"}
        self.assertEqual(missing_assets(expected, rows, lambda row: row["digest"]), ["source.smod.minisig"])
        rows[0]["digest"] = "different source bytes"
        with self.assertRaises(SystemExit):
            missing_assets(expected, rows, lambda row: row["digest"])

    def test_duplicate_names_do_not_have_a_safe_resume(self):
        row = {"name": "source.smod", "state": "uploaded", "digest": "release bytes"}
        with self.assertRaises(SystemExit):
            missing_assets({"source.smod": "release bytes"}, [row, dict(row)], lambda asset: asset["digest"])


class Collection(unittest.TestCase):
    def setUp(self):
        self.work = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        plan = self.work / "plan"
        plan.mkdir()
        (plan / "releases.json").write_text("[]", encoding="utf-8")
        json_write(plan / "intent.json", {"commit": "b" * 40, "repo": "test/example"})
        json_write(plan / "plan.json", {"schema": 1, "tool_commit": "a" * 40, "plan": {
            "id": "example", "author_tag": "v1.0.0", "mode": "initial", "source_asset": "example-1.0.0.smod",
            "descriptor": "example-1.0.0+26.3-0.1.0.binaries.json", "hosts": [
                {"name": LINUX, "archive": "client.tar.xz", "source_producer": True},
                {"name": f"{LINUX}-server", "archive": "server.tar.xz", "source_producer": False},
            ],
        }})
        self.batch = Batch(self.work)

    def test_an_unknown_schema_is_refused_without_writing(self):
        json_write(self.batch.shared / "plan.json", {"schema": 7, "tool_commit": "f" * 40})
        before = {path: path.read_bytes() if path.is_file() else None for path in self.work.rglob("*")}
        with self.assertRaises(SystemExit):
            Batch(self.work)
        self.assertEqual({path: path.read_bytes() if path.is_file() else None for path in self.work.rglob("*")}, before)

    def test_missing_host_blocks_signing_before_writing_any_output(self):
        client = self.work / "host-results" / LINUX
        client.mkdir(parents=True)
        (client / "client.tar.xz").touch()
        (client / self.batch.source).touch()
        with patch.object(self.batch, "check_tool"), self.assertRaises(SystemExit):
            sign(self.batch, self.work)
        self.assertFalse(self.batch.signed.exists())

    def test_a_saved_batch_is_pinned_to_both_the_plan_and_each_file(self):
        batch = self.batch
        batch.assets.mkdir(parents=True)
        for name in batch.release_names():
            (batch.assets / name).write_bytes(name.encode())
        expected = {name: sha(batch.assets / name) for name in batch.release_names()}
        json_write(batch.signed / "batch.json", {"intent": batch.intent, "sha256": expected})
        self.assertEqual(batch.check_signed(), expected)
        (batch.assets / batch.source).write_bytes(b"changed bytes")
        with self.assertRaises(SystemExit):
            batch.check_signed()
        (batch.assets / batch.source).write_bytes(batch.source.encode())
        batch.intent["releases"] = "different release list"
        with self.assertRaises(SystemExit):
            batch.check_signed()


class Rebuild(unittest.TestCase):
    def setUp(self):
        self.work = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        plan = self.work / "plan"
        plan.mkdir()
        self.source = {"tag": "26.3-0.1.0", "commit": "a" * 40}
        self.target = {"tag": "26.3-0.1.2", "commit": "b" * 40}
        self.source_name = "example-1.0.0.smod"
        self.base = "https://github.com/test/example/releases/download/v1.0.0"
        self.record = {"key": "reviewed key", "owners": ["test"], "owner_ids": [42], "size": 100,
                       "sha256": "a" * 64, "signature": "original signature", "url": f"{self.base}/{self.source_name}"}
        self.previous = {"tag": "26.3-0.1.1", "commit": "c" * 40}
        json_write(plan / "releases.json", [self.source, self.previous, self.target])
        json_write(plan / "intent.json", {"commit": "f" * 40, "repo": "test/example"})
        json_write(plan / "plan.json", {"schema": 1, "tool_commit": "e" * 40, "plan": {
            "id": "example", "version": "1.0.0", "author_tag": "v1.0.0", "mode": "rebuild",
            "source_asset": self.source_name, "descriptor": "example-1.0.0+26.3-0.1.2.binaries.json",
            "source": self.source, "target": self.target, "reviewed_source": self.record,
            "hosts": [{"name": LINUX, "archive": "target.tar.xz", "source_producer": False}],
        }})
        self.batch = Batch(self.work)
        self.batch.assets.mkdir(parents=True)
        for name in self.batch.release_names():
            (self.batch.assets / name).write_bytes(name.encode())
        self.hashes = {name: sha(self.batch.assets / name) for name in self.batch.release_names()}
        json_write(self.batch.signed / "batch.json", {"intent": self.batch.intent, "sha256": self.hashes})
        self.registry = self.work / "registry"
        (self.registry / "mods").mkdir(parents=True)
        self.entry = self.registry / "mods" / "example.toml"
        self.seed = f'''id = "example"
kind = "native-mod"
key = "reviewed key"
owners = ["test"]
owner_ids = [42]
[[versions]]
version = "1.0.0"
game = "26.3"
sunrise = "{self.source['tag']}"
file = "{self.record['url']}"
sha256 = "{self.record['sha256']}"
signature = "{self.record['signature']}"
[[versions.builds]]
sunrise = "{self.previous['tag']}"
'''
        self.entry.write_text(self.seed, encoding="utf-8")
        self.published = self.work / "published.json"
        json_write(self.published, {"schema": 1, "releases": [self.source, self.previous], "revoked": [],
            "projects": [{"id": "example", "kind": "native-mod", "key": self.record["key"], "versions": [{
                "version": "1.0.0", "game": "26.3", "sunrise": self.source["tag"],
                "file": {"name": self.source_name, "size": 100, "sha256": self.record["sha256"],
                         "signature": self.record["signature"]},
                "native_binaries": [{"build": self.previous["commit"]}],
            }]}],
        })
        self.enterContext(patch.dict("os.environ", SUNRISE_MOD_PUBLIC_KEY=self.record["key"]))

    def confirm(self):
        return confirmed_rebuild(self.batch, self.registry, self.published, self.hashes)

    def test_only_the_identical_reviewed_append_is_a_noop(self):
        self.assertFalse(self.confirm())
        descriptor = self.batch.descriptor
        row = self.seed + f'''[[versions.builds]]
sunrise = "{self.target['tag']}"
sha256 = "{self.hashes[descriptor]}"
signature = "{descriptor}.minisig"
binaries = "{self.base}/{descriptor}"
'''
        self.entry.write_text(row, encoding="utf-8")
        self.assertTrue(self.confirm())
        self.entry.write_text(row.replace(f"{descriptor}.minisig", "different signature"), encoding="utf-8")
        with self.assertRaisesRegex(SystemExit, "different build"):
            self.confirm()

    def test_a_published_body_change_blocks_the_append(self):
        index = json_read(self.published)
        index["releases"].append(self.target)
        index["projects"][0]["versions"][0]["verdicts"] = {
            self.target["tag"]: {"verdict": "rebuild", "review": ["game::tick"], "targets": []},
        }
        json_write(self.published, index)
        with self.assertRaisesRegex(SystemExit, "published fit break"):
            self.confirm()

    def test_a_new_owner_key_file_or_takedown_blocks_the_append(self):
        for old, new in [("[42]", "[99]"), ("reviewed key", "other key"), ("original signature", "changed signature"),
                         (self.record["url"], f"{self.base}/different.smod")]:
            with self.subTest(old=old):
                self.entry.write_text(self.seed.replace(old, new), encoding="utf-8")
                with self.assertRaisesRegex(SystemExit, "changed"):
                    self.confirm()
        self.entry.write_text(self.seed, encoding="utf-8")
        index = json_read(self.published)
        index["revoked"] = [{"id": "example", "sha256": self.record["sha256"]}]
        json_write(self.published, index)
        with self.assertRaisesRegex(SystemExit, "taken down"):
            self.confirm()

    def test_live_stable_pins_stop_publication_even_if_registry_pages_is_stale(self):
        stage = self.work / "submission"
        (stage / "mods").mkdir(parents=True)
        (stage / "PULL_REQUEST.md").write_text("Append build\n", encoding="utf-8")
        (stage / "mods" / "example.toml").write_bytes(self.entry.read_bytes())
        stale_releases = self.batch.releases.read_bytes()
        stale_index = self.published.read_bytes()
        for role in ("source", "target"):
            for withdrawn in (False, True):
                with self.subTest(role=role, withdrawn=withdrawn):
                    tag = self.batch.plan[role]["tag"]
                    live = [row for row in json_read(self.batch.releases) if row["tag"] != tag]
                    if not withdrawn:
                        live.append({"tag": tag, "commit": "f" * 40})

                    def fresh(*args):
                        self.assertEqual(args[:3], ("registry", "releases", "--out"))
                        json_write(args[3], live)

                    with (patch.object(self.batch, "check_tool"),
                          patch("sunrise_release_github.registry"),
                          patch("sunrise_release_github.stage"),
                          patch("sunrise_release_github.tag_commit", return_value="9" * 40),
                          patch("sunrise_release_github.release", return_value={"draft": False}),
                          patch("sunrise_release_github.remote_files", return_value=[self.batch.descriptor]),
                          patch("sunrise_release_github.tool", side_effect=fresh) as releases,
                          patch("sunrise_release_github.call") as call,
                          patch("sunrise_release_github.api") as api,
                          patch.dict("os.environ", GH_TOKEN="unused-fixture-token")):
                        with self.assertRaisesRegex(SystemExit, f"Sunrise {role} tag {tag}"):
                            upload(self.batch)
                        with self.assertRaisesRegex(SystemExit, f"Sunrise {role} tag {tag}"):
                            submit(self.batch, self.registry, stage, "test/registry")
                        self.assertEqual(releases.call_count, 2)
                        call.assert_not_called()
                        api.assert_not_called()
                    self.assertEqual(self.batch.releases.read_bytes(), stale_releases)
                    self.assertEqual(self.published.read_bytes(), stale_index)

    def test_upload_never_uploads_or_recreates_a_missing_original_source(self):
        with (patch.object(self.batch, "check_tool"), patch("sunrise_release_github.registry"),
              patch("sunrise_release_github.stage"), patch("sunrise_release_github.tag_commit", return_value="9" * 40),
              patch("sunrise_release_github.release", return_value={"draft": False}),
              patch("sunrise_release_github.remote_files", return_value=[self.source_name]),
              patch("sunrise_release_github.call") as call):
            with self.assertRaisesRegex(SystemExit, "original reviewed source is missing"):
                upload(self.batch)
            call.assert_not_called()


if __name__ == "__main__":
    unittest.main()
