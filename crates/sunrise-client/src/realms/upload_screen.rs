//! The upload screens: `RealmsSelectFileToUploadScreen` (`UploadSelect`) listing the saves
//! folder with its search box, `RealmsUploadScreen` (`Uploading`) packing and sending the
//! chosen world, and `RealmsCreateWorldFlow`'s `AlertScreen` (`CreateWorldUpload`) over the
//! world it generates first.
//!
//! `realms.rs` keeps the `SubScreen` variants and one call per site; the arms and the
//! methods that are the upload screens' are here.

use super::client::{RealmServer, RealmSetting, RealmSlot, RealmWorldOptions};
use super::{
    Outcome, REALMS_UPLOAD_SEARCH, RED, ROW_HEIGHT, ResetKind, Screen, SubScreen, centered, client,
    human_readable, icon_sprite, percent_label, rgba, spawn, take, transfer_bar_width, upload,
    upload_limit_size, upload_matches,
};
use crate::hud::{HudAtlas, HudBatch};
use std::sync::Arc;
use std::time::Instant;

impl Screen {
    /// `RealmsSelectFileToUploadScreen`'s Enter (`WorldListEntry.keyPressed`): the selected
    /// row uploads. `None` for any other screen.
    pub(super) fn activate_upload_select(&mut self) -> Option<Outcome> {
        if matches!(self.subscreen, SubScreen::UploadSelect { .. }) {
            self.upload_selected();
            Some(Outcome::None)
        } else {
            None
        }
    }

    /// `RealmsSelectWorldTemplateScreen.selectTemplate`: the minigame task, or the reset.
    /// `RealmsSelectFileToUploadScreen.upload`: the selected row of the filtered list
    /// opens the upload screen (the button and a row's Enter both run it).
    fn upload_selected(&mut self) {
        let SubScreen::UploadSelect {
            server,
            slot,
            creation,
            worlds,
            selected,
            query,
            ..
        } = &self.subscreen
        else {
            return;
        };
        let Some(world) = selected
            .and_then(|i| {
                worlds
                    .iter()
                    .filter(|world| upload_matches(world, query))
                    .nth(i)
            })
            .cloned()
        else {
            return;
        };
        let (server, slot) = (server.clone(), *slot);
        let creation = creation.clone();
        self.start_upload(server, slot, world, creation);
    }

    /// `new RealmsSelectFileToUploadScreen`: the game's own worlds (`WorldSelectionList`), as the
    /// title screen lists them.
    pub(super) fn open_upload_select(
        &mut self,
        server: RealmServer,
        slot: i32,
        kind: ResetKind,
        creation: Option<(String, String)>,
    ) {
        let worlds = self
            .saves
            .as_deref()
            .map(crate::title::list_worlds)
            .unwrap_or_default();
        self.subscreen = SubScreen::UploadSelect {
            server,
            slot,
            kind,
            creation,
            worlds,
            selected: None,
            scroll: crate::ui::Scroll::default(),
            query: String::new(),
        };
    }

