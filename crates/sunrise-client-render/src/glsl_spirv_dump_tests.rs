//! Every program `sunrise-glsl` translates for the game, written as SPIR-V and hashed into a text
//! manifest: vanilla's core shaders, its post shaders, and a resource pack's core, post and
//! Polytone GPU particle shaders. A change that must leave the translator's output alone (#5533
//! step 3, which moves the directives from naga's preprocessor into `sunrise-glsl`) shows it by
//! leaving the manifests byte-identical.
//!
//! Each line names a program and gives each stage's module as the SHA-256 of its SPIR-V words
//! in little-endian bytes and their count. A program that doesn't translate gives the file and
//! line of each error instead (not the message, which names what the preprocessor read).
//!
//! - `glsl_spirv_dump_matches_the_committed_fixture` (runs in `verify`): the committed pack
//!   `tests/glsl_spirv_dump/directives`, whose shaders use `#if`, `#elif`, `#ifdef`, `defined`,
//!   macros of both kinds, `#undef`, `#line`, `__LINE__` and includes under conditions, loaded
//!   through the game's entry points: the atlas's core shaders (`hud::pack_text::load_shaders`),
//!   a post pass (`post_glsl::translate`) and a GPU particle's shader
//!   (`particles::polytone::Registry::read`). Against `tests/glsl_spirv_dump/directives.txt`.
//! - `glsl_spirv_dump_matches_vanilla` (daily, `SUNRISE_TEST_RESOURCES`): vanilla's core shaders
//!   as the atlas loads them, every core vertex and fragment shader pair under no define and
//!   under each name its directives test, and every post fragment shader with each post vertex
//!   shader. Against `tests/glsl_spirv_dump/vanilla.txt`.
//!
//! To write a missing manifest, run the test alone with `SUNRISE_BLESS_GLSL_SPIRV_DUMP=1`; it
//! only creates missing files. A change meant to alter the output deletes the manifest and
//! blesses it again, and its PR says why it changed.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use sunrise_assets::resource_source::ResourceSource;
use sunrise_glsl::{Glue, Translated};

/// `module`'s SPIR-V as `<sha256> <words>`, or its validation or writer error's kind.
fn spirv(module: &naga::Module) -> String {
    let info = match naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(module)
    {
        Ok(info) => info,
        Err(e) => return format!("invalid {:?}", e.into_inner()),
    };
    match naga::back::spv::write_vec(module, &info, &naga::back::spv::Options::default(), None) {
        Ok(words) => {
            let mut hash = Sha256::new();
            for word in &words {
                hash.update(word.to_le_bytes());
            }
            format!("{:x} {}", hash.finalize(), words.len())
        }
        Err(e) => format!("unwritten {e:?}"),
    }
}

/// One manifest line: `label`, then each stage's SPIR-V, or where the errors are.
fn line(out: &mut String, label: &str, program: Result<[&naga::Module; 2], Vec<String>>) {
    match program {
        Ok([vertex, fragment]) => writeln!(
            out,
            "{label}\n  vertex {}\n  fragment {}",
            spirv(vertex),
            spirv(fragment)
        ),
        Err(at) => writeln!(out, "{label}\n  refused at {}", at.join(" ")),
    }
    .unwrap();
}

fn stages(program: &Translated) -> [&naga::Module; 2] {
    [&program.vertex, &program.fragment]
}

fn located(errors: &[sunrise_glsl::Error]) -> Vec<String> {
    errors
        .iter()
        .map(|e| format!("{}:{}", e.file, e.line))
        .collect()
}

/// The core shaders the atlas loads from `root` over `game` (`hud::pack_text::load_shaders`),
/// each program by the slot it fills: those `root` replaces.
fn atlas_programs(out: &mut String, root: &ResourceSource, game: &ResourceSource) {
    let mut atlas = crate::hud::HudAtlas::boot();
    atlas.load_pack_shaders(root, game);
    let text = &atlas.text_shaders;
    let slots: [(&str, Option<&Translated>); 8] = [
        ("lines", atlas.lines_shader.as_deref()),
        ("sky", atlas.sky_shader.as_deref()),
        ("stars", atlas.stars_shader.as_deref()),
        ("lightmap", atlas.lightmap_shader.as_deref()),
        ("text gui", text.gui.as_deref()),
        ("text world", text.world.as_deref()),
        ("text see-through", text.see_through.as_deref()),
        (
            "clouds classic",
            atlas.clouds_shader.as_deref().map(|c| &c.classic),
        ),
    ];
    for (slot, program) in slots {
        match program {
            Some(program) => line(out, &format!("atlas {slot}"), Ok(stages(program))),
            None => writeln!(out, "atlas {slot}\n  not loaded").unwrap(),
        }
    }
    if let Some(clouds) = atlas.clouds_shader.as_deref() {
        for (n, program) in clouds.oit.iter().enumerate() {
            line(out, &format!("atlas clouds oit {n}"), Ok(stages(program)));
        }
    }
}

