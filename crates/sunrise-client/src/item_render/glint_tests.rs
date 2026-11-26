use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn png(path: &Path, w: u32, h: u32, color: [u8; 4]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&color.repeat((w * h) as usize))
        .unwrap();
    writer.finish().unwrap();
}

fn fixture(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("sunrise-glint-5851-{name}-{}", std::process::id()));
    let write = |name: &str, data: &[u8]| {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    };
    write("items.json", b"{}");
    write(
        "assets/minecraft/atlases/items.json",
        br#"{"sources":[
        {"type":"directory","source":"item","prefix":"item/"},
        {"type":"single","resource":"test:source","sprite":"test:item/enchanted_book"},
        {"type":"unstitch","resource":"test:sheet","divisor_x":2,"divisor_y":1,
         "regions":[{"sprite":"minecraft:item/compass_00","x":1,"y":0,"width":1,"height":1}]}]}"#,
    );
    for (name, w, h) in [
        ("minecraft/item/enchanted_book", 16, 16),
        ("minecraft/item/book", 8, 16),
        ("minecraft/item/diamond_sword", 32, 16),
        ("minecraft/item/clock_00", 16, 64),
        ("test/source", 16, 16),
        ("test/sheet", 16, 16),
    ] {
        let (namespace, path) = name.split_once('/').unwrap();
        png(
            &root.join(format!("assets/{namespace}/textures/{path}.png")),
            w,
            h,
            [255; 4],
        );
    }
    write(
        "assets/minecraft/textures/item/clock_00.png.mcmeta",
        br#"{"animation":{"width":16,"height":16}}"#,
    );
    root
}

fn state(sprite: &str, foil: FoilType, display: DisplayContext) -> ItemRenderState {
    let material = sunrise_assets::model::Material {
        sprite: sprite.into(),
        force_translucent: false,
    };
    let baked = BakedItemModel {
        quads: vec![sunrise_assets::bake::ModelQuad {
            positions: [
                [-0.5, 0.5, 0.0],
                [-0.5, -0.5, 0.0],
                [0.5, -0.5, 0.0],
                [0.5, 0.5, 0.0],
            ],
            uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            facing: Direction::South,
            cull: None,
            material: material.clone(),
            tint_index: -1,
            shade_direction_override: None,
            light_emission: 0,
            light: [0; 2],
            color: None,
            ambient_occlusion: None,
            render_type: None,
            shade: true,
        }],
        uses_block_light: false,
        particle: material,
        transforms: Default::default(),
    };
    ItemRenderState {
        display,
        layers: vec![LayerRender {
            geometry: LayerGeometry::Quads(Arc::new(baked)),
            tints: LayerTints::Owned(Vec::new()),
            foil,
            matrix: Mat4::IDENTITY,
            particle: sprite.into(),
            uses_block_light: false,
            extents: Arc::new(vec![]),
            display,
            submit_options: mod_api::SpecialModelSubmitOptions::default(),
            render_type: None,
        }],
        oversized_in_gui: false,
        animated: false,
        cit_glint: None,
        hdr_tier: 0,
        hdr_light: 0,
        held_tint: None,
    }
}

#[test]
fn held_item_tint_is_per_draw_on_cpu_and_gpu() {
    use crate::{
        gfx::entity_skin::{self, SkinFrame},
        mobs::skin::{Skin, SkinModels},
    };

    let root = fixture("held-item-tint");
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        &data,
    )
    .unwrap();
    fs::remove_dir_all(root).unwrap();
    let sprite = "item/book";
    let sprites = HashMap::from([(
        sprite.into(),
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 8,
            h: 16,
        },
    )]);
    let mut plain = state(sprite, FoilType::None, DisplayContext::ThirdPersonRightHand);
    let LayerGeometry::Quads(baked) = &mut plain.layers[0].geometry else {
        unreachable!()
    };
    Arc::get_mut(baked).unwrap().quads[0].tint_index = 0;
    plain.layers[0].tints = LayerTints::Owned(vec![0x7F_FF_60_11]);
    let mut tinted = plain.clone();
    // The high byte is ignored. With ambient-only lighting on the south face,
    // the blue byte goes 17 -> 13 -> 5; lighting before tinting would give 4.
    tinted.set_held_tint(0x23_80_40_C8);
    let lights = Some((Vec3::NEG_Z, Vec3::NEG_Z));
    let pose = Mat4::from_translation(Vec3::Y);
    let mut cpu = Vec::new();
    let mut frame = SkinFrame::default();
    let mut models = SkinModels::default();
    for draw in [&plain, &tinted, &plain] {
        draw.submit(
            &renderer,
            &sprites,
            &NoHost,
            pose,
            &mut WorldSink {
                out: &mut cpu,
                light: 0xF0F0,
                lights,
                glint: None,
                tier: 0,
            },
        );
        assert!(draw.submit_skinned(
            &renderer,
            &sprites,
            pose,
            (0xF0F0, lights, 0),
            &mut Skin::new(&mut models, &mut frame),
        ));
    }
    let gpu = entity_skin::expand(&frame.new_model, &frame);
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&gpu),
        bytemuck::cast_slice::<_, u8>(&cpu)
    );
    assert_eq!(
        cpu.iter().map(|v| v.color).collect::<Vec<_>>(),
        [[0x7F_06_26_66; 4], [0x7F_05_09_33; 4], [0x7F_06_26_66; 4]].concat()
    );
}

