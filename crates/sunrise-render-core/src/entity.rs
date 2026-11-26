//! Entity vertex and overlay layout primitives.

use crate::EntitySpan;
use bytemuck::{Pod, Zeroable};

pub const TRANSLUCENT_ITEM_OVERLAY: u32 = 32;
pub const TRANSLUCENT_GLINT_ITEM_OVERLAY: u32 = 33;

/// An entity, item and effect vertex (the entity pipelines), with a face
/// normal for `entity.wgsl`'s directional light.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct EntityVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
    /// RGBA8: tint multiplied by directional shade and ambient occlusion.
    pub color: u32,
    /// Smooth light, 16x fixed point: sky in bits 0..8, block in bits 8..16.
    pub light: u32,
    /// RGBA8_SNORM face normal; w = -127 for two-sided lighting, 0 to keep baked/unlit colour,
    /// `CULL_BACK` to keep it and also drop back-facing pixels.
    pub normal: [i8; 4],
    /// The `GLINT_TEXTURING` atlas UV for a foil item quad; zero elsewhere.
    pub glint_uv: [f32; 2],
}

pub const FLUID_OVERLAY: u32 = 41;
pub const TERRAIN_MODEL_SOLID_OVERLAY: u32 = 42;
pub const TERRAIN_MODEL_CUTOUT_OVERLAY: u32 = 43;
pub const TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY: u32 = 44;
/// `RenderTypes.translucentMovingBlock`: a translucent block drawn outside the terrain, from the
/// terrain sprite arrays; alpha blended, depth written, quads sorted far to near.
pub const TERRAIN_MODEL_TRANSLUCENT_OVERLAY: u32 = 46;
/// `RenderTypes.patternedShieldGlint`: equal-depth, no-depth-write glint-only overlay pass.
pub const GLINT_ONLY_OVERLAY: u32 = 48;
/// `RenderTypes.bannerPattern`: translucent-blended, no-depth-write, no-alpha-cutout pattern pass.
pub const BANNER_PATTERN_OVERLAY: u32 = 49;
/// Opaque `Font.DisplayMode.POLYGON_OFFSET` text (`SubmitNodeCollection.canRenderAsSolid`).
pub const TEXT_POLYGON_OFFSET_SOLID_OVERLAY: u32 = 50;
/// Opaque `Font.DisplayMode.NORMAL` text, including the outline behind a sign's offset fill.
pub const TEXT_NORMAL_SOLID_OVERLAY: u32 = 51;
/// `RenderTypes.entitySolid(texture)`: a quad of a client mod's dynamic texture
/// (`BlockEntityDrawContext::textured_quad`), opaque, in submission order.
pub const DYNAMIC_TEXTURE_SOLID_OVERLAY: u32 = 52;
/// `RenderTypes.entityCutout(texture)`: as [`DYNAMIC_TEXTURE_SOLID_OVERLAY`], alpha under 0.1
/// discarded.
pub const DYNAMIC_TEXTURE_CUTOUT_OVERLAY: u32 = 53;
/// `RenderTypes.entityTranslucent(texture)`: as [`DYNAMIC_TEXTURE_SOLID_OVERLAY`], alpha
/// blended, depth written, quads sorted far to near.
pub const DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY: u32 = 54;
/// Armor foil without its color material: additive, with no depth or outline writes.
pub const ARMOR_GLINT_ONLY_OVERLAY: u32 = 55;
/// The overlays of a name tag's `Font.DisplayMode.NORMAL` copy when a pack's `core/text` draws
/// it (`RenderTypes.text`, `RenderPipelines.TEXT`): opaque on no background, which
/// `SubmitNodeCollection.canRenderAsSolid` puts in the solid phase, and translucent (a discrete
/// tag on its background; `OIT_TEXT` under Improved Transparency). Their own materials, stored
/// last, which a world view draws after the native entity layer. A `shaders/` pack's shadow pass
/// takes them with the rest of their entity's run (`pack_entities` spans), whatever the layer.
pub const NAME_TAG_SOLID_OVERLAY: u32 = 57;
pub const NAME_TAG_OVERLAY: u32 = 56;
pub const WATER_MASK_OVERLAY: u32 = 13;
pub const TRANSLUCENT_PARTICLE_OVERLAY: u32 = 15;
pub const OIT_ENTITY: u32 = 1 << 24;

/// Native terrain-array UV/payloads must never enter a shared-atlas shader-pack pass.
pub fn is_native_terrain(layer: u32) -> bool {
    matches!(
        layer & 0xffff,
        FLUID_OVERLAY
            | TERRAIN_MODEL_SOLID_OVERLAY
            | TERRAIN_MODEL_CUTOUT_OVERLAY
            | TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY
            | TERRAIN_MODEL_TRANSLUCENT_OVERLAY
    )
}

