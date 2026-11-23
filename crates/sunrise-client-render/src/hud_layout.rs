//! Layout of vanilla's `Hud` as draw calls (`GuiGraphicsExtractor` blits,
//! fills and text): the hotbar with its selection, off-hand slot, items and
//! attack indicator, the crosshair, armour, hearts, food, air bubbles, mount
//! hearts and the experience bar, plus the draw-call types and colour
//! helpers the other layouts share (`hud_overlays`, `chat`). Computed without
//! drawing so the parity fixture can compare them with the vanilla client.
use crate::{font::Font, text::Text};
pub use mod_api::client::hud_layout::{Blend, HeartKind, PoseOp};
use std::borrow::Cow;
use sunrise_core::random::{LegacyRandom, RandomSource};

/// One `GuiGraphicsExtractor` call.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
pub enum GuiDraw {
    /// `blitSprite(pipeline, sprite, x, y, w, h, color)`; `region` =
    /// (spriteW, spriteH, u, v) for the partial overload.
    Sprite {
        sprite: Cow<'static, str>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        region: Option<[i32; 4]>,
        color: u32,
        blend: Blend,
    },
    /// `blitSprite(pipeline, TextureAtlasSprite, ...)` with a block atlas
    /// sprite (the nether portal overlay).
    AtlasSprite {
        sprite: Cow<'static, str>,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        color: u32,
    },
    /// `blit(pipeline, texture, x, y, u, v, w, h, srcW, srcH, texW, texH,
    /// color)`; `texture` is the path under `textures/` without `.png`.
    Blit {
        texture: Cow<'static, str>,
        blend: Blend,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        uv: [f32; 2],
        src: [i32; 2],
        tex: [i32; 2],
        color: u32,
    },
    /// `fill(x0, y0, x1, y1, color)` with the corners sorted.
    Fill {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: u32,
    },
    /// `fillGradient(x0, y0, x1, y1, top, bottom)`: ARGB `top` along the `y0` edge
    /// blending to `bottom` along the `y1` edge, in the current pose and clip.
    /// Zero area or inversion on exactly one axis draws nothing. With both axes inverted,
    /// bounds are sorted and colours flipped: `top` stays on `y0`, now the lower edge.
    FillGradient {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        top: u32,
        bottom: u32,
    },
    /// `text(font, text, x, y, argb, dropShadow)`.
    Text {
        text: Text,
        x: i32,
        y: i32,
        color: u32,
        shadow: bool,
    },
    /// `item(owner, stack, x, y, seed)` for a hotbar slot (0-8, 40 = off hand)
    /// under the pose's x/y scale about (cx, cy) (the pop squeeze).
    Item {
        slot: u32,
        x: i32,
        y: i32,
        seed: i32,
        scale: [f32; 2],
        pivot: [f32; 2],
    },
    /// `pose().pushMatrix()` followed by `ops`.
    Push(Vec<PoseOp>),
    /// `pose().popMatrix()`.
    Pop,
}

/// A JOML `Matrix3x2f` as `[m00, m01, m10, m11, m20, m21]`.
pub type Pose = [f32; 6];
pub const IDENTITY: Pose = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Applies `op` like JOML's `translate` / `scale` / `rotate` (same float order). A
/// `Rotate` by a NaN or infinite angle is refused: `m` stays as it was (see
/// [`pose_refused`]).
pub fn pose_apply(m: &mut Pose, op: PoseOp) {
    match op {
        PoseOp::Translate(x, y) => {
            m[4] += m[0] * x + m[2] * y;
            m[5] += m[1] * x + m[3] * y;
        }
        PoseOp::Scale(x, y) => {
            m[0] *= x;
            m[1] *= x;
            m[2] *= y;
            m[3] *= y;
        }
        PoseOp::Rotate(angle) if !angle.is_finite() => {}
        PoseOp::Rotate(angle) => {
            // `Matrix3x2f.rotate`: JOML's `Math.cos` / `Math.sin` are `java.lang.Math`'s,
            // computed in double and rounded to float.
            let f = f64::from(angle);
            let (cos, sin) = (
                sunrise_core::jmath::cos(f) as f32,
                sunrise_core::jmath::sin(f) as f32,
            );
            let m00 = m[0] * cos + m[2] * sin;
            let m01 = m[1] * cos + m[3] * sin;
            m[2] = m[0] * -sin + m[2] * cos;
            m[3] = m[1] * -sin + m[3] * cos;
            m[0] = m00;
            m[1] = m01;
        }
    }
}

/// The first op of `ops` that [`pose_apply`] refuses (a `Rotate` by a NaN or infinite
/// angle), if any.
pub fn pose_refused(ops: &[PoseOp]) -> Option<PoseOp> {
    ops.iter()
        .copied()
        .find(|op| matches!(op, PoseOp::Rotate(angle) if !angle.is_finite()))
}

pub const WHITE: u32 = 0xFFFF_FFFF;

