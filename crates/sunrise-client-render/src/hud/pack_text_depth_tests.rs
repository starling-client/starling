//! `GlslCompiler` defines `RENDERPEARL_DEPTH_IS_ZERO_TO_ONE` for every program it compiles on a
//! backend whose depth runs from 0 to 1, as wgpu's does: each pack core shader the atlas loads
//! (`load_shaders`) is translated, and its includes chosen, with it.
use super::text_tests::source;
use super::*;

const DEPTH: &str = "RENDERPEARL_DEPTH_IS_ZERO_TO_ONE";

/// A vertex stage any of the pipelines takes: its glue declares the vertex format's inputs.
const VSH: &str = "#version 330\nvoid main() {\n    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}\n";

/// The pipelines' stages (vertex, fragment), as `load_shaders` reads them.
const PIPELINES: [(&str, &str); 5] = [
    ("text", "text"),
    ("rendertype_lines", "rendertype_lines"),
    ("sky", "sky"),
    ("stars", "stars"),
    ("screenquad", "lightmap"),
];

/// The first pipeline Sunrise draws with each of [`PIPELINES`].
const DRAWN: [&str; 5] = [
    "minecraft:pipeline/text",
    "minecraft:pipeline/lines",
    "minecraft:pipeline/sky",
    "minecraft:pipeline/stars",
    "minecraft:pipeline/lightmap",
];

fn stage(name: &str, extension: &str) -> String {
    format!("assets/minecraft/shaders/core/{name}.{extension}")
}

/// A fragment stage declaring `Other`, a block no pipeline gives, and reading it only between
/// `condition` (a `#if` line) and its `#endif`: where it is read, the load fails.
fn fragment(condition: &str) -> String {
    format!(
        "#version 330\nlayout(std140) uniform Other {{\n    vec4 Tint;\n}};\n\
         layout(location = 0) out vec4 fragColor;\nvoid main() {{\n    \
         fragColor = vec4(0.25, 0.5, 0.75, 1.0);\n{condition}\n    fragColor *= Tint;\n#endif\n}}\n"
    )
}

/// The game's stages of every pipeline, the plain fragment stage `plain`, and `extra` files.
fn game(plain: &str, extra: &[(&str, String)]) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = PIPELINES
        .iter()
        .flat_map(|(v, f)| {
            [
                (stage(v, "vsh"), VSH.to_owned()),
                (stage(f, "fsh"), plain.to_owned()),
            ]
        })
        .collect();
    files.extend(extra.iter().map(|(p, t)| ((*p).to_owned(), t.clone())));
    files
}

/// Which programs load (the GUI's, the world's and the see-through text, the block outline,
/// the sky, the stars, the lightmap) from the packs at `root` over the game's `defaults`, or the
/// required pipeline that fails the load.
fn loaded(
    root: &[(String, String)],
    defaults: &[(String, String)],
) -> Result<[bool; 7], &'static str> {
    let files = |f: &[(String, String)]| {
        source(
            &f.iter()
                .map(|(p, t)| (p.as_str(), t.clone()))
                .collect::<Vec<_>>(),
        )
    };
    let mut atlas = super::super::HudAtlas::boot();
    load_shaders(&mut atlas, &files(root), &files(defaults)).map_err(|e| e.pipeline)?;
    let text = &atlas.text_shaders;
    Ok([
        text.gui.is_some(),
        text.world.is_some(),
        text.see_through.is_some(),
        atlas.lines_shader.is_some(),
        atlas.sky_shader.is_some(),
        atlas.stars_shader.is_some(),
        atlas.lightmap_shader.is_some(),
    ])
}

/// The programs pipeline `i` of [`PIPELINES`] makes, in [`loaded`]'s order.
fn programs_of(i: usize) -> [bool; 7] {
    let range = match i {
        0 => 0..3,
        n => n + 2..n + 3,
    };
    std::array::from_fn(|p| range.contains(&p))
}

