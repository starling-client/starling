//! Every pipeline vanilla 26.3 requires of the core shaders the atlas loads (`ShaderManager.apply`
//! compiles `RenderPipelines.requiredPipelines()` whatever the frame draws): a pack's shader that
//! one of them can't be made from fails the load, named by that pipeline, the grayscale text and
//! the Improved Transparency stages of the text and the block outline included, which Sunrise
//! never draws with the pack's shader.
use super::text_tests::source;
use super::*;

/// A vertex stage any of the pipelines takes: its glue declares the vertex format's inputs.
const VSH: &str = "#version 330\nvoid main() {\n    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n}\n";

/// A fragment stage every pipeline takes.
const FSH: &str = "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
                   void main() {\n    fragColor = vec4(0.25, 0.5, 0.75, 1.0);\n}\n";

/// The shaders `load_shaders` reads (vertex, fragment).
const SHADERS: [(&str, &str); 5] = [
    ("text", "text"),
    ("rendertype_lines", "rendertype_lines"),
    ("sky", "sky"),
    ("stars", "stars"),
    ("screenquad", "lightmap"),
];

fn stage(name: &str, extension: &str) -> String {
    format!("assets/minecraft/shaders/core/{name}.{extension}")
}

/// [`FSH`] using an undeclared name, which no translator takes, where `condition` (a `#if`
/// expression) holds.
fn failing_where(condition: &str) -> String {
    FSH.replace(
        "}\n",
        &format!("#if {condition}\n    fragColor = undeclared;\n#endif\n}}\n"),
    )
}

/// The game's stages of every shader, and `extra` files.
fn game(extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = SHADERS
        .iter()
        .flat_map(|(v, f)| [(stage(v, "vsh"), VSH.into()), (stage(f, "fsh"), FSH.into())])
        .collect();
    files.extend(extra.iter().cloned());
    files
}

/// `files` with `replaced`'s paths replaced.
fn with(files: &[(String, String)], replaced: &[(String, String)]) -> Vec<(String, String)> {
    let mut files = files.to_vec();
    files.retain(|(p, _)| replaced.iter().all(|(r, _)| r != p));
    files.extend(replaced.iter().cloned());
    files
}

fn resources(files: &[(String, String)]) -> ResourceSource {
    source(
        &files
            .iter()
            .map(|(p, t)| (p.as_str(), t.clone()))
            .collect::<Vec<_>>(),
    )
}

/// The atlas's load of the packs at `root` over the game's `defaults`: the required pipeline
/// that fails it, else whether each drawn program loaded (the GUI's, the world's and the
/// see-through text, the block outline, the sky, the stars, the lightmap).
fn load(
    root: &[(String, String)],
    defaults: &[(String, String)],
) -> Result<[bool; 7], &'static str> {
    let mut atlas = super::super::HudAtlas::boot();
    load_shaders(&mut atlas, &resources(root), &resources(defaults)).map_err(|e| e.pipeline)?;
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

