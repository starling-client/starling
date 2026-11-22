//! A pack's world and see-through text reading expression uniforms (`shader_modifiers`): each
//! pipeline's values, from the text shader's file, reach its own blocks through
//! [`CoreText::prepare`], after the pipeline's own.
use super::*;

/// The quad covers the target wherever it is placed; the world text's reads `Tint` in red and
/// the see-through text's (`IS_SEE_THROUGH`) `Fade`, beside a full green that shows it drew.
const VSH: &str = "#version 330\nlayout(location = 0) in vec3 Position;\n\
    void main() {\n    gl_Position = vec4(Position, 1.0);\n}\n";
const FSH: &str = "#version 330\n\
    #ifdef IS_SEE_THROUGH\nlayout(std140) uniform Fade {\n    float value;\n};\n\
    #else\nlayout(std140) uniform Tint {\n    float value;\n};\n#endif\n\
    layout(location = 0) out vec4 fragColor;\n\
    void main() {\n    fragColor = vec4(value, 1.0, 0.0, 1.0);\n}\n";

/// The text shader's file: values a unorm byte holds exactly (0.2 is 51, 0.6 is 153), as
/// expressions the packs' context evaluates.
const MODIFIERS: &str = r#"{"Tint": "0.1 + 0.1", "Fade": "0.3 * 2"}"#;

/// A pack with the text shader `vsh` and `fsh` and its `shader_modifiers` file `modifiers`, its
/// text programs loaded as the atlas loads them (`hud::pack_text::load_shaders`).
fn text_shaders(vsh: &str, fsh: &str, modifiers: &str) -> crate::gfx::core_text::TextShaders {
    use sunrise_assets::resource_source::ResourceSource;
    let pack = crate::hud::pack_text::text_tests::source(&[
        ("assets/minecraft/shaders/core/text.vsh", vsh.to_owned()),
        ("assets/minecraft/shaders/core/text.fsh", fsh.to_owned()),
        (
            "assets/minecraft/polytone/shader_modifiers/core/text.json",
            modifiers.to_owned(),
        ),
    ]);
    let mut atlas = crate::hud::HudAtlas::boot();
    atlas.load_pack_shaders(&pack, &ResourceSource::default());
    atlas.text_shaders
}

/// The world text pipelines of `shaders`, over a 1 by 1 glyph texture of `glyphs` and a black
/// lightmap.
fn core_text(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shaders: &crate::gfx::core_text::TextShaders,
    glyphs: [u8; 4],
) -> Option<CoreText> {
    let atlas = super::tests::texture(device, queue, glyphs);
    let lightmap = super::tests::texture(device, queue, [0; 4]);
    let sampler = device.create_sampler(&Default::default());
    let expressions = &shaders.expressions;
    CoreText::new(
        device,
        wgpu::TextureFormat::Rgba8Unorm,
        (shaders.world.as_deref()).map(|p| (p, &expressions.world[..])),
        (shaders.see_through.as_deref()).map(|p| (p, &expressions.see_through[..])),
        &atlas,
        (&lightmap, &sampler),
    )
}

/// The world text reads `Tint` and the see-through text `Fade`, both from the text shader's
/// file, each in its own pipeline's block after vanilla's four, holding its expression's value
/// once evaluated.
#[test]
#[ignore = "daily"]
fn a_pack_world_and_see_through_text_read_their_expression_uniforms() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let shaders = text_shaders(VSH, FSH, MODIFIERS);
    let mut text = core_text(&device, &queue, &shaders, [0; 4]).expect("the pipelines");
    let draw =
        |text: &CoreText, see_through| super::tests::draw(&device, &queue, text, see_through, 0);
    // Before the frame's context evaluates them, the blocks hold 0.
    assert_eq!(draw(&text, false), [0, 255, 0, 255]);
    for blocks in text.expression_blocks() {
        blocks.evaluate(&Default::default());
    }
    assert_eq!(draw(&text, false), [51, 255, 0, 255]);
    assert_eq!(draw(&text, true), [153, 255, 0, 255]);
}

