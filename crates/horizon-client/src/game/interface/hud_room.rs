//! The HUD's `Room` this frame (`docs/ui-kit.md`): what vanilla's HUD shows that Horizon's own
//! overlays keep clear of. `hud` empties it each frame and `hud_layers` fills it in before the
//! world's HUD draws, into `ui.room`; the controller notice, splitscreen's join strip and the
//! radial menu read it. With it, the frame's `motion::Clock` (`hud_clock`), and whether an
//! overlay holds the view (`overlay_holds_view`), which what vanilla draws there gives way to.
use super::{CREATIVE_SHIFT, Game, Screen};
use horizon_client::hud::HudBatch;
use horizon_client::hud_kit::motion::Clock;
use horizon_client::hud_kit::room::{Keep, Room};
use horizon_client::{boss_overlay, hud_layout, hud_overlays};

/// The HUD works in local safe-area coordinates. Move its vertices and item
/// scissors back into the full frame after drawing it.
pub(super) struct HudArea {
    full: [f32; 2],
    origin: [f32; 2],
    first_vertex: usize,
    first_blend: usize,
}

impl HudArea {
    pub(super) fn begin(batch: &mut HudBatch, [left, top, right, bottom]: [f32; 4]) -> Self {
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

    pub(super) fn finish(self, batch: &mut HudBatch) {
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
            use horizon_client::hud::Blend;
            if let Blend::Depth(rect) | Blend::DepthGlint(rect) = blend {
                rect[0] += (x * batch.scale) as u32;
                rect[1] += (y * batch.scale) as u32;
            }
        }
    }

