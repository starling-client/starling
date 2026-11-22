use super::*;

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
    /// 1 for the GUI's text (`RenderPipelines.GUI_TEXT`), which a resource
    /// pack's text shader draws when it has one.
    pub text: u32,
}
/// Sutherland-Hodgman against one half plane, interpolating every attribute.
fn clip_polygon(poly: &[HudVertex], axis: usize, limit: f32, keep_below: bool) -> Vec<HudVertex> {
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
        v.color = crate::hud_kit::shapes::mix_colour(a.color, b.color, t);
        v.glint_uv = [
            mix(a.glint_uv[0], b.glint_uv[0]),
            mix(a.glint_uv[1], b.glint_uv[1]),
        ];
        v.pos[axis] = limit;
        v
    };
    let mut out = Vec::with_capacity(poly.len() + 2);
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
/// Which take of a letter a hand-written font draws at the `i`th character of text at `x, y`:
/// the same each frame for text that stays put.
pub(super) fn take_seed(x: f32, y: f32, i: usize) -> u32 {
    let mut h = (x.round() as i32 as u32).wrapping_mul(0x9e37_79b9)
        ^ (y.round() as i32 as u32).wrapping_mul(0x85eb_ca6b)
        ^ (i as u32).wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h.wrapping_mul(0x2c1b_3c6d) >> 7
}
/// Scale inputs for one view, copied from its owner's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuiScale {
    pub requested: u32,
    pub unicode: bool,
}
/// Starling: the scale inputs of the last batch laid out (`HudBatch::new`), the requested scale in
/// the low bits and Force Unicode in the top one, for [`HudBatch::new_old`].
static LAST_GUI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// One frame of overlay geometry in GUI pixels.
pub struct HudBatch {
    pub pictures: Vec<Arc<pictures::Picture>>,
    pub vertices: Vec<HudVertex>,
    pub scale: f32,
    pub width: f32,
    pub height: f32,
    /// Glyph pixels to upload into the atlas before drawing.
    pub patches: Vec<crate::font::Patch>,
    /// Client mods' dynamic textures to upload into their atlas slots before drawing, sharing
    /// the pixels the game keeps of them.
    pub image_patches: Vec<dynamic_textures::ImagePatch>,
    /// Vertex ranges with another blend or depth/scissor state, in order.
    pub blends: Vec<(std::ops::Range<u32>, Blend)>,
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
/// The paint appended by one retained callback, excluding any painters it delegates to.
pub struct HudPaintCheckpoint {
    start: usize,
    pictures: usize,
    profiles: usize,
    own_start: usize,
    own_ranges: SmallVec<[std::ops::Range<usize>; 2]>,
    delegated: bool,
}

impl HudPaintCheckpoint {
    /// Exclude paint appended by a completed delegation.
    pub fn preserve_delegated(&mut self, range: std::ops::Range<usize>) {
        if self.own_start != range.start {
            self.own_ranges.push(self.own_start..range.start);
        }
        self.own_start = range.end;
        self.delegated = true;
    }

