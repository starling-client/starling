"""tools/test_areas.toml stays whole: every test belongs to an area, and every pattern in it matches something."""

import contextlib
import io
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tools import test_areas as areas
from tools.ci import ignored_tests, test_scope


class MapTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.map = areas.load()
        runnable, by_hand = areas.harness_names()
        cls.harnesses = runnable + sorted(by_hand)
        cls.checks = areas.lan_checks()
        cls.tracked = areas.tracked()

    def placed(self):
        return {name for area in self.map.areas for name in areas.matching(area.harnesses, self.harnesses)}

    def test_area_names_are_unique(self):
        names = [area.name for area in self.map.areas]
        self.assertEqual(sorted(set(names)), sorted(names))

    def test_every_pattern_matches_something(self):
        dead = [f"escalate {pattern}" for pattern in self.map.escalate if not areas.matching([pattern], self.tracked)]
        for area in self.map.areas:
            for kind, patterns, names in (("paths", area.paths, self.tracked),
                                          ("harnesses", area.harnesses, self.harnesses), ("lan", area.lan, self.checks)):
                dead += [f"{area.name} {kind} {pattern}" for pattern in patterns if not areas.matching([pattern], names)]
        self.assertEqual([], dead, "patterns in tools/test_areas.toml that match nothing")

    def test_cargo_entries_name_a_target(self):
        bad = [f"{area.name}: {entry}" for area in self.map.areas for entry in area.cargo
               if not areas.TARGET.match(entry.target)]
        self.assertEqual([], bad, "a cargo entry's target is `lib`, `bin:NAME` or `test:NAME`")

    @unittest.skipUnless(shutil.which("cargo"), "needs cargo metadata")
    def test_cargo_entries_name_a_package_and_its_target(self):
        targets = {(package["name"], "lib" if kind.endswith("lib") else f"{kind}:{target['name']}")
                   for package in test_scope.cargo_metadata()["packages"]
                   for target in package["targets"] for kind in target["kind"]}
        missing = [f"{area.name}: {entry.package} {entry.target}" for area in self.map.areas for entry in area.cargo
                   if (entry.package, entry.target) not in targets]
        self.assertEqual([], missing, "cargo entries whose package has no such target")

    def test_a_library_file_selects_its_own_unit_tests(self):
        # A test of the library itself, not only of the gameplay calling it.
        for path, package, test in (
                ("crates/sunrise-world/src/nbt_path.rs", "sunrise-world", "nbt_path::tests::get_follows_the_path"),
                ("crates/sunrise-client-render/src/hud/gui_sprite.rs", "sunrise-client-render",
                 "hud::gui_sprite::tests::vanilla_ids_drop_their_namespace_and_a_mods_keep_theirs")):
            with self.subTest(path=path):
                entries = [entry for area in areas.select(self.map, [path]).areas for entry in area.cargo]
                self.assertTrue(any(areas.covers(entry, package, "lib", test) for entry in entries))

    def test_every_harness_is_in_an_area_or_unassigned(self):
        unplaced = sorted(set(self.harnesses) - self.placed() - set(self.map.unassigned))
        self.assertEqual([], unplaced, "put each new harness in an area of tools/test_areas.toml")

    def test_unassigned_lists_only_unplaced_harnesses(self):
        # The list only shrinks: a name leaves it when an area takes the harness, or when the harness goes.
        stale = sorted(set(self.map.unassigned) - (set(self.harnesses) - self.placed()))
        self.assertEqual([], stale, "[unassigned] names a harness an area holds, or one that no longer exists")

    def test_every_lan_check_is_in_an_area(self):
        loose = [check for check in self.checks if not any(areas.matching(area.lan, [check]) for area in self.map.areas)]
        self.assertEqual([], loose)

    def test_every_game_boot_test_is_in_an_area(self):
        rows = [row for row in ignored_tests.listed_entries() if row.mark == "daily" and row.reason == "game-boot"]
        self.assertTrue(rows)
        loose = [f"{row.package} {row.reference} {row.test}" for row in rows
                 if not any(areas.covers(entry, row.package, row.reference, row.test)
                            for area in self.map.areas for entry in area.cargo)]
        self.assertEqual([], loose, "give each game-boot test's module a cargo entry in tools/test_areas.toml")


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.map = areas.Map(
            areas=(
                areas.Area("boom", "", ("crates/game/src/explosion*",), (), ("tnt_*",), ()),
                areas.Area("ground", "", ("crates/world/src/worldgen/*",), (), ("biome_*",), ()),
            ),
            escalate=("Cargo.lock",),
            unassigned=(),
        )

    def test_a_path_selects_its_area(self):
        selection = areas.select(self.map, ["crates/game/src/explosion/rays.rs"])
        self.assertEqual(["boom"], [area.name for area in selection.areas])
        self.assertEqual((), selection.unmapped)

    def test_a_harness_script_selects_the_areas_listing_it(self):
        selection = areas.select(self.map, ["tools/test_biome_zoom_parity.py"])
        self.assertEqual(["ground"], [area.name for area in selection.areas])

    def test_areas_come_in_the_maps_order_once(self):
        selection = areas.select(self.map, ["crates/world/src/worldgen/a.rs", "crates/game/src/explosion.rs",
                                            "crates/world/src/worldgen/b.rs"])
        self.assertEqual(["boom", "ground"], [area.name for area in selection.areas])

    def test_documentation_selects_nothing_and_is_not_unmapped(self):
        selection = areas.select(self.map, ["docs/architecture.md", "crates/game/README.md"])
        self.assertEqual(((), (), ()), selection)

    def test_an_unclaimed_path_is_reported(self):
        selection = areas.select(self.map, ["crates/game/src/menu.rs"])
        self.assertEqual((), selection.areas)
        self.assertEqual(("crates/game/src/menu.rs",), selection.unmapped)

    def test_an_escalating_path_is_reported_apart(self):
        selection = areas.select(self.map, ["Cargo.lock"])
        self.assertEqual(("Cargo.lock",), selection.escalated)
        self.assertEqual((), selection.unmapped)

    def test_daily_rows_run_by_exact_name_and_only_with_daily(self):
        area = areas.Area("boom", "", (), (areas.Cargo("game", "lib", "game::tnt::"),), (), ())
        row = ignored_tests.Row("game", "game::tnt::tests::a_fuse_burns", "lib", "daily", "x.txt", "game-boot")
        other = ignored_tests.Row("game", "game::menu::tests::b", "lib", "daily", "x.txt", "game-boot")
        commands, tiers = areas.plan(area, [], [], [row, other])
        self.assertEqual([["cargo", "test", "-p", "game", "--lib", "--", "game::tnt::"]], commands)
        self.assertEqual([row], tiers["daily"])
        with mock.patch.object(ignored_tests, "shard_runs", return_value=True):
            commands, _ = areas.plan(area, [], [], [row, other], daily=True)
        self.assertEqual(["cargo", "test", "-p", "game", "--lib", "--", "--ignored", "--exact",
                          "game::tnt::tests::a_fuse_burns"], commands[-1])

    def test_a_proof_run_by_hand_is_named_not_passed_to_parity(self):
        area = areas.Area("boom", "", (), (), ("tnt_*",), ())
        commands, tiers = areas.plan(area, ["tnt_fuse"], [], [], by_hand=["tnt_proof"])
        self.assertEqual([[sys.executable, "tools/parity.py", "tnt_fuse"]], commands)
        self.assertEqual(["tnt_proof"], tiers["by_hand"])

    def test_an_opt_in_lan_check_runs_only_on_prebuilt_binaries(self):
        area = areas.Area("mods", "", (), (), (), ("recipes-smod",))
        commands, tiers = areas.plan(area, [], ["chest", "recipes-smod"], [], lan=True)
        self.assertEqual(([], ["recipes-smod"]), (commands, tiers["lan"]))
        commands, _ = areas.plan(area, [], ["chest", "recipes-smod"], [], lan=True, lan_bins="/bins")
        self.assertEqual([[sys.executable, "tools/lan_e2e.py", "--bin-dir", "/bins", "--check", "recipes-smod"]],
                         commands)


