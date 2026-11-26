use super::*;
<<<<<<< HEAD
#[path = "batch/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;
||||||| 46c00106def
=======
use std::ops::Range;
>>>>>>> ed6772e2118

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HudVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: u32,
    pub depth: f32,
    /// The `GLINT_TEXTURING` UV into the repeating glint texture, unused
    /// unless `uv.x >= 2`.
    pub glint_uv: [f32; 2],
    /// Optional explicit packed lightmap coordinates and alpha-cutoff flags for special item
    /// submissions (`0` for normal HUD draws): bit 24 enables frame lightmap sampling (bits
    /// `0..8` = block byte, bits `8..16` = sky byte), bit 25 selects `0.1` item alpha cutoff.
    pub light: u32,
    /// 1 for the GUI's text (`RenderPipelines.GUI_TEXT`), which a resource
    /// pack's text shader draws when it has one.
    pub text: u32,
}
/// A triangle clipped by a rectangle has at most seven vertices.
type ClippedTriangle = SmallVec<[HudVertex; 7]>;

fn triangle_area([a, b, c]: &[HudVertex; 3]) -> f32 {
    (b.pos[0] - a.pos[0]) * (c.pos[1] - a.pos[1]) - (c.pos[0] - a.pos[0]) * (b.pos[1] - a.pos[1])
}

fn clip_triangle(triangle: &[HudVertex; 3], [x0, y0, x1, y1]: [f32; 4]) -> ClippedTriangle {
    let mut poly = ClippedTriangle::from_slice(triangle);
    for (axis, limit, keep_below) in [(0, x0, false), (0, x1, true), (1, y0, false), (1, y1, true)]
    {
        poly = clip_polygon(&poly, axis, limit, keep_below);
    }
    poly
}

/// Sutherland-Hodgman against one half plane, interpolating every attribute.
fn clip_polygon(poly: &[HudVertex], axis: usize, limit: f32, keep_below: bool) -> ClippedTriangle {
    let inside = |v: &HudVertex| {
        if keep_below {
            v.pos[axis] < limit
        } else {
            v.pos[axis] >= limit
        }
    };
    let lerp = |a: &HudVertex, b: &HudVertex, t: f32| {
        let mix = |p: f32, q: f32| p + (q - p) * t;
        let mut v = *a;
        v.pos = [mix(a.pos[0], b.pos[0]), mix(a.pos[1], b.pos[1])];
        v.uv = [mix(a.uv[0], b.uv[0]), mix(a.uv[1], b.uv[1])];
        v.depth = mix(a.depth, b.depth);
        // A shaded shape (a gradient, `hud_kit::shapes`) keeps its colours across the cut.
        v.color = if a.color == b.color {
            a.color
        } else {
            let channel = |shift: u32| {
                let (p, q) = (
                    ((a.color >> shift) & 0xFF) as f32,
                    ((b.color >> shift) & 0xFF) as f32,
                );
                ((p + (q - p) * t).round().clamp(0.0, 255.0) as u32) << shift
            };
            channel(0) | channel(8) | channel(16) | channel(24)
        };
        v.glint_uv = [
            mix(a.glint_uv[0], b.glint_uv[0]),
            mix(a.glint_uv[1], b.glint_uv[1]),
        ];
        v.pos[axis] = limit;
        v
    };
    let mut out = ClippedTriangle::new();
    for i in 0..poly.len() {
        let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
        let (ia, ib) = (inside(a), inside(b));
        if ia {
            out.push(*a);
        }
        if ia != ib {
            let t = (limit - a.pos[axis]) / (b.pos[axis] - a.pos[axis]);
            out.push(lerp(a, b, t));
        }
    }
    out
}
/// Scale inputs for one view, copied from its owner's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuiScale {
    pub requested: u32,
    pub unicode: bool,
}
/// One frame of overlay geometry in GUI pixels.
pub struct HudBatch {
    pub pictures: Vec<Arc<pictures::Picture>>,
    pub vertices: Vec<HudVertex>,
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    /// Physical viewport; GUI width and height round up at non-integer ratios.
    viewport: [u32; 2],
    /// Glyph pixels to upload into the atlas before drawing.
    pub patches: Vec<crate::font::Patch>,
    /// The changes to client mods' dynamic textures, applied in order before drawing. Never
    /// rolled back: a change a refused draw would have followed was still made.
    pub dynamic_commands: Vec<dynamic_textures::Command>,
    /// Vertex ranges with another blend or depth/scissor state, in order.
    pub blends: Vec<(Range<u32>, Blend)>,
    /// Disjoint, ordered vertex ranges with a screen-space scissor in GUI pixels.
    scissors: SmallVec<[(Range<u32>, [f32; 4]); 4]>,
    /// The textures and time of the `Blend::EndPortal` runs.
    pub end_portal: EndPortalFill,
    /// `options.glintStrength` for the glint pass (`GlintAlpha`).
    pub glint_alpha: f32,
    /// PvP & HUD's glint colour (`pvp::Tweaks::glint_texel`), 0 for vanilla's.
    pub glint_colour: u32,
    /// A resource pack text shader's `GameTime`: the day's fraction
    /// (`gameTime % 24000` and the partial tick, over 24000).
    pub game_time: f32,
    /// Screen's first vertex: the preceding HUD is part of its blurred background.
    pub blur_at: u32,
    /// Player object profiles encountered while painting text this frame.
    pub inline_profiles: Vec<std::sync::Arc<serde_json::Value>>,
}
struct HudTail {
    vertices: usize,
    pictures: usize,
    patches: usize,
    inline_profiles: usize,
    blends: usize,
    last_blend: Option<(std::ops::Range<u32>, Blend)>,
    end_portal: EndPortalFill,
    glint_alpha: f32,
    blur_at: u32,
}

