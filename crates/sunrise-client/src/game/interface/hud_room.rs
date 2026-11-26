//! The HUD's `Room` this frame (`docs/ui-kit.md`): what vanilla's HUD shows that Sunrise's own
//! overlays keep clear of. `hud` empties it each frame and `hud_layers` fills it in before the
//! world's HUD draws, into `ui.room`; the controller notice and radial menu read it. With it, the frame's `motion::Clock` (`hud_clock`), and whether an
//! overlay holds the view (`overlay_holds_view`), which what vanilla draws there gives way to.
use super::{Game, Screen};
use sunrise_client::hud::{HudBatch, HudVertex};
use sunrise_client::hud_kit::motion::Clock;
use sunrise_client::hud_kit::room::{Keep, Room};
use sunrise_client::{boss_overlay, hud_layout, hud_overlays};
#[path = "hud_room/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;

impl Game<'_> {
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

    /// The HUD's clock this frame (`hud_kit::motion`): still, so overlays show their motion
    /// settled, while the game is paused, frame-stepped (`SUNRISE_STILL`, a screenshot
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

    /// Reserve the highest painted status row, vanilla or replaced, down to the bottom
    /// of the view. Measure only the installed body's paint, not the surrounding hook draws.
    pub(super) fn keep_status_rows(&mut self, vertices: &[HudVertex], w: f32, h: f32) {
        let top = vertices.iter().map(|v| v.pos[1]).fold(h, f32::min);
        if top < self.ui.room.bottom() {
            let half = hud_overlays::HOTBAR_HALF_WIDTH;
            self.ui.room.add(
                Keep::StatusRows,
                [
                    ((w as i32) / 2 - half) as f32,
                    top,
                    (2 * half) as f32,
                    h - top,
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

    /// ToastManager.extractRenderState: F1 and overlays hide ordinary toasts without pausing them.
    /// A mod's HUD veto hides ordinary toasts, not mandatory diagnostics.
    pub(super) fn draw_every_toast(
        &mut self,
        batch: &mut HudBatch,
        replay_playing: bool,
        gui_canceled: bool,
    ) {
        let overlay_or_replay = replay_playing || self.overlay_holds_view();
        let diagnostic_hidden = overlay_or_replay;
        let hidden = overlay_or_replay || self.ui.hide_gui || gui_canceled;
        self.draw_toasts(batch, hidden);
        self.draw_tutorial_toasts(batch, hidden);
        self.draw_pack_toasts(batch, hidden, diagnostic_hidden);
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