    /// `RealmsUploadScreen`: the local world's folder on a worker, packed and posted.
    fn start_upload(
        &mut self,
        server: RealmServer,
        slot: i32,
        world: crate::title::WorldEntry,
        creation: Option<(String, String)>,
    ) {
        let Some(saves) = self.saves.clone() else {
            return;
        };
        let progress = Arc::new(upload::UploadProgress::default());
        let job = client().map(|client| {
            let progress = Arc::clone(&progress);
            let active = server.active_slot;
            let options =
                RealmWorldOptions {
                    // `RealmsWorldOptions.createFromSettings`: the world's own settings, its saved
                    // version as the world version.
                    spawn_protection: 0,
                    force_game_mode: false,
                    difficulty: crate::save::Level::read_summary(&saves.join(&world.folder))
                        .map_or(2, |level| {
                            crate::title::DIFFICULTIES
                                .iter()
                                .position(|d| *d == level.difficulty)
                                .unwrap_or(2) as i32
                        }),
                    game_mode: world.game_mode,
                    slot_name: world.name.clone(),
                    version: world.version.clone(),
                    compatibility: "UNVERIFIABLE".into(),
                    template_id: -1,
                    template_image: None,
                    empty: false,
                };
            let realm_slot = RealmSlot {
                slot_id: slot,
                options,
                settings: vec![RealmSetting {
                    name: "hardcore".into(),
                    value: world.hardcore.to_string(),
                }],
            };
            let world_dir = saves.join(&world.folder);
            spawn(move || {
                // `RealmsUploadScreen.init`'s tasks: `RealmCreationTask` (initializeRealm) runs
                // first, then the slot switch, then the upload itself.
                if let Some((name, description)) = creation.as_ref() {
                    client
                        .initialize_realm(server.id, name, description)
                        .map_err(|e| upload::UploadError::Failed(e.text()))?;
                }
                // `RealmsUploadScreen.init`: `SwitchSlotTask` first, when the slot isn't active.
                if slot != active {
                    client
                        .switch_slot(server.id, slot)
                        .map_err(|e| upload::UploadError::Failed(e.text()))?;
                }
                upload::upload_world(&client, server.id, &realm_slot, &world_dir, &progress)
            })
        });
        self.subscreen = SubScreen::Uploading {
            server,
            slot,
            world,
            progress,
            job,
            error: None,
            done: false,
            started: Instant::now(),
        };
    }

    /// `RealmsUploadScreen`'s answer (Done, or the error it keeps on screen) and
    /// `RealmsCreateWorldFlow.createWorld`'s (the screen it returns to). `Some` once a screen
    /// took its answer.
    pub(super) fn poll_upload_screens(&mut self) -> Option<Outcome> {
        match &mut self.subscreen {
            SubScreen::Uploading {
                job, error, done, ..
            } => {
                if let Some(result) = job.as_ref().and_then(take) {
                    *job = None;
                    *done = true;
                    if let Err(e) = result {
                        *error = Some(e);
                    }
                }
            }
            SubScreen::CreateWorldUpload { request, job, .. } => {
                if let Some(result) = job.as_ref().and_then(take) {
                    let request = request.clone();
                    *job = None;
                    match result {
                        // `RealmsCreateWorldFlow.createWorld`'s success: the return screen
                        // (the configuration screen) — or, for a Realm just created, it is
                        // played (`RealmsMainScreen.play(server, returnScreen, true)`).
                        Ok(()) => {
                            if request.creation.is_some() {
                                return Some(self.start_join_after_create(request.server));
                            }
                            self.back_to_configure_worlds(request.server);
                        }
                        // `RealmsUploadCanceledException`: the reset screen it started from.
                        Err(upload::UploadError::Cancelled) => {
                            self.subscreen = Self::reset_screen(request);
                        }
                        // `RealmsGenericErrorScreen(mco.create.world.failed, lastScreen)`:
                        // the generic title over that message.
                        Err(_) => {
                            self.subscreen = SubScreen::Error {
                                title: crate::lang::get("mco.errorMessage.generic"),
                                detail: crate::lang::get("mco.create.world.failed"),
                                next: Box::new(Self::reset_screen(request)),
                            };
                        }
                    }
                }
            }
            _ => return None,
        }
        Some(Outcome::None)
    }