/// A pack replacing one pipeline's fragment stage with one that reads a block no pipeline gives
/// only where the define is missing (`#ifndef`, `#if !defined`) loads every program of that
/// pipeline, and only that pipeline's; one reading it only where the define is there (`#ifdef`,
/// `#if defined`) fails the load at the first pipeline Sunrise draws with it. `#undef` takes it
/// away for the rest of the stage.
#[test]
fn every_core_program_is_translated_with_the_zero_to_one_depth_define() {
    let plain = fragment("#if 0");
    let defaults = game(&plain, &[]);
    for (i, (_, name)) in PIPELINES.iter().enumerate() {
        for (condition, loads) in [
            (format!("#ifndef {DEPTH}"), true),
            (format!("#if !defined({DEPTH})"), true),
            (format!("#ifdef {DEPTH}"), false),
            (format!("#if defined({DEPTH})"), false),
            (format!("#undef {DEPTH}\n#ifndef {DEPTH}"), false),
        ] {
            let mut root = defaults.clone();
            let fsh = stage(name, "fsh");
            root.retain(|(p, _)| *p != fsh);
            root.push((fsh, fragment(&condition)));
            let expected = if loads {
                Ok(programs_of(i))
            } else {
                Err(DRAWN[i])
            };
            assert_eq!(loaded(&root, &defaults), expected, "{name}: {condition}");
        }
    }
    assert_eq!(
        loaded(&defaults, &defaults),
        Ok([false; 7]),
        "the game's own"
    );
}

/// Telling a pack's shader from the game's sees the define as the translation does: a stage the
/// same as the game's whose include behind `#ifdef` the pack replaces is the pack's, and
/// translated with the pack's include; one whose replaced include is behind `#ifndef`, which
/// the compiler drops, is the game's.
#[test]
fn an_include_override_counts_only_where_the_depth_define_keeps_it() {
    const INCLUDE: &str = "assets/minecraft/shaders/include/depth_branch.glsl";
    for (directive, changed) in [("#ifdef", true), ("#ifndef", false)] {
        let fsh = fragment("#if 0").replace(
            "layout(location = 0) out",
            &format!("{directive} {DEPTH}\n#moj_import <depth_branch.glsl>\n#endif\nlayout(location = 0) out"),
        );
        let defaults = game(&fsh, &[(INCLUDE, "const float hz_game = 1.0;".to_owned())]);
        let mut root = defaults.clone();
        root.retain(|(p, _)| p != INCLUDE);
        root.push((INCLUDE.to_owned(), "const float hz_pack = 1.0;".to_owned()));
        let expected = [changed; 7];
        assert_eq!(loaded(&root, &defaults), Ok(expected), "{directive}");
        // An include the pack replaces with one that doesn't translate fails the load where it
        // counts (the text's pipeline first), and changes nothing where the compiler drops it.
        root.retain(|(p, _)| p != INCLUDE);
        root.push((INCLUDE.to_owned(), "uniform float Broken;".to_owned()));
        let broken = if changed {
            Err("minecraft:pipeline/text")
        } else {
            Ok([false; 7])
        };
        assert_eq!(loaded(&root, &defaults), broken, "{directive}: broken");
    }
}

/// The define is there before the pack's code, as shaderc's command-line macros are: the pack
/// defining it again the same way (whitespace aside) changes nothing, and defining it otherwise
/// is a redefinition the compiler refuses, as glslang does, which fails the load. Another macro
/// of the pack's is its own.
#[test]
fn a_pack_redefining_the_depth_define_otherwise_is_refused() {
    let defaults = game(&fragment("#if 0"), &[]);
    for (define, loads) in [
        (format!("#define {DEPTH}"), true),
        (format!("#define {DEPTH}   "), true),
        (format!("#define {DEPTH} 1"), false),
        ("#define PACK_OWN 1".to_owned(), true),
    ] {
        let fsh = stage("text", "fsh");
        let mut root = defaults.clone();
        root.retain(|(p, _)| *p != fsh);
        let text = fragment(&format!("#ifndef {DEPTH}"))
            .replace("#version 330\n", &format!("#version 330\n{define}\n"));
        root.push((fsh, text));
        let expected = if loads {
            Ok(programs_of(0))
        } else {
            Err("minecraft:pipeline/text")
        };
        assert_eq!(loaded(&root, &defaults), expected, "{define}");
    }
}
