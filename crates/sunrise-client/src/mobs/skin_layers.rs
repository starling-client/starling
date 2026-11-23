//! 3D skin layers (off by default) fill the space between the player's base
//! model and the second skin layer. Painted texels span both surfaces, while
//! transparent texels leave openings with visible edges.
//!
//! The geometry is built once per (skin texture, slim/wide) from the decoded
//! 64×64 RGBA skin and cached by the game; the model tree, part names and the
//! texture's normalised UVs are the ones the renderer already uses, so both
//! the CPU and the GPU (`super::skin`) pose paths draw it unchanged.
use super::{Part, Polygon};

/// The parts of `PlayerModel` that make up the outer layer, as the prepared
/// `entity-models.json` names them.
pub const OUTER_PARTS: [&str; 6] = [
    "hat",
    "jacket",
    "left_sleeve",
    "right_sleeve",
    "left_pants",
    "right_pants",
];

/// The size of a prepared skin image: `resample_64` always hands out this
/// many texels a side, wide and legacy skins alike.
pub const SIZE: usize = 64;

/// A texel this transparent is not drawn at all (`alpha` of 0–255); the ones
/// just above it stay, so a tinted glass-like layer keeps its translucency.
const ALPHA_CUTOFF: u8 = 8;

/// Thickness in model pixels, including a small overlap with the body so
/// coplanar faces cannot leave cracks where the layers meet.
const MIN_THICKNESS: f32 = 1.0;
const BODY_OVERLAP: f32 = 1.0 / 1024.0;

/// Fill the player's outer parts down to the corresponding faces of the body.
pub fn extrude(model: &Part, skin: &[u8], size: usize) -> Option<Part> {
    if skin.len() < size * size * 4 {
        return None;
    }
    let mut changed = false;
    let mut out = model.clone();
    explode(&mut out, skin, size, &mut changed);
    changed.then_some(out)
}

fn explode(part: &mut Part, skin: &[u8], size: usize, changed: &mut bool) {
    // In the player model, each overlay is an untransformed child of its body
    // part. Their faces share the same coordinate system and animation.
    for child in &mut part.children {
        if OUTER_PARTS.contains(&child.name.as_str())
            && child.pose == [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
        {
            child.polygons = child
                .polygons
                .iter()
                .flat_map(|outer| {
                    match part
                        .polygons
                        .iter()
                        .find(|inner| inner.normal == outer.normal)
                    {
                        Some(inner) => {
                            extrude_polygon(outer, inner, &child.polygons, skin, size, changed)
                        }
                        None => vec![outer.clone()],
                    }
                })
                .collect();
        }
        explode(child, skin, size, changed);
    }
}

/// One shell face, or the original face if it has no usable texel grid.
fn extrude_polygon(
    polygon: &Polygon,
    inner: &Polygon,
    outer_faces: &[Polygon],
    skin: &[u8],
    size: usize,
    changed: &mut bool,
) -> Vec<Polygon> {
    let (u0, u1, v0, v1) = polygon
        .vertices
        .iter()
        .fold((f32::MAX, f32::MIN, f32::MAX, f32::MIN), |a, v| {
            (a.0.min(v[3]), a.1.max(v[3]), a.2.min(v[4]), a.3.max(v[4]))
        });
    let texels_u = ((u1 - u0) * size as f32).round() as i32;
    let texels_v = ((v1 - v0) * size as f32).round() as i32;
    if texels_u <= 0 || texels_v <= 0 || texels_u > 32 || texels_v > 32 {
        return vec![polygon.clone()];
    }
    let (Some(a), Some(b), Some(c)) = (
        corner(polygon, u0, v0),
        corner(polygon, u1, v0),
        corner(polygon, u0, v1),
    ) else {
        return vec![polygon.clone()];
    };
    let n = glam::Vec3::from_array(polygon.normal).normalize_or_zero();
    if n == glam::Vec3::ZERO {
        return vec![polygon.clone()];
    }
    // The face's own axes, from its UV grid, so the skirt normals point away
    // from the slab no matter how the quad is wound.
    let du = (b - a).normalize_or_zero();
    let dv = (c - a).normalize_or_zero();
    let span_u = (u1 - u0).max(f32::EPSILON);
    let span_v = (v1 - v0).max(f32::EPSILON);
    let at = |u: f32, v: f32| a + (b - a) * ((u - u0) / span_u) + (c - a) * ((v - v0) / span_v);
    // Project along the face normal. Shrinking the tangents toward the inner
    // corners would turn every pixel into a wedge.
    let depth = (a - position(&inner.vertices[0])).dot(n);
    if depth <= f32::EPSILON {
        return vec![polygon.clone()];
    }
    let inward = -n * (depth + BODY_OVERLAP).max(MIN_THICKNESS);
    let axes = [du, dv];
    let inner_bounds = axes.map(|axis| {
        inner
            .vertices
            .iter()
            .map(|v| position(v).dot(axis))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
                (min.min(value), max.max(value))
            })
    });
    let facings = [-du, du, -dv, dv];
    let adjacent = facings.map(|facing| {
        outer_faces.iter().find(|face| {
            glam::Vec3::from_array(face.normal).dot(facing) > 0.999
                && face
                    .vertices
                    .iter()
                    .any(|v| (position(v) - a).dot(n).abs() < 1e-4)
        })
    });
    let base_u = (u0 * size as f32).round() as i32;
    let base_v = (v0 * size as f32).round() as i32;
    let opaque = |i: i32, j: i32| {
        if i < 0 || j < 0 || i >= texels_u || j >= texels_v {
            return false;
        }
        let x = (base_u + i) as usize;
        let y = (base_v + j) as usize;
        x < size && y < size && skin[(y * size + x) * 4 + 3] > ALPHA_CUTOFF
    };
    let mut out = Vec::with_capacity((texels_u * texels_v) as usize);
    let step = 1.0 / size as f32;
    for j in 0..texels_v {
        for i in 0..texels_u {
            if !opaque(i, j) {
                continue;
            }
            let (ua, va) = (u0 + i as f32 * step, v0 + j as f32 * step);
            let (ub, vb) = (ua + step, va + step);
            let (p00, p10) = (at(ua, va), at(ub, va));
            let (p11, p01) = (at(ub, vb), at(ua, vb));
            let uv = ((ua + ub) * 0.5, (va + vb) * 0.5);
            out.push(quad([p00, p10, p11, p01], uv, polygon.normal));
            // Shared sides between painted texels are inside the shell.
            let sides = [
                ((i - 1, j), [p00, p01]),
                ((i + 1, j), [p10, p11]),
                ((i, j - 1), [p00, p10]),
                ((i, j + 1), [p01, p11]),
            ];
            for (side, ((ni, nj), [s0, s1])) in sides.into_iter().enumerate() {
                if opaque(ni, nj) {
                    continue;
                }
                // At a box edge an opaque perpendicular texel already
                // covers this wall. Omit it to avoid coplanar faces.
                let boundary = ni < 0 || nj < 0 || ni >= texels_u || nj >= texels_v;
                if boundary
                    && adjacent[side]
                        .is_some_and(|face| solid_at(face, (s0 + s1 + inward) * 0.5, skin, size))
                {
                    continue;
                }
                out.push(quad(
                    [s0, s1, s1 + inward, s0 + inward],
                    uv,
                    facings[side].to_array(),
                ));
            }
            // Only texels that overhang the body have a visible reverse
            // face. Filling it closes the piece when viewed from the inside.
            if axes
                .iter()
                .zip(inner_bounds)
                .any(|(axis, (min, max))| p00.dot(*axis) < min || p11.dot(*axis) > max)
            {
                out.push(quad(
                    [p00 + inward, p10 + inward, p11 + inward, p01 + inward],
                    uv,
                    (-n).to_array(),
                ));
            }
        }
    }
    *changed = true;
    out
}