struct HudDrawRollback<'a> {
    batch: &'a mut HudBatch,
    tail: Option<HudTail>,
}

impl Drop for HudDrawRollback<'_> {
    fn drop(&mut self) {
        let Some(tail) = self.tail.take() else { return };
        self.batch.truncate_vertices(tail.vertices);
        self.batch.pictures.truncate(tail.pictures);
        self.batch.patches.truncate(tail.patches);
        self.batch.inline_profiles.truncate(tail.inline_profiles);
        self.batch.blends.truncate(tail.blends);
        // `blend_since` can extend the preceding run instead of appending a new one.
        if let Some(last) = tail.last_blend {
            self.batch.blends[tail.blends - 1] = last;
        }
        self.batch.end_portal = tail.end_portal;
        self.batch.glint_alpha = tail.glint_alpha;
        self.batch.blur_at = tail.blur_at;
    }
}

/// The paint appended by one retained callback, excluding any painters it delegates to.
pub struct HudPaintCheckpoint {
    start: usize,
    pictures: usize,
    profiles: usize,
}

impl HudPaintCheckpoint {
    pub fn discard(self, batch: &mut HudBatch) {
        if self.start < batch.vertices.len() {
            let start = self.start as u32;
            batch.vertices.truncate(self.start);
            let first = batch.blends.partition_point(|(run, _)| run.end <= start);
            for (run, _) in &mut batch.blends[first..] {
                run.end = run.end.min(start);
            }
            batch.blends.retain(|(run, _)| run.start < run.end);
            let first = batch.scissors.partition_point(|(run, _)| run.end <= start);
            for (run, _) in &mut batch.scissors[first..] {
                run.end = run.end.min(start);
            }
            batch.scissors.retain(|(run, _)| run.start < run.end);
        }
        // Glyph atlas uploads are reusable by other painters.
        batch.pictures.truncate(self.pictures);
        batch.inline_profiles.truncate(self.profiles);
    }
}

/// How a run of HUD vertices blends and tests depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blend {
    Picture(u64),
    /// A run sampling the dynamic texture with this handle (`dynamic_textures::Command`).
    Dynamic(u64),
    /// A run sampling a separately stitched registered atlas page.
    AtlasPage(usize, AtlasBlend),
    /// `BlendFunction.TRANSLUCENT`.
    Alpha,
    /// `BlendFunction.INVERT` (`RenderPipelines.CROSSHAIR`, `GUI_INVERT`).
    Invert,
    /// `BlendFunction.ADDITIVE` (`RenderPipelines.GUI_TEXT_HIGHLIGHT`).
    Additive,
    /// Alpha blending with an isolated depth buffer and a pixel scissor.
    Depth([u32; 4]),
    /// The glint over the `Depth` run before it with the same scissor, in its
    /// pass: `RenderPipelines.GLINT`'s additive blend, depth tested against the
    /// model and never written.
    DepthGlint([u32; 4]),
    /// `RenderPipelines.VIGNETTE`: the destination darkened by the texture
    /// (`ZERO, ONE_MINUS_SRC_COLOR`).
    Vignette,
    /// `RenderPipelines.END_PORTAL` as a GUI fill: the End portal's layers
    /// projected from the screen (`HudBatch::end_portal` sets the textures).
    EndPortal,
    /// An OptiFine CIT glint layer (`cit_glint`), blended as entry `n` of
    /// `GlintTextures::blends` says.
    CitGlint(u8),
}
impl Blend {
    pub(super) fn scissor(self) -> Option<[u32; 4]> {
        match self {
            Self::Depth(clip) | Self::DepthGlint(clip) => Some(clip),
            _ => None,
        }
    }
}

/// The backend scissor, intersected with its viewport in device pixels.
pub(super) fn hud_scissor(clip: [u32; 4], size: [u32; 2], zoom: f32) -> Option<[u32; 4]> {
    let [x, y, w, h] = clip.map(|v| (v as f32 * zoom) as u32);
    let (x, y) = (x.min(size[0]), y.min(size[1]));
    let (w, h) = (w.min(size[0] - x), h.min(size[1] - y));
    (w != 0 && h != 0).then_some([x, y, w, h])
}

/// The 2D sampling pipelines that can draw a registered atlas's sprite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasBlend {
    Alpha,
    Invert,
    Additive,
    Vignette,
}

