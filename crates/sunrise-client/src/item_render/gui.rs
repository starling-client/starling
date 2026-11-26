//! `GuiGraphics.renderItem` drawn into the HUD batch: the item's layers are
//! submitted under the GUI item pose (`translate(x + 8, y + 8)`,
//! `scale(16, -16, 16)`), lit with `Lighting.Entry.ITEMS_3D` or `ITEMS_FLAT`
//! (`minecraft_mix_light` on the transformed normals), back faces culled,
//! drawn far to near, and clipped to the item's 16x16 slot unless the model
//! is oversized in the GUI (`GuiItemAtlas` draws each item into a scissored
//! 16x16 slot; oversized items get a picture-in-picture of their bounds).
use super::{Foil, Glint, GlintFrame, ItemQuad, ItemRenderState, ItemRenderer, SpecialHost};
use crate::cit::Rule;
use crate::cit_glint::StackGlint;
use crate::hud::cit_glint::{GlintTextures, layer_uv, vertex_color};
use crate::hud::{HudBatch, Rect};
use glam::{EulerRot, Mat4, Vec2, Vec3};
use std::{collections::HashMap, f32::consts::PI, sync::Arc};

mod depth;

/// `Lighting.DIFFUSE_LIGHT_0` and `DIFFUSE_LIGHT_1`.
fn diffuse_lights() -> (Vec3, Vec3) {
    (
        Vec3::new(0.2, 1.0, -0.7).normalize(),
        Vec3::new(-0.2, 1.0, 0.7).normalize(),
    )
}
/// `Lighting.Entry.ITEMS_FLAT`: `rotationY(-PI/8).rotateX(3PI/4)`.
pub fn items_flat_lights() -> (Vec3, Vec3) {
    let m = Mat4::from_rotation_y(-PI / 8.0) * Mat4::from_rotation_x(PI * 3.0 / 4.0);
    let (a, b) = diffuse_lights();
    (m.transform_vector3(a), m.transform_vector3(b))
}
/// JOML `rotateYXZ(y, x, z)`.
fn rotate_yxz(y: f32, x: f32, z: f32) -> Mat4 {
    Mat4::from_euler(EulerRot::YXZ, y, x, z)
}
/// `Lighting.Entry.ITEMS_3D`.
pub fn items_3d_lights() -> (Vec3, Vec3) {
    let m = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
        * rotate_yxz(1.082_104_1, 3.237_585_8, 0.0)
        * rotate_yxz(-PI / 8.0, PI * 3.0 / 4.0, 0.0);
    let (a, b) = diffuse_lights();
    (m.transform_vector3(a), m.transform_vector3(b))
}

/// A GUI-space vertex: position, atlas UV.
#[derive(Clone, Copy, Debug)]
struct Point {
    p: Vec2,
    uv: [f32; 2],
    glint_uv: [f32; 2],
    cit_uv: [f32; 2],
}
fn lerp(a: Point, b: Point, t: f32) -> Point {
    Point {
        p: a.p + (b.p - a.p) * t,
        uv: [
            a.uv[0] + (b.uv[0] - a.uv[0]) * t,
            a.uv[1] + (b.uv[1] - a.uv[1]) * t,
        ],
        cit_uv: std::array::from_fn(|k| a.cit_uv[k] + (b.cit_uv[k] - a.cit_uv[k]) * t),
        glint_uv: [
            a.glint_uv[0] + (b.glint_uv[0] - a.glint_uv[0]) * t,
            a.glint_uv[1] + (b.glint_uv[1] - a.glint_uv[1]) * t,
        ],
    }
}
/// Sutherland-Hodgman against one half plane `dot(p, n) <= limit`.
fn clip_edge(poly: &[Point], axis: usize, limit: f32, keep_below: bool) -> Vec<Point> {
    let inside = |p: &Point| {
        let v = p.p[axis];
        if keep_below { v <= limit } else { v >= limit }
    };
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ia, ib) = (inside(&a), inside(&b));
        if ia {
            out.push(a);
        }
        if ia != ib {
            let t = (limit - a.p[axis]) / (b.p[axis] - a.p[axis]);
            out.push(lerp(a, b, t));
        }
    }
    out
}
/// A triangle clipped to `[x0, x1] x [y0, y1]`, as a fan.
fn clip_triangle(tri: [Point; 3], rect: [f32; 4]) -> Vec<Point> {
    let mut poly = tri.to_vec();
    for (axis, limit, below) in [
        (0, rect[0], false),
        (0, rect[2], true),
        (1, rect[1], false),
        (1, rect[3], true),
    ] {
        if poly.is_empty() {
            break;
        }
        poly = clip_edge(&poly, axis, limit, below);
    }
    poly
}

/// RGBA vertex colour of a lit quad: `minecraft_mix_light` on the tint.
/// Skips directional diffuse shading when `q.lightmap_only` is set (`SpecialModelLighting::LightmapOnly`).
pub fn lit_color(q: &ItemQuad, lights: (Vec3, Vec3)) -> u32 {
    let light = if q.lightmap_only {
        1.0
    } else {
        crate::block_entity_render::diffuse(q.normal, lights)
    };
    let [a, r, g, b] = q.color.to_be_bytes();
    let c = |v: u8| (f32::from(v) * light) as u8;
    crate::hud::rgba(c(r), c(g), c(b), a)
}