fn sprite(sprite: &'static str, x: i32, y: i32, w: i32, h: i32) -> GuiDraw {
    GuiDraw::Sprite {
        sprite: Cow::Borrowed(sprite),
        x,
        y,
        w,
        h,
        region: None,
        color: WHITE,
        blend: Blend::Textured,
    }
}
fn region(
    sprite: &'static str,
    size: [i32; 2],
    uv: [i32; 2],
    at: [i32; 2],
    w: i32,
    h: i32,
) -> GuiDraw {
    GuiDraw::Sprite {
        sprite: Cow::Borrowed(sprite),
        x: at[0],
        y: at[1],
        w,
        h,
        region: Some([size[0], size[1], uv[0], uv[1]]),
        color: WHITE,
        blend: Blend::Textured,
    }
}
/// A draw through `RenderPipelines.CROSSHAIR`.
fn crosshair_blend(mut d: GuiDraw) -> GuiDraw {
    if let GuiDraw::Sprite { blend, .. } = &mut d {
        *blend = Blend::Crosshair;
    }
    d
}
/// `graphics.fill`: the corners are sorted.
pub fn fill(x0: i32, y0: i32, x1: i32, y1: i32, color: u32) -> GuiDraw {
    GuiDraw::Fill {
        x0: x0.min(x1),
        y0: y0.min(y1),
        x1: x0.max(x1),
        y1: y0.max(y1),
        color,
    }
}
/// `graphics.text`: dropped when fully transparent.
fn text(out: &mut Vec<GuiDraw>, text: Text, x: i32, y: i32, color: u32, shadow: bool) {
    if color >> 24 != 0 {
        out.push(GuiDraw::Text {
            text,
            x,
            y,
            color,
            shadow,
        });
    }
}
/// `Options.getBackgroundColor(float)`: `chat_only` = `backgroundForChatOnly`.
pub fn background_color(chat_only: bool, opacity: f64, default: f32) -> u32 {
    let a = if chat_only { default } else { opacity as f32 };
    color_from_float(a, 0.0, 0.0, 0.0)
}

/// The alpha of a name tag's faded text (`NameTagFeatureRenderer`'s see-through and sneaking
/// text) that goes with [`background_color`]'s opacity: the floor of 255 times the larger of
/// (opacity + 0.75) / 2 and 0.5. 0x7F at the default 0.25, 159 at 0.5, 223 at 1.0.
pub fn faded_text_alpha(chat_only: bool, opacity: f64, default: f32) -> u32 {
    let a = if chat_only { default } else { opacity as f32 };
    as8(((a + 0.75) / 2.0).max(0.5))
}

/// `Mth.floor(float)` and `Mth.ceil(float)`.
pub use sunrise_core::mth::{ceil_f32 as ceil, floor_f32 as floor};
/// `ARGB.as8BitChannel`.
pub fn as8(v: f32) -> u32 {
    floor(v * 255.0) as u32
}
/// `ARGB.white(float)`.
pub fn white(alpha: f32) -> u32 {
    (as8(alpha) << 24) | 0xFF_FFFF
}
/// `ARGB.colorFromFloat`.
pub fn color_from_float(a: f32, r: f32, g: f32, b: f32) -> u32 {
    argb(as8(a), as8(r), as8(g), as8(b))
}
/// `ARGB.color(a, r, g, b)`.
pub fn argb(a: u32, r: u32, g: u32, b: u32) -> u32 {
    (a & 255) << 24 | (r & 255) << 16 | (g & 255) << 8 | (b & 255)
}
/// `ARGB.multiply`.
pub fn multiply(l: u32, r: u32) -> u32 {
    if l == WHITE {
        return r;
    }
    if r == WHITE {
        return l;
    }
    let c = |s: u32| ((l >> s) & 255) * ((r >> s) & 255) / 255;
    argb(c(24), c(16), c(8), c(0))
}
/// `Mth.lerpDiscrete`.
pub fn lerp_discrete(alpha: f32, p0: i32, p1: i32) -> i32 {
    p0 + floor(alpha * (p1 - p0 - 1) as f32) + i32::from(alpha > 0.0)
}
/// `Font.width` of styled text.
pub fn width(font: &Font, t: &Text) -> i32 {
    font.width_text(t) as i32
}

/// `Hud.extractItemHotbar` inputs.
pub struct HotbarInputs {
    pub selected: i32,
    /// The main arm is the right one (the off hand sits left of the hotbar).
    pub right_handed: bool,
    pub offhand: bool,
    /// Pop time minus partial tick (`ItemStack.getPopTime`) per hotbar slot
    /// that holds an item, then the off hand.
    pub slots: [Option<f32>; 9],
    pub offhand_pop: f32,
    /// `getAttackStrengthScale(0)` when the attack indicator is in HOTBAR mode.
    pub attack: Option<f32>,
}

/// `Hud.extractItemHotbar`.
pub fn hotbar(w: i32, h: i32, i: &HotbarInputs) -> Vec<GuiDraw> {
    hotbar_with_attack_side(w, h, i, None)
}

