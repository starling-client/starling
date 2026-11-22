//! The save screen freezes simulation, but keeps pumping window events while
//! snapshots are prepared in small batches and the region workers drain.
use super::{Game, loading_frame};
use std::{
    collections::{HashSet, VecDeque},
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};
use sunrise_client::{Result, hud::HudBatch};

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
    Writes(Option<Receiver<std::io::Result<()>>>),
    Server,
    Quit,
}

// Bound the work in both sparse and block-entity-heavy worlds. A single chunk
// is indivisible; check the time budget after each snapshot, not each batch.
const SNAPSHOTS_PER_FRAME: usize = 16;
const SNAPSHOT_BUDGET: Duration = Duration::from_millis(2);
const CLOSING_SNAPSHOTS_PER_FRAME: usize = 256;
const CLOSING_SNAPSHOT_BUDGET: Duration = Duration::from_millis(10);

impl Game<'_> {
    /// Save the level's chunk tickets: `/forceload`'s in vanilla's `chunk_tickets.dat`, the
    /// mods' in Sunrise's own `mod_chunk_tickets.dat`.
    pub(super) fn save_chunk_tickets(&self) -> Result<()> {
        if self.startup_ticket_restore_pending {
            return Ok(());
        }
        self.save
            .save_chunk_tickets(self.streaming.commanded(), self.tickets.portal_tickets())?;
        self.save
            .save_mod_tickets(self.streaming.mod_tickets(), |slot| {
                mod_api::TicketController::from_slot(slot).map(|c| c.id())
            })
    }
    pub(super) fn begin_closing_save(&mut self, quit: bool) -> Result<()> {
        if quit {
            self.close_client_tasks();
        }
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
            self.connection_left(sunrise_hooks::connection::Left::Quit);
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
        self.clear_device_keys();
        self.physical_keys.clear();
        self.set_mapping_held("key.attack", false);
        self.set_mapping_held("key.use", false);
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
        let defers = self.pending_save.as_ref().is_some_and(|save| {
            !matches!(save.stage, Stage::Server | Stage::Quit)
                && save.started.elapsed() < Duration::from_secs(30)
        });
        if defers && self.defer_chunk_disk_save() {
            return Ok(false);
        }
        let Some(mut save) = self.pending_save.take() else {
            return Ok(false);
        };
        let (per_frame, budget) = if save.close {
            (CLOSING_SNAPSHOTS_PER_FRAME, CLOSING_SNAPSHOT_BUDGET)
        } else {
            (SNAPSHOTS_PER_FRAME, SNAPSHOT_BUDGET)
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
                    let deadline = Instant::now() + budget;
                    loop {
                        match self.streaming.prepare_poll_save(&mut self.world, deadline) {
                            sunrise_client::streaming::SaveStep::Pending => break,
                            sunrise_client::streaming::SaveStep::Full(chunk) => {
                                self.native_late_full_chunk_snapshot(&chunk);
                                self.streaming.save_finished(chunk);
                            }
                            sunrise_client::streaming::SaveStep::Protos(protos) => {
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
                    for _ in 0..per_frame {
                        let Some(&key) = keys.front() else { break };
                        // ChunkMap.save consumes the dirty mark before capturing the snapshot.
                        self.world.modified.remove(&key);
                        let chunks = self.with_entities(self.world.chunks.get(&key));
                        self.native_full_chunk_snapshots(&chunks);
                        self.save.save_later(
                            &chunks,
                            self.pack.assets.registry,
                            world_biomes!(self),
                        )?;
                        keys.pop_front();
                        if started.elapsed() >= budget {
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
                    for _ in 0..per_frame {
                        let Some(&key) = keys.front() else { break };
                        let chunks = self.entity_chunks(&HashSet::from([key]), false);
                        self.save.save_entities_later(chunks)?;
                        keys.pop_front();
                        if started.elapsed() >= budget {
                            break;
                        }
                    }
                    if keys.is_empty() {
                        self.prune_entity_memory();
                        self.save_raids();
                        self.save.save_attachments(&self.world.attachments)?;
                        self.save_chunk_tickets()?;
                        self.save_game_metadata()?;
                        save.stage = Stage::Writes(Some(self.save.flush_later()?));
                    }
                    Ok(false)
                }
                Stage::Writes(flush) => {
                    let Some(receive) = flush else {
                        *flush = Some(self.save.flush_later()?);
                        return Ok(false);
                    };
                    if !self.finish_player_autosaves(false)? {
                        return Ok(false);
                    }
                    let result = match receive.try_recv() {
                        Ok(result) => result,
                        Err(TryRecvError::Empty) => return Ok(false),
                        Err(TryRecvError::Disconnected) => {
                            Err(std::io::Error::other("the region writer stopped"))
                        }
                    };
                    // A later shutdown barrier must force the files again if this one failed.
                    *flush = None;
                    result?;
                    if !save.close {
                        self.message = Some((
                            sunrise_client::lang::get("commands.save.success"),
                            Instant::now(),
                        ));
                    }
                    Ok(true)
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
                // Keep ownership (and the lock) on errors too. Native shutdown must
                // finish a durable barrier before it can declare the save completed.
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
        let waiting = matches!(
            self.pending_save.as_ref().map(|save| &save.stage),
            Some(Stage::Server | Stage::Writes(_))
        );
        if waiting {
            thread_local! {
                static LAST_FRAME: std::cell::Cell<Option<Instant>> =
                    const { std::cell::Cell::new(None) };
            }
            const FRAME: Duration = Duration::from_millis(33);
            if let Some(last) = LAST_FRAME.with(|c| c.get()) {
                let since = last.elapsed();
                if since < FRAME {
                    std::thread::sleep(FRAME - since);
                }
            }
            LAST_FRAME.with(|c| c.set(Some(Instant::now())));
        }
        // disconnectWithSavingScreen: a GenericMessageScreen, its title and
        // no navigation.
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new(
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
            Some(Stage::Writes(_)) => 0.9,
            _ => 0.0,
        };
        sunrise_client::level_loading::draw_progress(
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
        renderer.screenshot = Some(sunrise_client::gfx::ScreenshotRequest {
            path,
            size: Some([64, 64]),
            scaled: false,
            background: true,
        });
    }
}

impl Game<'_> {
    /// Save the level in place: its changed chunks, its entities and its raids.
    pub(super) fn save_level(&mut self, wait: bool) -> Result<()> {
        self.admit_level_save()?;
        // No save holds a player vehicle: a mob a mod seated on a player gets off first.
        self.eject_player_riders(None);
        self.save_border();
        self.save_chunk_tickets()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.streaming.prepare_save(&mut self.world, wait, deadline) {
                sunrise_client::streaming::SaveStep::Full(chunk) => {
                    self.native_late_full_chunk_snapshot(&chunk);
                    self.streaming.save_finished(chunk);
                }
                sunrise_client::streaming::SaveStep::Protos(protos) => {
                    self.native_proto_snapshots(&protos);
                    self.streaming.finish_save(protos, wait);
                    break;
                }
                sunrise_client::streaming::SaveStep::Pending => unreachable!(),
            }
        }
        let keys: Vec<_> = self.world.modified.iter().copied().collect();
        self.save.save_attachments(&self.world.attachments)?;
        // ChunkMap.save calls tryMarkSaved before creating the snapshot. A record failure
        // finishes this attempt and does not put the mark back.
        for key in &keys {
            self.world.modified.remove(key);
        }
        let chunks = self.with_entities(keys.iter().filter_map(|k| self.world.chunks.get(k)));
        self.native_full_chunk_snapshots(&chunks);
        self.save
            .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))?;
        // Prepare entities while the chunk writer runs, then use one barrier.
        if !wait {
            self.world.modified.autosaved(&keys, Instant::now());
        }
        self.save_entities(false)?;
        self.save_raids();
        if wait {
            self.save.flush()?;
        }
        if mod_api::__private::listened(mod_api::HookId::LevelSave) {
            self.native_level_save(self.dim.id());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use std::sync::{Arc, mpsc};
    use sunrise_client::save::{LevelShape, SaveDir};
    use sunrise_core::nbt::{DISK_WRITE_NESTING_GUARD, Tag};

    fn finish_staged_save(game: &mut Game<'_>) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let quit = game.poll_closing_save().unwrap();
            if game.pending_save.is_none() {
                return quit;
            }
            assert!(
                Instant::now() < deadline,
                "the region writer never acknowledged the save"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn both_successful_save_paths_resolve_the_banner_in_the_active_language() {
        use sunrise_client::lang::{Language, scoped};

        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let dir = tempfile::tempdir().unwrap();
                game.world_root = dir.path().to_path_buf();
                game.save = Arc::new(SaveDir::new(
                    game.world_root.clone(),
                    LevelShape::OVERWORLD,
                    Some(Default::default()),
                ));
                let translated = "[test locale]";
                let language = Language::of(
                    "en_test",
                    [("commands.save.success".into(), translated.into())],
                );
                scoped(language, || {
                    game.save_game().unwrap();
                    let explicit = game.message.take().unwrap().0;
                    game.pending_save = Some(ClosingSave {
                        quit: false,
                        close: false,
                        reply: None,
                        started: Instant::now(),
                        stage: Stage::Writes(Some(game.save.flush_later().unwrap())),
                    });
                    assert!(!finish_staged_save(game));
                    assert!(game.pending_save.is_none());
                    let incremental = game.message.take().unwrap().0;
                    assert_eq!(
                        (explicit.as_str(), incremental.as_str()),
                        (translated, translated)
                    );
                });
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_and_control_saves_complete_after_reporting_a_record_failure() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                for quit in [false, true] {
                    let dir = tempfile::tempdir().unwrap();
                    game.save = Arc::new(SaveDir::new(
                        dir.path().to_path_buf(),
                        LevelShape::OVERWORLD,
                        None,
                    ));
                    let invalid = (0..=DISK_WRITE_NESTING_GUARD)
                        .fold(Tag::Int(0), |nested, _| Tag::List(vec![nested]));
                    game.save
                        .save_entities(vec![((2, 0), vec![invalid])])
                        .unwrap();
                    assert_eq!(game.save.record_save_failures(), 1);
                    let (send, receive) = mpsc::channel();
                    game.pending_save = Some(ClosingSave {
                        quit,
                        close: quit,
                        reply: (!quit).then_some(send),
                        started: Instant::now(),
                        stage: Stage::Writes(Some(game.save.flush_later().unwrap())),
                    });
                    game.saved_for_exit = false;
                    game.message = None;
                    assert_eq!(finish_staged_save(game), quit);
                    assert!(game.pending_save.is_none());
                    if !quit {
                        let reply: serde_json::Value =
                            serde_json::from_str(&receive.try_recv().unwrap()).unwrap();
                        assert_eq!(reply, serde_json::json!({"ok": true}));
                    }
                    assert_eq!(game.saved_for_exit, quit);
                    assert_eq!(game.message.is_some(), !quit);
                }
            });
        });
    }
}

impl Game<'_> {
    /// Starling: the focus goes first when the game leaves the screen on a phone (home, the recent
    /// apps, a call), before the window, and the system may end the process from then on: the
    /// world is saved at once, as Bedrock does.
    pub(super) fn save_when_focus_leaves(&mut self, event: &winit::event::WindowEvent) {
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
    pub(super) fn save_for_background(&mut self) {
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
                server.tell(super::owner_link::OwnerRequest::SaveForBackground);
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
    pub(super) fn save_for_background_here(&mut self) {
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
fn log_when_written(saves: Vec<std::sync::Arc<sunrise_client::save::SaveDir>>, started: Instant) {
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
