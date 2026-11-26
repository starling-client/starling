// Still-fluid tiling follows Catnip/Ponder 1.0.82 FluidRenderHelper.renderTiledFace.
// Copyright (c) 2022 The Create Team
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use glam::{Mat4, Vec3};
use sunrise_core::geometry::Direction;

/// Controls a still-fluid box. The tint is stack ARGB, independent of biome tint.
#[derive(Clone, Copy, Debug)]
pub struct NativeFluidRenderOptions {
    pub argb: Option<u32>,
    pub render_bottom: bool,
    /// TextureAtlasSprite.uvShrinkRatio: 4 / the largest stitched atlas dimension.
    /// Explicit because Sunrise's block sprites occupy arrays rather than a stitched atlas.
    pub uv_shrink_ratio: f32,
}

/// A corner before the caller's pose, with UVs within the still sprite, and its outward normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [i8; 3],
}

/// Preserve sky light and increase block light to the fluid's luminosity (native light packing).
pub fn fluid_light(light: u32, luminosity: u8) -> u32 {
    (light & 0xf0) | (((light >> 8 & 0xf0).max(u32::from(luminosity.min(15)) * 16)) << 8)
}

/// Emit tiled fluid quads transformed by `pose`, deriving renderable tile spans from `pose`
/// before traversing each face so provably overflowing faces or wide spans are skipped without
/// enumerating their individual tiles, while valid neighboring spans are retained. Returning
/// `false` from `emit` stops further emission (for example at a buffer limit). Negative
/// coordinates and exact integer boundaries retain the source's local UV fractions.
pub fn fluid_box(
    bounds: [f32; 6],
    pose: Mat4,
    options: NativeFluidRenderOptions,
    mut emit: impl FnMut([FluidVertex; 4]) -> bool,
) {
    if !options.uv_shrink_ratio.is_finite()
        || !pose.is_finite()
        || bounds.iter().any(|x| !x.is_finite())
        || (0..3).any(|i| bounds[i] > bounds[i + 3])
    {
        return;
    }
    let [x0, y0, z0, x1, y1, z1] = bounds;
    for (side, face) in [
        (Direction::Down, [x0, z0, x1, z1, y0]),
        (Direction::Up, [x0, z0, x1, z1, y1]),
        (Direction::North, [x0, y0, x1, y1, z0]),
        (Direction::South, [x0, y0, x1, y1, z1]),
        (Direction::West, [z0, y0, z1, y1, x0]),
        (Direction::East, [z0, y0, z1, y1, x1]),
    ] {
        if (side != Direction::Down || options.render_bottom)
            && !tiled_face(&mut emit, side, face, pose, options.uv_shrink_ratio)
        {
            return;
        }
    }
}

struct FacePose {
    rows: [[f32; 4]; 3],
    axes: [usize; 3],
    plane: f32,
}

#[derive(Clone, Copy)]
enum SpanRequirement {
    /// A coordinate can stay if each constraint could be satisfied somewhere on the other axis.
    Possible,
    /// Every corner in the strip must be finite, at both ends of the other axis.
    AllCorners,
}

impl FacePose {
    fn new(side: Direction, plane: f32, pose: Mat4) -> Self {
        let axes = match side {
            Direction::Down | Direction::Up => [0, 2, 1],
            Direction::North | Direction::South => [0, 1, 2],
            Direction::West | Direction::East => [2, 1, 0],
        };
        Self {
            rows: [0, 1, 2].map(|row| pose.row(row).to_array()),
            axes,
            plane,
        }
    }

