//! Starling's part of atlas.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Below this a texture at half size stops being worth what it costs to look at.
pub(in super::super) const SMALLEST_SHRUNK_SIDE: u32 = 32;

/// The interface's textures shrink only once the others are down to this side.
pub(in super::super) const INTERFACE_WAITS_FOR_SIDE: u32 = 64;

/// A side of a GPU's largest texture, once the renderer has found the GPU; 0 before.
pub(super) static GPU_TEXTURE_SIDE: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// The renderer's word on what the GPU takes, which the atlas is packed within.
pub fn set_gpu_texture_side(side: u32) {
    GPU_TEXTURE_SIDE.store(side, std::sync::atomic::Ordering::Relaxed);
}

/// The most an atlas may measure a side. On a phone the renderer finds the GPU while the atlas
/// loads, and the small GPUs are there, so this waits for its word (not for ever: it only asks
/// the driver); elsewhere a GPU not found yet counts as one that takes the most.
pub(in super::super) fn atlas_limit() -> u32 {
    let side = || GPU_TEXTURE_SIDE.load(std::sync::atomic::Ordering::Relaxed);
    if cfg!(target_os = "android") {
        let start = std::time::Instant::now();
        while side() == 0 && start.elapsed() < std::time::Duration::from_secs(20) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    match side() {
        0 => MAX_ATLAS_SIZE,
        side => side.min(MAX_ATLAS_SIZE),
    }
}

/// `image` at half its size, by every other texel: no new colors or alpha, so what was cut out
/// stays cut out.
pub(in super::super) fn halved((name, w, h, data): &PackedImage) -> PackedImage {
    let (half_w, half_h) = (w.div_ceil(2), h.div_ceil(2));
    let mut half = Vec::with_capacity((half_w * half_h * 4) as usize);
    for y in 0..half_h {
        for x in 0..half_w {
            let at = ((y * 2 * w + x * 2) * 4) as usize;
            half.extend_from_slice(&data[at..at + 4]);
        }
    }
    (name.clone(), half_w, half_h, half)
}

// Bake the settings glyph into its plate: drawing a translucent glyph over
// a translucent plate changes its effective alpha and darkens it twice.
pub(in super::super) fn android_utility_sprites(images: &[PackedImage]) -> Vec<PackedImage> {
    const GEAR: [&str; 13] = [
        ".....###.....",
        "..##.###.##..",
        ".###########.",
        ".###########.",
        "..###...###..",
        "####.....####",
        "####.....####",
        "####.....####",
        "..###...###..",
        ".###########.",
        ".###########.",
        "..##.###.##..",
        ".....###.....",
    ];
    let mut out = Vec::new();
    for name in ["pause", "inventory", "settings"] {
        for pressed in [false, true] {
            let suffix = if pressed { "_pressed" } else { "" };
            let source = if name == "settings" {
                "custom_blank"
            } else {
                name
            };
            let key = format!("starling:gui/sprites/touch/{source}{suffix}");
            let Some((_, w, h, pixels)) = images.iter().find(|i| i.0 == key).or_else(|| {
                images
                    .iter()
                    .find(|i| i.0 == format!("starling:gui/sprites/touch/{source}"))
            }) else {
                continue;
            };
            let mut pixels = pixels.clone();
            if name == "settings" && *w >= 13 && *h >= 13 {
                let ox = (*w - 13) / 2;
                let oy = (*h - 13) / 2;
                for (y, row) in GEAR.iter().enumerate() {
                    for (x, pixel) in row.bytes().enumerate() {
                        if pixel == b'#' {
                            let index = (((oy + y as u32) * w + ox + x as u32) * 4) as usize;
                            let ink = if pressed { 230 } else { 0 };
                            pixels[index..index + 3].fill(ink);
                        }
                    }
                }
            }
            if name != "inventory" {
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel[3] = (u16::from(pixel[3]) * 204 / 255) as u8;
                }
            }
            out.push((
                format!("starling:android-controls/{name}{suffix}"),
                *w,
                *h,
                pixels,
            ));
        }
    }
    out
}