fn solid_at(face: &Polygon, p: glam::Vec3, skin: &[u8], size: usize) -> bool {
    let [a, b, _, d] = face.vertices;
    let ab = position(&b) - position(&a);
    let ad = position(&d) - position(&a);
    let offset = p - position(&a);
    let s = offset.dot(ab) / ab.length_squared();
    let t = offset.dot(ad) / ad.length_squared();
    if !(0.0..1.0).contains(&s)
        || !(0.0..1.0).contains(&t)
        || (offset - ab * s - ad * t).length_squared() > 1e-8
    {
        return false;
    }
    let texel = |axis: usize| {
        ((a[axis] + (b[axis] - a[axis]) * s + (d[axis] - a[axis]) * t) * size as f32).floor()
            as usize
    };
    let (x, y) = (texel(3), texel(4));
    x < size && y < size && skin[(y * size + x) * 4 + 3] == 255
}

fn quad(mut positions: [glam::Vec3; 4], uv: (f32, f32), normal: [f32; 3]) -> Polygon {
    let winding = (positions[1] - positions[0]).cross(positions[2] - positions[0]);
    if winding.dot(glam::Vec3::from_array(normal)) < 0.0 {
        positions.swap(1, 3);
    }
    Polygon {
        normal,
        vertices: std::array::from_fn(|k| {
            [positions[k].x, positions[k].y, positions[k].z, uv.0, uv.1]
        }),
        cull_back: false,
    }
}

fn corner(polygon: &Polygon, u: f32, v: f32) -> Option<glam::Vec3> {
    polygon
        .vertices
        .iter()
        .find(|a| (a[3] - u).abs() < 1e-4 && (a[4] - v).abs() < 1e-4)
        .map(position)
}

