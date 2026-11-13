//! The save screen freezes simulation, but keeps pumping window events while
//! snapshots are prepared in small batches and the region workers drain.
use super::{Game, loading_frame};
use horizon_client::{Result, hud::HudBatch, save::SaveDir};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) struct ClosingSave {
    pub quit: bool,
    close: bool,
    reply: Option<std::sync::mpsc::Sender<String>>,
    started: Instant,
    stage: Stage,
}
enum Stage {
    Generation,
    Chunks(VecDeque<(i32, i32)>),
    Entities(VecDeque<(i32, i32)>),
    Writes,
    Server,
    Quit,
}

// Bound the work in both sparse and block-entity-heavy worlds. A single chunk
// is indivisible; check the time budget after each snapshot, not each batch.
const SNAPSHOTS_PER_FRAME: usize = 16;
const SNAPSHOT_BUDGET: Duration = Duration::from_millis(2);

impl Game<'_> {
    /// Save the level's chunk tickets: `/forceload`'s in vanilla's `chunk_tickets.dat`, the
    /// mods' in Horizon's own `mod_chunk_tickets.dat`.
    pub(super) fn save_chunk_tickets(&self) -> Result<()> {
        self.save
            .save_chunk_tickets(self.streaming.commanded(), self.tickets.portal_tickets())?;
        self.save
            .save_mod_tickets(self.streaming.mod_tickets(), |slot| {
                mod_api::TicketController::from_slot(slot).map(|c| c.id())
            })
    }
    pub(super) fn begin_closing_save(&mut self, quit: bool) -> Result<()> {
        if let Some(save) = &mut self.pending_save {
            save.quit |= quit;
            return Ok(());
        }
        if self.title.is_some() || self.saved_for_exit {
            // A quit from the title still needs the event loop to finish.
            self.pending_save = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Quit,
            });
            return Ok(());
        }
        if self.integrated.is_some() {
            // Minecraft.disconnect: the client logs out before its server halts.
            self.client_logging_out();
        }
        // Stop client-side terrain work before the integrated server is shut down. The
        // server's own stop path aborts its workers, but the window's streaming registration
        // is dropped only after the event loop exits and would otherwise wait for queued work.
        self.streaming.abort_generation();
        if let Some(integrated) = &self.integrated {
            integrated.request_stop();
            self.pending_save = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Server,
            });
        } else if self.remote.is_some() {
            if !quit {
                return self.leave_server(None);
            }
            self.client_logging_out();
            if let Some(mut remote) = self.remote.take() {
                remote.session.disconnect();
            }
            self.saved_for_exit = true;
            self.pending_save = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Quit,
            });
        } else {
            self.close_container();
            self.award_custom_stat("leave_game", 1);
            self.lan_close(false);
            self.free_player_beds()?;
            self.pending_save = Some(ClosingSave {
                quit,
                close: true,
                reply: None,
                started: Instant::now(),
                stage: Stage::Generation,
            });
        }
        self.capture(false);
        Ok(())
    }

    /// The focus goes first when the game leaves the screen on a phone (home, the recent apps, a
    /// call), before the window, and the system may end the process from then on: the world is
    /// saved at once, as Bedrock does. Apart from `handle_window_event`: horizon-mixins refuses to
    /// hook a function with a `#[cfg]` in it (E_UNSUPPORTED_CFG).
    pub(super) fn save_when_focus_leaves(&mut self, event: &winit::event::WindowEvent) {
        if !cfg!(target_os = "android") {
            return;
        }
        if let winit::event::WindowEvent::Focused(focused) = event {
            if *focused {
                self.saved_for_background = false;
            } else {
                self.save_for_background();
            }
        }
    }

    /// The game left the screen (Android: the focus went, or the window), and from now on the
    /// system may end the process without a word: the world is saved where it stands, once each
    /// time the game goes, as Bedrock does, and the log is flushed. It is the autosave's save
    /// (vanilla's when the pause menu opens): the files go to the writer threads and nothing waits
    /// for them, neither the window nor Android's UI thread behind it (a full save waits for the
    /// generation in flight, seconds on a phone). The log says when they are written.
    pub(super) fn save_for_background(&mut self) {
        if !std::mem::replace(&mut self.saved_for_background, true)
            && self.title.is_none()
            && self.pending_save.is_none()
            && self.integrated_starting.is_none()
            && self.pending_entry.is_none()
        {
            if let Some(server) = &self.integrated {
                log::info!("Horizon: saving the world, the game left the screen");
                // Out of sight the world waits as saved; the next frame pauses it or not again.
                if !self.is_published() {
                    server.set_paused(true);
                }
                server.execute(|g| {
                    let started = Instant::now();
                    match g.save_game_with(false) {
                        Ok(()) => {
                            let mut saves = g.levels.parked_saves();
                            saves.push(g.save.clone());
                            log_when_written(saves, started);
                        }
                        Err(e) => log::warn!("Horizon: failed to save the world: {e}"),
                    }
                });
            } else if self.remote.is_none() {
                log::info!("Horizon: saving the world, the game left the screen");
                let started = Instant::now();
                match self.save_game_with(false) {
                    Ok(()) => log_when_written(vec![self.save.clone()], started),
                    Err(e) => log::warn!("Horizon: failed to save the world: {e}"),
                }
            }
        }
        horizon_client::logging::flush();
    }

    /// Control saves acknowledge only after durable completion, while their
    /// snapshot work is spread across event-loop turns too.
    pub(super) fn begin_control_save(
        &mut self,
        all: bool,
        reply: std::sync::mpsc::Sender<String>,
    ) -> Result<()> {
        if self.pending_save.is_some() {
            return Err("a world save is already in progress".into());
        }
        if all {
            self.world
                .modified
                .extend(self.world.chunks.keys().copied());
        }
        self.keys.clear();
        self.physical_keys.clear();
        self.left = false;
        self.right = false;
        self.pending_save = Some(ClosingSave {
            quit: false,
            close: false,
            reply: Some(reply),
            started: Instant::now(),
            stage: Stage::Generation,
        });
        Ok(())
    }

    /// `true` means the window can exit. The session lock stays in Game until
    /// every snapshot has been queued and every writer has synced its files.
    pub(super) fn poll_closing_save(&mut self) -> Result<bool> {
        let Some(mut save) = self.pending_save.take() else {
            return Ok(false);
        };
        let result = (|| -> Result<bool> {
            match &mut save.stage {
                Stage::Quit => Ok(true),
                Stage::Server => {
                    let server = self
                        .integrated
                        .as_ref()
                        .ok_or("the integrated server stopped with an error")?;
                    if !server.stopped() {
                        return Ok(false);
                    }
                    // Drop/join only after completion; leave_server's teardown
                    // can then run without blocking on the server's writes.
                    if save.quit {
                        self.integrated.take().unwrap().stop()?;
                    } else {
                        self.leave_server(None)?;
                    }
                    Ok(true)
                }
                Stage::Generation => {
                    if save.started.elapsed() > Duration::from_secs(30) {
                        return Err("chunk generation did not finish before saving".into());
                    }
                    let deadline = Instant::now() + Duration::from_millis(2);
                    loop {
                        match self.streaming.prepare_poll_save(&mut self.world, deadline) {
                            horizon_client::streaming::SaveStep::Pending => break,
                            horizon_client::streaming::SaveStep::Full(chunk) => {
                                self.native_late_full_chunk_snapshot(&chunk);
                                self.streaming.save_finished(chunk);
                            }
                            horizon_client::streaming::SaveStep::Protos(protos) => {
                                self.native_proto_snapshots(&protos);
                                if self.streaming.submit_protos(protos) {
                                    save.stage = Stage::Chunks(
                                        self.world.modified.iter().copied().collect(),
                                    );
                                }
                                break;
                            }
                        }
                    }
                    Ok(false)
                }
                Stage::Chunks(keys) => {
                    let started = Instant::now();
                    for _ in 0..SNAPSHOTS_PER_FRAME {
                        let Some(&key) = keys.front() else { break };
                        let chunks = self.with_entities(self.world.chunks.get(&key));
                        self.native_full_chunk_snapshots(&chunks);
                        self.save.save_later(
                            &chunks,
                            self.pack.assets.registry,
                            world_biomes!(self),
                        )?;
                        self.world.modified.remove(&key);
                        keys.pop_front();
                        if started.elapsed() >= SNAPSHOT_BUDGET {
                            break;
                        }
                    }
                    if keys.is_empty() {
                        self.spawn_loaded_entities();
                        save.stage = Stage::Entities(self.world.chunks.keys().copied().collect());
                    }
                    Ok(false)
                }
                Stage::Entities(keys) => {
                    let started = Instant::now();
                    for _ in 0..SNAPSHOTS_PER_FRAME {
                        let Some(&key) = keys.front() else { break };
                        let chunks = self.entity_chunks(&HashSet::from([key]), false);
                        self.save.save_entities_later(chunks)?;
                        keys.pop_front();
                        if started.elapsed() >= SNAPSHOT_BUDGET {
                            break;
                        }
                    }
                    if keys.is_empty() {
                        self.prune_entity_memory();
                        self.save_raids();
                        self.save.save_attachments(&self.world.attachments)?;
                        self.save_chunk_tickets()?;
                        self.save_game_metadata()?;
                        save.stage = Stage::Writes;
                    }
                    Ok(false)
                }
                Stage::Writes => {
                    let players = self.finish_player_autosaves(false)?;
                    Ok(self.save.writes_finished()? && players)
                }
            }
        })();
        match result {
            Ok(true) => {
                if let Some(reply) = save.reply.take() {
                    let _ = reply.send(serde_json::json!({ "ok": true }).to_string());
                }
                if save.quit && !save.close {
                    // A window close during a control save still has to close
                    // containers and save the resulting inventory changes.
                    self.begin_closing_save(true)?;
                    return Ok(false);
                }
                if save.quit {
                    self.saved_for_exit = true;
                    Ok(true)
                } else {
                    if save.close && self.title.is_none() {
                        self.finish_perf();
                        self.finish_close_world()?;
                    }
                    self.clock.reset();
                    self.previous = Instant::now();
                    Ok(false)
                }
            }
            other => {
                if let Err(error) = &other
                    && let Some(reply) = save.reply.take()
                {
                    let _ =
                        reply.send(serde_json::json!({ "error": error.to_string() }).to_string());
                }
                // Keep ownership (and the lock) on errors too. Native shutdown
                // sees the same durable-write failure instead of declaring success.
                self.pending_save = Some(save);
                other
            }
        }
    }

    pub(super) fn render_closing_save(&mut self) -> Result<()> {
        if matches!(
            self.pending_save.as_ref().map(|save| &save.stage),
            Some(Stage::Quit)
        ) {
            return Ok(());
        }
        // disconnectWithSavingScreen: a GenericMessageScreen, its title and
        // no navigation.
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new(
                "saving",
                super::screen_info::tr("menu.savingLevel", &[]),
            )
            .without_navigation()
        });
        let Some(renderer) = &mut self.renderer else {
            return Ok(());
        };
        let mut hud = HudBatch::new(
            [renderer.config.width, renderer.config.height],
            self.settings.gui_scale_inputs(),
        );
        let progress = match self.pending_save.as_ref().map(|save| &save.stage) {
            Some(Stage::Chunks(_)) => 0.25,
            Some(Stage::Entities(_)) => 0.5,
            Some(Stage::Writes) => 0.9,
            _ => 0.0,
        };
        horizon_client::level_loading::draw_progress(
            &mut hud,
            &self.gui.atlas,
            "Saving world...",
            progress,
        );
        renderer.render(&loading_frame(&hud))?;
        Ok(())
    }

    /// `GameRenderer.tryTakeScreenshotIfNeeded`: while a local world without a
    /// saved icon is on the screen, grab the rendered frame once a second until
    /// the save has one. The Select World list shows it as the world's preview.
    pub(super) fn tick_world_icon(&mut self) {
        // Its grab would take a sequence's frame.
        if self.screenshot_sequence.is_some() {
            return;
        }
        let Some(icon) = self.world_icon.as_ref() else {
            return;
        };
        if icon
            .last_attempt
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        if icon.path.is_file() {
            // tryTakeScreenshotIfNeeded's hasWorldScreenshot: it has one, stop asking.
            self.world_icon = None;
            return;
        }
        // takeAutoScreenshot only saves with every loaded section rendered, or
        // the icon would picture the holes the streamer has not filled yet.
        if self.streaming.pending() != 0
            || !self.pending_uploads.is_empty()
            || self.remote_backlog() != 0
        {
            return;
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        // countRenderedSections() > 10: a client that joined its server
        // thread has nothing pending before the first chunks arrive, and the
        // icon would be only sky.
        if renderer.drawn_sections <= 10 {
            return;
        }
        if renderer.screenshot.is_some() {
            // F2's or an agent's screenshot owns the frame this time; ours
            // comes around again with the next attempt.
            return;
        }
        let path = icon.path.clone();
        if let Some(icon) = self.world_icon.as_mut() {
            icon.last_attempt = Some(Instant::now());
        }
        renderer.screenshot = Some(horizon_client::gfx::ScreenshotRequest {
            path,
            size: Some([64, 64]),
            scaled: false,
            background: true,
        });
    }
}

