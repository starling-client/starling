"""Machine-consumed contracts of the guarded API-only state symbol refactor."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools/refactors/api-pr3b/swap_state_symbols.py"
# The scratch directories live under the checkout's `target/`, which a cold CI cache does not have yet.
(ROOT / "target").mkdir(exist_ok=True)


class BlockStateSwapTest(unittest.TestCase):
    def run_swap(self, source, apply, other="pub type Read = BlockStateId;\n"):
        scratch = tempfile.TemporaryDirectory(dir=ROOT / "target", prefix="swap-proof-")
        self.addCleanup(scratch.cleanup)
        root = Path(scratch.name)
        (root / "Cargo.toml").write_text("[workspace]\n")
        api = root / "crates/horizon-mod-api/src"
        api.mkdir(parents=True)
        (api / "values.rs").write_text(source)
        (api / "also.rs").write_text("pub type Read = BlockStateId;\n")
        engine = root / "crates/horizon-world/src/registry.rs"
        engine.parent.mkdir(parents=True)
        engine.write_text("pub struct BlockStateId(pub u32);\n")
        report = root / "report.json"
        command = ["python3", str(SCRIPT), "--root", str(root), "--report", str(report)]
        run = subprocess.run(command + (["--apply"] if apply else []), capture_output=True, text=True)
        self.assertEqual(engine.read_text(), "pub struct BlockStateId(pub u32);\n")
        self.assertEqual((api / "also.rs").read_text(), other)
        return run, (api / "values.rs").read_text(), json.loads(report.read_text())

    def test_api_types_change_but_engine_storage_does_not(self):
        run, result, report = self.run_swap("pub struct Read { pub state: BlockStateId, pub block: BlockId }\n", True, "pub type Read = BlockState;\n")
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(result, "pub struct Read { pub state: BlockState, pub block: BlockKey }\n")
        self.assertTrue(report["applied"])
        values = next(row for row in report["files"] if row["file"].endswith("values.rs"))
        self.assertEqual((values["states"], values["keys"]), (1, 1))
        self.assertNotEqual(report["files"][0]["before_sha256"], report["files"][0]["after_sha256"])

    def test_dry_run_never_writes_and_constructor_refusal_is_atomic(self):
        source = "fn fixture() { let state = BlockStateId(19); }\n"
        for apply in [False, True]:
            run, result, report = self.run_swap(source, apply)
            self.assertEqual(result, source)
            self.assertFalse(report["applied"])
            self.assertEqual(len(report["refused"]), 1)
            self.assertEqual(run.returncode == 0, not apply)

    def test_known_constructor_is_deferred_without_guessing_its_semantics(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target", prefix="swap-deferred-") as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text("[workspace]\n")
            api = root / "crates/horizon-mod-api/src"
            api.mkdir(parents=True)
            fixture = api / "wasm_key_tests.rs"
            fixture.write_text("fn fixture() { let state = BlockStateId(19); }\n")
            report = root / "report.json"
            subprocess.run(["python3", str(SCRIPT), "--root", str(root), "--report", str(report), "--apply", "--defer-constructor-fixtures"], check=True, capture_output=True)
            self.assertIn("BlockState(19)", fixture.read_text())
            result = json.loads(report.read_text())
            self.assertTrue(result["applied"])
            self.assertEqual(len(result["hand_fixes"]), 1)

    def test_only_transitional_arms_are_removed(self):
        with tempfile.TemporaryDirectory(dir=ROOT / "target", prefix="swap-arms-") as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text("[workspace]\n")
            (root / "crates/horizon-mod-api/src").mkdir(parents=True)
            dispatch = root / "crates/horizon-mod-api-gen/src/dispatch.rs"
            dispatch.parent.mkdir(parents=True)
            preserved = 'normalize("BlockStateId"); engine::<horizon_world::registry::BlockStateId>();\n'
            dispatch.write_text(preserved + '                    "BlockStateId" if table == "block" => { first() }\n                    "BlockStateId" => { second() }\n                    "& ItemStack" => { third() }\n')
            report = root / "report.json"
            subprocess.run(["python3", str(SCRIPT), "--root", str(root), "--report", str(report), "--apply"], check=True, capture_output=True)
            self.assertEqual(dispatch.read_text(), preserved + '                    "& ItemStack" => { third() }\n')
            self.assertEqual(json.loads(report.read_text())["files"][0]["states_after"], 2)

    def test_qualified_engine_import_requires_ownership_review(self):
        source = "use horizon_world::registry::BlockStateId;\n"
        run, result, report = self.run_swap(source, True)
        self.assertNotEqual(run.returncode, 0)
        self.assertEqual(result, source)
        self.assertFalse(report["applied"])
        self.assertTrue(report["refused"])