fn position(v: &[f32; 5]) -> glam::Vec3 {
    glam::Vec3::new(v[0], v[1], v[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2×2-texel face facing +Z, its four corners at the given depth.
    fn face() -> Polygon {
        Polygon {
            normal: [0.0, 0.0, 1.0],
            vertices: [
                [-1.0, -1.0, 0.5, 0.0, 0.0],
                [1.0, -1.0, 0.5, 0.5, 0.0],
                [1.0, 1.0, 0.5, 0.5, 0.5],
                [-1.0, 1.0, 0.5, 0.0, 0.5],
            ],
            cull_back: false,
        }
    }

    fn skin(alpha: [u8; 4]) -> Vec<u8> {
        let mut skin = vec![0; 4 * 4 * 4];
        for (i, a) in alpha.iter().enumerate() {
            skin[i * 4 + 3] = *a;
        }
        skin
    }

    fn head() -> Part {
        let mut body_face = face();
        for v in &mut body_face.vertices {
            v[0] *= 0.75;
            v[1] *= 0.75;
            v[2] = 0.25;
        }
        let pose = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        Part {
            name: "head".into(),
            pose,
            polygons: vec![body_face],
            children: vec![Part {
                name: "hat".into(),
                pose,
                polygons: vec![face()],
                children: Vec::new(),
            }],
        }
    }

    #[test]
    fn painted_texel_forms_a_square_solid_joined_to_the_body() {
        let model = head();
        let out = extrude(&model, &skin([255, 0, 0, 0]), 4).expect("a hat");
        assert_eq!(out.polygons[0].vertices, model.polygons[0].vertices);
        let polygons = &out.children[0].polygons;
        assert_eq!(polygons.len(), 6);
        let underside = &polygons[5];
        assert_eq!(underside.normal, [0.0, 0.0, -1.0]);
        assert!(
            underside
                .vertices
                .iter()
                .all(|v| v[2] == -0.5 && v[3..] == [0.125, 0.125])
        );
        for v in polygons[0].vertices {
            assert_eq!(v[2], 0.5);
            assert_eq!(v[3..], [0.125, 0.125]);
        }
        for polygon in &polygons[1..5] {
            let [a, b, c, _] = polygon.vertices.map(|v| position(&v));
            assert!(
                (b - a)
                    .cross(c - a)
                    .dot(glam::Vec3::from_array(polygon.normal))
                    > 0.0
            );
            assert_eq!(polygon.vertices.iter().filter(|v| v[2] == -0.5).count(), 2);
            assert_eq!(polygon.vertices.iter().filter(|v| v[2] == 0.5).count(), 2);
        }
        // Every wall is square: the corners on both surfaces have identical
        // tangential coordinates, even though the body underneath is narrower.
        for polygon in &polygons[1..5] {
            for outer in polygon.vertices.iter().filter(|v| v[2] == 0.5) {
                assert!(
                    polygon
                        .vertices
                        .iter()
                        .any(|inner| { inner[2] == -0.5 && inner[..2] == outer[..2] })
                );
            }
        }
    }

    #[test]
    fn texel_inside_the_body_footprint_has_no_hidden_cap() {
        let mut model = head();
        // The outer face covers four texels per edge. Cell (1, 1) falls
        // wholly inside the smaller body face.
        for v in &mut model.children[0].polygons[0].vertices {
            v[3] *= 2.0;
            v[4] *= 2.0;
        }
        let mut skin = skin([0; 4]);
        skin[(4 + 1) * 4 + 3] = 255;
        let out = extrude(&model, &skin, 4).unwrap();
        assert_eq!(out.children[0].polygons.len(), 5);
    }

    #[test]
    fn opaque_neighbor_owns_the_shared_edge() {
        let mut model = head();
        let side = Polygon {
            normal: [-1.0, 0.0, 0.0],
            vertices: [
                [-1.0, -1.0, 0.5, 0.5, 0.0],
                [-1.0, -1.0, -1.5, 1.0, 0.0],
                [-1.0, 1.0, -1.5, 1.0, 0.5],
                [-1.0, 1.0, 0.5, 0.5, 0.5],
            ],
            cull_back: false,
        };
        let mut inner = side.clone();
        for v in &mut inner.vertices {
            v[0] = -0.75;
        }
        model.polygons.push(inner);
        model.children[0].polygons.push(side);
        let polygons_on_side = |alpha| {
            let out = extrude(&model, &skin([255, 0, alpha, 0]), 4).unwrap();
            out.children[0]
                .polygons
                .iter()
                .filter(|p| p.vertices.iter().all(|v| v[0] == -1.0))
                .count()
        };
        // An open edge needs the front texel's wall. When the perpendicular
        // texel is opaque, its cap covers the same area.
        assert_eq!(polygons_on_side(0), 1);
        assert_eq!(polygons_on_side(255), 1);
        assert_eq!(polygons_on_side(128), 2);
    }

    #[test]
    fn a_transparent_face_draws_nothing() {
        let out = extrude(&head(), &skin([0, 0, 0, 0]), 4).expect("a hat");
        assert!(out.children[0].polygons.is_empty());
    }

    #[test]
    fn a_part_without_an_outer_layer_is_untouched() {
        let mut model = head();
        model.children.clear();
        assert!(extrude(&model, &skin([255; 4]), 4).is_none());
    }
}