/// Encode `ItemQuad`'s explicit lightmap coordinates and item alpha-cutoff flag into
/// [`crate::hud::HudVertex::light`]. Returns `0` for ordinary HUD quads.
fn hud_light_word(q: &ItemQuad) -> u32 {
    let mut word = 0u32;
    if let Some(packed) = q.packed_light {
        let block = packed & 0xFF;
        let sky = (packed >> 16) & 0xFF;
        word |= 0x0100_0000 | block | (sky << 8);
    }
    if q.explicit_alpha_cutoff {
        word |= 0x0200_0000;
    }
    word
}

fn emit(
    batch: &mut HudBatch,
    fan: &[Point],
    color: impl Fn(Vec2) -> u32,
    light: u32,
    glint: Option<&Glint>,
) {
    for i in 1..fan.len().saturating_sub(1) {
        let (a, b, c) = (fan[0], fan[i], fan[i + 1]);
        let corners = [a, b, c, c];
        if let Some(glint) = glint {
            let start = batch.vertices.len();
            batch.quad_glint(
                corners.map(|v| v.p.to_array()),
                corners.map(|v| [v.uv[0] + 2.0, v.uv[1]]),
                corners.map(|v| glint.uv(v.glint_uv)),
                u32::MAX,
            );
            batch.blend_since(start, crate::hud::Blend::Additive);
        } else {
            batch.quad_lit(
                corners.map(|v| v.p.to_array()),
                corners.map(|v| v.uv),
                color(a.p),
                light,
            );
        }
    }
}

/// A stack's OptiFine CIT glint layers this frame.
#[derive(Clone, Copy)]
struct CitDraw<'a> {
    glint: &'a StackGlint,
    textures: &'a GlintTextures,
    rules: &'a [Rule],
    millis: i64,
}

/// The CIT glint layers over the item's clipped faces (`fans`, in draw order), bottom first:
/// each layer is one run over all of them, blended as its rule says, at its intensity times
/// the glint strength option (`HudBatch::glint_alpha`).
fn emit_cit(batch: &mut HudBatch, fans: &[Vec<Point>], cit: CitDraw) {
    let seconds = cit.millis as f64 / 1000.0;
    for (i, layer) in cit.glint.layers.iter().enumerate() {
        let Some(slot) = cit.textures.slot_of.get(layer.rule).copied().flatten() else {
            continue;
        };
        let Some(constants) = cit.textures.slots.get(usize::from(slot)) else {
            continue;
        };
        let intensity = cit.glint.intensity(i, seconds) * batch.glint_alpha;
        if intensity <= 0.0 {
            continue;
        }
        let glint = &cit.rules[layer.rule].glint;
        let color = vertex_color(slot, intensity);
        let uv = |v: Point| {
            layer_uv(
                v.cit_uv,
                cit.millis as f32,
                glint.speed,
                glint.rotation,
                8.0,
            )
        };
        let start = batch.vertices.len();
        for fan in fans {
            for k in 1..fan.len().saturating_sub(1) {
                let corners = [fan[0], fan[k], fan[k + 1], fan[k + 1]];
                batch.quad_glint(
                    corners.map(|v| v.p.to_array()),
                    corners.map(|v| v.uv),
                    corners.map(uv),
                    color,
                );
            }
        }
        batch.blend_since(start, crate::hud::Blend::CitGlint(constants.blend));
    }
}

/// Submit, light, cull, sort and clip one item's quads into `batch` at GUI
/// position `(x, y)`.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    millis: i64,
    glint_speed: f64,
    glint_alpha: f32,
) {
    draw_with_frame(
        state,
        renderer,
        sprites,
        host,
        batch,
        x,
        y,
        GlintFrame {
            millis,
            speed: glint_speed,
            alpha: glint_alpha,
            sprite: sprites.contains_key("misc/enchanted_glint_item"),
        },
    );
}

/// [`draw`] with the frame's exact [`GlintFrame`] so `millis` does not round-trip through seconds.
#[allow(clippy::too_many_arguments)]
pub fn draw_with_frame(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    frame: GlintFrame,
) {
    if state.is_empty() {
        return;
    }
    // A static item drawn where it was drawn before gives the same vertices:
    // take them instead of submitting, sorting and clipping its quads again.
    let key = DrawnKey::of(state, renderer, sprites, x, y);
    if let Some(key) = &key
        && let Some(drawn) = renderer
            .gui_drawn
            .lock()
            .ok()
            .and_then(|d| d.get(key).cloned())
    {
        if !is_oversized_target(state) {
            renderer.gui_item_slot(state, batch.scale.max(1.0));
        }
        batch.vertices.extend_from_slice(&drawn);
        return;
    }
    let start = batch.vertices.len();
    draw_now(state, renderer, sprites, host, batch, x, y, frame, None);
    if let Some(key) = key
        && let Ok(mut drawn) = renderer.gui_drawn.lock()
    {
        // GUI positions and items are few; a long scroll through every tab stays bounded.
        if drawn.len() >= DRAWN_LIMIT {
            drawn.clear();
        }
        drawn.insert(key, batch.vertices[start..].into());
    }
}

/// [`draw`] clipped to `scissor` (`[x0, y0, x1, y1]`) as well as the item's slot:
/// a list's scissor cuts the item of a half-visible row exactly. Not cached, the
/// scissor is not part of the cache key.
#[allow(clippy::too_many_arguments)]
pub fn draw_scissored(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    scissor: [f32; 4],
) {
    if state.is_empty() {
        return;
    }
    draw_now(
        state,
        renderer,
        sprites,
        host,
        batch,
        x,
        y,
        GlintFrame::default(),
        Some(scissor),
    );
}

/// Entries of `ItemRenderer::gui_drawn` before it starts over.
const DRAWN_LIMIT: usize = 8192;

