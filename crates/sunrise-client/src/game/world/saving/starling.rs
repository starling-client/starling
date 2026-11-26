//! Starling's part of saving.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Game<'_> {
    /// Starling: the focus goes first when the game leaves the screen on a phone (home, the recent
    /// apps, a call), before the window, and the system may end the process from then on: the
    /// world is saved at once, as Bedrock does.
    pub(in super::super) fn save_when_focus_leaves(&mut self, event: &winit::event::WindowEvent) {
        if !cfg!(target_os = "android") {
            return;
        }
        if let winit::event::WindowEvent::Focused(focused) = event {
            // Out of focus but still on screen (a floating app over it, the recent apps, the
            // notification shade), the window keeps its surface and `suspended` never comes: the
            // sound goes with the focus, as the game pauses with it (Options' "Pause in the
            // Background", vanilla's pauseOnLostFocus). Turned off, the game plays on, heard.
            let pauses = self.settings.file.bool("pauseOnLostFocus");
            if *focused {
                self.saved_for_background = false;
                self.audio.resume_output();
            } else {
                self.save_for_background();
                if pauses {
                    self.audio.suspend_output();
                }
            }
        }
    }

    /// Starling: the game left the screen (Android: the focus went, or the window), and from now
    /// on the system may end the process without a word: the world is saved where it stands, once
    /// each time the game goes, as Bedrock does, and the log is flushed. It is the autosave's
    /// save (vanilla's when the pause menu opens): the files go to the writer threads and nothing
    /// waits for them, neither the window nor Android's UI thread behind it. The log says when
    /// they are written.
    pub(in super::super) fn save_for_background(&mut self) {
        if !std::mem::replace(&mut self.saved_for_background, true)
            && self.title.is_none()
            && self.pending_save.is_none()
            && self.integrated_starting.is_none()
            && self.pending_entry.is_none()
        {
            if let Some(server) = &self.integrated {
                log::info!("Sunrise: saving the world, the game left the screen");
                // Out of sight the world waits as saved; the next frame pauses it or not again.
                if !self.is_published() {
                    server.set_paused(true);
                }
                server.tell(super::super::owner_link::OwnerRequest::SaveForBackground);
            } else if self.remote.is_none() {
                log::info!("Sunrise: saving the world, the game left the screen");
                let started = Instant::now();
                match self.save_game_with(false) {
                    Ok(()) => log_when_written(vec![self.save.clone()], started),
                    Err(e) => log::warn!("Sunrise: failed to save the world: {e}"),
                }
            }
        }
        sunrise_client::logging::flush();
    }

    /// `save_for_background` on the server thread, between two ticks.
    pub(in super::super) fn save_for_background_here(&mut self) {
        let started = Instant::now();
        match self.save_game_with(false) {
            Ok(()) => {
                let mut saves = self.levels.parked_saves();
                saves.push(self.save.clone());
                log_when_written(saves, started);
            }
            Err(e) => log::warn!("Sunrise: failed to save the world: {e}"),
        }
    }
}

/// Starling: says in the log when a background save's files are written, from a thread of its own.
pub(super) fn log_when_written(
    saves: Vec<std::sync::Arc<sunrise_client::save::SaveDir>>,
    started: Instant,
) {
    let waiter = std::thread::Builder::new()
        .name("Save waiter".into())
        .spawn(move || {
            let written = saves.iter().try_for_each(|save| save.flush());
            sunrise_client::save::later::flush();
            match written {
                Ok(()) => log::info!(
                    "Sunrise: the world is saved ({:.2} s)",
                    started.elapsed().as_secs_f32()
                ),
                Err(e) => log::warn!("Sunrise: failed to save the world: {e}"),
            }
            sunrise_client::logging::flush();
        });
    if let Err(e) = waiter {
        log::warn!("Sunrise: could not wait for the save: {e}");
    }
}
