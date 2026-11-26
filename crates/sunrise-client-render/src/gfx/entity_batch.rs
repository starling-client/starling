//! Stable entity material grouping with renderer-owned, reusable CPU scratch.
use super::Vertex;
use sunrise_render_core::entity::QuadSort;
#[cfg(test)]
use sunrise_render_core::entity::quad_distance_sq;
pub use sunrise_render_core::entity::{
    ARMOR_GLINT_ONLY_OVERLAY, DYNAMIC_TEXTURE_CUTOUT_OVERLAY, DYNAMIC_TEXTURE_SOLID_OVERLAY,
    DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY, TEXT_NORMAL_SOLID_OVERLAY,
    TEXT_POLYGON_OFFSET_SOLID_OVERLAY, dynamic_texture_bits,
};
pub use sunrise_render_core::entity::{NAME_TAG_OVERLAY, NAME_TAG_SOLID_OVERLAY};

/// Device-derived vertex and index byte capacities governing the combined entity batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityQuadCapacity {
    /// Maximum byte size of the combined entity vertex buffer (`max_buffer_size`, narrowed to
    /// `max_storage_buffer_binding_size` when GPU entity skinning binds it as a storage buffer).
    pub vertex_bytes: u64,
    /// Maximum byte size of the shared quad index buffer (`max_buffer_size`).
    pub index_bytes: u64,
}

impl EntityQuadCapacity {
    /// Bytes occupied by one complete quad (4 vertices) in the entity vertex buffer.
    pub const VERTEX_QUAD_BYTES: u64 = (4 * std::mem::size_of::<Vertex>()) as u64;
    /// Bytes occupied by one complete quad (6 `u32` indices) in the quad index buffer.
    pub const INDEX_QUAD_BYTES: u64 = (6 * std::mem::size_of::<u32>()) as u64;
    /// Maximum quads addressable by `draw_indexed` (`i32` base vertex and `u32` index count).
    pub const MAX_ADDRESSABLE_QUADS: u64 = {
        let by_base_vertex = i32::MAX as u64 / 4;
        let by_index = u32::MAX as u64 / 6;
        if by_base_vertex < by_index {
            by_base_vertex
        } else {
            by_index
        }
    };

    /// Derives the combined entity batch capacity from the active device limits.
    pub fn from_limits(limits: &wgpu::Limits, storage_bound: bool) -> Self {
        let max_buffer = limits.max_buffer_size;
        let vertex_bytes = if storage_bound {
            max_buffer.min(limits.max_storage_buffer_binding_size)
        } else {
            max_buffer
        };
        Self {
            vertex_bytes,
            index_bytes: max_buffer,
        }
    }

    /// Capacity for a device whose vertex and index buffers share `max_buffer_size`.
    pub const fn from_max_buffer_size(max_buffer_size: u64) -> Self {
        Self {
            vertex_bytes: max_buffer_size,
            index_bytes: max_buffer_size,
        }
    }

    /// Baseline capacity matching `wgpu::Limits::default()` when no GPU device is attached.
    pub fn default_device() -> Self {
        Self::from_limits(&wgpu::Limits::default(), false)
    }

    /// Maximum complete quads that fit in both the entity vertex buffer and the quad index buffer.
    pub fn max_quads(self) -> usize {
        let by_vertex = self.vertex_bytes / Self::VERTEX_QUAD_BYTES;
        let by_index = self.index_bytes / Self::INDEX_QUAD_BYTES;
        let quads = by_vertex.min(by_index).min(Self::MAX_ADDRESSABLE_QUADS);
        usize::try_from(quads).unwrap_or(usize::MAX / 4)
    }

    /// Maximum complete vertices corresponding to [`Self::max_quads`].
    pub fn max_vertices(self) -> usize {
        self.max_quads().saturating_mul(4)
    }

    /// Maximum quads the shared `u32` quad index buffer can hold within `index_bytes`.
    pub fn max_index_quads(self) -> u32 {
        let by_index = self.index_bytes / Self::INDEX_QUAD_BYTES;
        let quads = by_index.min(u32::MAX as u64 / 6);
        u32::try_from(quads).unwrap_or(u32::MAX / 6)
    }

    /// Index buffer quad capacity grown to the next power of two without exceeding `index_bytes`.
    pub fn index_buffer_quads(self, quads: u32) -> u32 {
        let most = self.max_index_quads();
        if quads == 0 || most == 0 {
            return 0;
        }
        let needed = quads.min(most);
        needed
            .checked_next_power_of_two()
            .unwrap_or(most)
            .min(most)
            .max(needed)
    }
}

fn quad_is_finite(quad: &[Vertex; 4]) -> bool {
    quad.iter().all(|v| {
        v.pos.into_iter().all(f32::is_finite)
            && v.uv.into_iter().all(f32::is_finite)
            && v.glint_uv.into_iter().all(f32::is_finite)
    })
}

/// Scoped submission sink for complete material-tagged quads entering the renderer's combined
/// entity vertex and index buffers.
///
/// Validates each quad's vertex coordinates and UVs as finite and admits only complete quads that
/// fit within [`EntityQuadCapacity`] after reserving `reserved_quads` for other draw sources in
/// the same combined batch.
pub struct EntityQuadScope<'a> {
    vertices: &'a mut Vec<Vertex>,
    vertex_limit: usize,
}

impl<'a> EntityQuadScope<'a> {
    /// Opens a submission scope appending complete quads to `vertices`, bounded by `capacity`
    /// minus `reserved_quads` already occupied by other draw sources in the same batch.
    pub fn new(
        vertices: &'a mut Vec<Vertex>,
        capacity: EntityQuadCapacity,
        reserved_quads: usize,
    ) -> Self {
        let quad_limit = capacity.max_quads().saturating_sub(reserved_quads);
        Self {
            vertices,
            vertex_limit: quad_limit.saturating_mul(4),
        }
    }

    /// The vertices appended so far, for a caller that appends a run of its own and then
    /// finishes it with [`Self::retain_valid_since`].
    pub fn vertices_mut(&mut self) -> &mut Vec<Vertex> {
        self.vertices
    }

    /// Whether at least one more complete quad fits in this scope's remaining batch budget.
    pub fn has_room(&self) -> bool {
        self.vertices
            .len()
            .checked_add(4)
            .is_some_and(|end| end <= self.vertex_limit)
    }

    /// Complete quads that can still be appended to `self.vertices` within this batch's budget.
    pub fn remaining_quads(&self) -> usize {
        self.vertex_limit.saturating_sub(self.vertices.len()) / 4
    }

    /// Submits one complete material-tagged quad.
    ///
    /// Returns `true` if further quads may still be submitted (including when this quad was
    /// skipped for non-finite coordinates or UVs), or `false` once the device-derived batch
    /// capacity is full.
    pub fn submit_quad(&mut self, quad: [Vertex; 4]) -> bool {
        if !self.has_room() {
            return false;
        }
        if !quad_is_finite(&quad) {
            return true;
        }
        self.vertices.extend_from_slice(&quad);
        self.has_room()
    }

    /// Truncates the contribution starting at `start` to complete quads within the remaining
    /// budget. Earlier geometry (and the caller's offsets into it) belongs to the caller.
    pub fn truncate_to_budget_since(&mut self, start: usize) {
        let end = self.vertices.len().min(self.vertex_limit);
        self.vertices
            .truncate(start + end.saturating_sub(start) / 4 * 4);
    }

    /// Compacts `vertices[start..]` in place to retain only complete finite quads within this
    /// scope's budget.
    pub fn retain_valid_since(&mut self, start: usize) {
        let start = start.min(self.vertices.len());
        let mut write = start;
        let mut read = start;
        let end = self.vertices.len();
        while read + 4 <= end && write + 4 <= self.vertex_limit {
            let quad = [
                self.vertices[read],
                self.vertices[read + 1],
                self.vertices[read + 2],
                self.vertices[read + 3],
            ];
            if quad_is_finite(&quad) {
                if write != read {
                    self.vertices[write..write + 4].copy_from_slice(&quad);
                }
                write += 4;
            }
            read += 4;
        }
        self.vertices.truncate(write);
    }
}