/// What a `Blend::EndPortal` fill samples: the atlas rects (u0, v0, width,
/// height) of `end_sky.png` and `end_portal.png`, and the `GameTime` uniform.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EndPortalFill {
    pub sprites: [[f32; 4]; 2],
    pub game_time: f32,
}
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (a as u32) << 24 | (b as u32) << 16 | (g as u32) << 8 | r as u32
}
impl HudBatch {
    /// Window.calculateScale: the largest scale keeping >= 320x240 GUI pixels,
    /// capped by the GUI Scale option (0 = auto).
    pub fn new(window: [u32; 2], inputs: GuiScale) -> Self {
        LAST_GUI_SCALE.store(
            (inputs.requested & 0x7fff_ffff) | (u32::from(inputs.unicode) << 31),
            std::sync::atomic::Ordering::Relaxed,
        );
        let gui =
            sunrise_client_input::window::GuiSize::new(window, inputs.requested, inputs.unicode);
        Self {
            pictures: Vec::new(),
            vertices: Vec::new(),
            scale: gui.scale as f32,
            width: gui.size[0] as f32,
            height: gui.size[1] as f32,
            viewport: window,
            patches: Vec::new(),
            dynamic_commands: Vec::new(),
            blends: Vec::new(),
            scissors: SmallVec::new(),
            end_portal: EndPortalFill::default(),
            glint_alpha: 0.75,
            glint_colour: 0,
            game_time: 0.0,
            blur_at: 0,
            inline_profiles: Vec::new(),
        }
    }
    /// An offscreen or test batch, independent of the active view's settings.
    pub fn scratch(window: [u32; 2]) -> Self {
        Self::new(
            window,
            GuiScale {
                requested: 0,
                unicode: false,
            },
        )
    }
    /// Start tracking one callback's writes to this batch without copying existing paint.
    pub fn paint_checkpoint(&self) -> HudPaintCheckpoint {
        HudPaintCheckpoint {
            start: self.vertices.len(),
            pictures: self.pictures.len(),
            profiles: self.inline_profiles.len(),
        }
    }