/// Starling's own touch controls (packaging/touch/make.py draws them): a button's sprite and its
/// `_pressed`, the D-pad's cells and the stick's, as `starling:touch/<name>`.
pub(super) const TOUCH_SPRITES: &[(&str, &[u8])] = &[
    (
        "custom_blank",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_blank.png"),
    ),
    (
        "custom_star",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_star.png"),
    ),
    (
        "custom_star_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_star_pressed.png"),
    ),
    (
        "custom_heart",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_heart.png"),
    ),
    (
        "custom_heart_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_heart_pressed.png"),
    ),
    (
        "custom_shield",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_shield.png"),
    ),
    (
        "custom_shield_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_shield_pressed.png"),
    ),
    (
        "custom_pickaxe",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_pickaxe.png"),
    ),
    (
        "custom_pickaxe_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_pickaxe_pressed.png"),
    ),
    (
        "custom_potion",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_potion.png"),
    ),
    (
        "custom_potion_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_potion_pressed.png"),
    ),
    (
        "custom_apple",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_apple.png"),
    ),
    (
        "custom_apple_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_apple_pressed.png"),
    ),
    (
        "custom_home",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_home.png"),
    ),
    (
        "custom_home_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_home_pressed.png"),
    ),
    (
        "custom_flag",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_flag.png"),
    ),
    (
        "custom_flag_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_flag_pressed.png"),
    ),
    (
        "custom_fire",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_fire.png"),
    ),
    (
        "custom_fire_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_fire_pressed.png"),
    ),
    (
        "custom_skull",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_skull.png"),
    ),
    (
        "custom_skull_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_skull_pressed.png"),
    ),
    (
        "custom_crown",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_crown.png"),
    ),
    (
        "custom_crown_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_crown_pressed.png"),
    ),
    (
        "custom_plus",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_plus.png"),
    ),
    (
        "custom_plus_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_plus_pressed.png"),
    ),
    (
        "custom_check",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_check.png"),
    ),
    (
        "custom_check_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_check_pressed.png"),
    ),
    (
        "custom_cross",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_cross.png"),
    ),
    (
        "custom_cross_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_cross_pressed.png"),
    ),
    (
        "custom_music",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_music.png"),
    ),
    (
        "custom_music_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/custom_music_pressed.png"),
    ),
    (
        "attack",
        include_bytes!("../../../../sunrise-client/assets/touch/attack.png"),
    ),
    (
        "attack_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/attack_pressed.png"),
    ),
    (
        "camera",
        include_bytes!("../../../../sunrise-client/assets/touch/camera.png"),
    ),
    (
        "camera_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/camera_pressed.png"),
    ),
    (
        "chat",
        include_bytes!("../../../../sunrise-client/assets/touch/chat.png"),
    ),
    (
        "chat_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/chat_pressed.png"),
    ),
    (
        "command",
        include_bytes!("../../../../sunrise-client/assets/touch/command.png"),
    ),
    (
        "command_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/command_pressed.png"),
    ),
    (
        "debug",
        include_bytes!("../../../../sunrise-client/assets/touch/debug.png"),
    ),
    (
        "debug_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/debug_pressed.png"),
    ),
    (
        "dpad_down",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_down.png"),
    ),
    (
        "dpad_down_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_down_pressed.png"),
    ),
    (
        "dpad_left",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_left.png"),
    ),
    (
        "dpad_left_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_left_pressed.png"),
    ),
    (
        "dpad_middle",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_middle.png"),
    ),
    (
        "dpad_right",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_right.png"),
    ),
    (
        "dpad_right_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_right_pressed.png"),
    ),
    (
        "dpad_up",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_up.png"),
    ),
    (
        "dpad_up_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/dpad_up_pressed.png"),
    ),
    (
        "drop",
        include_bytes!("../../../../sunrise-client/assets/touch/drop.png"),
    ),
    (
        "drop_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/drop_pressed.png"),
    ),
    (
        "inventory",
        include_bytes!("../../../../sunrise-client/assets/touch/inventory.png"),
    ),
    (
        "inventory_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/inventory_pressed.png"),
    ),
    (
        "jump",
        include_bytes!("../../../../sunrise-client/assets/touch/jump.png"),
    ),
    (
        "jump_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/jump_pressed.png"),
    ),
    (
        "offhand",
        include_bytes!("../../../../sunrise-client/assets/touch/offhand.png"),
    ),
    (
        "offhand_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/offhand_pressed.png"),
    ),
    (
        "pause",
        include_bytes!("../../../../sunrise-client/assets/touch/pause.png"),
    ),
    (
        "pause_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/pause_pressed.png"),
    ),
    (
        "pick",
        include_bytes!("../../../../sunrise-client/assets/touch/pick.png"),
    ),
    (
        "pick_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/pick_pressed.png"),
    ),
    (
        "players",
        include_bytes!("../../../../sunrise-client/assets/touch/players.png"),
    ),
    (
        "players_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/players_pressed.png"),
    ),
    (
        "sneak",
        include_bytes!("../../../../sunrise-client/assets/touch/sneak.png"),
    ),
    (
        "sneak_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/sneak_pressed.png"),
    ),
    (
        "stick",
        include_bytes!("../../../../sunrise-client/assets/touch/stick.png"),
    ),
    (
        "stick_knob",
        include_bytes!("../../../../sunrise-client/assets/touch/stick_knob.png"),
    ),
    (
        "use",
        include_bytes!("../../../../sunrise-client/assets/touch/use.png"),
    ),
    (
        "use_pressed",
        include_bytes!("../../../../sunrise-client/assets/touch/use_pressed.png"),
    ),
];

