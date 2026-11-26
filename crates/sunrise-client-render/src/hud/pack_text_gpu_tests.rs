//! A pack's GUI text through its pipeline: an expression uniform (`shader_modifiers/core/text.json`)
//! evaluated from the file reaches the GUI text's block through `HudRenderer::prepare`, and a
//! sampler only the fragment stage reads draws the atlas.
use super::*;
use crate::hud::coverage_tests::{Gpu, quad};
use crate::hud::{Blend, GuiScale, rgba};

/// GUI pixels 0 to 4 across and 0 to 2 down fill the target.
const VSH: &str = "#version 330\nlayout(location = 0) in vec3 Position;\n\
    void main() {\n    gl_Position = vec4(Position.x / 2.0 - 1.0, 1.0 - Position.y, 0.0, 1.0);\n}\n";

/// The pack's GUI text program of `fsh` (with [`VSH`]) and the file `modifiers`, as the atlas
/// loads it, set on `gpu`'s HUD.
fn set_pack_text(gpu: &mut Gpu, fsh: &str, modifiers: &str) {
    let pack = super::text_tests::source(&[
        ("assets/minecraft/shaders/core/text.vsh", VSH.to_owned()),
        ("assets/minecraft/shaders/core/text.fsh", fsh.to_owned()),
        (
            "assets/minecraft/polytone/shader_modifiers/core/text.json",
            modifiers.to_owned(),
        ),
    ]);
    let mut atlas = super::super::HudAtlas::boot();
    load_shaders(&mut atlas, &pack, &ResourceSource::default()).unwrap();
    let text = &atlas.text_shaders;
    let gui = text.gui.as_deref().expect("the GUI text's program");
    gpu.hud
        .set_text_shader(&gpu.device, Some((gui, &text.expressions.gui[..])))
        .unwrap();
    assert!(gpu.hud.pack_text.is_some());
}

/// Two GUI text quads over the 4 by 2 target, prepared as a frame does: its 8 pixels.
fn drawn(gpu: &mut Gpu) -> Vec<[u8; 4]> {
    let mut batch = HudBatch::new(
        [4, 2],
        GuiScale {
            requested: 1,
            unicode: false,
        },
    );
    for x in [0.0, 2.0] {
        quad(&mut batch, Blend::Alpha, rgba(255, 255, 255, 255), x, 0.25);
    }
    for v in &mut batch.vertices {
        v.text = 1;
    }
    let mut uploads = crate::gfx::upload::Uploads::new(&gpu.device);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    gpu.hud
        .prepare(&gpu.device, &mut uploads, &mut encoder, &batch, [4, 2]);
    uploads.finish(&encoder);
    gpu.queue.submit([encoder.finish()]);
    let (pixels, _) = gpu.draw(&batch, 0.0, 0.0, false);
    pixels.as_chunks::<4>().0.to_vec()
}

/// The GUI text reads `Tint` from the text shader's file (`0.1 + 0.1`, 51 in a unorm byte) in a
/// block after vanilla's four, beside a full green that shows it drew: 0 until the frame's
/// context evaluates it.
#[test]
#[ignore = "daily"]
fn a_pack_gui_text_reads_its_expression_uniform() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let fsh = "#version 330\nlayout(std140) uniform Tint {\n    float value;\n};\n\
               layout(location = 0) out vec4 fragColor;\n\
               void main() {\n    fragColor = vec4(value, 1.0, 0.0, 1.0);\n}\n";
    set_pack_text(&mut gpu, fsh, r#"{"Tint": "0.1 + 0.1"}"#);
    assert_eq!(drawn(&mut gpu), [[0, 255, 0, 255]; 8]);
    let blocks = gpu
        .hud
        .text_expression_blocks()
        .expect("the pack's GUI text");
    blocks.evaluate(&Default::default());
    assert_eq!(drawn(&mut gpu), [[51, 255, 0, 255]; 8]);
}

