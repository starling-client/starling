//! A pack's `core/text` and its Polytone expression uniforms (`shader_modifiers/core/text.json`)
//! as the atlas loads them, for the GUI's, the world's and the see-through text.
use super::*;
use sunrise_assets::resource_source::Resource;

const VSH: &str = "assets/minecraft/shaders/core/text.vsh";
const FSH: &str = "assets/minecraft/shaders/core/text.fsh";
const MODIFIERS: &str = "assets/minecraft/polytone/shader_modifiers/core/text.json";

/// `text.vsh` in shape, its blocks written out, reading `Lift` (a one-float block) in the vertex
/// stage when `lift`.
fn vsh(lift: bool) -> String {
    let (block, read) = if lift {
        (
            "layout(std140) uniform Lift {\n    float lift;\n};\n",
            " + lift",
        )
    } else {
        ("", "")
    };
    format!(
        "#version 330\n\
         layout(std140) uniform DynamicTransforms {{\n    mat4 ModelViewMat;\n    mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n}};\n\
         layout(std140) uniform Projection {{\n    mat4 ProjMat;\n}};\n{block}\
         layout(location = 0) in vec3 Position;\nlayout(location = 1) in vec4 Color;\nlayout(location = 2) in vec2 UV0;\n\
         layout(location = 2) out vec4 vertexColor;\nlayout(location = 3) out vec2 texCoord0;\n\
         void main() {{\n    gl_Position = ProjMat * ModelViewMat * vec4(Position{read}, 1.0);\n    vertexColor = Color;\n    texCoord0 = UV0;\n}}\n"
    )
}

/// `text.fsh` in shape, its colour times `Wave` (`wave`: declared as `float`, or as `vec4`, which
/// no expression uniform is).
fn fsh(wave: &str) -> String {
    format!(
        "#version 330\nuniform sampler2D Sampler0;\n\
         layout(std140) uniform Wave {{\n    {wave} wave;\n}};\n\
         layout(location = 2) in vec4 vertexColor;\nlayout(location = 3) in vec2 texCoord0;\n\
         layout(location = 0) out vec4 fragColor;\n\
         void main() {{\n    fragColor = texture(Sampler0, texCoord0) * vertexColor * float(wave);\n}}\n"
    )
}

/// A pack of `files` (path and text).
pub(crate) fn source(files: &[(&str, String)]) -> ResourceSource {
    ResourceSource::indexed(
        files
            .iter()
            .map(|(p, t)| ((*p).to_owned(), Resource::bytes(t.clone().into_bytes())))
            .collect(),
    )
}

/// The game's own text stages, and a pack's over them with its `shader_modifiers` file.
fn packs(
    vertex: String,
    fragment: String,
    modifiers: Option<&str>,
) -> (ResourceSource, ResourceSource) {
    let game = || {
        let plain = fsh("float")
            .replace(" * float(wave)", "")
            .replace("layout(std140) uniform Wave {\n    float wave;\n};\n", "");
        vec![(VSH, vsh(false)), (FSH, plain)]
    };
    let mut pack = vec![(VSH, vertex), (FSH, fragment)];
    pack.extend(modifiers.map(|m| (MODIFIERS, m.to_owned())));
    (source(&pack), source(&game()))
}

/// The three text programs and their expression uniforms, as the atlas loads them.
fn text_programs(
    root: &ResourceSource,
    defaults: &ResourceSource,
) -> [Option<(
    Arc<Translated>,
    Vec<crate::polytone_post::ExpressionUniform>,
)>; 3] {
    let mut atlas = super::super::HudAtlas::boot();
    load_shaders(&mut atlas, root, defaults).unwrap();
    let shaders = atlas.text_shaders;
    let expressions = shaders.expressions;
    [
        (shaders.gui, expressions.gui),
        (shaders.world, expressions.world),
        (shaders.see_through, expressions.see_through),
    ]
    .map(|(program, expressions)| program.map(|program| (program, expressions)))
}

/// A pack's text shader reading `Wave` in its fragment stage and `Lift` in its vertex stage,
/// with `core/text.json` giving both: every one of the three programs loads, and resolves each
/// block to the stage that reads it and its expression's value (0.25 and 2, exact in f32).
#[test]
fn the_text_programs_resolve_their_expression_uniforms_at_load() {
    let (root, defaults) = packs(
        vsh(true),
        fsh("float"),
        Some(r#"{"Wave": "0.5 * 0.5", "Lift": "1 + 1"}"#),
    );
    let frame = crate::environment::Frame::default();
    for (i, loaded) in text_programs(&root, &defaults).into_iter().enumerate() {
        let (program, expressions) = loaded.unwrap_or_else(|| panic!("program {i}"));
        let mut got: Vec<_> = expressions
            .iter()
            .map(|e| (e.name.as_str(), e.stages, e.evaluate(&frame)))
            .collect();
        got.sort_by_key(|(name, ..)| *name);
        assert_eq!(
            got,
            [
                ("Lift", wgpu::ShaderStages::VERTEX, 2.0),
                ("Wave", wgpu::ShaderStages::FRAGMENT, 0.25)
            ],
            "program {i}"
        );
        assert_eq!(program.other.len(), 2, "program {i}");
    }
}

/// Each text pipeline binds zero without a matching expression in its own modifier file.
#[test]
fn a_text_program_without_its_expression_uniform_reads_zero() {
    let (root, defaults) = packs(vsh(false), fsh("float"), None);
    let other_shader = source(&[
        (VSH, vsh(false)),
        (FSH, fsh("float")),
        (
            "assets/minecraft/polytone/shader_modifiers/core/sky.json",
            r#"{"Wave": "1"}"#.to_owned(),
        ),
    ]);
    for pack in [&root, &other_shader] {
        for loaded in text_programs(pack, &defaults) {
            let (_, expressions) = loaded.expect("the shader loads");
            assert_eq!(expressions.len(), 1);
            assert_eq!(expressions[0].name, "Wave");
            assert_eq!(expressions[0].evaluate(&Default::default()), 0.0);
        }
    }
}

/// A pack's text shader reading no expression uniform loads as before, with none.
#[test]
fn a_text_program_reading_no_expression_uniform_has_none() {
    let plain = fsh("float").replace(" * float(wave)", "");
    let (root, defaults) = packs(vsh(false), plain, Some(r#"{"Wave": "1"}"#));
    for loaded in text_programs(&root, &defaults) {
        let (_, expressions) = loaded.expect("loads");
        assert!(expressions.is_empty());
    }
}

/// A text program whose stage reads a block that isn't one float fails the load, named by its
/// pipeline, even when it is the see-through text's alone: the pipeline can't give the block.
#[test]
fn a_text_program_reading_a_non_float_expression_block_fails_the_load() {
    let fsh = "#version 330\nlayout(location = 0) out vec4 fragColor;\n\
               #ifdef IS_SEE_THROUGH\nlayout(std140) uniform Missing {\n    vec4 value;\n};\n\
               #else\nconst vec4 value = vec4(1.0);\n#endif\n\
               void main() {\n    fragColor = value;\n}\n"
        .to_owned();
    let (root, defaults) = packs(vsh(false), fsh, Some(r#"{"Other": "1"}"#));
    let mut atlas = super::super::HudAtlas::boot();
    let refused = load_shaders(&mut atlas, &root, &defaults).unwrap_err();
    assert_eq!(refused.pipeline, "minecraft:pipeline/text_see_through");
}