    /// The upload screens' clicked buttons: `RealmsSelectFileToUploadScreen`'s Upload and
    /// Back, `RealmsUploadScreen`'s Cancel and Done, `RealmsCreateWorldFlow`'s alert Cancel.
    /// `Outcome::None` for any other button.
    pub(super) fn click_upload_screen(&mut self, hit: Option<u8>) -> Outcome {
        match hit {
            Some(82) => {
                self.upload_selected();
                Outcome::None
            }
            Some(84) => {
                if let SubScreen::Uploading { progress, done, .. } = &self.subscreen
                    && !*done
                {
                    progress.cancel();
                }
                Outcome::None
            }
            Some(85) => {
                if let SubScreen::Uploading { server, .. } = &self.subscreen {
                    let server = server.clone();
                    self.back_to_configure_worlds(server);
                }
                Outcome::None
            }
            Some(93) => {
                if let SubScreen::CreateWorldUpload { progress, .. } = &self.subscreen {
                    progress.cancel();
                }
                Outcome::None
            }
            Some(83) => {
                if let SubScreen::UploadSelect {
                    server,
                    slot,
                    kind,
                    creation,
                    ..
                } = &self.subscreen
                {
                    let (server, slot, kind) = (server.clone(), *slot, *kind);
                    let creation = creation.clone();
                    self.subscreen = SubScreen::ResetWorld {
                        server,
                        slot,
                        kind,
                        creation,
                        templates: None,
                        job: None,
                        reset: None,
                        error: None,
                    };
                }
                Outcome::None
            }
            _ => Outcome::None,
        }
    }

    /// `RealmsSelectFileToUploadScreen`'s rows: the row under the cursor of the filtered list.
    pub(super) fn click_upload_list(&self, x: f32, y: f32, w: f32, h: f32) -> Option<usize> {
        match &self.subscreen {
            SubScreen::UploadSelect {
                worlds,
                scroll,
                query,
                ..
            } => {
                // `RealmsSelectFileToUploadScreen`: 270-wide rows from y 33 to H - 33,
                // the list filtered by the search box.
                let (top, bottom) = (33.0, h - 33.0);
                let rx = ((w - 270.0) / 2.0).floor();
                let shown = worlds
                    .iter()
                    .filter(|world| upload_matches(world, query))
                    .count();
                if y < top || y >= bottom || x < rx || x >= rx + 270.0 {
                    None
                } else {
                    let row = ((y - top - 2.0 + scroll.amount) / ROW_HEIGHT).floor();
                    (row >= 0.0)
                        .then_some(row as usize)
                        .filter(|row| *row < shown)
                }
            }
            _ => None,
        }
    }

    /// `WorldListEntry.mouseClicked`'s double click (`onEntryInteract`): the row's level opens
    /// `RealmsUploadScreen`.
    pub(super) fn upload_select_row(&mut self, row: usize) {
        let SubScreen::UploadSelect {
            server,
            slot,
            worlds,
            query,
            creation,
            ..
        } = &self.subscreen
        else {
            return;
        };
        let world = worlds
            .iter()
            .filter(|world| upload_matches(world, query))
            .nth(row)
            .cloned();
        if let Some(world) = world {
            let (server, slot, creation) = (server.clone(), *slot, creation.clone());
            self.start_upload(server, slot, world, creation);
        }
    }

    /// `RealmsSelectFileToUploadScreen`'s wheel. `Some(false)` when the screen has no list to
    /// scroll, `Some(true)` once the list scrolled.
    pub(super) fn wheel_upload_select(
        &mut self,
        delta: f32,
        cursor: (f32, f32),
        (_, h): (f32, f32),
    ) -> Option<bool> {
        let (top, bottom, content, height) = match &self.subscreen {
            SubScreen::UploadSelect { worlds, query, .. } => {
                // The draw's rows run from the 33-tall header to the 33-tall footer.
                (
                    33.0,
                    h - 33.0,
                    worlds
                        .iter()
                        .filter(|world| upload_matches(world, query))
                        .count() as f32
                        * ROW_HEIGHT,
                    ROW_HEIGHT,
                )
            }
            _ => return None,
        };
        if cursor.1 < top || cursor.1 >= bottom {
            return None;
        }
        if let SubScreen::UploadSelect { scroll, .. } = &mut self.subscreen {
            scroll.wheel(delta, height / 2.0, top, bottom, content);
            return Some(true);
        }
        Some(false)
    }

