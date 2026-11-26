//! The normal stream (`pack_atlas::TerrainDraws::Normals`): each terrain vertex's quad's face
//! normal and tangent, worked out at upload as `pack_atlas`'s `FACE` works them out in a vertex
//! shader that reads the quad's corners, for per-section draws that read no arena page. Vanilla
//! 26.3's block vertices have no normal (`DefaultVertexFormat.BLOCK`), so packs get one from the
//! quad itself. Beside them, each vertex's quad's block id and source word (`Entry::source`), for
//! `mc_Entity` and `at_midBlock` (spec §5.1), which flat draws and per-section draws reading
//! their page look up in storage.
use crate::mesher::{TerrainSource, TerrainVertex};
use glam::{Vec2, Vec3, Vec4};

/// The normal and tangent a quad of no area, or whose UVs span none, keeps: `FACE`'s constants,
/// as per-section draws without the stream give every vertex (`pack_atlas`'s `hz_Normal`,
/// `hz_tangent`).
const NORMAL: Vec3 = Vec3::Y;
const TANGENT: Vec4 = Vec4::new(1.0, 0.0, 0.0, 1.0);

/// The source word of a quad with no known source (none recorded, or not one a quad): id -1 and
/// no valid flag, as flat draws read an unknown source (`pack_atlas`'s `hz_known_source`).
const UNKNOWN: [u32; 2] = [u32::MAX, 0];

/// One vertex's entry in the stream, as `pack_atlas::NORMAL_ATTRIBUTES` reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Entry {
    /// The normal (w 0) and tangent (`pack`).
    pub(super) normal: [i8; 8],
    /// The quad's `mc_Entity` id under the pack's `block.properties`, and its source's word
    /// with the block's cell, fluid and valid flags and emitted light
    /// (`TerrainSource::with_id`); `UNKNOWN` without one.
    pub(super) source: [u32; 2],
}

/// Vertices `stream` works out at a time: a whole number of quads, so a quad never straddles
/// two of them; 16 KiB of entries.
pub(super) const CHUNK: usize = 1024;

/// `vertices`' normal stream into `out` (as many entries), one entry a vertex: each quad of
/// four consecutive vertices (the mesher's quads, which per-section draws index as `FACE` does)
/// its own (`quad`) and its source's (`sources[i]` for quad `i`, its id under `ids`), and a last
/// partial quad the constants and no source. `sources` holds one a quad or none (unknown).
pub(super) fn stream(
    vertices: &[TerrainVertex],
    sources: &[TerrainSource],
    ids: &[i32],
    out: &mut [Entry],
) {
    debug_assert_eq!(vertices.len(), out.len());
    let (quads, rest) = vertices.as_chunks::<4>();
    let (whole, partial) = out.as_chunks_mut::<4>();
    for (i, (corners, to)) in quads.iter().zip(whole).enumerate() {
        let source = sources.get(i).map_or(UNKNOWN, |s| s.with_id(ids));
        let normal = quad(corners);
        *to = [Entry { normal, source }; 4];
    }
    partial.fill(Entry {
        normal: pack(NORMAL, TANGENT),
        source: UNKNOWN,
    });
    debug_assert_eq!(partial.len(), rest.len());
}

/// A quad's normal and tangent from its first three corners, as `FACE` works them out: the
/// normal along `(p1 - p0) x (p2 - p0)`, the tangent where U grows across the face with the
/// handedness of the bitangent in its w, in the stream's bytes (`pack`).
fn quad(corners: &[TerrainVertex; 4]) -> [i8; 8] {
    let p = |i: usize| Vec3::from_array(corners[i].pos);
    let t = |i: usize| Vec2::new(f32::from(corners[i].uv[0]), f32::from(corners[i].uv[1]));
    let (e1, e2) = (p(1) - p(0), p(2) - p(0));
    let (d1, d2) = (t(1) - t(0), t(2) - t(0));
    let (mut normal, mut tangent) = (NORMAL, TANGENT);
    let n = e1.cross(e2);
    // NaN corners compare false, as in GLSL: the constants.
    if n.dot(n) > 1e-12 {
        normal = n.normalize();
        let r = glsl_sign(d1.x * d2.y - d2.x * d1.y);
        let t = (e1 * d2.y - e2 * d1.y) * r;
        let b = (e2 * d1.x - e1 * d2.x) * r;
        if t.dot(t) > 1e-12 {
            let t = (t - normal * normal.dot(t)).normalize();
            let w = if t.cross(normal).dot(b) < 0.0 {
                -1.0
            } else {
                1.0
            };
            tangent = t.extend(w);
        }
    }
    pack(normal, tangent)
}