/// Polytone's `PolyGlobals`, imported from the built-in `polytone_globals.glsl` beside the
/// expression uniforms: the world and the see-through text each read the block's bytes as the
/// game gives them (the player's block height, 64, in blue), zeros when none are given, and zeros
/// for the block and the uniforms alike once a frame where no world runs zeroes them.
#[test]
#[ignore = "daily"]
fn a_pack_world_and_see_through_text_read_poly_globals() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let fsh = FSH
        .replace(
            "#version 330\n",
            "#version 330\n#moj_import <polytone_globals.glsl>\n",
        )
        .replace("0.0, 1.0);", "float(PolyPlayerBlockPos.y) / 255.0, 1.0);");
    let shaders = text_shaders(VSH, &fsh, MODIFIERS);
    let mut text = core_text(&device, &queue, &shaders, [0; 4]).expect("the pipelines");
    let reads: Vec<bool> = text
        .expression_blocks()
        .map(|b| b.reads_globals())
        .collect();
    assert_eq!(reads, [true, true]);
    let globals = sunrise_polytone::globals::Globals {
        player: [3.5, 64.2, 7.0],
        ..Default::default()
    }
    .std140();
    let given = |text: &mut CoreText, globals| {
        for blocks in text.expression_blocks() {
            blocks.evaluate(&Default::default());
            blocks.set_globals(globals);
        }
    };
    let draw =
        |text: &CoreText, see_through| super::tests::draw(&device, &queue, text, see_through, 0);
    given(&mut text, Some(&globals));
    assert_eq!(draw(&text, false), [51, 255, 64, 255]);
    assert_eq!(draw(&text, true), [153, 255, 64, 255]);
    // No globals given: the block reads zeros.
    given(&mut text, None);
    assert_eq!(draw(&text, false), [51, 255, 0, 255]);
    assert_eq!(draw(&text, true), [153, 255, 0, 255]);
    // A title or loading frame (`Renderer::zero_text_blocks`).
    given(&mut text, Some(&globals));
    for blocks in text.expression_blocks() {
        blocks.zero();
    }
    assert_eq!(draw(&text, false), [0, 255, 0, 255]);
    assert_eq!(draw(&text, true), [0, 255, 0, 255]);
}

/// Each world text pipeline is refused on its own: a see-through program reading 13 expression
/// blocks in its fragment stage, past the device's 12 uniform blocks a stage
/// (`max_uniform_buffers_per_shader_stage`'s default), leaves the see-through text native while
/// the world text, reading one, keeps the pack's pipeline.
#[test]
#[ignore = "daily"]
fn a_world_text_pipeline_over_the_block_limit_is_refused_alone() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
        eprintln!("skipped: no adapter");
        return;
    };
    assert_eq!(device.limits().max_uniform_buffers_per_shader_stage, 12);
    let many: Vec<String> = (0..13).map(|i| format!("B{i}")).collect();
    let blocks: String = many
        .iter()
        .map(|name| format!("layout(std140) uniform {name} {{\n    float v{name};\n}};\n"))
        .collect();
    let sum: Vec<String> = many.iter().map(|name| format!("v{name}")).collect();
    let fsh = format!(
        "#version 330\n#ifdef IS_SEE_THROUGH\n{blocks}#define VALUE ({})\n#else\n\
         layout(std140) uniform Tint {{\n    float value;\n}};\n#define VALUE value\n#endif\n\
         layout(location = 0) out vec4 fragColor;\n\
         void main() {{\n    fragColor = vec4(VALUE, 0.0, 0.0, 1.0);\n}}\n",
        sum.join(" + ")
    );
    let file: String = many
        .iter()
        .map(|name| format!("\"{name}\": \"0\", "))
        .collect::<String>()
        + "\"Tint\": \"0.2\"";
    let shaders = text_shaders(VSH, &fsh, &format!("{{{file}}}"));
    let expressions = &shaders.expressions;
    assert_eq!(
        (expressions.world.len(), expressions.see_through.len()),
        (1, 13)
    );
    let text = core_text(&device, &queue, &shaders, [0; 4]).expect("the world text's pipeline");
    assert!(text.polygon_offset.is_some());
    assert!(text.see_through.is_none());
}

/// The world text's samplers are bound like its blocks: a program whose fragment stage samples
/// the glyphs (`Sampler0`, 4 and 5) and nothing else gets those two, fragment only, and no
/// lightmap (`Sampler2`, 8 and 9); the pipeline built so draws the glyph texture's texel.
#[test]
#[ignore = "daily"]
fn a_world_text_sampler_is_bound_to_the_stage_that_reads_it() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let fsh = "#version 330\nuniform sampler2D Sampler0;\nuniform sampler2D Sampler2;\n\
               layout(location = 0) out vec4 fragColor;\n\
               void main() {\n    fragColor = texture(Sampler0, vec2(0.5));\n}\n";
    let shaders = text_shaders(VSH, fsh, "{}");
    let program = shaders.world.as_deref().expect("the world text's program");
    let view = super::tests::texture(&device, &queue, [0; 4]);
    let sampler = device.create_sampler(&Default::default());
    let (layout, group) = super::super::expression_blocks::sampler_bindings(
        program,
        &[(4, &view, &sampler), (8, &view, &sampler)],
    );
    let fragment = wgpu::ShaderStages::FRAGMENT;
    let layout: Vec<_> = layout.iter().map(|e| (e.binding, e.visibility)).collect();
    assert_eq!(layout, [(4, fragment), (5, fragment)]);
    assert_eq!(group.iter().map(|e| e.binding).collect::<Vec<_>>(), [4, 5]);
    let text = core_text(&device, &queue, &shaders, [51, 102, 153, 255]).expect("the pipelines");
    assert_eq!(
        super::tests::draw(&device, &queue, &text, false, 0),
        [51, 102, 153, 255]
    );
}