/// A pack's fragment stage that only one required pipeline can't make fails the load, named by
/// it: each grayscale text pipeline, each Improved Transparency stage of `OIT_TEXT`,
/// `OIT_TEXT_GRAYSCALE` and `OIT_LINES_TRANSLUCENT`, and the pipelines Sunrise draws. Where no
/// pipeline keeps the broken line, the pack's programs load.
#[test]
fn a_program_any_required_pipeline_cant_make_fails_the_load() {
    let defaults = game(&[]);
    let text = |condition: &str| ("text", condition.to_owned());
    let plain = "!defined(OIT) && !defined(IS_GUI) && !defined(IS_SEE_THROUGH)";
    for ((shader, condition), pipeline) in [
        (
            text(&format!("{plain} && !defined(IS_GRAYSCALE)")),
            "minecraft:pipeline/text",
        ),
        (
            text(&format!("{plain} && defined(IS_GRAYSCALE)")),
            "minecraft:pipeline/text_grayscale",
        ),
        (
            text("defined(OIT_DEPTH_BOUNDS)"),
            "minecraft:pipeline/oit_depth_bounds_text",
        ),
        (
            text("defined(OIT_TRANSMITTANCE)"),
            "minecraft:pipeline/oit_transmittance_text",
        ),
        (
            text("defined(OIT_ACCUMULATE)"),
            "minecraft:pipeline/oit_accumulate_text",
        ),
        (
            text("defined(OIT_DEPTH_BOUNDS) && defined(IS_GRAYSCALE)"),
            "minecraft:pipeline/oit_depth_bounds_text_grayscale",
        ),
        (
            text("defined(OIT_TRANSMITTANCE) && defined(IS_GRAYSCALE)"),
            "minecraft:pipeline/oit_transmittance_text_grayscale",
        ),
        (
            text("defined(OIT_ACCUMULATE) && defined(IS_GRAYSCALE)"),
            "minecraft:pipeline/oit_accumulate_text_grayscale",
        ),
        (
            text("defined(IS_GUI) && !defined(IS_GRAYSCALE)"),
            "minecraft:pipeline/gui_text",
        ),
        (
            text("defined(IS_GUI) && defined(IS_GRAYSCALE)"),
            "minecraft:pipeline/gui_text_grayscale",
        ),
        (
            text("defined(IS_SEE_THROUGH) && !defined(IS_GRAYSCALE)"),
            "minecraft:pipeline/text_see_through",
        ),
        (
            text("defined(IS_SEE_THROUGH) && defined(IS_GRAYSCALE)"),
            "minecraft:pipeline/text_grayscale_see_through",
        ),
        (
            ("rendertype_lines", "!defined(OIT)".into()),
            "minecraft:pipeline/lines",
        ),
        (
            ("rendertype_lines", "defined(OIT_DEPTH_BOUNDS)".into()),
            "minecraft:pipeline/oit_depth_bounds_lines_translucent",
        ),
        (
            ("rendertype_lines", "defined(OIT_TRANSMITTANCE)".into()),
            "minecraft:pipeline/oit_transmittance_lines_translucent",
        ),
        (
            ("rendertype_lines", "defined(OIT_ACCUMULATE)".into()),
            "minecraft:pipeline/oit_accumulate_lines_translucent",
        ),
        (("sky", "1".into()), "minecraft:pipeline/sky"),
        (("stars", "1".into()), "minecraft:pipeline/stars"),
        (("lightmap", "1".into()), "minecraft:pipeline/lightmap"),
    ] {
        let root = with(
            &defaults,
            &[(stage(shader, "fsh"), failing_where(&condition))],
        );
        assert_eq!(
            load(&root, &defaults),
            Err(pipeline),
            "{shader}: {condition}"
        );
    }
    // Every required pipeline of the text, the outline and the others makes its program.
    for (i, (_, name)) in SHADERS.iter().enumerate() {
        let root = with(&defaults, &[(stage(name, "fsh"), failing_where("0"))]);
        let drawn: [bool; 7] = std::array::from_fn(|p| match i {
            0 => p < 3,
            n => p == n + 2,
        });
        assert_eq!(load(&root, &defaults), Ok(drawn), "{name}");
    }
}

/// Vanilla's link check runs for every required pipeline: a varying whose type differs between
/// the stages only under `IS_GRAYSCALE` fails the load at the grayscale text, which Sunrise never
/// draws with the pack's shader.
#[test]
fn a_varying_mismatch_in_a_pipeline_sunrise_doesnt_draw_fails_the_load() {
    let defaults = game(&[]);
    let vsh = "#version 330\nlayout(location = 0) out vec2 tag;\n\
               void main() {\n    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);\n    tag = vec2(1.0);\n}\n";
    let fsh = |grayscale: &str| {
        format!(
            "#version 330\n#ifdef IS_GRAYSCALE\nlayout(location = 0) {grayscale} tag;\n#else\n\
             layout(location = 0) in vec2 tag;\n#endif\nlayout(location = 0) out vec4 fragColor;\n\
             void main() {{\n    fragColor = vec4(tag.x);\n}}\n"
        )
    };
    let pack = |grayscale| {
        with(
            &defaults,
            &[
                (stage("text", "vsh"), vsh.to_owned()),
                (stage("text", "fsh"), fsh(grayscale)),
            ],
        )
    };
    let text = std::array::from_fn(|p| p < 3);
    assert_eq!(load(&pack("in vec2"), &defaults), Ok(text));
    for mismatch in ["in vec3", "flat in vec2"] {
        assert_eq!(
            load(&pack(mismatch), &defaults),
            Err("minecraft:pipeline/text_grayscale"),
            "{mismatch}"
        );
    }
}