def package(name, folder, *dependencies):
    return {"name": name, "manifest_path": f"/repo/{folder}/Cargo.toml",
            "targets": [{"src_path": f"/repo/{folder}/src/lib.rs"}],
            "dependencies": [{"name": dependency} for dependency in dependencies]}


class RunFromDiffTests(unittest.TestCase):
    """`run --changed-files`: the paths no area claims still get their crates' tests."""

    def cargo_runs(self, paths):
        table = areas.Map((areas.Area("boom", "", ("crates/game/src/explosion*",),
                                      (areas.Cargo("game", "lib", "explosion::"),), (), ()),), ("Cargo.lock",), ())
        metadata = {"workspace_root": "/repo", "packages": [
            package("game", "crates/game"), package("menu", "crates/menu"), package("app", "crates/app", "menu")]}
        ran = []

        def run(command, cwd):
            ran.append(list(command))
            return subprocess.CompletedProcess(command, 0)

        with tempfile.TemporaryDirectory() as folder, contextlib.ExitStack() as stack:
            listed = Path(folder) / "paths.txt"
            listed.write_text("\n".join(paths), encoding="utf-8")
            for name, value in (("load", table), ("harness_names", ([], {})), ("lan_checks", []), ("daily_rows", [])):
                stack.enter_context(mock.patch.object(areas, name, return_value=value))
            stack.enter_context(mock.patch.object(test_scope, "cargo_metadata", return_value=metadata))
            stack.enter_context(mock.patch.object(areas.subprocess, "run", side_effect=run))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            self.assertEqual(0, areas.main(["run", "--changed-files", str(listed)]))
        return [command for command in ran if command[0] == "cargo"]

    def test_an_unmapped_only_diff_tests_its_crates_and_their_dependents(self):
        self.assertEqual([["cargo", "test", "-p", "app", "-p", "menu", "--lib", "--bins", "--tests"]],
                         self.cargo_runs(["crates/menu/src/slots.rs"]))

    def test_a_mixed_diff_runs_its_areas_and_the_rest_as_crates(self):
        area = ["cargo", "test", "-p", "game", "--lib", "--", "explosion::"]
        self.assertEqual([area, ["cargo", "test", "-p", "app", "-p", "menu", "--lib", "--bins", "--tests"]],
                         self.cargo_runs(["crates/game/src/explosion.rs", "crates/menu/src/slots.rs"]))
        # A path that reaches every area runs the whole workspace's tests.
        self.assertEqual([area, ["cargo", "test", "--workspace", "--lib", "--bins", "--tests"]],
                         self.cargo_runs(["crates/game/src/explosion.rs", "Cargo.lock"]))


if __name__ == "__main__":
    unittest.main()