    /// Paint an append-only group, restoring the batch tail and the atlas font on `Err` or
    /// unwind. Nested groups use the same batch and font; a successful child is still undone
    /// if its parent aborts. Requested glyph uploads are appended only on outermost success.
    pub fn draw_scope<T, E>(
        &mut self,
        font: &crate::font::Font,
        draw: impl FnOnce(&mut Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let tail = HudTail {
            vertices: self.vertices.len(),
            pictures: self.pictures.len(),
            patches: self.patches.len(),
            inline_profiles: self.inline_profiles.len(),
            blends: self.blends.len(),
            last_blend: self.blends.last().cloned(),
            end_portal: self.end_portal,
            glint_alpha: self.glint_alpha,
            blur_at: self.blur_at,
        };
        let mut rollback = HudDrawRollback {
            batch: self,
            tail: Some(tail),
        };
        let font = font.begin_draw();
        let value = draw(rollback.batch)?;
        rollback.batch.patches.extend(font.commit());
        rollback.tail = None;
        Ok(value)
    }

    /// Draw the vertices added since `start` with the inverting blend.
    pub fn invert_since(&mut self, start: usize) {
        self.blend_since(start, Blend::Invert);
    }
    /// Draw the vertices added since `start` with `blend`.
    pub fn blend_since(&mut self, start: usize, blend: Blend) {
        let range = start as u32..self.vertices.len() as u32;
        if range.is_empty() || blend == Blend::Alpha {
            return;
        }
        match self.blends.last_mut() {
            Some((last, b)) if last.end == range.start && *b == blend => last.end = range.end,
            _ => self.blends.push((range, blend)),
        }
    }
    /// Make an ordinary GUI sprite draw use its registered atlas page, if it has one.
    pub fn bind_gui_atlas(&mut self, start: usize, atlas: &HudAtlas, name: &str) {
        if let Some(page) = atlas.gui_atlas_page(name) {
            self.blend_since(start, Blend::AtlasPage(page, AtlasBlend::Alpha));
        }
    }
    /// `GuiGraphicsExtractor.textHighlight`: with `invert_text` the area's
    /// colours are inverted first (white text on a dark field turns blue on
    /// white), then blue is added.
    pub fn text_highlight(
        &mut self,
        atlas: &HudAtlas,
        (x, y, w, h): (f32, f32, f32, f32),
        invert_text: bool,
    ) {
        if invert_text {
            let start = self.vertices.len();
            self.fill(atlas, x, y, w, h, rgba(255, 255, 255, 255));
            self.invert_since(start);
        }
        let start = self.vertices.len();
        self.fill(atlas, x, y, w, h, rgba(0, 0, 255, 255));
        self.blend_since(start, Blend::Additive);
    }
    /// The vertex ranges in order, each with its blend.
    pub fn runs(&self) -> Vec<(std::ops::Range<u32>, Blend)> {
        let mut out = Vec::new();
        let mut at = 0;
        for (r, blend) in &self.blends {
            if r.start > at {
                out.push((at..r.start, Blend::Alpha));
            }
            out.push((r.clone(), *blend));
            at = r.end;
        }
        let end = self.vertices.len() as u32;
        if end > at {
            out.push((at..end, Blend::Alpha));
        }
        out
    }
    pub(super) fn runs_in(
        &self,
        vertices: std::ops::Range<u32>,
    ) -> impl Iterator<Item = (std::ops::Range<u32>, Blend)> + '_ {
        // Scissors and other non-default runs are kept in vertex order.
        let first = self
            .blends
            .partition_point(|(run, _)| run.end <= vertices.start);
        let mut runs = self.blends[first..].iter().peekable();
        let end = vertices.end.min(self.vertices.len() as u32);
        let mut at = vertices.start;
        std::iter::from_fn(move || {
            while let Some((run, blend)) = runs.peek().copied().filter(|(run, _)| run.start < end) {
                if at < run.start {
                    let gap = at..run.start;
                    at = run.start;
                    return Some((gap, Blend::Alpha));
                }
                runs.next();
                let range = run.start.max(vertices.start)..run.end.min(end);
                at = at.max(range.end);
                if !range.is_empty() {
                    return Some((range, *blend));
                }
            }
            let tail = at..end;
            at = end;
            (!tail.is_empty()).then_some((tail, Blend::Alpha))
        })
    }
    /// `AbstractContainerScreen.init`: center in integer GUI coordinates.
    pub fn centered_origin(&self, width: f32, height: f32) -> (f32, f32) {
        (
            ((self.width - width) / 2.0).trunc(),
            ((self.height - height) / 2.0).trunc(),
        )
    }
    pub fn quad(&mut self, p: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: u32) {
        self.quad_lit(p, uv, color, 0);
    }
    /// [`Self::quad`] carrying an explicit [`HudVertex::light`] word for lightmapped GUI items.
    pub fn quad_lit(&mut self, p: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: u32, light: u32) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: p[i],
                uv: uv[i],
                color,
                depth: 0.0,
                glint_uv: [0.0, 0.0],
                light,
                text: 0,
            });
        }
    }
    /// A quad whose texture is the `GLINT_TEXTURING` sample at `glint_uv`;
    /// `uv` is the item's atlas UV, which the shader masks by.
    pub fn quad_glint(
        &mut self,
        p: [[f32; 2]; 4],
        uv: [[f32; 2]; 4],
        glint_uv: [[f32; 2]; 4],
        color: u32,
    ) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: p[i],
                uv: uv[i],
                color,
                depth: 0.0,
                glint_uv: glint_uv[i],
                light: 0,
                text: 0,
            });
        }
    }
    pub fn quad_3d(&mut self, p: [[f32; 3]; 4], uv: [[f32; 2]; 4], color: u32) {
        self.quad_3d_lit(p, uv, color, 0);
    }
    /// [`Self::quad_3d`] carrying an explicit [`HudVertex::light`] word for lightmapped GUI items.
    pub fn quad_3d_lit(&mut self, p: [[f32; 3]; 4], uv: [[f32; 2]; 4], color: u32, light: u32) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: [p[i][0], p[i][1]],
                uv: uv[i],
                color,
                depth: p[i][2],
                glint_uv: [0.0, 0.0],
                light,
                text: 0,
            });
        }
    }
    /// `RenderTypes.armorCutoutNoCullGlint` (and the items' glint types) over a
    /// [`Self::quad_3d`] face: `uv` is the face's atlas UV,
    /// which the shader masks by, and `glint_uv` the transformed glint UV.
    /// `armor` samples `enchanted_glint_armor` (`ARMOR_ENTITY_GLINT`), otherwise
    /// `enchanted_glint_item`. Drawn by [`Self::depth_glint_since`].
    pub fn quad_3d_glint(
        &mut self,
        p: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        glint_uv: [[f32; 2]; 4],
        armor: bool,
    ) {
        let marker = if armor { 4.0 } else { 2.0 };
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: [p[i][0], p[i][1]],
                uv: [uv[i][0] + marker, uv[i][1]],
                color: u32::MAX,
                depth: p[i][2],
                glint_uv: glint_uv[i],
                light: 0,
                text: 0,
            });
        }
    }
    /// Keep per-vertex depth for an orthographic GUI model, with +z toward the viewer. `clip` is
    /// an explicit scissor in GUI pixels, cut to whole device pixels as it always was.
    pub fn depth_since(&mut self, start: usize, clip: [f32; 4]) {
        self.depth_glint_since(start, self.vertices.len(), clip);
    }
    /// `RenderPipelines.GLINT` after a GUI model: [`Self::depth_since`] for a model whose glint faces
    /// ([`Self::quad_3d_glint`]) follow its own from `glint`: both share the
    /// depth buffer, and the glint is drawn over the model in the same pass.
    pub fn depth_glint_since(&mut self, start: usize, glint: usize, clip: [f32; 4]) {
        let [x0, y0, x1, y1] = clip.map(|v| (v * self.scale).max(0.0) as u32);
        self.depth_pass_since(
            start,
            glint,
            [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)],
        );
    }
    /// [`Self::depth_glint_since`] for the screen bounds of the geometry itself (`lo` to `hi`, in
    /// GUI pixels): the scissor takes every device pixel the bounds touch, so the min edges round
    /// down and the max edges up.
    pub fn depth_glint_over(&mut self, start: usize, glint: usize, lo: [f32; 2], hi: [f32; 2]) {
        let [x0, y0] = lo.map(|v| (v * self.scale).max(0.0).floor() as u32);
        let [x1, y1] = hi.map(|v| (v * self.scale).max(0.0).ceil() as u32);
        self.depth_pass_since(
            start,
            glint,
            [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)],
        );
    }
    /// [`Self::depth_since`] for the screen bounds of the geometry itself: see
    /// [`Self::depth_glint_over`].
    pub fn depth_over(&mut self, start: usize, lo: [f32; 2], hi: [f32; 2]) {
        self.depth_glint_over(start, self.vertices.len(), lo, hi);
    }
    /// The depth runs of `start..` with the scissor `clip` in device pixels (`[x, y, w, h]`).
    fn depth_pass_since(&mut self, start: usize, glint: usize, clip: [u32; 4]) {
        let glint = glint.clamp(start, self.vertices.len());
        let vertices = &mut self.vertices[start..];
        if vertices.is_empty() {
            return;
        }
        let min = vertices
            .iter()
            .map(|v| v.depth)
            .fold(f32::INFINITY, f32::min);
        let max = vertices
            .iter()
            .map(|v| v.depth)
            .fold(f32::NEG_INFINITY, f32::max);
        for v in vertices {
            v.depth = (max - v.depth) / (max - min).max(f32::EPSILON);
        }
        // Separate previews clear depth independently, even if their scissors match.
        if glint > start {
            self.blends
                .push((start as u32..glint as u32, Blend::Depth(clip)));
        }
        let end = self.vertices.len();
        if end > glint {
            self.blends
                .push((glint as u32..end as u32, Blend::DepthGlint(clip)));
        }
    }
    pub fn rect_uv(&mut self, x: f32, y: f32, w: f32, h: f32, r: Rect, color: u32) {
        self.quad(
            [[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
            color,
        );
    }
    pub fn sprite(&mut self, atlas: &HudAtlas, name: &str, x: f32, y: f32) {
        if let Some(r) = atlas.gui_draw_rect(name).copied() {
            let start = self.vertices.len();
            self.rect_uv(x, y, r.w as f32, r.h as f32, r, u32::MAX);
            self.bind_gui_atlas(start, atlas, name);
        }
    }
    pub fn sprite_scaled(
        &mut self,
        atlas: &HudAtlas,
        name: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: u32,
    ) {
        if let Some(r) = atlas.gui_draw_rect(name).copied() {
            let start = self.vertices.len();
            self.rect_uv(x, y, w, h, r, color);
            self.bind_gui_atlas(start, atlas, name);
        }
    }
    /// Draw part of a sprite, `fraction` of its width from the left.
    pub fn sprite_part(&mut self, atlas: &HudAtlas, name: &str, x: f32, y: f32, fraction: f32) {
        if let Some(mut r) = atlas.gui_draw_rect(name).copied() {
            let w = r.w as f32 * fraction;
            r.u1 = r.u0 + (r.u1 - r.u0) * fraction;
            let start = self.vertices.len();
            self.rect_uv(x, y, w, r.h as f32, r, u32::MAX);
            self.bind_gui_atlas(start, atlas, name);
        }
    }
    /// Scale the vertices pushed since `start` about `(cx, cy)` (a GUI pose
    /// matrix of translate, scale, translate back).
    pub fn scale_since(&mut self, start: usize, cx: f32, cy: f32, sx: f32, sy: f32) {
        let scale = |[x, y]: [f32; 2]| [cx + (x - cx) * sx, cy + (y - cy) * sy];
        for v in &mut self.vertices[start..] {
            v.pos = scale(v.pos);
        }
        // A legacy size parameter can scale a whole item draw, including clips added by
        // its decorator. The enclosing screen's scissor is not on this tail yet.
        let start = start as u32;
        let mut first = self
            .scissors
            .partition_point(|(range, _)| range.end <= start);
        if let Some((range, rect)) = self.scissors.get_mut(first)
            && range.start < start
        {
            let tail = (start..range.end, *rect);
            range.end = start;
            first += 1;
            self.scissors.insert(first, tail);
        }
        for (_, rect) in &mut self.scissors[first..] {
            let [l, t, r, b] = *rect;
            *rect = if l < r && t < b {
                let [x0, y0] = scale([l, t]);
                let [x1, y1] = scale([r, b]);
                [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
            } else {
                [0.0; 4]
            };
        }
    }
    /// Intersect `[x0, x1) × [y0, y1)` with the scissor of the draws since `start`.
    /// Like `GuiRenderer.executeDraw`, leave triangles and their attributes untouched: the
    /// rasterizer's scissor gates fragments without changing interpolation inside the rectangle.
    pub fn clip_since(&mut self, start: usize, x0: f32, y0: f32, x1: f32, y1: f32) {
        let mut at = start as u32;
        let end = self.vertices.len() as u32;
        let mut i = self.scissors.partition_point(|(range, _)| range.end <= at);
        while at < end {
            let next = self.scissors.get(i).map_or(end, |(range, _)| range.start);
            if at < next {
                let rect = [x0, y0, x1, y1];
                if i > 0 && self.scissors[i - 1].0.end == at && self.scissors[i - 1].1 == rect {
                    self.scissors[i - 1].0.end = next;
                    at = next;
                    continue;
                }
                self.scissors.insert(i, (at..next, rect));
                at = next;
            } else {
                let (range, [l, t, r, b]) = &self.scissors[i];
                let overlap = [l.max(x0), t.max(y0), r.min(x1), b.min(y1)];
                if range.start < at {
                    let tail = at..range.end;
                    self.scissors[i].0.end = at;
                    i += 1;
                    self.scissors.insert(i, (tail, overlap));
                } else {
                    self.scissors[i].1 = overlap;
                }
                at = self.scissors[i].0.end;
            }
            if i > 0
                && self.scissors[i - 1].0.end == self.scissors[i].0.start
                && self.scissors[i - 1].1 == self.scissors[i].1
            {
                let end = self.scissors.remove(i).0.end;
                self.scissors[i - 1].0.end = end;
            } else {
                i += 1;
            }
        }
    }

    /// The intervals of a draw run in order, including any unscissored gaps.
    pub fn scissors_in(
        &self,
        vertices: Range<u32>,
    ) -> impl Iterator<Item = (Range<u32>, Option<[f32; 4]>)> + '_ {
        let mut at = vertices.start;
        let mut clips = self.scissors[self.scissors.partition_point(|(r, _)| r.end <= at)..]
            .iter()
            .peekable();
        std::iter::from_fn(move || {
            if at >= vertices.end {
                return None;
            }
            let (end, clip) = match clips.peek() {
                Some((range, _)) if range.start > at => (range.start, None),
                Some(_) => {
                    let (range, clip) = clips.next().unwrap();
                    (range.end, Some(*clip))
                }
                None => (vertices.end, None),
            };
            let end = end.min(vertices.end);
            let range = at..end;
            at = end;
            Some((range, clip))
        })
    }

    /// Each six-vertex quad's box `[x0, y0, x1, y1]` with the scissor the rasterizer gates it
    /// by, if any.
    pub fn quad_scissors(&self) -> impl Iterator<Item = ([f32; 4], Option<[f32; 4]>)> + '_ {
        let quads = self.vertices.as_chunks::<6>().0;
        quads.iter().enumerate().map(|(i, quad)| {
            let first = (i * 6) as u32;
            let clip = self.scissors[self.scissors.partition_point(|(r, _)| r.end <= first)..]
                .first()
                .filter(|(range, _)| range.start <= first)
                .map(|(_, clip)| *clip);
            let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for v in quad {
                bounds = [
                    bounds[0].min(v.pos[0]),
                    bounds[1].min(v.pos[1]),
                    bounds[2].max(v.pos[0]),
                    bounds[3].max(v.pos[1]),
                ];
            }
            (bounds, clip)
        })
    }

    /// The part of each quad's box its scissor lets through, `None` where it hides the quad.
    pub fn visible_quads(&self) -> impl Iterator<Item = Option<[f32; 4]>> + '_ {
        self.quad_scissors().map(|(bounds, clip)| {
            let [l, t, r, b] = clip.unwrap_or(bounds);
            let cut = [
                bounds[0].max(l),
                bounds[1].max(t),
                bounds[2].min(r),
                bounds[3].min(b),
            ];
            (cut[0] < cut[2] && cut[1] < cut[3]).then_some(cut)
        })
    }

    /// Discard appended vertices and their scissor state.
    pub fn truncate_vertices(&mut self, len: usize) {
        self.vertices.truncate(len);
        let len = len as u32;
        let first = self
            .scissors
            .partition_point(|(range, _)| range.start < len);
        self.scissors.truncate(first);
        if let Some((range, _)) = self.scissors.last_mut() {
            range.end = range.end.min(len);
        }
    }
    /// `blit` of the top-left `w x h` texels of a `tw x th` texture.
    pub fn sprite_region(
        &mut self,
        atlas: &HudAtlas,
        name: &str,
        [x, y, w, h]: [f32; 4],
        tw: f32,
        th: f32,
    ) {
        if let Some(mut r) = atlas.sprites.get(name).copied() {
            r.u1 = r.u0 + (r.u1 - r.u0) * w / tw;
            r.v1 = r.v0 + (r.v1 - r.v0) * h / th;
            self.rect_uv(x, y, w, h, r, u32::MAX);
        }
    }
    pub fn fill(&mut self, atlas: &HudAtlas, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let mut r = atlas.white;
        let (cu, cv) = ((r.u0 + r.u1) / 2.0, (r.v0 + r.v1) / 2.0);
        r.u0 = cu;
        r.u1 = cu;
        r.v0 = cv;
        r.v1 = cv;
        self.rect_uv(x, y, w, h, r, color);
    }
    /// Any four-cornered shape in one colour, corners in order round it.
    pub fn fill_quad(&mut self, atlas: &HudAtlas, corners: [[f32; 2]; 4], color: u32) {
        let w = atlas.white;
        let uv = [(w.u0 + w.u1) / 2.0, (w.v0 + w.v1) / 2.0];
        self.quad(corners, [uv; 4], color);
    }
    /// GuiGraphics.fillGradient: `top` to `bottom`, down the rectangle.
    pub fn fill_gradient(
        &mut self,
        atlas: &HudAtlas,
        (x, y, w, h): (f32, f32, f32, f32),
        top: u32,
        bottom: u32,
    ) {
        let r = atlas.white;
        let uv = [(r.u0 + r.u1) / 2.0, (r.v0 + r.v1) / 2.0];
        let p = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        let c = [top, top, bottom, bottom];
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: p[i],
                uv,
                color: c[i],
                depth: 0.0,
                glint_uv: [0.0, 0.0],
                light: 0,
                text: 0,
            });
        }
    }

    pub(super) fn draw_inline_object(
        &mut self,
        atlas: &HudAtlas,
        object: &crate::text::InlineObject,
        x: f32,
        y: f32,
        tint: u32,
    ) {
        use crate::text::InlineObject;
        match object {
            InlineObject::Player { profile, hat, .. } => {
                let id = crate::skins::parse_profile(profile).id;
                let name = inline_profile_name(profile);
                let by_id = atlas.object_skins.get(&format!("id:{id:032x}"));
                let by_name = name.and_then(|name| {
                    atlas
                        .object_skins
                        .get(&format!("name:{}", name.to_ascii_lowercase()))
                });
                let resolved = if profile.get("id").is_some() {
                    by_id.or(by_name)
                } else {
                    by_name.or(by_id)
                };
                let default = crate::tab_list::default_skin(id);
                let skin = resolved
                    .filter(|path| atlas.sprites.contains_key(*path))
                    .map_or(default, String::as_str);
                let Some(rect) = atlas.sprites.get(skin).copied() else {
                    return;
                };
                for u in [8.0, 40.0].into_iter().take(if *hat { 2 } else { 1 }) {
                    let du = (rect.u1 - rect.u0) / 64.0;
                    let dv = (rect.v1 - rect.v0) / 64.0;
                    self.rect_uv(
                        x,
                        y,
                        8.0,
                        8.0,
                        Rect {
                            u0: rect.u0 + du * u,
                            v0: rect.v0 + dv * 8.0,
                            u1: rect.u0 + du * (u + 8.0),
                            v1: rect.v0 + dv * 16.0,
                            w: 8,
                            h: 8,
                        },
                        tint,
                    );
                }
            }
            InlineObject::Dynamic { handle, size, .. } => {
                let side = crate::font::INLINE_OBJECT_SIZE;
                self.dynamic_texture(
                    *handle,
                    *size,
                    [x, y, side, side],
                    [0.0, 0.0],
                    [[1, 1], [1, 1]],
                    tint,
                );
            }
            InlineObject::Atlas { .. } => {
                if let Some(rect) = atlas.inline_object_sprite(object) {
                    let size = crate::font::INLINE_OBJECT_SIZE;
                    let start = self.vertices.len();
                    self.rect_uv(x, y, size, size, rect, tint);
                    if let crate::text::InlineObject::Atlas {
                        atlas: id, sprite, ..
                    } = object
                        && id.strip_prefix("minecraft:").unwrap_or(id) == "gui"
                    {
                        self.bind_gui_atlas(start, atlas, sprite);
                    }
                }
            }
        }
    }

    /// SubmitNodeCollection.submitNameTag's text in its own space (x right,
    /// y down, in GUI pixels): centred at `-Font.width / 2`, drawn as
    /// Font.prepareText draws any text (styles, fonts and glyph boxes; no
    /// shadow), over `background` (RGBA) unless its alpha is 0. Returns how
    /// many vertices at the start of the batch are the background.
    pub fn name_tag(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        y: f32,
        color: u32,
        background: u32,
    ) -> usize {
        let x = -atlas.font.width_text(text) / 2.0;
        let start = self.vertices.len();
        let advance = self.text_styled(atlas, text, x, y, color, false);
        if background >> 24 == 0 {
            return 0;
        }
        // PreparedTextBuilder.markBackground: one pixel round the advances,
        // from a pixel above the line to 9 below its top.
        let glyphs = self.vertices.split_off(start);
        self.fill(atlas, x - 1.0, y - 1.0, advance + 1.0, 10.0, background);
        let behind = self.vertices.len() - start;
        self.vertices.extend(glyphs);
        behind
    }

    /// `[x, y, width, height]` of the geometry appended since `start`, in GUI pixels,
    /// after clipping to the viewport and the scissor of each draw run.
    pub fn visible_vertex_bounds(&self, start: usize) -> Option<[f32; 4]> {
        if self.viewport.contains(&0) || start >= self.vertices.len() {
            return None;
        }
        let mut bounds = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        let mut include = |[x, y]: [f32; 2]| {
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].min(y);
            bounds[2] = bounds[2].max(x);
            bounds[3] = bounds[3].max(y);
        };
        for (range, blend) in self.runs_in(start as u32..self.vertices.len() as u32) {
            let [x, y, w, h] = if let Some(clip) = blend.scissor() {
                let Some(scissor) = hud_scissor(clip, self.viewport, 1.0) else {
                    continue;
                };
                scissor
            } else {
                [0, 0, self.viewport[0], self.viewport[1]]
            };
            let [x0, y0, x1, y1] = [x, y, x + w, y + h].map(|v| v as f32 / self.scale);
            let vertices = &self.vertices[range.start as usize..range.end as usize];
            for triangle in vertices.as_chunks::<3>().0 {
                if triangle.iter().flat_map(|v| v.pos).any(|v| !v.is_finite())
                    || triangle_area(triangle) == 0.0
                {
                    continue;
                }
                if triangle.iter().all(|v| {
                    let [x, y] = v.pos;
                    x >= x0 && x <= x1 && y >= y0 && y <= y1
                }) {
                    triangle.iter().for_each(|v| include(v.pos));
                } else {
                    let poly = clip_triangle(triangle, [x0, y0, x1, y1]);
                    if poly.len() >= 3 {
                        // A polygon left on a clipping edge has no area. It must not
                        // extend the bounds of neighbouring, actually drawn triangles.
                        let (mut lo, mut hi) = (poly[0].pos, poly[0].pos);
                        for v in &poly[1..] {
                            for axis in 0..2 {
                                lo[axis] = lo[axis].min(v.pos[axis]);
                                hi[axis] = hi[axis].max(v.pos[axis]);
                            }
                        }
                        if lo[0] < hi[0] && lo[1] < hi[1] {
                            include(lo);
                            include(hi);
                        }
                    }
                }
            }
        }
        let [x0, y0, x1, y1] = bounds;
        (x0 < x1 && y0 < y1).then_some([x0, y0, x1 - x0, y1 - y0])
    }
}

