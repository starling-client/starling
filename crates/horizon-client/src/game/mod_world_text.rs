//! Capability `render`'s world text (`render.set-world-text{,2}`, docs/mods.md):
//! a mod's lines drawn in the world, billboarded to the camera like name
//! tags. Colours, gradients, rainbow, glow, sparkles, letter spacing,
//! outline and background boxes are drawn as font colours and quads; a mod
//! never supplies shader code.
#[cfg(feature = "mods")]
use super::{Game, view_vector};
#[cfg(feature = "mods")]
use horizon_client::{
    block_entity_render::vertex_light,
    gfx::TEXT_POLYGON_OFFSET_OVERLAY,
    hud::{HudAtlas, HudBatch},
    mesher::Vertex,
    text::{Span, Style, Text},
};
#[cfg(feature = "mods")]
use std::time::{SystemTime, UNIX_EPOCH};

/// One GUI pixel of text in the world: `Matrix4f.scale(0.025F, -0.025F, 0.025F)`.
#[cfg(feature = "mods")]
const WORLD_TEXT_UNIT: f32 = 0.025;
/// The mod's texts are near the player; beyond this they are dropped.
#[cfg(feature = "mods")]
const MAX_DISTANCE_SQUARED: f64 = 128.0 * 128.0;
/// A line is never wider than this in the world, in blocks: a longer line
/// shrinks its type to fit, so lines stay readable and apart.
#[cfg(feature = "mods")]
const MAX_LINE_WIDTH: f32 = 4.0;
/// The halo copies around a glowing line, in GUI pixels.
const GLOW_OFFSETS: [(f32, f32); 8] = [
    (-1.0, 0.0),
    (1.0, 0.0),
    (0.0, -1.0),
    (0.0, 1.0),
    (-0.7, -0.7),
    (-0.7, 0.7),
    (0.7, -0.7),
    (0.7, 0.7),
];

#[cfg(feature = "mods")]
fn argb(alpha: u32, rgb: u32) -> u32 {
    (alpha << 24) | rgb
}