/// GLSL's `sign`: 0 for 0, where `f32::signum` gives 1.
fn glsl_sign(x: f32) -> f32 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// The normal (w 0) and tangent as `Snorm8x4` bytes (`pack_atlas::NORMAL_ATTRIBUTES`): each
/// component times 127, rounded, which the vertex fetch reads back as the byte over 127.
fn pack(normal: Vec3, tangent: Vec4) -> [i8; 8] {
    let byte = |c: f32| (c.clamp(-1.0, 1.0) * 127.0).round() as i8;
    let [nx, ny, nz] = normal.to_array().map(byte);
    let [tx, ty, tz, tw] = tangent.to_array().map(byte);
    [nx, ny, nz, 0, tx, ty, tz, tw]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad of `corners` with UVs `uvs` (in `TerrainVertex::new`'s units: 0 to 1 a sprite).
    fn quad_of(corners: [[f32; 3]; 4], uvs: [[f32; 2]; 4]) -> [TerrainVertex; 4] {
        std::array::from_fn(|i| TerrainVertex::new(corners[i], uvs[i], 0, u32::MAX, 0))
    }

    const UVS: [[f32; 2]; 4] = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];

    /// A floor quad wound so that `(p1 - p0) x (p2 - p0)` points up: normal (0, 127, 0), U
    /// growing along +x, V along +z.
    #[test]
    fn an_up_facing_quad_streams_its_normal_and_tangent() {
        let up = quad_of(
            [[0., 1., 1.], [1., 1., 1.], [1., 1., 0.], [0., 1., 0.]],
            UVS,
        );
        assert_eq!(quad(&up), [0, 127, 0, 0, 127, 0, 0, 127]);
        // Its corners the other way round, the same UVs: facing down, U still along +x, V
        // along -z.
        let down = [up[3], up[2], up[1], up[0]];
        let down = quad_of(down.map(|v| v.pos), UVS);
        assert_eq!(quad(&down), [0, -127, 0, 0, 127, 0, 0, 127]);
    }

    /// The up quad with its U reversed (growing along -x): the tangent along -x and, the
    /// bitangent (V growing, +z) now on the other side of `tangent x normal` (-z), the other
    /// hand: w -127.
    #[test]
    fn a_mirrored_quad_streams_the_other_hand() {
        let mirrored = quad_of(
            [[0., 1., 1.], [1., 1., 1.], [1., 1., 0.], [0., 1., 0.]],
            [[1., 1.], [0., 1.], [0., 0.], [1., 0.]],
        );
        assert_eq!(quad(&mirrored), [0, 127, 0, 0, -127, 0, 0, -127]);
    }

    /// A face slanted 45 degrees about x: its normal (0, -1, 1) / sqrt(2), 90 of 127 a
    /// component (89.80 rounded); its tangent along +x.
    #[test]
    fn a_slanted_quad_streams_its_quantised_normal() {
        let slanted = quad_of(
            [[0., 0., 0.], [1., 0., 0.], [1., 1., 1.], [0., 1., 1.]],
            [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
        );
        assert_eq!(quad(&slanted), [0, -90, 90, 0, 127, 0, 0, 127]);
    }

    /// No area, or UVs of no area: the constants (0, 1, 0) and (1, 0, 0, 1).
    #[test]
    fn degenerate_quads_keep_the_constants() {
        let line = quad_of(
            [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.], [3., 0., 0.]],
            UVS,
        );
        assert_eq!(quad(&line), [0, 127, 0, 0, 127, 0, 0, 127]);
        // East-facing: the normal is real, the tangent the constant.
        let flat_uv = quad_of(
            [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]],
            [[0.5, 0.5]; 4],
        );
        assert_eq!(quad(&flat_uv), [127, 0, 0, 0, 127, 0, 0, 127]);
        let nan = quad_of([[f32::NAN; 3]; 4], UVS);
        assert_eq!(quad(&nan), [0, 127, 0, 0, 127, 0, 0, 127]);
    }

    /// Every vertex of a quad gets its quad's entry; a last partial quad the constants.
    #[test]
    fn the_stream_has_an_entry_a_vertex() {
        let east = quad_of(
            [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]],
            UVS,
        );
        let mut vertices = east.to_vec();
        vertices.push(east[0]);
        let mut out = [Entry {
            normal: [9; 8],
            source: [9; 2],
        }; 5];
        stream(&vertices, &[], &[], &mut out);
        // Facing east, U growing along +y; no source.
        let east = Entry {
            normal: [127, 0, 0, 0, 0, 127, 0, 127],
            source: UNKNOWN,
        };
        assert_eq!(out[..4], [east; 4]);
        let partial = Entry {
            normal: [0, 127, 0, 0, 127, 0, 0, 127],
            source: UNKNOWN,
        };
        assert_eq!(out[4], partial);
    }

    /// Each quad's entry holds its own source's id under the pack's ids and its word: a block's
    /// model id, a fluid's fluid id (the second of its state's two), -1 for a state past the
    /// table, and the cell, fluid flag, valid flag (1 << 13) and emitted light as recorded.
    #[test]
    fn each_quad_streams_its_source() {
        let east = quad_of(
            [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]],
            UVS,
        );
        let vertices: Vec<_> = (0..3).flat_map(|_| east).collect();
        // States 0 and 1: ids 5 and 50, then 6 and 60.
        let ids = [5, 50, 6, 60];
        let sources = [
            TerrainSource::new(1, [2, 3, 4], 7, false),
            TerrainSource::new(0, [15, 0, 9], 15, true),
            TerrainSource::new(2, [0, 0, 0], 0, false),
        ];
        let mut out = [Entry::default(); 12];
        stream(&vertices, &sources, &ids, &mut out);
        let words = [
            [6, 2 | 3 << 4 | 4 << 8 | 1 << 13 | 7 << 14],
            [50, 15 | 9 << 8 | 1 << 12 | 1 << 13 | 15 << 14],
            [u32::MAX, 1 << 13],
        ];
        for (quad, word) in out.chunks(4).zip(words) {
            assert_eq!(quad.iter().map(|e| e.source).collect::<Vec<_>>(), [word; 4]);
        }
    }
}