// Adapted from Horizon Android fa548eb7abe91c194d3780e4580b9746323cd606 (MIT).
// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
/// Mobile gauge placement, including space for an offhand slot on the same side.
pub fn hotbar_with_attack_side(
    w: i32,
    h: i32,
    i: &HotbarInputs,
    attack_left: Option<bool>,
) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    let center = w / 2;
    out.push(sprite("hud/hotbar", center - 91, h - 22, 182, 22));
    out.push(sprite(
        "hud/hotbar_selection",
        center - 91 - 1 + i.selected * 20,
        h - 22 - 1,
        24,
        23,
    ));
    // The off hand is opposite the main arm.
    let offhand_left = i.right_handed;
    if i.offhand {
        if offhand_left {
            out.push(sprite(
                "hud/hotbar_offhand_left",
                center - 91 - 29,
                h - 23,
                29,
                24,
            ));
        } else {
            out.push(sprite(
                "hud/hotbar_offhand_right",
                center + 91,
                h - 23,
                29,
                24,
            ));
        }
    }
    let slot = |slot: u32, x: i32, y: i32, seed: i32, pop: f32, out: &mut Vec<GuiDraw>| {
        let (scale, pivot) = if pop > 0.0 {
            let squeeze = 1.0 + pop / 5.0;
            (
                [1.0 / squeeze, (squeeze + 1.0) / 2.0],
                [(x + 8) as f32, (y + 12) as f32],
            )
        } else {
            ([1.0, 1.0], [0.0, 0.0])
        };
        out.push(GuiDraw::Item {
            slot,
            x,
            y,
            seed,
            scale,
            pivot,
        });
    };
    let mut seed = 1;
    for (n, pop) in i.slots.iter().enumerate() {
        if let Some(pop) = pop {
            slot(
                n as u32,
                center - 90 + n as i32 * 20 + 2,
                h - 16 - 3,
                seed,
                *pop,
                &mut out,
            );
        }
        seed += 1;
    }
    if i.offhand {
        let x = if offhand_left {
            center - 91 - 26
        } else {
            center + 91 + 10
        };
        slot(40, x, h - 16 - 3, seed, i.offhand_pop, &mut out);
    }
    if let Some(scale) = i.attack
        && scale < 1.0
    {
        let y = h - 20;
        let mut x = if attack_left.unwrap_or(!offhand_left) {
            center - 91 - 22
        } else {
            center + 91 + 6
        };
        if let Some(left) = attack_left
            && i.offhand
            && left == offhand_left
        {
            x += if left { -26 } else { 26 };
        }
        let progress = (scale * 19.0) as i32;
        out.push(sprite(
            "hud/hotbar_attack_indicator_background",
            x,
            y,
            18,
            18,
        ));
        out.push(region(
            "hud/hotbar_attack_indicator_progress",
            [18, 18],
            [0, 18 - progress],
            [x, y + 18 - progress],
            18,
            progress,
        ));
    }
    out
}

/// `Hud.extractCrosshair`'s full attack indicator: at full strength on a
/// living, alive `crosshairPickEntity` (`target`) with a weapon slower than
/// `MIN_CROSSHAIR_ATTACK_SPEED` (`delay` is
/// `getCurrentItemAttackStrengthDelay`).
pub fn full_attack_indicator(scale: f32, delay: f32, target: Option<bool>) -> bool {
    scale >= 1.0 && delay > 5.0 && target == Some(true)
}

/// `Hud.extractCrosshair` (first person, not hidden for spectators), every
/// sprite through the inverting `CROSSHAIR` pipeline: `attack` is
/// `getAttackStrengthScale(0)` in CROSSHAIR indicator mode and `full`
/// whether the full indicator shows ([`full_attack_indicator`]).
pub fn crosshair(w: i32, h: i32, attack: Option<f32>, full: bool) -> Vec<GuiDraw> {
    crosshair_draws(w, h, attack, full)
        .into_iter()
        .map(crosshair_blend)
        .collect()
}
fn crosshair_draws(w: i32, h: i32, attack: Option<f32>, full: bool) -> Vec<GuiDraw> {
    let mut out = vec![sprite("hud/crosshair", (w - 15) / 2, (h - 15) / 2, 15, 15)];
    if let Some(scale) = attack {
        let (x, y) = (w / 2 - 8, h / 2 - 7 + 16);
        if full {
            out.push(sprite("hud/crosshair_attack_indicator_full", x, y, 16, 16));
        } else if scale < 1.0 {
            let progress = (scale * 17.0) as i32;
            out.push(sprite(
                "hud/crosshair_attack_indicator_background",
                x,
                y,
                16,
                4,
            ));
            out.push(region(
                "hud/crosshair_attack_indicator_progress",
                [16, 4],
                [0, 0],
                [x, y],
                progress,
                4,
            ));
        }
    }
    out
}

