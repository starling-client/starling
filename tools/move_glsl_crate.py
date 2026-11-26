#!/usr/bin/env python3
"""Move the core-shader GLSL translator out of sunrise-client-render into its own crate, sunrise-glsl.

Run once from the repository root. It only moves files and wires the new crate into the workspace;
the hand-written commits after it give the crate its inputs and the vanilla layer its wrapper.
"""

import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent
RENDER = "crates/sunrise-client-render/src"
GLSL = "crates/sunrise-glsl"

MOVES = [
    (f"{RENDER}/core_shader.rs", f"{GLSL}/src/lib.rs"),
    (f"{RENDER}/core_shader_texel_tests.rs", f"{GLSL}/src/texel_tests.rs"),
    (f"{RENDER}/core_shader/glsl_fixups.rs", f"{GLSL}/src/glsl_fixups.rs"),
    (f"{RENDER}/core_shader/glsl_fixups/tests/mod.rs", f"{GLSL}/src/glsl_fixups/tests/mod.rs"),
    (f"{RENDER}/core_shader/import_conditions.rs", f"{GLSL}/src/import_conditions.rs"),
]

MANIFEST = """[package]
name = "sunrise-glsl"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "One GLSL program and a description of its inputs as validated naga modules"

[dependencies]
naga = { version = "30.0.1", features = ["glsl-in"] }
pp-rs = "0.2.1"

[lints]
workspace = true
"""


def git(*args: str) -> None:
    subprocess.run(["git", *args], cwd=ROOT, check=True)


def main() -> None:
    for source, target in MOVES:
        (ROOT / target).parent.mkdir(parents=True, exist_ok=True)
        git("mv", source, target)
    (ROOT / GLSL / "Cargo.toml").write_text(MANIFEST)

    workspace = ROOT / "Cargo.toml"
    text = workspace.read_text()
    text = text.replace('    "crates/sunrise-shaders",\n', '    "crates/sunrise-glsl",\n    "crates/sunrise-shaders",\n', 1)
    workspace.write_text(text)

    render = ROOT / "crates/sunrise-client-render/Cargo.toml"
    text = render.read_text()
    text = text.replace(
        'sunrise-shaders = { path = "../sunrise-shaders" }\n',
        'sunrise-glsl = { path = "../sunrise-glsl" }\nsunrise-shaders = { path = "../sunrise-shaders" }\n',
        1,
    )
    render.write_text(text)

    lib = ROOT / GLSL / "src/lib.rs"
    text = lib.read_text()
    text = text.replace('#[path = "core_shader_texel_tests.rs"]\n', '#[path = "texel_tests.rs"]\n')
    lib.write_text(text)

    layers = ROOT / "tools/layers.json"
    text = layers.read_text()
    text = re.sub(r'(\n(\s*)"sunrise-core": "engine",\n)', r'\1\2"sunrise-glsl": "engine",\n', text, count=1)
    layers.write_text(text)

    git("add", "Cargo.toml", "crates/sunrise-client-render/Cargo.toml", f"{GLSL}/Cargo.toml", "tools/layers.json", f"{GLSL}/src/lib.rs")


if __name__ == "__main__":
    main()