/// A pack replacing only an include the text keeps under `IS_GRAYSCALE` changes the text, which
/// is then the pack's in every pipeline: one that translates loads the drawn programs, one that
/// doesn't fails the load at the grayscale text. Where every pipeline drops the include, the
/// game's text stays.
#[test]
fn an_include_only_a_grayscale_pipeline_keeps_counts() {
    const INCLUDE: &str = "assets/minecraft/shaders/include/gray.glsl";
    let fsh = |directive: &str| {
        FSH.replace(
            "layout(location = 0) out",
            &format!("{directive}\n#moj_import <gray.glsl>\n#endif\nlayout(location = 0) out"),
        )
    };
    for (directive, kept) in [("#ifdef IS_GRAYSCALE", true), ("#if 0", false)] {
        let game_include = (INCLUDE.to_owned(), "const float hz_game = 1.0;".to_owned());
        let defaults = with(
            &game(&[game_include]),
            &[(stage("text", "fsh"), fsh(directive))],
        );
        let own = |text: &str| with(&defaults, &[(INCLUDE.to_owned(), text.to_owned())]);
        let text = std::array::from_fn(|p| kept && p < 3);
        assert_eq!(
            load(&own("const float hz_pack = 1.0;"), &defaults),
            Ok(text),
            "{directive}"
        );
        let broken = if kept {
            Err("minecraft:pipeline/text_grayscale")
        } else {
            Ok([false; 7])
        };
        assert_eq!(
            load(&own("uniform float Broken;"), &defaults),
            broken,
            "{directive}: broken"
        );
    }
}

/// A read block the pipeline can't give (one that isn't one float) fails the load, named by the
/// pipeline whose program reads it, whether Sunrise draws that pipeline or not: as vanilla's
/// builder fails on a descriptor missing from the layout.
#[test]
fn a_block_the_pipeline_cant_give_fails_the_load() {
    let defaults = game(&[]);
    // [`FSH`] reading `Missing`, a `vec4` block, where `condition` holds.
    let reading = |condition: &str| {
        FSH.replace(
            "void main() {",
            &format!(
                "#if {condition}\nlayout(std140) uniform Missing {{\n    vec4 value;\n}};\n#define TINT value\n\
                 #else\n#define TINT vec4(1.0)\n#endif\nvoid main() {{"
            ),
        )
        .replace("fragColor = vec4(0.25, 0.5, 0.75, 1.0);", "fragColor = TINT;")
    };
    let plain = "!defined(OIT) && !defined(IS_GUI) && !defined(IS_SEE_THROUGH)";
    for ((shader, condition), pipeline) in [
        (("sky", "1".to_owned()), "minecraft:pipeline/sky"),
        (
            ("text", format!("{plain} && defined(IS_GRAYSCALE)")),
            "minecraft:pipeline/text_grayscale",
        ),
        (
            (
                "text",
                "defined(IS_GUI) && defined(IS_GRAYSCALE)".to_owned(),
            ),
            "minecraft:pipeline/gui_text_grayscale",
        ),
        (
            (
                "text",
                "defined(IS_SEE_THROUGH) && defined(IS_GRAYSCALE)".to_owned(),
            ),
            "minecraft:pipeline/text_grayscale_see_through",
        ),
        (
            ("text", "defined(OIT_TRANSMITTANCE)".to_owned()),
            "minecraft:pipeline/oit_transmittance_text",
        ),
        (
            ("rendertype_lines", "defined(OIT_ACCUMULATE)".to_owned()),
            "minecraft:pipeline/oit_accumulate_lines_translucent",
        ),
    ] {
        let root = with(&defaults, &[(stage(shader, "fsh"), reading(&condition))]);
        assert_eq!(
            load(&root, &defaults),
            Err(pipeline),
            "{shader}: {condition}"
        );
    }
    // Where no pipeline reads it, the pack's programs load.
    let root = with(&defaults, &[(stage("text", "fsh"), reading("0"))]);
    assert_eq!(load(&root, &defaults), Ok(std::array::from_fn(|p| p < 3)));
}