/// `ExperienceBar.extractBackground` and `ContextualBar.extractExperienceLevel`;
/// `width` measures text with the font.
pub fn experience(
    w: i32,
    h: i32,
    progress: f32,
    level: i32,
    bar: bool,
    width: &dyn Fn(&str) -> i32,
) -> Vec<GuiDraw> {
    experience_colored([None; 2], w, h, progress, level, bar, width)
}
/// [`experience`] with a resource pack's level number colours (Polytone `text.xpbar`: the
/// number's, then its outline's; `None` keeps vanilla's).
pub fn experience_colored(
    [number, outline]: [Option<u32>; 2],
    w: i32,
    h: i32,
    progress: f32,
    level: i32,
    bar: bool,
    width: &dyn Fn(&str) -> i32,
) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    if bar {
        let (left, top) = (w / 2 - 91, h - 32 + 3);
        let filled = (progress * 183.0) as i32;
        out.push(sprite("hud/experience_bar_background", left, top, 182, 5));
        if filled > 0 {
            out.push(region(
                "hud/experience_bar_progress",
                [182, 5],
                [0, 0],
                [left, top],
                filled,
                5,
            ));
        }
    }
    if level > 0 {
        let text = level.to_string();
        let x = (w - width(&text)) / 2;
        let y = h - 24 - 9 - 2;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            self::text(
                &mut out,
                Text::literal(&text),
                x + dx,
                y + dy,
                outline.map_or(0xFF00_0000, opaque),
                false,
            );
        }
        let color = number.map_or(0xFF80_FF20, opaque);
        self::text(&mut out, Text::literal(text), x, y, color, false);
    }
    out
}

// no wire: the vanilla layout reads these only during HUD extraction.
/// The local player's state read by `Hud.extractHearts`.
pub struct HealthInputs {
    /// `Hud.tickCount`.
    pub tick: i32,
    pub max_health: f32,
    pub absorption: f32,
    pub regeneration: bool,
    pub hardcore: bool,
    /// Whether the current frame uses the blinking sprites, from `HealthHud::update_health`.
    pub blink: bool,
}

/// The local player's state read by `Hud.extractFood`.
pub struct FoodInputs {
    pub tick: i32,
    pub food: i32,
    pub saturation: f32,
    pub hunger: bool,
}

/// The local player's state read by `Hud.extractAirBubbles`.
pub struct AirInputs {
    pub tick: i32,
    /// `getAirSupply`, `getMaxAirSupply`, `isEyeInFluid(WATER)`.
    pub air: i32,
    pub max_air: i32,
    pub under_water: bool,
}

/// State kept between frames (`Hud.lastHealth`, `displayHealth`, ...).
#[derive(Clone, Debug)]
pub struct HealthHud {
    pub last_health: i32,
    pub display_health: i32,
    /// `Util.getMillis()` of the last health change.
    pub last_health_time: i64,
    pub health_blink_time: i64,
    /// `Hud.lastBubblePopSoundPlayed`.
    pub last_bubble_pop_sound: i32,
    /// `Hud.random`: hearts reseed it, then food and air consume from the same stream.
    pub random: LegacyRandom,
}

impl Default for HealthHud {
    fn default() -> Self {
        Self {
            last_health: 0,
            display_health: 0,
            last_health_time: 0,
            health_blink_time: 0,
            last_bubble_pop_sound: 0,
            random: crate::random_source::create(),
        }
    }
}

/// The two status-bar heights for one HUD frame, measured from its bottom edge.
#[derive(Default)]
pub struct StatusBarHeights {
    left: StatusColumn,
    right: StatusColumn,
}

struct StatusColumn {
    height: i32,
}

impl Default for StatusColumn {
    fn default() -> Self {
        Self { height: 39 }
    }
}

impl StatusColumn {
    /// Reserve a row (or group of rows) and return its bottom row's top edge.
    fn push(&mut self, screen_height: i32, height: i32) -> i32 {
        let y = screen_height - self.height;
        self.height += height;
        y
    }
}

/// `Hud.getVehicleMaxHearts` for a vehicle that shows its health.
pub fn vehicle_max_hearts(max_health: f32) -> i32 {
    (((max_health + 0.5) as i32) / 2).min(30)
}

/// `Hud.extractVehicleHealth`.
pub fn vehicle_health(
    w: i32,
    h: i32,
    hearts: i32,
    health: f32,
    heights: &mut StatusBarHeights,
    out: &mut Vec<GuiDraw>,
) {
    let current = f64::from(health).ceil() as i32;
    let x_right = w / 2 + 91;
    let rows = hearts / 10 + i32::from(hearts % 10 > 0);
    let mut yo = heights.right.push(h, rows * 10);
    let mut hearts = hearts;
    let mut base = 0;
    while hearts > 0 {
        let row = hearts.min(10);
        hearts -= row;
        for i in 0..row {
            let xo = x_right - i * 8 - 9;
            out.push(sprite("hud/heart/vehicle_container", xo, yo, 9, 9));
            if i * 2 + 1 + base < current {
                out.push(sprite("hud/heart/vehicle_full", xo, yo, 9, 9));
            }
            if i * 2 + 1 + base == current {
                out.push(sprite("hud/heart/vehicle_half", xo, yo, 9, 9));
            }
        }
        yo -= 10;
        base += 20;
    }
}

