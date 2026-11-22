//! Core terrain types, coordinate keys, and chunk visibility/fade calculations.

use alloc::vec::Vec;

/// A chunk section coordinate key `(x, y, z)`.
pub type SectionKey = (i32, i32, i32);

/// The default fixed-point UV division steps for terrain vertices.
pub const UV_STEPS: f32 = 32768.0;

/// `LevelRenderer.CHUNK_VISIBILITY_THRESHOLD`: a section's block entities wait for it.
pub const CHUNK_VISIBILITY_THRESHOLD: f32 = 0.3;

/// `SectionRenderDispatcher.RenderSection.getVisibility`: how far through its fade-in a section
/// uploaded `elapsed_ms` ago is.
pub fn chunk_visibility(elapsed_ms: u64, fade_ms: u64) -> f32 {
    if elapsed_ms >= fade_ms {
        1.0
    } else {
        elapsed_ms as f32 / fade_ms as f32
    }
}

/// `Util.toMillis`: `Mth.floor(seconds * 1000.0)`.
pub fn fade_millis(seconds: f32) -> u64 {
    (f64::from(seconds) * 1000.0).floor().max(0.0) as u64
}

/// A section's instance: its camera-relative origin and its UV step, negated
/// past its fade code while it fades in over far terrain (`lod::instance`).
pub fn section_instance(origin: [f32; 3], fade: u32, visibility: f32) -> [f32; 4] {
    let step = 1.0 / UV_STEPS;
    [
        origin[0],
        origin[1],
        origin[2],
        if fade == 0 {
            // The step is a power of two, so its mantissa is free: it holds the vanilla
            // chunk fade-in's `1 - visibility` in 23 bits (terrain.wgsl's `instance_step` and
            // `instance_visibility` read it back). A whole section is the bare step.
            let faded = ((1.0 - visibility) * 8_388_608.0)
                .round()
                .clamp(0.0, 8_388_607.0) as u32;
            f32::from_bits(step.to_bits() | faded)
        } else {
            -(fade as f32 + step)
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_visibility_boundaries() {
        assert_eq!(chunk_visibility(0, 500), 0.0);
        assert_eq!(chunk_visibility(250, 500), 0.5);
        assert_eq!(chunk_visibility(500, 500), 1.0);
        assert_eq!(chunk_visibility(1000, 500), 1.0);
    }

    #[test]
    fn fade_millis_calculation() {
        assert_eq!(fade_millis(0.5), 500);
        assert_eq!(fade_millis(1.0), 1000);
        assert_eq!(fade_millis(0.0), 0);
    }

    #[test]
    fn section_instance_unfaded() {
        let inst = section_instance([16.0, 32.0, 48.0], 0, 1.0);
        assert_eq!(inst[0], 16.0);
        assert_eq!(inst[1], 32.0);
        assert_eq!(inst[2], 48.0);
        assert_eq!(inst[3], 1.0 / UV_STEPS);
    }
}

/// The default flat normal map texel value `[128, 128, 255, 255]`.
pub const FLAT_NORMAL: [u8; 4] = [128, 128, 255, 255];

/// The default empty specular map texel value `[0, 0, 0, 0]`.
pub const NO_SPECULAR: [u8; 4] = [0, 0, 0, 0];

/// Sprite array layer flag for opaque leaves and quads over opaque texels.
pub const OPAQUE_LAYER: u16 = 1 << 15;

/// An entity, item and effect vertex (the entity pipelines).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
    pub color: u32,
    pub light: u32,
    pub normal: [i8; 4],
    pub glint_uv: [f32; 2],
}

/// Section-relative terrain vertex.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
    pub pos: [f32; 3],
    pub uv: [u16; 2],
    pub layer: u16,
    pub light: u16,
    pub color: u32,
}

#[inline]
fn round_to_u16(v: f32) -> u16 {
    let v = v.clamp(0.0, f32::from(u16::MAX));
    let whole = v as u32;
    (whole + u32::from(v - whole as f32 >= 0.5)) as u16
}