#[cfg(feature = "mods")]
fn split_argb(color: u32) -> (u8, u8, u8) {
    ((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

#[cfg(feature = "mods")]
fn pack_rgb(r: u8, g: u8, b: u8) -> u32 {
    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

#[cfg(feature = "mods")]
fn lerp_rgb(a: u32, b: u32, t: f32) -> u32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    pack_rgb(
        mix((a >> 16) as u8, (b >> 16) as u8),
        mix((a >> 8) as u8, (b >> 8) as u8),
        mix(a as u8, b as u8),
    )
}

/// Full-saturation HSV at `hue` (0..1) as 0xRRGGBB.
#[cfg(feature = "mods")]
fn rainbow(hue: f32) -> u32 {
    let h = hue.rem_euclid(1.0) * 6.0;
    let i = h.floor() as u32;
    let f = h - i as f32;
    let one = f;
    let two = 1.0 - f;
    let (r, g, b) = match i {
        0 => (1.0, one, 0.0),
        1 => (two, 1.0, 0.0),
        2 => (0.0, 1.0, one),
        3 => (0.0, two, 1.0),
        4 => (one, 0.0, 1.0),
        _ => (1.0, 0.0, two),
    };
    pack_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Seconds since the epoch, for rainbow and twinkle phases.
#[cfg(feature = "mods")]
fn time_now() -> f32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f32())
        .unwrap_or(0.0)
}

/// The line's text with per-glyph colours (gradient and rainbow are per
/// glyph) plus its width in GUI pixels including letter spacing.
#[cfg(feature = "mods")]
fn styled_line(
    atlas: &HudAtlas,
    shape: &horizon_mods::WorldTextShape2,
    now: f32,
) -> (Text, f32) {
    let base = split_argb(shape.color);
    let spacing = shape.letter_spacing;
    let bold = shape.bold;
    let mut widths: Vec<f32> = shape
        .text
        .chars()
        .map(|ch| {
            let glyph = atlas.font.glyph(ch);
            glyph.advance + if bold { glyph.offset } else { 0.0 } + spacing
        })
        .collect();
    let total: f32 = widths.iter().sum();
    let mut ts = Vec::with_capacity(widths.len());
    let mut x = 0.0;
    for w in &widths {
        ts.push(if total > 0.0 { (x + w * 0.5) / total } else { 0.0 });
        x += w;
    }
    widths.clear();
    let mut text = Text::default();
    for (i, ch) in shape.text.chars().enumerate() {
        let t = ts[i];
        let color = if shape.rainbow {
            rainbow(now * 0.33 + t * 1.5)
        } else if let Some(other) = shape.gradient {
            lerp_rgb(
                pack_rgb(base.0, base.1, base.2),
                pack_rgb((other >> 16) as u8, (other >> 8) as u8, other as u8),
                t,
            )
        } else {
            pack_rgb(base.0, base.1, base.2)
        };
        text.spans.push(Span {
            text: ch.to_string(),
            style: Style {
                color: Some(color),
                bold: shape.bold.then_some(true),
                ..Style::default()
            },
        });
    }
    (text, total)
}

/// The halo and outline passes behind the line, then the line, then its
/// sparkles, drawn into `batch`.
#[cfg(feature = "mods")]
#[allow(clippy::too_many_arguments)]
fn draw_spans(
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    shape: &horizon_mods::WorldTextShape2,
    text: &Text,
    width: f32,
    x: f32,
    y: f32,
    now: f32,
) {
    let alpha = (shape.color >> 24).max(1);
    let base = split_argb(shape.color);
    let span_color = |rgb: u32| argb(alpha, rgb);
    let single = |rgb: u32| Text::styled(shape.text.as_str(), Style::color(rgb));
    if let Some(background) = shape.background.filter(|c| (c >> 24) > 0) {
        batch.fill(atlas, x - 1.5, y - 1.5, width + 3.0, 12.0, background);
    }
    if shape.glow {
        let gc = shape.glow_color.unwrap_or(shape.color);
        let glow = split_argb(gc);
        let halo_alpha = if shape.glow_color.is_some() {
            (gc >> 24).max(1)
        } else {
            (alpha / 2).max(1)
        };
        let halo = single(pack_rgb(glow.0, glow.1, glow.2));
        for (dx, dy) in GLOW_OFFSETS {
            batch.text_styled_spaced(
                atlas,
                &halo,
                x + dx,
                y + dy,
                argb(halo_alpha, 0xFF_FFFF),
                false,
                shape.letter_spacing,
                true,
            );
        }
    }
    if let Some(outline_color) = shape.outline {
        let oc = split_argb(outline_color);
        let outline = single(pack_rgb(oc.0, oc.1, oc.2));
        let d = shape.outline_thickness;
        let half = d * 0.7;
        for (dx, dy) in [
            (-d, 0.0),
            (d, 0.0),
            (0.0, -d),
            (0.0, d),
            (-half, -half),
            (-half, half),
            (half, -half),
            (half, half),
        ] {
            batch.text_styled_spaced(
                atlas,
                &outline,
                x + dx,
                y + dy,
                argb((outline_color >> 24).max(1), 0xFF_FFFF),
                false,
                shape.letter_spacing,
                true,
            );
        }
    }
    batch.text_styled_spaced(atlas, text, x, y, span_color(0xFF_FFFF), shape.shadow, shape.letter_spacing, true);
    // Twinkles drifting over the line.
    if shape.sparkle && shape.sparkle_amount > 0.0 {
        let amount = shape.sparkle_amount.clamp(0.0, 2.0);
        let count = (6.0 * amount).round() as usize;
        for k in 0..count {
            let phase = now * 1.7 + k as f32 * 1.31;
            let twinkle = 0.3 + 0.7 * (0.5 + 0.5 * (now * 3.1 + k as f32 * 1.73).sin());
            let size = (1.0 + 1.1 * (now * 2.3 + k as f32 * 2.17).sin().abs())
                * (0.6 + 0.4 * amount);
            let dx = x + ((k as f32 + 0.5) / count.max(1) as f32) * width + phase.sin() * 3.0;
            let dy = y + 5.5 + phase.cos() * 7.0;
            let a = (alpha as f32 * twinkle) as u32;
            let star = (0.65 + 0.35 * twinkle).min(1.0);
            let r = (base.0 as f32 * (1.0 - star) + 255.0 * star) as u8;
            let g = (base.1 as f32 * (1.0 - star) + 255.0 * star) as u8;
            let b = (base.2 as f32 * (1.0 - star) + 255.0 * star) as u8;
            let c = argb(a, pack_rgb(r, g, b));
            let corners = [
                [dx, dy - size],
                [dx + size * 0.35, dy - size * 0.35],
                [dx + size, dy],
                [dx + size * 0.35, dy + size * 0.35],
                [dx, dy + size],
                [dx - size * 0.35, dy + size * 0.35],
                [dx - size, dy],
                [dx - size * 0.35, dy - size * 0.35],
            ];
            batch.fill_quad(atlas, [corners[0], corners[2], corners[4], corners[6]], c);
            batch.fill_quad(atlas, [corners[1], corners[3], corners[5], corners[7]], c);
        }
    }
}

/// A level-8 shape, with the level-9 knobs at their defaults.
#[cfg(feature = "mods")]
fn v1_to_v2(shape: &horizon_mods::WorldTextShape) -> horizon_mods::WorldTextShape2 {
    horizon_mods::WorldTextShape2 {
        text: shape.text.clone(),
        at: shape.at,
        scale: shape.scale,
        color: shape.color,
        gradient: shape.gradient,
        rainbow: shape.rainbow,
        bold: shape.bold,
        shadow: shape.shadow,
        glow: shape.glow,
        sparkle: shape.sparkle,
        see_through: shape.see_through,
        letter_spacing: 0.0,
        roll: 0.0,
        outline: None,
        outline_thickness: 1.0,
        background: None,
        glow_color: None,
        sparkle_amount: 1.0,
    }
}

#[cfg(feature = "mods")]
impl Game<'_> {
    pub(super) fn mod_world_text_geometry(
        &mut self,
        camera: glam::DVec3,
        out: &mut Vec<Vertex>,
        see_through: &mut Vec<Vertex>,
    ) {
        let texts: Vec<_> = self.mods.mods.world_texts().map(v1_to_v2).collect();
        let texts2: Vec<_> = self.mods.mods.world_texts2().cloned().collect();
        if texts.is_empty() && texts2.is_empty() {
            return;
        }
        // Mod world texts are drawn with the opaque ENTITIES pass, like name
        // tags: the pack's translucent-entities program drops the font's
        // vertex-alpha quads.
        self.mark_entity(out.len(), "minecraft:text_display");
        self.push_world_texts(
            camera,
            texts.into_iter().chain(texts2),
            out,
            see_through,
            false,
        );
        self.mark_translucent_entity(out.len(), "");
    }

    /// `render.set-post-world-text2`'s quads: drawn after the world post
    /// pass (a shader pack's composite), over the finished image. Not
    /// blended into the entity stream, not packed-claimed: the pack never
    /// sees these.
    pub(super) fn mod_post_world_text_geometry(
        &mut self,
        camera: glam::DVec3,
        out: &mut Vec<Vertex>,
    ) {
        let shapes: Vec<_> = self.mods.mods.world_texts_post2().cloned().collect();
        if shapes.is_empty() {
            return;
        }
        // The post pass draws every shape, so the see-through ones share `out`.
        self.push_world_texts(camera, shapes.into_iter(), out, &mut Vec::new(), true);
    }

    fn push_world_texts(
        &mut self,
        camera: glam::DVec3,
        shapes: impl Iterator<Item = horizon_mods::WorldTextShape2>,
        out: &mut Vec<Vertex>,
        see_through: &mut Vec<Vertex>,
        post: bool,
    ) {
        let now = time_now();
        let (view_yaw, view_pitch) =
            if self.camera_type == super::perspective::CameraType::ThirdPersonFront {
                (self.yaw + 180.0, -self.pitch)
            } else {
                (self.yaw, self.pitch)
            };
        let forward = glam::DVec3::from_array(view_vector(view_yaw, view_pitch));
        let right = forward.cross(glam::DVec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let away = forward.as_vec3() * 0.002;
        for shape in shapes {
            let o = glam::DVec3::new(
                f64::from(shape.at[0]),
                f64::from(shape.at[1]),
                f64::from(shape.at[2]),
            ) - camera;
            if o.length_squared() > MAX_DISTANCE_SQUARED {
                continue;
            }
            let light = self.world.light(
                shape.at[0].floor() as i32,
                shape.at[1].floor() as i32,
                shape.at[2].floor() as i32,
            );
            let lit = (light >> 4).max(2) << 4 | (light & 15).max(2);
            let light = if shape.see_through {
                0
            } else {
                vertex_light(lit)
            };
            let layer = if shape.see_through {
                2
            } else {
                TEXT_POLYGON_OFFSET_OVERLAY
            };
            let atlas = &self.gui.atlas;
            let (text, width) = styled_line(atlas, &shape, now);
            let raw_unit = WORLD_TEXT_UNIT * shape.scale;
            let fitted = if width * raw_unit > MAX_LINE_WIDTH {
                MAX_LINE_WIDTH / width
            } else {
                raw_unit
            };
            let (sin, cos) = shape.roll.to_radians().sin_cos();
            let r = right.as_vec3() * fitted;
            let u = up.as_vec3() * fitted;
            let (rr, uu) = if shape.roll == 0.0 {
                (r, u)
            } else {
                (r * cos - u * sin, r * sin + u * cos)
            };
            let o = o.as_vec3();
            let at = |x: f32, y: f32| o + rr * x - uu * y;
            let x = -width / 2.0;
            let mut batch = HudBatch::new([1, 1]);
            draw_spans(atlas, &mut batch, &shape, &text, width, x, 0.0, now);
            let target: &mut Vec<Vertex> = if shape.see_through && !post {
                &mut *see_through
            } else {
                &mut *out
            };
            for quad in batch.vertices.as_chunks::<6>().0 {
                for v in [quad[5], quad[2], quad[1], quad[0]] {
                    target.push(Vertex {
                        normal: [0; 4],
                        pos: (at(v.pos[0], v.pos[1]) + away).to_array(),
                        uv: v.uv,
                        layer,
                        color: v.color,
                        light,
                        glint_uv: [0.0, 0.0],
                    });
                }
            }
            self.map_patches.extend(batch.patches);
        }
    }
}