fn heart(
    kind: Option<HeartKind>,
    absorbing: bool,
    hardcore: bool,
    half: bool,
    blink: bool,
) -> &'static str {
    // Hud.HeartType sprites: [full, full_blinking, half, half_blinking] x [normal, hardcore].
    let table: [&'static str; 8] = match (kind, absorbing) {
        (None, _) => [
            "hud/heart/container",
            "hud/heart/container_blinking",
            "hud/heart/container",
            "hud/heart/container_blinking",
            "hud/heart/container_hardcore",
            "hud/heart/container_hardcore_blinking",
            "hud/heart/container_hardcore",
            "hud/heart/container_hardcore_blinking",
        ],
        (_, true) => [
            "hud/heart/absorbing_full",
            "hud/heart/absorbing_full_blinking",
            "hud/heart/absorbing_half",
            "hud/heart/absorbing_half_blinking",
            "hud/heart/absorbing_hardcore_full",
            "hud/heart/absorbing_hardcore_full_blinking",
            "hud/heart/absorbing_hardcore_half",
            "hud/heart/absorbing_hardcore_half_blinking",
        ],
        (Some(HeartKind::Normal), _) => [
            "hud/heart/full",
            "hud/heart/full_blinking",
            "hud/heart/half",
            "hud/heart/half_blinking",
            "hud/heart/hardcore_full",
            "hud/heart/hardcore_full_blinking",
            "hud/heart/hardcore_half",
            "hud/heart/hardcore_half_blinking",
        ],
        (Some(HeartKind::Poisoned), _) => [
            "hud/heart/poisoned_full",
            "hud/heart/poisoned_full_blinking",
            "hud/heart/poisoned_half",
            "hud/heart/poisoned_half_blinking",
            "hud/heart/poisoned_hardcore_full",
            "hud/heart/poisoned_hardcore_full_blinking",
            "hud/heart/poisoned_hardcore_half",
            "hud/heart/poisoned_hardcore_half_blinking",
        ],
        (Some(HeartKind::Withered), _) => [
            "hud/heart/withered_full",
            "hud/heart/withered_full_blinking",
            "hud/heart/withered_half",
            "hud/heart/withered_half_blinking",
            "hud/heart/withered_hardcore_full",
            "hud/heart/withered_hardcore_full_blinking",
            "hud/heart/withered_hardcore_half",
            "hud/heart/withered_hardcore_half_blinking",
        ],
        (Some(HeartKind::Frozen), _) => [
            "hud/heart/frozen_full",
            "hud/heart/frozen_full_blinking",
            "hud/heart/frozen_half",
            "hud/heart/frozen_half_blinking",
            "hud/heart/frozen_hardcore_full",
            "hud/heart/frozen_hardcore_full_blinking",
            "hud/heart/frozen_hardcore_half",
            "hud/heart/frozen_hardcore_half_blinking",
        ],
    };
    table[usize::from(hardcore) * 4 + usize::from(half) * 2 + usize::from(blink)]
}

/// `Hud.extractArmorLevel`: stack armour above the heart rows actually extracted.
pub fn armor_level(
    armor: i32,
    w: i32,
    h: i32,
    heights: &mut StatusBarHeights,
    out: &mut Vec<GuiDraw>,
) {
    if armor > 0 {
        let y = heights.left.push(h, 10);
        let x_left = w / 2 - 91;
        for n in 0..10 {
            let name = match (n * 2 + 1).cmp(&armor) {
                std::cmp::Ordering::Less => "hud/armor_full",
                std::cmp::Ordering::Equal => "hud/armor_half",
                std::cmp::Ordering::Greater => "hud/armor_empty",
            };
            out.push(sprite(name, x_left + n * 8, y, 9, 9));
        }
    }
}

impl HealthHud {
    /// `Hud.extractHealthLevel`'s bookkeeping before `extractHearts` asks for the sprite kind.
    pub fn update_health(
        &mut self,
        health: f32,
        tick: i32,
        millis: i64,
        damage_cooldown: bool,
    ) -> bool {
        let current = ceil(health);
        let time = i64::from(tick);
        let blink = self.health_blink_time > time && (self.health_blink_time - time) / 3 % 2 == 1;
        if current < self.last_health && damage_cooldown {
            self.last_health_time = millis;
            self.health_blink_time = time + 20;
        } else if current > self.last_health && damage_cooldown {
            self.last_health_time = millis;
            self.health_blink_time = time + 10;
        }
        if millis - self.last_health_time > 1000 {
            self.display_health = current;
            self.last_health_time = millis;
        }
        self.last_health = current;
        self.random.set_seed(i64::from(tick.wrapping_mul(312_871)));
        blink
    }

