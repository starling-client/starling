//! `PoseOp::Rotate` against JOML's `Matrix3x2f.rotate` (what `Matrix3x2fStack.rotate` runs):
//! quarter turns, composition with `Translate` and `Scale` in vanilla's order, nested
//! `Push` / `Pop`, text following the turn, and a non-finite angle refused.
// The expected matrices spell JOML's products term by term, factors of -1 included.
#![allow(clippy::neg_multiply)]
use super::{GuiDraw, IDENTITY, Pose, PoseOp, paint, pose_apply, pose_refused};
use crate::hud::{HudAtlas, HudBatch};
use std::f32::consts::{FRAC_PI_2, PI};

/// `(float) Math.cos((double) (float) (PI / 2))` and the `sin` of `(float) PI`, as HotSpot
/// answers them: the float angles are a little past the turn, so the zero is not exact.
const COS_HALF_PI: f32 = -4.371139000186243e-8_f64 as f32;
const SIN_PI: f32 = -8.742277657347586e-8_f64 as f32;

fn atlas() -> HudAtlas {
    HudAtlas::build(&crate::resource_source::ResourceSource::directory(
        "/nonexistent/sunrise-test-assets".into(),
    ))
    .unwrap()
}

fn posed(ops: &[PoseOp]) -> Pose {
    let mut m = IDENTITY;
    for op in ops {
        pose_apply(&mut m, *op);
    }
    m
}

fn bits(m: Pose) -> [u32; 6] {
    m.map(f32::to_bits)
}

#[test]
fn quarter_turns_give_joml_matrices_bit_for_bit() {
    let c = COS_HALF_PI;
    assert_eq!(
        bits(posed(&[PoseOp::Rotate(FRAC_PI_2)])),
        bits([c, 1.0, -1.0, c, 0.0, 0.0])
    );
    assert_eq!(
        bits(posed(&[PoseOp::Rotate(-FRAC_PI_2)])),
        bits([c, -1.0, 1.0, c, 0.0, 0.0])
    );
    assert_eq!(
        bits(posed(&[PoseOp::Rotate(PI)])),
        bits([-1.0, SIN_PI, -SIN_PI, -1.0, 0.0, 0.0])
    );
    // Two quarter turns are JOML's products of the first one's floats, not a half turn.
    let twice = posed(&[PoseOp::Rotate(FRAC_PI_2), PoseOp::Rotate(FRAC_PI_2)]);
    let (m00, m01) = (c * c + -1.0 * 1.0, 1.0 * c + c * 1.0);
    let (m10, m11) = (c * -1.0 + -1.0 * c, 1.0 * -1.0 + c * c);
    assert_eq!(bits(twice), bits([m00, m01, m10, m11, 0.0, 0.0]));
    assert_eq!(bits(posed(&[PoseOp::Rotate(0.0)])), bits(IDENTITY));
}

#[test]
fn rotate_composes_with_translate_and_scale_in_vanillas_order() {
    let c = COS_HALF_PI;
    // translate(10, 20).rotate(pi / 2).scale(2, 3): the turn is about the translated origin and
    // the scale is along the turned axes.
    let m = posed(&[
        PoseOp::Translate(10.0, 20.0),
        PoseOp::Rotate(FRAC_PI_2),
        PoseOp::Scale(2.0, 3.0),
    ]);
    assert_eq!(bits(m), bits([c * 2.0, 2.0, -3.0, c * 3.0, 10.0, 20.0]));
    // rotate(pi / 2).translate(10, 0): the move is along the turned x axis, so downwards.
    let m = posed(&[PoseOp::Rotate(FRAC_PI_2), PoseOp::Translate(10.0, 0.0)]);
    assert_eq!(
        bits(m),
        bits([c, 1.0, -1.0, c, c * 10.0 + -1.0 * 0.0, 1.0 * 10.0 + c * 0.0])
    );
}

