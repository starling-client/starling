"""The launcher mirror hash follows the public surface, and a change needs its announcement."""
import unittest

from tools.ci import check_launcher_mirrors as mirrors


MANIFEST = {
    "version": 1,
    "mirror": [
        {"launcher": "smod", "sources": ["crates/sunrise-mod-package/src"], "surface": "sha256:a"},
        {"launcher": "proxy", "sources": [], "note": "only in the launcher"},
    ],
}


class LauncherMirrorTests(unittest.TestCase):
    def test_a_private_change_keeps_the_surface(self):
        before = "\n".join(mirrors.public_surface("pub struct S;\npub fn f() -> u8 { 1 }\n"))
        after = "\n".join(mirrors.public_surface("pub struct S;\npub fn f() -> u8 { 2 }\nfn hidden() {}\n"))
        self.assertEqual(before, after)

    def test_a_public_change_moves_the_surface(self):
        before = "\n".join(mirrors.public_surface("pub fn f() -> u8 { 1 }\n"))
        after = "\n".join(mirrors.public_surface("pub fn f() -> u16 { 1 }\n"))
        self.assertNotEqual(before, after)

    def test_a_new_public_item_moves_the_surface(self):
        before = "\n".join(mirrors.public_surface("pub fn f() {}\n"))
        after = "\n".join(mirrors.public_surface("pub fn f() {}\npub fn g() {}\n"))
        self.assertNotEqual(before, after)

    def test_a_reexported_name_moves_the_surface(self):
        before = "\n".join(mirrors.public_surface("pub use crate::{a, b};\n"))
        after = "\n".join(mirrors.public_surface("pub use crate::{a, b, c};\n"))
        self.assertNotEqual(before, after)

    def test_a_mismatched_hash_fails(self):
        problems = mirrors.check(MANIFEST, {"smod": "sha256:b"})
        self.assertIn("Run --write", problems[0])

    def test_a_matching_hash_passes(self):
        self.assertEqual(mirrors.check(MANIFEST, {"smod": "sha256:a"}), [])

    def test_a_changed_surface_needs_the_marker_in_the_pull_request_body(self):
        base = dict(MANIFEST, mirror=[dict(MANIFEST["mirror"][0], surface="sha256:old")])
        config = dict(MANIFEST, mirror=[dict(MANIFEST["mirror"][0], surface="sha256:a")])
        self.assertTrue(any("launcher-mirror:" in problem
                            for problem in mirrors.check(config, {"smod": "sha256:a"}, base, "")))
        announced = "launcher-mirror: the launcher must take the new field\n"
        self.assertEqual(mirrors.check(config, {"smod": "sha256:a"}, base, announced), [])

    def test_every_shipped_manifest_source_exists(self):
        manifest = mirrors.load_manifest()
        for mirror in manifest["mirror"]:
            for source in mirror.get("sources", []):
                with self.subTest(launcher=mirror["launcher"], source=source):
                    self.assertTrue(
                        (mirrors.ROOT / source).exists(),
                        f"{mirror['launcher']}: {source} is not in the repository",
                    )

    def test_a_mirror_with_no_source_is_not_hashed(self):
        hashes, notes = mirrors.measured(MANIFEST)
        self.assertNotIn("proxy", hashes)
        self.assertIn("proxy: no source", notes[0])


if __name__ == "__main__":
    unittest.main()