    /// `Hud.extractHearts`: append the heart sprites after bookkeeping and the kind hook.
    pub fn health_level(
        &mut self,
        i: &HealthInputs,
        kind: HeartKind,
        w: i32,
        h: i32,
        heights: &mut StatusBarHeights,
        out: &mut Vec<GuiDraw>,
    ) {
        let (current, old, blink) = (self.last_health, self.display_health, i.blink);
        let x_left = w / 2 - 91;
        let max_health = i.max_health.max(old.max(current) as f32);
        let absorption = ceil(i.absorption);
        let rows = ceil((max_health + absorption as f32) / 2.0 / 10.0);
        let row_height = (10 - (rows - 2)).max(3);
        let y_hearts = heights.left.push(h, (rows - 1) * row_height + 10);
        let offset_index = if i.regeneration {
            i.tick % ceil(max_health + 5.0)
        } else {
            -1
        };
        let containers = f64::from(max_health / 2.0).ceil() as i32;
        let absorb_containers = (f64::from(absorption) / 2.0).ceil() as i32;
        for c in (0..containers + absorb_containers).rev() {
            let (row, column) = (c / 10, c % 10);
            let x = x_left + column * 8;
            let mut y = y_hearts - row * row_height;
            if current + absorption <= 4 {
                y += self.random.next_int_bound(2).unwrap_or(0);
            }
            if c < containers && c == offset_index {
                y -= 2;
            }
            out.push(sprite(
                heart(None, false, i.hardcore, false, blink),
                x,
                y,
                9,
                9,
            ));
            let halves = c * 2;
            if c >= containers {
                let ah = halves - containers * 2;
                if ah < absorption {
                    let withered = kind == HeartKind::Withered;
                    let name = heart(
                        Some(kind),
                        !withered,
                        i.hardcore,
                        ah + 1 == absorption,
                        false,
                    );
                    out.push(sprite(name, x, y, 9, 9));
                }
            }
            if blink && halves < old {
                let name = heart(Some(kind), false, i.hardcore, halves + 1 == old, true);
                out.push(sprite(name, x, y, 9, 9));
            }
            if halves < current {
                let name = heart(Some(kind), false, i.hardcore, halves + 1 == current, false);
                out.push(sprite(name, x, y, 9, 9));
            }
        }
    }

    /// `Hud.extractFood`: append the food row and reserve its space in the right column.
    pub fn food_level(
        &mut self,
        i: &FoodInputs,
        w: i32,
        h: i32,
        heights: &mut StatusBarHeights,
        out: &mut Vec<GuiDraw>,
    ) {
        let x_right = w / 2 + 91;
        let y_food = heights.right.push(h, 10);
        let (empty, half, full) = if i.hunger {
            (
                "hud/food_empty_hunger",
                "hud/food_half_hunger",
                "hud/food_full_hunger",
            )
        } else {
            ("hud/food_empty", "hud/food_half", "hud/food_full")
        };
        for n in 0..10 {
            let mut y = y_food;
            if i.saturation <= 0.0 && i.tick % (i.food * 3 + 1) == 0 {
                y += self.random.next_int_bound(3).unwrap_or(0) - 1;
            }
            let x = x_right - n * 8 - 9;
            out.push(sprite(empty, x, y, 9, 9));
            if n * 2 + 1 < i.food {
                out.push(sprite(full, x, y, 9, 9));
            }
            if n * 2 + 1 == i.food {
                out.push(sprite(half, x, y, 9, 9));
            }
        }
    }

    /// `Hud.extractAirBubbles`: append the air row and return this frame's bubble-pop volume
    /// and pitch, if any.
    pub fn air_level(
        &mut self,
        i: &AirInputs,
        w: i32,
        h: i32,
        heights: &mut StatusBarHeights,
        out: &mut Vec<GuiDraw>,
    ) -> Option<(f32, f32)> {
        let mut pop_sound = None;
        let x_right = w / 2 + 91;
        let max_air = i.max_air.max(1);
        let air = i.air.clamp(0, max_air);
        if i.under_water || air < max_air {
            let y = heights.right.push(h, 10);
            let bubble = |offset: i32| ceil(((air + offset) * 10) as f32 / max_air as f32);
            let full = bubble(-2);
            let popping = bubble(0);
            let delay = i32::from(air != 0 && i.under_water);
            let empty = 10 - bubble(delay);
            if !i.under_water {
                self.last_bubble_pop_sound = 0;
            }
            for b in 1..=10 {
                let x = x_right - (b - 1) * 8 - 9;
                if b <= full {
                    out.push(sprite("hud/air", x, y, 9, 9));
                } else if full != popping && b == popping && i.under_water {
                    out.push(sprite("hud/air_bursting", x, y, 9, 9));
                    // Hud.playAirBubblePoppedSound.
                    if self.last_bubble_pop_sound != b {
                        let volume = 0.5 + 0.1 * (empty - 3 + 1).max(0) as f32;
                        let pitch = 1.0 + 0.1 * (empty - 5 + 1).max(0) as f32;
                        pop_sound = Some((volume, pitch));
                        self.last_bubble_pop_sound = b;
                    }
                } else if b > 10 - empty {
                    let wobble = if empty == 10 && i.tick % 2 == 0 {
                        self.random.next_int_bound(2).unwrap_or(0)
                    } else {
                        0
                    };
                    out.push(sprite("hud/air_empty", x, y + wobble, 9, 9));
                }
            }
        }
        pop_sound
    }
}

/// Draws the calls with the HUD batch under their poses; items are left to
/// the caller (they need the item renderer) and so are the vignette and
/// nausea blits (their blending needs the screen-effect pipeline).
/// [`paint_items`] draws both.
pub fn paint(batch: &mut crate::hud::HudBatch, atlas: &crate::hud::HudAtlas, draws: &[GuiDraw]) {
    paint_with(batch, atlas, draws, None);
}