/// Logs, from a thread of its own, once the writer threads have put a save that did not wait on
/// disk: in a bug report it shows whether the process ended before.
fn log_when_written(saves: Vec<Arc<SaveDir>>, started: Instant) {
    let waiter = std::thread::Builder::new()
        .name("Save waiter".into())
        .spawn(move || {
            let written = saves.iter().try_for_each(|save| save.flush());
            horizon_client::save::later::flush();
            match written {
                Ok(()) => log::info!(
                    "Horizon: the world is saved ({:.2} s)",
                    started.elapsed().as_secs_f32()
                ),
                Err(e) => log::warn!("Horizon: failed to save the world: {e}"),
            }
            horizon_client::logging::flush();
        });
    if let Err(e) = waiter {
        log::warn!("Horizon: could not wait for the save: {e}");
    }
}

impl Game<'_> {
    /// Save the level in place: its changed chunks, its entities and its raids.
    pub(super) fn save_level(&mut self, wait: bool) -> Result<()> {
        self.save_border();
        self.save.save_attachments(&self.world.attachments)?;
        self.save
            .save_chunk_tickets(&self.streaming.forced, self.tickets.portal_tickets())?;
        self.streaming.save(&mut self.world, wait);
        let now = Instant::now();
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks = self.with_entities(keys.iter().filter_map(|k| self.world.chunks.get(k)));
        self.save
            .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))?;
        // Prepare entities while the chunk writer runs, then use one barrier.
        if !wait {
            self.world.modified.autosaved(&keys, now);
        }
        self.save_entities(false)?;
        self.save_raids();
        if wait {
            self.save.flush()?;
        }
        Ok(())
    }
}