/// What a GUI item's vertices depend on: its layers' baked models (kept for the
/// renderer's life, so their addresses name them), tints and transforms, the
/// atlas, and the exact position. None for items whose drawing changes with time
/// or the host: a glint, or a special model renderer.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct DrawnKey {
    renderer: usize,
    sprites: (usize, usize),
    at: [u32; 2],
    oversized: bool,
    layers: Vec<(usize, Vec<i32>, [u32; 16], bool)>,
}
impl DrawnKey {
    fn of(
        state: &ItemRenderState,
        renderer: &ItemRenderer,
        sprites: &HashMap<String, Rect>,
        x: f32,
        y: f32,
    ) -> Option<Self> {
        if state.animated || state.cit_glint.is_some() {
            return None;
        }
        let layers = state
            .layers
            .iter()
            .map(|layer| match (&layer.geometry, &layer.tints) {
                (super::LayerGeometry::Quads(baked), super::LayerTints::Owned(tints))
                    if layer.foil == super::FoilType::None
                        && layer.submit_options
                            == mod_api::SpecialModelSubmitOptions::default() =>
                {
                    Some((
                        Arc::as_ptr(baked) as usize,
                        tints.clone(),
                        layer.matrix.to_cols_array().map(f32::to_bits),
                        layer.uses_block_light,
                    ))
                }
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            renderer: std::ptr::from_ref(renderer) as usize,
            sprites: (std::ptr::from_ref(sprites) as usize, sprites.len()),
            at: [x.to_bits(), y.to_bits()],
            oversized: state.oversized_in_gui,
            layers,
        })
    }
}

/// `GuiItemAtlas.MINIMUM_TEXTURE_SIZE`.
const MINIMUM_ATLAS_TEXTURE_SIZE: u32 = 512;

/// `Mth.smallestSquareSide`: smallest `n` such that `n * n >= count`.
fn smallest_square_side(count: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    let mut side = (count as f64).sqrt().ceil() as u32;
    while side.saturating_mul(side) < count {
        side += 1;
    }
    while side > 0 && (side - 1).saturating_mul(side - 1) >= count {
        side -= 1;
    }
    side
}