    #[cfg(target_os = "android")]
    pub(super) fn local_point(&self, point: [f32; 2], scale: f32) -> [f32; 2] {
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

    pub(super) fn interface_area(&self, batch: &mut HudBatch) -> HudArea {
        let insets = self.interface_insets(batch);
        HudArea::begin(batch, insets)
    }

    pub(super) fn draw_interface_overlays(&mut self, batch: &mut HudBatch) {
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
        self.draw_kinetic_info(batch);

        let area = self.interface_area(batch);
        self.draw_chat_bar(batch);
        self.draw_command_prompt(batch);
        area.finish(batch);
    }

    fn interface_insets(&self, batch: &HudBatch) -> [f32; 4] {
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
    fn menu_margins(&self, batch: &HudBatch, editing: bool) -> [f32; 4] {
        use horizon_client::android_display::{DisplayOption as D, interface_safe_zone};
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
    pub(super) fn button_safe_insets(&self, batch: &HudBatch) -> [f32; 4] {
        use horizon_client::android_display::{DisplayOption as D, interface_safe_zone};
        interface_safe_zone(
            [batch.width, batch.height],
            [
                D::ButtonsHorizontal.value(&self.settings),
                D::ButtonsVertical.value(&self.settings),
            ],
        )
    }

    /// Rebase the pointer when the menu opens, closes, or changes size.
    pub(super) fn sync_interface_cursor(&mut self, size: [u32; 2]) {
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

    pub(in crate::game) fn interface_point(&self, p: [f32; 2], scale: f32) -> (f32, f32) {
        let [x, y] = self.interface_origin();
        (p[0] / scale - x, p[1] / scale - y)
    }

    pub(in crate::game) fn interface_window_point(
        &self,
        point: (f32, f32),
        scale: f32,
    ) -> winit::dpi::PhysicalPosition<f64> {
        let [x, y] = self.interface_origin();
        winit::dpi::PhysicalPosition::new(
            f64::from((point.0 + x) * scale),
            f64::from((point.1 + y) * scale),
        )
    }

    pub(in crate::game) fn interface_origin(&self) -> [f32; 2] {
        #[cfg(target_os = "android")]
        {
            self.ui.interface_origin
        }
        #[cfg(not(target_os = "android"))]
        {
            [0.0; 2]
        }
    }

    pub(super) fn draw_performance_overlays(&mut self, batch: &mut HudBatch) {
        let hud = self.hud_safe_insets(batch);
        #[cfg(target_os = "android")]
        if self.title.is_some() || self.screen != Screen::Playing {
            // The compact counter sits below Back in the utility buttons' layout.
            let area = HudArea::begin(batch, self.button_safe_insets(batch));
            self.frame_graph.draw_compact(
                self.settings.horizon.overlay,
                batch,
                &self.gui.atlas,
                25.0,
            );
            area.finish(batch);

            let area = HudArea::begin(batch, hud);
            self.draw_stats_overlay(batch);
            area.finish(batch);
            return;
        }
        let area = HudArea::begin(batch, hud);
        self.frame_graph
            .draw(self.settings.horizon.overlay, batch, &self.gui.atlas);
        self.draw_stats_overlay(batch);
        area.finish(batch);
    }

    pub(super) fn draw_hud_crosshair(
        &mut self,
        batch: &mut HudBatch,
        spectator: bool,
        indicator: u8,
        strength: f32,
        delay: f32,
    ) {
        let (wi, hi) = (batch.width as i32, batch.height as i32);
        // Hud.extractCrosshair / extractItemHotbar / extractPlayerHealth /
        // extractVehicleHealth and the experience bar, laid out by hud_layout.
        // Combat feel Legacy hides the attack indicator unless the player keeps it.
        // The 3D crosshair (F3) takes the 2D one's place.
        // Hud.extractCrosshair: a spectator only sees it over something to open.
        if self.camera_type.is_first_person()
            && (!spectator || self.spectator_crosshair())
            && !self.debug_crosshair()
            && !self.touch_hides_crosshair()
            && !self.overlay_holds_view()
        {
            let attack = (indicator == 1).then_some(strength);
            let full = attack.is_some_and(|a| {
                hud_layout::full_attack_indicator(a, delay, self.crosshair_living())
            });
            let draws = hud_layout::crosshair(wi, hi, attack, full);
            // Under one creative window it hides; Bedrock Inventory has a gap at the middle.
            if self.screen != Screen::Creative || CREATIVE_SHIFT.get() == 0.0 {
                self.gui_layer("minecraft:crosshair", batch, |g, batch| {
                    // The custom crosshair takes the sprite's place (the first draw); the
                    // attack indicator stays.
                    if g.custom_crosshair() {
                        g.draw_custom_crosshair(batch, wi, hi);
                        hud_layout::paint(batch, &g.gui.atlas, &draws[1..]);
                    } else {
                        hud_layout::paint(batch, &g.gui.atlas, &draws);
                    }
                });
            }
        }
    }

    pub(super) fn hud_safe_insets(&self, batch: &HudBatch) -> [f32; 4] {
        #[cfg(target_os = "android")]
        {
            use horizon_client::android_display::{DisplayOption as D, safe_zone};
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
    fn draw_safe_zone(
        &self,
        batch: &mut HudBatch,
        zone: horizon_client::android_display::SafeZone,
    ) {
        use horizon_client::android_display::SafeZone;
        let (margins, color) = match zone {
            SafeZone::Hud => (self.hud_safe_insets(batch), 0xffffffff),
            SafeZone::Interface => (
                self.menu_margins(batch, false),
                horizon_client::hud::rgba(188, 171, 255, 255),
            ),
            SafeZone::Buttons => (
                self.button_safe_insets(batch),
                horizon_client::hud::rgba(115, 225, 193, 255),
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
    pub(super) fn preview_display_safe_zone(&mut self, batch: &mut HudBatch) {
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
    /// The HUD's clock this frame (`hud_kit::motion`): still, so overlays show their motion
    /// settled, while the game is paused, frame-stepped (`HORIZON_STILL`, a screenshot
    /// sequence) or a repro recording or replay runs.
    pub(super) fn hud_clock(&self) -> Clock {
        let still = self.pause
            || self.clock.still()
            || self.screenshot_sequence.is_some()
            || self.repro.recording_active()
            || self.repro.replaying();
        Clock::new(std::time::Instant::now(), still)
    }

    /// What the HUD shows on a `w` x `h` GUI px view this frame, as keep-outs: the boss bars
    /// while the HUD shows (`show_hud`: not F1, not Replay Studio), as wide as the bar or its
    /// widest name.
    pub(super) fn hud_room(&self, w: f32, h: f32, show_hud: bool) -> Room {
        let mut room = Room::new(w, h);
        if !show_hud {
            return room;
        }
        let (wi, hi) = (w as i32, h as i32);
        let bars = self.boss_overlay.bars(boss_overlay::now_millis());
        let drawn = hud_overlays::boss_bars_drawn(hi, bars.len());
        if drawn > 0 {
            let font = &self.gui.atlas.font;
            let half = bars[..drawn]
                .iter()
                .map(|b| (hud_layout::width(font, &b.name.resolve_legacy()) + 1) / 2)
                .fold(hud_overlays::BOSS_BAR_WIDTH / 2, i32::max);
            let bottom = hud_overlays::boss_bars_bottom(hi, bars.len());
            room.add(
                Keep::BossBars,
                [
                    (wi / 2 - half) as f32,
                    0.0,
                    (2 * half) as f32,
                    bottom as f32,
                ],
            );
        }
        room
    }

    /// The status rows above the hotbar in a `w` x `h` GUI px view, from what they draw this
    /// frame (`draws`): a keep-out from their highest row's top down, as wide as the hotbar. Called
    /// as they are laid out, after `hud_room`, so the overlays drawn over the HUD see them.
    pub(super) fn keep_status_rows(&mut self, draws: &[hud_layout::GuiDraw], w: i32, h: i32) {
        if let Some(top) = hud_layout::draws_top(draws).filter(|top| *top < h) {
            let half = hud_overlays::HOTBAR_HALF_WIDTH;
            self.ui.room.add(
                Keep::StatusRows,
                [
                    (w / 2 - half) as f32,
                    top as f32,
                    (2 * half) as f32,
                    (h - top) as f32,
                ],
            );
        }
    }

    /// An overlay holds the middle of the view (the radial menu, once it shows: a quick tap of
    /// its opener draws nothing, so nothing blinks). The 2D and F3 crosshairs, the chat's lines,
    /// the toasts and the first-person hands give way to it.
    pub(super) fn overlay_holds_view(&self) -> bool {
        self.pad_radial_shown()
    }

    /// ToastManager.extractRenderState: F1 hides the toasts (`hidden`), and so does an overlay
    /// holding the view, as the radial menu's page tabs run across the top where the toasts are.
    /// They go on timing out meanwhile, as under F1.
    pub(super) fn draw_every_toast(&mut self, batch: &mut HudBatch, hidden: bool) {
        let hidden = hidden || self.overlay_holds_view();
        self.draw_toasts(batch, hidden);
        self.draw_tutorial_toasts(batch, hidden);
        self.draw_pack_toasts(batch, hidden);
        self.draw_music_toast(batch, hidden);
    }

    /// The hands drawn in first person this frame: none while an overlay holds the view, as the
    /// radial menu's tiles sit where they are.
    pub(super) fn hands_shown(&self) -> [bool; 2] {
        if self.overlay_holds_view() {
            [false; 2]
        } else {
            self.hands_to_render()
        }
    }
}
