//! An image whose qualifier is 16-bit normalized (`layout(rgba16)`, `layout(rgba16_snorm)`) and
//! whose buffer or `image.*` is the 16-bit float format of its channels (`RGBA16F`) binds that
//! format with a note, keeping the floats a program stores (#548, decision D-3); one declared in
//! its buffer's own format gets no note, and an image of another width (`layout(rgba8)` on
//! `RGBA16F`) still refuses its program. Original fixtures, no pack files.
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::hud::HudBatch;
use wgpu::TextureFormat as T;

/// The screen pixels and the notes of the pack `files` after one frame, on a device with the
/// adapter's own format features, as the game requests: the images here are declared without
/// `readonly` or `writeonly`, as the issue's pack declares its own, and an `RGBA16F` image read
/// and written in one program needs them.
fn run(files: Vec<(String, String)>) -> Option<(Vec<[u8; 4]>, Vec<String>)> {
    let _gpu = crate::gfx::test_gpu::lock();
    let gpu = Gpu::with_features(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)?;
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let mut post = gpu.load(
        files,
        World {
            atlas: None,
            material_maps: None,
            lightmap: color.clone(),
            terrain: None,
            block_ids: &[],
            sprite_rects: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        sunrise_shaders::GameInputs::default(),
    );
    let hud = HudBatch::scratch(SIZE);
    let mut frame = fixture_frame(&hud);
    frame.title = false;
    let pixels = gpu.render(&mut post, &color, &depth, &frame, &mut |_, _, _, _| {});
    Some((pixels, post.notes.clone()))
}

/// A compute `composite` that adds `colorimg8` and `imgA`'s values at texel (1, 1) to
/// themselves (a load before the store: its loads read a copy taken before the dispatch), both
/// declared `layout(<qualifier>)`, and a `final` that shows `colortex8`'s red and green less 1,
/// `imgA`'s red less 3.75 and its green plus 0.375 as alpha.
fn compute_pack(qualifier: &str) -> Vec<(String, String)> {
    vec![
        (
            "shaders.properties".into(),
            "image.imgA = texA RGBA RGBA16F HALF_FLOAT false false 4 4\n".into(),
        ),
        (
            "composite.csh".into(),
            format!(
                "#version 430\nlayout(local_size_x = 1) in;\nconst ivec3 workGroups = ivec3(1, 1, 1);\n\
                 layout({qualifier}) uniform image2D colorimg8;\n\
                 layout({qualifier}) uniform image2D imgA;\n\
                 void main() {{\n    ivec2 p = ivec2(1, 1);\n\
                 imageStore(colorimg8, p, imageLoad(colorimg8, p) + vec4(2.0, 1.25, 5.75, 1.0));\n\
                 imageStore(imgA, p, imageLoad(imgA, p) + vec4(4.5, 0.5, 0.5, 1.0));\n}}\n"
            ),
        ),
        ("final.vsh".into(), VERTEX.into()),
        (
            "final.fsh".into(),
            "#version 130\n/*\nconst int colortex8Format = RGBA16F;\n*/\n\
             uniform sampler2D colortex8;\nuniform sampler2D texA;\nvarying vec2 uv;\n\
             void main() {\n    vec4 c = texelFetch(colortex8, ivec2(1, 1), 0);\n\
             vec4 a = texelFetch(texA, ivec2(1, 1), 0);\n\
             gl_FragColor = vec4(c.r - 1.0, c.g - 1.0, a.r - 3.75, a.g + 0.375);\n}\n"
                .into(),
        ),
    ]
}

/// The issue's case: `layout(rgba16)` on `colortex8Format = RGBA16F`, and on an `image.*` of
/// `RGBA16F`, accessed as the texture's half floats (decision D-3). Stored unclamped: (2.0, 1.25)
/// and 4.5 less (1, 1, 3.75) are (1.0, 0.25, 0.75), which a clamped store or a refused program
/// (0 in both) shows as black. The 0.5 stored in `imgA`'s green reads back as 0.5 (alpha
/// 0.875); stored as the unorm16 bits of 0.5 (0x8000, a half float's -0.0) it would read 0
/// (alpha 0.375), and a refused program shows that too.
#[test]
#[ignore = "daily"]
fn an_rgba16_image_of_a_float_buffer_keeps_the_floats_stored() {
    let Some((pixels, notes)) = run(compute_pack("rgba16")) else {
        return;
    };
    assert!(
        pixels.iter().all(|&p| p == [255, 64, 191, 223]),
        "{:?}, notes {notes:?}",
        pixels[0]
    );
    // Each rebound image is noted, naming the program, the image, the qualifier and the format
    // it binds.
    for image in ["`colorimg8`", "`imgA`"] {
        assert!(
            notes.iter().any(|n| n.starts_with("composite:")
                && n.contains(image)
                && n.contains("rgba16")
                && n.contains("Rgba16Float")),
            "{image}: {notes:?}"
        );
    }
}

/// The same pack with `layout(rgba16f)`, the buffers' own format: the same values, and no note
/// about either image.
#[test]
#[ignore = "daily"]
fn an_rgba16f_image_of_a_float_buffer_binds_without_a_note() {
    let Some((pixels, notes)) = run(compute_pack("rgba16f")) else {
        return;
    };
    assert!(
        pixels.iter().all(|&p| p == [255, 64, 191, 223]),
        "{:?}, notes {notes:?}",
        pixels[0]
    );
    assert!(
        !notes
            .iter()
            .any(|n| n.contains("`colorimg8`") || n.contains("`imgA`")),
        "{notes:?}"
    );
}

/// `layout(rgba8)` on the same `RGBA16F` buffer and image: another texel size, so the program
/// is refused, naming the first image it refuses, and nothing is stored.
#[test]
#[ignore = "daily"]
fn an_rgba8_image_of_a_float_buffer_still_refuses_its_program() {
    let Some((pixels, notes)) = run(compute_pack("rgba8")) else {
        return;
    };
    assert!(
        pixels.iter().all(|&p| p == [0, 0, 0, 96]),
        "{:?}, notes {notes:?}",
        pixels[0]
    );
    assert!(
        notes
            .iter()
            .any(|n| n.starts_with("composite:")
                && (n.contains("`colorimg8`") || n.contains("`imgA`"))),
        "{notes:?}"
    );
}

/// A quad program's `layout(rgba16_snorm)` image of `colortex9Format = RGBA16F`, passed to a
/// function of the pack's: stored unclamped, (-0.75, 3.0, 1.5) shows through `final` (adding
/// 1, subtracting 2, subtracting 0.75) as (0.25, 1.0, 0.75), where a clamp to [-1, 1] shows
/// (0.25, 0, 0.25) and a refused program shows [255, 0, 0, 255].
#[test]
#[ignore = "daily"]
fn an_rgba16_snorm_image_of_a_float_buffer_keeps_the_floats_a_quad_program_stores() {
    let files = vec![
        ("composite.vsh".into(), VERTEX.into()),
        (
            "composite.fsh".into(),
            "#version 430 compatibility\n/* RENDERTARGETS: 1 */\n\
             /*\nconst int colortex9Format = RGBA16F;\n*/\n\
             layout(rgba16_snorm) uniform image2D colorimg9;\n\
             void put(image2D target, ivec2 p) { imageStore(target, p, vec4(-0.75, 3.0, 1.5, 1.0)); }\n\
             void main() {\n    put(colorimg9, ivec2(gl_FragCoord.xy));\n\
             gl_FragData[0] = vec4(1.0);\n}\n"
                .into(),
        ),
        ("final.vsh".into(), VERTEX.into()),
        (
            "final.fsh".into(),
            "#version 130\nuniform sampler2D colortex9;\nvarying vec2 uv;\n\
             void main() {\n    vec4 c = texelFetch(colortex9, ivec2(gl_FragCoord.xy), 0);\n\
             gl_FragColor = vec4(c.r + 1.0, c.g - 2.0, c.b - 0.75, 1.0);\n}\n"
                .into(),
        ),
    ];
    let Some((pixels, notes)) = run(files) else {
        return;
    };
    assert!(
        pixels.iter().all(|&p| p == [64, 255, 191, 255]),
        "{:?}, notes {notes:?}",
        pixels[0]
    );
}