/// A post pass of `vertex` and `fragment` (`ns:post/name`) from `root`, as a chain translates it.
fn post_program(out: &mut String, root: &ResourceSource, vertex: &str, fragment: &str) {
    let declarable = crate::post_glsl::Declarable::of(&wgpu::Limits::default());
    let label = format!("post {vertex} {fragment}");
    match crate::post_glsl::translate(root, vertex, fragment, declarable) {
        Ok(program) => line(out, &label, Ok([&program.vertex, &program.fragment])),
        Err(crate::post_glsl::Refused::Translator { stage, errors }) => {
            line(out, &format!("{label} ({stage})"), Err(located(&errors)))
        }
        Err(refused) => writeln!(out, "{label}\n  refused {refused}").unwrap(),
    }
}

/// The manifest at `manifest` (from the crate root) is `dump`; a missing one is written when
/// `SUNRISE_BLESS_GLSL_SPIRV_DUMP` is set.
fn check(manifest: &str, dump: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest);
    match std::fs::read_to_string(&path) {
        Ok(committed) => assert!(
            committed == dump,
            "{manifest} differs from this head's dump:\n{}",
            first_difference(&committed, dump)
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            assert!(
                std::env::var_os("SUNRISE_BLESS_GLSL_SPIRV_DUMP").is_some(),
                "{manifest} is missing; write it with SUNRISE_BLESS_GLSL_SPIRV_DUMP=1"
            );
            std::fs::write(&path, dump).unwrap();
        }
        Err(e) => panic!("{manifest}: {e}"),
    }
}

/// The first line where `a` and `b` differ, with both.
fn first_difference(a: &str, b: &str) -> String {
    let (mut a_lines, mut b_lines) = (a.lines(), b.lines());
    for n in 1.. {
        match (a_lines.next(), b_lines.next()) {
            (None, None) => break,
            (x, y) if x == y => {}
            (x, y) => return format!("line {n}:\n  committed {x:?}\n  this head {y:?}"),
        }
    }
    String::new()
}

/// A block registry for the particle registry: air alone.
fn blocks() -> &'static sunrise_world::registry::BlockRegistry {
    use sunrise_core::nbt::Tag;
    Box::leak(Box::new(
        sunrise_world::registry::BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            (
                "states",
                Tag::List(vec![Tag::String("minecraft:air".into())]),
            ),
            ("defaults", Tag::compound([("minecraft:air", Tag::Int(0))])),
        ]))
        .unwrap(),
    ))
}

#[test]
fn glsl_spirv_dump_matches_the_committed_fixture() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glsl_spirv_dump/directives");
    let root = ResourceSource::directory(dir);
    let mut out = String::new();
    atlas_programs(&mut out, &root, &ResourceSource::default());
    post_program(
        &mut out,
        &root,
        "fixture:post/directives",
        "fixture:post/directives",
    );
    let registry = crate::particles::polytone::Registry::read(&root, Default::default(), blocks());
    match registry.gpu_shader("fixture:rain") {
        Some(shader) => line(
            &mut out,
            "polytone fixture:rain",
            Ok(stages(&shader.program)),
        ),
        None => writeln!(out, "polytone fixture:rain\n  not loaded").unwrap(),
    }
    // The fixture reaches every path it is there for: each program it has loads (the atlas's
    // other slots are vanilla's, which it doesn't replace).
    for label in [
        "atlas text gui",
        "atlas text world",
        "atlas text see-through",
        "post fixture:post/directives",
        "polytone fixture:rain",
    ] {
        let mut lines = out.lines().skip_while(|l| !l.starts_with(label));
        assert!(
            lines.nth(1).is_some_and(|l| l.starts_with("  vertex")),
            "{label}:\n{out}"
        );
    }
    check("tests/glsl_spirv_dump/directives.txt", &out);
}

/// Vanilla's resources (`SUNRISE_TEST_RESOURCES`, else the prepared ones in `.local`).
fn vanilla_root() -> PathBuf {
    let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3")
        });
    assert!(
        root.join("assets/minecraft/shaders/core/text.vsh")
            .is_file(),
        "no vanilla shaders under {}",
        root.display()
    );
    root
}

/// Every vanilla vertex attribute, for a pair translated outside its pipeline.
const ANY_GLUE: Glue = Glue {
    inputs: &[
        ("Position", "vec3"),
        ("Color", "vec4"),
        ("UV0", "vec2"),
        ("UV1", "ivec2"),
        ("UV2", "ivec2"),
        ("Normal", "vec3"),
        ("LineWidth", "float"),
    ],
    prelude: "",
    setup: "",
    finish: "",
    vertex_id: "int(gl_VertexIndex)",
    fragment_finish: None,
    interface: sunrise_shaders::DEVICELESS_INTERFACE_LIMITS,
};