/// [`paint`], and with `items` also each `Item` (its stack, scaled about its pivot, then its
/// decorations) and the blits of every blend. Without `items` those draw nothing, as `paint`.
fn paint_with<'a>(
    batch: &mut crate::hud::HudBatch,
    atlas: &crate::hud::HudAtlas,
    draws: impl IntoIterator<Item = &'a GuiDraw>,
    mut items: Option<&mut dyn GuiItems>,
) {
    // The initial identity is implicit, so a layer without Push does not allocate a pose stack.
    let mut stack = Vec::new();
    for d in draws {
        let start = batch.vertices.len();
        let pose = *stack.last().unwrap_or(&IDENTITY);
        match d {
            GuiDraw::Sprite {
                sprite,
                x,
                y,
                w,
                h,
                region,
                color,
                blend,
            } => {
                // An effect sprite (a content mod's under its namespace) draws as the missing
                // sprite where the atlas lacks it; any other GUI sprite is then not drawn.
                let effect = sprite.starts_with("mob_effect/")
                    || sprite
                        .split_once(':')
                        .is_some_and(|(_, path)| path.starts_with("mob_effect/"));
                let found = match effect {
                    true => atlas.gui_sprite_or_missing(sprite),
                    false => atlas.gui_rect(sprite).copied(),
                };
                let Some(mut r) = found else {
                    continue;
                };
                if let Some([sw, sh, u, v]) = *region {
                    let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                    let u0 = r.u0 + du * u as f32 / sw as f32;
                    let v0 = r.v0 + dv * v as f32 / sh as f32;
                    r.u1 = u0 + du * *w as f32 / sw as f32;
                    r.v1 = v0 + dv * *h as f32 / sh as f32;
                    r.u0 = u0;
                    r.v0 = v0;
                }
                let at = [*x as f32, *y as f32, *w as f32, *h as f32];
                match crate::hud::sprite_modifiers::draws(atlas, sprite, r, at, abgr(*color)) {
                    // A Polytone screen sprite modifier: the sprite changed, then its overlays.
                    Some(draws) => {
                        for d in draws {
                            batch.rect_uv(d.at[0], d.at[1], d.at[2], d.at[3], d.rect, d.color);
                        }
                    }
                    None => batch.rect_uv(at[0], at[1], at[2], at[3], r, abgr(*color)),
                }
                if *blend == Blend::Crosshair {
                    batch.invert_since(start);
                }
            }
            GuiDraw::AtlasSprite {
                sprite,
                x,
                y,
                w,
                h,
                color,
            } => {
                // A block atlas sprite, by its packed name, never a GUI sprite of the same id.
                if let Some(r) = atlas.sprites.get(sprite.as_ref()).copied() {
                    batch.rect_uv(*x as f32, *y as f32, *w as f32, *h as f32, r, abgr(*color));
                }
            }
            GuiDraw::Blit { blend, .. } if *blend != Blend::Textured && items.is_none() => {}
            GuiDraw::Blit {
                texture,
                blend,
                x,
                y,
                w,
                h,
                uv,
                src,
                tex,
                color,
            } => {
                let Some(mut r) = atlas.blit_rect(texture.as_ref()).copied() else {
                    continue;
                };
                let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                let u0 = r.u0 + du * uv[0] / tex[0] as f32;
                let v0 = r.v0 + dv * uv[1] / tex[1] as f32;
                r.u1 = r.u0 + du * (uv[0] + src[0] as f32) / tex[0] as f32;
                r.v1 = r.v0 + dv * (uv[1] + src[1] as f32) / tex[1] as f32;
                r.u0 = u0;
                r.v0 = v0;
                batch.rect_uv(*x as f32, *y as f32, *w as f32, *h as f32, r, abgr(*color));
                match blend {
                    Blend::Textured => {}
                    Blend::Vignette => batch.blend_since(start, crate::hud::Blend::Vignette),
                    Blend::Nausea => batch.blend_since(start, crate::hud::Blend::Additive),
                    Blend::Crosshair => batch.invert_since(start),
                }
            }
            GuiDraw::Item {
                slot,
                x,
                y,
                seed,
                scale,
                pivot,
            } => {
                if let Some(items) = items.as_deref_mut() {
                    // `extractSlot`: the item under the pop squeeze, its decorations outside it.
                    items.item(batch, *slot, *x, *y, *seed);
                    batch.scale_since(start, pivot[0], pivot[1], scale[0], scale[1]);
                    items.decorations(batch, *slot, *x, *y);
                }
            }
            GuiDraw::Fill {
                x0,
                y0,
                x1,
                y1,
                color,
            } => {
                // `GuiGraphics.fill` orders the corners; a wrapped bar end can be past the
                // other by more than an `int` holds, so the span is taken in floats.
                let (x0, x1) = ((*x0).min(*x1) as f32, (*x0).max(*x1) as f32);
                let (y0, y1) = ((*y0).min(*y1) as f32, (*y0).max(*y1) as f32);
                batch.fill(atlas, x0, y0, x1 - x0, y1 - y0, abgr(*color));
            }
            GuiDraw::FillGradient {
                x0,
                y0,
                x1,
                y1,
                top,
                bottom,
            } => {
                if x1 != x0 && y1 != y0 && (x1 > x0) == (y1 > y0) {
                    let (x, y) = ((*x0).min(*x1) as f32, (*y0).min(*y1) as f32);
                    let rect = (x, y, (*x0).max(*x1) as f32 - x, (*y0).max(*y1) as f32 - y);
                    let (top, bottom) = if y1 > y0 {
                        (*top, *bottom)
                    } else {
                        (*bottom, *top)
                    };
                    batch.fill_gradient(atlas, rect, abgr(top), abgr(bottom));
                }
            }
            GuiDraw::Text {
                text,
                x,
                y,
                color,
                shadow,
            } => {
                batch.text_styled(atlas, text, *x as f32, *y as f32, abgr(*color), *shadow);
            }
            GuiDraw::Push(ops) => {
                let mut m = pose;
                for op in ops {
                    pose_apply(&mut m, *op);
                }
                stack.push(m);
            }
            GuiDraw::Pop => {
                stack.pop();
            }
        }
        if pose != IDENTITY {
            for v in &mut batch.vertices[start..] {
                let [x, y] = v.pos;
                v.pos = [
                    pose[0] * x + pose[2] * y + pose[4],
                    pose[1] * x + pose[3] * y + pose[5],
                ];
            }
        }
    }
}