/// The names of [`TOUCH_SPRITES`]: every sprite the touch controls draw.
pub fn touch_sprite_names() -> impl Iterator<Item = &'static str> {
    TOUCH_SPRITES.iter().map(|(name, _)| *name)
}

/// [`TOUCH_SPRITES`] a pack has not drawn already (a pack's `starling:touch/jump` wins).
pub(in super::super) fn builtin_touch_sprites(images: &[PackedImage]) -> Vec<PackedImage> {
    TOUCH_SPRITES
        .iter()
        .filter_map(|(name, png)| {
            let key = format!("starling:gui/sprites/touch/{name}");
            if images.iter().any(|(k, ..)| *k == key) {
                return None;
            }
            let (w, h, mut pixels) = decode_bytes(png).ok()?;
            if matches!(*name, "pause" | "inventory") {
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    if pixel[0] >= 245 && pixel[1] >= 245 && pixel[2] >= 245 {
                        pixel[..3].copy_from_slice(&[32, 32, 32]);
                    }
                }
            }
            Some((key, w, h, pixels))
        })
        // A slot for each custom button's own images, whichever it has now.
        .chain(
            (0..crate::own_buttons::pressed_slot(crate::own_buttons::SLOTS)).map(|id| {
                let side = crate::own_buttons::SIZE;
                let blank = || vec![0; (side * side * 4) as usize];
                let pixels = crate::own_buttons::load(id).unwrap_or_else(blank);
                (crate::own_buttons::sprite(id), side, side, pixels)
            }),
        )
        .collect()
}

/// One of [`TOUCH_SPRITES`], as RGBA pixels: a place for a custom button's image to start from.
pub fn touch_sprite(name: &str) -> Option<Vec<u8>> {
    let (_, png) = TOUCH_SPRITES.iter().find(|(n, _)| *n == name)?;
    decode_bytes(png).ok().map(|(_, _, pixels)| pixels)
}
