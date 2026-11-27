//! Starling's part of hud_room.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;
#[cfg(target_os = "android")]
use crate::game::Screen;

/// The HUD works in local safe-area coordinates. Move its vertices and item
/// scissors back into the full frame after drawing it.
pub(in super::super) struct HudArea {
    pub(super) full: [f32; 2],
    pub(super) origin: [f32; 2],
    pub(super) first_vertex: usize,
    pub(super) first_blend: usize,
}

impl HudArea {
    pub(in super::super) fn begin(
        batch: &mut HudBatch,
        [left, top, right, bottom]: [f32; 4],
    ) -> Self {
        let area = Self {
            full: [batch.width, batch.height],
            origin: [left, top],
            first_vertex: batch.vertices.len(),
            first_blend: batch.blends.len(),
        };
        batch.width -= left + right;
        batch.height -= top + bottom;
        area
    }

    pub(in super::super) fn finish(self, batch: &mut HudBatch) {
        let [x, y] = self.origin;
        [batch.width, batch.height] = self.full;
        if self.origin == [0.0; 2] {
            return;
        }
        for vertex in &mut batch.vertices[self.first_vertex..] {
            vertex.pos[0] += x;
            vertex.pos[1] += y;
        }
        for (_, blend) in &mut batch.blends[self.first_blend..] {
            use sunrise_client::hud::Blend;
            if let Blend::Depth(rect) | Blend::DepthGlint(rect) = blend {
                rect[0] += (x * batch.scale) as u32;
                rect[1] += (y * batch.scale) as u32;
            }
        }
    }

    #[cfg(target_os = "android")]
    pub(in super::super) fn local_point(&self, point: [f32; 2], scale: f32) -> [f32; 2] {
        std::array::from_fn(|axis| point[axis] / scale - self.origin[axis])
    }
}