/// The corners of what `paint` drew, rounded to a thousandth of a pixel.
fn corners(batch: &HudBatch, range: std::ops::Range<usize>) -> Vec<[i32; 2]> {
    let mut out: Vec<[i32; 2]> = batch.vertices[range]
        .iter()
        .map(|v| v.pos.map(|p| (p * 1000.0).round() as i32))
        .collect();
    out.sort();
    out.dedup();
    out
}

fn fill() -> GuiDraw {
    GuiDraw::Fill {
        x0: 0,
        y0: 0,
        x1: 4,
        y1: 2,
        color: 0xFFFF_FFFF,
    }
}

#[test]
fn nested_push_and_pop_scope_the_turn_and_what_is_drawn_follows_it() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    paint(
        &mut batch,
        &atlas,
        &[
            GuiDraw::Push(vec![PoseOp::Translate(100.0, 50.0)]),
            GuiDraw::Push(vec![PoseOp::Rotate(FRAC_PI_2)]),
            fill(),
            GuiDraw::Pop,
            fill(),
            GuiDraw::Pop,
            fill(),
        ],
    );
    assert_eq!(batch.vertices.len(), 18);
    // A quarter turn clockwise about (100, 50): x runs down, y runs left.
    assert_eq!(
        corners(&batch, 0..6),
        [
            [98000, 50000],
            [98000, 54000],
            [100000, 50000],
            [100000, 54000]
        ]
    );
    assert_eq!(
        corners(&batch, 6..12),
        [
            [100000, 50000],
            [100000, 52000],
            [104000, 50000],
            [104000, 52000]
        ]
    );
    assert_eq!(
        corners(&batch, 12..18),
        [[0, 0], [0, 2000], [4000, 0], [4000, 2000]]
    );
}

#[test]
fn text_turns_with_the_pose() {
    let atlas = atlas();
    let text = |batch: &mut HudBatch, ops: Vec<PoseOp>| {
        paint(
            batch,
            &atlas,
            &[
                GuiDraw::Push(ops),
                GuiDraw::Text {
                    text: crate::text::Text::literal("Hi"),
                    x: 0,
                    y: 0,
                    color: 0xFFFF_FFFF,
                    shadow: false,
                },
                GuiDraw::Pop,
            ],
        );
    };
    let (mut flat, mut turned) = (HudBatch::scratch([854, 480]), HudBatch::scratch([854, 480]));
    text(&mut flat, vec![PoseOp::Translate(100.0, 50.0)]);
    text(
        &mut turned,
        vec![PoseOp::Translate(100.0, 50.0), PoseOp::Rotate(FRAC_PI_2)],
    );
    assert!(!flat.vertices.is_empty(), "no glyphs drawn");
    assert_eq!(flat.vertices.len(), turned.vertices.len());
    for (f, t) in flat.vertices.iter().zip(&turned.vertices) {
        // (x, y) about (100, 50) turns to (100 - (y - 50), 50 + (x - 100)).
        let want = [100.0 - (f.pos[1] - 50.0), 50.0 + (f.pos[0] - 100.0)];
        assert!(
            (t.pos[0] - want[0]).abs() < 1e-3 && (t.pos[1] - want[1]).abs() < 1e-3,
            "{:?} is not {want:?}",
            t.pos
        );
        assert_eq!(t.uv, f.uv);
    }
}

#[test]
fn a_non_finite_angle_is_refused_and_leaves_the_pose_as_it_was() {
    for angle in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let ops = [
            PoseOp::Translate(1.0, 2.0),
            PoseOp::Rotate(angle),
            PoseOp::Scale(2.0, 2.0),
        ];
        let mut m = posed(&ops[..1]);
        let before = m;
        pose_apply(&mut m, ops[1]);
        assert_eq!(bits(m), bits(before));
        assert_eq!(bits(posed(&ops)), bits(posed(&[ops[0], ops[2]])), "{angle}");
        assert!(
            matches!(pose_refused(&ops), Some(PoseOp::Rotate(a)) if a.is_nan() == angle.is_nan())
        );
    }
    assert_eq!(
        pose_refused(&[PoseOp::Rotate(1.0), PoseOp::Scale(f32::NAN, 1.0)]),
        None
    );
}