/// The GUI text reads Polytone's `PolyGlobals` from the built-in `polytone_globals.glsl` as the
/// game gives them (the player's block height, 64, in blue) beside `Tint` (51 in red): zeros when
/// none are given, and zeros for both once a frame where no world runs zeroes them.
#[test]
#[ignore = "daily"]
fn a_pack_gui_text_reads_poly_globals() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let fsh = "#version 330\n#moj_import <polytone_globals.glsl>\n\
               layout(std140) uniform Tint {\n    float value;\n};\n\
               layout(location = 0) out vec4 fragColor;\n\
               void main() {\n    \
               fragColor = vec4(value, 1.0, float(PolyPlayerBlockPos.y) / 255.0, 1.0);\n}\n";
    set_pack_text(&mut gpu, fsh, r#"{"Tint": "0.1 + 0.1"}"#);
    let globals = sunrise_polytone::globals::Globals {
        player: [3.5, 64.2, 7.0],
        ..Default::default()
    }
    .std140();
    let given = |gpu: &mut Gpu, globals| {
        let blocks = gpu
            .hud
            .text_expression_blocks()
            .expect("the pack's GUI text");
        assert!(blocks.reads_globals());
        blocks.evaluate(&Default::default());
        blocks.set_globals(globals);
    };
    given(&mut gpu, Some(&globals));
    assert_eq!(drawn(&mut gpu), [[51, 255, 64, 255]; 8]);
    // No globals given: the block reads zeros.
    given(&mut gpu, None);
    assert_eq!(drawn(&mut gpu), [[51, 255, 0, 255]; 8]);
    // A title or loading frame (`Renderer::zero_text_blocks`).
    given(&mut gpu, Some(&globals));
    gpu.hud.text_expression_blocks().unwrap().zero();
    assert_eq!(drawn(&mut gpu), [[0, 255, 0, 255]; 8]);
}

/// A GUI text program whose fragment stage alone samples the atlas (`Sampler0`) is bound to it
/// there and draws its texel: the atlas's first, opaque white.
#[test]
#[ignore = "daily"]
fn a_pack_gui_text_samples_the_atlas_in_its_fragment_stage() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let fsh = "#version 330\nuniform sampler2D Sampler0;\nlayout(location = 0) out vec4 fragColor;\n\
               void main() {\n    fragColor = texture(Sampler0, vec2(0.25, 0.5));\n}\n";
    set_pack_text(&mut gpu, fsh, "{}");
    assert_eq!(drawn(&mut gpu), [[255, 255, 255, 255]; 8]);
}

/// An older vanilla's `DynamicTransforms`, with a member past 26.3's (`More`), in a GUI text
/// shader: vanilla's builder takes it (it checks a descriptor's name, type and dimension, not a
/// block's size), and so does Sunrise, binding the block at the shader's size with the bytes past
/// vanilla's reading zero: `More` is 0 (red), `ColorModulator` vanilla's white (its alpha 51 in
/// blue at 0.2), beside a full green that shows it drew.
#[test]
#[ignore = "daily"]
fn a_gui_text_shader_with_an_older_larger_block_draws() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let fsh = "#version 330\nlayout(std140) uniform DynamicTransforms {\n    mat4 ModelViewMat;\n    \
               mat4 TextureMat;\n    vec4 ColorModulator;\n    vec3 ModelOffset;\n    vec4 More;\n};\n\
               layout(location = 0) out vec4 fragColor;\nvoid main() {\n    \
               fragColor = vec4(More.x, 1.0, ColorModulator.a * 0.2, 1.0);\n}\n";
    set_pack_text(&mut gpu, fsh, "{}");
    assert_eq!(drawn(&mut gpu), [[0, 255, 51, 255]; 8]);
}

