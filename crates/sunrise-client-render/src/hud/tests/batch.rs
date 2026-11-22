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
    assert!(!batch.vertices.is_empty());
    assert!(batch.vertices.iter().all(|v| v.pos[1] <= 4.0));
    let area: f32 = batch
        .vertices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            ((t[1].pos[0] - t[0].pos[0]) * (t[2].pos[1] - t[0].pos[1])
                - (t[2].pos[0] - t[0].pos[0]) * (t[1].pos[1] - t[0].pos[1]))
                .abs()
                / 2.0
        })
        .sum();
    // Rows 0..4 of the triangle: 16 * 16 / 2 - 12 * 12 / 2.
    assert!((area - 56.0).abs() < 1e-3, "{area}");
}

#[test]
fn clip_since_moves_blend_ranges_with_their_vertices() {
    let mut batch = HudBatch::scratch([320, 240]);
    let start = batch.vertices.len();
    let tri = |batch: &mut HudBatch, y: f32| {
        batch.quad(
            [[0.0, y], [8.0, y], [0.0, y + 8.0], [0.0, y + 8.0]],
            [[0.0; 2]; 4],
            u32::MAX,
        );
    };
    tri(&mut batch, 100.0); // outside, dropped
    let foil = batch.vertices.len();
    tri(&mut batch, 0.0); // straddles the bottom edge
    batch.blend_since(foil, Blend::Additive);
    batch.clip_since(start, 0.0, 0.0, 16.0, 4.0);
    assert_eq!(
        batch.blends,
        [(0..batch.vertices.len() as u32, Blend::Additive)]
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
    // A clip containing both faces leaves every attribute as it was.
    batch.clip_since(start, 0.0, 0.0, 16.0, 16.0);
    assert_eq!(batch.vertices.len(), before.len());
    for (v, w) in batch.vertices.iter().zip(&before) {
        assert_eq!((v.pos, v.color, v.text), (w.pos, w.color, w.text));
        let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(near(v.depth, w.depth), "depth {} vs {}", v.depth, w.depth);
        assert!(near(v.uv[0], w.uv[0]) && near(v.uv[1], w.uv[1]));
    }
    // A clip cutting the sloped face keeps its depth across the cut.
    batch.clip_since(start, 0.0, 0.0, 16.0, 4.0);
    let sloped: Vec<_> = batch.vertices[6..]
        .iter()
        .map(|v| (v.pos[1], v.depth))
        .collect();
    let at = |y: f32| before[6..].iter().find(|v| v.pos[1] == y).unwrap().depth;
    for (y, depth) in sloped {
        let want = at(2.0) + (at(6.0) - at(2.0)) * (y - 2.0) / 4.0;
        assert!((depth - want).abs() < 1e-6, "y {y}: {depth} vs {want}");
    }
}
