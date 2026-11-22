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
        .set_text_shader(&gpu.device, Some((gui, &text.expressions.gui[..])));
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
