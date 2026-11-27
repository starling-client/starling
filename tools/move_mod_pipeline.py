#!/usr/bin/env python3
"""Move mod render pipelines' shader code out of the WGSL shader-pack crate into its own crate,
sunrise-mod-pipeline: the composer (mod_pipeline.rs), the crash guard (guard.rs), and the parts of
the pack checker both use (Problem, validate, located, translate, Composed, the colour helpers).
The pack crate then uses these from the new crate. Run once from the checkout root."""
import subprocess
from pathlib import Path

PACK = Path("crates/sunrise-shader-api")
NEW = Path("crates/sunrise-mod-pipeline")


def replace(path, old, new, occurrences=1):
    path = Path(path)
    text = path.read_text()
    if text.count(old) != occurrences:
        raise ValueError(f"{path}: expected {occurrences} occurrences of {old!r}")
    path.write_text(text.replace(old, new))


def cut(path, start, end):
    """Remove and return the text of `path` from the line `start` begins to the line `end`
    begins (not included)."""
    path = Path(path)
    text = path.read_text()
    a = text.index(start)
    b = text.index(end, a)
    if text.count(start) != 1:
        raise ValueError(f"{path}: {start!r} is not unique")
    path.write_text(text[:a] + text[b:])
    return text[a:b]


def main():
    (NEW / "src").mkdir(parents=True)
    subprocess.run(["git", "mv", PACK / "src/mod_pipeline.rs", NEW / "src/lib.rs"], check=True)
    subprocess.run(["git", "mv", PACK / "src/guard.rs", NEW / "src/guard.rs"], check=True)

    check = PACK / "src/check.rs"
    stages = PACK / "src/stages.rs"
    problem = cut(check, "/// What is wrong, and where:", "/// One stage that passed")
    validate = cut(check, "/// Validates `module`, parsed from", "/// A refusal for each of `cost`'s")
    located = cut(check, "pub(crate) fn located(", "fn parse(composed: &Composed)")
    translate = cut(check, "/// Translates every entry point for Vulkan", "#[cfg(test)]")
    composed = cut(stages, "/// A stage's WGSL, and where each part of it came from.",
                   "/// The binding of a pack's first post target")
    colors = cut(stages, "/// 0.2's colour boundary,", "const POST_RESOURCES")

    (NEW / "src/check.rs").write_text(
        "//! The checks a shader of the game's passes before the renderer builds it: naga\n"
        "//! validation located in the file a refusal comes from, and translation for every backend\n"
        "//! wgpu uses.\n"
        "use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};\n"
        "use std::collections::HashSet;\n"
        "use std::fmt;\n\n"
        + problem + composed + validate + located + translate)
    lib = NEW / "src/lib.rs"
    replace(lib, "use crate::check::{Problem, located, problem, translate, validate};\n"
                 "use crate::stages::Composed;\n",
            "pub mod check;\npub mod guard;\n\n"
            "use crate::check::{Composed, Problem, located, problem, translate, validate};\n")
    replace(lib, "        crate::stages::COLOR_HELPERS\n", "        COLOR_HELPERS\n")
    replace(lib, "/// The folder under each namespace's `shaders`", colors + "/// The folder under each namespace's `shaders`")

    (NEW / "Cargo.toml").write_text('''[package]
name = "sunrise-mod-pipeline"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Mod render pipelines' WGSL: composed with the renderer's prelude and entry points, and checked as wgpu builds it"

[dependencies]
# Parses and validates the shader, and translates it as wgpu does for Vulkan, Metal and DirectX 12.
naga = { version = "30.0.1", features = ["wgsl-in", "spv-out", "msl-out", "hlsl-out"] }

[lints]
workspace = true
''')
    replace("Cargo.toml", '    "crates/sunrise-shader-api",\n',
            '    "crates/sunrise-shader-api",\n    "crates/sunrise-mod-pipeline",\n')

    replace(PACK / "Cargo.toml", 'serde = { version = "1", features = ["derive"] }\n',
            'serde = { version = "1", features = ["derive"] }\n'
            'sunrise-mod-pipeline = { path = "../sunrise-mod-pipeline" }\n')
    replace(PACK / "src/lib.rs", "pub mod guard;\n", "")
    replace(PACK / "src/lib.rs", "pub mod mod_pipeline;\n", "")
    replace("crates/sunrise-client-render/Cargo.toml", 'sunrise-shader-api = { path = "../sunrise-shader-api" }\n',
            'sunrise-mod-pipeline = { path = "../sunrise-mod-pipeline" }\n'
            'sunrise-shader-api = { path = "../sunrise-shader-api" }\n')
    replace("crates/cargo-sunrise-mod/Cargo.toml", 'sunrise-shader-api = { path = "../sunrise-shader-api" }\n',
            'sunrise-mod-pipeline = { path = "../sunrise-mod-pipeline" }\n')
    replace("crates/sunrise-client-render/src/mod_pipelines.rs",
            "use sunrise_shader_api::mod_pipeline::{FOLDER, compose_with};",
            "use sunrise_mod_pipeline::{FOLDER, compose_with};")
    replace("crates/sunrise-client-render/src/mod_pipelines.rs",
            "`sunrise_shader_api::mod_pipeline`", "`sunrise_mod_pipeline`")
    replace("crates/sunrise-client-render/src/gfx/mod_pipelines.rs",
            "use sunrise_shader_api::mod_pipeline::{FRAGMENT_ENTRY, VERTEX_ENTRY};",
            "use sunrise_mod_pipeline::{FRAGMENT_ENTRY, VERTEX_ENTRY};")
    replace("crates/cargo-sunrise-mod/src/check.rs", "use sunrise_shader_api::mod_pipeline;\n", "")
    replace("crates/cargo-sunrise-mod/src/check.rs", "mod_pipeline::", "sunrise_mod_pipeline::", 2)
    replace("crates/cargo-sunrise-mod/src/check.rs", "(`sunrise_shader_api::mod_pipeline`)",
            "(`sunrise_mod_pipeline`)")


if __name__ == "__main__":
    main()
