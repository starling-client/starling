use super::super::*;
use super::support::*;

#[test]
fn a_pack_overlay_reads_font_sheets_from_the_game_below_it() {
    let (pack, vanilla) = pack_font_over_defaults("sheets");
    let atlas = HudAtlas::build_over(
        &ResourceSource::directory(pack.clone()),
        &ResourceSource::directory(vanilla.clone()),
    )
    .unwrap();
    let glyph = atlas.font.glyph('a');
    let _ = fs::remove_dir_all(pack.parent().unwrap());
    assert!(glyph.rect.is_some(), "text draws, not the missing glyph");
    assert!(glyph.advance > 0.0);
}

#[test]
fn the_fonts_only_atlas_measures_text_like_the_full_one() {
    let root = two_glyph_font("atlas");
    let full = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let fonts = HudAtlas::fonts(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert!(fonts.pixels.is_empty());
    let width = full.font.width("abba");
    assert!(width > 0.0);
    assert_eq!(fonts.font.width("abba"), width);
}

#[test]
fn outlined_world_text_keeps_font_metrics_and_overrides_outline_color() {
    let root = two_glyph_font("world-outline");
    let atlas = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    let mut batch = HudBatch::scratch([1, 1]);
    let text = crate::text::Text::colored("ab", 0xff0000);
    let outline = rgba(1, 2, 3, 128);
    let (advance, front) =
        batch.text_styled_outline_world(&atlas, &text, 10.0, 20.0, u32::MAX, outline);
    assert_eq!((advance, front, batch.vertices.len()), (10.0, 96, 108));
    assert!(batch.vertices[..front].iter().all(|v| v.color == outline));
    assert!(
        batch.vertices[front..]
            .iter()
            .all(|v| v.color == rgba(255, 0, 0, 255))
    );
    // Eight independently offset silhouettes, retaining both glyphs and
    // the bitmap font's advance (a=4, b=6); front text has no shadow.
    for (i, [dx, dy]) in [
        [-1.0, -1.0],
        [-1.0, 0.0],
        [-1.0, 1.0],
        [0.0, -1.0],
        [0.0, 1.0],
        [1.0, -1.0],
        [1.0, 0.0],
        [1.0, 1.0],
    ]
    .into_iter()
    .enumerate()
    {
        let at = batch.vertices[i * 12].pos;
        let main = batch.vertices[front].pos;
        assert_eq!(at, [main[0] + dx, main[1] + dy]);
        assert_eq!(batch.vertices[i * 12 + 6].pos[0] - at[0], 4.0);
    }
}

#[test]
fn obfuscated_glyphs_keep_the_width() {
    let root = two_glyph_font("obfuscated");
    let fonts = HudAtlas::fonts(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    let (a, b) = (fonts.font.glyph('a').advance, fonts.font.glyph('b').advance);
    assert_ne!(a, b);
    for _ in 0..20 {
        assert_eq!(fonts.font.random_glyph(&Default::default(), a).advance, a);
        assert_eq!(fonts.font.random_glyph(&Default::default(), b).advance, b);
    }
    // No glyph that wide: FontSet's missing glyph.
    let missing = fonts.font.random_glyph(&Default::default(), 30.0);
    assert!(missing.rect.is_none() && missing.right == 5.0);
}

#[test]
fn component_shadow_color_overrides_text_call_shadow() {
    let root = two_glyph_font("component-shadow");
    let atlas = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let mut batch = HudBatch::scratch([320, 240]);
    let text = crate::text::Text::styled(
        "a",
        crate::text::Style {
            shadow_color: Some(0x8040_2010),
            ..Default::default()
        },
    );
    batch.text_styled(&atlas, &text, 10.0, 10.0, u32::MAX, false);
    assert_eq!(batch.vertices.len(), 12);
    assert_eq!(batch.vertices[0].color, rgba(0x40, 0x20, 0x10, 0x80));
    batch.vertices.clear();
    batch.text_styled_world(&atlas, &text, 10.0, 10.0, u32::MAX, false);
    assert_eq!(batch.vertices.len(), 12);
    assert_eq!(batch.vertices[0].depth, 0.0);
    assert_eq!(batch.vertices[6].depth, 0.03);
    batch.vertices.clear();
    let no_shadow = crate::text::Text::styled(
        "a",
        crate::text::Style {
            shadow_color: Some(0),
            ..Default::default()
        },
    );
    batch.text_styled(&atlas, &no_shadow, 10.0, 10.0, u32::MAX, true);
    assert_eq!(batch.vertices.len(), 6);
    batch.vertices.clear();
    let gradient = crate::component::to_text(
        &serde_json::json!({
            "text": "",
            "extra": [
                {"text": "a", "color": "#ff0000"},
                {"text": "b", "color": "#0000ff"}
            ]
        }),
        &mut |_| crate::text::Text::empty(),
    );
    batch.text_styled(&atlas, &gradient, 10.0, 10.0, u32::MAX, false);
    assert_eq!(batch.vertices[0].color, rgba(255, 0, 0, 255));
    assert_eq!(batch.vertices[6].color, rgba(0, 0, 255, 255));
}

#[test]
fn player_object_draws_an_inline_head() {
    let root = two_glyph_font("inline-head");
    write_png(&root, "entity/player/slim/alex", 64, 64);
    write_png(&root, "entity/player/wide/steve", 64, 64);
    let mut atlas = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let text = crate::component::to_text(
        &serde_json::json!({"object":"player","player":{"id":"00000000-0000-0000-0000-000000000000"}}),
        &mut |_| crate::text::Text::empty(),
    );
    let mut batch = HudBatch::scratch([320, 240]);
    assert_eq!(
        batch.text_styled(&atlas, &text, 10.0, 10.0, u32::MAX, false),
        8.0
    );
    assert_eq!(batch.vertices.len(), 12); // face and hat, six vertices each
    assert_eq!(batch.inline_profiles.len(), 1);
    atlas.set_object_skin("name:elyapluggin".into(), "entity/player/wide/steve".into());
    let named = crate::component::to_text(
        &serde_json::json!({"object":"player","player":{"name":"elyapluggin"}}),
        &mut |_| crate::text::Text::empty(),
    );
    batch.vertices.clear();
    batch.text_styled(&atlas, &named, 10.0, 10.0, u32::MAX, false);
    let skin = atlas.sprites["entity/player/wide/steve"];
    assert_eq!(batch.vertices[0].uv[0], skin.u0 + (skin.u1 - skin.u0) / 8.0);
    batch.vertices.clear();
    batch.text_styled(&atlas, &named, 10.0, 10.0, u32::MAX, true);
    assert_eq!(batch.vertices.len(), 24); // shadow and front, each with face and hat
    assert_eq!(batch.vertices[0].pos, [11.0, 10.0]);
    assert_eq!(batch.vertices[0].color, rgba(63, 63, 63, 255));
    assert_eq!(batch.vertices[12].pos, [10.0, 9.0]);
    assert_eq!(batch.vertices[12].color, u32::MAX);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn atlas_object_draws_an_item_sprite_inline() {
    let root = two_glyph_font("inline-item");
    write_png(&root, "item/porkchop", 16, 16);
    let atlas = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let text = crate::component::to_text(
        &serde_json::json!({
            "text": "a",
            "extra": [{"object": "atlas", "sprite": "minecraft:item/porkchop"}]
        }),
        &mut |_| crate::text::Text::empty(),
    );
    let mut batch = HudBatch::scratch([320, 240]);
    let width = batch.text_styled(&atlas, &text, 10.0, 10.0, u32::MAX, false);
    assert_eq!(width, atlas.font.glyph('a').advance + 8.0);
    assert_eq!(batch.vertices.len(), 12); // glyph and one sprite
    let sprite = atlas.sprites["item/porkchop"];
    assert_eq!(batch.vertices[6].uv, [sprite.u0, sprite.v0]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn name_tags_draw_bold_text_and_tall_glyphs_like_any_text() {
    // `ascii.png`'s 8x8 'a' (4 pixels wide) and `accented.png`'s 9x12
    // 'й' (height 12, ascent 10: the breve above the line), 7 wide.
    let root = std::env::temp_dir().join(format!("sunrise-name-tag-{}", std::process::id()));
    let textures = root.join("assets/minecraft/textures/font");
    fs::create_dir_all(root.join("assets/minecraft/font")).unwrap();
    fs::create_dir_all(&textures).unwrap();
    fs::write(
        root.join("assets/minecraft/font/default.json"),
        r#"{"providers":[
            {"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["a"]},
            {"type":"bitmap","file":"minecraft:font/tall.png","height":12,"ascent":10,"chars":["й"]}]}"#,
    )
    .unwrap();
    for (name, (w, h), used) in [("a", (8, 8), 4), ("tall", (9, 12), 7)] {
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|i| [255, 255, 255, if i % w < used { 255 } else { 0 }])
            .collect();
        let file = fs::File::create(textures.join(format!("{name}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
    }
    let atlas = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    let text = crate::text::Text::literal("aй").bold();
    let mut batch = HudBatch::scratch([1, 1]);
    let behind = batch.name_tag(&atlas, &text, 0.0, u32::MAX, 0x3F00_0000);
    // Each quad's box: x0, y0, x1, y1.
    let boxes: Vec<[f32; 4]> = batch
        .vertices
        .as_chunks::<6>()
        .0
        .iter()
        .map(|q| [q[0].pos[0], q[0].pos[1], q[2].pos[0], q[2].pos[1]])
        .collect();
    // Advances 5 and 8, plus 1 each for bold: Font.width 15, centred.
    let x = -7.5;
    // Each glyph twice (bold), the copy a pixel right; 'й' in its 9x12
    // box from 3 pixels above the line, each bold quad 0.1 larger.
    let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4);
    let expected = [
        [x - 0.1, -0.1, x + 8.1, 8.1],
        [x + 0.9, -0.1, x + 9.1, 8.1],
        [x + 5.9, -3.1, x + 15.1, 9.1],
        [x + 6.9, -3.1, x + 16.1, 9.1],
    ];
    for (got, want) in boxes[1..].iter().zip(expected) {
        assert!(near(*got, want), "{got:?} != {want:?}");
    }
    assert_eq!(boxes.len(), 1 + expected.len());
    // The background first: a pixel round the advances, 10 pixels tall.
    assert_eq!(behind, 6);
    assert_eq!(boxes[0], [x - 1.0, -1.0, x + 15.0, 9.0]);
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn missing_glyph_quads_match_vanilla_26_3_witness() {
    let witness_path = std::env::var("SUNRISE_DISPLAY_PARITY_WITNESS").expect(
        "SUNRISE_DISPLAY_PARITY_WITNESS must be set (run via tools/test_displays_parity.py)",
    );
    let witness_text = std::fs::read_to_string(&witness_path)
        .unwrap_or_else(|e| panic!("failed to read {witness_path}: {e}"));
    let w: serde_json::Value = serde_json::from_str(&witness_text).unwrap();
    let mg = &w["missing_glyph_quads"];
    let bitmap = &mg["bitmap"];
    let hex_f32 = |v: &serde_json::Value| {
        f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
    };
    let hex_argb_to_abgr = |v: &serde_json::Value| {
        let argb = u32::from_str_radix(v.as_str().unwrap(), 16).unwrap();
        rgba(
            (argb >> 16) as u8,
            (argb >> 8) as u8,
            argb as u8,
            (argb >> 24) as u8,
        )
    };

    let empty_root =
        std::env::temp_dir().join(format!("sunrise-missing-glyph-{}", std::process::id()));
    let _ = fs::remove_dir_all(&empty_root);
    fs::create_dir_all(&empty_root).unwrap();
    let packed_atlas = HudAtlas::build(&ResourceSource::directory(empty_root.clone())).unwrap();
    let _ = fs::remove_dir_all(&empty_root);

    for (atlas_name, atlas) in [("boot", HudAtlas::boot()), ("packed", packed_atlas)] {
        // Glyph metrics match SpecialGlyphs.MISSING.
        let g = atlas.font.glyph('\u{2196}');
        assert_eq!(g.left.to_bits(), hex_f32(&bitmap["left_bits"]).to_bits());
        assert_eq!(g.right.to_bits(), hex_f32(&bitmap["right_bits"]).to_bits());
        assert_eq!(g.top.to_bits(), hex_f32(&bitmap["top_bits"]).to_bits());
        assert_eq!(
            g.bottom.to_bits(),
            hex_f32(&bitmap["bottom_bits"]).to_bits()
        );
        assert_eq!(
            g.advance.to_bits(),
            hex_f32(&bitmap["advance_bits"]).to_bits()
        );
        assert_eq!(
            g.offset.to_bits(),
            hex_f32(&bitmap["bold_offset_bits"]).to_bits()
        );
        let bold_style = crate::text::Style {
            bold: Some(true),
            ..Default::default()
        };
        assert_eq!(
            atlas.font.advance('\u{2196}', &bold_style).to_bits(),
            hex_f32(&bitmap["bold_advance_bits"]).to_bits()
        );

        // The 5x8 SpecialGlyphs.MISSING bitmap is present in the atlas with the exact mask.
        let sprite = atlas
            .sprites
            .get("sunrise/missing_glyph")
            .unwrap_or_else(|| panic!("{atlas_name}: missing sunrise/missing_glyph sprite"));
        assert_eq!(
            sprite.texels((atlas.width, atlas.height)),
            (
                bitmap["pixel_width"].as_u64().unwrap() as u32,
                bitmap["pixel_height"].as_u64().unwrap() as u32,
            )
        );
        let mask: Vec<String> = (0..8u32)
            .map(|y| {
                (0..5u32)
                    .map(|x| {
                        let px = texel_at(&atlas, *sprite, x, y);
                        if px == [255, 255, 255, 255] {
                            '#'
                        } else {
                            assert_eq!(px, [0, 0, 0, 0]);
                            '.'
                        }
                    })
                    .collect()
            })
            .collect();
        let want_mask: Vec<&str> = bitmap["mask"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(mask, want_mask, "{atlas_name}: missing glyph pixel mask");

        let cases = [
            (
                "plain_flat",
                0.0f32,
                0.0,
                0xFFFF_FFFFu32,
                false,
                false,
                false,
                true,
            ),
            ("bold_flat", 0.0, 0.0, 0xFFFF_FFFF, true, false, false, true),
            (
                "bold_world",
                0.0,
                0.0,
                0xFFFF_FFFF,
                true,
                false,
                false,
                false,
            ),
            (
                "bold_shadow_world",
                0.0,
                0.0,
                0xFFFF_FFFF,
                true,
                false,
                true,
                false,
            ),
            (
                "italic_flat",
                0.0,
                0.0,
                0xFFFF_FFFF,
                false,
                true,
                false,
                true,
            ),
            (
                "bold_italic_shadow_world",
                2.0,
                3.0,
                0xFFFF_AA00,
                true,
                true,
                true,
                false,
            ),
        ];
        for (name, x, y, argb, bold, italic, shadow, flat) in cases {
            let want = &mg["cases"][name];
            let mut text = crate::text::Text::literal("\u{2196}");
            if bold {
                text = text.bold();
            }
            if italic {
                text = text.italic();
            }
            let color = rgba(
                (argb >> 16) as u8,
                (argb >> 8) as u8,
                argb as u8,
                (argb >> 24) as u8,
            );
            let mut batch = HudBatch::scratch([320, 240]);
            if flat {
                batch.text_styled(&atlas, &text, x, y, color, shadow);
            } else {
                batch.text_styled_world(&atlas, &text, x, y, color, shadow);
            }
            let quads = batch.vertices.as_chunks::<6>().0;
            let want_quads = want["quads"].as_array().unwrap();
            assert_eq!(
                quads.len(),
                want_quads.len(),
                "{atlas_name}/{name}: quad count"
            );
            // BakedSheetGlyph emits corners [tl, bl, br, tr]; HudBatch::quad stores [tl, tr, br, tl, br, bl].
            let corner_idx = [0usize, 5, 2, 1];
            for (qi, (got_q, want_q)) in quads.iter().zip(want_quads).enumerate() {
                let want_vs = want_q.as_array().unwrap();
                for (ci, (&bi, want_v)) in corner_idx.iter().zip(want_vs).enumerate() {
                    let gv = got_q[bi];
                    let wp = want_v["pos_bits"].as_array().unwrap();
                    assert_eq!(
                        [gv.pos[0].to_bits(), gv.pos[1].to_bits(), gv.depth.to_bits()],
                        [
                            hex_f32(&wp[0]).to_bits(),
                            hex_f32(&wp[1]).to_bits(),
                            hex_f32(&wp[2]).to_bits(),
                        ],
                        "{atlas_name}/{name} quad {qi} corner {ci} pos"
                    );
                    assert_eq!(
                        gv.color,
                        hex_argb_to_abgr(&want_v["color_argb"]),
                        "{atlas_name}/{name} quad {qi} corner {ci} color"
                    );
                }
            }
        }
    }
}

#[test]
fn world_glyph_copies_emit_per_glyph_before_decorations() {
    let root = two_glyph_font("world-glyph-order");
    let atlas = HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
        root.to_path_buf(),
    ))
    .unwrap();
    let _ = fs::remove_dir_all(&root);
    let mut batch = HudBatch::scratch([320, 240]);
    let text = crate::text::Text::styled(
        "ab",
        crate::text::Style {
            bold: Some(true),
            underlined: Some(true),
            strikethrough: Some(true),
            color: Some(0xFFAA00),
            ..Default::default()
        },
    );
    batch.text_styled_world(&atlas, &text, 0.0, 0.0, u32::MAX, true);
    let quads = batch.vertices.as_chunks::<6>().0;
    // 2 glyphs * (2 shadow + 2 fg) = 8 glyph quads, then 2 glyphs * 2 decorations * (shadow + fg) = 8 effect quads.
    assert_eq!(quads.len(), 16);
    let fg = rgba(0xFF, 0xAA, 0x00, 0xFF);
    // Glyph 0: shadow, bold shadow, fg, bold fg.
    assert_ne!(quads[0][0].color, fg);
    assert_ne!(quads[1][0].color, fg);
    assert_eq!(quads[2][0].color, fg);
    assert_eq!(quads[3][0].color, fg);
    assert!(quads[2][0].pos[0] < quads[4][0].pos[0]);
    // Glyph 1: shadow, bold shadow, fg, bold fg.
    assert_ne!(quads[4][0].color, fg);
    assert_ne!(quads[5][0].color, fg);
    assert_eq!(quads[6][0].color, fg);
    assert_eq!(quads[7][0].color, fg);
    // Decorations follow all glyphs, with world depths 0.01 (shadow) and 0.04 (foreground).
    for pair in quads[8..].as_chunks::<2>().0 {
        assert_eq!(pair[0][0].depth, 0.01_f32);
        assert_eq!(pair[1][0].depth, 0.01_f32 + 0.03_f32);
    }
}
