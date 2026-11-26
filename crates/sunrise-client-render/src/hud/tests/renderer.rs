use super::super::*;
use super::support::*;

#[test]
fn effect_icons_are_painted_from_the_mob_effect_directory() {
    use crate::{effects::EffectInstance, hud_layout, hud_overlays, inventory_effects};
    // A synthetic resource tree exercises loading and painting together.
    // No Minecraft installation or GPU is needed for this regression test.
    let root = std::env::temp_dir().join(format!("sunrise-effect-icons-{}", std::process::id()));
    write_vanilla_gui_definition(&root);
    for (name, size) in [
        ("mob_effect/levitation", 18),
        ("gui/sprites/hud/effect_background", 24),
        ("gui/sprites/container/inventory/effect_background", 32),
    ] {
        let path = root.join(format!("assets/minecraft/textures/{name}.png"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![255; (size * size * 4) as usize])
            .unwrap();
    }
    let atlas = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let effect = EffectInstance::new("levitation", -1, 0);
    let icon = hud_overlays::EffectIcon::from(&effect);
    let draws = hud_overlays::effects(320, false, &[icon]);
    let mut batch = HudBatch::scratch([320, 240]);
    hud_layout::paint(&mut batch, &atlas, &draws);
    // Both the background and the 18x18 effect icon must reach the batch.
    assert_eq!(batch.vertices.len(), 12);
    let rect = atlas.sprites["mob_effect/levitation"];
    assert_eq!(batch.vertices[6].pos, [298.0, 30.0]);
    assert_eq!(batch.vertices[6].uv, [rect.u0, rect.v0]);
    assert_eq!(batch.vertices[8].uv, [rect.u1, rect.v1]);

    let panels = {
        let space = crate::inventory_effects::EffectsSpace {
            screen_width: 320,
            origin: [72, 37],
            image_width: 176,
        };
        let vanilla = crate::inventory_effects::EffectsLayout::vanilla(space.available());
        crate::inventory_effects::panels(
            &space,
            &vanilla,
            std::slice::from_ref(&effect),
            20.0,
            &|_| None,
            &|_| 60,
        )
    };
    batch.vertices.clear();
    let tooltip = inventory_effects::paint(&mut batch, &atlas, &panels, [257, 44]);
    let lines = inventory_effects::tooltip(tooltip.unwrap());
    let plain: Vec<_> = lines.iter().map(|(t, c)| (t.plain(), *c)).collect();
    assert_eq!(
        plain,
        [
            ("levitation".to_owned(), u32::MAX),
            ("∞".to_owned(), u32::MAX)
        ]
    );
    assert_eq!(batch.vertices.len(), 24);
    assert_eq!(batch.vertices[18].pos, [257.0, 44.0]);
    assert_eq!(batch.vertices[18].uv, [rect.u0, rect.v0]);
    assert_eq!(batch.vertices[20].uv, [rect.u1, rect.v1]);

    // Wide panels retain the four-pixel border instead of stretching it.
    let panels = {
        let space = crate::inventory_effects::EffectsSpace {
            screen_width: 480,
            origin: [72, 37],
            image_width: 176,
        };
        let vanilla = crate::inventory_effects::EffectsLayout::vanilla(space.available());
        crate::inventory_effects::panels(
            &space,
            &vanilla,
            std::slice::from_ref(&effect),
            20.0,
            &|_| None,
            &|_| 60,
        )
    };
    batch.vertices.clear();
    assert!(inventory_effects::paint(&mut batch, &atlas, &panels, [0, 0]).is_none());
    assert!(batch.vertices.iter().any(|v| v.color == 0xff808080));
    assert_eq!(batch.vertices[0].pos, [250.0, 37.0]);
    assert_eq!(batch.vertices[1].pos, [254.0, 37.0]);
    let background = atlas.sprites["gui/sprites/container/inventory/effect_background"];
    assert!(
        (batch.vertices[1].uv[0] - background.u0 - (background.u1 - background.u0) / 8.0).abs()
            < 1e-6
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn gui_models_keep_separate_depth_runs_between_overlay_draws() {
    let mut batch = HudBatch::scratch([640, 480]);
    batch.scale = 2.0;
    let p = [
        [0.0, 0.0, 2.0],
        [0.0, 1.0, 2.0],
        [1.0, 1.0, 2.0],
        [1.0, 0.0, 2.0],
    ];
    let uv = [[0.0; 2]; 4];
    batch.quad([[0.0; 2]; 4], uv, u32::MAX);
    for _ in 0..2 {
        let start = batch.vertices.len();
        batch.quad_3d(p, uv, u32::MAX);
        batch.depth_since(start, [1.0, 2.0, 4.0, 6.0]);
    }
    batch.quad([[0.0; 2]; 4], uv, u32::MAX);
    assert_eq!(
        batch.runs(),
        vec![
            (0..6, Blend::Alpha),
            (6..12, Blend::Depth([2, 4, 6, 8])),
            (12..18, Blend::Depth([2, 4, 6, 8])),
            (18..24, Blend::Alpha),
        ]
    );
    assert!(
        batch
            .vertices
            .iter()
            .all(|v| v.depth.is_finite() && (0.0..=1.0).contains(&v.depth))
    );
}

/// At GUI scale 2, geometry ending at GUI x = 5.375 reaches device x = 10.75 and takes
/// the pixel centred at 10.5. An explicit clip ending at GUI x = 5.125 reaches device
/// x = 10.25 and gains no column.
#[test]
fn a_depth_scissor_over_geometry_rounds_outward_and_an_explicit_clip_does_not() {
    let quad = |batch: &mut HudBatch| {
        let start = batch.vertices.len();
        batch.quad_3d(
            [
                [0.625, 1.0, 0.0],
                [0.625, 3.25, 0.0],
                [5.375, 3.25, 1.0],
                [5.375, 1.0, 1.0],
            ],
            [[0.0; 2]; 4],
            u32::MAX,
        );
        start
    };
    let mut batch = HudBatch::scratch([640, 480]);
    batch.scale = 2.0;
    let start = quad(&mut batch);
    batch.depth_over(start, [0.625, 1.0], [5.375, 3.25]);
    assert_eq!(batch.runs(), vec![(0..6, Blend::Depth([1, 2, 10, 5]))]);
    let mut batch = HudBatch::scratch([640, 480]);
    batch.scale = 2.0;
    let start = quad(&mut batch);
    batch.depth_since(start, [0.625, 1.0, 5.125, 3.25]);
    assert_eq!(batch.runs(), vec![(0..6, Blend::Depth([1, 2, 9, 4]))]);
}

#[test]
fn a_gui_models_glint_follows_it_in_its_depth_pass() {
    let mut batch = HudBatch::scratch([640, 480]);
    batch.scale = 2.0;
    let face = |z: f32| [[0.0, 0.0, z], [0.0, 1.0, z], [1.0, 1.0, z], [1.0, 0.0, z]];
    let uv = [[0.25, 0.5]; 4];
    let start = batch.vertices.len();
    batch.quad_3d(face(1.0), uv, u32::MAX);
    batch.quad_3d(face(3.0), uv, u32::MAX);
    let glint = batch.vertices.len();
    batch.quad_3d_glint(face(3.0), uv, [[7.0, 8.0]; 4], true);
    batch.quad_3d_glint(face(1.0), uv, [[1.0, 2.0]; 4], false);
    batch.depth_glint_since(start, glint, [1.0, 2.0, 4.0, 6.0]);
    assert_eq!(
        batch.runs(),
        vec![
            (0..12, Blend::Depth([2, 4, 6, 8])),
            (12..24, Blend::DepthGlint([2, 4, 6, 8])),
        ]
    );
    // The glint shares its face's depth, normalised with the model's.
    assert_eq!(batch.vertices[12].depth, batch.vertices[6].depth);
    assert_eq!(batch.vertices[18].depth, batch.vertices[0].depth);
    // The shader picks the armour glint by u >= 4, the item's by u >= 2,
    // and masks by the atlas UV under the marker.
    assert_eq!(batch.vertices[12].uv, [4.25, 0.5]);
    assert_eq!(batch.vertices[12].glint_uv, [7.0, 8.0]);
    assert_eq!(batch.vertices[18].uv, [2.25, 0.5]);
    // A model with no glint keeps a single depth run.
    let start = batch.vertices.len();
    batch.quad_3d(face(1.0), uv, u32::MAX);
    batch.depth_glint_since(start, batch.vertices.len(), [1.0, 2.0, 4.0, 6.0]);
    assert_eq!(
        batch.runs().last(),
        Some(&(24..30, Blend::Depth([2, 4, 6, 8])))
    );
}

#[test]
fn hud_shader_validates_both_fragment_entries() {
    let module = naga::front::wgsl::parse_str(include_str!("../../hud.wgsl")).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn inverted_ranges_split_the_draws_in_order() {
    let mut batch = HudBatch::scratch([854, 480]);
    let quad = |b: &mut HudBatch| b.quad([[0.0; 2]; 4], [[0.0; 2]; 4], 0);
    quad(&mut batch);
    let start = batch.vertices.len();
    quad(&mut batch);
    batch.invert_since(start);
    let start = batch.vertices.len();
    quad(&mut batch);
    batch.invert_since(start);
    batch.invert_since(batch.vertices.len());
    let start = batch.vertices.len();
    quad(&mut batch);
    batch.blend_since(start, Blend::Additive);
    quad(&mut batch);
    assert_eq!(
        batch.blends,
        vec![(6..18, Blend::Invert), (18..24, Blend::Additive)]
    );
    assert_eq!(
        batch.runs(),
        vec![
            (0..6, Blend::Alpha),
            (6..18, Blend::Invert),
            (18..24, Blend::Additive),
            (24..30, Blend::Alpha)
        ]
    );
}

#[test]
fn blur_boundary_splits_blend_runs_without_losing_or_repeating_vertices() {
    let mut batch = HudBatch::scratch([854, 480]);
    for _ in 0..5 {
        batch.quad([[0.0; 2]; 4], [[0.0; 2]; 4], 0);
    }
    batch.blends = vec![
        (6..18, Blend::Invert),
        (18..24, Blend::Depth([0, 0, 10, 10])),
    ];
    for split in [0, 6, 12, 18, 24, 30] {
        let before = batch.runs_in(0..split);
        let after = batch.runs_in(split..30);
        let actual: Vec<_> = before
            .chain(after)
            .flat_map(|(range, blend)| range.map(move |v| (v, blend)))
            .collect();
        let expected: Vec<_> = batch
            .runs()
            .into_iter()
            .flat_map(|(range, blend)| range.map(move |v| (v, blend)))
            .collect();
        assert_eq!(actual, expected, "blur at {split}");
    }
}

#[test]
fn container_origins_use_java_integer_division() {
    // Odd scaled dimensions, odd image sizes and an oversized panel.
    for (width, height) in [(640, 360), (641, 361), (456, 256), (534, 300), (100, 100)] {
        let mut batch = HudBatch::scratch([width, height]);
        batch.width = width as f32;
        batch.height = height as f32;
        for (image_width, image_height) in [(195_i32, 136_i32), (176, 166), (176, 222)] {
            assert_eq!(
                batch.centered_origin(image_width as f32, image_height as f32),
                (
                    ((width as i32 - image_width) / 2) as f32,
                    ((height as i32 - image_height) / 2) as f32
                )
            );
        }
    }
}