/// Whether `layer` is a quad of a dynamic texture (`DYNAMIC_TEXTURE_*_OVERLAY`).
pub fn is_dynamic_texture(layer: u32) -> bool {
    matches!(
        layer & 0xffff,
        DYNAMIC_TEXTURE_SOLID_OVERLAY
            | DYNAMIC_TEXTURE_CUTOUT_OVERLAY
            | DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY
    )
}

/// The `glint_uv` of a dynamic-texture quad: the texture's handle as two words' bits. No shader
/// reads `glint_uv` of these quads, and the batch's regrouping keeps it with its vertices, so
/// each quad names its texture wherever it lands in the stream. Every word is a finite float.
pub fn dynamic_texture_bits(handle: u64) -> [f32; 2] {
    let word = |shift: u32| f32::from_bits(((handle >> shift) & 0x007f_ffff) as u32);
    // 23 bits to a word: handles count up from 1, so 46 bits last longer than the process.
    [word(0), word(23)]
}

/// The handle `dynamic_texture_bits` wrote into `glint_uv`.
pub fn dynamic_texture_handle(glint_uv: [f32; 2]) -> u64 {
    u64::from(glint_uv[0].to_bits()) | u64::from(glint_uv[1].to_bits()) << 23
}

/// One run of consecutive dynamic-texture quads of one overlay and one texture, as
/// [`dynamic_texture_runs`] reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynamicTextureRun {
    pub overlay: u32,
    pub handle: u64,
    /// The first quad, counted in quads from the start of the slice.
    pub first: u32,
    pub quads: u32,
}

/// `vertices`' dynamic-texture quads as runs: consecutive quads of the same overlay and texture
/// extend one run, and any other quad ends it.
pub fn dynamic_texture_runs(
    vertices: &[EntityVertex],
) -> impl Iterator<Item = DynamicTextureRun> + '_ {
    let mut quads = vertices.as_chunks::<4>().0.iter().enumerate().peekable();
    core::iter::from_fn(move || {
        let (first, quad) = quads.next()?;
        let overlay = quad[0].layer & 0xffff;
        let handle = dynamic_texture_handle(quad[0].glint_uv);
        let mut count = 1;
        while let Some((_, next)) = quads.peek() {
            if next[0].layer & 0xffff != overlay
                || dynamic_texture_handle(next[0].glint_uv) != handle
            {
                break;
            }
            quads.next();
            count += 1;
        }
        Some(DynamicTextureRun {
            overlay,
            handle,
            first: first as u32,
            quads: count,
        })
    })
}

/// `MeshData.decodeQuadCentroids` + `VertexSorting.DISTANCE_TO_ORIGIN`:
/// JOML `Vector3f(0, 0, 0).distanceSquared(mid)` using JOML's right-to-left unfused `Math.fma`.
#[inline]
pub fn quad_distance_sq(q: &[EntityVertex; 4]) -> f32 {
    let dx = 0.0 - (q[0].pos[0] + q[2].pos[0]) * 0.5;
    let dy = 0.0 - (q[0].pos[1] + q[2].pos[1]) * 0.5;
    let dz = 0.0 - (q[0].pos[2] + q[2].pos[2]) * 0.5;
    dx * dx + (dy * dy + dz * dz)
}

/// Sorts quads far to near by [`quad_distance_sq`] (`VertexSorting.DISTANCE_TO_ORIGIN`), quads at
/// one distance in the order they came, through buffers kept between frames so a sort allocates
/// nothing once warm.
#[derive(Default)]
pub struct QuadSort {
    /// Each quad's distance and its place before the sort, in the sorted order.
    keys: alloc::vec::Vec<(f32, u32)>,
    quads: alloc::vec::Vec<[EntityVertex; 4]>,
}

impl QuadSort {
    /// Sorts `quads` in place: the order a stable sort gives, by an unstable one on
    /// (distance, original index).
    pub fn sort(&mut self, quads: &mut [[EntityVertex; 4]]) {
        self.keys.clear();
        self.keys.extend(
            quads
                .iter()
                .enumerate()
                .map(|(i, q)| (quad_distance_sq(q), i as u32)),
        );
        if quads.len() <= 1 {
            return;
        }
        self.keys
            .sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        self.quads.clear();
        self.quads.extend_from_slice(quads);
        for (quad, &(_, from)) in quads.iter_mut().zip(&self.keys) {
            *quad = self.quads[from as usize];
        }
    }

    /// After [`QuadSort::sort`]: for each quad in its sorted place, the place it came from.
    pub fn sources(&self) -> impl Iterator<Item = u32> + '_ {
        self.keys.iter().map(|&(_, from)| from)
    }
}

