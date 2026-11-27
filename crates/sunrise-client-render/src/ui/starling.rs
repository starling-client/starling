//! Starling's part of ui.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Starling's low-budget skin (`assets/starling/skin.json`, packaging/lowbudget): how many takes
/// of each button it drew (`widget/button_1`...), and the writing on a pressed one (RGB, 0: none).
pub(super) static BUTTON_TAKES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

pub(super) static PRESSED_LABEL: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

pub fn load_skin(root: &sunrise_assets::resource_source::ResourceSource) {
    use std::sync::atomic::Ordering::Relaxed;
    let skin: serde_json::Value = root
        .read("assets/starling/skin.json")
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let takes = skin["button_takes"].as_u64().unwrap_or(1).clamp(1, 16) as u32;
    BUTTON_TAKES.store(takes, Relaxed);
    let label = skin["pressed_label"]
        .as_str()
        .and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok());
    PRESSED_LABEL.store(label.map_or(0, |rgb| rgb | 0xff00_0000), Relaxed);
}

/// The take of a button `sprite` drawn at `x, y`: a skin draws its buttons several times over, and
/// a button keeps its take where it stands.
pub fn button_take(sprite: &str, x: i32, y: i32) -> String {
    let takes = BUTTON_TAKES.load(std::sync::atomic::Ordering::Relaxed);
    if takes <= 1 {
        return sprite.to_owned();
    }
    let mut h = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    match (h.wrapping_mul(0x2c1b_3c6d) >> 9) % takes {
        0 => sprite.to_owned(),
        k => format!("{sprite}_{k}"),
    }
}

/// The writing on a pressed button when the skin darkens it (0xRRGGBB).
pub fn pressed_label() -> Option<u32> {
    match PRESSED_LABEL.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        argb => Some(argb & 0x00ff_ffff),
    }
}

/// A GUI sprite drawn at a size as vanilla's `blitSprite` does: nine-sliced when its `.mcmeta`
/// says so (the buttons), stretched otherwise (an icon).
pub fn sprite_sized(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
) {
    nine_slice(batch, atlas, sprite, x, y, w, h, color);
}

impl std::fmt::Debug for Caret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Caret")
            .field("field", &self.field)
            .field("text", &"<hidden>")
            .finish()
    }
}

impl Caret {
    /// Draw only a masked copy; editing retains the original text and cursor.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_password(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &str, usize),
        rect: [f32; 4],
        focused: bool,
        focused_at: Instant,
    ) {
        let mut masked = if focused {
            self.sync(id, value, max).clone()
        } else {
            let mut idle = EditBox::new(0.0, 0.0, 0.0, 0.0);
            idle.max_length = max;
            idle.set_value(value);
            idle
        };
        masked.value = "*".repeat(value.chars().count());
        [masked.x, masked.y, masked.width, masked.height] = rect;
        masked.focused = focused;
        masked.focused_at = focused_at;
        masked.draw(batch, atlas, None);
    }

    pub fn press_password(
        &mut self,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &mut String, usize),
        rect: [f32; 4],
        x: f32,
        shift: bool,
    ) {
        let mut mask = "*".repeat(value.chars().count());
        self.press(atlas, (id, &mut mask, max), rect, x, shift);
        self.edit.value.clone_from(value);
    }

    pub fn drag_password(
        &mut self,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &mut String, usize),
        x: f32,
    ) -> bool {
        if !self.held || self.last_click.is_none_or(|(_, field)| field != id) {
            return false;
        }
        let mut mask = "*".repeat(value.chars().count());
        self.edit.value.clone_from(&mask);
        let dragged = self.drag(atlas, (id, &mut mask, max), x);
        self.edit.value.clone_from(value);
        dragged
    }
}

impl EditBox {
    /// Passwords are not Minecraft chat: a section sign is a legal credential.
    pub fn insert_password(&mut self, text: &str) {
        if !self.editable {
            return;
        }
        use mod_api::client::widgets::{byte_index, char_len, utf16_len, utf16_prefix};
        let plain: String = text.chars().filter(|c| !c.is_control()).collect();
        // TextEdit::insert_text without its chat filter (`filter_text`): the selection replaced,
        // cut to what the maximum length leaves room for, as EditBox.insertText.
        let edit = &mut self.edit;
        let (a, b) = edit.selected();
        let selected_units: usize = edit
            .value
            .chars()
            .skip(a)
            .take(b - a)
            .map(char::len_utf16)
            .sum();
        let room = edit
            .max_length
            .saturating_sub(utf16_len(&edit.value) - selected_units);
        let insert = utf16_prefix(&plain, room);
        let (ba, bb) = (byte_index(&edit.value, a), byte_index(&edit.value, b));
        edit.value.replace_range(ba..bb, &insert);
        edit.set_cursor(a + char_len(&insert), false);
        self.moved();
    }
}
