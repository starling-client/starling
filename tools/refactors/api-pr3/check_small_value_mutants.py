#!/usr/bin/env python3
"""Compile the production enum declarations and their case-order regressions.

This small proof is independent of game resources. Cargo still checks the full
integration. Mutations affect a temporary copy of the production declaration;
no checked-in source is ever rewritten.
"""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
API = ROOT / "crates/horizon-mod-api/src"


def run(source, directory):
    path = directory / "proof.rs"
    path.write_text(source)
    binary = directory / "proof"
    subprocess.run(["rustc", "--edition", "2024", "--test", str(path), "-o", str(binary)], check=True)
    return subprocess.run([str(binary)], capture_output=True, text=True)


def main():
    target = ROOT / "target"
    target.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="small-values-", dir=target) as work:
        directory = Path(work)
        key = (API / "key_mapping.rs").read_text()
        key = key[key.index("/// The screens"):key.index("/// A key a mod")]
        shape = (API / "recipe_rule.rs").read_text()
        shape = shape[shape.index("/// The explosion"):shape.index("/// An `ItemStackTemplate`")]
        identifier = (API / "unresolved_fluid.rs").read_text()
        identifier = identifier[identifier.index("fn namespaced_id("):]
        engine = (ROOT / "crates/horizon-mod-package/src/content.rs").read_text()
        engine = engine[engine.index("pub fn namespaced_id("):engine.index("fn vanilla_id(")]
        identifier += "\nmod engine {\n" + engine + "\n}\n"
        identifier += '''
#[test]
fn engine_identifier_samples_match() {
    for id in ["minecraft:water", "mod:flowing/water", "n.s-0:x_y", "UPPER:path", ":path", "mod:", "a:b:c", "a/b:c", "mod:café"] {
        assert_eq!(namespaced_id(id), engine::namespaced_id(id), "{id}");
    }
}
'''
        for name, source, old, new in [
            ("namespaced identifiers", identifier, "ok(b) || b == b'/'", "ok(b)"),
            ("key contexts", key, "    InGame,\n", "    InGame = 2,\n"),
            ("firework shapes", shape, "    Star,\n", "    Star = 4,\n"),
        ]:
            baseline = run(source, directory)
            assert baseline.returncode == 0, baseline.stdout + baseline.stderr
            mutant = run(source.replace(old, new, 1), directory)
            assert mutant.returncode != 0, f"surviving mutant: {name}"
            print(f"{name}: baseline passed; production mutant killed")


if __name__ == "__main__":
    main()
