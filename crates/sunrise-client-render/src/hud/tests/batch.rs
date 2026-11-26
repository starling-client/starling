use super::super::*;

#[test]
fn clip_since_keeps_the_visible_part_of_a_straddling_triangle() {
    let mut batch = HudBatch::scratch([320, 240]);
    let start = batch.vertices.len();
    // A right triangle (a quad `a, b, c, c`) whose centre lies outside the rect.
    batch.quad(
        [[0.0, 0.0], [16.0, 0.0], [0.0, 16.0], [0.0, 16.0]],
        [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.0, 1.0]],
        u32::MAX,
    );
    batch.clip_since(start, 0.0, 0.0, 16.0, 4.0);
    assert_eq!(batch.vertices.len(), 6);
    assert_eq!(batch.vertices[2].pos, [0.0, 16.0]);
    assert_eq!(
        batch.scissors_in(0..6).collect::<Vec<_>>(),
        [(0..6, Some([0.0, 0.0, 16.0, 4.0])),]
    );
}

#[test]
fn clip_since_keeps_blend_ranges_in_submission_order() {
    let mut batch = HudBatch::scratch([320, 240]);
    let start = batch.vertices.len();
    let tri = |batch: &mut HudBatch, y: f32| {
        batch.quad(
            [[0.0, y], [8.0, y], [0.0, y + 8.0], [0.0, y + 8.0]],
            [[0.0; 2]; 4],
            u32::MAX,
        );
    };
    tri(&mut batch, 100.0); // outside the hardware scissor
    let foil = batch.vertices.len();
    tri(&mut batch, 0.0); // straddles the bottom edge
    batch.blend_since(foil, Blend::Additive);
    batch.clip_since(start, 0.0, 0.0, 16.0, 4.0);
    assert_eq!(
        batch.runs(),
        [(0..6, Blend::Alpha), (6..12, Blend::Additive)]
    );
    assert_eq!(
        batch.scissors_in(0..12).collect::<Vec<_>>(),
        [(0..12, Some([0.0, 0.0, 16.0, 4.0])),]
    );
}

#[test]
fn clip_since_keeps_the_depth_of_an_item_or_scene_face() {
    let mut batch = HudBatch::scratch([320, 240]);
    let start = batch.vertices.len();
    // Two axis-aligned model faces at different depths, as an item model or a
    // `draw_scene` picture leaves them after `depth_since`.
    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    batch.quad_3d(
        [
            [0.0, 0.0, 1.0],
            [8.0, 0.0, 1.0],
            [8.0, 8.0, 1.0],
            [0.0, 8.0, 1.0],
        ],
        uv,
        u32::MAX,
    );
    batch.quad_3d(
        [
            [2.0, 2.0, 3.0],
            [6.0, 2.0, 3.0],
            [6.0, 6.0, 5.0],
            [2.0, 6.0, 5.0],
        ],
        uv,
        u32::MAX,
    );
    batch.depth_since(start, [0.0, 0.0, 320.0, 240.0]);
    let before = batch.vertices.clone();
    assert!(before.iter().any(|v| v.depth > 0.0));
    batch.clip_since(start, 0.0, 0.0, 16.0, 16.0);
    batch.clip_since(start, 0.0, 0.0, 16.0, 4.0);
    assert_eq!(
        batch.scissors_in(0..12).collect::<Vec<_>>(),
        [(0..12, Some([0.0, 0.0, 16.0, 4.0])),]
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&batch.vertices),
        bytemuck::cast_slice::<_, u8>(&before)
    );
}

#[test]
fn clip_since_restricts_only_the_scoped_part_of_a_coalesced_blend() {
    let mut batch = HudBatch::scratch([320, 240]);
    let quad = |batch: &mut HudBatch, x: f32, y: f32| {
        batch.quad(
            [[x, y], [x + 4.0, y], [x + 4.0, y + 4.0], [x, y + 4.0]],
            [[0.0; 2]; 4],
            u32::MAX,
        );
    };
    quad(&mut batch, 32.0, 32.0);
    batch.invert_since(0);
    let start = batch.vertices.len();
    quad(&mut batch, -8.0, 0.0); // outside
    quad(&mut batch, 0.0, 0.0); // cut at x = 2
    batch.invert_since(start);
    assert_eq!(batch.blends, [(0..18, Blend::Invert)]);

    batch.clip_since(start, 2.0, 0.0, 8.0, 8.0);
    let start = batch.vertices.len();
    quad(&mut batch, 0.0, 0.0);
    batch.invert_since(start);
    batch.clip_since(start, 2.0, 0.0, 2.0, 8.0); // empty scissor
    quad(&mut batch, 16.0, 16.0);
    assert_eq!(
        batch.runs(),
        [(0..24, Blend::Invert), (24..30, Blend::Alpha)]
    );
    assert_eq!(
        batch.scissors_in(0..30).collect::<Vec<_>>(),
        [
            (0..6, None),
            (6..18, Some([2.0, 0.0, 8.0, 8.0])),
            (18..24, Some([2.0, 0.0, 2.0, 8.0])),
            (24..30, None),
        ]
    );
    assert_eq!(batch.vertices[24].pos, [16.0, 16.0]);
}

