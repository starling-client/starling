//! `gbuffers_water` blends every buffer it writes with the translucent default (spec 3.2:
//! `SRC_ALPHA ONE_MINUS_SRC_ALPHA ONE ONE_MINUS_SRC_ALPHA`) unless `blend.gbuffers_water` or a
//! `blend.gbuffers_water.<buffer>` of the pack says otherwise; a buffer's own `off` overwrites that
//! buffer alone (#433: a pack keeps an encoded normal exact under glass with
//! `blend.gbuffers_water.colortex2=off`). Original fixture, no pack files.
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::hud::HudBatch;
use wgpu::{TextureFormat as T, util::DeviceExt};

/// One `gbuffers_water` that draws water (alpha 1) and then glass (alpha 0.25) over it, colour
/// into colortex0 and an encoded normal into colortex2, both RGBA16F; `final` shows colortex0 on
/// the left half and colortex2 on the right half.
fn files(properties: &str) -> Vec<(String, String)> {
    vec![
        ("shaders.properties".into(), properties.into()),
        (
            "gbuffers_water.vsh".into(),
            "#version 120\nvarying float glass;\n\
             void main() { gl_Position = vec4(gl_Vertex.xy, 0.0, 1.0); glass = gl_Vertex.z; }\n"
                .into(),
        ),
        (
            "gbuffers_water.fsh".into(),
            "#version 120\n/* RENDERTARGETS: 0,2 */\n\
             /*\nconst int colortex0Format = RGBA16F;\nconst int colortex2Format = RGBA16F;\n*/\n\
             varying float glass;\n\
             void main() {\n\
             \x20   if (glass > 0.5) {\n\
             \x20       gl_FragData[0] = vec4(0.75, 0.125, 0.875, 0.25);\n\
             \x20       gl_FragData[1] = vec4(0.5625, 0.75, 0.3125, 0.25);\n\
             \x20   } else {\n\
             \x20       gl_FragData[0] = vec4(0.25, 0.5, 0.0625, 1.0);\n\
             \x20       gl_FragData[1] = vec4(0.125, 0.25, 0.875, 1.0);\n\
             \x20   }\n}\n"
                .into(),
        ),
        ("final.vsh".into(), VERTEX.into()),
        (
            "final.fsh".into(),
            "#version 120\nuniform sampler2D colortex0;\nuniform sampler2D colortex2;\nvarying vec2 uv;\n\
             void main() {\n\
             \x20   vec3 c = uv.x < 0.5 ? texture2D(colortex0, uv).rgb : texture2D(colortex2, uv).rgb;\n\
             \x20   gl_FragColor = vec4(c, 1.0);\n}\n"
                .into(),
        ),
    ]
}

/// The picture of one frame whose translucent terrain is the water quad, then the glass quad.
fn picture(gpu: &Gpu, properties: &str) -> Vec<[u8; 4]> {
    // Water at z = 0, glass at z = 1: the vertex shader passes z on as `glass` and draws both
    // at the same depth.
    let mut positions = Vec::new();
    for z in [0.0f32, 1.0] {
        positions.extend([
            [-1.0, -1.0, z],
            [1.0, -1.0, z],
            [1.0, 1.0, z],
            [-1.0, -1.0, z],
            [1.0, 1.0, z],
            [-1.0, 1.0, z],
        ]);
    }
    let vertices = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Original water and glass quads"),
            contents: bytemuck::cast_slice(&positions),
            usage: wgpu::BufferUsages::VERTEX,
        });
    const ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0=>Float32x3];
    let layouts = [Some(wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    })];
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let mut post = gpu.load(
        files(properties),
        World {
            atlas: None,
            material_maps: None,
            lightmap: color.clone(),
            terrain: Some(Geometry {
                buffers: &layouts,
                table: None,
                depth_write: true,
                atlas: None,
                material_maps: false,
                ids: None,
                cull: false,
                pages: false,
            }),
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        sunrise_shaders::GameInputs {
            terrain: Some(sunrise_shaders::GbuffersInputs {
                declarations: "layout(location=0)in vec3 fixture_position;\n".into(),
                body: "hz_Vertex=vec4(fixture_position,1.0);\n".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    assert!(post.has_water(), "{:?}", post.notes);
    let hud = HudBatch::scratch(SIZE);
    gpu.render(
        &mut post,
        &color,
        &depth,
        &fixture_frame(&hud),
        &mut |rp, list, _, _| {
            if list == TerrainList::Translucent {
                rp.set_vertex_buffer(0, vertices.slice(..));
                rp.draw(0..12, 0..1);
            }
        },
    )
}

/// Every pixel of the left half is `left`, every pixel of the right half `right`.
fn halves(pixels: &[[u8; 4]], left: [u8; 4], right: [u8; 4], case: &str) {
    let width = SIZE[0] as usize;
    assert_eq!(pixels.len(), width * SIZE[1] as usize);
    for (i, &p) in pixels.iter().enumerate() {
        let want = if i % width < width / 2 { left } else { right };
        assert_eq!(p, want, "{case}: pixel {i}");
    }
}

#[test]
#[ignore = "daily"]
fn water_blends_each_buffer_unless_that_buffer_is_off() {
    let _gpu = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("a Vulkan adapter for the water blend fixture");
    // Rgba8Unorm bytes round(c * 255) of each expected colour. Glass over water with the default
    // factors is src * 0.25 + dst * 0.75 per channel; colortex0: 0.75*0.25+0.25*0.75 = 0.375,
    // 0.125*0.25+0.5*0.75 = 0.40625, 0.875*0.25+0.0625*0.75 = 0.265625.
    let color_blended = [96, 104, 68, 255];
    // colortex2: 0.5625*0.25+0.125*0.75 = 0.234375, 0.75*0.25+0.25*0.75 = 0.375,
    // 0.3125*0.25+0.875*0.75 = 0.734375.
    let normal_blended = [60, 96, 187, 255];
    // What the glass wrote, overwriting: (0.75, 0.125, 0.875) and (0.5625, 0.75, 0.3125).
    let color_glass = [191, 32, 223, 255];
    let normal_glass = [143, 191, 80, 255];
    // No `blend` key: both buffers blend (spec 3.2), the encoded normal included.
    halves(&picture(&gpu, ""), color_blended, normal_blended, "default");
    // The normal buffer's own `off`: that buffer takes what the glass wrote, colortex0 still blends.
    halves(
        &picture(&gpu, "blend.gbuffers_water.colortex2=off\n"),
        color_blended,
        normal_glass,
        "colortex2 off",
    );
    // The program's `off`: every buffer takes what the glass wrote.
    halves(
        &picture(&gpu, "blend.gbuffers_water=off\n"),
        color_glass,
        normal_glass,
        "program off",
    );
    // The normal buffer's own factors `ONE ONE_MINUS_SRC_ALPHA`: src + dst * 0.75 there,
    // 0.5625+0.125*0.75 = 0.65625, 0.75+0.25*0.75 = 0.9375, 0.3125+0.875*0.75 = 0.96875;
    // colortex0 still takes the default.
    halves(
        &picture(
            &gpu,
            "blend.gbuffers_water.colortex2=ONE ONE_MINUS_SRC_ALPHA ONE ONE_MINUS_SRC_ALPHA\n",
        ),
        color_blended,
        [167, 239, 247, 255],
        "colortex2 own factors",
    );
}
