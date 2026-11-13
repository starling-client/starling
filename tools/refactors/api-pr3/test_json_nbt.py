#!/usr/bin/env python3
"""Focused boundary proof, not workspace CI. All generated files stay in target/."""
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
OUT = ROOT / "target/json-nbt-proof"


def run(*args, **kwargs):
    subprocess.run(args, check=True, **kwargs)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    game = ROOT / ".local/minecraft/26.3"
    classpath = os.pathsep.join(map(str, [game / "jars/server-game.jar", *sorted((game / "libraries/server").rglob("*.jar"))]))
    run("javac", "-cp", classpath, "-d", str(OUT), str(HERE / "JsonNbtOracle.java"))
    oracle = OUT / "oracle.json"
    run("java", "-cp", str(OUT) + os.pathsep + classpath, "JsonNbtOracle", str(HERE / "json_nbt_samples.json"), str(oracle))
    # Compile the exact production source in a minimal crate: no graphics/game
    # build is needed to test a pure JSON/NBT boundary.
    (OUT / "src").mkdir(exist_ok=True)
    (OUT / "Cargo.toml").write_text(f'''[package]
name = "json-nbt-proof"
version = "0.0.0"
edition = "2024"
[workspace]
[dependencies]
horizon-core = {{ path = {json.dumps(str(ROOT / "crates/horizon-core"))} }}
serde_json = "1.0.151"
''')
    (OUT / "src/lib.rs").write_text(
        f'#[path = {json.dumps(str(ROOT / "crates/horizon-client/src/json_nbt.rs"))}]\npub mod production;\n'
        f'#[cfg(test)]\n#[path = {json.dumps(str(HERE / "json_nbt_parity.rs"))}]\nmod parity;\n'
    )
    env = dict(os.environ, JSON_NBT_SAMPLES=str(HERE / "json_nbt_samples.json"), JSON_NBT_ORACLE=str(oracle))
    run("cargo", "test", "--manifest-path", str(OUT / "Cargo.toml"), env=env)
    production = ROOT / "crates/horizon-client/src/json_nbt.rs"
    original = production.read_text()
    mutations = [
        ("wrong integer width", "return Tag::Long(value);", "return Tag::Int(value as i32);"),
        ("wrong fraction width", "f64::from(float) == value", "true"),
        ("accept nested null", "Value::Null if nested => return Err(JsonNbtError::NestedNull),", "Value::Null if nested => return Ok(Tag::End),"),
        ("numeric arrays", "Value::Array(values) => Tag::List(", "Value::Array(values) if values == &vec![serde_json::json!(1), serde_json::json!(2)] => Tag::ByteArray(vec![1, 2]),\n        Value::Array(values) => Tag::List("),
    ]
    try:
        for name, old, new in mutations:
            assert original.count(old) == 1, name
            production.write_text(original.replace(old, new))
            result = subprocess.run(["cargo", "test", "--manifest-path", str(OUT / "Cargo.toml"), "--lib", "production::tests"], env=env, capture_output=True, text=True)
            if result.returncode != 101 or "test result: FAILED" not in result.stdout:
                raise AssertionError(f"Mutation did not fail an assertion: {name}\n{result.stdout}\n{result.stderr}")
            print(f"Killed mutation: {name}", flush=True)
    finally:
        production.write_text(original)
    run("cargo", "test", "--manifest-path", str(OUT / "Cargo.toml"), env=env)


if __name__ == "__main__":
    main()
