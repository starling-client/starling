//! Starling's part of hud_layout.rs: the phone's own placement of the Hotbar-mode attack indicator.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

// Adapted from Horizon Android fa548eb7abe91c194d3780e4580b9746323cd606 (MIT).
// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
/// `hotbar_attack_indicator` on the side the phone's settings choose: `attack_left` (`None` is vanilla's,
/// opposite the off hand), moved past an off-hand slot on that side.
pub fn hotbar_attack_indicator_on(
    w: i32,
    h: i32,
    right_handed: bool,
    offhand: bool,
    attack_left: Option<bool>,
    scale: f32,
    out: &mut Vec<GuiDraw>,
) {
    if scale < 1.0 {
        let y = h - 20;
        let center = w / 2;
        // The off hand sits left of the hotbar for a right-handed player.
        let offhand_left = right_handed;
        let mut x = if attack_left.unwrap_or(!offhand_left) {
            center - 91 - 22
        } else {
            center + 91 + 6
        };
        if let Some(left) = attack_left
            && offhand
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
}