fn inline_profile_name(profile: &serde_json::Value) -> Option<&str> {
    profile
        .as_str()
        .or_else(|| profile.get("name").and_then(serde_json::Value::as_str))
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct HudUniform {
    size: [f32; 4],
    /// `EndPortalFill`: the two sprites, then the game time in x.
    end_portal: [[f32; 4]; 3],
}
impl HudUniform {
    pub(super) fn new(size: [f32; 4], fill: EndPortalFill, glint_colour: u32) -> Self {
        let [r, g, b, _] = glint_colour.to_le_bytes().map(|c| f32::from(c) / 255.0);
        let tint = if glint_colour == 0 {
            [-1.0, 0.0, 0.0]
        } else {
            [r, g, b]
        };
        Self {
            size,
            end_portal: [
                fill.sprites[0],
                fill.sprites[1],
                [fill.game_time, tint[0], tint[1], tint[2]],
            ],
        }
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;

    #[test]
    fn visible_vertex_bounds_tracks_drawn_slice_and_ignores_empty_geometry() {
        let mut batch = HudBatch::scratch([320, 240]);
        let uv = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        };
        assert_eq!(batch.visible_vertex_bounds(0), None);
        batch.rect_uv(10.0, 20.0, 30.0, 40.0, uv, u32::MAX);
        assert_eq!(
            batch.visible_vertex_bounds(0),
            Some([10.0, 20.0, 30.0, 40.0])
        );

        let layer_start = batch.vertices.len();
        assert_eq!(batch.visible_vertex_bounds(layer_start), None);
        batch.rect_uv(295.0, 1.0, 24.0, 24.0, uv, u32::MAX);
        batch.rect_uv(270.0, 26.0, 24.0, 24.0, uv, u32::MAX);
        assert_eq!(
            batch.visible_vertex_bounds(layer_start),
            Some([270.0, 1.0, 49.0, 49.0])
        );

        let degenerate = batch.vertices.len();
        batch.rect_uv(5.0, 5.0, 0.0, 10.0, uv, u32::MAX);
        assert_eq!(batch.visible_vertex_bounds(degenerate), None);
    }

    #[test]
    fn visible_vertex_bounds_clips_preview_triangles_per_run_and_uses_the_physical_viewport() {
        let mut batch = HudBatch::new(
            [641, 481],
            GuiScale {
                requested: 2,
                unicode: false,
            },
        );
        // Same triangle in two model preview runs. The second scissor overlaps the
        // triangle's axis-aligned box, but is wholly beyond the diagonal edge x+y=40.
        let preview = [
            [0.0, 0.0, 0.0],
            [40.0, 0.0, 0.0],
            [0.0, 40.0, 1.0],
            [0.0, 40.0, 1.0],
        ];
        batch.quad_3d(preview, [[0.0; 2]; 4], u32::MAX);
        batch.depth_since(0, [10.25, 10.0, 22.5, 40.0]);
        let hidden = batch.vertices.len();
        batch.quad_3d(preview, [[0.0; 2]; 4], u32::MAX);
        batch.depth_since(hidden, [30.0, 30.0, 80.0, 80.0]);
        assert_eq!(batch.visible_vertex_bounds(hidden), None);
        assert_eq!(
            batch.visible_vertex_bounds(0),
            Some([10.0, 10.0, 12.5, 20.0])
        );

        let ordinary = batch.vertices.len();
        batch.quad(
            [[319.0, 1.0], [321.0, 1.0], [321.0, 3.0], [319.0, 3.0]],
            [[0.0; 2]; 4],
            u32::MAX,
        );
        assert_eq!(
            batch.visible_vertex_bounds(ordinary),
            Some([319.0, 1.0, 1.5, 2.0])
        );
    }
}