/// An ARGB colour as the HUD batch's packed RGBA.
pub fn abgr(color: u32) -> u32 {
    let [a, r, g, b] = color.to_be_bytes();
    crate::hud::rgba(r, g, b, a)
}

/// A pack colour without an alpha drawn opaque, as `ARGB.opaque` makes it (one with alpha keeps it).
fn opaque(color: u32) -> u32 {
    if color >> 24 == 0 {
        color | 0xFF00_0000
    } else {
        color
    }
}

#[path = "hud_layout_items.rs"]
mod items;
pub use items::{GuiItems, Undrawable, item_slot, paint_checked, paint_items, undrawable};

#[cfg(test)]
#[path = "hud_layout_gradient_tests.rs"]
mod gradient_tests;
#[cfg(test)]
#[path = "hud_layout_rotate_tests.rs"]
mod rotate_tests;

#[cfg(test)]
mod player_health_tests {
    use super::*;

    #[test]
    fn status_layers_stack_without_changing_the_distinct_draw_steps() {
        let mut hud = HealthHud::default();
        let blink = hud.update_health(20.0, 1, 2000, false);
        let health = HealthInputs {
            tick: 1,
            max_health: 20.0,
            absorption: 4.0,
            regeneration: false,
            hardcore: false,
            blink,
        };
        let food = FoodInputs {
            tick: 1,
            food: 19,
            saturation: 5.0,
            hunger: false,
        };
        let air = AirInputs {
            tick: 1,
            air: 300,
            max_air: 300,
            under_water: true,
        };
        let (w, h) = (320, 240);
        let mut heights = StatusBarHeights::default();
        let mut rows = Vec::new();
        hud.health_level(&health, HeartKind::Normal, w, h, &mut heights, &mut rows);
        assert_eq!(rows.len(), 24);
        assert_eq!(rows[0], sprite("hud/heart/container", 77, 191, 9, 9));
        assert_eq!(rows[22], sprite("hud/heart/container", 69, 201, 9, 9));
        rows.clear();
        armor_level(1, w, h, &mut heights, &mut rows);
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0], sprite("hud/armor_half", 69, 181, 9, 9));
        rows.clear();
        hud.food_level(&food, w, h, &mut heights, &mut rows);
        assert_eq!(rows.len(), 20);
        assert_eq!(rows[0], sprite("hud/food_empty", 242, 201, 9, 9));
        rows.clear();
        assert_eq!(hud.air_level(&air, w, h, &mut heights, &mut rows), None);
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[0], sprite("hud/air", 242, 191, 9, 9));

        // A mount's second row takes the place where air was, not a food row.
        let mut heights = StatusBarHeights::default();
        rows.clear();
        vehicle_health(w, h, 11, 22.0, &mut heights, &mut rows);
        assert_eq!(
            rows[0],
            sprite("hud/heart/vehicle_container", 242, 201, 9, 9)
        );
        assert_eq!(
            rows[20],
            sprite("hud/heart/vehicle_container", 242, 191, 9, 9)
        );
        rows.clear();
        assert_eq!(hud.air_level(&air, w, h, &mut heights, &mut rows), None);
        assert_eq!(rows[0], sprite("hud/air", 242, 181, 9, 9));
    }
}

#[cfg(test)]
mod faded_text_tests {
    use super::{background_color, faded_text_alpha};

    #[test]
    fn faded_text_follows_the_text_background_opacity() {
        // Chat only: the default 0.25 whatever the slider says.
        assert_eq!(faded_text_alpha(true, 1.0, 0.25), 0x7F);
        assert_eq!(background_color(true, 1.0, 0.25), 0x3F00_0000);
        // The slider at 0.5: backdrop 127, text 159; at 1.0, text 223; never below 0.5.
        assert_eq!(background_color(false, 0.5, 0.25) >> 24, 127);
        assert_eq!(faded_text_alpha(false, 0.5, 0.25), 159);
        assert_eq!(faded_text_alpha(false, 1.0, 0.25), 223);
        assert_eq!(faded_text_alpha(false, 0.0, 0.25), 127);
    }
}