impl Game<'_> {
    /// Menus use the framebuffer's GUI scale and lay out within its safe area.
    pub(in crate::game) fn interface_batch(&self, size: [u32; 2]) -> HudBatch {
        let mut batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        let [left, top, right, bottom] = self.interface_insets(&batch);
        batch.width -= left + right;
        batch.height -= top + bottom;
        batch
    }

    pub(in super::super) fn interface_area(&self, batch: &mut HudBatch) -> HudArea {
        let insets = self.interface_insets(batch);
        HudArea::begin(batch, insets)
    }

    pub(in super::super) fn draw_interface_overlays(&mut self, batch: &mut HudBatch) {
        let area = self.interface_area(batch);
        self.draw_slot_touch(batch);
        // The confirmation screen takes the dialog's place.
        if self.command_prompt.is_none() {
            self.draw_dialog(batch);
        }
        self.draw_pack_prompt(batch);
        self.draw_code_of_conduct(batch);
        self.draw_link_prompt(batch);
        area.finish(batch);

        self.draw_touch(batch);

        let area = self.interface_area(batch);
        self.draw_chat_bar(batch);
        self.draw_command_prompt(batch);
        area.finish(batch);
    }

    pub(super) fn interface_insets(&self, batch: &HudBatch) -> [f32; 4] {
        #[cfg(target_os = "android")]
        if self.screen != Screen::Playing
            || self.title.is_some()
            || self.screen_over_playing()
            || self.remote_loading()
        {
            // Keep the editor in place while its sliders preview the menu bounds.
            let editing = self.screen == Screen::Options && self.ui.options.display_open();
            return self.menu_margins(batch, editing);
        }
        let _ = batch;
        [0.0; 4]
    }

    #[cfg(target_os = "android")]
    pub(super) fn menu_margins(&self, batch: &HudBatch, editing: bool) -> [f32; 4] {
        use sunrise_client::android_display::{DisplayOption as D, interface_safe_zone};
        let margins = if editing {
            [0; 2]
        } else {
            [
                D::InterfaceHorizontal.value(&self.settings),
                D::InterfaceVertical.value(&self.settings),
            ]
        };
        interface_safe_zone([batch.width, batch.height], margins)
    }

    #[cfg(target_os = "android")]
    pub(in super::super) fn button_safe_insets(&self, batch: &HudBatch) -> [f32; 4] {
        use sunrise_client::android_display::{DisplayOption as D, interface_safe_zone};
        interface_safe_zone(
            [batch.width, batch.height],
            [
                D::ButtonsHorizontal.value(&self.settings),
                D::ButtonsVertical.value(&self.settings),
            ],
        )
    }

    /// Rebase the pointer when the menu opens, closes, or changes size.
    pub(in super::super) fn sync_interface_cursor(&mut self, size: [u32; 2]) {
        #[cfg(target_os = "android")]
        {
            let full = HudBatch::new(size, self.settings.gui_scale_inputs());
            let [x, y, _, _] = self.interface_insets(&full);
            let [old_x, old_y] = std::mem::replace(&mut self.ui.interface_origin, [x, y]);
            self.cursor.0 += old_x - x;
            self.cursor.1 += old_y - y;
        }
        #[cfg(not(target_os = "android"))]
        let _ = size;
    }

    /// Starling's Stats overlay, within the HUD's safe area. The frames' counter and graph are
    /// Sunrise's own layer now (sunrise-perf-overlay).
    pub(in super::super) fn draw_performance_overlays(&mut self, batch: &mut HudBatch) {
        let area = HudArea::begin(batch, self.hud_safe_insets(batch));
        self.draw_stats_overlay(batch);
        area.finish(batch);
    }

    pub(in super::super) fn hud_safe_insets(&self, batch: &HudBatch) -> [f32; 4] {
        #[cfg(target_os = "android")]
        {
            use sunrise_client::android_display::{DisplayOption as D, safe_zone};
            safe_zone(
                [batch.width, batch.height],
                [
                    D::SafeHorizontal.value(&self.settings),
                    D::SafeVertical.value(&self.settings),
                ],
            )
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = batch;
            [0.0; 4]
        }
    }

    #[cfg(target_os = "android")]
    pub(super) fn draw_safe_zone(
        &self,
        batch: &mut HudBatch,
        zone: sunrise_client::android_display::SafeZone,
    ) {
        use sunrise_client::android_display::SafeZone;
        let (margins, color) = match zone {
            SafeZone::Hud => (self.hud_safe_insets(batch), 0xffffffff),
            SafeZone::Interface => (
                self.menu_margins(batch, false),
                sunrise_client::hud::rgba(188, 171, 255, 255),
            ),
            SafeZone::Buttons => (
                self.button_safe_insets(batch),
                sunrise_client::hud::rgba(115, 225, 193, 255),
            ),
        };
        let [left, top, right, bottom] = margins;
        let [right, bottom] = [batch.width - right - 2.0, batch.height - bottom - 2.0];
        for (x, y, dx, dy) in [
            (left, top, 1.0, 1.0),
            (right, top, -1.0, 1.0),
            (left, bottom, 1.0, -1.0),
            (right, bottom, -1.0, -1.0),
        ] {
            for [x, y, w, h] in [
                [x.min(x + 12.0 * dx), y, 14.0, 2.0],
                [x, y.min(y + 12.0 * dy), 2.0, 14.0],
            ] {
                batch.fill(&self.gui.atlas, x + 1.0, y + 1.0, w, h, 0xff162026);
                batch.fill(&self.gui.atlas, x, y, w, h, color);
            }
        }
    }

    pub(in super::super) fn preview_display_safe_zone(&mut self, batch: &mut HudBatch) {
        #[cfg(target_os = "android")]
        if self.screen == Screen::Options {
            let [l, t, r, b] = self.interface_insets(batch);
            if let Some(zone) = self.ui.options.safe_zone_preview(
                &self.settings,
                (batch.width - l - r, batch.height - t - b),
                self.cursor,
            ) {
                self.draw_safe_zone(batch, zone);
            }
        }
        let _ = batch;
    }
}