/// Complete quad counts admitted into [`EntityBatch::combined`] by [`EntityBatch::update_within`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EntityBatchCounts {
    pub(super) materials: [u32; MATERIALS],
    pub(super) admitted_entity_vertices: usize,
    pub(super) additive: u32,
    pub(super) hand: u32,
    pub(super) see_through: u32,
    pub(super) shadow_only: u32,
}

/// Entity materials by a quad's overlay (`Vertex::layer`), in draw order:
/// ordinary entities, world text drawn with a depth bias (35,
/// `RenderPipelines.TEXT_POLYGON_OFFSET`), breaking cracks (11), translucent
/// breaking cracks (12), the boat water mask (13, `RenderPipelines.WATER_MASK`),
/// energy swirls (34, `RenderPipelines.ENERGY_SWIRL`), entity shadows (14,
/// `RenderPipelines.ENTITY_SHADOW`), translucent-layer particles (15, drawn
/// after translucent terrain: `SubmitNodeCollection.submitQuadParticleGroup`)
/// and translucent item quads (32, drawn without depth writes).
///
/// With improved transparency on, quads tagged `OIT_ENTITY` are a tenth
/// material at the end: the render types vanilla gives an `OitPipelineSet`
/// (translucent, emissive, eyes and breeze wind entity models). Off, the tag is
/// ignored and they stay in the first one, in their order.
/// Sorted still-fluid geometry follows, then the two native world-canvas text materials,
/// NORMAL and POLYGON_OFFSET, the four native terrain model materials, then (stored after them,
/// materials 21 to 23 by index) the three dynamic-texture materials
/// (`RenderTypes.entitySolid`, `entityCutout`, `entityTranslucent` of a texture), banner patterns,
/// glint-only overlays, solid `POLYGON_OFFSET` and `NORMAL` text
/// (`SubmitNodeCollection.canRenderAsSolid`), then a pack-drawn name tag's solid (57) and
/// translucent (56) copies (materials 24 and 25, stored last).
/// Material storage order is independent of their draw stages.
const MATERIALS: usize = 26;
// A subrange of the energy swirl pass, also drawn by secondary world views.
const ARMOR_FOIL: usize = MATERIALS;
/// The dynamic-texture materials, solid, cutout and translucent (`MATERIALS` order).
pub(super) const DYNAMIC_MATERIALS: std::ops::Range<usize> = 21..24;
/// A pack-drawn name tag's materials, solid then translucent.
const NAME_TAG_MATERIALS: std::ops::Range<usize> = 24..26;
/// Materials stored in [`EntityBatch::combined`] ahead of the dynamic-texture ones.
const BEFORE_DYNAMIC: usize = 17;

