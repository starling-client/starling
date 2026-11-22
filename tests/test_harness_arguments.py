"""tools/parity.py starts every harness with no arguments, so a harness may not require one.

A script that needs arguments is a proof run by hand, and says how in PARITY_MANUAL, which the runner reads to leave
it out. boat_passenger_tree and passenger_relation required --work-dir and failed every run that selected them
(main 6b3b5cecf48, 2026-11-16).

The check reads main() and the `if __name__ == "__main__":` block, at their top level only, for an argument the
script cannot run without: `add_argument(..., required=True)`, `assert "--x" in sys.argv`, an
`if "--x" not in sys.argv:` that stops, or `sys.argv.index("--x")` with no default added before it.
"""
import ast
from pathlib import Path
import tempfile
import unittest

from tools import parity

ROOT = Path(__file__).resolve().parents[1]


def options(node):
    return {n.value for n in ast.walk(node) if isinstance(n, ast.Constant) and isinstance(n.value, str)
            and n.value.startswith("--")}


def is_argv(node):
    return ast.unparse(node) == "sys.argv"


def stops(body):
    """Whether a block only ends the run: parser.error, raise or exit."""
    if len(body) != 1:
        return False
    statement = body[0]
    return isinstance(statement, ast.Raise) or (
        isinstance(statement, ast.Expr) and isinstance(statement.value, ast.Call)
        and isinstance(statement.value.func, ast.Attribute) and statement.value.func.attr in ("error", "exit"))


def entry_blocks(tree):
    for node in tree.body:
        if isinstance(node, ast.FunctionDef) and node.name == "main":
            yield node.body
        if isinstance(node, ast.If) and "__main__" in ast.unparse(node.test):
            yield node.body


def required(tree):
    """(line, option) for each argument the script's entry stops without."""
    found = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "add_argument" \
                and any(k.arg == "required" and getattr(k.value, "value", None) is True for k in node.keywords):
            found.append((node.lineno, node.args[0].value))
    for body in entry_blocks(tree):
        supplied = set()
        for statement in body:
            if isinstance(statement, ast.Assert) and isinstance(statement.test, ast.Compare) \
                    and isinstance(statement.test.ops[0], ast.In) and is_argv(statement.test.comparators[0]):
                found += [(statement.lineno, o) for o in options(statement.test.left) - supplied]
                supplied |= options(statement.test.left)  # reported once
                continue
            if isinstance(statement, ast.If) and isinstance(statement.test, ast.Compare) \
                    and isinstance(statement.test.ops[0], ast.NotIn) and is_argv(statement.test.comparators[0]):
                named = options(statement.test.left)
                if stops(statement.body):
                    found += [(statement.lineno, o) for o in named - supplied]
                    supplied |= named  # reported once
                elif named & options(ast.Module(body=statement.body, type_ignores=[])):
                    supplied |= named  # a default added when it is missing
                continue
            if isinstance(statement, ast.If):
                continue  # what a branch reads is that branch's business
            for node in ast.walk(statement):
                if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute) and node.func.attr == "index" \
                        and is_argv(node.func.value) and node.args and isinstance(node.args[0], ast.Constant) \
                        and node.args[0].value not in supplied:
                    found.append((node.lineno, node.args[0].value))
    return found


class HarnessArguments(unittest.TestCase):
    def test_a_harness_the_runner_starts_needs_no_arguments(self):
        harnesses = sorted((ROOT / "tools").glob("test_*_parity.py"))
        self.assertTrue(harnesses)
        for harness in harnesses:
            if parity.harness_settings(harness).get("PARITY_MANUAL"):
                continue  # the runner leaves it out
            tree = ast.parse(harness.read_text(encoding="utf-8"))
            with self.subTest(harness=harness.name):
                self.assertEqual(required(tree), [], f"{harness.name} needs arguments tools/parity.py never "
                                 "passes: give each a default, or declare PARITY_MANUAL with how to run it")

    def test_the_check_sees_each_way_a_script_requires_an_argument(self):
        cases = {
            "def main():\n    assert '--work-dir' in sys.argv\n": [(2, "--work-dir")],
            "def main():\n    if '--work-dir' not in sys.argv: parser.error('needed')\n": [(2, "--work-dir")],
            "def main():\n    p.add_argument('--side', required=True)\n": [(2, "--side")],
            "if __name__ == '__main__':\n    x = sys.argv[sys.argv.index('--work-dir') + 1]\n": [(2, "--work-dir")],
            "def main():\n    if '--work-dir' not in sys.argv:\n        sys.argv += ['--work-dir', 'd']\n"
            "    x = sys.argv[sys.argv.index('--work-dir') + 1]\n": [],
            "def main():\n    if '--reference' in sys.argv:\n        i = sys.argv.index('--reference')\n": [],
        }
        for source, expected in cases.items():
            with self.subTest(source=source):
                self.assertEqual(required(ast.parse(source)), expected)

    def test_only_a_proof_the_runner_leaves_out_is_exempt(self):
        cases = {'PARITY_MANUAL = "run by hand"\n': True, 'PARITY_MANUAL = ""\n': False,
                 'PARITY_MANUAL = OTHER = "run by hand"\n': False}
        with tempfile.TemporaryDirectory() as folder:
            for source, exempt in cases.items():
                with self.subTest(source=source):
                    harness = Path(folder) / "test_x_parity.py"
                    harness.write_text(source)
                    self.assertEqual(bool(parity.harness_settings(harness).get("PARITY_MANUAL")), exempt)


if __name__ == "__main__":
    unittest.main()
