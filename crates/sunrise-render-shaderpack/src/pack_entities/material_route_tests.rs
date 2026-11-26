use super::*;

#[test]
fn optional_variants_split_contiguous_materials_without_guessing_from_alpha() {
    let mut vertices = Vec::new();
    for layer in [0, OIT_ENTITY, 0, TRANSLUCENT_PARTICLE_OVERLAY, 0] {
        vertices.extend(
            [Vertex {
                layer: layer | 3 << 16,
                color: 0x40808080,
                ..bytemuck::Zeroable::zeroed()
            }; 4],
        );
    }
    let spans = [
        EntitySpan {
            stage: render_stage("BLOCK_ENTITIES"),
            id: 71,
            ..Default::default()
        },
        EntitySpan {
            start: 8,
            stage: render_stage("PARTICLES"),
            ..Default::default()
        },
    ];
    let collect = |drawn| {
        material_runs(&vertices, &spans, drawn, None)
            .map(|r| (r.range, r.program))
            .collect::<Vec<_>>()
    };
    let mut legacy = [true; PROGRAMS];
    legacy[BLOCK_TRANSLUCENT] = false;
    legacy[PARTICLE_TRANSLUCENT] = false;
    assert_eq!(
        collect(legacy),
        [(0..8, Some(BLOCK)), (8..20, Some(PARTICLE))]
    );
    assert_eq!(
        collect([true; PROGRAMS]),
        [
            (0..4, Some(BLOCK)),
            (4..8, Some(BLOCK_TRANSLUCENT)),
            (8..12, Some(PARTICLE)),
            (12..16, Some(PARTICLE_TRANSLUCENT)),
            (16..20, Some(PARTICLE)),
        ]
    );
    let casters = Casters {
        blocks: true,
        ..Default::default()
    };
    assert!(casters.cast(&spans[0], BLOCK_TRANSLUCENT));
    assert_eq!(shadow_program(BLOCK_TRANSLUCENT), BLOCK);
    assert!(!casters.cast(&spans[1], PARTICLE_TRANSLUCENT));
}

/// Sign text in a block entity's run goes to the block program (spec 2.3: `gbuffers_block` draws
/// sign text), split where its depth mode changes: only POLYGON_OFFSET text takes the biased
/// pipeline.
#[test]
fn sign_text_runs_with_the_block_program_split_by_depth_mode() {
    use sunrise_render_core::entity::TEXT_NORMAL_SOLID_OVERLAY;
    let mut vertices = Vec::new();
    for layer in [
        0,
        TEXT_NORMAL_SOLID_OVERLAY,
        TEXT_POLYGON_OFFSET_SOLID_OVERLAY,
        TEXT_POLYGON_OFFSET_SOLID_OVERLAY,
        0,
        TEXT_POLYGON_OFFSET_SOLID_OVERLAY | OIT_ENTITY,
    ] {
        vertices.extend(
            [Vertex {
                layer: layer | 3 << 16,
                ..bytemuck::Zeroable::zeroed()
            }; 4],
        );
    }
    let spans = [EntitySpan {
        stage: render_stage("BLOCK_ENTITIES"),
        id: 71,
        ..Default::default()
    }];
    let runs = material_runs(&vertices, &spans, [true; PROGRAMS], None)
        .map(|r| (r.range, r.program, r.polygon_offset))
        .collect::<Vec<_>>();
    assert_eq!(
        runs,
        [
            (0..8, Some(BLOCK), false),
            (8..16, Some(BLOCK), true),
            (16..20, Some(BLOCK), false),
            (20..24, Some(BLOCK_TRANSLUCENT), true),
        ]
    );
    // Text outside a block entity's run is not the block program's.
    let entity = [EntitySpan {
        stage: render_stage("ENTITIES"),
        id: 72,
        ..Default::default()
    }];
    assert_eq!(
        material_runs(&vertices[8..12], &entity, [true; PROGRAMS], None)
            .map(|r| (r.range, r.program, r.polygon_offset))
            .collect::<Vec<_>>(),
        [(0..4, Some(ENTITY), false)]
    );
}

/// A block atlas of 64 by 32 pixels: layer 0 at (0, 0), layer 1 at (16, 0), both 16 pixels a
/// side; layer 2 at (32, 16), 8 a side (a stitched sprite of its own size).
fn atlas() -> BlockAtlas {
    BlockAtlas::new(&[0, 16, 16, 16, 32 | 16 << 16, 8], [64, 32]).unwrap()
}