/// The names the directives of `text` test: each `#ifdef`/`#ifndef` name, each `defined` one,
/// and the other names of `#if` and `#elif` (whose value they compare).
fn tested_names(text: &str, flags: &mut BTreeSet<String>, valued: &mut BTreeSet<String>) {
    for line in text.lines() {
        let Some(directive) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let directive = directive.trim_start();
        let word_end = directive
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(directive.len());
        let (word, rest) = directive.split_at(word_end);
        if !matches!(word, "if" | "elif" | "ifdef" | "ifndef") {
            continue;
        }
        let words: Vec<&str> = rest
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty() && !w.starts_with(|c: char| c.is_ascii_digit()))
            .collect();
        let mut after_defined = false;
        for w in words {
            if w == "defined" {
                after_defined = true;
                continue;
            }
            if word.starts_with("ifn") || word == "ifdef" || after_defined {
                flags.insert(w.to_owned());
            } else {
                valued.insert(w.to_owned());
            }
            after_defined = false;
        }
    }
}

/// The ids `text` includes (`#include <ns:path>` in vanilla's layout).
fn included(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("#include"))
        .filter_map(|t| {
            t.trim()
                .strip_prefix('<')?
                .strip_suffix('>')
                .map(str::to_owned)
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn glsl_spirv_dump_matches_vanilla() {
    let root = vanilla_root();
    let source = ResourceSource::directory(root.clone());
    let mut out = String::new();
    // Over no game resources, so that vanilla's own stages count as a pack's.
    atlas_programs(&mut out, &source, &ResourceSource::default());
    let core = root.join("assets/minecraft/shaders/core");
    let read = |id: &str| -> Option<String> {
        let (ns, path) = id.split_once(':')?;
        std::fs::read_to_string(root.join("assets").join(ns).join(path)).ok()
    };
    let mut names: Vec<String> = std::fs::read_dir(&core)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for fragment in names.iter().filter_map(|n| n.strip_suffix(".fsh")) {
        // A fragment shader of its own vertex shader, else of the one vanilla's pipelines give
        // it (the full-screen quad's, or the sprite animation's).
        let vertex = if names.contains(&format!("{fragment}.vsh")) {
            fragment.to_owned()
        } else if fragment.starts_with("animate_sprite") {
            "animate_sprite".to_owned()
        } else {
            "screenquad".to_owned()
        };
        let text = |name: &str, ext: &str| {
            std::fs::read_to_string(core.join(format!("{name}.{ext}"))).unwrap()
        };
        let (vsh, fsh) = (text(&vertex, "vsh"), text(fragment, "fsh"));
        let (mut flags, mut valued) = (BTreeSet::new(), BTreeSet::new());
        let mut pending = vec![vsh.clone(), fsh.clone()];
        let mut seen = BTreeSet::new();
        while let Some(file) = pending.pop() {
            tested_names(&file, &mut flags, &mut valued);
            for id in included(&file) {
                let (ns, path) = id.split_once(':').unwrap_or(("minecraft", &id));
                let id = format!("{ns}:shaders/include/{path}");
                if seen.insert(id.clone())
                    && let Some(text) = read(&id)
                {
                    pending.push(text);
                }
            }
        }
        let mut sets: Vec<Vec<String>> = vec![Vec::new()];
        sets.extend(flags.iter().chain(&valued).map(|n| vec![n.clone()]));
        sets.extend(
            valued
                .iter()
                .flat_map(|n| [4, 8, 16].map(|v| vec![format!("{n} {v}")])),
        );
        for set in sets {
            let defines: Vec<&str> = set.iter().map(String::as_str).collect();
            let program = crate::core_shader::translate_core_with(
                &vsh,
                &fsh,
                &defines,
                &read,
                &ANY_GLUE,
                &|_| true,
            );
            let label = format!("core {vertex}.vsh {fragment}.fsh {defines:?}");
            match &program {
                Ok(program) => line(&mut out, &label, Ok(stages(program))),
                Err(errors) => line(&mut out, &label, Err(located(errors))),
            }
        }
    }
    let post = root.join("assets/minecraft/shaders/post");
    let mut post_names: Vec<String> = std::fs::read_dir(&post)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    post_names.sort();
    let mut vertices = vec!["minecraft:core/screenquad".to_owned()];
    vertices.extend(
        (post_names.iter().filter_map(|n| n.strip_suffix(".vsh")))
            .map(|n| format!("minecraft:post/{n}")),
    );
    for fragment in post_names.iter().filter_map(|n| n.strip_suffix(".fsh")) {
        for vertex in &vertices {
            post_program(
                &mut out,
                &source,
                vertex,
                &format!("minecraft:post/{fragment}"),
            );
        }
    }
    check("tests/glsl_spirv_dump/vanilla.txt", &out);
}