impl TerrainVertex {
    pub fn new(pos: [f32; 3], uv: [f32; 2], layer: u16, color: u32, light: u16) -> Self {
        Self {
            pos,
            uv: uv.map(|v| round_to_u16(v * UV_STEPS)),
            layer,
            color,
            light,
        }
    }
}

/// Source metadata for shader pack meshed terrain quads.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainSource {
    pub state: u32,
    pub cell: u32,
}

impl TerrainSource {
    pub fn new(state: u32, pos: [i32; 3], emission: u8, fluid: bool) -> Self {
        debug_assert!(pos.iter().all(|&v| (0..16).contains(&v)));
        Self {
            state,
            cell: pos[0] as u32
                | (pos[1] as u32) << 4
                | (pos[2] as u32) << 8
                | u32::from(fluid) << 12
                | 1 << 13
                | u32::from(emission.min(15)) << 14,
        }
    }
}

/// HDR tiers and light emission constants.
pub const LOW: u8 = 1;
pub const INCANDESCENT: u8 = 2;
pub const BRIGHT: u8 = 3;
pub const VERY_BRIGHT: u8 = 4;
pub const WHOLE: u8 = 8;

pub fn emitting(color: u32, tier: u8) -> u32 {
    let level = (tier & !WHOLE).min(VERY_BRIGHT);
    let code = if level > 0 && tier & WHOLE != 0 {
        level + 4
    } else {
        level
    };
    color | (u32::from(code) << 24)
}

/// Where a run of `Frame::entities` begins, and what it is (slice 7): the run lasts until the
/// next span. Recorded only while a shader pack draws entities, so Sunrise's own frames carry
/// none; Sunrise itself still draws the whole list at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntitySpan {
    /// The first vertex.
    pub start: u32,
    /// The render stage (5.4: `ENTITIES`, `BLOCK_ENTITIES`, `PARTICLES`, `DESTROY`), which picks
    /// the program.
    pub stage: u32,
    /// `entityId` for entities, `blockEntityId` for block entities (5.3), else 0.
    pub id: i32,
    /// An entity part that blends (a name tag, a shadow): `gbuffers_entities_translucent`.
    pub translucent: bool,
    /// Hidden from the camera (Sunrise's entity culling, the player in first person): only the
    /// shadow pass draws it.
    pub hidden: bool,
    /// The local player (`shadowPlayer`).
    pub player: bool,
    /// A block entity whose block emits light (`shadowLightBlockEntities`).
    pub emissive: bool,
}

impl EntitySpan {
    /// Marks vertices `start..end` of `spans`' list (one entity) as hidden: the span covering
    /// `start` is split there, the spans inside are hidden, and the geometry after `end` goes
    /// on in the last span's kind. False when no span covers `start`.
    pub fn hide(spans: &mut Vec<EntitySpan>, start: usize, end: usize) -> bool {
        let Some(i) = spans.iter().rposition(|s| s.start as usize <= start) else {
            return false;
        };
        let after = spans[spans.len() - 1];
        if spans[i].start as usize != start {
            let split = EntitySpan {
                start: start as u32,
                ..spans[i]
            };
            spans.insert(i + 1, split);
        }
        for s in spans.iter_mut().skip(i) {
            if s.start as usize >= start {
                s.hidden = true;
            }
        }
        if end > start {
            spans.push(EntitySpan {
                start: end as u32,
                ..after
            });
        }
        true
    }
}

/// One unsplit camera-relative weather vertex. UV animation and atlas-local
/// repetition happen in the weather shader, rather than rebuilding tiled quads.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WeatherVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub rect: [f32; 4],
    pub animation: [f32; 4],
    pub bases: [u32; 4],
    pub color: u32,
    pub light: u32,
    pub snow: u32,
}

/// Current ticks are masked on the CPU before conversion to float, including
/// negative/large tick values.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WeatherTime {
    pub values: [f32; 4],
}

impl WeatherTime {
    pub fn new(ticks: i64, partial: f32) -> Self {
        Self {
            values: [(ticks & 131_071) as f32, (ticks & 511) as f32, partial, 0.0],
        }
    }
}