    pub fn discard(self, batch: &mut HudBatch) {
        // Remove from back to front so earlier ranges still refer to the original indices.
        for range in std::iter::once(self.own_start..batch.vertices.len())
            .chain(self.own_ranges.into_iter().rev())
            .filter(|range| !range.is_empty())
        {
            let start = range.start as u32;
            let len = (range.end - range.start) as u32;
            let move_endpoint = |at: u32| at - at.saturating_sub(start).min(len);
            drop(batch.vertices.drain(range));
            let first = batch.blends.partition_point(|(run, _)| run.end <= start);
            for (run, _) in &mut batch.blends[first..] {
                *run = move_endpoint(run.start)..move_endpoint(run.end);
            }
            batch.blends.retain(|(run, _)| !run.is_empty());
        }
        // Glyph atlas uploads are reusable by other painters. A delegated draw can also
        // share a picture or inline skin first seen by the wrapper. Picture references
        // can be checked against surviving blend runs; the inline skin cache deduplicates
        // across the frame and does not carry a per-vertex reference.
        if !self.delegated {
            batch.pictures.truncate(self.pictures);
            batch.inline_profiles.truncate(self.profiles);
        } else {
            let first = batch
                .blends
                .partition_point(|(run, _)| run.end <= self.start as u32);
            let surviving = &batch.blends[first..];
            let mut i = 0;
            batch.pictures.retain(|picture| {
                let keep = i < self.pictures
                    || surviving
                        .iter()
                        .any(|(_, blend)| *blend == Blend::Picture(picture.id));
                i += 1;
                keep
            });
        }
    }
}

/// How a run of HUD vertices blends and tests depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Blend {
    Picture(u64),
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
        Self::with_gui_size(gui)
    }
    /// Starling: the one-argument constructor native mods made for Sunrise before each view had
    /// its own scale settings, laid out with the last batch's (`HudBatch::new`); auto before any.
    /// `native_mod_adapt.py` points their `HudBatch::new(size)` here.
    pub fn new_old(window: [u32; 2]) -> Self {
        let bits = LAST_GUI_SCALE.load(std::sync::atomic::Ordering::Relaxed);
        Self::new(
            window,
            GuiScale {
                requested: bits & 0x7fff_ffff,
                unicode: bits >> 31 == 1,
            },
        )
    }
    /// An offscreen or test batch, independent of the active view's settings.
    pub fn scratch(window: [u32; 2]) -> Self {
        Self::with_gui_size(sunrise_client_input::window::GuiSize::new(window, 0, false))
    }
    fn with_gui_size(gui: sunrise_client_input::window::GuiSize) -> Self {
        Self {
            pictures: Vec::new(),
            vertices: Vec::new(),
            scale: gui.scale as f32,
            width: gui.size[0] as f32,
            height: gui.size[1] as f32,
            patches: Vec::new(),
            image_patches: Vec::new(),
            blends: Vec::new(),
            end_portal: EndPortalFill::default(),
            glint_alpha: 0.75,
            glint_colour: 0,
            game_time: 0.0,
            blur_at: 0,
            inline_profiles: Vec::new(),
        }
    }
    /// Start tracking one callback's writes to this batch without copying existing paint.
    pub fn paint_checkpoint(&self) -> HudPaintCheckpoint {
        let start = self.vertices.len();
        HudPaintCheckpoint {
            start,
            pictures: self.pictures.len(),
            profiles: self.inline_profiles.len(),
            own_start: start,
            own_ranges: SmallVec::new(),
            delegated: false,
        }
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
    ) -> SmallVec<[(std::ops::Range<u32>, Blend); 16]> {
        let mut runs = SmallVec::new();
        let mut at = vertices.start;
        for (range, blend) in &self.blends {
            if range.start > at {
                let start = at.max(vertices.start);
                let end = range.start.min(vertices.end);
                if start < end {
                    runs.push((start..end, Blend::Alpha));
                }
            }
            let start = range.start.max(vertices.start);
            let end = range.end.min(vertices.end);
            if start < end {
                runs.push((start..end, *blend));
            }
            at = at.max(range.end);
        }
        let end = (self.vertices.len() as u32).min(vertices.end);
        if at < end {
            runs.push((at.max(vertices.start)..end, Blend::Alpha));
        }
        runs
    }
    /// `AbstractContainerScreen.init`: center in integer GUI coordinates.
    pub fn centered_origin(&self, width: f32, height: f32) -> (f32, f32) {
        (
            ((self.width - width) / 2.0).trunc(),
            ((self.height - height) / 2.0).trunc(),
        )
    }
    pub fn quad(&mut self, p: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: u32) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: p[i],
                uv: uv[i],
                color,
                depth: 0.0,
                glint_uv: [0.0, 0.0],
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
                text: 0,
            });
        }
    }
    pub fn quad_3d(&mut self, p: [[f32; 3]; 4], uv: [[f32; 2]; 4], color: u32) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.vertices.push(HudVertex {
                pos: [p[i][0], p[i][1]],
                uv: uv[i],
                color,
                depth: p[i][2],
                glint_uv: [0.0, 0.0],
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
            self.rect_uv(x, y, r.w as f32, r.h as f32, r, u32::MAX);
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
            self.rect_uv(x, y, w, h, r, color);
        }
    }
    /// Draw part of a sprite, `fraction` of its width from the left.
    pub fn sprite_part(&mut self, atlas: &HudAtlas, name: &str, x: f32, y: f32, fraction: f32) {
        if let Some(mut r) = atlas.gui_draw_rect(name).copied() {
            let w = r.w as f32 * fraction;
            r.u1 = r.u0 + (r.u1 - r.u0) * fraction;
            self.rect_uv(x, y, w, r.h as f32, r, u32::MAX);
        }
    }
    /// Clip the quads drawn since `start` to `[x0, x1) x [y0, y1)` (the
    /// GUI scissor): axis-aligned quads are cut with their UVs, others (item
    /// triangles) are clipped as polygons.
    /// Scale the vertices pushed since `start` about `(cx, cy)` (a GUI pose
    /// matrix of translate, scale, translate back).
    pub fn scale_since(&mut self, start: usize, cx: f32, cy: f32, sx: f32, sy: f32) {
        for v in &mut self.vertices[start..] {
            v.pos[0] = cx + (v.pos[0] - cx) * sx;
            v.pos[1] = cy + (v.pos[1] - cy) * sy;
        }
    }
    pub fn clip_since(&mut self, start: usize, x0: f32, y0: f32, x1: f32, y1: f32) {
        let quads: Vec<[HudVertex; 6]> = self.vertices[start..].as_chunks::<6>().0.to_vec();
        self.vertices.truncate(start);
        // Where each source quad now begins, to move the blend ranges (foil
        // items are an additive run) along with their vertices.
        let mut moved = Vec::with_capacity(quads.len() + 1);
        for q in quads {
            moved.push(self.vertices.len() as u32);
            let corners = [q[0], q[1], q[2], q[5]];
            let (mut x_lo, mut x_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            let (mut y_lo, mut y_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for v in &corners {
                x_lo = x_lo.min(v.pos[0]);
                x_hi = x_hi.max(v.pos[0]);
                y_lo = y_lo.min(v.pos[1]);
                y_hi = y_hi.max(v.pos[1]);
            }
            // An axis-aligned rectangle, its corners in any order.
            let find = |x: f32, y: f32| corners.iter().find(|v| v.pos == [x, y]).copied();
            let rect = (
                find(x_lo, y_lo),
                find(x_hi, y_lo),
                find(x_hi, y_hi),
                find(x_lo, y_hi),
            );
            let (Some(tl), Some(tr), Some(br), Some(bl)) = rect else {
                // Item geometry arrives as triangles: clip each against the
                // rectangle and re-emit the fan, so a triangle straddling the
                // edge keeps its visible part instead of vanishing whole.
                for tri in [[q[0], q[1], q[2]], [q[3], q[4], q[5]]] {
                    let area = (tri[1].pos[0] - tri[0].pos[0]) * (tri[2].pos[1] - tri[0].pos[1])
                        - (tri[2].pos[0] - tri[0].pos[0]) * (tri[1].pos[1] - tri[0].pos[1]);
                    if area == 0.0 {
                        continue;
                    }
                    let mut poly = tri.to_vec();
                    for (axis, limit, keep_below) in
                        [(0, x0, false), (0, x1, true), (1, y0, false), (1, y1, true)]
                    {
                        poly = clip_polygon(&poly, axis, limit, keep_below);
                    }
                    for i in 1..poly.len().saturating_sub(1) {
                        // A triangle as the quad `a, b, c, c`.
                        let (a, b, c) = (poly[0], poly[i], poly[i + 1]);
                        self.vertices.extend_from_slice(&[a, b, c, a, c, c]);
                    }
                }
                continue;
            };
            let (cx0, cy0) = (x_lo.max(x0), y_lo.max(y0));
            let (cx1, cy1) = (x_hi.min(x1), y_hi.min(y1));
            if cx0 >= cx1 || cy0 >= cy1 {
                continue;
            }
            let t = |p: f32, lo: f32, hi: f32| if hi == lo { 0.0 } else { (p - lo) / (hi - lo) };
            let corner = |x, y| {
                clip_corner::clip_corner(
                    [tl, tr, br, bl],
                    [x, y],
                    [t(x, x_lo, x_hi), t(y, y_lo, y_hi)],
                )
            };
            let c = [
                corner(cx0, cy0),
                corner(cx1, cy0),
                corner(cx1, cy1),
                corner(cx0, cy1),
            ];
            self.vertices.extend([0, 1, 2, 0, 2, 3].map(|i| c[i]));
        }
        moved.push(self.vertices.len() as u32);
        let at = |v: u32| moved[((v as usize).saturating_sub(start) / 6).min(moved.len() - 1)];
        for (range, _) in &mut self.blends {
            if range.start as usize >= start {
                *range = at(range.start)..at(range.end);
            }
        }
        self.blends.retain(|(range, _)| !range.is_empty());
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
            InlineObject::Atlas { .. } => {
                if let Some(rect) = atlas.inline_object_sprite(object) {
                    let size = crate::font::INLINE_OBJECT_SIZE;
                    self.rect_uv(x, y, size, size, rect, tint);
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