/// Intersects shader identities with overlay runs, without reading posed vertices back.
pub fn skin_runs(
    skin_jobs: impl IntoIterator<Item = (u32, u32, u32)>, // (dst, count, overlay)
    skin_spans: &[EntitySpan],
) -> alloc::vec::Vec<(EntitySpan, core::ops::Range<u32>, u32)> {
    let mut out: alloc::vec::Vec<(EntitySpan, core::ops::Range<u32>, u32)> = alloc::vec::Vec::new();
    let mut span_index = 0;
    for (job_dst, job_count, job_overlay) in skin_jobs {
        let mut at = job_dst;
        let end = at + job_count;
        while at < end {
            while span_index + 1 < skin_spans.len() && skin_spans[span_index + 1].start <= at {
                span_index += 1;
            }
            let Some(&span) = skin_spans.get(span_index) else {
                break;
            };
            let stop = end.min(skin_spans.get(span_index + 1).map_or(end, |s| s.start));
            if stop <= at {
                break;
            }
            if let Some((old, range, overlay)) = out.last_mut()
                && (EntitySpan { start: 0, ..*old }) == (EntitySpan { start: 0, ..span })
                && *overlay == job_overlay
                && range.end == at
            {
                range.end = stop;
            } else {
                out.push((span, at..stop, job_overlay));
            }
            at = stop;
        }
    }
    out
}

/// An item of skin runs: a run of posed model vertices with a destination offset, count, and overlay.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct SkinRunJob {
    pub dst: u32,
    pub count: u32,
    pub overlay: u32,
}

/// A lightweight representation of posed model runs for shader pack entity preparation.
#[derive(Clone, Copy, Debug, Default)]
pub struct PosedModelRuns<'a> {
    pub vertices: u32,
    pub jobs: &'a [SkinRunJob],
    pub spans: &'a [EntitySpan],
}

/// The overlay code of white flashes: `WHITE + u`, u in 1..=15.
pub const WHITE_OVERLAY: u32 = 16;
/// The hurt red (rows 0 to 7) as RGBA bytes: 0xB2FF0000.
pub const HURT_TEXEL: [u8; 4] = [255, 0, 0, 0xB2];

/// The white row's alpha at column u: `(int) ((1 - u / 15 * 0.75) * 255)`
/// in float precision.
pub fn white_alpha(u: u32) -> u8 {
    ((1.0f32 - u as f32 / 15.0 * 0.75) * 255.0) as u8
}

/// The light bytes of a crack vertex carrying the origin of the destroy stage sprite.
pub fn stage_light([u0, v0]: [f32; 2]) -> u32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 65535.0).round() as u32;
    q(u0) | (q(v0) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(overlay: u32, handle: u64) -> [EntityVertex; 4] {
        [EntityVertex {
            pos: [0.0; 3],
            uv: [0.0; 2],
            layer: overlay,
            color: 0,
            light: 0,
            normal: [0; 4],
            glint_uv: dynamic_texture_bits(handle),
        }; 4]
    }

    #[test]
    fn a_handle_survives_its_bits_and_every_word_is_finite() {
        for handle in [1, 2, 0x7f_ffff, 0x80_0000, 0x1234_5678_9abc] {
            let bits = dynamic_texture_bits(handle);
            assert!(bits.iter().all(|w| w.is_finite()));
            assert_eq!(dynamic_texture_handle(bits), handle);
        }
    }

    #[test]
    fn consecutive_quads_of_one_texture_make_one_run() {
        let vertices: alloc::vec::Vec<EntityVertex> = [
            quad(DYNAMIC_TEXTURE_CUTOUT_OVERLAY, 7),
            quad(DYNAMIC_TEXTURE_CUTOUT_OVERLAY, 7),
            quad(DYNAMIC_TEXTURE_CUTOUT_OVERLAY, 9),
        ]
        .concat();
        let runs: alloc::vec::Vec<_> = dynamic_texture_runs(&vertices).collect();
        assert_eq!(
            runs,
            [
                DynamicTextureRun {
                    overlay: DYNAMIC_TEXTURE_CUTOUT_OVERLAY,
                    handle: 7,
                    first: 0,
                    quads: 2
                },
                DynamicTextureRun {
                    overlay: DYNAMIC_TEXTURE_CUTOUT_OVERLAY,
                    handle: 9,
                    first: 2,
                    quads: 1
                },
            ]
        );
    }

    #[test]
    fn another_overlay_ends_a_run_of_the_same_texture() {
        let vertices: alloc::vec::Vec<EntityVertex> = [
            quad(DYNAMIC_TEXTURE_SOLID_OVERLAY, 7),
            quad(DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY, 7),
        ]
        .concat();
        assert_eq!(dynamic_texture_runs(&vertices).count(), 2);
    }
}