#[test]
fn gui_glint_matches_pinned_vanilla_atlas_input() {
    let root = fixture("gui");
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        &data,
    )
    .unwrap();
    renderer.prepare_glint_atlas(16384, (0, 2)).unwrap();
    // The real vanilla oracle also checks a rectangular texture, an animation frame,
    // an unstitch source and an equal-sized sprite in a second namespace.
    for (name, x, y, w, h) in [
        ("item/enchanted_book", 1, 33, 16, 16),
        ("item/book", 53, 1, 8, 16),
        ("item/diamond_sword", 1, 1, 32, 16),
        ("item/clock_00", 35, 1, 16, 16),
        ("item/compass_00", 55, 33, 8, 16),
        ("test:item/enchanted_book", 19, 33, 16, 16),
        ("missingno", 37, 33, 16, 16),
    ] {
        let r = renderer
            .glint_atlas
            .lock()
            .unwrap()
            .get_stitched(name)
            .unwrap();
        assert_eq!(
            [r.u0, r.v0, r.u1, r.v1],
            [x, y, x + w, y + h].map(|n| n as f32 / 64.0)
        );
    }
    let sprites = HashMap::from([
        (
            "item/enchanted_book".into(),
            Rect {
                u0: 0.2,
                v0: 0.3,
                u1: 0.4,
                v1: 0.5,
                w: 16,
                h: 16,
            },
        ),
        (
            "misc/enchanted_glint_item".into(),
            Rect {
                u0: 0.0,
                v0: 0.0,
                u1: 1.0,
                v1: 1.0,
                w: 128,
                h: 128,
            },
        ),
    ]);
    let render = |foil| {
        let mut batch = crate::hud::HudBatch::scratch([64, 64]);
        gui::draw(
            &state("minecraft:item/enchanted_book", foil, DisplayContext::Gui),
            &renderer,
            &sprites,
            &NoHost,
            &mut batch,
            0.0,
            0.0,
            0,
            0.0,
            0.75,
        );
        batch
    };
    let batch = render(FoilType::Standard);
    // Real 26.3 SpriteSourceList + SpriteResourceLoader + Stitcher + JOML, over this fixture:
    // atlas 64x64, book rectangle (1,33)..(17,49). HUD packing above deliberately differs.
    let expected = [
        [-0.5931978, 4.084038],
        [-0.94049406, 6.0536532],
        [1.0291214, 6.4009495],
        [1.3764176, 4.431334],
    ];
    let mut seen = [false; 4];
    for vertex in batch.vertices.iter().filter(|v| v.uv[0] >= 2.0) {
        let index = match vertex.pos {
            [0.0, 0.0] => 0,
            [0.0, 16.0] => 1,
            [16.0, 16.0] => 2,
            [16.0, 0.0] => 3,
            p => panic!("unexpected corner {p:?}"),
        };
        for (got, want) in vertex.glint_uv.into_iter().zip(expected[index]) {
            assert!((got - want).abs() < 1e-5, "glint {got} != vanilla {want}");
        }
        seen[index] = true;
    }
    assert!(seen.into_iter().all(|x| x));
    let plain = render(FoilType::None);
    let base: Vec<_> = batch
        .vertices
        .iter()
        .filter(|v| v.uv[0] < 2.0)
        .map(|v| (v.pos, v.uv, v.color))
        .collect();
    assert_eq!(
        base,
        plain
            .vertices
            .iter()
            .map(|v| (v.pos, v.uv, v.color))
            .collect::<Vec<_>>()
    );
    let book_rect = renderer
        .glint_atlas
        .lock()
        .unwrap()
        .get_stitched("item/enchanted_book")
        .unwrap();
    let book_atlas_uv = [
        [book_rect.u0, book_rect.v0],
        [book_rect.u0, book_rect.v1],
        [book_rect.u1, book_rect.v1],
        [book_rect.u1, book_rect.v0],
    ];
    for display in [
        DisplayContext::Ground,
        DisplayContext::FirstPersonRightHand,
        DisplayContext::Fixed,
    ] {
        let mut quads = Vec::new();
        state("minecraft:item/enchanted_book", FoilType::Standard, display).submit(
            &renderer,
            &sprites,
            &NoHost,
            Mat4::IDENTITY,
            &mut quads,
        );
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].gui_glint_uv, Some(book_atlas_uv), "{display:?}");
        assert_eq!(
            quads[0].foil_uv,
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]
        );
    }
    // `FoilType::Special` projects decal UV3 while keeping sprite-local `foil_uv`.
    // Independent exact bits for `DisplayContext::Gui` (0.5x -> 2.0x inverse), `Mat4::IDENTITY`,
    // `Direction::South` on corners `[[-0.5, 0.5, 0], [-0.5, -0.5, 0], [0.5, -0.5, 0], [0.5, 0.5, 0]]`:
    // `(-x_rot * 1/128, -y_rot * 1/128)` with exact JOML `Quat::rotation_x(FRAC_PI_2)` rounding.
    let mut special_quads = Vec::new();
    state(
        "minecraft:item/enchanted_book",
        FoilType::Special,
        DisplayContext::Gui,
    )
    .submit(
        &renderer,
        &sprites,
        &NoHost,
        Mat4::IDENTITY,
        &mut special_quads,
    );
    assert_eq!(special_quads.len(), 1);
    let expected_south_gui_uv3: [[u32; 2]; 4] = [
        [0xbc00_0000, 0xbc00_0000],
        [0xbc00_0000, 0x3c00_0000],
        [0x3c00_0000, 0x3c00_0000],
        [0x3c00_0000, 0xbc00_0000],
    ];
    assert_eq!(
        special_quads[0]
            .gui_glint_uv
            .unwrap()
            .map(|p| p.map(f32::to_bits)),
        expected_south_gui_uv3
    );
    assert_eq!(
        special_quads[0].foil_uv,
        [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]
    );
    // Nested stacks keep cuboid atlas glint across both outer and inner display contexts.
    for (outer, inner) in [
        (DisplayContext::Fixed, DisplayContext::Gui),
        (DisplayContext::Gui, DisplayContext::Fixed),
    ] {
        let book = "minecraft:item/enchanted_book";
        let mut state = state(book, FoilType::Standard, outer);
        state
            .layers
            .extend(self::state(book, FoilType::Standard, inner).layers);
        let mut quads = Vec::new();
        state.submit(&renderer, &sprites, &NoHost, Mat4::IDENTITY, &mut quads);
        assert_eq!(quads.len(), 2);
        assert_eq!(quads[0].gui_glint_uv, Some(book_atlas_uv), "{outer:?}");
        assert_eq!(quads[1].gui_glint_uv, Some(book_atlas_uv), "{inner:?}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn glint_atlas_boundaries() {
    let root = fixture("boundary");
    // Pinned vanilla over the same generated fixture; padding 1,2,4,8,16.
    for (i, (width, height, x, y)) in [
        (64, 64, 1, 33),
        (128, 64, 2, 34),
        (128, 64, 4, 36),
        (128, 64, 40, 40),
        (256, 128, 16, 80),
    ]
    .into_iter()
    .enumerate()
    {
        let atlas = glint_atlas::load_atlas(
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            "minecraft:items",
            0,
            i as u32,
            16384,
        )
        .unwrap();
        let r = atlas.get_stitched("item/enchanted_book").unwrap();
        assert_eq!(
            [r.u0, r.v0, r.u1, r.v1],
            [
                x as f32 / width as f32,
                y as f32 / height as f32,
                (x + 16) as f32 / width as f32,
                (y + 16) as f32 / height as f32
            ]
        );
    }
    png(
        &root.join("assets/minecraft/textures/item/wide.png"),
        16384,
        1,
        [255; 4],
    );
    let atlas = glint_atlas::load_atlas(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        "minecraft:items",
        0,
        0,
        32768,
    )
    .unwrap();
    let book = atlas.get_stitched("item/enchanted_book").unwrap();
    assert_eq!(
        [book.u0, book.v0, book.u1, book.v1],
        [1.0 / 32768.0, 33.0 / 64.0, 17.0 / 32768.0, 49.0 / 64.0]
    );
    let atlas = glint_atlas::load_atlas(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        "minecraft:items",
        0,
        0,
        16384,
    )
    .unwrap();
    let book = atlas.get_stitched("item/enchanted_book").unwrap();
    assert_eq!(
        [book.u0, book.v0, book.u1, book.v1],
        [1.0 / 64.0, 33.0 / 128.0, 17.0 / 64.0, 49.0 / 128.0]
    );
    assert!(atlas.get_stitched("item/wide").is_none());
    assert_eq!(
        atlas.get_or_missing("item/wide").unwrap().u0,
        atlas.missing().unwrap().u0
    );
    assert!(
        glint_atlas::load_atlas(
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            "minecraft:items",
            0,
            0,
            16,
        )
        .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_and_generated_sprites_match_vanilla() {
    let root = fixture("metadata");
    for name in ["invalid_size", "invalid_metadata", "not_animated"] {
        png(
            &root.join(format!("assets/minecraft/textures/item/{name}.png")),
            16,
            32,
            [255; 4],
        );
    }
    for (name, meta) in [
        ("invalid_size", r#"{"animation":{"width":9,"height":16}}"#),
        ("invalid_metadata", r#"{"animation":{"frames":"bad"}}"#),
        ("not_animated", r#"{"texture":{"blur":true}}"#),
    ] {
        fs::write(
            root.join(format!("assets/minecraft/textures/item/{name}.png.mcmeta")),
            meta,
        )
        .unwrap();
    }
    let path = root.join("assets/minecraft/atlases/items.json");
    let mut atlas: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    atlas["sources"].as_array_mut().unwrap().extend([
        serde_json::json!({"type":"single","resource":"test:absent","sprite":"test:missing_alias"}),
        serde_json::json!({"type":"paletted_permutations","textures":["test:trim"],"palette_key":"test:key","permutations":{"red":"test:red"}}),
    ]);
    fs::write(path, atlas.to_string()).unwrap();
    for (path, w, h, color) in [
        ("assets/test/textures/trim.png", 8, 16, [255; 4]),
        ("assets/test/textures/palettes/key.png", 1, 1, [255; 4]),
        (
            "assets/test/textures/palettes/red.png",
            1,
            1,
            [255, 0, 0, 255],
        ),
    ] {
        png(&root.join(path), w, h, color);
    }
    let atlas = glint_atlas::load_atlas(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        "minecraft:items",
        0,
        0,
        16384,
    )
    .unwrap();
    // Real SpriteResourceLoader rejects both invalid animations; TextureAtlas resolves missingno.
    for name in ["item/invalid_size", "item/invalid_metadata"] {
        assert!(atlas.get_stitched(name).is_none());
        let r = atlas.get_or_missing(name).unwrap();
        assert_eq!(
            [r.u0, r.v0, r.u1, r.v1],
            [103.0 / 128.0, 1.0 / 64.0, 119.0 / 128.0, 17.0 / 64.0]
        );
    }
    let r = atlas.get_stitched("item/not_animated").unwrap();
    assert_eq!(
        [r.u0, r.v0, r.u1, r.v1],
        [1.0 / 128.0, 1.0 / 64.0, 17.0 / 128.0, 33.0 / 64.0]
    );
    let r = atlas.get_stitched("test:trim_red").unwrap();
    assert_eq!(
        [r.u0, r.v0, r.u1, r.v1],
        [19.0 / 128.0, 37.0 / 64.0, 27.0 / 128.0, 53.0 / 64.0]
    );
    assert!(atlas.get_stitched("test:missing_alias").is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn namespaces_sharing_a_prefix_are_all_listed_in_the_glint_atlas() {
    // `a0:` sorts before `a:` as an identifier, though `assets/a/` comes first as a path.
    let root = std::env::temp_dir().join(format!("sunrise-glint-prefix-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("assets/minecraft/atlases")).unwrap();
    fs::write(
        root.join("assets/minecraft/atlases/items.json"),
        r#"{"sources":[{"type":"single","resource":"a:item/one"},{"type":"single","resource":"a0:item/two"}]}"#,
    )
    .unwrap();
    png(
        &root.join("assets/a/textures/item/one.png"),
        16,
        16,
        [255; 4],
    );
    png(
        &root.join("assets/a0/textures/item/two.png"),
        16,
        16,
        [255; 4],
    );
    let atlas = glint_atlas::load_atlas(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        "minecraft:items",
        0,
        0,
        16384,
    )
    .unwrap();
    // Each is its own 16 x 16 cell; one the listing missed would be the missing sprite's.
    let corner = |name: &str| {
        let r = atlas.get_stitched(name).unwrap();
        [r.u0, r.v0]
    };
    assert_ne!(corner("a:item/one"), corner("a0:item/two"));
    assert_ne!(corner("a:item/one"), corner("missingno"));
    assert_ne!(corner("a0:item/two"), corner("missingno"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_texture_metadata_does_not_move_book_glint() {
    let root = fixture("texture-metadata");
    let image = root.join("assets/minecraft/textures/item/large.png");
    png(&image, 512, 256, [255; 4]);
    let metadata = image.with_extension("png.mcmeta");
    for invalid in [
        r#"{"texture":[]}"#,
        r#"{"texture":null}"#,
        r#"{"texture":{"mipmap_strategy":"issue5851_invalid"}}"#,
        r#"{"texture":{"blur":"true"}}"#,
        r#"{"texture":{"clamp":1}}"#,
        r#"{"texture":{"alpha_cutoff_bias":"0.5"}}"#,
    ] {
        fs::write(&metadata, invalid).unwrap();
        let atlas = glint_atlas::load_atlas(
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            "minecraft:items",
            0,
            0,
            16384,
        )
        .unwrap();
        assert!(atlas.get_stitched("item/large").is_none(), "{invalid}");
        let rejected = atlas.get_or_missing("item/large").unwrap();
        let missing = atlas.missing().unwrap();
        assert_eq!(
            [rejected.u0, rejected.v0, rejected.u1, rejected.v1],
            [missing.u0, missing.v0, missing.u1, missing.v1],
            "{invalid}"
        );
        // Existing real-vanilla fixture: rejecting the extra image leaves its book unchanged.
        let book = atlas.get_stitched("item/enchanted_book").unwrap();
        assert_eq!(
            [book.u0, book.v0, book.u1, book.v1],
            [1.0 / 64.0, 33.0 / 64.0, 17.0 / 64.0, 49.0 / 64.0],
            "{invalid}"
        );
    }
    for strategy in ["auto", "mean", "cutout", "strict_cutout", "dark_cutout"] {
        fs::write(
            &metadata,
            serde_json::json!({"texture":{"blur":true,"clamp":false,
                "mipmap_strategy":strategy,"alpha_cutoff_bias":0.5}})
            .to_string(),
        )
        .unwrap();
        let atlas = glint_atlas::load_atlas(
            &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
            "minecraft:items",
            0,
            0,
            16384,
        )
        .unwrap();
        let large = atlas.get_stitched("item/large").unwrap();
        assert_eq!((large.w, large.h), (512, 256));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn glint_atlas_reads_prepared_images_and_metadata_together() {
    let root = fixture("prepared-metadata");
    let image = root.join("assets/minecraft/textures/item/large.png");
    png(&image, 512, 256, [255; 4]);
    let metadata = image.with_extension("png.mcmeta");
    fs::write(&metadata, r#"{"texture":[]}"#).unwrap();
    fs::write(root.join("sunrise-resources.json"), "{}").unwrap();
    sunrise_core::prepared_files::write(&root).unwrap();
    // The renderer reads the prepared snapshot, even if loose files now differ.
    fs::write(&metadata, r#"{"texture":{"blur":true}}"#).unwrap();
    png(
        &root.join("assets/minecraft/textures/item/enchanted_book.png"),
        256,
        256,
        [0, 255, 0, 255],
    );
    let atlas = glint_atlas::load_atlas(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        "minecraft:items",
        0,
        0,
        16384,
    )
    .unwrap();
    assert!(atlas.get_stitched("item/large").is_none());
    assert_eq!(
        atlas.get_or_missing("item/large").unwrap().u0,
        atlas.missing().unwrap().u0
    );
    let book = atlas.get_stitched("item/enchanted_book").unwrap();
    assert_eq!(
        [book.u0, book.v0, book.u1, book.v1],
        [1.0 / 64.0, 33.0 / 64.0, 17.0 / 64.0, 49.0 / 64.0]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cuboid_glint_prefers_stitched_blocks_and_blocks_missingno() {
    let root = fixture("material-baker-fallback");
    // Extra large sprite in `blocks` makes its `missingno` rectangle distinct from `items`.
    fs::write(
        root.join("assets/minecraft/atlases/items.json"),
        r#"{"sources":[
            {"type":"single","resource":"test:bad_item","sprite":"test:shared"},
            {"type":"single","resource":"minecraft:item/enchanted_book"}
        ]}"#,
    )
    .unwrap();
    fs::write(
        root.join("assets/minecraft/atlases/blocks.json"),
        r#"{"sources":[
            {"type":"single","resource":"test:good_block","sprite":"test:shared"},
            {"type":"single","resource":"test:big_block"}
        ]}"#,
    )
    .unwrap();
    // Corrupt PNG in `items` fails to decode; valid PNG in `blocks` stitches for the same ID.
    fs::create_dir_all(root.join("assets/test/textures")).unwrap();
    fs::write(root.join("assets/test/textures/bad_item.png"), b"not a png").unwrap();
    png(
        &root.join("assets/test/textures/good_block.png"),
        16,
        16,
        [255; 4],
    );
    png(
        &root.join("assets/test/textures/big_block.png"),
        128,
        128,
        [255; 4],
    );
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        &data,
    )
    .unwrap();
    renderer
        .prepare_glint_atlas_with_mips(16384, (0, 2), 0)
        .unwrap();

    let items_missing = renderer.glint_atlas.lock().unwrap().missing().unwrap();
    let blocks_missing = renderer
        .block_glint_atlas
        .lock()
        .unwrap()
        .missing()
        .unwrap();
    let blocks_shared = renderer
        .block_glint_atlas
        .lock()
        .unwrap()
        .get_stitched("test:shared")
        .unwrap();
    assert!(
        renderer
            .glint_atlas
            .lock()
            .unwrap()
            .get_stitched("test:shared")
            .is_none()
    );
    assert_eq!(
        renderer
            .glint_atlas
            .lock()
            .unwrap()
            .get_or_missing("test:shared")
            .unwrap(),
        items_missing
    );
    assert_ne!(
        [
            items_missing.u0,
            items_missing.v0,
            items_missing.u1,
            items_missing.v1
        ],
        [
            blocks_missing.u0,
            blocks_missing.v0,
            blocks_missing.u1,
            blocks_missing.v1
        ]
    );

    let dummy = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    let sprites = HashMap::from([("test:shared".into(), dummy), ("missingno".into(), dummy)]);
    let rect_uv = |r: Rect| [[r.u0, r.v0], [r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0]];

    // Failed items source + stitched blocks source resolves to blocks' stitched rectangle.
    let mut shared_quads = Vec::new();
    state("test:shared", FoilType::Standard, DisplayContext::Gui).submit(
        &renderer,
        &sprites,
        &NoHost,
        Mat4::IDENTITY,
        &mut shared_quads,
    );
    assert_eq!(shared_quads.len(), 1);
    assert_eq!(shared_quads[0].gui_glint_uv, Some(rect_uv(blocks_shared)));

    // Explicit `minecraft:missingno` resolves to blocks' missingno, never items' missingno.
    let mut missing_quads = Vec::new();
    state(
        "minecraft:missingno",
        FoilType::Standard,
        DisplayContext::Gui,
    )
    .submit(
        &renderer,
        &sprites,
        &NoHost,
        Mat4::IDENTITY,
        &mut missing_quads,
    );
    assert_eq!(missing_quads.len(), 1);
    assert_eq!(missing_quads[0].gui_glint_uv, Some(rect_uv(blocks_missing)));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn mip_level_reduces_from_loaded_sprites_and_restitching_updates_block_glint() {
    let root = fixture("mip-restitch");
    fs::write(
        root.join("assets/minecraft/atlases/blocks.json"),
        r#"{"sources":[
            {"type":"single","resource":"test:stone"},
            {"type":"single","resource":"test:tall"}
        ]}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("assets/test/textures")).unwrap();
    png(
        &root.join("assets/test/textures/stone.png"),
        16,
        16,
        [255; 4],
    );
    // 12x16 has lowestOneBit(12) == 4, reducing requested mip 4 to mip 2 (padding 4).
    png(
        &root.join("assets/test/textures/tall.png"),
        12,
        16,
        [255; 4],
    );
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        &data,
    )
    .unwrap();
    let mut settings = crate::settings::Settings::default();
    let (f, a, m) = settings.texture_atlas_settings();
    renderer
        .prepare_glint_atlas_with_mips(16384, (f, a), m)
        .unwrap();
    let mip4_rect = renderer
        .block_glint_atlas
        .lock()
        .unwrap()
        .get_stitched("test:stone")
        .unwrap();
    // Requesting 16 mip levels still reduces cleanly to mip 2 without any artificial cap.
    let reduced = glint_atlas::load_atlas(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        "minecraft:blocks",
        16,
        0,
        16384,
    )
    .unwrap();
    assert_eq!(reduced.get_stitched("test:stone").unwrap(), mip4_rect);

    // Changing live `mipmap_levels` from 4 to 0 and restitching moves the block glint rectangle.
    settings.mipmap_levels = 0;
    let (f, a, m) = settings.texture_atlas_settings();
    assert_eq!(m, 0);
    renderer
        .prepare_glint_atlas_with_mips(16384, (f, a), m)
        .unwrap();
    let mip0_rect = renderer
        .block_glint_atlas
        .lock()
        .unwrap()
        .get_stitched("test:stone")
        .unwrap();
    assert_ne!(
        [mip4_rect.u0, mip4_rect.v0, mip4_rect.u1, mip4_rect.v1],
        [mip0_rect.u0, mip0_rect.v0, mip0_rect.u1, mip0_rect.v1]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn prepared_item_atlas_matches_pinned_vanilla() {
    let root =
        PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources"));
    let atlas = glint_atlas::load_atlas(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        "minecraft:items",
        0,
        0,
        16384,
    )
    .unwrap();
    assert_eq!(atlas.len(), 896);
    let book = atlas.get_stitched("item/enchanted_book").unwrap();
    assert_eq!(
        [book.u0, book.v0, book.u1, book.v1],
        [437.0 / 1024.0, 163.0 / 512.0, 453.0 / 1024.0, 179.0 / 512.0]
    );
    let glint = Glint {
        millis: 0,
        speed: 0.0,
        alpha: 0.75,
        scale: 8.0,
    };
    let uv = glint.uv([book.u0, book.v0]);
    for (got, want) in uv.into_iter().zip([2.919935, 3.101028]) {
        assert!((got - want).abs() < 1e-5);
    }
}

#[test]
#[ignore = "requires an independently generated pinned vanilla atlas JSON and matching resources"]
fn external_item_atlas_oracle() {
    let root = PathBuf::from(std::env::var_os("SUNRISE_GLINT_ROOT").expect("resource root"));
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(
            std::env::var_os("SUNRISE_GLINT_EXPECTED").expect("vanilla atlas JSON"),
        )
        .unwrap(),
    )
    .unwrap();
    let bit = std::env::var("SUNRISE_GLINT_ANISO_BITS")
        .unwrap()
        .parse()
        .unwrap();
    let sprites = glint_atlas::load_atlas(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        "minecraft:items",
        0,
        glint_padding(if bit == 0 { 0 } else { 2 }, bit) as u32,
        32768,
    )
    .unwrap();
    for (name, value) in expected["sprites"].as_object().unwrap() {
        let actual = sprites.get_stitched(sprite_name(name)).unwrap();
        for (got, key) in [actual.u0, actual.v0, actual.u1, actual.v1]
            .into_iter()
            .zip(["u0", "v0", "u1", "v1"])
        {
            let want = value[key].as_f64().unwrap() as f32;
            assert!(
                (got - want).abs() < 1e-7,
                "{name} {key}: {got} != vanilla {want}"
            );
        }
    }
    if let Some(rejected) = expected["rejected"].as_array() {
        let missing = sprites.missing().unwrap();
        for name in rejected {
            let key = sprite_name(name.as_str().unwrap());
            assert!(sprites.get_stitched(key).is_none(), "{key}");
            let actual = sprites.get_or_missing(key).unwrap();
            assert_eq!(
                [actual.u0, actual.v0, actual.u1, actual.v1],
                [missing.u0, missing.v0, missing.u1, missing.v1]
            );
        }
        let renderer = ItemRenderer::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &data,
        )
        .unwrap();
        renderer
            .prepare_glint_atlas(32768, (if bit == 0 { 0 } else { 2 }, bit))
            .unwrap();
        let hud_sprites = HashMap::from([
            ("item/enchanted_book".into(), missing),
            ("misc/enchanted_glint_item".into(), missing),
        ]);
        let mut batch = crate::hud::HudBatch::scratch([64, 64]);
        gui::draw(
            &state(
                "minecraft:item/enchanted_book",
                FoilType::Standard,
                DisplayContext::Gui,
            ),
            &renderer,
            &hud_sprites,
            &NoHost,
            &mut batch,
            0.0,
            0.0,
            0,
            0.0,
            0.75,
        );
        let want = &expected["sprites"]["minecraft:item/enchanted_book"]["glint_uv_clock0"];
        let mut seen = [false; 4];
        for vertex in batch.vertices.iter().filter(|v| v.uv[0] >= 2.0) {
            let index = match vertex.pos {
                [0.0, 0.0] => 0,
                [0.0, 16.0] => 3,
                [16.0, 16.0] => 2,
                [16.0, 0.0] => 1,
                p => panic!("unexpected corner {p:?}"),
            };
            for (got, value) in vertex
                .glint_uv
                .into_iter()
                .zip(want[index].as_array().unwrap())
            {
                assert!((got - value.as_f64().unwrap() as f32).abs() < 1e-5);
            }
            seen[index] = true;
        }
        assert!(seen.into_iter().all(|v| v));
    }
}

/// A posed item has a depth of its own, as a GUI model does: its vertices carry a depth from the
/// turn (nearest 0, farthest 1) and one run takes the depth test within the item's own bounds.
#[test]
fn a_posed_item_has_a_depth_of_its_own() {
    let root = fixture("posed");
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        &data,
    )
    .unwrap();
    let sprite = Rect {
        u0: 0.2,
        v0: 0.3,
        u1: 0.4,
        v1: 0.5,
        w: 16,
        h: 16,
    };
    let sprites = HashMap::from([("item/enchanted_book".to_owned(), sprite)]);
    let mut batch = crate::hud::HudBatch::scratch([64, 64]);
    // A flat face about its centre (40.75, 30.5), 16 pixels a block, turned 60 degrees about y:
    // its corners land at x 40.75 +- 4 (16 * 0.5 * cos 60) and y 30.5 +- 8, one side nearer.
    let pose = Mat4::from_translation(Vec3::new(40.75, 30.5, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0))
        * Mat4::from_rotation_y(60f32.to_radians());
    assert!(gui::draw_posed(
        &state(
            "minecraft:item/enchanted_book",
            FoilType::None,
            DisplayContext::Gui
        ),
        &gui::GuiItemEnv {
            renderer: &renderer,
            sprites: &sprites,
            host: &NoHost,
        },
        &mut batch,
        pose,
        GlintFrame::default(),
    ));
    assert_eq!(batch.vertices.len(), 6);
    let depths: Vec<f32> = batch.vertices.iter().map(|v| v.depth).collect();
    assert_eq!(depths.iter().copied().fold(f32::INFINITY, f32::min), 0.0);
    assert_eq!(
        depths.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        1.0
    );
    // The scissor takes every pixel the bounds [36.75, 44.75] x [22.5, 38.5] touch, including
    // the column centred at x = 44.5: rounding the edges out gives a width of 9 and a height of 17.
    assert_eq!(batch.scale, 1.0);
    assert_eq!(
        batch.blends,
        [(0..6, crate::hud::Blend::Depth([36, 22, 9, 17]))]
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn translucent_standard_and_special_foil_emit_translucent_glint_overlay_with_distinct_uvs() {
    let root = fixture("translucent-glint-oit");
    png(
        &root.join("assets/minecraft/textures/item/translucent_crystal.png"),
        16,
        16,
        [200, 120, 255, 128],
    );
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        &data,
    )
    .unwrap();
    renderer.prepare_glint_atlas(16384, (0, 2)).unwrap();
    let hud_rect = Rect {
        u0: 0.1,
        v0: 0.2,
        u1: 0.3,
        v1: 0.4,
        w: 16,
        h: 16,
    };
    let sprites = HashMap::from([
        ("item/translucent_crystal".to_owned(), hud_rect),
        ("item/enchanted_book".to_owned(), hud_rect),
    ]);
    let glint = Glint {
        millis: 123_456,
        speed: 0.5,
        alpha: 0.75,
        scale: Glint::ITEM_SCALE,
    };
    let atlas_rect = renderer
        .glint_atlas
        .lock()
        .unwrap()
        .get_stitched("item/translucent_crystal")
        .unwrap();
    let expected_atlas_uv = [
        [atlas_rect.u0, atlas_rect.v0],
        [atlas_rect.u0, atlas_rect.v1],
        [atlas_rect.u1, atlas_rect.v1],
        [atlas_rect.u1, atlas_rect.v0],
    ];
    let expected_hud_uv = [
        [hud_rect.u0, hud_rect.v0],
        [hud_rect.u0, hud_rect.v1],
        [hud_rect.u1, hud_rect.v1],
        [hud_rect.u1, hud_rect.v0],
    ];

    for (foil, is_special) in [(FoilType::Standard, false), (FoilType::Special, true)] {
        let mut out = Vec::new();
        let st = state(
            "minecraft:item/translucent_crystal",
            foil,
            DisplayContext::Ground,
        );
        st.submit(
            &renderer,
            &sprites,
            &NoHost,
            Mat4::IDENTITY,
            &mut WorldSink {
                out: &mut out,
                light: 0xF0F0,
                lights: None,
                glint: Some(glint),
                tier: 0,
            },
        );
        assert_eq!(out.len(), 4);
        assert!(
            out.iter()
                .all(|v| (v.layer & 0xFFFF) == crate::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY),
            "translucent {foil:?} must use TRANSLUCENT_GLINT_ITEM_OVERLAY (OIT bucket 8)"
        );
        let uv0: [[u32; 2]; 4] = std::array::from_fn(|i| out[i].uv.map(f32::to_bits));
        let guv: [[u32; 2]; 4] = std::array::from_fn(|i| out[i].glint_uv.map(f32::to_bits));
        assert_eq!(uv0, expected_hud_uv.map(|p| p.map(f32::to_bits)));
        let atlas_glint = expected_atlas_uv.map(|p| glint.uv(p).map(f32::to_bits));
        if is_special {
            assert_ne!(
                guv, atlas_glint,
                "special foil must project UV3, not atlas UV0"
            );
            assert_ne!(guv, uv0);
        } else {
            assert_eq!(guv, atlas_glint, "standard foil must use glint-atlas UV0");
            assert_ne!(guv, uv0);
        }

        // `force_translucent` on an opaque sprite routes the same way.
        let mut forced = state("minecraft:item/enchanted_book", foil, DisplayContext::Gui);
        let LayerGeometry::Quads(baked) = &mut forced.layers[0].geometry else {
            unreachable!();
        };
        let mut b = (**baked).clone();
        b.quads[0].material.force_translucent = true;
        *baked = Arc::new(b);
        let mut forced_out = Vec::new();
        forced.submit(
            &renderer,
            &sprites,
            &NoHost,
            Mat4::IDENTITY,
            &mut WorldSink {
                out: &mut forced_out,
                light: 0xF0F0,
                lights: None,
                glint: Some(glint),
                tier: 0,
            },
        );
        assert_eq!(forced_out.len(), 4);
        assert!(
            forced_out
                .iter()
                .all(|v| (v.layer & 0xFFFF) == crate::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY)
        );
    }

    // Both foil variants submitted alongside another translucent surface in one combined stream.
    let mut combined = Vec::new();
    let other_translucent = state(
        "minecraft:item/translucent_crystal",
        FoilType::None,
        DisplayContext::Ground,
    );
    let std_translucent = state(
        "minecraft:item/translucent_crystal",
        FoilType::Standard,
        DisplayContext::Ground,
    );
    let spec_translucent = state(
        "minecraft:item/translucent_crystal",
        FoilType::Special,
        DisplayContext::Ground,
    );
    for (st, z) in [
        (&other_translucent, -3.0_f32),
        (&std_translucent, -2.0_f32),
        (&spec_translucent, -2.0_f32),
    ] {
        st.submit(
            &renderer,
            &sprites,
            &NoHost,
            Mat4::from_translation(Vec3::new(0.0, 0.0, z)),
            &mut WorldSink {
                out: &mut combined,
                light: 0xF0F0,
                lights: None,
                glint: Some(glint),
                tier: 0,
            },
        );
    }
    assert_eq!(combined.len(), 12);
    assert!(
        combined[0..4]
            .iter()
            .all(|v| (v.layer & 0xFFFF) == crate::gfx::TRANSLUCENT_ITEM_OVERLAY),
        "overlapping unglinted translucent surface uses TRANSLUCENT_ITEM_OVERLAY"
    );
    assert!(
        combined[4..8]
            .iter()
            .all(|v| (v.layer & 0xFFFF) == crate::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY),
        "standard foil overlapping translucent surface uses TRANSLUCENT_GLINT_ITEM_OVERLAY"
    );
    assert!(
        combined[8..12]
            .iter()
            .all(|v| (v.layer & 0xFFFF) == crate::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY),
        "special foil overlapping translucent surface uses TRANSLUCENT_GLINT_ITEM_OVERLAY"
    );
    let std_guv: [[u32; 2]; 4] =
        std::array::from_fn(|i| combined[4 + i].glint_uv.map(f32::to_bits));
    let spec_guv: [[u32; 2]; 4] =
        std::array::from_fn(|i| combined[8 + i].glint_uv.map(f32::to_bits));
    assert_eq!(
        std_guv,
        expected_atlas_uv.map(|p| glint.uv(p).map(f32::to_bits)),
        "standard foil in combined stream must use glint-atlas UV0"
    );
    assert_ne!(
        spec_guv, std_guv,
        "special foil in combined stream must use distinct decal UV3"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn special_foil_preserves_degenerate_inverse_and_normal_overflow_direction() {
    let root = fixture("decal-degenerate-and-normal");
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        &data,
    )
    .unwrap();
    renderer.prepare_glint_atlas(16384, (0, 2)).unwrap();
    let sprites = dummy_hud_sprites(&["item/enchanted_book"]);
    let book_state = |facing: Direction, display: DisplayContext| {
        let mut st = state("minecraft:item/enchanted_book", FoilType::Special, display);
        let LayerGeometry::Quads(baked) = &mut st.layers[0].geometry else {
            unreachable!();
        };
        let mut b = (**baked).clone();
        b.quads[0].facing = facing;
        *baked = Arc::new(b);
        st
    };

    // Flattened [1, 1, 0] preserves non-finite inverse projection instead of returning [0, 0].
    for display in [DisplayContext::Gui, DisplayContext::Ground] {
        let mut quads = Vec::new();
        book_state(Direction::South, display).submit(
            &renderer,
            &sprites,
            &NoHost,
            Mat4::from_scale(Vec3::new(1.0, 1.0, 0.0)),
            &mut quads,
        );
        assert_eq!(quads.len(), 1);
        let uv3 = quads[0].gui_glint_uv.unwrap();
        assert!(
            uv3.iter().all(|[u, v]| !u.is_finite() && !v.is_finite()),
            "{display:?} flattened [1, 1, 0] must preserve non-finite UV3, got {uv3:?}"
        );
    }

    // East-facing quad at scale [1e-21, 1, 1]: transformed normal overflows normalization to
    // (0, 0, 0), so `Direction.getApproximateNearest` selects North instead of East.
    let overflow_scale = Mat4::from_scale(Vec3::new(1e-21, 1.0, 1.0));
    let mut east_quads = Vec::new();
    book_state(Direction::East, DisplayContext::Ground).submit(
        &renderer,
        &sprites,
        &NoHost,
        overflow_scale,
        &mut east_quads,
    );
    let mut north_quads = Vec::new();
    book_state(Direction::North, DisplayContext::Ground).submit(
        &renderer,
        &sprites,
        &NoHost,
        overflow_scale,
        &mut north_quads,
    );
    let mut east_unscaled = Vec::new();
    book_state(Direction::East, DisplayContext::Ground).submit(
        &renderer,
        &sprites,
        &NoHost,
        Mat4::IDENTITY,
        &mut east_unscaled,
    );
    let east_uv = east_quads[0].gui_glint_uv.unwrap();
    let north_uv = north_quads[0].gui_glint_uv.unwrap();
    let unscaled_east_uv = east_unscaled[0].gui_glint_uv.unwrap();
    assert_eq!(
        east_uv.map(|p| p.map(f32::to_bits)),
        north_uv.map(|p| p.map(f32::to_bits)),
        "east quad at scale [1e-21, 1, 1] must match North fallback"
    );
    assert_ne!(
        east_uv.map(|p| p.map(f32::to_bits)),
        unscaled_east_uv.map(|p| p.map(f32::to_bits)),
        "normal-overflow fallback must differ from unscaled East projection"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn gui_special_foil_projects_in_slot_and_oversized_target_pose_before_screen_placement() {
    let root = fixture("gui-slot-decal");
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        &data,
    )
    .unwrap();
    renderer.prepare_glint_atlas(16384, (0, 2)).unwrap();
    let sprites = dummy_hud_sprites(&["item/enchanted_book"]);
    let frame = GlintFrame {
        millis: 123_456,
        speed: 0.5,
        alpha: 0.75,
        sprite: true,
    };

    // Pack-model quad with a corner 2^-20 model units from slot center (0, 0, 0).
    let eps = 2.0_f32.powi(-20);
    let make_state = |offset: f32, oversized: bool| {
        let mut st = state(
            "minecraft:item/enchanted_book",
            FoilType::Special,
            DisplayContext::Gui,
        );
        let LayerGeometry::Quads(baked) = &mut st.layers[0].geometry else {
            unreachable!();
        };
        let mut b = (**baked).clone();
        b.quads[0].positions = [
            [offset, 0.25, 0.0],
            [offset, -0.25, 0.0],
            [0.25, -0.25, 0.0],
            [0.25, 0.25, 0.0],
        ];
        *baked = Arc::new(b);
        if oversized {
            st.oversized_in_gui = true;
            st.layers[0].extents =
                Arc::new(vec![Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 0.0)]);
        }
        st
    };

    let draw_glint_bits = |st: &ItemRenderState, scale: f32, x: f32, y: f32| -> Vec<[u32; 2]> {
        let mut batch = crate::hud::HudBatch::scratch([1024, 1024]);
        batch.scale = scale;
        gui::draw_with_frame(st, &renderer, &sprites, &NoHost, &mut batch, x, y, frame);
        batch
            .vertices
            .iter()
            .filter(|v| v.uv[0] >= 2.0)
            .map(|v| v.glint_uv.map(f32::to_bits))
            .collect()
    };

    for oversized in [false, true] {
        let shifted = make_state(eps, oversized);
        let centered = make_state(0.0, oversized);
        for scale in [1.0_f32, 2.0, 3.0] {
            // Fresh slot `(0, 0)` (or oversized target `(0, 0)`): `2^-20` survives.
            let at_origin = draw_glint_bits(&shifted, scale, 0.0, 0.0);
            for (x, y) in [(8.0, 8.0), (512.0, 256.0), (520.0, 300.0)] {
                let at_pos = draw_glint_bits(&shifted, scale, x, y);
                assert_eq!(
                    at_pos, at_origin,
                    "oversized={oversized} scale={scale} at ({x}, {y}) must reuse the same physical slot"
                );
            }
            renderer.end_gui_frame();
            let at_centered = draw_glint_bits(&centered, scale, 0.0, 0.0);
            renderer.end_gui_frame();
            assert!(!at_origin.is_empty());
            assert_ne!(
                at_origin, at_centered,
                "oversized={oversized} scale={scale}: 2^-20 offset from slot (0, 0) center must survive"
            );
        }
    }

    // Nonzero physical atlas slot `(16, 0)` at scale 1 (`left = 256`, center `264`, ulp `2^-15`):
    // drawing 16 distinct static items occupies slots `(0, 0)..(15, 0)`; the next item lands in
    // `(16, 0)`, where `264.0 + 16.0 * 2^-20 = 264.0 + 2^-16` rounds away to `264.0`.
    let mut fill_batch = crate::hud::HudBatch::scratch([1024, 1024]);
    fill_batch.scale = 1.0;
    let statics: Vec<ItemRenderState> = (0..16)
        .map(|_| {
            state(
                "minecraft:item/enchanted_book",
                FoilType::None,
                DisplayContext::Gui,
            )
        })
        .collect();
    for (i, st) in statics.iter().enumerate() {
        gui::draw_with_frame(
            st,
            &renderer,
            &sprites,
            &NoHost,
            &mut fill_batch,
            i as f32 * 16.0,
            0.0,
            frame,
        );
    }
    let shifted_slot16 = make_state(eps, false);
    let centered_slot16 = make_state(0.0, false);
    assert_eq!(renderer.gui_item_slot(&shifted_slot16, 1.0), (16, 0));
    let slot16_shifted_a = draw_glint_bits(&shifted_slot16, 1.0, 0.0, 0.0);
    let slot16_shifted_b = draw_glint_bits(&shifted_slot16, 1.0, 512.0, 256.0);
    assert_eq!(
        slot16_shifted_a, slot16_shifted_b,
        "same item at different screen positions in one frame reuses physical slot (16, 0)"
    );
    // `end_gui_frame` frees animated slot `(16, 0)` while keeping the 16 static slots.
    renderer.end_gui_frame();
    assert_eq!(renderer.gui_item_slot(&centered_slot16, 1.0), (16, 0));
    let slot16_centered = draw_glint_bits(&centered_slot16, 1.0, 0.0, 0.0);
    assert_eq!(
        slot16_shifted_a, slot16_centered,
        "at physical atlas slot (16, 0) scale 1, 2^-20 offset must round away to match offset 0.0"
    );
    // Oversized item bypasses `GuiItemAtlas` (origin `(0, 0)`), so `2^-20` still survives.
    let shifted_over = make_state(eps, true);
    let centered_over = make_state(0.0, true);
    assert_ne!(
        draw_glint_bits(&shifted_over, 1.0, 0.0, 0.0),
        draw_glint_bits(&centered_over, 1.0, 0.0, 0.0),
        "oversized item must not inherit GuiItemAtlas slot (16, 0)"
    );

    fs::remove_dir_all(root).ok();
}

fn glint_witness_26_3() -> Value {
    let path = std::env::var_os("SUNRISE_ITEM_GLINT_WITNESS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/witness/26.3/glint/witness.json")
        });
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read item glint witness {} (generate with tools/test_glint_parity.py --record-only): {e}",
            path.display()
        )
    });
    serde_json::from_str(&text).expect("valid 26.3 item_glint witness")
}

fn prepared_resources_26_3() -> PathBuf {
    std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3")
        })
}

fn full_renderer_26_3() -> ItemRenderer {
    let root = prepared_resources_26_3();
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .expect("26.3 prepared item data");
    let renderer = ItemRenderer::load(&ResourceRoot::directory(root), &data).unwrap();
    renderer.prepare_glint_atlas(16384, (0, 2)).unwrap();
    renderer
}

fn dummy_hud_sprites(names: &[&str]) -> HashMap<String, Rect> {
    // Deliberately non-vanilla rects so tests fail if HUD UVs leak into glint UVs.
    let rect = Rect {
        u0: 0.1,
        v0: 0.2,
        u1: 0.3,
        v1: 0.4,
        w: 16,
        h: 16,
    };
    let mut map = HashMap::from([("misc/enchanted_glint_item".to_owned(), rect)]);
    for &name in names {
        map.insert(name.to_owned(), rect);
    }
    map
}

fn parse_quad_uv_bits(v: &Value) -> [[u32; 2]; 4] {
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 4);
    std::array::from_fn(|i| {
        let pair = arr[i].as_array().unwrap();
        [
            pair[0].as_u64().unwrap() as u32,
            pair[1].as_u64().unwrap() as u32,
        ]
    })
}

type PosBits = [[u32; 3]; 4];
type UvBits = [[u32; 2]; 4];
type GeomKey = (PosBits, UvBits);

fn parse_quad_pos_bits(v: &Value) -> PosBits {
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 4);
    std::array::from_fn(|i| {
        let p = arr[i].as_array().unwrap();
        [
            p[0].as_u64().unwrap() as u32,
            p[1].as_u64().unwrap() as u32,
            p[2].as_u64().unwrap() as u32,
        ]
    })
}

fn vertex_quad_glint_bits(verts: &[Vertex]) -> Vec<UvBits> {
    assert_eq!(verts.len() % 4, 0);
    verts
        .as_chunks::<4>()
        .0
        .iter()
        .map(|quad| std::array::from_fn(|i| quad[i].glint_uv.map(f32::to_bits)))
        .collect()
}

struct SubmittedQuad {
    pos: PosBits,
    uv0: UvBits,
    uv3: Option<UvBits>,
    glint_uv: UvBits,
}

struct PairedWorldSink<'a> {
    world: WorldSink<'a>,
    quads: Vec<ItemQuad>,
}

impl ItemSink for PairedWorldSink<'_> {
    fn quad(&mut self, q: ItemQuad) {
        self.quads.push(q);
        self.world.quad(q);
    }
}

fn submitted_quads(
    quads: &[ItemQuad],
    verts: &[Vertex],
    special_atlas_rect: Option<Rect>,
) -> Vec<SubmittedQuad> {
    assert_eq!(verts.len(), quads.len() * 4);
    quads
        .iter()
        .zip(verts.as_chunks::<4>().0)
        .map(|(q, vquad)| {
            let pos: PosBits = std::array::from_fn(|i| vquad[i].pos.map(f32::to_bits));
            for i in 0..4 {
                assert_eq!(q.pos[i].to_array().map(f32::to_bits), pos[i]);
            }
            let glint_uv: UvBits = std::array::from_fn(|i| vquad[i].glint_uv.map(f32::to_bits));
            let (uv0, uv3) = match special_atlas_rect {
                Some(r) => (
                    q.foil_uv.map(|[u, v]| {
                        [r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v].map(f32::to_bits)
                    }),
                    Some(q.gui_glint_uv.unwrap().map(|p| p.map(f32::to_bits))),
                ),
                None => (q.gui_glint_uv.unwrap().map(|p| p.map(f32::to_bits)), None),
            };
            SubmittedQuad {
                pos,
                uv0,
                uv3,
                glint_uv,
            }
        })
        .collect()
}

/// Compare extruded `item/generated` quads: front/back faces `[0..2]` match by index, while side
/// faces `[2..]` match 1-to-1 by `(pos_bits, uv0_bits)` (vanilla's `HashSet<SideFace>` order
/// varies per launch through `SideDirection`'s `Enum` identity hash), asserting every field exact.
fn assert_extruded_quads_match(
    got: &[SubmittedQuad],
    expected_quads: &[Value],
    glint_key: &str,
    label: &str,
) {
    assert_eq!(got.len(), expected_quads.len(), "{label} quad count");
    for i in 0..2 {
        let w = &expected_quads[i];
        assert_eq!(
            got[i].pos,
            parse_quad_pos_bits(&w["pos_bits"]),
            "{label} quad[{i}] pos"
        );
        assert_eq!(
            got[i].uv0,
            parse_quad_uv_bits(&w["uv0_bits"]),
            "{label} quad[{i}] uv0"
        );
        assert_eq!(
            got[i].uv3,
            w.get("uv3_bits").map(parse_quad_uv_bits),
            "{label} quad[{i}] uv3"
        );
        assert_eq!(
            got[i].glint_uv,
            parse_quad_uv_bits(&w[glint_key]),
            "{label} quad[{i}] {glint_key}"
        );
    }
    let mut witness_sides: HashMap<GeomKey, &Value> =
        HashMap::with_capacity(expected_quads.len() - 2);
    for w in &expected_quads[2..] {
        let key = (
            parse_quad_pos_bits(&w["pos_bits"]),
            parse_quad_uv_bits(&w["uv0_bits"]),
        );
        assert!(
            witness_sides.insert(key, w).is_none(),
            "{label} duplicate witness side-face key {key:#x?}"
        );
    }
    let mut seen: std::collections::HashSet<GeomKey> =
        std::collections::HashSet::with_capacity(got.len() - 2);
    for q in &got[2..] {
        let key = (q.pos, q.uv0);
        assert!(
            seen.insert(key),
            "{label} duplicate Sunrise side-face key {key:#x?}"
        );
        let w = witness_sides
            .remove(&key)
            .unwrap_or_else(|| panic!("{label} unmatched Sunrise side-face key {key:#x?}"));
        assert_eq!(
            q.pos,
            parse_quad_pos_bits(&w["pos_bits"]),
            "{label} side pos"
        );
        assert_eq!(
            q.uv0,
            parse_quad_uv_bits(&w["uv0_bits"]),
            "{label} side uv0"
        );
        assert_eq!(
            q.uv3,
            w.get("uv3_bits").map(parse_quad_uv_bits),
            "{label} side uv3"
        );
        assert_eq!(
            q.glint_uv,
            parse_quad_uv_bits(&w[glint_key]),
            "{label} side {glint_key}"
        );
    }
    assert!(
        witness_sides.is_empty(),
        "{label} {} unmatched witness side faces",
        witness_sides.len()
    );
}

/// Exact `DisplayRenderer.ItemDisplayRenderer` pose for genpvp EID 83245 (`CENTER` billboard,
/// camera at zero rotation, translation `(-0.945, 0.3885, -4.2)`, `left_rotation = (0, 1, 0, 0)`,
/// `scale = 0.525`, then `ItemDisplayRenderer`'s `rotate(Axis.YP, PI)`).
fn eid_83245_pose() -> Mat4 {
    let mut p = crate::camera::joml::Mat4::IDENTITY;
    p.rotate(crate::camera::joml::Quat::rotation_yxz(
        (-std::f64::consts::PI / 180.0) as f32 * -180.0,
        0.0,
        0.0,
    ));
    p.translate(-0.945, 0.3885, -4.2);
    p.rotate(crate::camera::joml::Quat {
        x: 0.0,
        y: 1.0,
        z: 0.0,
        w: 0.0,
    });
    p.scale(0.525, 0.525, 0.525);
    p.rotate_y(std::f32::consts::PI);
    p.to_glam()
}

/// 26.3 parity: `Glint::uv` matches `TextureTransform.setupGlintTexturing` bit-for-bit.
#[test]
#[ignore = "daily"]
fn glint_uv_matches_vanilla_26_3_texture_transform_bits() {
    let witness = glint_witness_26_3();
    let transforms = witness["transforms"].as_array().unwrap();
    assert!(!transforms.is_empty());
    for entry in transforms {
        let name = entry["name"].as_str().unwrap();
        let scale = f32::from_bits(entry["scale_bits"].as_u64().unwrap() as u32);
        let nanos = entry["nanos"].as_i64().unwrap();
        let millis = entry["millis"].as_i64().unwrap();
        assert_eq!(
            std::time::Duration::from_nanos(nanos as u64).as_millis() as i64,
            millis,
            "26.3 {name} nanos={nanos} whole-ms truncation"
        );
        let speed = f64::from_bits(
            u64::from_str_radix(entry["speed_f64_bits"].as_str().unwrap(), 16).unwrap(),
        );
        let expected_scroll = f32::from_bits(entry["scroll_ticks_bits"].as_u64().unwrap() as u32);
        assert_eq!(
            Glint::scroll_ticks(millis, speed).to_bits(),
            expected_scroll.to_bits(),
            "26.3 {name} millis={millis} speed={speed} scroll_ticks"
        );
        let glint = Glint {
            millis,
            speed,
            alpha: 0.75,
            scale,
        };
        for (idx, sample) in entry["samples"].as_array().unwrap().iter().enumerate() {
            let in_arr = sample["in_uv_bits"].as_array().unwrap();
            let in_uv = [
                f32::from_bits(in_arr[0].as_u64().unwrap() as u32),
                f32::from_bits(in_arr[1].as_u64().unwrap() as u32),
            ];
            let out_arr = sample["out_uv_bits"].as_array().unwrap();
            let want = [
                out_arr[0].as_u64().unwrap() as u32,
                out_arr[1].as_u64().unwrap() as u32,
            ];
            let got = glint.uv(in_uv).map(f32::to_bits);
            assert_eq!(
                got, want,
                "26.3 {name} millis={millis} speed={speed} sample[{idx}] in={in_uv:?}: got={got:#x?} want={want:#x?}"
            );
        }
    }
}

/// 26.3 parity: `minecraft:end_crystal` submitted into `WorldSink` (including genpvp EID 83245
/// and every `ItemDisplayContext`) uses `GLINT_TEXTURING` (8.0) over vanilla `items` atlas UVs.
#[test]
#[ignore = "daily"]
fn end_crystal_world_sink_uses_atlas_uv_and_item_glint_scale_26_3() {
    use sunrise_assets::item_model::NoEnv;

    let witness = glint_witness_26_3();
    let renderer = full_renderer_26_3();
    let sprites = dummy_hud_sprites(&["item/end_crystal"]);
    let stack = StackView::new("minecraft:end_crystal", 1);
    let eid_quads = witness["end_crystal_item_display"]["quads"]
        .as_array()
        .unwrap();
    let eid_pose = eid_83245_pose();

    let cases: [(&str, DisplayContext, Mat4); 11] = [
        ("eid_83245", DisplayContext::Gui, eid_pose),
        ("gui", DisplayContext::Gui, Mat4::IDENTITY),
        ("ground", DisplayContext::Ground, Mat4::IDENTITY),
        ("fixed", DisplayContext::Fixed, Mat4::IDENTITY),
        (
            "firstperson_righthand",
            DisplayContext::FirstPersonRightHand,
            Mat4::IDENTITY,
        ),
        (
            "firstperson_lefthand",
            DisplayContext::FirstPersonLeftHand,
            Mat4::IDENTITY,
        ),
        (
            "thirdperson_righthand",
            DisplayContext::ThirdPersonRightHand,
            Mat4::IDENTITY,
        ),
        (
            "thirdperson_lefthand",
            DisplayContext::ThirdPersonLeftHand,
            Mat4::IDENTITY,
        ),
        ("head", DisplayContext::Head, Mat4::IDENTITY),
        ("on_shelf", DisplayContext::OnShelf, Mat4::IDENTITY),
        ("none", DisplayContext::None, Mat4::IDENTITY),
    ];

    for (label, ctx, pose) in cases {
        let expected_quads = if label == "eid_83245" {
            eid_quads
        } else {
            assert_eq!(
                witness["contexts"][label]["matches_eid_83245_all_quads"].as_bool(),
                Some(true)
            );
            witness["contexts"][label]["quads"].as_array().unwrap()
        };
        let state = renderer.resolve(&stack, ctx, None, None, 83245, &NoEnv);
        for (millis, key) in [
            (0_i64, "glint_uv_t0_bits"),
            (123_456_i64, "glint_uv_t123456_bits"),
        ] {
            let mut out = Vec::new();
            let mut sink = PairedWorldSink {
                world: WorldSink {
                    out: &mut out,
                    light: 0xF0F0,
                    lights: None,
                    glint: Some(Glint {
                        millis,
                        speed: 0.5,
                        alpha: 0.75,
                        scale: 8.0,
                    }),
                    tier: 0,
                },
                quads: Vec::new(),
            };
            state.submit(&renderer, &sprites, &NoHost, pose, &mut sink);
            let PairedWorldSink { quads, .. } = sink;
            assert_eq!(
                out.len(),
                expected_quads.len() * 4,
                "{label} {key} vertex count"
            );
            assert!(
                out.iter().all(|v| (v.layer & 0xFFFF) == 7),
                "{label} {key} expected glint overlay 7"
            );
            let got = submitted_quads(&quads, &out, None);
            assert_extruded_quads_match(
                &got,
                expected_quads,
                key,
                &format!("26.3 end_crystal {label} {key}"),
            );
        }
    }
}

/// 26.3 parity: block-textured enchanted items (`minecraft:stone`) use the stitched
/// `minecraft:blocks` (mip 4) atlas UVs with `GLINT_TEXTURING` (8.0) in both world and GUI.
#[test]
#[ignore = "daily"]
fn block_item_glint_uses_blocks_atlas_uv_26_3() {
    use sunrise_assets::item_model::NoEnv;

    let witness = glint_witness_26_3();
    let renderer = full_renderer_26_3();
    let sprites = dummy_hud_sprites(&["block/stone"]);
    let mut stack = StackView::new("minecraft:stone", 1);
    stack.patch.insert(
        "minecraft:enchantment_glint_override".into(),
        Some(Value::Bool(true)),
    );

    for (ctx_name, ctx) in [
        ("gui", DisplayContext::Gui),
        ("ground", DisplayContext::Ground),
    ] {
        let expected_quads = witness["block_item"][ctx_name]["quads"].as_array().unwrap();
        let state = renderer.resolve(&stack, ctx, None, None, 0, &NoEnv);
        for (millis, key) in [
            (0_i64, "glint_uv_t0_bits"),
            (123_456_i64, "glint_uv_t123456_bits"),
        ] {
            let mut out = Vec::new();
            state.submit(
                &renderer,
                &sprites,
                &NoHost,
                Mat4::IDENTITY,
                &mut WorldSink {
                    out: &mut out,
                    light: 0xF0F0,
                    lights: None,
                    glint: Some(Glint {
                        millis,
                        speed: 0.5,
                        alpha: 0.75,
                        scale: 8.0,
                    }),
                    tier: 0,
                },
            );
            assert_eq!(
                out.len(),
                expected_quads.len() * 4,
                "stone {ctx_name} {key}"
            );
            let got = vertex_quad_glint_bits(&out);
            for (q, want_quad) in expected_quads.iter().enumerate() {
                let want = parse_quad_uv_bits(&want_quad[key]);
                assert_eq!(
                    got[q], want,
                    "26.3 stone {ctx_name} {key} quad[{q}]: got={:#x?} want={:#x?}",
                    got[q], want
                );
            }
        }
    }

    // GUI draw_posed (no slot clipping) must match each emitted face's exact corner glint UVs.
    let state = renderer.resolve(&stack, DisplayContext::Gui, None, None, 0, &NoEnv);
    let mut batch = crate::hud::HudBatch::scratch([64, 64]);
    let pose = Mat4::from_translation(Vec3::new(8.0, 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    assert!(gui::draw_posed(
        &state,
        &gui::GuiItemEnv {
            renderer: &renderer,
            sprites: &sprites,
            host: &NoHost,
        },
        &mut batch,
        pose,
        GlintFrame {
            millis: 123_456,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        },
    ));
    let expected_gui_quads = witness["block_item"]["gui_slot_pose"]["quads"]
        .as_array()
        .unwrap();
    let gui_glint_verts: Vec<&crate::hud::HudVertex> =
        batch.vertices.iter().filter(|v| v.uv[0] >= 2.0).collect();
    // 3 visible cube faces * 2 triangles * 3 vertices = 18 glint vertices.
    assert_eq!(
        gui_glint_verts.len(),
        18,
        "stone gui_draw_posed should emit 3 visible faces (18 glint vertices)"
    );
    let mut matched_faces = std::collections::HashSet::new();
    for tri in gui_glint_verts.as_chunks::<3>().0 {
        let mut face_match = None;
        for (fi, wq) in expected_gui_quads.iter().enumerate() {
            let wpos = parse_quad_pos_bits(&wq["pos_bits"]);
            let wuv = parse_quad_uv_bits(&wq["glint_uv_t123456_bits"]);
            let all_corners_match = tri.iter().all(|v| {
                let xy = v.pos.map(f32::to_bits);
                let guv = v.glint_uv.map(f32::to_bits);
                (0..4).any(|k| [wpos[k][0], wpos[k][1]] == xy && wuv[k] == guv)
            });
            if all_corners_match {
                face_match = Some(fi);
                break;
            }
        }
        let fi = face_match.unwrap_or_else(|| {
            panic!(
                "stone gui glint triangle did not match any vanilla face's exact corner UVs: {:?}",
                tri.iter()
                    .map(|v| (v.pos.map(f32::to_bits), v.glint_uv.map(f32::to_bits)))
                    .collect::<Vec<_>>()
            )
        });
        matched_faces.insert(fi);
    }
    assert_eq!(
        matched_faces.len(),
        3,
        "stone gui glint must cover all 3 visible vanilla faces"
    );
}

/// 26.3 parity: `Foil::Special` (`minecraft:compass`) projects sheeted decal `UV3` via
/// `ItemFeatureRenderer.computeFoilDecalPose` and transforms it with `GLINT_TEXTURING` (8.0).
#[test]
#[ignore = "daily"]
fn compass_special_foil_uses_decal_uv_26_3() {
    use sunrise_assets::item_model::NoEnv;

    let witness = glint_witness_26_3();
    let renderer = full_renderer_26_3();
    let sprites = dummy_hud_sprites(&["item/compass_16", "item/compass_00"]);
    let mut stack = StackView::new("minecraft:compass", 1);
    stack.patch.insert(
        "minecraft:enchantment_glint_override".into(),
        Some(Value::Bool(true)),
    );

    let gui_slot_pose = Mat4::from_translation(Vec3::new(8.0, 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    let gui_slot_scale_2 = Mat4::from_translation(Vec3::new(16.0, 16.0, 0.0))
        * Mat4::from_scale(Vec3::new(32.0, -32.0, 32.0));
    let gui_slot_scale_3 = Mat4::from_translation(Vec3::new(24.0, 24.0, 0.0))
        * Mat4::from_scale(Vec3::new(48.0, -48.0, 48.0));
    let gui_slot_16_0_scale_1 = Mat4::from_translation(Vec3::new(256.0 + 8.0, 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    let gui_slot_3_2_scale_2 = Mat4::from_translation(Vec3::new(96.0 + 16.0, 64.0 + 16.0, 0.0))
        * Mat4::from_scale(Vec3::new(32.0, -32.0, 32.0));
    // Match `VanillaItemGlintOracle.java`'s JOML `PoseStack` (`Quaternionf().rotationY(0.7F)`).
    let mut gp = crate::camera::joml::Mat4::IDENTITY;
    gp.translate(1.5, 2.0, -3.5);
    gp.rotate(crate::camera::joml::Quat::rotation_y(0.7));
    gp.scale(0.5, 0.5, 0.5);
    let ground_posed = gp.to_glam();

    let cases: [(&str, DisplayContext, Mat4); 13] = [
        ("gui", DisplayContext::Gui, Mat4::IDENTITY),
        (
            "firstperson_righthand",
            DisplayContext::FirstPersonRightHand,
            Mat4::IDENTITY,
        ),
        (
            "firstperson_lefthand",
            DisplayContext::FirstPersonLeftHand,
            Mat4::IDENTITY,
        ),
        ("ground", DisplayContext::Ground, Mat4::IDENTITY),
        ("fixed", DisplayContext::Fixed, Mat4::IDENTITY),
        (
            "thirdperson_righthand",
            DisplayContext::ThirdPersonRightHand,
            Mat4::IDENTITY,
        ),
        ("head", DisplayContext::Head, Mat4::IDENTITY),
        ("gui_slot_pose", DisplayContext::Gui, gui_slot_pose),
        ("gui_slot_scale_2", DisplayContext::Gui, gui_slot_scale_2),
        ("gui_slot_scale_3", DisplayContext::Gui, gui_slot_scale_3),
        (
            "gui_slot_16_0_scale_1",
            DisplayContext::Gui,
            gui_slot_16_0_scale_1,
        ),
        (
            "gui_slot_3_2_scale_2",
            DisplayContext::Gui,
            gui_slot_3_2_scale_2,
        ),
        ("ground_posed", DisplayContext::Ground, ground_posed),
    ];

    let compass_rect = renderer
        .glint_atlas
        .lock()
        .unwrap()
        .get_stitched("item/compass_16")
        .unwrap();
    for (case_key, ctx, pose) in cases {
        let expected_quads = witness["special_foil"][case_key]["quads"]
            .as_array()
            .unwrap();
        let state = renderer.resolve(&stack, ctx, None, None, 0, &NoEnv);
        for (millis, key) in [
            (0_i64, "glint_uv_t0_bits"),
            (123_456_i64, "glint_uv_t123456_bits"),
        ] {
            let mut out = Vec::new();
            let mut sink = PairedWorldSink {
                world: WorldSink {
                    out: &mut out,
                    light: 0xF0F0,
                    lights: None,
                    glint: Some(Glint {
                        millis,
                        speed: 0.5,
                        alpha: 0.75,
                        scale: 8.0,
                    }),
                    tier: 0,
                },
                quads: Vec::new(),
            };
            state.submit(&renderer, &sprites, &NoHost, pose, &mut sink);
            let PairedWorldSink { quads, .. } = sink;
            assert_eq!(
                out.len(),
                expected_quads.len() * 4,
                "compass {case_key} {key} vertex count"
            );
            let got = submitted_quads(&quads, &out, Some(compass_rect));
            assert_extruded_quads_match(
                &got,
                expected_quads,
                key,
                &format!("26.3 compass {case_key} {key}"),
            );
        }
    }

    // Flattened compass at [1, 1, 0]: non-finite UV3 / glint UV matching vanilla (no zero fallback, no panic).
    let flat_pose = Mat4::from_scale(Vec3::new(1.0, 1.0, 0.0));
    for (case_key, ctx) in [
        ("flattened_gui", DisplayContext::Gui),
        ("flattened_ground", DisplayContext::Ground),
    ] {
        let expected_quads = witness["special_foil"][case_key]["quads"]
            .as_array()
            .unwrap();
        let state = renderer.resolve(&stack, ctx, None, None, 0, &NoEnv);
        let mut out = Vec::new();
        state.submit(
            &renderer,
            &sprites,
            &NoHost,
            flat_pose,
            &mut WorldSink {
                out: &mut out,
                light: 0xF0F0,
                lights: None,
                glint: Some(Glint {
                    millis: 123_456,
                    speed: 0.5,
                    alpha: 0.75,
                    scale: 8.0,
                }),
                tier: 0,
            },
        );
        assert_eq!(out.len(), expected_quads.len() * 4, "{case_key} count");
        assert!(
            expected_quads.iter().all(|w| {
                parse_quad_uv_bits(&w["uv3_bits"])
                    .iter()
                    .all(|&[u, v]| !f32::from_bits(u).is_finite() && !f32::from_bits(v).is_finite())
            }),
            "witness {case_key} expected non-finite UV3"
        );
        assert!(
            out.iter()
                .all(|v| !v.glint_uv[0].is_finite() && !v.glint_uv[1].is_finite()),
            "{case_key} must emit non-finite glint UVs"
        );
    }

    // All 6 face directions (Down, Up, North, South, West, East) via stone's 6 faces under
    // `FoilType::Special`, plus `[1e-21, 1, 1]` normal overflow selecting North for East/West.
    let mut stone_stack = StackView::new("minecraft:stone", 1);
    stone_stack.patch.insert(
        "minecraft:enchantment_glint_override".into(),
        Some(Value::Bool(true)),
    );
    let stone_sprites = dummy_hud_sprites(&["block/stone"]);
    for (case_key, ctx, pose) in [
        ("stone_six_dirs_gui", DisplayContext::Gui, Mat4::IDENTITY),
        (
            "stone_six_dirs_ground",
            DisplayContext::Ground,
            Mat4::IDENTITY,
        ),
        (
            "stone_scale_1e21_ground",
            DisplayContext::Ground,
            Mat4::from_scale(Vec3::new(1e-21, 1.0, 1.0)),
        ),
    ] {
        let expected_quads = witness["special_foil"][case_key]["quads"]
            .as_array()
            .unwrap();
        assert_eq!(expected_quads.len(), 6, "{case_key} all 6 faces");
        let mut state = renderer.resolve(&stone_stack, ctx, None, None, 0, &NoEnv);
        for layer in &mut state.layers {
            layer.foil = FoilType::Special;
        }
        let mut out = Vec::new();
        let mut sink = PairedWorldSink {
            world: WorldSink {
                out: &mut out,
                light: 0xF0F0,
                lights: None,
                glint: Some(Glint {
                    millis: 123_456,
                    speed: 0.5,
                    alpha: 0.75,
                    scale: 8.0,
                }),
                tier: 0,
            },
            quads: Vec::new(),
        };
        state.submit(&renderer, &stone_sprites, &NoHost, pose, &mut sink);
        let PairedWorldSink { quads, .. } = sink;
        assert_eq!(quads.len(), 6);
        let got_glint = vertex_quad_glint_bits(&out);
        for (q, want_quad) in expected_quads.iter().enumerate() {
            let want_uv3 = parse_quad_uv_bits(&want_quad["uv3_bits"]);
            let got_uv3 = quads[q].gui_glint_uv.unwrap().map(|p| p.map(f32::to_bits));
            assert_eq!(
                got_uv3, want_uv3,
                "26.3 {case_key} face[{q}] ({}) uv3",
                want_quad["facing"]
            );
            let want_glint = parse_quad_uv_bits(&want_quad["glint_uv_t123456_bits"]);
            assert_eq!(
                got_glint[q], want_glint,
                "26.3 {case_key} face[{q}] ({}) glint_uv",
                want_quad["facing"]
            );
        }
    }

    // Physical `GuiItemAtlas` / `DynamicAtlasAllocator` slot assignment parity across scales 1, 2, 3.
    let gui_atlas = &witness["gui_item_atlas"];
    let pair = |v: &Value| {
        let a = v.as_array().unwrap();
        (a[0].as_u64().unwrap() as u32, a[1].as_u64().unwrap() as u32)
    };
    for gui_scale in [1_u32, 2, 3] {
        let want = &gui_atlas["allocator"][format!("scale_{gui_scale}")];
        let scale_f = gui_scale as f32;
        let statics: Vec<ItemRenderState> = (0..16)
            .map(|_| {
                state(
                    "minecraft:item/compass_16",
                    FoilType::None,
                    DisplayContext::Gui,
                )
            })
            .collect();
        for st in &statics {
            renderer.gui_item_slot(st, scale_f);
        }
        let foil_a = state(
            "minecraft:item/compass_16",
            FoilType::Special,
            DisplayContext::Gui,
        );
        let foil_b = state(
            "minecraft:item/compass_00",
            FoilType::Special,
            DisplayContext::Gui,
        );
        assert_eq!(
            renderer.gui_item_slot(&foil_a, scale_f),
            pair(&want["foil_a"]),
            "scale {gui_scale} foil_a slot"
        );
        assert_eq!(
            renderer.gui_item_slot(&foil_a, scale_f),
            pair(&want["foil_a_reuse"]),
            "scale {gui_scale} foil_a reuse slot"
        );
        renderer.end_gui_frame();
        assert_eq!(
            renderer.gui_item_slot(&statics[0], scale_f),
            pair(&want["static_0_after_end_frame"]),
            "scale {gui_scale} static_0 stays allocated across frames"
        );
        assert_eq!(
            renderer.gui_item_slot(&foil_b, scale_f),
            pair(&want["foil_b_after_end_frame"]),
            "scale {gui_scale} foil_b reuses freed animated slot"
        );
    }

    // Oracle-backed tiny vertex offset (`2^-20`) rounding parity in slot `(0, 0)` vs `(16, 0)`.
    let frame = GlintFrame {
        millis: 123_456,
        speed: 0.5,
        alpha: 0.75,
        sprite: true,
    };
    let make_tiny_quad = |offset: f32| {
        let mut st = state(
            "minecraft:item/compass_16",
            FoilType::Special,
            DisplayContext::Gui,
        );
        let LayerGeometry::Quads(baked) = &mut st.layers[0].geometry else {
            unreachable!();
        };
        let mut b = (**baked).clone();
        b.quads[0].positions = [
            [offset, 0.25, 0.0],
            [offset, -0.25, 0.0],
            [0.25, -0.25, 0.0],
            [0.25, 0.25, 0.0],
        ];
        *baked = Arc::new(b);
        st
    };
    let assert_quad_gui_glint_bits = |batch: &crate::hud::HudBatch,
                                      corner_xy: [[f32; 2]; 4],
                                      want_bits: [[u32; 2]; 4],
                                      label: &str| {
        let verts: Vec<_> = batch.vertices.iter().filter(|v| v.uv[0] >= 2.0).collect();
        assert!(!verts.is_empty(), "{label}: no GUI glint vertices");
        let mut seen = [false; 4];
        for v in verts {
            let k = (0..4)
                .find(|&i| {
                    (v.pos[0] - corner_xy[i][0]).abs() <= 1e-3
                        && (v.pos[1] - corner_xy[i][1]).abs() <= 1e-3
                })
                .unwrap_or_else(|| panic!("{label}: unexpected vertex pos {:?}", v.pos));
            assert_eq!(
                v.glint_uv.map(f32::to_bits),
                want_bits[k],
                "{label}: corner {k} glint UV bits mismatch"
            );
            seen[k] = true;
        }
        assert_eq!(seen, [true; 4], "{label}: all 4 corners must be emitted");
    };
    let eps = 2.0_f32.powi(-20);
    let tiny_corners = [[8.0, 4.0], [8.0, 12.0], [12.0, 12.0], [12.0, 4.0]];
    let tor = &gui_atlas["tiny_offset_rounding"];
    // Switching back to scale 1.0 resets the atlas to empty (slot `(0, 0)`).
    for (offset, key) in [(eps, "slot_0_0_eps"), (0.0, "slot_0_0_zero")] {
        let st = make_tiny_quad(offset);
        let mut batch = crate::hud::HudBatch::scratch([1024, 1024]);
        batch.scale = 1.0;
        gui::draw_with_frame(
            &st, &renderer, &sprites, &NoHost, &mut batch, 0.0, 0.0, frame,
        );
        assert_quad_gui_glint_bits(
            &batch,
            tiny_corners,
            parse_quad_uv_bits(&tor[key]["glint_uv_t123456_bits"]),
            key,
        );
        renderer.end_gui_frame();
    }
    // Occupy slots `(0, 0)..(15, 0)` at scale 1 with 16 static items -> next item gets `(16, 0)`.
    let mut fill = crate::hud::HudBatch::scratch([1024, 1024]);
    fill.scale = 1.0;
    let statics16: Vec<ItemRenderState> = (0..16)
        .map(|_| {
            state(
                "minecraft:item/compass_16",
                FoilType::None,
                DisplayContext::Gui,
            )
        })
        .collect();
    for st in &statics16 {
        gui::draw_with_frame(st, &renderer, &sprites, &NoHost, &mut fill, 0.0, 0.0, frame);
    }
    for (offset, key) in [(eps, "slot_16_0_eps"), (0.0, "slot_16_0_zero")] {
        let st = make_tiny_quad(offset);
        let mut batch = crate::hud::HudBatch::scratch([1024, 1024]);
        batch.scale = 1.0;
        gui::draw_with_frame(
            &st, &renderer, &sprites, &NoHost, &mut batch, 0.0, 0.0, frame,
        );
        assert_quad_gui_glint_bits(
            &batch,
            tiny_corners,
            parse_quad_uv_bits(&tor[key]["glint_uv_t123456_bits"]),
            key,
        );
        renderer.end_gui_frame();
    }

    // Real `minecraft:compass` drawn via `gui::draw_with_frame` in physical slots `(16, 0)` (scale 1)
    // and `(3, 2)` (scale 2, after 35 static items).
    let compass_gui = renderer.resolve(&stack, DisplayContext::Gui, None, None, 0, &NoEnv);
    let compass_corners = [[0.0, 0.0], [0.0, 16.0], [16.0, 16.0], [16.0, 0.0]];
    {
        let mut batch = crate::hud::HudBatch::scratch([1024, 1024]);
        batch.scale = 1.0;
        gui::draw_with_frame(
            &compass_gui,
            &renderer,
            &sprites,
            &NoHost,
            &mut batch,
            0.0,
            0.0,
            frame,
        );
        let want = parse_quad_uv_bits(
            &witness["special_foil"]["gui_slot_16_0_scale_1"]["quads"][0]["glint_uv_t123456_bits"],
        );
        assert_quad_gui_glint_bits(
            &batch,
            compass_corners,
            want,
            "compass gui::draw_with_frame slot (16, 0) scale 1",
        );
        renderer.end_gui_frame();
    }
    {
        let mut fill2 = crate::hud::HudBatch::scratch([1024, 1024]);
        fill2.scale = 2.0;
        let statics35: Vec<ItemRenderState> = (0..35)
            .map(|_| {
                state(
                    "minecraft:item/compass_16",
                    FoilType::None,
                    DisplayContext::Gui,
                )
            })
            .collect();
        for st in &statics35 {
            gui::draw_with_frame(
                st, &renderer, &sprites, &NoHost, &mut fill2, 0.0, 0.0, frame,
            );
        }
        let mut batch = crate::hud::HudBatch::scratch([1024, 1024]);
        batch.scale = 2.0;
        gui::draw_with_frame(
            &compass_gui,
            &renderer,
            &sprites,
            &NoHost,
            &mut batch,
            0.0,
            0.0,
            frame,
        );
        let want = parse_quad_uv_bits(
            &witness["special_foil"]["gui_slot_3_2_scale_2"]["quads"][0]["glint_uv_t123456_bits"],
        );
        assert_quad_gui_glint_bits(
            &batch,
            compass_corners,
            want,
            "compass gui::draw_with_frame slot (3, 2) scale 2",
        );
        renderer.end_gui_frame();
    }
}

/// 26.3 parity: `minecraft:trident` uses `GLINT_TEXTURING` (8.0) + `items` atlas in 2D contexts
/// (`Gui`/`Ground`) and `ENTITY_GLINT_TEXTURING` (0.5) + raw UV in 3D hand contexts;
/// `minecraft:shield` (plain & patterned) uses `ENTITY_GLINT_TEXTURING` (0.5) +
/// `minecraft:shield_patterns` atlas UV in both `WorldSink` and GUI.
#[test]
#[ignore = "daily"]
fn special_model_trident_and_shield_glint_uses_entity_scale_26_3() {
    use sunrise_assets::item_model::NoEnv;

    let witness = glint_witness_26_3();
    let renderer = full_renderer_26_3();
    let sprites = dummy_hud_sprites(&[
        "item/trident",
        "entity/trident/trident",
        "entity/shield/shield_base_nopattern",
        "entity/shield/shield_base",
        "entity/shield/base",
    ]);

    let enchanted = |item: &str| {
        let mut s = StackView::new(item, 1);
        s.patch.insert(
            "minecraft:enchantment_glint_override".into(),
            Some(Value::Bool(true)),
        );
        s
    };

    let trident = enchanted("minecraft:trident");
    for (ctx_name, ctx) in [
        ("gui", DisplayContext::Gui),
        ("ground", DisplayContext::Ground),
    ] {
        let expected_quads = witness["special_models"]["trident"][ctx_name]["items"]["quads"]
            .as_array()
            .unwrap();
        let state = renderer.resolve(&trident, ctx, None, None, 0, &NoEnv);
        let mut out = Vec::new();
        let mut sink = PairedWorldSink {
            world: WorldSink {
                out: &mut out,
                light: 0xF0F0,
                lights: None,
                glint: Some(Glint {
                    millis: 123_456,
                    speed: 0.5,
                    alpha: 0.75,
                    scale: 8.0,
                }),
                tier: 0,
            },
            quads: Vec::new(),
        };
        state.submit(&renderer, &sprites, &NoHost, Mat4::IDENTITY, &mut sink);
        let PairedWorldSink { quads, .. } = sink;
        assert_eq!(out.len(), expected_quads.len() * 4, "trident 2d {ctx_name}");
        let got = submitted_quads(&quads, &out, None);
        assert_extruded_quads_match(
            &got,
            expected_quads,
            "glint_uv_t123456_bits",
            &format!("26.3 trident 2d {ctx_name}"),
        );
    }

    // Special-model child order differs from vanilla HashMap order, so compare sorted quads.
    for (ctx_name, ctx) in [
        (
            "firstperson_righthand",
            DisplayContext::FirstPersonRightHand,
        ),
        (
            "thirdperson_righthand",
            DisplayContext::ThirdPersonRightHand,
        ),
    ] {
        let expected_quads = witness["special_models"]["trident"][ctx_name]["models"][0]["quads"]
            .as_array()
            .unwrap();
        let mut want: Vec<[[u32; 2]; 4]> = expected_quads
            .iter()
            .map(|q| parse_quad_uv_bits(&q["glint_uv_t123456_bits"]))
            .collect();
        want.sort();
        let state = renderer.resolve(&trident, ctx, None, None, 0, &NoEnv);
        let mut out = Vec::new();
        state.submit(
            &renderer,
            &sprites,
            &NoHost,
            Mat4::IDENTITY,
            &mut WorldSink {
                out: &mut out,
                light: 0xF0F0,
                lights: None,
                glint: Some(Glint {
                    millis: 123_456,
                    speed: 0.5,
                    alpha: 0.75,
                    scale: 8.0,
                }),
                tier: 0,
            },
        );
        let mut got = vertex_quad_glint_bits(&out);
        got.sort();
        assert_eq!(got, want, "26.3 trident 3d {ctx_name}");
    }

    let shield_plain = enchanted("minecraft:shield");
    for (ctx_name, ctx) in [
        ("gui", DisplayContext::Gui),
        ("ground", DisplayContext::Ground),
    ] {
        let expected_quads =
            witness["special_models"]["shield_plain"][ctx_name]["models"][0]["quads"]
                .as_array()
                .unwrap();
        let mut want: Vec<[[u32; 2]; 4]> = expected_quads
            .iter()
            .map(|q| parse_quad_uv_bits(&q["glint_uv_t123456_bits"]))
            .collect();
        want.sort();
        let state = renderer.resolve(&shield_plain, ctx, None, None, 0, &NoEnv);
        let mut out = Vec::new();
        state.submit(
            &renderer,
            &sprites,
            &NoHost,
            Mat4::IDENTITY,
            &mut WorldSink {
                out: &mut out,
                light: 0xF0F0,
                lights: None,
                glint: Some(Glint {
                    millis: 123_456,
                    speed: 0.5,
                    alpha: 0.75,
                    scale: 8.0,
                }),
                tier: 0,
            },
        );
        let mut got = vertex_quad_glint_bits(&out);
        got.sort();
        assert_eq!(got, want, "26.3 shield_plain world {ctx_name}");
    }

    // GUI shields use ENTITY_GLINT_TEXTURING (0.5) over shield_patterns, never item scale 8.0.
    let state = renderer.resolve(&shield_plain, DisplayContext::Gui, None, None, 0, &NoEnv);
    let mut batch = crate::hud::HudBatch::scratch([64, 64]);
    let pose = Mat4::from_translation(Vec3::new(8.0, 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    assert!(gui::draw_posed(
        &state,
        &gui::GuiItemEnv {
            renderer: &renderer,
            sprites: &sprites,
            host: &NoHost,
        },
        &mut batch,
        pose,
        GlintFrame {
            millis: 123_456,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        },
    ));
    // Check every emitted shield GUI glint triangle against exact zero-tolerance UV bits
    // (including exact split vertices where `depth::sort` cuts plate faces against the handle)
    // and full face/corner/area coverage across every expected visible face.
    let expected_gui_quads = witness["special_models"]["shield_plain"]["gui_slot_pose"]["models"]
        [0]["quads"]
        .as_array()
        .unwrap();
    let entity_glint_cols: [f32; 16] = witness["transforms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| {
            t["name"].as_str() == Some("entity_glint_texturing")
                && t["millis"].as_i64() == Some(123_456)
                && t["speed"].as_f64() == Some(0.5)
        })
        .expect("entity_glint_texturing matrix at t=123456 speed=0.5")["matrix_cols_bits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| f32::from_bits(v.as_u64().unwrap() as u32))
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let xform_entity_glint_uv_bits = |uv: [f32; 2]| -> [u32; 2] {
        // JOML `Matrix4f.transform(Vector4f(u, v, 0, 1))` right-to-left unfused sums.
        let x =
            entity_glint_cols[0] * uv[0] + (entity_glint_cols[4] * uv[1] + entity_glint_cols[12]);
        let y =
            entity_glint_cols[1] * uv[0] + (entity_glint_cols[5] * uv[1] + entity_glint_cols[13]);
        [x.to_bits(), y.to_bits()]
    };
    let parsed_quads: Vec<([Vec3; 4], [[f32; 2]; 4], [[u32; 2]; 4])> = expected_gui_quads
        .iter()
        .map(|q| {
            let pb = parse_quad_pos_bits(&q["pos_bits"]);
            let pos = std::array::from_fn(|i| {
                Vec3::new(
                    f32::from_bits(pb[i][0]),
                    f32::from_bits(pb[i][1]),
                    f32::from_bits(pb[i][2]),
                )
            });
            let uv0b = parse_quad_uv_bits(&q["uv0_bits"]);
            let uv0 =
                std::array::from_fn(|i| [f32::from_bits(uv0b[i][0]), f32::from_bits(uv0b[i][1])]);
            let glint = parse_quad_uv_bits(&q["glint_uv_t123456_bits"]);
            (pos, uv0, glint)
        })
        .collect();
    // Handle top plane (`quad[6]`) splits the straddling front and right plate faces (`quad[3]`, `quad[4]`).
    let (p6, _, _) = &parsed_quads[6];
    let cut_n = (p6[1] - p6[0]).cross(p6[2] - p6[0]);
    let cut_d = cut_n.dot(p6[0]);
    let quad_area_2d = |p: &[Vec3; 4]| -> f32 {
        let tri_area = |a: Vec3, b: Vec3, c: Vec3| {
            0.5 * ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs()
        };
        tri_area(p[0], p[1], p[2]) + tri_area(p[0], p[2], p[3])
    };
    struct ExpectedShieldFace {
        quad_index: usize,
        /// Indices `0..4` are original corners; `4..` are exact split vertices on cut edges.
        verts: Vec<([f32; 2], [u32; 2])>,
        expected_tris: usize,
        expected_area: f32,
        matched_verts: Vec<bool>,
        matched_tris: usize,
        matched_area: f32,
    }
    let mut visible_faces: Vec<ExpectedShieldFace> = Vec::new();
    for (qi, (pos, uv0, glint_bits)) in parsed_quads.iter().enumerate() {
        let face_n = (pos[1] - pos[0]).cross(pos[2] - pos[0]);
        if face_n.z >= 0.0 {
            continue;
        }
        let mut verts: Vec<([f32; 2], [u32; 2])> = (0..4)
            .map(|k| {
                assert_eq!(
                    xform_entity_glint_uv_bits(uv0[k]),
                    glint_bits[k],
                    "oracle transform matrix must reproduce quad[{qi}] corner[{k}] exact bits"
                );
                ([pos[k].x, pos[k].y], glint_bits[k])
            })
            .collect();
        let ds: [f32; 4] = std::array::from_fn(|k| cut_n.dot(pos[k]) - cut_d);
        let splits = ds.iter().any(|&d| d > 1e-4) && ds.iter().any(|&d| d < -1e-4);
        if splits {
            for a in 0..4 {
                let b = (a + 1) % 4;
                if (ds[a] > 1e-4 && ds[b] < -1e-4) || (ds[a] < -1e-4 && ds[b] > 1e-4) {
                    let t = ds[a] / (ds[a] - ds[b]);
                    let sp = pos[a] + (pos[b] - pos[a]) * t;
                    let suv = [
                        uv0[a][0] + (uv0[b][0] - uv0[a][0]) * t,
                        uv0[a][1] + (uv0[b][1] - uv0[a][1]) * t,
                    ];
                    verts.push(([sp.x, sp.y], xform_entity_glint_uv_bits(suv)));
                }
            }
            assert_eq!(
                verts.len(),
                6,
                "split shield face quad[{qi}] must have 4 corners + 2 split vertices"
            );
        }
        let count = verts.len();
        visible_faces.push(ExpectedShieldFace {
            quad_index: qi,
            verts,
            expected_tris: if splits { 4 } else { 2 },
            expected_area: quad_area_2d(pos),
            matched_verts: vec![false; count],
            matched_tris: 0,
            matched_area: 0.0,
        });
    }
    let visible_indices: Vec<usize> = visible_faces.iter().map(|f| f.quad_index).collect();
    assert_eq!(
        visible_indices,
        [0, 3, 4, 6, 9, 10],
        "26.3 shield_plain gui must have exactly the 6 front-facing faces"
    );

    let gui_glint_verts: Vec<&crate::hud::HudVertex> =
        batch.vertices.iter().filter(|v| v.uv[0] >= 2.0).collect();
    let expected_total_tris: usize = visible_faces.iter().map(|f| f.expected_tris).sum();
    assert_eq!(
        gui_glint_verts.len(),
        expected_total_tris * 3,
        "26.3 shield_plain gui must emit exact glint vertex count across all 6 visible faces"
    );
    let vert_index_in_face =
        |face: &ExpectedShieldFace, v: &crate::hud::HudVertex| -> Option<usize> {
            let guv_bits = v.glint_uv.map(f32::to_bits);
            face.verts.iter().position(|&(xy, uv_bits)| {
                (v.pos[0] - xy[0]).abs() <= 1e-4
                    && (v.pos[1] - xy[1]).abs() <= 1e-4
                    && guv_bits == uv_bits
            })
        };
    for tri in gui_glint_verts.as_chunks::<3>().0 {
        let mut matched_face: Option<(usize, [usize; 3])> = None;
        for (fi, face) in visible_faces.iter().enumerate() {
            if let (Some(i0), Some(i1), Some(i2)) = (
                vert_index_in_face(face, tri[0]),
                vert_index_in_face(face, tri[1]),
                vert_index_in_face(face, tri[2]),
            ) {
                assert!(
                    matched_face.is_none(),
                    "shield gui glint triangle ambiguously matched multiple faces"
                );
                matched_face = Some((fi, [i0, i1, i2]));
            }
        }
        let (fi, idxs) = matched_face.unwrap_or_else(|| {
            panic!(
                "26.3 shield_plain gui glint triangle did not match any visible vanilla face with exact UV bits: {:?}",
                tri.iter()
                    .map(|v| (v.pos, v.glint_uv.map(f32::to_bits)))
                    .collect::<Vec<_>>()
            )
        });
        let face = &mut visible_faces[fi];
        face.matched_tris += 1;
        let area = 0.5
            * ((tri[1].pos[0] - tri[0].pos[0]) * (tri[2].pos[1] - tri[0].pos[1])
                - (tri[1].pos[1] - tri[0].pos[1]) * (tri[2].pos[0] - tri[0].pos[0]))
                .abs();
        face.matched_area += area;
        for idx in idxs {
            face.matched_verts[idx] = true;
        }
    }
    for face in &visible_faces {
        assert_eq!(
            face.matched_tris, face.expected_tris,
            "26.3 shield_plain gui face quad[{}] triangle count",
            face.quad_index
        );
        assert!(
            face.matched_verts.iter().all(|&m| m),
            "26.3 shield_plain gui face quad[{}] missed corner or split vertices: {:?}",
            face.quad_index,
            face.matched_verts
        );
        assert!(
            (face.matched_area - face.expected_area).abs() <= 1e-3,
            "26.3 shield_plain gui face quad[{}] area {} != expected {}",
            face.quad_index,
            face.matched_area,
            face.expected_area
        );
    }

    let mut shield_red = enchanted("minecraft:shield");
    shield_red.patch.insert(
        "minecraft:base_color".into(),
        Some(Value::String("red".into())),
    );
    let patterned_glint_quads =
        witness["special_models"]["shield_red"]["ground"]["models"][2]["quads"]
            .as_array()
            .unwrap();
    let mut want_single: Vec<[[u32; 2]; 4]> = patterned_glint_quads
        .iter()
        .map(|q| parse_quad_uv_bits(&q["glint_uv_t123456_bits"]))
        .collect();
    want_single.sort();
    let state = renderer.resolve(&shield_red, DisplayContext::Ground, None, None, 0, &NoEnv);
    let mut out = Vec::new();
    state.submit(
        &renderer,
        &sprites,
        &NoHost,
        Mat4::IDENTITY,
        &mut WorldSink {
            out: &mut out,
            light: 0xF0F0,
            lights: None,
            glint: Some(Glint {
                millis: 123_456,
                speed: 0.5,
                alpha: 0.75,
                scale: 8.0,
            }),
            tier: 0,
        },
    );
    // Base draws opaque (`overlay == 0`), pattern layers use `BANNER_PATTERN_OVERLAY`
    // (`RenderTypes.bannerPattern`), and one `GLINT_ONLY_OVERLAY` pass follows.
    let shield_red_models = witness["special_models"]["shield_red"]["ground"]["models"]
        .as_array()
        .unwrap();
    assert_eq!(shield_red_models.len(), 3);
    assert_eq!(
        shield_red_models[0]["pipeline"]["location"].as_str(),
        Some("minecraft:pipeline/entity_solid")
    );
    assert_eq!(
        shield_red_models[0]["pipeline"]["depth_write"].as_bool(),
        Some(true)
    );
    assert_eq!(
        shield_red_models[1]["pipeline"]["location"].as_str(),
        Some("minecraft:pipeline/banner_pattern")
    );
    assert_eq!(
        shield_red_models[1]["pipeline"]["depth_write"].as_bool(),
        Some(false)
    );
    assert!(shield_red_models[1]["pipeline"]["alpha_cutout"].is_null());
    assert_eq!(
        shield_red_models[1]["force_solid_model_phase"].as_bool(),
        Some(true)
    );
    assert_eq!(shield_red_models[1]["has_oit"].as_bool(), Some(false));
    assert_eq!(
        shield_red_models[2]["pipeline"]["location"].as_str(),
        Some("minecraft:pipeline/glint")
    );

    let mut base_verts = Vec::new();
    let mut pattern_verts = Vec::new();
    let mut glint_verts = Vec::new();
    for v in out {
        match v.layer & 0xFFFF {
            0 => base_verts.push(v),
            crate::gfx::BANNER_PATTERN_OVERLAY => pattern_verts.push(v),
            crate::gfx::GLINT_ONLY_OVERLAY => glint_verts.push(v),
            other => panic!("unexpected shield_red overlay {other}"),
        }
    }
    assert_eq!(base_verts.len(), want_single.len() * 4);
    assert_eq!(pattern_verts.len(), want_single.len() * 4);
    assert!(
        pattern_verts
            .iter()
            .all(|v| v.normal == crate::mesher::CULL_BACK),
        "banner_pattern layers cull back faces"
    );
    let mut got = vertex_quad_glint_bits(&glint_verts);
    got.sort();
    assert_eq!(got, want_single, "26.3 shield_red patterned_shield_glint");

    // Multi-layer patterned shield (`shield_red_stripe`): base (`0`), 2 pattern passes
    // (`BANNER_PATTERN_OVERLAY`), and 1 glint pass (`GLINT_ONLY_OVERLAY`).
    let stripe_models = witness["special_models"]["shield_red_stripe"]["ground"]["models"]
        .as_array()
        .unwrap();
    assert_eq!(stripe_models.len(), 4);
    assert_eq!(
        stripe_models[1]["pipeline"]["location"].as_str(),
        Some("minecraft:pipeline/banner_pattern")
    );
    assert_eq!(
        stripe_models[2]["pipeline"]["location"].as_str(),
        Some("minecraft:pipeline/banner_pattern")
    );
    let mut shield_red_stripe = shield_red.clone();
    shield_red_stripe.patch.insert(
        "minecraft:banner_patterns".into(),
        Some(serde_json::json!([{
            "pattern": {
                "asset_id": "minecraft:stripe_bottom",
                "translation_key": "block.minecraft.banner.stripe_bottom"
            },
            "color": "blue"
        }])),
    );
    let stripe_sprites = dummy_hud_sprites(&[
        "entity/shield/shield_base",
        "entity/shield/base",
        "entity/shield/stripe_bottom",
    ]);
    let stripe_state = renderer.resolve(
        &shield_red_stripe,
        DisplayContext::Ground,
        None,
        None,
        0,
        &NoEnv,
    );
    let mut stripe_out = Vec::new();
    stripe_state.submit(
        &renderer,
        &stripe_sprites,
        &NoHost,
        Mat4::IDENTITY,
        &mut WorldSink {
            out: &mut stripe_out,
            light: 0xF0F0,
            lights: None,
            glint: Some(Glint {
                millis: 123_456,
                speed: 0.5,
                alpha: 0.75,
                scale: 8.0,
            }),
            tier: 0,
        },
    );
    let count_overlay = |ov: u32| {
        stripe_out
            .iter()
            .filter(|v| (v.layer & 0xFFFF) == ov)
            .count()
    };
    assert_eq!(count_overlay(0), want_single.len() * 4);
    assert_eq!(
        count_overlay(crate::gfx::BANNER_PATTERN_OVERLAY),
        want_single.len() * 4 * 2
    );
    assert_eq!(
        count_overlay(crate::gfx::GLINT_ONLY_OVERLAY),
        want_single.len() * 4
    );

    // In GUI, patterned shield emits foil ONCE (same glint vertex count as plain shield).
    let mut red_batch = crate::hud::HudBatch::scratch([64, 64]);
    let state_red_gui = renderer.resolve(&shield_red, DisplayContext::Gui, None, None, 0, &NoEnv);
    gui::draw_with_frame(
        &state_red_gui,
        &renderer,
        &sprites,
        &NoHost,
        &mut red_batch,
        0.0,
        0.0,
        GlintFrame {
            millis: 123_456,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        },
    );
    let mut plain_batch = crate::hud::HudBatch::scratch([64, 64]);
    let state_plain_gui =
        renderer.resolve(&shield_plain, DisplayContext::Gui, None, None, 0, &NoEnv);
    gui::draw_with_frame(
        &state_plain_gui,
        &renderer,
        &sprites,
        &NoHost,
        &mut plain_batch,
        0.0,
        0.0,
        GlintFrame {
            millis: 123_456,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        },
    );
    let count_glint =
        |b: &crate::hud::HudBatch| b.vertices.iter().filter(|v| v.uv[0] >= 2.0).count();
    assert!(count_glint(&red_batch) > 0);
    assert_eq!(
        count_glint(&red_batch),
        count_glint(&plain_batch),
        "patterned shield GUI must emit foil once, not per pattern layer"
    );
}

/// 26.3 parity: `RenderTypes.bannerPattern` / `RenderPipelines.BANNER_PATTERN` blends a partially
/// transparent pattern texel (`alpha = 8/255 < 0.1`, no alpha cutout) without writing depth.
#[test]
#[ignore = "daily"]
fn shield_banner_pattern_blends_low_alpha_without_depth_write_26_3() {
    use crate::gfx::entity_pixel_test::{
        EntityWorldDraw, render_entity_world, test_world_projection,
    };
    use sunrise_assets::item_model::NoEnv;

    let witness = glint_witness_26_3();
    let bp = &witness["banner_pattern"];
    let pipe = &bp["pipeline"];
    assert_eq!(bp["render_type"].as_str(), Some("banner_pattern"));
    assert_eq!(
        pipe["location"].as_str(),
        Some("minecraft:pipeline/banner_pattern")
    );
    assert_eq!(pipe["cull"].as_bool(), Some(true));
    assert_eq!(pipe["depth_test"].as_str(), Some("GREATER_THAN_OR_EQUAL"));
    assert_eq!(pipe["depth_write"].as_bool(), Some(false));
    assert!(pipe["alpha_cutout"].is_null());
    assert_eq!(bp["force_solid_model_phase"].as_bool(), Some(true));
    assert_eq!(bp["has_oit"].as_bool(), Some(false));
    assert_eq!(pipe["blend"]["src_color"].as_str(), Some("SRC_ALPHA"));
    assert_eq!(
        pipe["blend"]["dst_color"].as_str(),
        Some("ONE_MINUS_SRC_ALPHA")
    );
    assert_eq!(pipe["blend"]["src_alpha"].as_str(), Some("ONE"));
    assert_eq!(
        pipe["blend"]["dst_alpha"].as_str(),
        Some("ONE_MINUS_SRC_ALPHA")
    );
    assert!(
        pipe["define_flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("NO_OVERLAY"))
    );

    let parse_rgba = |v: &Value| -> [u8; 4] {
        let a = v.as_array().unwrap();
        std::array::from_fn(|i| a[i].as_u64().unwrap() as u8)
    };
    let src_rgba = parse_rgba(&bp["alpha_8_src_rgba8"]);
    let dst_rgba = parse_rgba(&bp["base_dst_rgba8"]);
    let blended_rgba = parse_rgba(&bp["blended_rgba8"]);
    assert_eq!(src_rgba[3], 8);

    // 4x1 atlas: [0]=opaque base, [1]=alpha 8/255 pattern, [2]=clear pattern, [3]=white.
    let mut atlas = Vec::with_capacity(16);
    atlas.extend_from_slice(&dst_rgba);
    atlas.extend_from_slice(&src_rgba);
    atlas.extend_from_slice(&[0, 0, 0, 0]);
    atlas.extend_from_slice(&[255, 255, 255, 255]);
    let white_glint = [255u8, 255, 255, 255];
    let view_proj = test_world_projection(70.0, 1.0, 512.0);
    let pixel = |img: &[u8], x: usize, y: usize| -> [u8; 4] {
        let at = (y * 64 + x) * 4;
        [img[at], img[at + 1], img[at + 2], img[at + 3]]
    };

    // 1. Real 26.3 patterned shield with an alpha 8/255 stripe texel blends in classic and OIT.
    let renderer = full_renderer_26_3();
    let mut shield = StackView::new("minecraft:shield", 1);
    shield.patch.insert(
        "minecraft:base_color".into(),
        Some(Value::String("white".into())),
    );
    shield.patch.insert(
        "minecraft:banner_patterns".into(),
        Some(serde_json::json!([{
            "pattern": {
                "asset_id": "minecraft:stripe_bottom",
                "translation_key": "block.minecraft.banner.stripe_bottom"
            },
            "color": "white"
        }])),
    );
    let rect = |u0, u1| Rect {
        u0,
        v0: 0.1,
        u1,
        v1: 0.9,
        w: 1,
        h: 1,
    };
    let shield_sprites = HashMap::from([
        ("entity/shield/shield_base".to_owned(), rect(0.05, 0.20)),
        ("entity/shield/base".to_owned(), rect(0.55, 0.70)),
        ("entity/shield/stripe_bottom".to_owned(), rect(0.30, 0.45)),
    ]);
    let state = renderer.resolve(&shield, DisplayContext::Ground, None, None, 0, &NoEnv);
    let pose =
        Mat4::from_translation(Vec3::new(0.0, -0.5, -1.5)) * Mat4::from_scale(Vec3::splat(4.0));
    let mut shield_verts = Vec::new();
    state.submit(
        &renderer,
        &shield_sprites,
        &NoHost,
        pose,
        &mut WorldSink {
            out: &mut shield_verts,
            light: 0xF0F0,
            lights: None,
            glint: None,
            tier: 0,
        },
    );
    for oit in [false, true] {
        let img = render_entity_world(EntityWorldDraw {
            vertices: &shield_verts,
            atlas_size: [4, 1],
            atlas: &atlas,
            glint_size: [1, 1],
            glint: &white_glint,
            view_proj,
            glint_alpha: 0.0,
            oit,
        })
        .expect("shield banner pattern GPU render");
        assert_eq!(
            pixel(&img, 32, 32),
            blended_rgba,
            "oit={oit}: alpha 8/255 shield pattern texel must blend over base instead of discarding"
        );
    }

    // 2. Depth-write-off check: equal-depth `GLINT_ONLY_OVERLAY` passes on the left (opaque base
    //    wrote depth) and fails on the right (pattern-only, no depth write).
    let quad = |x0: f32, x1: f32, u: f32, layer: u32, normal: [i8; 4]| {
        [
            [x0, -1.0, -2.0],
            [x1, -1.0, -2.0],
            [x1, 1.0, -2.0],
            [x0, 1.0, -2.0],
        ]
        .map(|pos| Vertex {
            pos,
            uv: [u, 0.5],
            layer,
            color: u32::MAX,
            light: 0xF0F0,
            normal,
            glint_uv: [0.5, 0.5],
        })
    };
    let mut depth_verts = Vec::new();
    depth_verts.extend_from_slice(&quad(-1.0, -0.05, 0.125, 0, [0; 4]));
    depth_verts.extend_from_slice(&quad(
        -1.0,
        -0.05,
        0.375,
        crate::gfx::BANNER_PATTERN_OVERLAY,
        crate::mesher::CULL_BACK,
    ));
    depth_verts.extend_from_slice(&quad(
        -1.0,
        -0.05,
        0.125,
        crate::gfx::GLINT_ONLY_OVERLAY,
        [0; 4],
    ));
    depth_verts.extend_from_slice(&quad(
        0.05,
        1.0,
        0.375,
        crate::gfx::BANNER_PATTERN_OVERLAY,
        crate::mesher::CULL_BACK,
    ));
    depth_verts.extend_from_slice(&quad(
        0.05,
        1.0,
        0.125,
        crate::gfx::GLINT_ONLY_OVERLAY,
        [0; 4],
    ));
    let depth_img = render_entity_world(EntityWorldDraw {
        vertices: &depth_verts,
        atlas_size: [4, 1],
        atlas: &atlas,
        glint_size: [1, 1],
        glint: &white_glint,
        view_proj,
        glint_alpha: 1.0,
        oit: false,
    })
    .expect("banner pattern depth-write-off GPU render");
    assert_eq!(
        pixel(&depth_img, 16, 32),
        [255, 255, 255, 255],
        "equal-depth glint must pass where opaque base wrote depth"
    );
    assert_eq!(
        pixel(&depth_img, 48, 32),
        [src_rgba[3], 0, 0, 255],
        "banner_pattern must blend alpha 8/255 without writing depth, rejecting equal-depth glint"
    );
}

/// 26.3 parity: both translucent foil variants (`OIT_ITEM_GLINT` standard UV0 and
/// `OIT_ITEM_GLINT_SPECIAL` decal UV3) composite with an overlapping translucent surface in OIT
/// without leaking `GlintAlpha` into the `OIT_ALPHA_ONLY` bounds/transmittance stages.
#[test]
#[ignore = "daily"]
fn translucent_foil_variants_composite_with_overlapping_translucent_surface_in_oit_26_3() {
    use crate::gfx::entity_pixel_test::{
        EntityWorldDraw, render_entity_world, test_world_projection,
    };

    // 1. Verify vanilla 26.3's OIT stage shader defines from the oracle.
    let witness = glint_witness_26_3();
    let oit_sets = &witness["oit_item_glint"];
    let has_flag = |stage: &Value, flag: &str| {
        stage["define_flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some(flag))
    };
    for key in ["oit_item", "oit_item_glint", "oit_item_glint_special"] {
        let set = &oit_sets[key];
        for alpha_stage in ["depth_bounds", "transmittance"] {
            assert_eq!(set[alpha_stage]["depth_write"].as_bool(), Some(false));
            assert_eq!(set[alpha_stage]["alpha_cutout"].as_str(), Some("0.1"));
            assert!(has_flag(&set[alpha_stage], "OIT_ALPHA_ONLY"));
            assert!(
                !has_flag(&set[alpha_stage], "GLINT"),
                "{key}.{alpha_stage} must not enable GLINT in OIT_ALPHA_ONLY"
            );
            assert!(
                !has_flag(&set[alpha_stage], "GLINT_SPECIAL"),
                "{key}.{alpha_stage} must not enable GLINT_SPECIAL in OIT_ALPHA_ONLY"
            );
        }
        assert_eq!(set["accumulate"]["depth_write"].as_bool(), Some(false));
        assert_eq!(set["accumulate"]["alpha_cutout"].as_str(), Some("0.1"));
        assert!(!has_flag(&set["accumulate"], "OIT_ALPHA_ONLY"));
    }
    assert!(!has_flag(&oit_sets["oit_item"]["accumulate"], "GLINT"));
    assert!(has_flag(&oit_sets["oit_item_glint"]["accumulate"], "GLINT"));
    assert!(!has_flag(
        &oit_sets["oit_item_glint"]["accumulate"],
        "GLINT_SPECIAL"
    ));
    assert!(has_flag(
        &oit_sets["oit_item_glint_special"]["accumulate"],
        "GLINT"
    ));
    assert!(has_flag(
        &oit_sets["oit_item_glint_special"]["accumulate"],
        "GLINT_SPECIAL"
    ));

    // 2. Render both translucent foil variants overlapping another translucent surface through OIT.
    let root = fixture("translucent-glint-oit-gpu");
    png(
        &root.join("assets/minecraft/textures/item/translucent_crystal.png"),
        16,
        16,
        [40, 40, 40, 128],
    );
    png(
        &root.join("assets/minecraft/textures/item/translucent_pane.png"),
        16,
        16,
        [20, 40, 220, 128],
    );
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .unwrap();
    let renderer = ItemRenderer::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        &data,
    )
    .unwrap();
    renderer.prepare_glint_atlas(16384, (0, 2)).unwrap();

    // 2x1 atlas: texel 0 = neutral translucent item (alpha 128/255), texel 1 = blue translucent surface.
    let atlas: [u8; 8] = [40, 40, 40, 128, 20, 40, 220, 128];
    let sprites = HashMap::from([
        (
            "item/translucent_crystal".to_owned(),
            Rect {
                u0: 0.1,
                v0: 0.1,
                u1: 0.4,
                v1: 0.9,
                w: 1,
                h: 1,
            },
        ),
        (
            "item/translucent_pane".to_owned(),
            Rect {
                u0: 0.6,
                v0: 0.1,
                u1: 0.9,
                v1: 0.9,
                w: 1,
                h: 1,
            },
        ),
    ]);
    let glint = Glint {
        millis: 123_456,
        speed: 0.5,
        alpha: 1.0,
        scale: Glint::ITEM_SCALE,
    };
    let front_pose = Mat4::from_translation(Vec3::new(0.0, 0.0, -2.0))
        * Mat4::from_scale(Vec3::new(2.0, 2.0, 1.0));
    let back_pose = Mat4::from_translation(Vec3::new(0.0, 0.0, -4.0))
        * Mat4::from_scale(Vec3::new(4.0, 4.0, 1.0));

    let submit_one = |sprite: &str, foil: FoilType, pose: Mat4| -> Vec<Vertex> {
        let mut out = Vec::new();
        state(sprite, foil, DisplayContext::Ground).submit(
            &renderer,
            &sprites,
            &NoHost,
            pose,
            &mut WorldSink {
                out: &mut out,
                light: 0xF0F0,
                lights: None,
                glint: Some(glint),
                tier: 0,
            },
        );
        out
    };
    let center_glint_uv = |verts: &[Vertex]| -> [f32; 2] {
        [
            ((verts[0].glint_uv[0] + verts[2].glint_uv[0]) * 0.5).rem_euclid(1.0),
            ((verts[0].glint_uv[1] + verts[2].glint_uv[1]) * 0.5).rem_euclid(1.0),
        ]
    };
    let torus_dist_sq = |a: [f32; 2], b: [f32; 2]| -> f32 {
        let dx = (a[0] - b[0]).abs();
        let dy = (a[1] - b[1]).abs();
        let dx = dx.min(1.0 - dx);
        let dy = dy.min(1.0 - dy);
        dx * dx + dy * dy
    };

    let std_front = submit_one(
        "minecraft:item/translucent_crystal",
        FoilType::Standard,
        front_pose,
    );
    let std_back = submit_one(
        "minecraft:item/translucent_crystal",
        FoilType::Standard,
        back_pose,
    );
    let spec_front = submit_one(
        "minecraft:item/translucent_crystal",
        FoilType::Special,
        front_pose,
    );
    let spec_back = submit_one(
        "minecraft:item/translucent_crystal",
        FoilType::Special,
        back_pose,
    );
    let plain_front = submit_one(
        "minecraft:item/translucent_crystal",
        FoilType::None,
        front_pose,
    );
    let plain_back = submit_one(
        "minecraft:item/translucent_crystal",
        FoilType::None,
        back_pose,
    );
    let blue_front = submit_one(
        "minecraft:item/translucent_pane",
        FoilType::None,
        front_pose,
    );
    let blue_back = submit_one("minecraft:item/translucent_pane", FoilType::None, back_pose);

    let std_uv = center_glint_uv(&std_front);
    let spec_uv = center_glint_uv(&spec_front);
    assert!(
        torus_dist_sq(std_uv, spec_uv) > 0.01,
        "standard ({std_uv:?}) and special ({spec_uv:?}) center glint UVs must be distinct"
    );

    // 64x64 glint texture: green near standard UV0, red near special decal UV3.
    let mut glint_tex = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            let uv = [(x as f32 + 0.5) / 64.0, (y as f32 + 0.5) / 64.0];
            let texel = if torus_dist_sq(uv, std_uv) <= torus_dist_sq(uv, spec_uv) {
                [0u8, 220, 0, 255]
            } else {
                [220u8, 0, 0, 255]
            };
            glint_tex.extend_from_slice(&texel);
        }
    }
    let view_proj = test_world_projection(70.0, 1.0, 512.0);
    let render_center = |slices: &[&[Vertex]]| -> [u8; 4] {
        let mut verts = Vec::new();
        for s in slices {
            verts.extend_from_slice(s);
        }
        let img = render_entity_world(EntityWorldDraw {
            vertices: &verts,
            atlas_size: [2, 1],
            atlas: &atlas,
            glint_size: [64, 64],
            glint: &glint_tex,
            view_proj,
            glint_alpha: 1.0,
            oit: true,
        })
        .expect("OIT translucent foil overlap GPU render");
        let at = (32 * 64 + 32) * 4;
        [img[at], img[at + 1], img[at + 2], img[at + 3]]
    };

    let unglinted_front = render_center(&[&plain_front, &blue_back]);
    let unglinted_back = render_center(&[&blue_front, &plain_back]);

    // Standard foil: front over blue, behind blue, solo, and reversed submission order.
    let std_only = render_center(&[&std_front]);
    let std_over_blue_ab = render_center(&[&std_front, &blue_back]);
    let std_over_blue_ba = render_center(&[&blue_back, &std_front]);
    let blue_over_std_ab = render_center(&[&blue_front, &std_back]);
    let blue_over_std_ba = render_center(&[&std_back, &blue_front]);
    assert_eq!(
        std_over_blue_ab, std_over_blue_ba,
        "OIT standard foil over blue must be order-independent"
    );
    assert_eq!(
        blue_over_std_ab, blue_over_std_ba,
        "OIT blue over standard foil must be order-independent"
    );
    assert!(
        std_over_blue_ab[2] > std_only[2] + 20,
        "blue behind alpha=128/255 standard foil (glint_alpha=1.0) must not be culled: over={std_over_blue_ab:?} solo={std_only:?}"
    );
    assert!(
        std_over_blue_ab[1] > unglinted_front[1] + 50,
        "standard foil must add green glint over blue: foil={std_over_blue_ab:?} plain={unglinted_front:?}"
    );
    assert!(
        blue_over_std_ab[1] > unglinted_back[1] + 30,
        "standard foil behind blue must still show green glint: foil={blue_over_std_ab:?} plain={unglinted_back:?}"
    );
    assert!(
        blue_over_std_ab[2] > std_over_blue_ab[2] + 10,
        "blue in front must be bluer than blue behind standard foil: front={blue_over_std_ab:?} back={std_over_blue_ab:?}"
    );

    // Special foil: front over blue, behind blue, solo, and reversed submission order.
    let spec_only = render_center(&[&spec_front]);
    let spec_over_blue_ab = render_center(&[&spec_front, &blue_back]);
    let spec_over_blue_ba = render_center(&[&blue_back, &spec_front]);
    let blue_over_spec_ab = render_center(&[&blue_front, &spec_back]);
    let blue_over_spec_ba = render_center(&[&spec_back, &blue_front]);
    assert_eq!(
        spec_over_blue_ab, spec_over_blue_ba,
        "OIT special foil over blue must be order-independent"
    );
    assert_eq!(
        blue_over_spec_ab, blue_over_spec_ba,
        "OIT blue over special foil must be order-independent"
    );
    assert!(
        spec_over_blue_ab[2] > spec_only[2] + 20,
        "blue behind alpha=128/255 special foil (glint_alpha=1.0) must not be culled: over={spec_over_blue_ab:?} solo={spec_only:?}"
    );
    assert!(
        spec_over_blue_ab[0] > unglinted_front[0] + 50,
        "special foil must add red glint over blue: foil={spec_over_blue_ab:?} plain={unglinted_front:?}"
    );
    assert!(
        blue_over_spec_ab[0] > unglinted_back[0] + 30,
        "special foil behind blue must still show red glint: foil={blue_over_spec_ab:?} plain={unglinted_back:?}"
    );
    assert!(
        blue_over_spec_ab[2] > spec_over_blue_ab[2] + 10,
        "blue in front must be bluer than blue behind special foil: front={blue_over_spec_ab:?} back={spec_over_blue_ab:?}"
    );

    // Standard and Special must produce distinct composited pixels in both depth orders.
    assert_ne!(std_over_blue_ab, spec_over_blue_ab);
    assert_ne!(blue_over_std_ab, blue_over_spec_ab);

    fs::remove_dir_all(root).ok();
}

/// 26.3 parity: `prepare_glint_atlas_with_mips` matches vanilla `SpriteLoader.stitch` bit-for-bit
/// across `items` (none & aniso2), `blocks` (mip 4 & mip 0), and `shield_patterns`.
#[test]
#[ignore = "daily"]
fn glint_atlases_match_vanilla_26_3_mip_and_aniso() {
    let witness = glint_witness_26_3();
    let root = prepared_resources_26_3();
    let data = crate::test_resources::item_data(
        &root,
        sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
    )
    .expect("26.3 prepared item data");
    let renderer = ItemRenderer::load(&ResourceRoot::directory(root), &data).unwrap();

    let check = |atlas: &glint_atlas::GlintAtlas, key: &str| {
        let expected = &witness["atlases"][key];
        assert_eq!(
            atlas.len(),
            expected["sprite_count"].as_u64().unwrap() as usize,
            "{key} sprite_count"
        );
        for (id, sprite) in expected["sprites"].as_object().unwrap() {
            let r = atlas.get_stitched(sprite_name(id)).unwrap();
            let got = [r.u0, r.v0, r.u1, r.v1].map(f32::to_bits);
            let want = [
                sprite["u0_bits"].as_u64().unwrap() as u32,
                sprite["v0_bits"].as_u64().unwrap() as u32,
                sprite["u1_bits"].as_u64().unwrap() as u32,
                sprite["v1_bits"].as_u64().unwrap() as u32,
            ];
            assert_eq!(got, want, "26.3 atlas {key} sprite {id}");
        }
    };

    renderer
        .prepare_glint_atlas_with_mips(16384, (0, 2), 4)
        .unwrap();
    check(&renderer.glint_atlas.lock().unwrap(), "items_none");
    check(
        &renderer.block_glint_atlas.lock().unwrap(),
        "blocks_mip4_none",
    );
    check(
        &renderer.shield_glint_atlas.lock().unwrap(),
        "shield_patterns_none",
    );
    // `MaterialBaker.get` maps explicit `minecraft:missingno` to the blocks atlas's missing sprite.
    let resolved_missing = resolve_cuboid_glint(
        "missingno",
        &renderer.glint_atlas.lock().unwrap(),
        &renderer.block_glint_atlas.lock().unwrap(),
    )
    .unwrap();
    let want_missing: [u32; 4] = witness["material_baker"]["vanilla_missingno_bits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u32)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_eq!(
        [
            resolved_missing.u0,
            resolved_missing.v0,
            resolved_missing.u1,
            resolved_missing.v1
        ]
        .map(f32::to_bits),
        want_missing,
        "26.3 MaterialBaker missingno"
    );
    assert_eq!(
        witness["material_baker"]["synth_items_has_shared"].as_bool(),
        Some(false)
    );
    assert_eq!(
        witness["material_baker"]["synth_shared_atlas"].as_str(),
        Some("minecraft:textures/atlas/blocks.png")
    );
    assert_eq!(
        witness["material_baker"]["synth_missingno_atlas"].as_str(),
        Some("minecraft:textures/atlas/blocks.png")
    );

    renderer
        .prepare_glint_atlas_with_mips(16384, (0, 2), 0)
        .unwrap();
    check(
        &renderer.block_glint_atlas.lock().unwrap(),
        "blocks_mip0_none",
    );

    renderer
        .prepare_glint_atlas_with_mips(16384, (2, 2), 4)
        .unwrap();
    check(&renderer.glint_atlas.lock().unwrap(), "items_aniso2");
}