/// `GuiItemAtlas.computeTextureSizeFor(slotTextureSize, requiredSlotCount)`.
fn compute_atlas_texture_size(
    slot_texture_size: u32,
    required_slots: u32,
    max_texture_size: u32,
) -> u32 {
    let preferred = required_slots.saturating_add(required_slots / 2).max(1);
    let side = smallest_square_side(preferred);
    let needed = side.saturating_mul(slot_texture_size);
    let pow2 = needed
        .checked_next_power_of_two()
        .unwrap_or(max_texture_size);
    let cap = max_texture_size.max(MINIMUM_ATLAS_TEXTURE_SIZE);
    pow2.clamp(MINIMUM_ATLAS_TEXTURE_SIZE, cap)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SlotGeometryKey {
    Quads(usize),
    Special(String, String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SlotLayerKey {
    geometry: SlotGeometryKey,
    tints: Vec<i32>,
    special_tints: Option<String>,
    foil: u8,
    matrix: [u32; 16],
    uses_block_light: bool,
    display: sunrise_assets::item_model::DisplayContext,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SlotKey {
    oversized: bool,
    layers: Vec<SlotLayerKey>,
}

impl SlotKey {
    fn of(state: &ItemRenderState) -> Self {
        let layers = state
            .layers
            .iter()
            .map(|layer| {
                let geometry = match &layer.geometry {
                    super::LayerGeometry::Quads(baked) => {
                        SlotGeometryKey::Quads(Arc::as_ptr(baked) as usize)
                    }
                    super::LayerGeometry::Special { model, argument } => {
                        SlotGeometryKey::Special(format!("{model:?}"), argument.to_string())
                    }
                };
                let (tints, special_tints) = match &layer.tints {
                    super::LayerTints::Owned(tints) => (tints.clone(), None),
                    super::LayerTints::Special(tints) => (Vec::new(), Some(format!("{tints:?}"))),
                };
                let foil = match layer.foil {
                    super::FoilType::None => 0,
                    super::FoilType::Standard => 1,
                    super::FoilType::Special => 2,
                };
                SlotLayerKey {
                    geometry,
                    tints,
                    special_tints,
                    foil,
                    matrix: layer.matrix.to_cols_array().map(f32::to_bits),
                    uses_block_light: layer.uses_block_light,
                    display: layer.display,
                }
            })
            .collect();
        Self {
            oversized: state.oversized_in_gui,
            layers,
        }
    }
}

fn is_animated_slot(state: &ItemRenderState) -> bool {
    state.animated
        || state.cit_glint.is_some()
        || state.layers.iter().any(|l| l.foil != super::FoilType::None)
}

#[derive(Clone, Copy)]
struct SlotEntry {
    index: u32,
    discard_after_frame: bool,
    used_this_frame: bool,
}

/// Vanilla 26.3's `GuiItemAtlas` / `DynamicAtlasAllocator` physical slot tracker per GUI scale.
#[derive(Default)]
pub(crate) struct GuiItemAtlasSlots {
    gui_scale: u32,
    slot_texture_size: u32,
    texture_size: u32,
    storage_size: u32,
    free: Vec<bool>,
    used: HashMap<SlotKey, SlotEntry>,
}

impl GuiItemAtlasSlots {
    fn reset(&mut self, gui_scale: u32, texture_size: u32) {
        self.gui_scale = gui_scale;
        self.slot_texture_size = 16 * gui_scale;
        self.texture_size = texture_size;
        self.storage_size = (texture_size / self.slot_texture_size).max(1);
        let total = (self.storage_size * self.storage_size) as usize;
        self.free.clear();
        self.free.resize(total, true);
        self.used.clear();
    }

    pub(crate) fn end_frame(&mut self) {
        let free = &mut self.free;
        self.used.retain(|_, entry| {
            if entry.discard_after_frame {
                if let Some(slot) = free.get_mut(entry.index as usize) {
                    *slot = true;
                }
                false
            } else {
                entry.used_this_frame = false;
                true
            }
        });
    }

    pub(crate) fn slot_for(
        &mut self,
        state: &ItemRenderState,
        gui_scale: f32,
        max_texture_size: u32,
    ) -> (u32, u32) {
        let scale = (gui_scale.round() as u32).max(1);
        if self.gui_scale != scale || self.storage_size == 0 {
            let tex = compute_atlas_texture_size(16 * scale, 1, max_texture_size);
            self.reset(scale, tex);
        }
        let key = SlotKey::of(state);
        let animated = is_animated_slot(state);
        if let Some(entry) = self.used.get_mut(&key) {
            entry.discard_after_frame |= animated;
            entry.used_this_frame = true;
            return (
                entry.index % self.storage_size,
                entry.index / self.storage_size,
            );
        }
        let mut free_idx = self.free.iter().position(|&f| f);
        if free_idx.is_none() {
            // `DynamicAtlasAllocator.reclaimSpaceFor`: reclaim a slot not used this frame.
            if let Some(evict_key) = self
                .used
                .iter()
                .filter(|(_, e)| !e.used_this_frame)
                .min_by_key(|(_, e)| e.index)
                .map(|(k, _)| k.clone())
                && let Some(evicted) = self.used.remove(&evict_key)
            {
                self.free[evicted.index as usize] = true;
                free_idx = Some(evicted.index as usize);
            }
        }
        if free_idx.is_none() {
            let new_tex = compute_atlas_texture_size(
                self.slot_texture_size,
                (self.used.len() + 1) as u32,
                max_texture_size,
            );
            if new_tex > self.texture_size {
                let mut live: Vec<(SlotKey, SlotEntry)> = self
                    .used
                    .drain()
                    .filter(|(_, e)| e.used_this_frame)
                    .collect();
                live.sort_by_key(|(_, e)| e.index);
                self.reset(scale, new_tex);
                for (i, (k, mut e)) in live.into_iter().enumerate() {
                    if i < self.free.len() {
                        self.free[i] = false;
                        e.index = i as u32;
                        self.used.insert(k, e);
                    }
                }
                free_idx = self.free.iter().position(|&f| f);
            }
        }
        let Some(idx) = free_idx else {
            return (0, 0);
        };
        self.free[idx] = false;
        let index = idx as u32;
        self.used.insert(
            key,
            SlotEntry {
                index,
                discard_after_frame: animated,
                used_this_frame: true,
            },
        );
        (index % self.storage_size, index / self.storage_size)
    }
}

/// Whether `state` uses vanilla's `OversizedItemRenderer` target instead of `GuiItemAtlas`.
fn is_oversized_target(state: &ItemRenderState) -> bool {
    if !state.oversized_in_gui {
        return false;
    }
    let (lo, hi) = state.bounding_box();
    let actual_x_size = ((f64::from(hi.x) - f64::from(lo.x)) * 16.0).ceil() as i32;
    let actual_y_size = ((f64::from(hi.y) - f64::from(lo.y)) * 16.0).ceil() as i32;
    actual_x_size > 16 || actual_y_size > 16
}

/// Vanilla's pre-placement GUI pose (`GuiItemAtlas.drawToSlot` or `OversizedItemRenderer`),
/// screen-space origin `(rel_x0, rel_y0)` relative to `(x, y)`, and physical atlas slot origin
/// `(slot_left, slot_top)` in target pixels.
fn gui_target_pose(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    gui_scale: f32,
) -> (Mat4, f32, f32, f32, f32) {
    if state.oversized_in_gui {
        let (lo, hi) = state.bounding_box();
        let actual_x_size = ((f64::from(hi.x) - f64::from(lo.x)) * 16.0).ceil() as i32;
        let actual_y_size = ((f64::from(hi.y) - f64::from(lo.y)) * 16.0).ceil() as i32;
        if actual_x_size > 16 || actual_y_size > 16 {
            let floored_x_offset = ((f64::from(lo.x) * 16.0) as f32).floor() as i32;
            let floored_y_offset = ((f64::from(hi.y) * 16.0) as f32).floor() as i32;
            let rel_x0 = floored_x_offset + 8;
            let rel_y0 = -floored_y_offset + 8;
            let width = actual_x_size as f32 * gui_scale;
            let height = actual_y_size as f32 * gui_scale;
            let scale = gui_scale * 16.0;
            let center_x = (rel_x0 + rel_x0 + actual_x_size) as f32 * 0.5;
            let center_y = (rel_y0 + rel_y0 + actual_y_size) as f32 * 0.5;
            let pose = Mat4::from_translation(Vec3::new(width * 0.5, height * 0.5, 0.0))
                * Mat4::from_scale(Vec3::new(scale, scale, -scale))
                * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0))
                * Mat4::from_translation(Vec3::new(
                    (8.0 - center_x) / 16.0,
                    (center_y - 8.0) / 16.0,
                    0.0,
                ));
            return (pose, rel_x0 as f32, rel_y0 as f32, 0.0, 0.0);
        }
    }
    let (slot_x, slot_y) = renderer.gui_item_slot(state, gui_scale);
    let slot_size = 16.0 * gui_scale;
    let slot_left = slot_x as f32 * slot_size;
    let slot_top = slot_y as f32 * slot_size;
    let pose = Mat4::from_translation(Vec3::new(
        slot_left + slot_size * 0.5,
        slot_top + slot_size * 0.5,
        0.0,
    )) * Mat4::from_scale(Vec3::new(slot_size, -slot_size, slot_size));
    (pose, 0.0, 0.0, slot_left, slot_top)
}

/// [`draw`] without the cache.
#[allow(clippy::too_many_arguments)]
fn draw_now(
    state: &ItemRenderState,
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    host: &dyn SpecialHost,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    frame: GlintFrame,
    scissor: Option<[f32; 4]>,
) {
    let gui_scale = batch.scale.max(1.0);
    let (target_pose, rel_x0, rel_y0, slot_left, slot_top) =
        gui_target_pose(state, renderer, gui_scale);
    let mut quads: Vec<ItemQuad> = Vec::new();
    state.submit(renderer, sprites, host, target_pose, &mut quads);
    // Place submitted target-space geometry onto the GUI screen after decal UV3 projection.
    let origin_x = x + rel_x0;
    let origin_y = y + rel_y0;
    for q in &mut quads {
        for p in &mut q.pos {
            *p = Vec3::new(
                origin_x + (p.x - slot_left) / gui_scale,
                origin_y + (p.y - slot_top) / gui_scale,
                p.z / gui_scale,
            );
        }
    }
    let lights = if state.uses_block_light() {
        items_3d_lights()
    } else {
        items_flat_lights()
    };
    let rect = if state.oversized_in_gui {
        [
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::INFINITY,
        ]
    } else {
        [x, y, x + 16.0, y + 16.0]
    };
    let rect = match scissor {
        Some([x0, y0, x1, y1]) => [
            rect[0].max(x0),
            rect[1].max(y0),
            rect[2].min(x1),
            rect[3].min(y1),
        ],
        None => rect,
    };
    let glint = frame.glint(Glint::ITEM_SCALE);
    batch.glint_alpha = frame.alpha;
    let cit = state.cit_glint.as_deref().zip(renderer.cit.as_ref());
    let cit = cit.map(|(glint, models)| CitDraw {
        glint,
        textures: &models.glint_textures,
        rules: &models.rules.rules,
        millis: frame.millis,
    });
    draw_quads_with(&quads, lights, batch, rect, glint.as_ref(), cit);
}

/// What an item's drawing reads of the game besides the item itself: the renderer, the atlas'
/// sprites and the host of the special models.
pub struct GuiItemEnv<'a> {
    pub renderer: &'a ItemRenderer,
    pub sprites: &'a HashMap<String, Rect>,
    pub host: &'a dyn SpecialHost,
}

/// An item drawn under the caller's `pose` (GUI pixels, the slot's `translate(x + 8, y + 8)`
/// and `scale(16, -16, 16)` with a turn of the caller's in between), lit and culled as in a
/// slot but cut by no slot, and with a depth of its own like a GUI model
/// ([`HudBatch::depth_glint_over`]): its faces meet each other by depth, whatever the turn, and
/// never intersect what the screen drew before or after. The OptiFine CIT glint layers are not
/// drawn. False, with nothing drawn, when a corner is not finite.
pub fn draw_posed(
    state: &ItemRenderState,
    env: &GuiItemEnv,
    batch: &mut HudBatch,
    pose: Mat4,
    frame: GlintFrame,
) -> bool {
    if state.is_empty() {
        return true;
    }
    let mut quads: Vec<ItemQuad> = Vec::new();
    state.submit(env.renderer, env.sprites, env.host, pose, &mut quads);
    if !quads.iter().all(|q| q.pos.iter().all(|p| p.is_finite())) {
        return false;
    }
    let lights = if state.uses_block_light() {
        items_3d_lights()
    } else {
        items_flat_lights()
    };
    batch.glint_alpha = frame.alpha;
    let glint = frame.glint(Glint::ITEM_SCALE);
    let start = batch.vertices.len();
    // The foil's faces follow the item's own, as `depth_glint_over` takes them.
    let mut foil = Vec::new();
    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for face in ordered_gui_faces(&quads) {
        let q = &quads[face.quad];
        let color = lit_color(q, lights);
        let light = hud_light_word(q);
        // A whole quad goes out as one; a face the sort split goes out as a fan of triangles.
        let fan: Vec<[_; 4]> = if let Ok(quad) = <[_; 4]>::try_from(&face.corners[..]) {
            vec![quad]
        } else {
            (1..face.corners.len().saturating_sub(1))
                .map(|i| {
                    let c = face.corners[i + 1];
                    [face.corners[0], face.corners[i], c, c]
                })
                .collect()
        };
        for corners in fan {
            for c in corners {
                for axis in 0..2 {
                    lo[axis] = lo[axis].min(c.pos[axis]);
                    hi[axis] = hi[axis].max(c.pos[axis]);
                }
            }
            if q.foil != Foil::GlintOnly {
                batch.quad_3d_lit(
                    corners.map(|c| c.pos.to_array()),
                    corners.map(|c| c.uv),
                    color,
                    light,
                );
            }
            if q.foil != Foil::None && glint.is_some() {
                foil.push((q.entity_glint, corners));
            }
        }
    }
    let glint_start = batch.vertices.len();
    if let Some(glint) = glint {
        for (entity_glint, corners) in foil {
            let glint = glint.for_quad(entity_glint);
            batch.quad_3d_glint(
                corners.map(|c| c.pos.to_array()),
                corners.map(|c| c.uv),
                corners.map(|c| glint.uv(c.glint_uv)),
                false,
            );
        }
    }
    if batch.vertices.len() > start {
        batch.depth_glint_over(start, glint_start, lo, hi);
    }
    true
}

/// Lights, culls, sorts far to near and clips `quads` (GUI positions) to
/// `rect`.
fn draw_quads(
    quads: &[ItemQuad],
    lights: (Vec3, Vec3),
    batch: &mut HudBatch,
    rect: [f32; 4],
    glint: Option<&Glint>,
) {
    draw_quads_with(quads, lights, batch, rect, glint, None);
}

/// Order GUI faces with cutout/solid before translucent when any quad carries an explicit
/// `SpecialModelRenderType`, keeping `depth::sort` order within each phase (and identical order
/// when no explicit override is present).
fn ordered_gui_faces(quads: &[ItemQuad]) -> Vec<depth::Face> {
    let mut faces = depth::sort(quads);
    if quads.iter().any(|q| q.explicit_alpha_cutoff) {
        faces.sort_by_key(|face| quads[face.quad].translucent);
    }
    faces
}

/// [`draw_quads`], then a stack's CIT glint layers over every face.
fn draw_quads_with(
    quads: &[ItemQuad],
    lights: (Vec3, Vec3),
    batch: &mut HudBatch,
    rect: [f32; 4],
    glint: Option<&Glint>,
    cit: Option<CitDraw>,
) {
    // The faces the CIT layers go over once the item is drawn.
    let mut fans = Vec::new();
    for face in ordered_gui_faces(quads) {
        let q = &quads[face.quad];
        let points: Vec<_> = face
            .corners
            .iter()
            .map(|v| Point {
                p: v.pos.truncate(),
                uv: v.uv,
                glint_uv: v.glint_uv,
                cit_uv: v.cit_uv,
            })
            .collect();
        let color = lit_color(q, lights);
        let light = hud_light_word(q);
        for i in 1..points.len().saturating_sub(1) {
            let tri = [points[0], points[i], points[i + 1]];
            let fan = clip_triangle(tri, rect);
            if fan.len() < 3 {
                continue;
            }
            if q.foil != Foil::GlintOnly {
                emit(batch, &fan, |_| color, light, None);
                if cit.is_some() {
                    fans.push(fan.clone());
                }
            }
            if q.foil != Foil::None {
                let quad_glint = glint.map(|g| g.for_quad(q.entity_glint));
                emit(batch, &fan, |_| color, 0, quad_glint.as_ref());
            }
        }
    }
    if let Some(cit) = cit {
        emit_cit(batch, &fans, cit);
    }
}

/// `PictureInPictureRenderer.prepare` for the loom's 20x40 result rectangle
/// at `(x0, y0)` (`translate(w / 2, h)`, `scale(16, 16, -16)`, one texel per
/// GUI pixel), then `GuiBannerResultRenderer.renderToTexture`'s
/// `translate(0, 0.25, 0)`.
pub fn banner_result_pose(x0: f32, y0: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(x0 + 10.0, y0 + 40.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, 16.0, -16.0))
        * Mat4::from_translation(Vec3::new(0.0, 0.25, 0.0))
}

