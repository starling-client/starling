//! The subtitle overlay in the game (`Hud` -> `SubtitleOverlay`): the sound
//! engine reports the sounds it plays while the overlay listens, the HUD
//! renders it (which starts or stops the listening as the "Show Subtitles"
//! option says).
use super::Game;
use std::cell::{Cell, RefCell};
use sunrise_client::{
    hud::HudBatch,
    hud_layout,
    hud_overlays::{SubtitleFrame, SubtitleOverlay, now_ms},
};

#[derive(Default)]
pub(super) struct SubtitleState {
    overlay: RefCell<SubtitleOverlay>,
    pub(super) deferred: Cell<bool>,
}
impl Game<'_> {
    /// Feed the sounds the engine reported since the last tick
    /// (`SoundEventListener.onPlaySound`) while the overlay listens.
    pub(super) fn tick_subtitles(&mut self) {
        let mut overlay = self.subtitles.overlay.borrow_mut();
        let listening = overlay.listening();
        self.audio.record_heard = listening;
        let heard = self.audio.take_heard();
        if !listening {
            return;
        }
        let now = now_ms();
        for h in heard {
            let at = h.at.unwrap_or(self.audio.listener);
            overlay.on_play(Some(&h.subtitle), at, h.range, h.looping, now);
        }
    }
    /// Consume a paint scheduled by the original subtitle layer, without dispatching it again.
    pub(super) fn draw_deferred_subtitles(&self, batch: &mut HudBatch) {
        if self.subtitles.deferred.take() {
            self.draw_subtitles(batch);
        }
    }

    pub(super) fn draw_subtitles(&self, batch: &mut HudBatch) {
        let v = &self.view;
        let frame = SubtitleFrame {
            show: self.settings.show_subtitles,
            position: v.position,
            forward: v.forwards.map(f64::from),
            up: v.up.map(f64::from),
            multiplier: self.settings.notification_display_time,
            background: self.settings.text_background(0.8),
            width: batch.width as i32,
            height: batch.height as i32,
        };
        let atlas = &self.gui.atlas;
        let draws = self
            .subtitles
            .overlay
            .borrow_mut()
            .extract(&frame, &atlas.font, now_ms());
        hud_layout::paint(batch, atlas, &draws);
    }
}