/// A quad of a native terrain kind: a terrain model's sprite layer in the layer's high bits, a
/// fluid's in `glint_uv.x`.
fn native(overlay: u32, sprite: f32) -> [Vertex; 4] {
    let fluid = overlay == FLUID_OVERLAY;
    [Vertex {
        layer: if fluid {
            overlay
        } else {
            overlay | (sprite as u32) << 16
        },
        glint_uv: [if fluid { sprite } else { 0.0 }, 0.0],
        uv: [0.5, 0.5],
        ..bytemuck::Zeroable::zeroed()
    }; 4]
}

/// A block entity's or an entity's native terrain quads (a mod's fluids and terrain models) go to
/// the run's program when the block atlas holds their sprite, the translucent ones to its
/// translucent program; split from the model's quads in vertex order. Those it lacks stay
/// Sunrise's.
#[test]
fn native_terrain_quads_take_the_runs_program_where_the_block_atlas_holds_them() {
    let model = [Vertex {
        layer: 3 << 16,
        ..bytemuck::Zeroable::zeroed()
    }; 4];
    let mut nan_uv = native(TERRAIN_MODEL_SOLID_OVERLAY, 0.0);
    nan_uv[2].uv[0] = f32::NAN;
    let vertices = [
        model,
        native(TERRAIN_MODEL_SOLID_OVERLAY, 1.0),
        native(TERRAIN_MODEL_CUTOUT_OVERLAY, 2.0),
        native(TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, 0.0),
        native(FLUID_OVERLAY, 2.0),
        native(TERRAIN_MODEL_TRANSLUCENT_OVERLAY, 0.0),
        // Past the table, a negative and a NaN fluid layer, a NaN UV.
        native(TERRAIN_MODEL_SOLID_OVERLAY, 3.0),
        native(FLUID_OVERLAY, -1.0),
        native(FLUID_OVERLAY, f32::NAN),
        nan_uv,
        model,
    ]
    .concat();
    let spans = [EntitySpan {
        stage: render_stage("BLOCK_ENTITIES"),
        id: 71,
        ..Default::default()
    }];
    let atlas = atlas();
    let runs = |drawn, atlas| {
        material_runs(&vertices, &spans, drawn, atlas)
            .map(|r| {
                (
                    r.range,
                    r.program,
                    r.polygon_offset,
                    r.block_atlas,
                    r.sorted,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        runs([true; PROGRAMS], Some(&atlas)),
        [
            (0..4, Some(BLOCK), false, false, false),
            (4..16, Some(BLOCK), false, true, false),
            (16..24, Some(BLOCK_TRANSLUCENT), false, true, true),
            (24..40, None, false, false, false),
            (40..44, Some(BLOCK), false, false, false),
        ]
    );
    let mut opaque = [true; PROGRAMS];
    opaque[BLOCK_TRANSLUCENT] = false;
    assert_eq!(
        runs(opaque, Some(&atlas)),
        [
            (0..4, Some(BLOCK), false, false, false),
            (4..16, Some(BLOCK), false, true, false),
            (16..24, Some(BLOCK), false, true, true),
            (24..40, None, false, false, false),
            (40..44, Some(BLOCK), false, false, false),
        ],
        "no translucent block program: the block program draws them all, the translucent ones \
         sorted apart"
    );
    let mut no_block = [true; PROGRAMS];
    no_block[BLOCK] = false;
    no_block[BLOCK_TRANSLUCENT] = false;
    assert_eq!(
        runs(no_block, Some(&atlas)),
        [
            (0..4, Some(BLOCK), false, false, false),
            (4..40, None, false, false, false),
            (40..44, Some(BLOCK), false, false, false),
        ],
        "no block program: all Sunrise's, so none is cast either"
    );
    assert_eq!(
        runs([true; PROGRAMS], None),
        [
            (0..4, Some(BLOCK), false, false, false),
            (4..40, None, false, false, false),
            (40..44, Some(BLOCK), false, false, false),
        ],
        "no block atlas: all Sunrise's"
    );
    // In an entity's run (spec 2.3: `gbuffers_entities`), the same, with the translucent kinds
    // the translucent entity program's; Sunrise's when the pack has no program for them.
    let entity = [EntitySpan {
        stage: render_stage("ENTITIES"),
        id: 72,
        ..Default::default()
    }];
    let entity_runs = |drawn| {
        material_runs(&vertices[..28], &entity, drawn, Some(&atlas))
            .map(|r| (r.range, r.program, r.block_atlas, r.sorted))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        entity_runs([true; PROGRAMS]),
        [
            (0..4, Some(ENTITY), false, false),
            (4..16, Some(ENTITY), true, false),
            (16..24, Some(TRANSLUCENT), true, true),
            (24..28, None, false, false),
        ]
    );
    let mut no_entity = [true; PROGRAMS];
    no_entity[ENTITY] = false;
    no_entity[TRANSLUCENT] = false;
    assert_eq!(
        entity_runs(no_entity),
        [
            (0..4, Some(ENTITY), false, false),
            (4..28, None, false, false)
        ]
    );
}

/// Each UV in its sprite's place in the block atlas, clamped to the sprite; a terrain model's
/// float normal as signed bytes; overlay, HDR tier and sprite layer gone. Position, colour and
/// light kept.
#[test]
fn the_block_atlas_places_native_terrain_quads() {
    let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.5, 0.25]];
    let mut model = native(TERRAIN_MODEL_SOLID_OVERLAY, 2.0);
    let mut fluid = native(FLUID_OVERLAY, 1.0);
    let mut translucent = native(TERRAIN_MODEL_TRANSLUCENT_OVERLAY, 0.0);
    for (i, v) in model.iter_mut().enumerate() {
        v.uv = corners[i];
        v.pos = [i as f32, 2.0, 3.0];
        v.color = 0x8040_2010;
        v.light = 0x00a0_00f0;
        // `vs_native_model`'s normal: z's float bytes in the normal, x and y in `glint_uv`.
        v.normal = 0.0f32.to_ne_bytes().map(|b| b as i8);
        v.glint_uv = [-0.6, 0.8];
    }
    fluid[1].uv = [-0.5, 1.5];
    translucent[0].uv = [0.25, 0.75];
    let mut quads = [model, fluid, translucent].concat();
    atlas().place(&mut quads);
    let uvs = quads.iter().map(|v| v.uv).collect::<Vec<_>>();
    assert_eq!(
        uvs,
        [
            // Layer 2: (32 + 8u) / 64, (16 + 8v) / 32.
            [0.5, 0.5],
            [0.625, 0.5],
            [0.625, 0.75],
            [0.5625, 0.5625],
            // Layer 1: (16 + 16u) / 64, 16v / 32; the second corner clamped to (0, 1).
            [0.375, 0.25],
            [0.25, 0.5],
            [0.375, 0.25],
            [0.375, 0.25],
            // Layer 0: 16u / 64, 16v / 32.
            [0.0625, 0.375],
            [0.125, 0.25],
            [0.125, 0.25],
            [0.125, 0.25],
        ]
    );
    for (i, v) in quads[..4].iter().enumerate() {
        assert_eq!(
            (v.pos, v.color, v.light, v.normal),
            (
                [i as f32, 2.0, 3.0],
                0x8040_2010,
                0x00a0_00f0,
                [-76, 102, 0, 0]
            )
        );
    }
    assert!(quads[4..].iter().all(|v| v.normal == [0; 4]));
    assert!(quads.iter().all(|v| v.layer == 0 && v.glint_uv == [0.0; 2]));
}

#[test]
fn dynamic_texture_quads_run_in_the_pack_program_one_texture_a_run() {
    use sunrise_render_core::entity::{
        DYNAMIC_TEXTURE_CUTOUT_OVERLAY, DYNAMIC_TEXTURE_SOLID_OVERLAY, dynamic_texture_bits,
    };
    let mut vertices = Vec::new();
    for (layer, handle) in [
        (DYNAMIC_TEXTURE_SOLID_OVERLAY, 5),
        (DYNAMIC_TEXTURE_CUTOUT_OVERLAY, 5),
        (DYNAMIC_TEXTURE_CUTOUT_OVERLAY, 9),
    ] {
        vertices.extend(
            [Vertex {
                layer,
                glint_uv: dynamic_texture_bits(handle),
                ..bytemuck::Zeroable::zeroed()
            }; 4],
        );
    }
    let spans = [EntitySpan {
        stage: render_stage("ENTITIES"),
        id: 3,
        ..Default::default()
    }];
    let runs = material_runs(&vertices, &spans, [true; PROGRAMS], None)
        .map(|r| (r.range, r.program, r.texture))
        .collect::<Vec<_>>();
    assert_eq!(
        runs,
        [
            (0..8, Some(ENTITY), Some(5)),
            (8..12, Some(ENTITY), Some(9)),
        ]
    );
}