/// The quads of `GuiGraphicsExtractor.bannerPattern` (the loom's result
/// preview) at `(x0, y0)`: base dye and `(texture, dye)` layers.
pub fn banner_result_quads(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    x0: f32,
    y0: f32,
    base: &str,
    layers: &[(String, String)],
) -> Vec<ItemQuad> {
    let mut quads = Vec::new();
    super::special::submit_banner_result(
        renderer,
        banner_result_pose(x0, y0),
        base,
        layers,
        sprites,
        &mut quads,
    );
    quads
}

/// Draws the loom's result preview: the picture in picture is lit with
/// `Lighting.Entry.ITEMS_FLAT` and holds only its 20x40 rectangle.
pub fn draw_banner_result(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    batch: &mut HudBatch,
    x0: f32,
    y0: f32,
    base: &str,
    layers: &[(String, String)],
) {
    let quads = banner_result_quads(renderer, sprites, x0, y0, base, layers);
    draw_quads(
        &quads,
        items_flat_lights(),
        batch,
        [x0, y0, x0 + 20.0, y0 + 40.0],
        None,
    );
}

/// `Lighting.Entry.ENTITY_IN_UI` (`setupForEntityInInventory`), which the
/// enchanting screen's book is lit with.
fn entity_in_ui_lights() -> (Vec3, Vec3) {
    (
        Vec3::new(0.2, -1.0, -1.0).normalize(),
        Vec3::new(-0.2, -1.0, 0.0).normalize(),
    )
}