    fn clip_axis(
        &self,
        axis: usize,
        mut span: [f32; 2],
        other: [f32; 2],
        requirement: SpanRequirement,
    ) -> Option<[f32; 2]> {
        let varying = self.axes[axis];
        let fixed = self.axes[1 - axis];
        let all_corners = matches!(requirement, SpanRequirement::AllCorners);
        for row in self.rows {
            for end in 0..2 {
                // Along either axis each product and partial sum is monotone, with the sign of
                // that axis's coefficient. Use the favorable opposite-axis endpoint for a broad
                // span and the unfavorable one when both endpoints must fit (a tile strip).
                let high = (end == 1) == (row[varying] >= 0.0);
                let other_high = (high == all_corners) == (row[fixed] >= 0.0);
                let mut point = [0.0; 3];
                point[fixed] = other[usize::from(other_high)];
                point[self.axes[2]] = self.plane;
                let outside = |coord| {
                    let mut point = point;
                    point[varying] = coord;
                    overflows_toward(row, point, high)
                };
                if !outside(span[end]) {
                    continue;
                }
                if outside(span[1 - end]) {
                    return None;
                }
                // Search the ordered f32 encodings so we keep the exact rounded boundary,
                // independently of the scale of the input coordinates.
                let key = |v: f32| {
                    let bits = v.to_bits();
                    if v.is_sign_negative() {
                        !bits
                    } else {
                        bits ^ (1 << 31)
                    }
                };
                let float = |key: u32| {
                    f32::from_bits(if key >> 31 == 0 {
                        !key
                    } else {
                        key ^ (1 << 31)
                    })
                };
                let mut rejected = key(span[end]);
                let mut kept = key(span[1 - end]);
                while rejected.abs_diff(kept) > 1 {
                    let mid = rejected.midpoint(kept);
                    if outside(float(mid)) {
                        rejected = mid;
                    } else {
                        kept = mid;
                    }
                }
                span[end] = float(kept);
            }
        }
        Some(span)
    }
}

/// Test every rounded stage of `Mat4::transform_point3`, not just its affine result. Once an
/// operation overflows the corner is invalid. Saturating the *proof* terms lets later stages
/// stay monotone even at an already invalid corner (no infinity cancellation or NaNs); it
/// has no effect on finite corners or on the actual vertex transform below.
fn overflows_toward(row: [f32; 4], point: [f32; 3], high: bool) -> bool {
    let infinity = if high {
        f32::INFINITY
    } else {
        f32::NEG_INFINITY
    };
    let mut overflowed = false;
    let mut finite = |v: f32| {
        overflowed |= v == infinity;
        v.clamp(f32::MIN, f32::MAX)
    };
    let x = finite(row[0] * point[0]);
    let y = finite(row[1] * point[1]);
    let xy = finite(x + y);
    let z = finite(row[2] * point[2]);
    let xyz = finite(xy + z);
    let _ = finite(xyz + row[3]);
    overflowed
}

fn align_span(start: f32, end: f32, [lo, hi]: [f32; 2]) -> Option<(f32, f32)> {
    let aligned_start = if lo > start {
        // The finite span begins past `start`: begin on the first block boundary inside it, so
        // the overflowing prefix is never walked and a start too far out for a unit step to
        // advance does not reject the finite tiles beyond it.
        let cand = lo.ceil();
        // A unit step cannot reach past the exact integer range of f32.
        if cand > (1 << f32::MANTISSA_DIGITS) as f32 || cand >= end {
            return None;
        }
        cand
    } else {
        let next_start = start.floor() + 1.0;
        if next_start <= start {
            return (start >= lo && end <= hi).then_some((start, end));
        }
        start
    };
    let first_next = aligned_start.floor() + 1.0;
    let first_end = if first_next > aligned_start {
        first_next.min(end)
    } else {
        end
    };
    (first_end <= hi).then_some((aligned_start, hi))
}

