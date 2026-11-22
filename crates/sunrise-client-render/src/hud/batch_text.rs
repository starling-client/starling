use super::*;

impl HudBatch {
    pub fn text_width(atlas: &HudAtlas, text: &str) -> f32 {
        atlas.font.width(text)
    }
    /// Draw plain text (legacy `§` codes are honoured); returns the advance.
    pub fn text(
        &mut self,
        atlas: &HudAtlas,
        text: &str,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        let t = if text.contains('§') {
            crate::text::Text::legacy(text)
        } else {
            crate::text::Text::literal(text)
        };
        self.text_styled(atlas, &t, x, y, color, shadow)
    }
    /// Font.drawInternal: styled glyphs, shadows first, with bold, italic,
    /// underline and strikethrough. `color` is the default colour and alpha.
    pub fn text_styled(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        self.styled_text(atlas, text, x, y, color, shadow, false, None)
    }
    /// `text_styled` for text drawn in the world (`TextRenderable.render`
    /// with `flat` false): each quad's `depth` is its z in text units, the
    /// glyphs `Font.SHADOW_DEPTH` in front of their shadow, a bold copy 0.001
    /// in front of its glyph and the lines 0.01 (`BakedSheetGlyph`), so they
    /// do not fight over the same pixels.
    pub fn text_styled_world(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        self.styled_text(atlas, text, x, y, color, shadow, true, None)
    }
    /// `Font.drawInBatch8xOutline`: eight glyph offsets in the outline color,
    /// then the original text. Returns (advance, first front-text vertex), so
    /// world rendering can use POLYGON_OFFSET for the front and NORMAL for
    /// its outline. Offsets use each glyph's own shadow offset (unihex: 0.5).
    pub fn text_styled_outline_world(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        outline: u32,
    ) -> (f32, usize) {
        for dx in -1..=1 {
            for dy in -1..=1 {
                if dx != 0 || dy != 0 {
                    self.styled_text(
                        atlas,
                        text,
                        x,
                        y,
                        color,
                        false,
                        true,
                        Some((outline, [dx as f32, dy as f32])),
                    );
                }
            }
        }
        let front = self.vertices.len();
        (
            self.styled_text(atlas, text, x, y, color, false, true, None),
            front,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn styled_text(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
        world: bool,
        outline: Option<(u32, [f32; 2])>,
    ) -> f32 {
        // StringDecomposer.iterateFormatted inside Font.drawInternal:
        // `§` codes inside a span restyle what follows and are not drawn.
        let text = text.resolved();
        // Language.getVisualOrder: shaped and reordered for right-to-left text.
        let text = crate::bidi::visual(&text);
        let font = &atlas.font;
        let alpha = (color >> 24) as u8;
        let mut glyphs = Vec::new();
        let mut cursor = x;
        let mut previous = None;
        for (i, (ch, style)) in text.chars().enumerate() {
            let object = (ch == '\u{fffc}')
                .then_some(style.inline_object.as_ref())
                .flatten();
            if let Some(crate::text::InlineObject::Player { profile, .. }) = object
                && self.inline_profiles.len() < 256
                && !self.inline_profiles.contains(profile)
            {
                self.inline_profiles.push(profile.clone());
            }
            // Font.PreparedTextBuilder.accept: lines start 1 left of a
            // sequence's first character. Language.getVisualOrder makes a
            // sequence of each run of one style, which world text has.
            let first = i == 0 || world && previous != Some(style);
            previous = Some(style);
            // The style's font (`Style.getFont`), the default one unless set; a hand-written
            // skin's take of the letter by where it stands.
            let mut g = if object.is_some() {
                crate::font::Glyph {
                    rect: None,
                    left: 0.0,
                    top: 0.0,
                    right: 0.0,
                    bottom: crate::font::INLINE_OBJECT_SIZE,
                    advance: crate::font::INLINE_OBJECT_SIZE,
                    offset: 1.0,
                }
            } else {
                font.glyph_varied(ch, style, super::batch::take_seed(x, y, i))
            };
            // Font.StringRenderOutput.accept: obfuscated characters draw a
            // random glyph of the same width each frame.
            if object.is_none() && style.obfuscated == Some(true) && ch != ' ' {
                g = font.random_glyph(style, g.advance);
            }
            let c = style.color.map_or(color, |rgb| {
                rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, alpha)
            });
            let bold = object.is_none() && style.is_bold();
            let advance = g.advance + if bold { g.offset } else { 0.0 };
            glyphs.push((
                cursor,
                g,
                c,
                style.shadow_color,
                bold,
                style.is_italic(),
                style.underlined == Some(true),
                style.strikethrough == Some(true),
                first,
                advance,
                object.cloned(),
            ));
            cursor += advance;
        }
        self.patches.extend(font.take_patches());
        let white = font.white();
        let has_shadow = outline.is_none()
            && (shadow
                || glyphs
                    .iter()
                    .any(|glyph| glyph.3.is_some_and(|color| color != 0)));
        let passes: &[bool] = if has_shadow { &[true, false] } else { &[false] };
        // ARGB.scaleRGB(color, 0.25): the default shadow colour.
        let shade = |c: u32| {
            let b = c.to_le_bytes();
            rgba(
                (f32::from(b[0]) * 0.25) as u8,
                (f32::from(b[1]) * 0.25) as u8,
                (f32::from(b[2]) * 0.25) as u8,
                b[3],
            )
        };
        for &is_shadow in passes {
            for (
                gx,
                g,
                c,
                custom_shadow,
                bold,
                italic,
                underline,
                strike,
                first,
                advance,
                object,
            ) in &glyphs
            {
                let shadow_color = if let Some(argb) = custom_shadow {
                    let text_alpha = (c >> 24) as u8 as u16;
                    let shadow_alpha = (argb >> 24) as u8 as u16;
                    let alpha = (shadow_alpha * text_alpha / 255) as u8;
                    rgba((argb >> 16) as u8, (argb >> 8) as u8, *argb as u8, alpha)
                } else {
                    shade(*c)
                };
                let has_shadow = outline.is_none()
                    && (custom_shadow.is_some_and(|argb| argb != 0)
                        || custom_shadow.is_none() && shadow)
                    && !(world && shadow_color == 0);
                if is_shadow && !has_shadow {
                    continue;
                }
                let (dx, col) = if is_shadow {
                    (g.offset, shadow_color)
                } else {
                    (0.0, *c)
                };
                let (px, py, col) = if let Some((color, [dx, dy])) = outline {
                    (*gx + dx * g.offset, y + dy * g.offset, color)
                } else {
                    (*gx + dx, y + dx, col)
                };
                if let Some(object) = object {
                    // `PlainTextRenderable.render`: sprite glyphs use the same
                    // shadow colour and offset as text, with the front at SHADOW_DEPTH.
                    let start = self.vertices.len();
                    self.draw_inline_object(atlas, object, px, py - 1.0, col);
                    if world && has_shadow && !is_shadow {
                        for v in &mut self.vertices[start..] {
                            v.depth = 0.03;
                        }
                    }
                    continue;
                }
                let shear = |v: f32| if *italic { 1.0 - 0.25 * v } else { 0.0 };
                let thick = if *bold { 0.1 } else { 0.0 };
                let draw = |batch: &mut Self, ox: f32| match g.rect {
                    Some(r) => {
                        let (x0, x1, y0, y1) = (
                            px + ox + g.left,
                            px + ox + g.right,
                            py + g.top,
                            py + g.bottom,
                        );
                        batch.quad(
                            [
                                [x0 + shear(g.top) - thick, y0 - thick],
                                [x1 + shear(g.top) + thick, y0 - thick],
                                [x1 + shear(g.bottom) + thick, y1 + thick],
                                [x0 + shear(g.bottom) - thick, y1 + thick],
                            ],
                            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
                            col,
                        );
                    }
                    None if g.right > 0.0 => {
                        // SpecialGlyphs.MISSING: a 5x8 outline.
                        let (bx, by) = (px + ox, py);
                        for (fx, fy, fw, fh) in [
                            (0.0, 0.0, 5.0, 1.0),
                            (0.0, 7.0, 5.0, 1.0),
                            (0.0, 1.0, 1.0, 6.0),
                            (4.0, 1.0, 1.0, 6.0),
                        ] {
                            batch.rect_uv(bx + fx, by + fy, fw, fh, white, col);
                        }
                    }
                    None => {}
                };
                let base = if has_shadow && !is_shadow { 0.03 } else { 0.0 };
                let lift = |batch: &mut Self, start: usize, z: f32| {
                    if world {
                        for v in &mut batch.vertices[start..] {
                            v.depth = z;
                        }
                    }
                };
                let glyph = self.vertices.len();
                let start = self.vertices.len();
                draw(self, 0.0);
                lift(self, start, base);
                if *bold {
                    let start = self.vertices.len();
                    draw(self, g.offset);
                    lift(self, start, base + 0.001);
                }
                let x0 = if *first { px - 1.0 } else { px };
                let start = self.vertices.len();
                if *strike {
                    self.rect_uv(x0, py + 3.5, px + *advance - x0, 1.0, white, col);
                }
                if *underline {
                    self.rect_uv(x0, py + 8.0, px + *advance - x0, 1.0, white, col);
                }
                lift(self, start, base + 0.01);
                if !world {
                    for v in &mut self.vertices[glyph..] {
                        v.text = 1;
                    }
                }
            }
        }
        cursor - x
    }
}