/// The first quad of the dynamic-texture materials in [`EntityBatch::combined`]: the quads of
/// every material stored ahead of them, whatever is stored after (text, name tags).
pub(super) fn dynamic_texture_start(materials: &[u32; MATERIALS]) -> u32 {
    materials[..BEFORE_DYNAMIC].iter().sum()
}
/// Every named overlay id (a constant), each listed once: a new named overlay that reuses one of
/// them fails to build instead of drawing as the other. The codes `entity.wgsl` reads only as
/// literals (1, 2, 4 to 12, 36) are not listed.
const _: () = assert!(distinct(&[
    WATER_MASK_OVERLAY,
    SHADOW_OVERLAY,
    TRANSLUCENT_PARTICLE_OVERLAY,
    TRANSLUCENT_ITEM_OVERLAY,
    TRANSLUCENT_GLINT_ITEM_OVERLAY,
    ENERGY_SWIRL_OVERLAY,
    TEXT_POLYGON_OFFSET_OVERLAY,
    DRAGON_RAYS_OVERLAY,
    DISSOLVE_OVERLAY,
    WORLD_TEXT_OVERLAY,
    WORLD_TEXT_POLYGON_OFFSET_OVERLAY,
    FLUID_OVERLAY,
    TERRAIN_MODEL_SOLID_OVERLAY,
    TERRAIN_MODEL_CUTOUT_OVERLAY,
    TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY,
    ADDITIVE_PARTICLE_OVERLAY,
    TERRAIN_MODEL_TRANSLUCENT_OVERLAY,
    super::EXPLICIT_PARTICLE_OVERLAY,
    GLINT_ONLY_OVERLAY,
    BANNER_PATTERN_OVERLAY,
    TEXT_POLYGON_OFFSET_SOLID_OVERLAY,
    TEXT_NORMAL_SOLID_OVERLAY,
    DYNAMIC_TEXTURE_SOLID_OVERLAY,
    DYNAMIC_TEXTURE_CUTOUT_OVERLAY,
    DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY,
    ARMOR_GLINT_ONLY_OVERLAY,
    NAME_TAG_OVERLAY,
    NAME_TAG_SOLID_OVERLAY,
]));
/// Whether no two of `ids` are equal, and none is a white-flash code (`WHITE_OVERLAY`, 16, to 31).
const fn distinct(ids: &[u32]) -> bool {
    let mut i = 0;
    while i < ids.len() {
        if ids[i] >= 16 && ids[i] < 32 {
            return false;
        }
        let mut j = i + 1;
        while j < ids.len() {
            if ids[i] == ids[j] {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}
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
/// The overlay of a resource pack's Polytone custom particle drawn additively
/// (`additive_translucent`): fog tints its colour as vanilla's particle shader does, which the
/// other additive geometry does not take.
pub const ADDITIVE_PARTICLE_OVERLAY: u32 = 45;
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
/// Sorted depth-writing still fluid, sampling terrain sprite arrays rather than the shared atlas.
pub const FLUID_OVERLAY: u32 = 41;
/// A bit of `Vertex::layer`, above the overlay (low 16 bits) and the HDR tier
/// (bits 16 to 23): the quad's render type has an `OitPipelineSet`
/// (`RenderTypes.entityTranslucent`, `entityTranslucentEmissive`, `eyes`,
/// `breezeWind`). Only the improved transparency path reads it.
pub const OIT_ENTITY: u32 = 1 << 24;
/// A 1-bit parity flag toggled between consecutive strictly ordered draws in the
/// translucent block/item bucket (`FeatureRenderDispatcher.PhaseSubmitGrouper`)
/// so classic upload sorting never mixes quads across feature or render-type boundaries.
/// Stripped when writing `EntityBatch::combined`.
pub const TRANSLUCENT_DRAW_FLIP: u32 = 1 << 25;
/// `RenderPipelines.DRAGON_RAYS`: untextured colour, additive with depth writes.
pub const DRAGON_RAYS_OVERLAY: u32 = 37;
/// `RenderPipelines.ENTITY_CUTOUT_DISSOLVE` (the dying dragon): a pixel is dropped where the
/// vertex alpha is below the mask's, whose atlas UV rides in `Vertex::glint_uv`; the rest is
/// drawn opaque. No overlay.
pub const DISSOLVE_OVERLAY: u32 = 38;
/// Tags quads as drawn by a render type vanilla gives an `OitPipelineSet`
/// (`OIT_ENTITY`): improved transparency draws them in its stages.
pub fn mark_oit(quads: &mut [Vertex]) {
    for vertex in quads {
        if !is_native_terrain(vertex.layer) {
            vertex.layer |= OIT_ENTITY;
        }
    }
}
/// The overlay of depth-only quads that hide water (`RenderTypes.waterMask`).
pub const WATER_MASK_OVERLAY: u32 = 13;
/// The overlay of energy swirl quads (`RenderTypes.energySwirl`: the charged creeper's and the
/// powered wither's armour). The light bytes carry the sprite's origin and the colour bytes its
/// size (`breaking::stage_light`), the UV the scrolled position in the sprite.
pub const ENERGY_SWIRL_OVERLAY: u32 = 34;
/// The overlay of entity shadow quads (`RenderTypes.entityShadow`).
pub const SHADOW_OVERLAY: u32 = 14;
/// The overlay of translucent-layer particles (`SingleQuadParticle.Layer.translucent`).
pub const TRANSLUCENT_PARTICLE_OVERLAY: u32 = 15;
/// The overlay of item quads with translucent pixels (`Sheets.translucentItemSheet`,
/// `RenderPipelines.ITEM_TRANSLUCENT`); past the creeper swell range (16 to 31).
pub const TRANSLUCENT_ITEM_OVERLAY: u32 = 32;
/// A translucent item with an enchantment glint.
pub const TRANSLUCENT_GLINT_ITEM_OVERLAY: u32 = 33;
/// The overlay of text in `Font.DisplayMode.POLYGON_OFFSET` (text displays):
/// drawn like an ordinary entity, pulled toward the camera.
pub const TEXT_POLYGON_OFFSET_OVERLAY: u32 = 35;
/// A native world canvas, drawn after particles with normal text depth/culling.
pub const WORLD_TEXT_OVERLAY: u32 = 39;
/// The front of its eight-offset outlined text, using POLYGON_OFFSET.
pub const WORLD_TEXT_POLYGON_OFFSET_OVERLAY: u32 = 40;

/// Ordinary vertices before every prepared cut, in one walk of the pre-batch stream.
///
/// `EntityBatch::update` packs ordinary material first and draws the rest in later
/// passes, so a cut recorded in the original stream skips what moved. A cut at or
/// past the cursor continues that walk. A cut before the cursor restarts: staging
/// does not promise non-decreasing cuts, and a backwards cut must still match a
/// walk of that cut alone. Returns how many quads were read.
pub(super) fn ordinary_vertices_at_cuts(
    vertices: &[Vertex],
    cuts: impl IntoIterator<Item = u32>,
    oit: bool,
    out: &mut Vec<u32>,
) -> u64 {
    out.clear();
    let quads = vertices.as_chunks::<4>().0;
    let mut cursor = 0usize;
    let mut ordinary = 0u32;
    let mut walked = 0u64;
    for cut in cuts {
        let target = (cut as usize).min(vertices.len()) / 4;
        if target < cursor {
            cursor = 0;
            ordinary = 0;
        }
        while cursor < target {
            if material(quads[cursor][0].layer, oit) == 0 {
                ordinary += 4;
            }
            cursor += 1;
            walked += 1;
        }
        out.push(ordinary);
    }
    walked
}

/// The material of a quad by its overlay (`MATERIALS` order). The overlay is the
/// layer's low 16 bits, as entity.wgsl reads it; the HDR tier rides above that.
fn material(layer: u32, oit: bool) -> usize {
    match layer & 0xffff {
        TERRAIN_MODEL_SOLID_OVERLAY => return 13,
        TERRAIN_MODEL_CUTOUT_OVERLAY => return 14,
        TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY => return 15,
        TERRAIN_MODEL_TRANSLUCENT_OVERLAY => return 16,
        BANNER_PATTERN_OVERLAY => return 17,
        GLINT_ONLY_OVERLAY => return 18,
        TEXT_POLYGON_OFFSET_SOLID_OVERLAY => return 19,
        TEXT_NORMAL_SOLID_OVERLAY => return 20,
        DYNAMIC_TEXTURE_SOLID_OVERLAY => return DYNAMIC_MATERIALS.start,
        DYNAMIC_TEXTURE_CUTOUT_OVERLAY => return DYNAMIC_MATERIALS.start + 1,
        DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY => return DYNAMIC_MATERIALS.start + 2,
        NAME_TAG_SOLID_OVERLAY => return NAME_TAG_MATERIALS.start,
        NAME_TAG_OVERLAY => return NAME_TAG_MATERIALS.start + 1,
        ARMOR_GLINT_ONLY_OVERLAY => return ARMOR_FOIL,
        _ => {}
    }
    if layer & 0xffff == FLUID_OVERLAY {
        return 10;
    }
    if oit && layer & OIT_ENTITY != 0 {
        return 9;
    }
    match layer & 0xffff {
        WORLD_TEXT_OVERLAY => 11,
        WORLD_TEXT_POLYGON_OFFSET_OVERLAY => 12,
        TEXT_POLYGON_OFFSET_OVERLAY => 1,
        11 => 2,
        12 => 3,
        WATER_MASK_OVERLAY => 4,
        ENERGY_SWIRL_OVERLAY => 5,
        SHADOW_OVERLAY => 6,
        TRANSLUCENT_PARTICLE_OVERLAY => 7,
        TRANSLUCENT_ITEM_OVERLAY | TRANSLUCENT_GLINT_ITEM_OVERLAY => 8,
        _ => 0,
    }
}

#[derive(Default)]
pub(crate) struct EntityBatch {
    // Runs of consecutive quads of one material (first quad, quad count): a
    // model's quads share one, so whole runs are copied at once. Cleared each
    // frame, keeping their storage.
    runs: [Vec<(usize, usize)>; MATERIALS + 1],
    pub(super) combined: Vec<Vertex>,
    /// Foil-only quads in `combined`, immediately after the energy swirls.
    pub(super) armor_foil: std::ops::Range<u32>,
    sort: QuadSort,
    /// Additive quads tagged `OIT_ENTITY` that the last `update` moved into the
    /// last material (improved transparency on): the dragon's eyes, which share
    /// the additive buffer with its rays.
    pub(super) moved_additive: u32,
}

/// `runs` of `entities` appended to `combined`, their quads far to near, quads at one distance
/// in the order they came: the order a stable sort gives, by an in-place unstable one on
/// (distance, original index).
fn append_sorted(
    sort: &mut QuadSort,
    combined: &mut Vec<Vertex>,
    entities: &[Vertex],
    runs: &[(usize, usize)],
) {
    let start = combined.len();
    for &(first, count) in runs {
        combined.extend_from_slice(&entities[first * 4..(first + count) * 4]);
    }
    let quads = combined[start..].as_chunks_mut::<4>().0;
    sort.sort(quads);
}

/// `runs` of strictly ordered draws appended to `combined`, preserving draw order across runs
/// and sorting quads only within each draw (`RenderTypeFeatureRenderer.Group.getOrAddDraw`),
/// stripping `TRANSLUCENT_DRAW_FLIP` from the output vertices.
fn append_sorted_per_run(
    sort: &mut QuadSort,
    combined: &mut Vec<Vertex>,
    entities: &[Vertex],
    runs: &[(usize, usize)],
    sort_each: bool,
) {
    for &(first, count) in runs {
        let start = combined.len();
        combined.extend(
            entities[first * 4..(first + count) * 4]
                .iter()
                .map(|v| Vertex {
                    layer: v.layer & !TRANSLUCENT_DRAW_FLIP,
                    ..*v
                }),
        );
        if sort_each {
            let quads = combined[start..].as_chunks_mut::<4>().0;
            sort.sort(quads);
        }
    }
}

/// Draw-run key for `EntityBatch`: breaks runs across distinct materials, and inside material 8
/// (`TRANSLUCENT_ITEM_OVERLAY` / `TRANSLUCENT_GLINT_ITEM_OVERLAY`) breaks runs across foil vs
/// non-foil render types and `TRANSLUCENT_DRAW_FLIP` feature-group boundaries.
#[inline]
fn run_key(layer: u32, oit: bool) -> (usize, u32) {
    let kind = material(layer, oit);
    let draw = if kind == 8 {
        layer & (0xffff | TRANSLUCENT_DRAW_FLIP)
    } else {
        0
    };
    (kind, draw)
}

impl EntityBatch {
    /// Groups complete quads from `entities`, `additive`, `hand`, `see_through`, and `shadow_only`
    /// into [`Self::combined`] without exceeding `max_quads`.
    pub(super) fn update_within(
        &mut self,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
        see_through: &[Vertex],
        shadow_only: &[Vertex],
        oit: bool,
        max_quads: usize,
    ) -> EntityBatchCounts {
        let all_quads = entities.as_chunks::<4>().0;
        let quads = &all_quads[..all_quads.len().min(max_quads)];
        let admitted_entity_vertices = quads.len() * 4;
        let mut remaining = max_quads.saturating_sub(quads.len());

        let additive_all = additive.as_chunks::<4>().0;
        let additive_admitted = &additive_all[..additive_all.len().min(remaining)];
        remaining -= additive_admitted.len();

        let hand_all = hand.as_chunks::<4>().0;
        let hand_admitted = &hand_all[..hand_all.len().min(remaining)];
        remaining -= hand_admitted.len();

        let see_through_all = see_through.as_chunks::<4>().0;
        let see_through_admitted = &see_through_all[..see_through_all.len().min(remaining)];
        remaining -= see_through_admitted.len();

        let shadow_only_all = shadow_only.as_chunks::<4>().0;
        let shadow_only_admitted = &shadow_only_all[..shadow_only_all.len().min(remaining)];

        for runs in &mut self.runs {
            runs.clear();
        }
        let mut counts = [0usize; MATERIALS + 1];
        let mut start = 0;
        while start < quads.len() {
            let key = run_key(quads[start][0].layer, oit);
            let kind = key.0;
            let end = quads[start + 1..]
                .iter()
                .position(|quad| run_key(quad[0].layer, oit) != key)
                .map_or(quads.len(), |n| start + 1 + n);
            self.runs[kind].push((start, end - start));
            counts[kind] += end - start;
            start = end;
        }
        let total_quads = quads.len()
            + additive_admitted.len()
            + hand_admitted.len()
            + see_through_admitted.len()
            + shadow_only_admitted.len();
        self.combined.clear();
        self.combined.reserve(total_quads * 4);
        for (kind, runs) in self.runs[..10].iter().enumerate() {
            // Classic mode sorts `RenderTypes.TEXT_POLYGON_OFFSET` (1) and each translucent
            // block/item draw (8) far to near on upload (`VertexSorting.DISTANCE_TO_ORIGIN`),
            // while keeping feature and foil/non-foil draw boundaries in material 8 intact.
            if kind == 8 {
                append_sorted_per_run(&mut self.sort, &mut self.combined, entities, runs, !oit);
            } else if !oit && kind == 1 {
                append_sorted(&mut self.sort, &mut self.combined, entities, runs);
            } else {
                for &(first, count) in runs {
                    self.combined
                        .extend_from_slice(&entities[first * 4..(first + count) * 4]);
                }
            }
            if kind == 5 {
                let start = self.combined.len() as u32 / 4;
                for &(first, count) in &self.runs[ARMOR_FOIL] {
                    self.combined
                        .extend_from_slice(&entities[first * 4..(first + count) * 4]);
                }
                self.armor_foil = start..self.combined.len() as u32 / 4;
            }
        }
        counts[5] += counts[ARMOR_FOIL];
        // `RenderTypes.eyes` is blended like the other entities, not added: with
        // improved transparency on, tagged additive quads join the last
        // material, drawn without the additive overlay and the baked shading.
        self.moved_additive = 0;
        if oit {
            for quad in additive_admitted {
                if quad[0].layer & OIT_ENTITY != 0 {
                    self.combined.extend(quad.iter().map(|v| Vertex {
                        layer: v.layer & !0xffff,
                        normal: [0; 4],
                        ..*v
                    }));
                    counts[9] += 1;
                    self.moved_additive += 1;
                }
            }
        }
        // Fluid and RenderType.text sort their own alpha-blended quads far to
        // near. Fluid precedes both world-canvas layers, after existing materials
        // (including eyes moved out of additive).
        for runs in &self.runs[10..13] {
            append_sorted(&mut self.sort, &mut self.combined, entities, runs);
        }
        // Native solid/cutout models are depth writing and keep their submission order.
        for runs in &self.runs[13..16] {
            for &(first, count) in runs {
                self.combined
                    .extend_from_slice(&entities[first * 4..(first + count) * 4]);
            }
        }
        // Native translucent models sort their quads far to near, as `sortOnUpload` does.
        append_sorted(&mut self.sort, &mut self.combined, entities, &self.runs[16]);
        // Dynamic-texture quads follow the terrain models, before banner patterns: solid and
        // cutout keep their submission order, translucent ones sort far to near
        // (`RenderTypes.entityTranslucent` sorts on upload).
        for runs in &self.runs[DYNAMIC_MATERIALS.start..DYNAMIC_MATERIALS.end - 1] {
            for &(first, count) in runs {
                self.combined
                    .extend_from_slice(&entities[first * 4..(first + count) * 4]);
            }
        }
        let translucent = &self.runs[DYNAMIC_MATERIALS.end - 1];
        append_sorted(&mut self.sort, &mut self.combined, entities, translucent);
        // Classic `RenderTypes.textPolygonOffset` sorts on upload. `RenderTypes.text` (solid and
        // translucent), banner patterns and glint keep their submission order in either mode.
        let text = BEFORE_DYNAMIC..DYNAMIC_MATERIALS.start;
        for (kind, runs) in self.runs.iter().enumerate().take(text.end).skip(text.start) {
            if kind == 19 && !oit {
                append_sorted(&mut self.sort, &mut self.combined, entities, runs);
            } else {
                for &(first, count) in runs {
                    self.combined
                        .extend_from_slice(&entities[first * 4..(first + count) * 4]);
                }
            }
        }
        // A pack-drawn name tag's solid then translucent copies, last, in submission order.
        for runs in &self.runs[NAME_TAG_MATERIALS] {
            for &(first, count) in runs {
                self.combined
                    .extend_from_slice(&entities[first * 4..(first + count) * 4]);
            }
        }
        if self.moved_additive == 0 {
            self.combined
                .extend_from_slice(&additive[..additive_admitted.len() * 4]);
        } else {
            for quad in additive_admitted {
                if quad[0].layer & OIT_ENTITY == 0 {
                    self.combined.extend_from_slice(quad);
                }
            }
        }
        self.combined
            .extend_from_slice(&hand[..hand_admitted.len() * 4]);
        self.combined
            .extend_from_slice(&see_through[..see_through_admitted.len() * 4]);
        self.combined
            .extend_from_slice(&shadow_only[..shadow_only_admitted.len() * 4]);
        EntityBatchCounts {
            materials: std::array::from_fn(|i| counts[i] as u32),
            admitted_entity_vertices,
            additive: additive_admitted.len() as u32 - self.moved_additive,
            hand: hand_admitted.len() as u32,
            see_through: see_through_admitted.len() as u32,
            shadow_only: shadow_only_admitted.len() as u32,
        }
    }

    #[cfg(test)]
    pub(super) fn update(
        &mut self,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
        see_through: &[Vertex],
        oit: bool,
    ) -> [u32; MATERIALS] {
        self.update_within(entities, additive, hand, see_through, &[], oit, usize::MAX)
            .materials
    }
}

/// One cut, walked from the start of the stream. Tests compare the linear pass with this.
#[cfg(test)]
fn ordinary_vertices_before(vertices: &[Vertex], cut: u32, oit: bool) -> u32 {
    let cut = (cut as usize).min(vertices.len()) / 4 * 4;
    let mut ordinary = 0u32;
    for quad in vertices[..cut].as_chunks::<4>().0 {
        if material(quad[0].layer, oit) == 0 {
            ordinary += 4;
        }
    }
    ordinary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{hint::black_box, time::Instant};

    #[test]
    fn ordinary_cuts_skip_shadows_and_text() {
        let mut stream = Vec::new();
        for (layer, x) in [(14, 1.0), (0, 2.0)] {
            stream.extend(std::iter::repeat_n(
                Vertex {
                    layer,
                    pos: [x, 0.0, 0.0],
                    ..geometry(1, false)[0]
                },
                4,
            ));
        }
        let cut = stream.len() as u32;
        stream.extend(std::iter::repeat_n(
            Vertex {
                layer: 0,
                pos: [3.0, 0.0, 0.0],
                ..geometry(1, false)[0]
            },
            4,
        ));
        assert_eq!(ordinary_vertices_before(&stream, cut, false), 4);
        assert_ne!(cut, 4, "the raw cut still counts the shadow");
        let tagged = vec![
            Vertex {
                layer: OIT_ENTITY,
                ..geometry(1, false)[0]
            };
            4
        ];
        assert_eq!(ordinary_vertices_before(&tagged, 4, true), 0);
        assert_eq!(ordinary_vertices_before(&tagged, 4, false), 4);
    }

    #[test]
    fn one_walk_matches_the_per_draw_cut_on_mixed_streams() {
        let layers = [
            0u32,
            SHADOW_OVERLAY,
            TEXT_POLYGON_OFFSET_OVERLAY,
            TEXT_POLYGON_OFFSET_SOLID_OVERLAY,
            TEXT_NORMAL_SOLID_OVERLAY,
            NAME_TAG_OVERLAY,
            NAME_TAG_SOLID_OVERLAY,
            11,
            12,
            OIT_ENTITY,
            OIT_ENTITY | 11,
            WORLD_TEXT_OVERLAY,
            WORLD_TEXT_POLYGON_OFFSET_OVERLAY,
            TRANSLUCENT_ITEM_OVERLAY,
            TRANSLUCENT_GLINT_ITEM_OVERLAY,
            ENERGY_SWIRL_OVERLAY,
            WATER_MASK_OVERLAY,
            TRANSLUCENT_PARTICLE_OVERLAY,
            BANNER_PATTERN_OVERLAY,
            GLINT_ONLY_OVERLAY,
            7,
        ];
        let mut state = 0x1234_5678u32;
        let mut next = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            state
        };
        for oit in [false, true] {
            for _trial in 0..24 {
                let quads = (next() % 180) as usize;
                let stream: Vec<_> = (0..quads)
                    .flat_map(|_| {
                        let layer = layers[(next() as usize) % layers.len()];
                        std::iter::repeat_n(
                            Vertex {
                                layer,
                                ..geometry(1, false)[0]
                            },
                            4,
                        )
                    })
                    .collect();
                let draws = (next() % 36) as usize;
                let mut cuts: Vec<u32> = Vec::with_capacity(draws);
                for index in 0..draws {
                    let previous = cuts.last().copied().unwrap_or(0);
                    let cut = if index > 0 && next() % 5 == 0 {
                        next() % (stream.len() as u32 + 8)
                    } else {
                        previous.saturating_add(next() % 24)
                    };
                    cuts.push(cut);
                }
                let mut out = Vec::new();
                ordinary_vertices_at_cuts(&stream, cuts.iter().copied(), oit, &mut out);
                assert_eq!(out.len(), cuts.len());
                for (index, cut) in cuts.iter().enumerate() {
                    assert_eq!(
                        out[index],
                        ordinary_vertices_before(&stream, *cut, oit),
                        "oit {oit} cut {cut}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_decreasing_cut_matches_the_per_draw_walk() {
        let stream = geometry(8, true);
        let cuts = [24u32, 4, 16, 0, 24, 7];
        for oit in [false, true] {
            let mut out = Vec::new();
            ordinary_vertices_at_cuts(&stream, cuts, oit, &mut out);
            let expected: Vec<_> = cuts
                .iter()
                .map(|cut| ordinary_vertices_before(&stream, *cut, oit))
                .collect();
            assert_eq!(out, expected);
        }
    }

    #[test]
    fn a_long_prefix_is_walked_once_for_every_copy() {
        let stream_len = 10_000u32;
        let draws = 1_000u32;
        let stream = geometry(stream_len as usize, true);
        let end = stream.len() as u32;
        let cuts = vec![end; draws as usize];
        let mut out = Vec::new();
        let walked = ordinary_vertices_at_cuts(&stream, cuts.iter().copied(), true, &mut out);
        let stream_len = u64::from(stream_len);
        let draws = u64::from(draws);
        assert!(
            walked >= stream_len && walked <= stream_len + draws,
            "walked {walked} quads, bound {stream_len}..={}",
            stream_len + draws
        );
        let expected = ordinary_vertices_before(&stream, end, true);
        assert!(out.iter().all(|cut| *cut == expected));
        assert_eq!(out.len(), draws as usize);
    }
    #[test]
    fn sorted_quads_go_far_to_near_and_those_at_one_distance_keep_their_order() {
        // The distance is from the origin: (5, 0), (3, 4) and (4, 3) are all 5 away, so those
        // three must keep the order they came in.
        let quad = |x: f32, y: f32| {
            [Vertex {
                layer: TERRAIN_MODEL_TRANSLUCENT_OVERLAY,
                pos: [x, y, 0.0],
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<_> = [(2.0, 0.0), (5.0, 0.0), (9.0, 0.0), (3.0, 4.0), (4.0, 3.0)]
            .into_iter()
            .flat_map(|(x, y)| quad(x, y))
            .collect();
        let mut batch = EntityBatch::default();
        batch.update(&entities, &[], &[], &[], false);
        let order: Vec<_> = batch
            .combined
            .iter()
            .step_by(4)
            .map(|v| (v.pos[0], v.pos[1]))
            .collect();
        assert_eq!(
            order,
            [(9.0, 0.0), (5.0, 0.0), (3.0, 4.0), (4.0, 3.0), (2.0, 0.0)]
        );
    }
    #[test]
    fn native_models_keep_opaque_order_and_do_not_move_existing_materials_or_tails() {
        let quad = |layer, x| {
            [Vertex {
                layer,
                pos: [x, 0.0, 0.0],
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<_> = [
            (FLUID_OVERLAY, 10.0),
            (WORLD_TEXT_OVERLAY, 11.0),
            (TERRAIN_MODEL_SOLID_OVERLAY | 279 << 16, 3.0),
            (TERRAIN_MODEL_SOLID_OVERLAY, 1.0),
            (TERRAIN_MODEL_CUTOUT_OVERLAY, 4.0),
            (TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, 5.0),
            (OIT_ENTITY, 6.0),
            (0, 7.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let mut batch = EntityBatch::default();
        let counts = batch.update(
            &entities,
            &quad(0, 8.0),
            &quad(0, 9.0),
            &quad(0, 12.0),
            true,
        );
        assert_eq!(
            counts,
            [
                1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        let xs: Vec<_> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(
            xs,
            [7.0, 6.0, 10.0, 11.0, 3.0, 1.0, 4.0, 5.0, 8.0, 9.0, 12.0]
        );
        assert_eq!(
            batch.combined[16].layer,
            TERRAIN_MODEL_SOLID_OVERLAY | 279 << 16
        );
    }
    #[test]
    fn translucent_native_models_sort_far_to_near_after_the_opaque_ones() {
        let quad = |layer, x| {
            [Vertex {
                layer,
                pos: [x, 0.0, 0.0],
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<_> = [
            (TERRAIN_MODEL_TRANSLUCENT_OVERLAY | 5 << 16, 2.0),
            (TERRAIN_MODEL_SOLID_OVERLAY, 1.0),
            (TERRAIN_MODEL_TRANSLUCENT_OVERLAY | 5 << 16, 9.0),
            (TERRAIN_MODEL_TRANSLUCENT_OVERLAY | 5 << 16, 4.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let mut batch = EntityBatch::default();
        let counts = batch.update(&entities, &[], &[], &[], false);
        assert_eq!((counts[13], counts[16]), (1, 3));
        let xs: Vec<_> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [1.0, 9.0, 4.0, 2.0]);
    }
    #[test]
    fn tagging_oit_does_not_corrupt_a_native_models_sprite_index() {
        let mut vertices = [geometry(1, false)[0]; 2];
        vertices[0].layer = TERRAIN_MODEL_CUTOUT_OVERLAY | 17 << 16;
        vertices[1].layer = 3;
        mark_oit(&mut vertices);
        assert_eq!(vertices[0].layer, TERRAIN_MODEL_CUTOUT_OVERLAY | 17 << 16);
        assert_eq!(vertices[1].layer, 3 | OIT_ENTITY);
    }
    #[test]
    fn fluid_and_world_canvas_sort_after_oit_eyes_before_frame_tails() {
        let quad = |layer, x| {
            [Vertex {
                layer,
                pos: [x, 0.0, 0.0],
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<_> = [
            (0, 1.0),
            (35, 2.0),
            (OIT_ENTITY, 3.0),
            (WORLD_TEXT_OVERLAY, 4.0),
            (WORLD_TEXT_OVERLAY, 5.0),
            (WORLD_TEXT_POLYGON_OFFSET_OVERLAY, 6.0),
            (FLUID_OVERLAY, 11.0),
            (FLUID_OVERLAY | OIT_ENTITY, 12.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let additive: Vec<_> = [(OIT_ENTITY | 2, 7.0), (2, 8.0)]
            .into_iter()
            .flat_map(|(layer, x)| quad(layer, x))
            .collect();
        let mut batch = EntityBatch::default();
        assert_eq!(
            batch.update(&entities, &additive, &quad(0, 9.0), &quad(0, 10.0), true),
            [
                1, 1, 0, 0, 0, 0, 0, 0, 0, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        let positions: Vec<_> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(
            positions,
            [
                1.0, 2.0, 3.0, 7.0, 12.0, 11.0, 5.0, 4.0, 6.0, 8.0, 9.0, 10.0
            ]
        );
    }

    // Retain the original implementation as the equivalence/benchmark oracle.
    fn baseline(
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
    ) -> (Vec<Vertex>, [u32; MATERIALS]) {
        let mut combined = Vec::with_capacity(entities.len() + additive.len() + hand.len());
        let mut counts = [0; MATERIALS];
        for (i, layer) in [0, 35, 11, 12, 13, 34, 14, 15].into_iter().enumerate() {
            let start = combined.len();
            for quad in entities.as_chunks::<4>().0 {
                let material = match quad[0].layer {
                    35 => 35,
                    11 => 11,
                    12 => 12,
                    13 => 13,
                    34 => 34,
                    14 => 14,
                    15 => 15,
                    _ => 0,
                };
                if material == layer {
                    combined.extend_from_slice(quad);
                    counts[i] += 1;
                }
            }
            if layer == 35 {
                combined[start..]
                    .as_chunks_mut::<4>()
                    .0
                    .sort_by(|a, b| quad_distance_sq(b).total_cmp(&quad_distance_sq(a)));
            }
        }
        combined.extend_from_slice(&additive[..additive.len() / 4 * 4]);
        combined.extend_from_slice(&hand[..hand.len() / 4 * 4]);
        (combined, counts)
    }

    #[test]
    fn dynamic_textures_start_where_they_are_stored_with_name_tags_after_them() {
        // One quad per layer, in an order that stores differently: each quad's colour names it.
        let layers = [
            NAME_TAG_OVERLAY,
            DYNAMIC_TEXTURE_SOLID_OVERLAY,
            TEXT_NORMAL_SOLID_OVERLAY,
            0,
            NAME_TAG_SOLID_OVERLAY,
            DYNAMIC_TEXTURE_TRANSLUCENT_OVERLAY,
            BANNER_PATTERN_OVERLAY,
            TERRAIN_MODEL_SOLID_OVERLAY,
            DYNAMIC_TEXTURE_CUTOUT_OVERLAY,
            GLINT_ONLY_OVERLAY,
            SHADOW_OVERLAY,
            NAME_TAG_OVERLAY,
        ];
        let entities: Vec<Vertex> = layers
            .iter()
            .enumerate()
            .flat_map(|(i, &layer)| {
                std::iter::repeat_n(
                    Vertex {
                        layer,
                        color: i as u32,
                        ..geometry(1, false)[0]
                    },
                    4,
                )
            })
            .collect();
        let mut batch = EntityBatch::default();
        for oit in [false, true] {
            let counts = batch.update(&entities, &[], &[], &[], oit);
            let ids: Vec<u32> = batch.combined.iter().step_by(4).map(|v| v.color).collect();
            // Stored: ordinary (3), shadow (10), terrain model (7), then the dynamic textures
            // solid (1), cutout (8), translucent (5), then banner (6), glint (9), solid text (2)
            // and the name tags, solid (4) then translucent (0, 11).
            assert_eq!(ids, [3, 10, 7, 1, 8, 5, 6, 9, 2, 4, 0, 11]);
            assert_eq!(dynamic_texture_start(&counts), 3);
            assert_eq!(counts[DYNAMIC_MATERIALS], [1, 1, 1]);
        }
    }

    fn geometry(quads: usize, mixed: bool) -> Vec<Vertex> {
        (0..quads * 4)
            .map(|index| Vertex {
                pos: [index as f32, 2.0, 3.0],
                uv: [index as f32, 5.0],
                layer: if mixed {
                    [7, 11, 14, 35, 12, 4, 13, 15, 0][index / 4 % 9]
                } else {
                    0
                },
                color: index as u32,
                light: (index * 3) as u32,
                normal: [1, 2, 3, 4],

                glint_uv: [0.0, 0.0],
            })
            .collect()
    }

    fn equivalent(
        batch: &mut EntityBatch,
        entities: &[Vertex],
        additive: &[Vertex],
        hand: &[Vertex],
    ) {
        let (expected, counts) = baseline(entities, additive, hand);
        assert_eq!(batch.update(entities, additive, hand, &[], false), counts);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&batch.combined),
            bytemuck::cast_slice::<_, u8>(&expected)
        );
    }

    #[test]
    fn a_tier_in_the_high_bits_keeps_the_material() {
        for layer in [
            0u32,
            11,
            12,
            15,
            32,
            33,
            35,
            TEXT_POLYGON_OFFSET_SOLID_OVERLAY,
            BANNER_PATTERN_OVERLAY,
            GLINT_ONLY_OVERLAY,
            OIT_ENTITY,
        ] {
            for oit in [false, true] {
                assert_eq!(material(layer | 4 << 16, oit), material(layer, oit));
            }
        }
    }

    #[test]
    fn solid_text_stays_outside_oit_and_only_polygon_offset_sorts_in_classic() {
        let quad = |layer: u32, z: f32, id: u32| {
            [Vertex {
                layer,
                pos: [0.0, 0.0, z],
                color: id,
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<Vertex> = [
            (TEXT_NORMAL_SOLID_OVERLAY, 2.0, 4),
            (TEXT_POLYGON_OFFSET_SOLID_OVERLAY, 2.0, 1),
            (TEXT_POLYGON_OFFSET_SOLID_OVERLAY | OIT_ENTITY, 9.0, 2),
            (TEXT_NORMAL_SOLID_OVERLAY | OIT_ENTITY, 9.0, 5),
            (TEXT_POLYGON_OFFSET_SOLID_OVERLAY, 4.0, 3),
        ]
        .into_iter()
        .flat_map(|(layer, z, id)| quad(layer, z, id))
        .collect();
        let mut batch = EntityBatch::default();
        for (oit, want) in [(false, [2, 3, 1, 4, 5]), (true, [1, 2, 3, 4, 5])] {
            let counts = batch.update(&entities, &[], &[], &[], oit);
            assert_eq!(counts[19..21], [3, 2]);
            assert_eq!(counts[9], 0);
            let ids: Vec<u32> = batch.combined.iter().step_by(4).map(|v| v.color).collect();
            assert_eq!(ids, want);
        }
    }

    #[test]
    fn banner_pattern_and_glint_only_quads_stay_in_order_and_skip_oit() {
        let quad = |layer: u32, x: f32| {
            [Vertex {
                layer,
                pos: [x, 0.0, 0.0],
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<_> = [
            (0, 1.0),
            (BANNER_PATTERN_OVERLAY | OIT_ENTITY, 2.0),
            (GLINT_ONLY_OVERLAY | OIT_ENTITY, 3.0),
            (0, 4.0),
            (BANNER_PATTERN_OVERLAY, 5.0),
            (GLINT_ONLY_OVERLAY, 6.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        for oit in [false, true] {
            let mut batch = EntityBatch::default();
            let counts = batch.update(&entities, &[], &[], &[], oit);
            assert_eq!((counts[0], counts[9], counts[17], counts[18]), (2, 0, 2, 2));
            let xs: Vec<_> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
            assert_eq!(xs, [1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
        }
    }

    #[test]
    fn vertex_sorting_distance_breaks_4096_tie_and_keeps_translucent_draw_boundaries() {
        let quad = |layer: u32, pos: [f32; 3], id: u32| {
            [Vertex {
                layer,
                pos,
                color: id,
                ..geometry(1, false)[0]
            }; 4]
        };
        // (4096, 1, 1) has JOML distanceSquared = 4096^2 + (1 + 1) = 16777218 > 16777216.
        let tie_test: Vec<Vertex> = [
            (TEXT_POLYGON_OFFSET_OVERLAY, [4096.0, 0.0, 0.0], 1),
            (TEXT_POLYGON_OFFSET_OVERLAY, [4096.0, 1.0, 1.0], 2),
        ]
        .into_iter()
        .flat_map(|(layer, pos, id)| quad(layer, pos, id))
        .collect();
        let mut batch = EntityBatch::default();
        batch.update(&tie_test, &[], &[], &[], false);
        let ids: Vec<u32> = batch.combined.iter().step_by(4).map(|v| v.color).collect();
        assert_eq!(ids, [2, 1]);

        // Foil/non-foil boundaries and TRANSLUCENT_DRAW_FLIP keep submits separate while
        // still sorting quads inside each draw in classic mode.
        let mixed: Vec<Vertex> = [
            (TRANSLUCENT_ITEM_OVERLAY, [0.0, 0.0, 1.0], 10),
            (TRANSLUCENT_ITEM_OVERLAY, [0.0, 0.0, 5.0], 11),
            (TRANSLUCENT_GLINT_ITEM_OVERLAY, [0.0, 0.0, 2.0], 20),
            (TRANSLUCENT_GLINT_ITEM_OVERLAY, [0.0, 0.0, 8.0], 21),
            (
                TRANSLUCENT_ITEM_OVERLAY | TRANSLUCENT_DRAW_FLIP,
                [0.0, 0.0, 3.0],
                30,
            ),
            (
                TRANSLUCENT_ITEM_OVERLAY | TRANSLUCENT_DRAW_FLIP,
                [0.0, 0.0, 6.0],
                31,
            ),
        ]
        .into_iter()
        .flat_map(|(layer, pos, id)| quad(layer, pos, id))
        .collect();
        let counts = batch.update(&mixed, &[], &[], &[], false);
        assert_eq!(counts[8], 6);
        let ids: Vec<u32> = batch.combined.iter().step_by(4).map(|v| v.color).collect();
        assert_eq!(ids, [11, 10, 21, 20, 31, 30]);
        assert!(
            batch
                .combined
                .iter()
                .all(|v| v.layer & TRANSLUCENT_DRAW_FLIP == 0)
        );
    }

    #[test]
    fn oit_entity_quads_are_a_last_material_only_when_on() {
        let quad = |layer: u32, x: f32| {
            (0..4).map(move |_| Vertex {
                pos: [x, 0.0, 0.0],
                layer,
                ..geometry(1, false)[0]
            })
        };
        // Plain, tagged, plain, shadow, tagged with a hurt overlay.
        let entities: Vec<Vertex> = [
            (0, 0.0),
            (OIT_ENTITY | 3 << 16, 1.0),
            (1, 2.0),
            (SHADOW_OVERLAY, 3.0),
            (OIT_ENTITY | 1, 4.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let mut batch = EntityBatch::default();
        // Off: the tag changes nothing, the quads keep their place.
        let counts = batch.update(&entities, &[], &[], &[], false);
        assert_eq!(
            counts,
            [
                4, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        let xs: Vec<f32> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 1.0, 2.0, 4.0, 3.0]);
        // On: the tagged quads come last, in order.
        let counts = batch.update(&entities, &[], &[], &[], true);
        assert_eq!(
            counts,
            [
                2, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        );
        let xs: Vec<f32> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 2.0, 3.0, 1.0, 4.0]);
    }

    #[test]
    fn tagged_additive_quads_join_the_last_material_only_when_on() {
        let quad = |layer: u32, x: f32| {
            (0..4).map(move |_| Vertex {
                pos: [x, 0.0, 0.0],
                layer,
                normal: [1, 2, 3, 4],
                ..geometry(1, false)[0]
            })
        };
        // The dragon's eyes (tagged, overlay 2) and a ray (plain).
        let additive: Vec<Vertex> = [(2 | OIT_ENTITY | 3 << 16, 1.0), (2, 2.0)]
            .into_iter()
            .flat_map(|(layer, x)| quad(layer, x))
            .collect();
        let entities: Vec<Vertex> = quad(0, 0.0).collect();
        let mut batch = EntityBatch::default();
        // Off: untouched.
        let counts = batch.update(&entities, &additive, &[], &[], false);
        assert_eq!((counts[9], batch.moved_additive), (0, 0));
        assert_eq!(batch.combined.len(), 12);
        assert_eq!(batch.combined[4].layer, 2 | OIT_ENTITY | 3 << 16);
        // On: the eyes are an entity quad without the additive overlay, the ray stays.
        let counts = batch.update(&entities, &additive, &[], &[], true);
        assert_eq!((counts[0], counts[9], batch.moved_additive), (1, 1, 1));
        let xs: Vec<f32> = batch.combined.iter().step_by(4).map(|v| v.pos[0]).collect();
        assert_eq!(xs, [0.0, 1.0, 2.0]);
        assert_eq!(batch.combined[4].layer, OIT_ENTITY | 3 << 16);
        assert_eq!(batch.combined[4].normal, [0; 4]);
        assert_eq!(batch.combined[8].layer, 2);
    }

    fn display_parity_witness() -> serde_json::Value {
        let path = std::env::var("SUNRISE_DISPLAY_PARITY_WITNESS").expect(
            "SUNRISE_DISPLAY_PARITY_WITNESS must be set (run via tools/test_displays_parity.py)",
        );
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
        let witness: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(witness["meta"]["game_version"], "26.3");
        witness
    }

    fn witness_sort_quads(layer: u32) -> Vec<Vertex> {
        let centroids: [[f32; 3]; 5] = [
            [0.0, 0.0, 4.20025],
            [0.0, 0.0, 4.89025],
            [3.0, 4.0, 0.0],
            [4.0, 3.0, 0.0],
            [0.0, 0.0, 4.85080],
        ];
        centroids
            .into_iter()
            .enumerate()
            .flat_map(|(id, c)| {
                [Vertex {
                    layer,
                    pos: c,
                    color: id as u32,
                    ..geometry(1, false)[0]
                }; 4]
            })
            .collect()
    }

    #[test]
    #[ignore = "run by tools/test_displays_parity.py"]
    fn classic_polygon_offset_text_quads_sort_far_to_near_like_vanilla_26_3() {
        let witness = display_parity_witness();
        let section = &witness["text_submit_phases_and_sorting"];
        assert_eq!(
            section["pipelines"]["rt_text_polygon_offset_sort_on_upload"],
            true
        );
        let expected: Vec<u32> = section["vertex_sorting_distance_to_origin_order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u32)
            .collect();
        let entities = witness_sort_quads(TEXT_POLYGON_OFFSET_OVERLAY);
        let mut batch = EntityBatch::default();
        batch.update(&entities, &[], &[], &[], false);
        let got: Vec<u32> = batch.combined.iter().step_by(4).map(|v| v.color).collect();
        assert_eq!(got, expected);
    }

    #[test]
    #[ignore = "run by tools/test_displays_parity.py"]
    fn classic_translucent_item_quads_sort_far_to_near_like_vanilla_26_3() {
        let witness = display_parity_witness();
        assert_eq!(
            witness["block_display_render_type_and_cull"]["translucent_sort_on_upload"],
            true
        );
        let expected: Vec<u32> =
            witness["text_submit_phases_and_sorting"]["vertex_sorting_distance_to_origin_order"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect();
        let entities = witness_sort_quads(TRANSLUCENT_ITEM_OVERLAY);
        let mut batch = EntityBatch::default();
        batch.update(&entities, &[], &[], &[], false);
        let got: Vec<u32> = batch.combined.iter().step_by(4).map(|v| v.color).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn stable_material_order_tails_and_reuse() {
        let mut batch = EntityBatch::default();
        let mut entities = geometry(4096, true);
        // Only the first vertex determines a quad's material.
        entities[1].layer = 12;
        let additive = geometry(3, true);
        let hand = geometry(2, false);
        // Warm every bucket and the combined buffer to their maximum sizes.
        equivalent(&mut batch, &entities, &additive, &hand);
        let capacities = batch.runs.each_ref().map(|b| b.capacity());
        let capacity = batch.combined.capacity();
        let pointer = batch.combined.as_ptr();
        for length in [0, 1, 3, 4, 511, entities.len(), 17, 0, entities.len()] {
            equivalent(&mut batch, &entities[..length], &additive[..5], &hand[..3]);
            assert_eq!(batch.runs.each_ref().map(|b| b.capacity()), capacities);
            assert_eq!(batch.combined.capacity(), capacity);
            assert_eq!(batch.combined.as_ptr(), pointer);
        }
        equivalent(&mut batch, &[], &[], &[]);
        assert!(batch.combined.is_empty());
        // Grow again, then confirm empty/small frames retain the new storage.
        let larger = geometry(8192, true);
        equivalent(&mut batch, &larger, &additive, &hand);
        let capacities = batch.runs.each_ref().map(|b| b.capacity());
        let capacity = batch.combined.capacity();
        for entities in [&larger[..7], &larger[..], &[]] {
            equivalent(&mut batch, entities, &[], &[]);
            assert_eq!(batch.runs.each_ref().map(|b| b.capacity()), capacities);
            assert_eq!(batch.combined.capacity(), capacity);
        }
    }

    #[test]
    fn entity_quad_capacity_respects_vertex_index_and_storage_limits() {
        let limits = wgpu::Limits {
            max_buffer_size: EntityQuadCapacity::VERTEX_QUAD_BYTES * 5 + 32,
            max_storage_buffer_binding_size: EntityQuadCapacity::VERTEX_QUAD_BYTES * 3 + 16,
            ..Default::default()
        };

        let plain = EntityQuadCapacity::from_limits(&limits, false);
        assert_eq!(plain.max_quads(), 5);
        assert_eq!(plain.max_vertices(), 20);

        let storage = EntityQuadCapacity::from_limits(&limits, true);
        assert_eq!(storage.max_quads(), 3);
        assert_eq!(storage.max_vertices(), 12);

        // When index bytes are tighter than vertex bytes, index capacity wins.
        let index_tight = EntityQuadCapacity {
            vertex_bytes: EntityQuadCapacity::VERTEX_QUAD_BYTES * 10,
            index_bytes: EntityQuadCapacity::INDEX_QUAD_BYTES * 4 + 12,
        };
        assert_eq!(index_tight.max_quads(), 4);
        assert_eq!(index_tight.max_index_quads(), 4);
        assert_eq!(index_tight.index_buffer_quads(3), 4);
        assert_eq!(index_tight.index_buffer_quads(4), 4);
        assert_eq!(index_tight.index_buffer_quads(5), 4);

        // Power-of-two index growth clamps to max_index_quads instead of overshooting.
        let five_index = EntityQuadCapacity {
            vertex_bytes: EntityQuadCapacity::VERTEX_QUAD_BYTES * 10,
            index_bytes: EntityQuadCapacity::INDEX_QUAD_BYTES * 5,
        };
        assert_eq!(five_index.index_buffer_quads(5), 5);
        assert_eq!(five_index.index_buffer_quads(3), 4);
    }

    #[test]
    fn entity_quad_scope_accounts_for_reserved_and_existing_quads_and_drops_non_finite() {
        let capacity = EntityQuadCapacity::from_max_buffer_size(
            EntityQuadCapacity::VERTEX_QUAD_BYTES * 5 + 100,
        );
        // 1 existing quad in `vertices` + 2 reserved by other draw sources => 2 quads left.
        let mut vertices = geometry(1, false);
        let mut scope = EntityQuadScope::new(&mut vertices, capacity, 2);
        assert_eq!(scope.remaining_quads(), 2);

        let mut bad = [geometry(1, false)[0]; 4];
        bad[2].pos[1] = f32::INFINITY;
        assert!(
            scope.submit_quad(bad),
            "non-finite quad skips without stopping"
        );
        assert_eq!(scope.remaining_quads(), 2);

        let mut bad_uv = [geometry(1, false)[0]; 4];
        bad_uv[1].uv[0] = f32::NAN;
        assert!(scope.submit_quad(bad_uv));
        assert_eq!(scope.remaining_quads(), 2);

        let good = [geometry(1, false)[0]; 4];
        assert!(scope.submit_quad(good), "room remains after first quad");
        assert!(
            !scope.submit_quad(good),
            "second quad fills the remaining budget and signals stop"
        );
        assert!(!scope.submit_quad(good), "over-budget quad is rejected");
        assert_eq!(vertices.len(), 3 * 4);
    }

    #[test]
    fn entity_batch_update_within_bounds_combined_quads_across_all_streams() {
        let quad = |layer, x| {
            [Vertex {
                layer,
                pos: [x, 0.0, 0.0],
                ..geometry(1, false)[0]
            }; 4]
        };
        let entities: Vec<_> = [
            (FLUID_OVERLAY, 1.0),
            (TERRAIN_MODEL_SOLID_OVERLAY, 2.0),
            (TERRAIN_MODEL_TRANSLUCENT_OVERLAY, 3.0),
        ]
        .into_iter()
        .flat_map(|(layer, x)| quad(layer, x))
        .collect();
        let additive: Vec<_> = [quad(0, 4.0), quad(0, 5.0)].into_iter().flatten().collect();
        let hand: Vec<_> = quad(0, 6.0).into_iter().collect();
        let see_through: Vec<_> = quad(0, 7.0).into_iter().collect();
        let shadow_only: Vec<_> = quad(0, 8.0).into_iter().collect();

        let mut batch = EntityBatch::default();
        let counts = batch.update_within(
            &entities,
            &additive,
            &hand,
            &see_through,
            &shadow_only,
            false,
            4,
        );
        assert_eq!(counts.admitted_entity_vertices, 12);
        assert_eq!(counts.materials[10], 1);
        assert_eq!(counts.materials[13], 1);
        assert_eq!(counts.materials[16], 1);
        assert_eq!(counts.additive, 1);
        assert_eq!(counts.hand, 0);
        assert_eq!(counts.see_through, 0);
        assert_eq!(counts.shadow_only, 0);
        assert_eq!(batch.combined.len(), 4 * 4);
    }

    #[test]
    #[ignore = "manual production-helper microbenchmark"]
    fn benchmark_entity_batch() {
        let additive = geometry(16, false);
        let hand = geometry(8, false);
        for (name, mixed, quads) in [
            ("mixed", true, 65_536),
            ("single", false, 65_536),
            ("runs", true, 65_536),
            ("mixed", true, 262_144),
            ("single", false, 262_144),
        ] {
            let mut entities = geometry(quads, mixed);
            if name == "runs" {
                // Model/material runs: normal 16, glint 4, overlay 4, shadow 2.
                for (index, quad) in entities.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let layer = match index % 26 {
                        0..16 => 0,
                        16..20 => 11,
                        20..24 => 12,
                        _ => 4,
                    };
                    for vertex in quad {
                        vertex.layer = layer;
                    }
                }
            }
            let mut batch = EntityBatch::default();
            equivalent(&mut batch, &entities, &additive, &hand);
            for _ in 0..50 {
                black_box(baseline(black_box(&entities), &additive, &hand));
                black_box(batch.update(black_box(&entities), &additive, &hand, &[], false));
                black_box(&batch.combined);
            }
            let mut before = Vec::new();
            let mut after = Vec::new();
            for _ in 0..15 {
                let start = Instant::now();
                for _ in 0..100 {
                    black_box(baseline(black_box(&entities), &additive, &hand));
                }
                before.push(start.elapsed().as_nanos() / 100);
                let start = Instant::now();
                for _ in 0..100 {
                    black_box(batch.update(black_box(&entities), &additive, &hand, &[], false));
                    black_box(&batch.combined);
                }
                after.push(start.elapsed().as_nanos() / 100);
            }
            before.sort_unstable();
            after.sort_unstable();
            println!(
                "{name}: {quads} quads, baseline={} ns/frame optimized={} ns/frame speedup={:.2}x scratch={} bytes",
                before[7],
                after[7],
                before[7] as f64 / after[7] as f64,
                batch.combined.capacity() * std::mem::size_of::<Vertex>()
                    + batch
                        .runs
                        .iter()
                        .map(|b| b.capacity() * std::mem::size_of::<(usize, usize)>())
                        .sum::<usize>()
            );
        }
    }
}