/// `PictureInPictureRenderer.prepare` for the enchanting screen's 38x31 book
/// box at `(x0, y0)` (`translate(w / 2, 17)`, `scale(40, 40, -40)`), then
/// `GuiBookModelRenderer.renderToTexture`'s turns for `open`.
pub fn enchanting_book_pose(x0: f32, y0: f32, open: f32) -> Mat4 {
    let closed = 1.0 - open;
    Mat4::from_translation(Vec3::new(x0 + 19.0, y0 + 17.0, 0.0))
        * Mat4::from_scale(Vec3::new(40.0, 40.0, -40.0))
        * Mat4::from_rotation_y(PI)
        * Mat4::from_rotation_x(25f32.to_radians())
        * Mat4::from_translation(Vec3::new(closed * 0.2, closed * 0.1, closed * 0.25))
        * Mat4::from_rotation_y((-closed * 90.0 - 90.0).to_radians())
        * Mat4::from_rotation_x(PI)
}

/// `GuiBookModelRenderer.renderToTexture`'s page flips for `flip`: each
/// `clamp(frac(flip + offset) * 1.6 - 0.3, 0, 1)`.
pub fn enchanting_book_pages(flip: f32) -> (f32, f32) {
    let page = |offset: f32| {
        let x = flip + offset;
        ((x - x.floor()) * 1.6 - 0.3).clamp(0.0, 1.0)
    };
    (page(0.25), page(0.75))
}

