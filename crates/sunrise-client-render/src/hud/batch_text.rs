use super::*;

#[derive(Clone, Copy)]
struct StyledTextOptions<'a> {
    color: u32,
    shadow: bool,
    world: bool,
    outline: Option<(u32, [f32; 2])>,
    default_font: &'a str,
}

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
        if text.contains('§') {
            let text = crate::text::Text::legacy(text);
            return self.text_styled(atlas, &text, x, y, color, shadow);
        }
        let visual = crate::bidi::visual_str(text);
        let style = crate::text::Style::default();
        self.paint_visual_text(
            atlas,
            visual.chars().map(|c| (c, &style)),
            x,
            y,
            color,
            shadow,
            false,
            None,
            crate::font::DEFAULT,
        )
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
        self.styled_text(
            atlas,
            text,
            [x, y],
            StyledTextOptions {
                color,
                shadow,
                world: false,
                outline: None,
                default_font: crate::font::DEFAULT,
            },
        )
    }
    /// `text_styled` using `default_font` for spans without an explicit style font.
    pub fn text_styled_with_font(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        position: [f32; 2],
        color: u32,
        shadow: bool,
        default_font: &str,
    ) -> f32 {
        self.styled_text(
            atlas,
            text,
            position,
            StyledTextOptions {
                color,
                shadow,
                world: false,
                outline: None,
                default_font,
            },
        )
    }
    /// Draw a GUI line already resolved and put in visual order.
    pub fn text_visual(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    ) -> f32 {
        self.paint_visual_text(
            atlas,
            text.chars(),
            x,
            y,
            color,
            shadow,
            false,
            None,
            crate::font::DEFAULT,
        )
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
        self.styled_text(
            atlas,
            text,
            [x, y],
            StyledTextOptions {
                color,
                shadow,
                world: true,
                outline: None,
                default_font: crate::font::DEFAULT,
            },
        )
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
                        [x, y],
                        StyledTextOptions {
                            color,
                            shadow: false,
                            world: true,
                            outline: Some((outline, [dx as f32, dy as f32])),
                            default_font: crate::font::DEFAULT,
                        },
                    );
                }
            }
        }
        let front = self.vertices.len();
        (
            self.styled_text(
                atlas,
                text,
                [x, y],
                StyledTextOptions {
                    color,
                    shadow: false,
                    world: true,
                    outline: None,
                    default_font: crate::font::DEFAULT,
                },
            ),
            front,
        )
    }
    fn styled_text(
        &mut self,
        atlas: &HudAtlas,
        text: &crate::text::Text,
        [x, y]: [f32; 2],
        StyledTextOptions {
            color,
            shadow,
            world,
            outline,
            default_font,
        }: StyledTextOptions<'_>,
    ) -> f32 {
        // StringDecomposer.iterateFormatted inside Font.drawInternal:
        // `§` codes inside a span restyle what follows and are not drawn.
        let text = text.resolved();
        // Language.getVisualOrder: shaped and reordered for right-to-left text.
        let text = crate::bidi::visual(&text);
        self.paint_visual_text(
            atlas,
            text.chars(),
            x,
            y,
            color,
            shadow,
            world,
            outline,
            default_font,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_visual_text<'a>(
        &mut self,
        atlas: &HudAtlas,
        characters: impl Iterator<Item = (char, &'a crate::text::Style)>,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
        world: bool,
        outline: Option<(u32, [f32; 2])>,
        default_font: &str,
    ) -> f32 {
        let font = &atlas.font;
        let alpha = (color >> 24) as u8;
        let mut glyphs = SmallVec::<[_; 10]>::new();
        let mut cursor = x;
        let mut previous = None;
        for (i, (ch, style)) in characters.enumerate() {
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
<<<<<<< HEAD
                font.glyph_varied(ch, style, super::batch::take_seed(x, y, i))
||||||| 46c00106def
                font.glyph_styled(ch, style)
=======
                font.glyph_styled_with_default(ch, style, default_font)
>>>>>>> ed6772e2118
            };
            // Font.StringRenderOutput.accept: obfuscated characters draw a
            // random glyph of the same width each frame.
            if object.is_none() && style.obfuscated == Some(true) && ch != ' ' {
                g = font.random_glyph_with_default(style, g.advance, default_font);
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
        let missing_rect = font.missing_rect();
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
        let mut effects = SmallVec::<[_; 4]>::new();
        // Font.PreparedTextBuilder.visit: emit every glyph completely (shadow, bold shadow,
        // foreground, bold foreground) before moving to the next glyph, then emit all
        // strikethrough and underline effects after all glyphs.
        for (gx, g, c, custom_shadow, bold, italic, underline, strike, first, advance, object) in
            &glyphs
        {
            let shadow_color = if let Some(argb) = custom_shadow {
                let text_alpha = (c >> 24) as u8 as u16;
                let shadow_alpha = (argb >> 24) as u8 as u16;
                let alpha = (shadow_alpha * text_alpha / 255) as u8;
                rgba((argb >> 16) as u8, (argb >> 8) as u8, *argb as u8, alpha)
            } else {
                shade(*c)
            };
            let glyph_has_shadow = outline.is_none()
                && (custom_shadow.is_some_and(|argb| argb != 0)
                    || custom_shadow.is_none() && shadow)
                && !(world && shadow_color == 0);
            let lift = |batch: &mut Self, start: usize, z: f32| {
                if world {
                    for v in &mut batch.vertices[start..] {
                        v.depth = z;
                    }
                }
            };
            if let Some(object) = object {
                // `PlainTextRenderable.render`: shadow copy then foreground copy for this glyph.
                if let Some((color, [dx, dy])) = outline {
                    self.draw_inline_object(
                        atlas,
                        object,
                        *gx + dx * g.offset,
                        y + dy * g.offset - 1.0,
                        color,
                    );
                } else {
                    if glyph_has_shadow {
                        self.draw_inline_object(
                            atlas,
                            object,
                            *gx + g.offset,
                            y + g.offset - 1.0,
                            shadow_color,
                        );
                    }
                    let start = self.vertices.len();
                    self.draw_inline_object(atlas, object, *gx, y - 1.0, *c);
                    if world && glyph_has_shadow {
                        for v in &mut self.vertices[start..] {
                            v.depth = 0.03;
                        }
                    }
                }
                continue;
            }
            let shear = |v: f32| if *italic { 1.0 - 0.25 * v } else { 0.0 };
            let thick = if *bold { 0.1 } else { 0.0 };
            let draw_pass = |batch: &mut Self, px: f32, py: f32, col: u32, base: f32| {
                let draw_glyph = |batch: &mut Self, ox: f32, r: Rect| {
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
                };
                let draw = |batch: &mut Self, ox: f32| match g.rect {
                    Some(r) => draw_glyph(batch, ox, r),
                    None if g.right > 0.0 => {
                        let r = missing_rect
                            .expect("missing sunrise/missing_glyph sprite in drawable HudAtlas");
                        draw_glyph(batch, ox, r);
                    }
                    None => {}
                };
                let glyph = batch.vertices.len();
                let start = batch.vertices.len();
                draw(batch, 0.0);
                lift(batch, start, base);
                if *bold {
                    let start = batch.vertices.len();
                    draw(batch, g.offset);
                    lift(batch, start, base + 0.001);
                }
                if !world {
                    for v in &mut batch.vertices[glyph..] {
                        v.text = 1;
                    }
                }
            };
            if let Some((color, [dx, dy])) = outline {
                draw_pass(self, *gx + dx * g.offset, y + dy * g.offset, color, 0.0);
            } else {
                if glyph_has_shadow {
                    draw_pass(self, *gx + g.offset, y + g.offset, shadow_color, 0.0);
                }
                let base = if glyph_has_shadow { 0.03 } else { 0.0 };
                draw_pass(self, *gx, y, *c, base);
            }
            if *strike || *underline {
                let x0 = if *first { *gx - 1.0 } else { *gx };
                let width = *gx + *advance - x0;
                if *strike {
                    effects.push((
                        x0,
                        y + 3.5,
                        width,
                        *c,
                        shadow_color,
                        glyph_has_shadow,
                        g.offset,
                    ));
                }
                if *underline {
                    effects.push((
                        x0,
                        y + 8.0,
                        width,
                        *c,
                        shadow_color,
                        glyph_has_shadow,
                        g.offset,
                    ));
                }
            }
        }
        // BakedSheetGlyph.renderEffect: after all glyphs, emit each decoration's shadow then
        // foreground with its own offset and depth (0.01 shadow, 0.04 foreground in world).
        for (x0, ey, width, col, shadow_col, effect_shadow, offset) in effects {
            let mark = self.vertices.len();
            if let Some((color, [dx, dy])) = outline {
                let start = self.vertices.len();
                self.rect_uv(x0 + dx * offset, ey + dy * offset, width, 1.0, white, color);
                if world {
                    for v in &mut self.vertices[start..] {
                        v.depth = 0.01;
                    }
                }
            } else {
                let mut depth = if world { 0.01 } else { 0.0 };
                if effect_shadow {
                    let start = self.vertices.len();
                    self.rect_uv(x0 + offset, ey + offset, width, 1.0, white, shadow_col);
                    if world {
                        for v in &mut self.vertices[start..] {
                            v.depth = depth;
                        }
                        depth += 0.03;
                    }
                }
                let start = self.vertices.len();
                self.rect_uv(x0, ey, width, 1.0, white, col);
                if world {
                    for v in &mut self.vertices[start..] {
                        v.depth = depth;
                    }
                }
            }
            if !world {
                for v in &mut self.vertices[mark..] {
                    v.text = 1;
                }
            }
        }
        cursor - x
    }
}
