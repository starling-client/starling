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