    /// The upload screens' buttons, into the screen's `buttons`.
    pub(super) fn upload_buttons(
        &self,
        w: f32,
        h: f32,
        buttons: &mut Vec<(f32, f32, f32, String, bool, u8, Option<(&'static str, f32)>)>,
    ) {
        let cx = (w / 2.0).floor();
        let t = crate::lang::get;
        match &self.subscreen {
            SubScreen::UploadSelect {
                worlds,
                selected,
                query,
                ..
            } => {
                // `RealmsSelectFileToUploadScreen.init`: two default 150-wide buttons 8
                // apart in the 33-tall footer (h - 27); Upload needs a selected row of
                // the filtered list.
                let x = (w - 308.0) / 2.0;
                let shown = worlds
                    .iter()
                    .filter(|world| upload_matches(world, query))
                    .count();
                buttons.push((
                    x,
                    h - 27.0,
                    150.0,
                    t("mco.upload.button.name"),
                    selected.is_some_and(|i| i < shown),
                    82,
                    None,
                ));
                buttons.push((x + 158.0, h - 27.0, 150.0, t("gui.back"), true, 83, None));
            }
            SubScreen::Uploading { progress, done, .. } => {
                // `RealmsUploadScreen.init`: Done/Cancel is the default 150-wide button in
                // the 33-tall footer (h - 27), and Cancel is off while the answer verifies.
                if *done {
                    buttons.push((cx - 75.0, h - 27.0, 150.0, t("gui.done"), true, 85, None));
                } else {
                    buttons.push((
                        cx - 75.0,
                        h - 27.0,
                        150.0,
                        t("gui.cancel"),
                        !progress.completed(),
                        84,
                        None,
                    ));
                }
            }
            SubScreen::CreateWorldUpload { .. } => {
                // `AlertScreen.init`: Cancel is the 150-wide button at
                // `clamp(90 + textHeight + 12, h / 6 + 96, h - 24)`, the message empty.
                let y = (h / 6.0 + 96.0).max(102.0_f32.min(h - 24.0));
                buttons.push((cx - 75.0, y, 150.0, t("gui.cancel"), true, 93, None));
            }
            _ => {}
        }
    }

    /// The upload screens' body: `RealmsSelectFileToUploadScreen`'s list, `RealmsUploadScreen`'s
    /// status bar, and the alert's title.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_upload_screen(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        caret: &mut crate::ui::Caret,
        focused_at: Instant,
        (w, h): (f32, f32),
        cx: f32,
    ) {
        let t = crate::lang::get;
        match &self.subscreen {
            SubScreen::UploadSelect {
                worlds,
                selected,
                scroll,
                query,
                ..
            } => {
                // `RealmsSelectFileToUploadScreen.init`: the title and a 200-wide search
                // box 4 under it in the 33-tall header (title 0, box 13), then the list
                // from 33 to h - 33, its first entry at +2.
                centered(
                    batch,
                    atlas,
                    &t("mco.upload.select.world.title"),
                    cx,
                    0.0,
                    u32::MAX,
                );
                caret.draw(
                    batch,
                    atlas,
                    (REALMS_UPLOAD_SEARCH, query, 32),
                    [cx - 100.0, 13.0, 200.0, 20.0],
                    true,
                    focused_at,
                );
                if query.is_empty() {
                    batch.text(
                        atlas,
                        &t("selectWorld.search"),
                        cx - 96.0,
                        19.0,
                        rgba(64, 64, 64, 255),
                        true,
                    );
                }
                let shown: Vec<&crate::title::WorldEntry> = worlds
                    .iter()
                    .filter(|world| upload_matches(world, query))
                    .collect();
                let (top, bottom) = (33.0, h - 33.0);
                crate::ui::list_background(batch, atlas, top, bottom, false);
                let rx = ((w - 270.0) / 2.0).floor();
                let content = shown.len() as f32 * ROW_HEIGHT;
                let first = batch.vertices.len();
                for (i, world) in shown.iter().enumerate() {
                    let y = top + 2.0 + i as f32 * ROW_HEIGHT - scroll.amount;
                    if y + ROW_HEIGHT <= top || y >= bottom {
                        continue;
                    }
                    if *selected == Some(i) {
                        batch.fill(atlas, rx, y, 270.0, ROW_HEIGHT, 0xFF80_8080);
                        batch.fill(
                            atlas,
                            rx + 1.0,
                            y + 1.0,
                            268.0,
                            ROW_HEIGHT - 2.0,
                            0xFF00_0000,
                        );
                    }
                    // `WorldListEntry.extractContent`: the world's 32x32 `icon.png` at the
                    // content origin (a missing icon shows the unknown-server sprite),
                    // then three lines at contentY+1/+12/+21, the text at contentX + 35.
                    let pixels = self.saves.as_ref().and_then(|saves| {
                        let path = saves.join(&world.folder).join("icon.png");
                        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
                        let mut icons = self.world_icons.borrow_mut();
                        let cached = icons
                            .entry(path.clone())
                            .or_insert_with(|| (std::time::UNIX_EPOCH, None));
                        if cached.0 != modified {
                            *cached = (
                                modified,
                                crate::hud::decode(&path)
                                    .ok()
                                    .filter(|(w, h, _)| *w == 64 && *h == 64)
                                    .map(|(_, _, pixels)| pixels),
                            );
                        }
                        cached.1.clone()
                    });
                    let icon = pixels
                        .as_deref()
                        .and_then(|rgba| icon_sprite(batch, atlas, i, rgba));
                    match icon {
                        Some(sprite) => {
                            batch.sprite_scaled(
                                atlas,
                                &sprite,
                                rx + 2.0,
                                y + 2.0,
                                32.0,
                                32.0,
                                u32::MAX,
                            );
                        }
                        None => {
                            batch.sprite_scaled(
                                atlas,
                                "misc/unknown_server",
                                rx + 2.0,
                                y + 2.0,
                                32.0,
                                32.0,
                                u32::MAX,
                            );
                        }
                    }
                    let text_x = rx + 37.0;
                    batch.text(atlas, &world.name, text_x, y + 3.0, u32::MAX, true);
                    let info = format!("{} · {}", world.folder, world.version);
                    batch.text(atlas, &info, text_x, y + 14.0, 0xFF80_8080, true);
                    let mode = if world.hardcore {
                        "gameMode.hardcore"
                    } else {
                        match world.game_mode {
                            1 => "gameMode.creative",
                            2 => "gameMode.adventure",
                            3 => "gameMode.spectator",
                            _ => "gameMode.survival",
                        }
                    };
                    batch.text(
                        atlas,
                        &t(mode),
                        text_x,
                        y + 23.0,
                        if world.hardcore { RED } else { 0xFF80_8080 },
                        true,
                    );
                }
                batch.clip_since(first, 0.0, top, w, bottom);
                crate::ui::scrollbar(
                    batch,
                    atlas,
                    rx + 270.0 + 8.0,
                    top,
                    bottom,
                    content,
                    scroll.amount,
                );
                if shown.is_empty() {
                    centered(
                        batch,
                        atlas,
                        &t("mco.upload.select.world.none"),
                        cx,
                        top + 16.0,
                        u32::MAX,
                    );
                }
            }
            SubScreen::Uploading {
                world,
                progress,
                error,
                done,
                started,
                ..
            } => {
                // `RealmsUploadScreen.extractRenderState`: the status, dots, the bar and the
                // error lines; the upload speed is not shown (the port keeps no rate).
                let running = if progress.completed() {
                    t("mco.upload.verifying")
                } else if progress.started() {
                    crate::lang::tr("mco.upload.uploading", &[&world.name])
                } else {
                    t("mco.upload.preparing")
                };
                let status = match error {
                    Some(upload::UploadError::Cancelled) => t("mco.upload.cancelled"),
                    Some(upload::UploadError::WorldNotClosed) => t("mco.upload.close.failure"),
                    Some(upload::UploadError::Failed(message)) => {
                        crate::lang::tr("mco.upload.failed", &[message])
                    }
                    Some(upload::UploadError::TooLarge) => running.clone(),
                    None if *done => t("mco.upload.done"),
                    None => running,
                };
                centered(batch, atlas, &status, cx, 50.0, u32::MAX);
                if !*done {
                    let elapsed = started.elapsed().as_millis() / 500;
                    let dots = ["", ".", ". .", ". . ."][elapsed as usize % 4];
                    let width = HudBatch::text_width(atlas, &status);
                    batch.text(atlas, dots, cx + width / 2.0 + 5.0, 50.0, u32::MAX, true);
                }
                // `uploadStarted() && !cancelled`: a cancelled upload hides its bar.
                if progress.started() && !progress.is_cancelled() {
                    let percentage = progress.percentage();
                    let left = (w - 200.0) / 2.0;
                    let right = left + transfer_bar_width(percentage);
                    batch.fill(atlas, left - 1.0, 79.0, right - left + 2.0, 17.0, u32::MAX);
                    batch.fill(
                        atlas,
                        left,
                        80.0,
                        (right - left).max(0.0),
                        15.0,
                        0xFF80_8080,
                    );
                    let percent = percent_label(percentage);
                    centered(
                        batch,
                        atlas,
                        &crate::lang::tr("mco.upload.percent", &[&percent]),
                        cx,
                        84.0,
                        u32::MAX,
                    );
                    // `extractUploadSpeed`: `(Unit.humanReadable(bytesPerSecond) + "/s")`
                    // just right of the percentage. The rate is the bytes over the
                    // screen's elapsed time (`UploadStatus.refreshBytesPerSecond`'s idea).
                    let elapsed = started.elapsed().as_secs_f64();
                    let rate = if elapsed > 0.0 {
                        (progress.written() as f64 / elapsed) as u64
                    } else {
                        0
                    };
                    if rate > 0 {
                        let label = format!("({}/s)", human_readable(rate));
                        let percent_w = HudBatch::text_width(atlas, &percent);
                        batch.text(
                            atlas,
                            &label,
                            cx + (percent_w / 2.0).floor() + 15.0,
                            84.0,
                            u32::MAX,
                            true,
                        );
                    }
                }
                if let Some(upload::UploadError::TooLarge) = error {
                    centered(
                        batch,
                        atlas,
                        &t("mco.upload.failed.too_big.title"),
                        cx,
                        110.0,
                        RED,
                    );
                    let size = upload_limit_size(upload::SIZE_LIMIT);
                    centered(
                        batch,
                        atlas,
                        &crate::lang::tr("mco.upload.failed.too_big.description", &[&size]),
                        cx,
                        122.0,
                        RED,
                    );
                }
            }
            SubScreen::CreateWorldUpload { .. } => {
                // `AlertScreen.extractRenderState`: the title at 70 over an empty message;
                // the Cancel button is drawn with the screen's buttons.
                centered(
                    batch,
                    atlas,
                    &t("mco.create.world.reset.title"),
                    cx,
                    70.0,
                    u32::MAX,
                );
            }
            _ => {}
        }
    }

    /// Escape on the upload screens, as the shared `Escape` enum's arms had it: the upload's
    /// Cancel aborts the body and Escape is taken; the alert's Escape does nothing; the reset
    /// screen (and the upload's Done) rebuild the screen `escape` builds for them.
    pub(super) fn escape_upload_screens(&mut self) -> Option<bool> {
        match &self.subscreen {
            SubScreen::UploadSelect {
                server,
                slot,
                kind,
                creation,
                ..
            } => {
                let (server, slot, kind) = (server.clone(), *slot, *kind);
                let creation = creation.clone();
                self.subscreen = SubScreen::ResetWorld {
                    server,
                    slot,
                    kind,
                    creation,
                    templates: None,
                    job: None,
                    reset: None,
                    error: None,
                };
                Some(false)
            }
            SubScreen::Uploading {
                server,
                progress,
                done,
                ..
            } => {
                if *done {
                    self.back_to_configure_worlds(server.clone());
                } else {
                    // `RealmsUploadScreen.keyPressed`: Escape cancels the body.
                    progress.cancel();
                }
                Some(false)
            }
            // `AlertScreen.shouldCloseOnEsc` is false: Escape does nothing while the
            // generated world uploads.
            SubScreen::CreateWorldUpload { .. } => Some(false),
            _ => None,
        }
    }
}