fn tiled_face(
    out: &mut impl FnMut([FluidVertex; 4]) -> bool,
    side: Direction,
    face: [f32; 5],
    pose: Mat4,
    ratio: f32,
) -> bool {
    let [a0, b0, a1, b1, plane] = face;
    if a0 >= a1 || b0 >= b1 {
        return true;
    }
    let face_pose = FacePose::new(side, plane, pose);
    let Some(a_span) = face_pose.clip_axis(0, [a0, a1], [b0, b1], SpanRequirement::Possible) else {
        return true;
    };
    let Some((start_a, max_end_a)) = align_span(a0, a1, a_span) else {
        return true;
    };
    let b_span = [b0, b1];
    let whole_width =
        face_pose.clip_axis(1, b_span, [start_a, max_end_a], SpanRequirement::AllCorners)
            == Some(b_span);
    let positive = matches!(side, Direction::Up | Direction::South | Direction::East);
    let shrink = ratio * 0.25;
    let lerp = |v| v + shrink * (0.5 - v);
    let mut a = start_a;
    while a < a1 {
        let next_a = a.floor() + 1.0;
        let end_a = if next_a > a { next_a.min(a1) } else { a1 };
        if end_a > max_end_a {
            break;
        }
        let (start_b, max_end_b) = if whole_width {
            (b0, b1)
        } else {
            let Some(aligned) = face_pose
                .clip_axis(1, b_span, [a, end_a], SpanRequirement::AllCorners)
                .and_then(|span| align_span(b0, b1, span))
            else {
                a = end_a;
                continue;
            };
            aligned
        };
        let (u0, u1) = if matches!(side, Direction::North | Direction::East) {
            (end_a.ceil() - end_a, end_a.ceil() - a)
        } else {
            (a - a.floor(), end_a - a.floor())
        };
        let (u0, u1) = (lerp(u0), lerp(u1));
        let mut b = start_b;
        while b < b1 {
            let next_b = b.floor() + 1.0;
            let end_b = if next_b > b { next_b.min(b1) } else { b1 };
            if end_b > max_end_b {
                break;
            }
            let (v0, v1) = if side == Direction::Up {
                (b - b.floor(), end_b - b.floor())
            } else {
                (end_b.ceil() - end_b, end_b.ceil() - b)
            };
            let (v0, v1) = (lerp(v0), lerp(v1));
            let local_positions = match side {
                Direction::West | Direction::East => {
                    let (near, far) = if positive { (end_a, a) } else { (a, end_a) };
                    [
                        [plane, end_b, near],
                        [plane, b, near],
                        [plane, b, far],
                        [plane, end_b, far],
                    ]
                }
                Direction::North | Direction::South => {
                    let (near, far) = if positive { (a, end_a) } else { (end_a, a) };
                    [
                        [near, end_b, plane],
                        [near, b, plane],
                        [far, b, plane],
                        [far, end_b, plane],
                    ]
                }
                Direction::Down | Direction::Up => {
                    let (near, far) = if positive { (b, end_b) } else { (end_b, b) };
                    [
                        [a, plane, near],
                        [a, plane, far],
                        [end_a, plane, far],
                        [end_a, plane, near],
                    ]
                }
            };
            let positions =
                local_positions.map(|p| pose.transform_point3(Vec3::from_array(p)).to_array());
            let normal = match side {
                Direction::Down => [0, -1, 0],
                Direction::Up => [0, 1, 0],
                Direction::North => [0, 0, -1],
                Direction::South => [0, 0, 1],
                Direction::West => [-1, 0, 0],
                Direction::East => [1, 0, 0],
            };
            let uvs = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
            if uvs.iter().flatten().all(|v| v.is_finite()) {
                let quad = std::array::from_fn(|i| FluidVertex {
                    pos: positions[i],
                    uv: uvs[i],
                    normal,
                });
                if !out(quad) {
                    return false;
                }
            }
            b = end_b;
        }
        a = end_a;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_tiles_beyond_an_unrepresentable_unit_step_are_kept() {
        // At -2e7 a unit step rounds back to the same f32, yet the finite span starts at -0.5.
        assert_eq!(-20_000_000.0_f32 + 1.0, -20_000_000.0);
        let (start, end) = align_span(-20_000_000.0, 1.0, [-0.5, 1.0]).unwrap();
        assert_eq!((start, end), (0.0, 1.0));
        // A start that is itself inside the span and cannot advance still refuses a partial span.
        assert_eq!(align_span(-20_000_000.0, 1.0, [-30_000_000.0, 0.5]), None);
        assert_eq!(
            align_span(-20_000_000.0, 1.0, [-30_000_000.0, 1.0]),
            Some((-20_000_000.0, 1.0))
        );
    }

    #[test]
    fn source_tiling_is_outward_and_does_not_inset_geometry() {
        let bounds = [-1.25, 0.125, -0.25, 1.5, 1.125, 0.75];
        let mut result = Vec::new();
        fluid_box(
            bounds,
            Mat4::IDENTITY,
            NativeFluidRenderOptions {
                argb: None,
                render_bottom: false,
                uv_shrink_ratio: 4.0 / 2048.0,
            },
            |quad| {
                result.extend_from_slice(&quad);
                true
            },
        );
        assert_eq!(result.len(), (4 * 2 * 2 + 2 * 2 * 2 + 4 * 2) * 4);
        for quad in result.as_chunks::<4>().0 {
            let [a, b, c, _] = quad.map(|v| glam::Vec3::from_array(v.pos));
            assert!(
                (b - a)
                    .cross(c - a)
                    .dot(glam::Vec3::from_array(quad[0].normal.map(f32::from)))
                    > 0.0
            );
            assert!(
                quad.iter()
                    .all(|v| (0..3).all(|i| v.pos[i] >= bounds[i] && v.pos[i] <= bounds[i + 3]))
            );
        }
        assert!(
            result
                .iter()
                .any(|v| v.pos == [bounds[0], bounds[4], bounds[2]])
        );
    }

    #[test]
    fn large_coordinates_large_quad_counts_and_unclamped_shrink_emit_without_artificial_caps() {
        // Coordinates beyond the removed 1,048,576-block cap emit normally.
        let mut far_quads = 0usize;
        fluid_box(
            [2_000_000.0, 0.0, 0.0, 2_000_001.0, 1.0, 1.0],
            Mat4::IDENTITY,
            NativeFluidRenderOptions {
                argb: None,
                render_bottom: true,
                uv_shrink_ratio: 1.5,
            },
            |_| {
                far_quads += 1;
                true
            },
        );
        assert_eq!(far_quads, 6);

        // Coordinates at f32 ULP >= 2.0 terminate cleanly instead of looping.
        let mut huge_quads = 0usize;
        fluid_box(
            [20_000_000.0, 0.0, 0.0, 20_000_002.0, 1.0, 1.0],
            Mat4::IDENTITY,
            NativeFluidRenderOptions {
                argb: None,
                render_bottom: true,
                uv_shrink_ratio: 0.0,
            },
            |_| {
                huge_quads += 1;
                true
            },
        );
        assert_eq!(huge_quads, 6);

        // Boxes exceeding the removed 65,536-quad cap emit every tiled quad.
        let mut big_quads = 0usize;
        fluid_box(
            [0.0, 0.0, 0.0, 200.0, 1.0, 200.0],
            Mat4::IDENTITY,
            NativeFluidRenderOptions {
                argb: None,
                render_bottom: true,
                uv_shrink_ratio: 0.0,
            },
            |_| {
                big_quads += 1;
                true
            },
        );
        assert_eq!(big_quads, 200 * 200 * 2 + 200 * 4);

        // Returning false stops further emission immediately (buffer limit).
        let mut capped = 0usize;
        fluid_box(
            [0.0, 0.0, 0.0, 10.0, 1.0, 10.0],
            Mat4::IDENTITY,
            NativeFluidRenderOptions {
                argb: None,
                render_bottom: true,
                uv_shrink_ratio: 0.0,
            },
            |_| {
                capped += 1;
                capped < 5
            },
        );
        assert_eq!(capped, 5);
    }

    #[test]
    fn overflowing_pose_skips_invalid_wide_spans_and_retains_valid_neighbors() {
        let options = NativeFluidRenderOptions {
            argb: None,
            render_bottom: true,
            uv_shrink_ratio: 0.0,
        };

        // Entire wide X/Z box overflows Y under a large finite Y scale: zero tiles enumerated.
        let mut visited = 0usize;
        fluid_box(
            [0.0, 10.0, 0.0, 1_000_000.0, 11.0, 1_000_000.0],
            Mat4::from_scale(Vec3::new(1.0, 1e38, 1.0)),
            options,
            |_| {
                visited += 1;
                true
            },
        );
        assert_eq!(visited, 0);

        // Wide X span overflows past x = 3 while x in [0, 3] remains valid; top face (y = 10)
        // overflows completely while bottom (y = 0) and side tiles in [0, 3] x [0, 3] remain.
        let mut valid = 0usize;
        let mut total = 0usize;
        fluid_box(
            [0.0, 0.0, 0.0, 1_000_000.0, 10.0, 1.0],
            Mat4::from_scale(Vec3::new(1e38, 1e38, 1.0)),
            options,
            |quad| {
                total += 1;
                if quad.iter().all(|v| v.pos.into_iter().all(f32::is_finite)) {
                    valid += 1;
                }
                true
            },
        );
        // Valid tiles: Down (3x1 = 3), Up (0), North (3x3 = 9), South (3x3 = 9), West (1x3 = 3),
        // East (0 because x1 = 1_000_000 overflows). Total valid = 24, and no wide traversal.
        assert_eq!(valid, 24);
        assert!(total <= 30, "enumerated too many tiles: {total}");

        // Leading invalid wide span [-1_000_000, 0) skipped in O(1); valid neighbor [0, 3] kept.
        let pose = Mat4::from_translation(Vec3::new(-3.4e38, 0.0, 0.0))
            * Mat4::from_scale(Vec3::new(1e36, 1.0, 1.0));
        let mut kept = 0usize;
        let mut seen = 0usize;
        fluid_box(
            [-1_000_000.0, 0.0, 0.0, 3.0, 1.0, 1.0],
            pose,
            options,
            |quad| {
                seen += 1;
                if quad.iter().all(|v| v.pos.into_iter().all(f32::is_finite)) {
                    kept += 1;
                }
                true
            },
        );
        // Valid faces on [0, 3] x [0, 1] x [0, 1]: Down(3) + Up(3) + North(3) + South(3) + East(1) = 13.
        // West is at x0 = -1_000_000 (overflows -> 0).
        assert_eq!(kept, 13);
        assert!(seen <= 20, "leading invalid span enumerated tiles: {seen}");
    }

    #[test]
    fn partial_transform_overflow_skips_wide_spans_but_keeps_neighboring_tiles() {
        use glam::Vec4;

        for (side, pose, wide, neighbors) in [
            // X + Y overflows on the top face even though translation would cancel Y.
            (
                Direction::Up,
                Mat4::from_cols(
                    Vec4::new(2e32, 0.0, 0.0, 0.0),
                    Vec4::new(-2e38, 1.0, 0.0, 0.0),
                    Vec4::Z,
                    Vec4::new(-2e38, 0.0, 0.0, 1.0),
                ),
                [1_000_000.0, 0.0, 1_200_000.0, 1_000_000.0, -1.0],
                [701_409.0, 0.0, 701_415.0, 2.0, -1.0],
            ),
            // X + Y is finite here, but adding the Z term overflows, along the other tile axis.
            (
                Direction::North,
                Mat4::from_cols(
                    Vec4::Y,
                    Vec4::new(2e32, 0.0, 0.0, 0.0),
                    Vec4::new(-2e38, 0.0, 1.0, 0.0),
                    Vec4::new(-2e38, 0.0, 0.0, 1.0),
                ),
                [0.0, 1_000_000.0, 1_000_000.0, 1_200_000.0, -1.0],
                [0.0, 701_409.0, 2.0, 701_415.0, -1.0],
            ),
        ] {
            let [a0, b0, a1, b1, plane] = wide;
            assert!(
                FacePose::new(side, plane, pose)
                    .clip_axis(0, [a0, a1], [b0, b1], SpanRequirement::Possible)
                    .is_none(),
                "the whole face is invalid before tile traversal"
            );
            let mut quads = 0;
            tiled_face(
                &mut |quad| {
                    assert!(quad.iter().all(|v| v.pos.into_iter().all(f32::is_finite)));
                    quads += 1;
                    true
                },
                side,
                neighbors,
                pose,
                0.0,
            );
            // [701409, 701410] and [701410, 701411] are both finite; the next tile's far
            // endpoint is not. Each accepted interval pairs with two tiles on the other axis.
            assert_eq!(quads, 4);
        }
    }

    #[test]
    fn reversed_or_non_finite_inputs_emit_nothing() {
        for (bounds, pose, ratio) in [
            ([1.0, 0.0, 0.0, 0.0, 1.0, 1.0], Mat4::IDENTITY, 0.0),
            ([0.0, 0.0, 0.0, f32::NAN, 1.0, 1.0], Mat4::IDENTITY, 0.0),
            ([0.0, 0.0, 0.0, 1.0, 1.0, 1.0], Mat4::IDENTITY, f32::NAN),
            (
                [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                Mat4::from_scale(Vec3::splat(f32::INFINITY)),
                0.0,
            ),
        ] {
            fluid_box(
                bounds,
                pose,
                NativeFluidRenderOptions {
                    argb: None,
                    render_bottom: true,
                    uv_shrink_ratio: ratio,
                },
                |_| panic!("invalid box must not emit: {bounds:?} {ratio:?}"),
            );
        }
    }
    #[test]
    fn luminosity_changes_only_block_light() {
        assert_eq!(fluid_light(0x20a0, 15), 0xf0a0);
        assert_eq!(fluid_light(0x25a5, 15), 0xf0a0);
        assert_eq!(fluid_light(0xe030, 3), 0xe030);
    }
}