/// A `DynamicTransforms` with `extra` members past 26.3's, and a fragment stage reading
/// `ColorModulator` from it.
fn transforms_reading(extra: &str) -> String {
    format!(
        "#version 330\nlayout(std140) uniform DynamicTransforms {{\n    mat4 ModelViewMat;\n    \
         mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n{extra}}};\n\
         layout(location = 0) out vec4 fragColor;\nvoid main() {{\n    fragColor = ColorModulator;\n}}\n"
    )
}

/// An older vanilla's `DynamicTransforms`, larger than 26.3's, which vanilla's builder takes (it
/// checks a descriptor's name, type and dimension, not a block's size): within the block's slot
/// the program loads, bound at its size; past the slot (the sky's 160 bytes and an 8-member
/// array, 288) the sky draws natively, the pack stays, and the note names the block and its
/// size, as before this change.
#[test]
fn an_older_larger_vanilla_block_loads_or_draws_natively() {
    let defaults = game(&[]);
    let sky = |extra: &str| {
        let root = with(
            &defaults,
            &[(stage("sky", "fsh"), transforms_reading(extra))],
        );
        load(&root, &defaults)
    };
    let drawn_sky = std::array::from_fn(|p| p == 4);
    assert_eq!(sky("    vec4 More;\n"), Ok(drawn_sky));
    assert_eq!(
        sky("    vec4 More[6];\n"),
        Ok(drawn_sky),
        "256 bytes, the whole slot"
    );
    assert_eq!(sky("    vec4 More[8];\n"), Ok([false; 7]));
    let program = |extra: &str| {
        crate::core_shader::translate_core(
            VSH,
            &transforms_reading(extra),
            &[],
            &|_| None,
            &crate::gfx::core_sky::GLUE,
        )
        .unwrap()
    };
    use crate::gfx::expression_blocks::{OversizedBlock, oversized_block};
    assert_eq!(oversized_block(&program("    vec4 More[6];\n"), &[]), None);
    assert_eq!(
        oversized_block(&program("    vec4 More[8];\n"), &[]),
        Some(OversizedBlock {
            block: "DynamicTransforms",
            size: 288
        })
    );
}

/// Vanilla's own shaders (the prepared resources), as a pack copying them ships them, make every
/// required program: each drawn one loads, none fails the load.
#[test]
#[ignore = "daily"]
fn vanillas_own_core_shaders_make_every_required_program() {
    let root = std::env::var_os("SUNRISE_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::VANILLA_VERSION)
        });
    let vanilla = ResourceSource::directory(root);
    assert!(
        vanilla.contains("assets/minecraft/shaders/core/text.fsh"),
        "prepared resources"
    );
    let mut atlas = super::super::HudAtlas::boot();
    // Over no game resources, every shader is a pack's.
    load_shaders(&mut atlas, &vanilla, &ResourceSource::default()).unwrap();
    let text = &atlas.text_shaders;
    assert!(text.gui.is_some() && text.world.is_some() && text.see_through.is_some());
    assert!(atlas.lines_shader.is_some());
    assert!(atlas.sky_shader.is_some() && atlas.stars_shader.is_some());
    assert!(atlas.lightmap_shader.is_some() && atlas.clouds_shader.is_some());
}