#[test]
fn draw_scope_rollback_restores_the_prior_batch_tail() {
    let font = crate::font::Font::metrics(&ResourceSource::indexed(Default::default()));
    let mut batch = HudBatch::scratch([320, 240]);
    let corners = [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]];
    let portal = EndPortalFill {
        game_time: 0.5,
        ..Default::default()
    };
    batch.quad(corners, [[0.0; 2]; 4], u32::MAX);
    batch.invert_since(0);
    batch.clip_since(0, 1.0, 0.0, 3.0, 4.0);
    batch.end_portal = portal;
    batch.glint_alpha = 0.5;
    batch.blur_at = 6;
    let profile = Arc::new(serde_json::json!({"name":"baseline"}));
    batch.inline_profiles.push(profile.clone());
    assert_eq!(
        batch.draw_scope(&font, |batch| {
            batch.quad(corners, [[0.0; 2]; 4], u32::MAX);
            batch.invert_since(6);
            batch.clip_since(6, 1.0, 0.0, 3.0, 4.0);
            batch.quad(corners, [[0.0; 2]; 4], u32::MAX);
            batch.blend_since(12, Blend::Additive);
            batch.end_portal = EndPortalFill::default();
            batch.glint_alpha = 0.25;
            batch.blur_at = 18;
            batch.pictures.push(Arc::new(pictures::Picture {
                id: 1,
                width: 1,
                height: 1,
                pixels: vec![255; 4],
                flags: TextureFlags::default(),
            }));
            batch.patches.push(crate::font::Patch {
                x: 4,
                y: 0,
                width: 1,
                height: 1,
                rgba: vec![255; 4],
            });
            batch
                .inline_profiles
                .push(Arc::new(serde_json::json!({"name":"scoped"})));
            Err::<(), _>("abort")
        }),
        Err("abort")
    );
    batch.quad(corners.map(|[x, y]| [x + 8.0, y]), [[0.0; 2]; 4], u32::MAX);
    assert_eq!(batch.runs(), [(0..6, Blend::Invert), (6..12, Blend::Alpha)]);
    assert_eq!(
        batch.vertices[..6]
            .iter()
            .map(|v| v.pos)
            .collect::<Vec<_>>(),
        [
            [0.0, 0.0],
            [4.0, 0.0],
            [4.0, 4.0],
            [0.0, 0.0],
            [4.0, 4.0],
            [0.0, 4.0]
        ]
    );
    assert_eq!(batch.vertices[6].pos, [8.0, 0.0]);
    assert_eq!(
        batch.scissors_in(0..12).collect::<Vec<_>>(),
        [(0..6, Some([1.0, 0.0, 3.0, 4.0])), (6..12, None),]
    );
    assert_eq!(
        (batch.end_portal, batch.glint_alpha, batch.blur_at),
        (portal, 0.5, 6)
    );
    assert!(batch.pictures.is_empty());
    assert!(batch.patches.is_empty());
    assert_eq!(batch.inline_profiles, [profile]);
}

#[test]
fn an_inline_dynamic_texture_draws_as_its_own_texture_in_the_glyphs_box() {
    let atlas = HudAtlas::fonts(&Default::default()).unwrap();
    let mut span = crate::text::Text::literal("\u{fffc}").spans[0].clone();
    span.style.inline_object = Some(crate::text::InlineObject::Dynamic {
        id: "demo:star".into(),
        handle: 7,
        size: [16, 16],
        fallback: ":star:".into(),
    });
    let mut batch = HudBatch::scratch([320, 240]);
    batch.text_styled(
        &atlas,
        &crate::text::Text::from_spans(vec![span]),
        10.0,
        20.0,
        u32::MAX,
        false,
    );
    let [_, _, w, h] = batch.visible_vertex_bounds(0).unwrap();
    let side = crate::font::INLINE_OBJECT_SIZE;
    assert_eq!((w, h), (side, side), "one glyph box");
    assert_eq!(batch.blends, [(0..6, Blend::Dynamic(7))]);
}