/// The device refusing the GUI text's pipeline fails the load, named by vanilla's `GUI_TEXT`,
/// with the HUD's coverage on (the test HUD's) and whether or not the program writes the colour
/// at location 0, which coverage wraps. Every pack shader the device would refuse here is refused
/// earlier, by vanilla's checks, or made to fit, so the program is built by hand: its fragment
/// stage samples binding 4 (`Sampler0`'s) as a 3D texture where the pipeline binds a 2D one. The
/// same program sampling it 2D is taken.
#[test]
#[ignore = "daily"]
fn a_gui_text_pipeline_the_device_refuses_fails_the_load() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(mut gpu) = Gpu::new() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let program = |dimension: &str, at: &str, location: u32| {
        crate::core_shader::validated(
            naga::front::wgsl::parse_str(
                "@vertex fn main(@location(0) p: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4(p, 0.0, 1.0); }",
            )
            .unwrap(),
            naga::front::wgsl::parse_str(&format!(
                "@group(0) @binding(4) var t: texture_{dimension}<f32>;\n\
                 @group(0) @binding(5) var s: sampler;\n\
                 @fragment fn main() -> @location({location}) vec4<f32> {{ return textureSample(t, s, {at}); }}"
            ))
            .unwrap(),
            Vec::new(),
        )
    };
    let taken = program("2d", "vec2(0.5)", 0);
    gpu.hud
        .set_text_shader(&gpu.device, Some((&taken, &[])))
        .unwrap();
    assert!(gpu.hud.pack_text.is_some());
    for location in [0, 1] {
        let refused = program("3d", "vec3(0.5)", location);
        let error = gpu
            .hud
            .set_text_shader(&gpu.device, Some((&refused, &[])))
            .unwrap_err();
        assert_eq!(
            error.split_once(": ").map(|(p, _)| p),
            Some("minecraft:pipeline/gui_text"),
            "colour at location {location}"
        );
    }
}

/// The pack's GUI text program of `vsh` and `fsh`, as the atlas loads it.
fn gui_program(vsh: &str, fsh: &str) -> std::sync::Arc<sunrise_glsl::Translated> {
    let pack = super::text_tests::source(&[
        ("assets/minecraft/shaders/core/text.vsh", vsh.to_owned()),
        ("assets/minecraft/shaders/core/text.fsh", fsh.to_owned()),
    ]);
    let mut atlas = super::super::HudAtlas::boot();
    load_shaders(&mut atlas, &pack, &ResourceSource::default()).unwrap();
    atlas.text_shaders.gui.expect("the GUI text's program")
}

/// A GUI text shader that writes no colour at location 0, or one the device doesn't take there,
/// draws natively with the HUD's coverage on and off alike; one whose varying the fragment stage reads `noperspective` or `centroid`
/// (vanilla's check compares only flat-ness) is drawn.
#[test]
#[ignore = "daily"]
fn a_gui_text_shader_is_taken_or_left_the_same_in_every_coverage_mode() {
    let _gpu = crate::gfx::test_gpu::lock();
    let Some(gpu) = Gpu::new() else {
        eprintln!("skipped: no adapter");
        return;
    };
    let atlas = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = atlas.create_view(&Default::default());
    let made = |program: &sunrise_glsl::Translated, coverage| {
        PackText::new(
            &gpu.device,
            crate::gfx::hud_coverage::HUD_FORMAT,
            &view,
            (program, &[]),
            coverage,
        )
        .unwrap()
        .is_some()
    };
    let none = gui_program(
        VSH,
        "#version 330\nlayout(location = 1) out vec4 fragColor;\nvoid main() {\n    fragColor = vec4(1.0);\n}\n",
    );
    assert_eq!([made(&none, true), made(&none, false)], [false, false]);
    // A colour the device doesn't take into the HUD's 4-channel target (`vec3`, `uvec4`), which
    // vanilla's builder doesn't check: native in every mode, the pack kept.
    for output in [
        "vec3 fragColor;\nvoid main() {\n    fragColor = vec3(1.0);",
        "uvec4 fragColor;\nvoid main() {\n    fragColor = uvec4(1u);",
    ] {
        let program = gui_program(
            VSH,
            &format!("#version 330\nlayout(location = 0) out {output}\n}}\n"),
        );
        assert_eq!(
            [made(&program, true), made(&program, false)],
            [false, false],
            "{output}"
        );
    }
    let vsh = "#version 330\nlayout(location = 0) in vec3 Position;\nlayout(location = 0) out vec2 tag;\n\
               void main() {\n    gl_Position = vec4(Position, 1.0);\n    tag = vec2(1.0);\n}\n";
    for qualifier in ["noperspective", "centroid"] {
        let fsh = format!(
            "#version 330\nlayout(location = 0) {qualifier} in vec2 tag;\n\
             layout(location = 0) out vec4 fragColor;\nvoid main() {{\n    fragColor = vec4(tag, 0.0, 1.0);\n}}\n"
        );
        let program = gui_program(vsh, &fsh);
        assert_eq!(
            [made(&program, true), made(&program, false)],
            [true, true],
            "{qualifier}"
        );
    }
}
