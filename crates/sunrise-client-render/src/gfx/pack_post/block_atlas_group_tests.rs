//! The block program's bind groups for End portal faces and for a block entity's native terrain
//! quads (`Pass::portal_groups`, `Pass::block_atlas_groups`) are dropped with the plain ones when
//! a resize makes the views they name again (#5090); original fixture, no pack assets.
use super::depth_test::{Gpu, SIZE, VERTEX, fixture_frame};
use super::*;
use crate::gfx::pack_draw::EntitySpan;
use crate::gfx::{TERRAIN_MODEL_SOLID_OVERLAY, Vertex};
use crate::hud::HudBatch;
use sunrise_shaders::render_stage;
use wgpu::TextureFormat as T;

/// A whole-screen quad of overlay `layer`, UVs over its sprite.
fn quad(layer: u32) -> [Vertex; 4] {
    [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|[x, y]| Vertex {
        pos: [x, y, 0.5],
        uv: [(x + 1.0) / 2.0, (y + 1.0) / 2.0],
        layer,
        color: u32::MAX,
        ..bytemuck::Zeroable::zeroed()
    })
}

#[test]
#[ignore = "daily"]
fn a_resize_drops_the_portal_and_block_atlas_groups() {
    let _gpu = crate::gfx::test_gpu::lock();
    let gpu = Gpu::new().expect("an executed adapter is required for this control");
    let files = vec![
        ("final.vsh".into(), VERTEX.into()),
        (
            "final.fsh".into(),
            "#version 120\nuniform sampler2D colortex0; varying vec2 uv; void main(){gl_FragColor=texture2D(colortex0,uv);}"
                .into(),
        ),
        (
            "gbuffers_block.vsh".into(),
            "#version 120\nvarying vec2 t; void main(){gl_Position=gl_Vertex; t=gl_MultiTexCoord0.xy;}"
                .into(),
        ),
        (
            "gbuffers_block.fsh".into(),
            "#version 120\nuniform sampler2D gtexture; varying vec2 t; void main(){gl_FragData[0]=texture2D(gtexture,t);}"
                .into(),
        ),
    ];
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    let mut input = super::super::pack_entities::inputs();
    input.body = "hz_Vertex=vec4(hz_v_pos,1.0); hz_MultiTexCoord0=vec4(hz_v_uv,0.0,1.0); hz_Color=hz_v_color;".into();
    // The block atlas: the fixture's colour texture, one sprite layer over its top left corner.
    let rects = [0, 8];
    let mut post = gpu.load(
        files,
        World {
            atlas: Some((color.clone(), SIZE)),
            material_maps: None,
            lightmap: color.clone(),
            terrain: None,
            block_ids: &[],
            sprite_rects: &rects,
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        sunrise_shaders::GameInputs {
            entities: Some(input),
            ..Default::default()
        },
    );
    // One block entity: an End portal face (overlay 9) and a native terrain model quad.
    let vertices = [quad(9), quad(TERRAIN_MODEL_SOLID_OVERLAY)].concat();
    let spans = [EntitySpan {
        stage: render_stage("BLOCK_ENTITIES"),
        id: 3,
        ..Default::default()
    }];
    let hud = HudBatch::scratch(SIZE);
    gpu.clear(&color, &depth, 0.0);
    let mut rest = Vec::new();
    post.prepare_entities(
        &gpu.device,
        &gpu.queue,
        &vertices,
        &spans,
        [0.; 4],
        // The portal sprite's rectangle in the atlas.
        [0.0, 0.0, 0.5, 0.5],
        &mut rest,
        None,
        (&[], &[]),
    );
    assert!(rest.is_empty(), "the pack draws both");
    gpu.render(
        &mut post,
        &color,
        &depth,
        &fixture_frame(&hud),
        &mut |_, _, _, _| {},
    );
    let block = super::super::pack_entities::BLOCK;
    let cached = |post: &PackPost| {
        let pass = post.entities.as_ref().unwrap().passes[block]
            .as_ref()
            .unwrap();
        (
            pass.groups.len(),
            pass.portal_groups.len(),
            pass.block_atlas_groups.len(),
        )
    };
    let (plain, portal, atlas) = cached(&post);
    assert!(
        plain > 0 && portal > 0 && atlas > 0,
        "the frame made each kind: {:?}",
        (plain, portal, atlas)
    );
    post.resize(
        &gpu.device,
        [SIZE[0] / 2, SIZE[1] / 2],
        &std::sync::atomic::AtomicBool::new(false),
        crate::gfx::in_flight::poll_live,
    )
    .unwrap();
    assert_eq!(
        cached(&post),
        (0, 0, 0),
        "new targets: no group names the old views"
    );
}