/// `GuiGraphicsExtractor.book` from `EnchantmentScreen.extractBook`: the
/// enchanting table's book, `open` (0 to 1) and turning its pages with
/// `flip`, in the 38x31 box at `(x0, y0)`.
pub fn draw_enchanting_book(
    renderer: &ItemRenderer,
    sprites: &HashMap<String, Rect>,
    batch: &mut HudBatch,
    x0: f32,
    y0: f32,
    open: f32,
    flip: f32,
) {
    let (page1, page2) = enchanting_book_pages(flip);
    let mut quads = Vec::new();
    super::special::submit_book(
        renderer,
        enchanting_book_pose(x0, y0, open),
        open,
        page1,
        page2,
        sprites,
        &mut quads,
    );
    draw_quads(
        &quads,
        entity_in_ui_lights(),
        batch,
        [x0, y0, x0 + 38.0, y0 + 31.0],
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enchanting_book_pages_turn_with_the_flip() {
        let close = |flip: f32, want: (f32, f32)| {
            let (a, b) = enchanting_book_pages(flip);
            assert!(
                (a - want.0).abs() < 1e-5 && (b - want.1).abs() < 1e-5,
                "{flip}: {a} {b}, want {want:?}"
            );
        };
        close(0.0, (0.1, 0.9));
        // A whole page further, the same pages.
        close(1.0, (0.1, 0.9));
        close(0.5, (0.9, 0.1));
        // The pages rest flat at either end of a turn.
        close(-0.25, (0.0, 0.5));
        close(0.6875, (1.0, 0.4));
    }
    #[test]
    fn a_closed_book_turns_away_and_an_open_one_faces_the_screen() {
        // The box centre maps the model origin; the book's origin moves with
        // `open` (`translate((1 - open) * (0.2, 0.1, 0.25))` before the turn).
        let open = enchanting_book_pose(0.0, 0.0, 1.0).transform_point3(Vec3::ZERO);
        assert!(
            (open - Vec3::new(19.0, 17.0, 0.0)).length() < 1e-4,
            "{open}"
        );
        let closed = enchanting_book_pose(0.0, 0.0, 0.0).transform_point3(Vec3::ZERO);
        assert!((closed - open).length() > 1.0, "{closed}");
    }

    fn face(y0: f32, y1: f32, z: impl Fn(f32) -> f32, color: u32) -> ItemQuad {
        ItemQuad {
            pos: [(0.0, y0), (0.0, y1), (8.0, y1), (8.0, y0)].map(|(x, y)| Vec3::new(x, y, z(y))),
            uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            foil_uv: [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            gui_glint_uv: None,
            normal: Vec3::Z,
            color,
            foil: Foil::None,
            entity_glint: false,
            light_emission: 0,
            cull_back: false,
            translucent: false,
            oit: false,
            banner_pattern: false,
            packed_light: None,
            lightmap_only: false,
            explicit_alpha_cutoff: false,
        }
    }

    // Sample the emitted triangles in draw order, using opaque test faces.
    fn color_at(batch: &HudBatch, p: Vec2) -> Option<u32> {
        batch
            .vertices
            .as_chunks::<3>()
            .0
            .iter()
            .rev()
            .find_map(|tri| {
                let [a, b, c] = std::array::from_fn(|i| Vec2::from(tri[i].pos));
                let area = (b - a).perp_dot(c - a);
                if area.abs() < 1e-6 {
                    return None;
                }
                let u = (b - p).perp_dot(c - p) / area;
                let v = (c - p).perp_dot(a - p) / area;
                let w = 1.0 - u - v;
                (u >= 0.0 && v >= 0.0 && w >= 0.0).then_some(tri[0].color)
            })
    }

    #[test]
    fn banner_cloth_occludes_the_support_at_the_top_edge() {
        // The cloth is nearer where it overlaps the bar, although its
        // average depth is farther away because it slopes along its height.
        let bar = face(0.0, 1.0, |_| 0.0, 0xFF80_4000);
        let cloth = face(0.0, 8.0, |y| 2.0 - y, 0xFFFF_FFFF);
        let mut batch = HudBatch::scratch([320, 240]);
        draw_quads(
            &[bar, cloth],
            (Vec3::Z, Vec3::Z),
            &mut batch,
            [0.0, 0.0, 16.0, 16.0],
            None,
        );
        assert_eq!(color_at(&batch, Vec2::new(4.0, 0.5)), Some(u32::MAX));
    }

    #[test]
    fn intersecting_faces_change_draw_order_at_their_intersection() {
        let flat = face(0.0, 8.0, |_| 0.0, 0xFFFF_0000);
        let tilted = face(0.0, 8.0, |y| y - 4.0, 0xFF00_00FF);
        for quads in [[flat, tilted], [tilted, flat]] {
            let mut batch = HudBatch::scratch([320, 240]);
            draw_quads(
                &quads,
                (Vec3::Z, Vec3::Z),
                &mut batch,
                [0.0, 0.0, 16.0, 16.0],
                None,
            );
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, 2.0)),
                Some(lit_color(&flat, (Vec3::Z, Vec3::Z)))
            );
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, 6.0)),
                Some(lit_color(&tilted, (Vec3::Z, Vec3::Z)))
            );
            // The cut must interpolate the texture along with the position.
            for v in &batch.vertices {
                assert!((v.uv[0] - v.pos[0] / 8.0).abs() < 1e-6);
                assert!((v.uv[1] - v.pos[1] / 8.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn coplanar_pattern_layers_keep_submission_order() {
        let base = face(0.0, 8.0, |y| 2.0 - y, 0xFFFF_FFFF);
        let pattern = ItemQuad {
            color: 0xFF00_00FF,
            ..base
        };
        let quads = [face(0.0, 1.0, |_| 0.0, 0xFF80_4000), base, pattern];
        let mut batch = HudBatch::scratch([320, 240]);
        draw_quads(
            &quads,
            (Vec3::Z, Vec3::Z),
            &mut batch,
            [0.0, 0.0, 16.0, 16.0],
            None,
        );
        for y in [0.5, 6.0] {
            assert_eq!(
                color_at(&batch, Vec2::new(4.0, y)),
                Some(lit_color(&quads[2], (Vec3::Z, Vec3::Z)))
            );
        }
    }

    #[test]
    fn cit_layers_draw_once_over_the_whole_item() {
        use crate::cit_glint::Global;
        use crate::hud::Blend;
        use crate::hud::cit_glint::Slot;
        use sunrise_optifine::Properties;
        let rule = |name: &str, text: &str| {
            let path = format!("assets/minecraft/optifine/cit/{name}.properties");
            let text = format!("type=enchantment\ntexture=glint\n{text}");
            Rule::parse(&path, &Properties::parse_equals_first(&text), |_| true).unwrap()
        };
        let rules = [
            rule("shown", ""),
            rule("fire", "layer=1\nenchantments=fire_aspect"),
        ];
        // Shared by level, and the stack has no Fire Aspect: that layer is at intensity 0.
        let global = Global::parse(&Properties::parse_equals_first("method=average"));
        let glint = StackGlint::new(&rules, &[0, 1], &[("minecraft:sharpness", 3)], &global);
        assert_eq!(glint.layers.len(), 2);
        let slot = |blend| Slot {
            rgba: [1.0; 4],
            blur: false,
            blend,
        };
        let textures = GlintTextures {
            width: 1,
            height: 1,
            slot_of: vec![Some(1), Some(0)],
            slots: vec![slot(0), slot(1)],
            ..GlintTextures::default()
        };
        let cit = CitDraw {
            glint: &glint,
            textures: &textures,
            rules: &rules,
            millis: 0,
        };
        let mut batch = HudBatch::scratch([320, 240]);
        batch.glint_alpha = 0.5;
        let mut quad = face(0.0, 8.0, |_| 0.0, 0xFFFF_FFFF);
        quad.gui_glint_uv = Some([[42.0, 42.0]; 4]);
        draw_quads_with(
            &[quad],
            (Vec3::Z, Vec3::Z),
            &mut batch,
            [0.0, 0.0, 16.0, 16.0],
            None,
            Some(cit),
        );
        // The square's two triangles, then the one layer over both: six vertices a triangle.
        assert_eq!(batch.vertices.len(), 24);
        assert_eq!(
            batch.runs(),
            [(0..12, Blend::Alpha), (12..24, Blend::CitGlint(1))]
        );
        for v in &batch.vertices[12..] {
            // Slot 1 in red; full intensity at half the glint strength in alpha.
            assert_eq!(v.color.to_le_bytes(), [1, 0, 0, 128]);
            assert_eq!(v.glint_uv, v.pos, "CIT retains the sprite-local UVs");
        }
    }

    #[test]
    fn flat_lights_match_joml() {
        // new Matrix4f().rotationY(-PI/8).rotateX(3PI/4).transformDirection(L0).
        let (l0, _) = items_flat_lights();
        let l = Vec3::new(0.2, 1.0, -0.7).normalize();
        let (s, c) = ((3.0 * PI / 4.0).sin(), (3.0 * PI / 4.0).cos());
        let rx = Vec3::new(l.x, c * l.y - s * l.z, s * l.y + c * l.z);
        let (s, c) = ((-PI / 8.0).sin(), (-PI / 8.0).cos());
        let ry = Vec3::new(c * rx.x + s * rx.z, rx.y, -s * rx.x + c * rx.z);
        assert!((l0 - ry).length() < 1e-6);
    }
    #[test]
    fn triangles_clip_to_the_slot() {
        let p = |x: f32, y: f32| Point {
            p: Vec2::new(x, y),
            uv: [x, y],
            glint_uv: [0.0, 0.0],
            cit_uv: [0.0, 0.0],
        };
        let fan = clip_triangle(
            [p(-4.0, 0.0), p(8.0, 0.0), p(8.0, 8.0)],
            [0.0, 0.0, 16.0, 16.0],
        );
        assert!(fan.iter().all(|v| v.p.x >= 0.0));
        // UVs follow the cut affinely.
        assert!(fan.iter().all(|v| (v.uv[0] - v.p.x).abs() < 1e-6));
        assert!(
            clip_triangle(
                [p(20.0, 0.0), p(30.0, 0.0), p(30.0, 5.0)],
                [0.0, 0.0, 16.0, 16.0]
            )
            .is_empty()
        );
    }
}
