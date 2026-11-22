import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "ci"))
import mojang_republish as mr  # noqa: E402


def detail(client="c1", server="s1", java=25, library="1" * 40, version="9.9"):
    return {"id": version, "downloads": {"client": {"sha1": client}, "server": {"sha1": server}},
            "javaVersion": {"majorVersion": java}, "assetIndex": {"sha1": "new11111"},
            "libraries": [{"downloads": {"artifact": {"path": "a.jar", "sha1": library}}}]}


# A version no code names: the lock entry alone says what is pinned.
GAME = {"version": "9.9", "manifest_sha1": "aaaa1111", "client_sha1": "c1", "server_sha1": "s1", "java_major": 25,
        "libraries_sha256": mr.lab.libraries_digest(detail())}


def manifest(sha1):
    return {"versions": [{"id": "9.8", "sha1": "zzzz", "url": "u0"},
                         {"id": "9.9", "sha1": sha1, "url": f"https://x/{sha1}.json"}]}


class Plan(unittest.TestCase):
    def run_plan(self, sha1, info=None, issues=None):
        def fetch(url):
            self.assertIsNotNone(info, "the pinned manifest must not be downloaded")
            return info
        return mr.plan(copy.deepcopy(GAME), manifest(sha1), fetch, issues or {})

    def test_the_pinned_file_needs_nothing(self):
        self.assertIsNone(self.run_plan("aaaa1111"))

    def test_a_reissue_the_launcher_accepts_needs_nothing(self):
        game = {**copy.deepcopy(GAME), "accepted_manifest_sha1": ["bbbb2222"]}
        self.assertIsNone(mr.plan(game, manifest("bbbb2222"), lambda url: self.fail("downloaded"), {}))

    def test_a_reissue_with_the_pinned_content_asks_for_the_launchers_list(self):
        action = self.run_plan("cccc3333", detail())
        self.assertEqual((action["kind"], action["title"], action["changed"]),
                         ("open", "Mojang re-issued 9.9: cccc3333", []))
        self.assertIn('"accepted_manifest_sha1": [\n    "cccc3333"\n  ]', action["body"])
        issues = {"Mojang re-issued 9.9: cccc3333": 9}
        self.assertEqual(self.run_plan("cccc3333", detail(), issues)["kind"], "skip")

    def test_changed_content_opens_an_issue_naming_what_differs(self):
        for info, key in ((detail(client="c2"), "client_sha1"), (detail(server="s2"), "server_sha1"),
                          (detail(java=26), "java_major"), (detail(library="2" * 40), "libraries_sha256")):
            action = self.run_plan("cccc3333", info)
            self.assertEqual((action["kind"], action["title"], action["changed"]),
                             ("open", "Mojang changed 9.9: cccc3333", [key]))
            self.assertIn(f"`{key}`", action["body"])
            self.assertIn("`cccc3333`", action["body"])

    def test_existing_issue_is_skipped(self):
        issues = {"Mojang changed 9.9: cccc3333": 7, "Mojang changed 9.9: dddd4444": 8}
        action = self.run_plan("cccc3333", detail(client="c2"), issues)
        self.assertEqual((action["kind"], action["number"]), ("skip", 7))

    def test_missing_version_is_an_error(self):
        with self.assertRaises(ValueError):
            mr.plan(GAME, {"versions": []}, None, {})

    def test_open_issues_filters_by_title_prefix(self):
        listed = '[{"number":1,"title":"Mojang changed 9.9: cccc3333"},{"number":2,"title":"Other"}]'
        self.assertEqual(mr.open_issues_titled("9.9", runner=lambda *a, **k: listed),
                         {"Mojang changed 9.9: cccc3333": 1})
        listed = '[{"number":3,"title":"Mojang re-issued 9.9: cccc3333"},{"number":1,"title":"Mojang changed 9.9: x"}]'
        self.assertEqual(mr.open_issues_titled("9.9", runner=lambda *a, **k: listed, title=mr.REISSUE),
                         {"Mojang re-issued 9.9: cccc3333": 3})


if __name__ == "__main__":
    unittest.main()
