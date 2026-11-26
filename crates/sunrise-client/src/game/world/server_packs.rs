//! A server's resource packs bound to the game (`ClientCommonPacketListenerImpl.
//! handleResourcePackPush` and `handleResourcePackPop`, its `PackConfirmScreen`,
//! and `DownloadedPackSource` as `Minecraft` drives it): the prompt, the
//! server list's setting, downloads into `downloads/`, and a resource reload
//! with the server's packs on top of the player's own.
use super::{
    Game,
    packs::ReloadKind,
    toast_manager::{SystemToastId, Toast},
};
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};
use sunrise_client::{
    Result,
    confirm_screen::Confirm,
    hud::{HudAtlas, HudBatch, rgba},
    multiplayer, packs,
    scoreboard::{comp_text, tag_to_json},
    server_packs::{Prompt, ServerPacks, action, download, parse_sha1, valid_url},
    text::{Text, named_color},
    toasts::SystemToast,
    ui,
};
use sunrise_protocol::{packets::configuration::ResourcePackPush, servers};
#[path = "server_packs/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;

/// `ClientCommonPacketListenerImpl.PackConfirmScreen`: the packs it asks
/// about, whether the last one is required, and the server's message.
pub(super) struct PackPrompt {
    requests: Vec<(u128, String, String)>,
    required: bool,
    prompt: Option<Text>,
}
impl PackPrompt {
    fn texts(&self) -> (Text, Text, [Text; 2]) {
        let t = |key: &str| Text::translate(key, &[]);
        let (title, line2) = if self.required {
            let yellow = named_color("yellow").unwrap_or(0xFFFF55);
            (
                t("multiplayer.requiredTexturePrompt.line1"),
                t("multiplayer.requiredTexturePrompt.line2")
                    .with_color(yellow)
                    .bold(),
            )
        } else {
            (
                t("multiplayer.texturePrompt.line1"),
                t("multiplayer.texturePrompt.line2"),
            )
        };
        // preparePackPrompt.
        let message = match &self.prompt {
            Some(prompt) => Text::translate(
                "multiplayer.texturePrompt.serverPrompt",
                &[line2, prompt.clone()],
            ),
            None => line2,
        };
        let buttons = if self.required {
            [t("gui.proceed"), t("menu.disconnect")]
        } else {
            [t("gui.yes"), t("gui.no")]
        };
        (title, message, buttons)
    }
    fn layout(&self, atlas: &HudAtlas, size: (f32, f32)) -> Confirm {
        let (title, message, _) = self.texts();
        Confirm::new(&atlas.font, size, &title, &message)
    }
}

/// The game's `DownloadedPackSource`.
#[derive(Default)]
pub(super) struct ServerPackState {
    packs: ServerPacks,
    /// `downloads/` in the game folder, opened at the first download.
    queue: Option<download::Queue>,
    pub(super) prompt: Option<PackPrompt>,
    /// The server list entry the setting is saved to (none: a direct join).
    pub(super) address: Option<String>,
    /// That entry's name, with `address`, so a pack answer updates only that row.
    pub(super) name: Option<String>,
    /// The account the downloads are made for (the download headers).
    user: (String, u128),
    /// The packs loaded on top of the player's own, bottom first.
    pub(super) active: Vec<packs::Pack>,
    /// The download batches' notifiers, oldest first.
    notifiers: Vec<PackNotifier>,
    /// Distinguish batches while their download notifications are still queued or visible.
    next_download_toast: usize,
}
impl ServerPackState {
    /// Submit every request to the pack manager and save the server list. A missing
    /// policy change leaves the current entry alone but still saves the list.
    fn complete_prompt(&mut self, prompt: PackPrompt, policy_change: Option<bool>) {
        for (id, url, hash) in prompt.requests {
            self.packs.push(id, &url, parse_sha1(&hash));
        }
        // ServerList.saveSingleServer: one row, matched by name and address.
        let (Some(address), Some(name)) = (&self.address, &self.name) else {
            return;
        };
        let file = multiplayer::servers_file(&saves());
        let _guard = servers::lock();
        let (mut shown, mut hidden) = servers::load_all(&file);
        if let Some(setting) = policy_change {
            servers::save_single_accept_textures(
                &mut shown,
                &mut hidden,
                name,
                address,
                Some(setting),
            );
        }
        if let Err(e) = servers::save_all(&file, &shown, &hidden) {
            log::warn!("Sunrise: could not save {}: {e}", file.display());
        }
    }
}
/// `DownloadedPackSource.createDownloadNotifier`: one batch's progress in its
/// own `SystemToast`.
struct PackNotifier {
    progress: Receiver<download::Progress>,
    total: usize,
    /// `count`: the requests started.
    count: usize,
    /// `failCount`.
    failed: usize,
    /// `totalBytes`: the Content-Length of the request in flight.
    size: Option<u64>,
    /// The bytes it has written so far: none before the first response.
    bytes: Option<u64>,
    /// The last request has finished.
    done: bool,
    id: usize,
    toast_cancelled: bool,
}
impl PackNotifier {
    fn new(progress: Receiver<download::Progress>, total: usize, id: usize) -> Self {
        Self {
            progress,
            total,
            count: 0,
            failed: 0,
            size: None,
            bytes: None,
            done: false,
            id,
            toast_cancelled: false,
        }
    }
    /// Update once for the accumulated events. Returns whether to keep polling
    /// the channel and whether a successful download should hide its toast.
    fn poll(&mut self, update: impl FnOnce(SystemToast) -> bool) -> (bool, bool) {
        let (mut changed, mut hide) = (false, false);
        let ended = loop {
            let event = match self.progress.try_recv() {
                Ok(event) => event,
                Err(e) => break e == TryRecvError::Disconnected,
            };
            changed |= match event {
                download::Progress::RequestStart => {
                    self.count += 1;
                    true
                }
                download::Progress::DownloadStart(size) => {
                    self.size = size;
                    self.bytes = Some(0);
                    true
                }
                download::Progress::Bytes(bytes) => {
                    self.bytes = Some(bytes);
                    true
                }
                download::Progress::RequestFinished(ok) => {
                    self.failed += usize::from(!ok);
                    self.done = self.count == self.total;
                    // Every pack came: forceHide; else the failure title.
                    hide |= self.done && self.failed == 0;
                    self.done && self.failed > 0
                }
            };
        };
        if changed && !self.toast_cancelled {
            let (title, message) = self.text();
            self.toast_cancelled = !update(SystemToast::new(title, message));
        }
        (!ended, hide)
    }
    /// The toast's title and message.
    fn text(&self) -> (Text, Option<Text>) {
        if self.done && self.failed > 0 {
            let failed = Text::tr("download.pack.failed", &[&self.failed, &self.total]);
            return (failed, None);
        }
        let title = Text::tr("download.pack.title", &[&self.count, &self.total]);
        let message = self.bytes.map(|bytes| match self.size {
            Some(size) if size > 0 => {
                Text::tr("download.pack.progress.percent", &[&(bytes * 100 / size)])
            }
            _ => Text::tr(
                "download.pack.progress.bytes",
                &[&sunrise_client::realms::human_readable(bytes)],
            ),
        });
        (title, message)
    }
}
pub(super) fn saves() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join("saves")
}

/// `WorldOpenFlows.WORLD_PACK_ID`: the id of a world's own resource pack.
const WORLD_PACK_ID: u128 = 0x640a6a92_b6cb_48a0_b391_831586500359;

impl Game<'_> {
    /// `WorldOpenFlows.loadBundledResourcePack`: a world's own
    /// `resourcepacks/resources.zip` is pushed as a local pack before the world
    /// connects, after `configureForLocalWorld` so the pack is accepted at once.
    pub(super) fn server_packs_world_pack(&mut self, folder: &str) {
        let path = saves()
            .join(folder)
            .join("resourcepacks")
            .join("resources.zip");
        if path.is_file() {
            self.server_packs.packs.local_world();
            self.server_packs.packs.push_local(WORLD_PACK_ID, path);
        }
    }
    /// `ConnectScreen`: the server's setting decides what its packs do
    /// (`configureForServerControl`); single player's own server takes them
    /// all, quietly (`configureForLocalWorld`).
    pub(super) fn server_packs_connect(
        &mut self,
        address: Option<&str>,
        list_name: Option<&str>,
        user: (String, u128),
    ) {
        let state = &mut self.server_packs;
        state.user = user;
        state.prompt = None;
        state.notifiers.clear();
        let Some(address) = address else {
            state.address = None;
            state.name = None;
            state.packs.local_world();
            return;
        };
        let file = multiplayer::servers_file(&saves());
        // ServerList.get: the shown entries, then the hidden ones.
        let (shown, hidden) = servers::load_all(&file);
        let entry = servers::find(&shown, &hidden, address, list_name).cloned();
        let setting = entry.as_ref().and_then(|e| e.accept_textures);
        state.address = entry.as_ref().map(|e| e.ip.clone());
        state.name = entry.map(|e| e.name);
        state.packs.connect(Prompt::for_server(setting));
        // GetServerDetailsTask.scheduleResourcePackDownload: the Realm's own pack, which the
        // player agreed to before connecting (`allowServerPacks`, then `pushPack`).
        if let Some(pack) = sunrise_client::realms::take_pending_pack(address) {
            state.packs.allow();
            state.packs.push(pack.id, &pack.url, parse_sha1(&pack.hash));
        }
    }
    /// `Minecraft.clearDownloadedResourcePacks`: every server pack goes at once.
    pub(super) fn server_packs_disconnect(&mut self) {
        self.server_packs.prompt = None;
        self.server_packs.notifiers.clear();
        self.server_packs.packs.disconnect();
        self.tick_server_packs();
    }

    /// `handleResourcePackPush`.
    pub(super) fn handle_pack_push(&mut self, p: ResourcePackPush) {
        if !valid_url(&p.url) {
            self.send_pack_status(p.id, action::INVALID_URL);
            return;
        }
        let setting = self.server_setting();
        let prompt = Prompt::for_server(setting);
        if prompt != Prompt::Pending && (!p.required || prompt != Prompt::Declined) {
            self.server_packs
                .packs
                .push(p.id, &p.url, parse_sha1(&p.hash));
            // `acceptPack`/`pushPack` report ACCEPTED or DECLINED from the packet handler, so
            // it reaches the server in this phase: before a FinishConfiguration behind it.
            self.send_pack_replies();
            return;
        }
        // addOrUpdatePackPrompt: one prompt for every pack pushed before it is answered.
        let text = p.prompt.as_ref().map(|t| comp_text(&tag_to_json(t)));
        // A new prompt opens as a screen (a mod may keep it shut, and then
        // nothing answers the pack); an open one only takes the pack.
        if self.server_packs.prompt.is_none()
            && !self.open_screen_overlay("server_pack_prompt", |game| {
                game.server_packs.prompt = Some(PackPrompt {
                    requests: Vec::new(),
                    required: p.required,
                    prompt: None,
                });
            })
        {
            return;
        }
        let state = &mut self.server_packs;
        let open = state.prompt.get_or_insert(PackPrompt {
            requests: Vec::new(),
            required: p.required,
            prompt: None,
        });
        open.requests.push((p.id, p.url, p.hash));
        open.required = p.required;
        open.prompt = text;
        // Gui.setScreen: KeyMapping.releaseAll, so no walking behind the prompt.
        self.clear_device_keys();
        self.capture(false);
    }
    /// `handleResourcePackPop`: one pack, or all of them.
    pub(super) fn handle_pack_pop(&mut self, id: Option<u128>) {
        match id {
            Some(id) => self.server_packs.packs.pop(id),
            None => self.server_packs.packs.pop_all(),
        }
    }
    /// The joined server's entry in the server list: its `acceptTextures`.
    fn server_setting(&self) -> Option<bool> {
        let address = self.server_packs.address.as_deref()?;
        let (shown, hidden) = servers::load_all(&multiplayer::servers_file(&saves()));
        servers::find(&shown, &hidden, address, self.server_packs.name.as_deref())?.accept_textures
    }
    fn send_pack_status(&self, id: u128, status: i32) {
        if let Some(r) = &self.remote {
            r.session.send_pack_status(id, status);
        }
    }
    /// `DownloadedPackSource`'s feedback: `connection.send` as each status is reported.
    fn send_pack_replies(&mut self) {
        for (id, status) in self.server_packs.packs.take_replies() {
            self.send_pack_status(id, status);
        }
    }

    /// `PackConfirmScreen`'s callback: push every pending pack and save the server list.
    /// Declining a required pack closes the connection without changing its stored policy.
    pub(super) fn answer_pack_prompt(&mut self, yes: bool) -> Result<()> {
        let Some(prompt) = self.server_packs.prompt.take() else {
            return Ok(());
        };
        if self.player.is_some() && self.screen == super::Screen::Playing {
            self.capture(true);
        }
        if yes {
            self.server_packs.packs.allow();
        } else {
            self.server_packs.packs.reject();
            if prompt.required {
                let reason = Text::translate("multiplayer.requiredTexturePrompt.disconnect", &[]);
                // The requests still get local DECLINED feedback. The transport is already
                // closed, so cleanup drops the replies without sending them.
                return self.close_server_session(Some(reason), |state| {
                    state.complete_prompt(prompt, None);
                });
            }
        }
        self.refresh_screen_widgets();
        self.server_packs.complete_prompt(prompt, Some(yes));
        // `allowServerPacks` and `pushPack` report from the screen's callback.
        self.send_pack_replies();
        Ok(())
    }

    /// Runs what the packs need (`createUpdateScheduler`): the downloads
    /// that finished, the ones to start, a resource reload when the active
    /// packs change, and the statuses for the server.
    pub(super) fn tick_server_packs(&mut self) {
        let mut notifiers = std::mem::take(&mut self.server_packs.notifiers);
        notifiers.retain_mut(|n| {
            let id = SystemToastId::PackDownload(n.id);
            let (keep, hide) = n.poll(|new| self.add_or_update_system_toast(id, new));
            if hide && let Some(toast) = self.toast_manager.system_mut(id) {
                toast.force_hide();
            }
            keep
        });
        self.server_packs.notifiers = notifiers;
        // A reload waits for the world's resources (`reload_packs`).
        if self.rest_loader.is_some() {
            return;
        }
        self.drive_pack_reload();
        self.drive_pack_browser();
        // One reload at a time: the player's own waits under its overlay
        // before the server's packs can ask for theirs (`ServerPacks` keeps
        // its own reload from overlapping).
        let selection_reloading = self
            .pack_reload
            .as_ref()
            .is_some_and(|r| r.kind() == ReloadKind::Selection);
        let finished = self
            .server_packs
            .queue
            .as_ref()
            .map(download::Queue::finished)
            .unwrap_or_default();
        for batch in finished {
            self.server_packs.packs.downloaded(batch);
        }
        while !selection_reloading && self.server_packs.packs.wants_tick() {
            let tick = self.server_packs.packs.tick();
            if !tick.download.is_empty() {
                self.start_pack_downloads(tick.download);
            }
            if let Some(load) = tick.reload {
                self.start_pack_reload(&load);
            }
        }
        self.send_pack_replies();
    }
    fn start_pack_downloads(&mut self, requests: Vec<sunrise_client::server_packs::Request>) {
        let state = &mut self.server_packs;
        let toast_id = state.next_download_toast;
        state.next_download_toast += 1;
        if state.queue.is_none() {
            let dir = std::env::current_dir()
                .unwrap_or_default()
                .join("downloads");
            match download::Queue::open(dir) {
                Ok(queue) => state.queue = Some(queue),
                Err(e) => {
                    log::error!("Sunrise: failed to open the download queue: {e}");
                    // Every request failed, as the notifier would hear it.
                    let (progress, events) = std::sync::mpsc::channel();
                    for _ in &requests {
                        let _ = progress.send(download::Progress::RequestStart);
                        let _ = progress.send(download::Progress::RequestFinished(false));
                    }
                    state
                        .notifiers
                        .push(PackNotifier::new(events, requests.len(), toast_id));
                    let failed = requests.iter().map(|r| r.id).collect();
                    state.packs.downloaded(sunrise_client::server_packs::Batch {
                        downloaded: Vec::new(),
                        failed,
                    });
                    return;
                }
            }
        }
        let headers = download::headers(&state.user.0, state.user.1);
        let (progress, events) = std::sync::mpsc::channel();
        state
            .notifiers
            .push(PackNotifier::new(events, requests.len(), toast_id));
        if let Some(queue) = &state.queue {
            queue.download(requests, headers, progress);
        }
    }
    /// `DownloadedPackSource.startReload`: the packs on top of the player's
    /// own, staged on a worker (`Minecraft.reloadResourcePacks` under a
    /// `LoadingOverlay`); `drive_pack_reload` applies it. A pack with no
    /// readable `pack.mcmeta` fails the reload at once, as `onFailure`.
    fn start_pack_reload(&mut self, load: &[(u128, PathBuf)]) {
        let server: Option<Vec<_>> = load
            .iter()
            .map(|(id, path)| packs::server_pack(*id, path))
            .collect();
        let Some(server) = server else {
            log::warn!("Sunrise: invalid pack metadata in a server pack, ignoring all");
            self.finish_pack_reload(false);
            return;
        };
        self.server_packs.active = server;
        if let Err(e) = self.spawn_pack_reload(ReloadKind::Server) {
            log::error!("Sunrise: could not start the pack reload worker: {e}");
            self.finish_pack_reload(false);
        }
    }
    /// `ServerPacks::reloaded` and the recovery reload it can ask for.
    pub(super) fn finish_pack_reload(&mut self, ok: bool) {
        if let Some(recovery) = self.server_packs.packs.reloaded(ok) {
            self.start_pack_reload(&recovery);
        }
    }

    pub(super) fn draw_pack_prompt(&self, batch: &mut HudBatch) {
        let Some(prompt) = &self.server_packs.prompt else {
            return;
        };
        sunrise_client::widgets::cover_frame();
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        if self.player.is_none() {
            // In place of the connecting screen, on its background.
            batch.fill(
                atlas,
                0.0,
                0.0,
                w,
                h,
                sunrise_client::hud::rgba(20, 20, 20, 255),
            );
        }
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.player.is_some());
        let (title, message, [yes, no]) = prompt.texts();
        // PackConfirmScreen is a ConfirmScreen.
        sunrise_client::widgets::begin_screen(|| {
            super::screen_info::confirm("pack_prompt", &title.plain(), &message.plain())
        });
        let mut layout = prompt.layout(atlas, (w, h));
        layout.declare_no();
        layout.draw(batch, atlas, self.cursor, [&yes, &no]);
        self.draw_overlay_mounted_widgets(batch, "server_pack_prompt");
    }

    /// Admit new server reports in the receiving window. `SystemToast.addOrUpdate`
    /// resets an existing chunk toast instead of admitting a second one.
    pub(super) fn add_server_report_toasts(&mut self) {
        let pending = self.client_reports.take();
        let toasts = [
            pending
                .low_disk_space
                .then(sunrise_client::chunk_io::low_disk_space_toast),
            pending
                .load_failure
                .map(|(x, z)| sunrise_client::chunk_io::load_failure_toast(x, z)),
            pending
                .save_failure
                .map(|(x, z)| sunrise_client::chunk_io::save_failure_toast(x, z)),
        ];
        for (index, new) in toasts.into_iter().enumerate() {
            let Some(new) = new else { continue };
            self.add_or_update_system_toast(SystemToastId::ChunkIo(index), new);
        }
        for mod_id in pending.scheduled_task_failures {
            let notice = SystemToast::new(
                Text::tr("sunrise.mods.scheduled_task_error.title", &[]),
                Some(Text::tr(
                    "sunrise.mods.scheduled_task_error.message",
                    &[&mod_id],
                )),
            )
            .with_title_color(rgba(255, 85, 85, 255));
            self.add_system_toast(None, notice);
        }
    }

    /// Queued hook-listener panics from any thread, each shown once per
    /// `(hook, mod)` as a red-titled system toast. Enqueued directly on the
    /// client toast queue rather than through `toast_add` so a mod cannot veto
    /// or rewrite its own failure notice; headless servers log the error and do
    /// not drain the client queue.
    pub(super) fn add_hook_panic_toasts(&mut self) {
        if self.serving.is_some() || self.options.dedicated.is_some() {
            return;
        }
        for report in mod_api::__private::take_hook_panics() {
            let toast = SystemToast::new(
                Text::tr("sunrise.mods.hook_panic.title", &[]),
                Some(Text::tr(
                    "sunrise.mods.hook_panic.description",
                    &[&report.mod_id, &report.hook],
                )),
            )
            .with_title_color(sunrise_client::hud::rgba(255, 85, 85, 255));
            // Audio playback notices retain the player's existing F1 visibility rule.
            let hide_with_f1 = matches!(report.hook, "AudioStream::read" | "AudioStream::drop");
            let clock = sunrise_client::toasts::ToastClock::of(self.clock.still());
            self.toast_manager.post(
                clock,
                Toast::Diagnostic {
                    toast,
                    hide_with_f1,
                },
            );
        }
    }

    /// A left click on the prompt: its answer when it hit a button.
    pub(super) fn pack_prompt_click(&mut self) -> Result<bool> {
        let Some(prompt) = &self.server_packs.prompt else {
            return Ok(false);
        };
        let answer = prompt
            .layout(&self.gui.atlas, self.gui_size())
            .answer_at(self.cursor);
        match answer {
            Some(yes) => {
                self.play_audio_button_click();
                self.answer_pack_prompt(yes)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{
        in_own_process,
        native_client_session::tests::{finish_login, logged_client},
        native_entity_refs::tests::with_client_game,
    };
    use download::Progress::*;
    use std::{path::Path, time::Duration};
    use sunrise_protocol::{
        connection::Connection,
        packets::{Empty, play::Serverbound},
    };

    const PACK_IDS: [u128; 2] = [31, 32];

    fn with_pack_prompt(required: bool, test: impl FnOnce(&mut Game<'_>, &mut Connection, &Path)) {
        const ADDRESS: &str = "pack-test.invalid";
        let dir = tempfile::tempdir().unwrap();
        sunrise_client::logging::set_player_dir(dir.path().to_owned());
        let file = dir.path().join("servers.dat");
        if required {
            // Start the joined shown row at Disabled (`0`) and include an unrelated entry
            // whose Disabled policy is encoded as an NBT Int `0` (valid under vanilla's
            // NbtOps.getBooleanValue) so a lossy decoder or policy reset would be caught.
            let root = sunrise_core::nbt::NamedTag {
                name: sunrise_core::nbt::JavaString::from(""),
                tag: sunrise_core::nbt::Tag::compound([(
                    "servers",
                    sunrise_core::nbt::Tag::List(vec![
                        sunrise_core::nbt::Tag::compound([
                            ("name", sunrise_core::nbt::Tag::String("Shown".into())),
                            ("ip", sunrise_core::nbt::Tag::String(ADDRESS.into())),
                            ("acceptTextures", sunrise_core::nbt::Tag::Byte(0)),
                            ("hidden", sunrise_core::nbt::Tag::Byte(0)),
                        ]),
                        sunrise_core::nbt::Tag::compound([
                            ("name", sunrise_core::nbt::Tag::String("Numeric".into())),
                            ("ip", sunrise_core::nbt::Tag::String("other.invalid".into())),
                            ("acceptTextures", sunrise_core::nbt::Tag::Int(0)),
                            ("hidden", sunrise_core::nbt::Tag::Byte(0)),
                        ]),
                        sunrise_core::nbt::Tag::compound([
                            ("name", sunrise_core::nbt::Tag::String("Hidden".into())),
                            ("ip", sunrise_core::nbt::Tag::String(ADDRESS.into())),
                            ("acceptTextures", sunrise_core::nbt::Tag::Byte(1)),
                            ("hidden", sunrise_core::nbt::Tag::Byte(1)),
                        ]),
                    ]),
                )]),
            };
            let bytes = sunrise_core::nbt::write_named(&root, sunrise_core::nbt::Limits::default())
                .unwrap();
            std::fs::write(&file, bytes).unwrap();
        } else {
            let shown = servers::ServerEntry::new("Shown", ADDRESS);
            let mut hidden = servers::ServerEntry::new("Hidden", ADDRESS);
            hidden.accept_textures = Some(true);
            servers::save_all(&file, &[shown], &[hidden]).unwrap();
        }

        with_client_game(|game| {
            let mut peer = Connection::local(logged_client(game));
            peer.reader
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            finish_login(game, &mut peer);
            game.server_packs_connect(Some(ADDRESS), None, ("SessionReadTest".into(), 42));
            for id in PACK_IDS {
                game.handle_pack_push(ResourcePackPush {
                    id,
                    url: "https://pack-test.invalid/server.zip".into(),
                    hash: String::new(),
                    required,
                    prompt: None,
                });
            }
            test(game, &mut peer, &file);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn required_decline_closes_the_wire_but_reports_locally_and_saves() {
        in_own_process(
            "game::server_packs::tests::required_decline_closes_the_wire_but_reports_locally_and_saves",
            || {
                with_pack_prompt(true, |game, peer, file| {
                    let original_bytes = std::fs::read(file).unwrap();
                    let backup = file.with_file_name("servers.dat_old");
                    // Mark an old mtime to detect an actual rewrite.
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(file)
                        .unwrap()
                        .set_modified(std::time::UNIX_EPOCH)
                        .unwrap();
                    let old_modified = file.metadata().unwrap().modified().unwrap();
                    game.server_packs.packs.record_feedback();
                    game.answer_pack_prompt(false).unwrap();

                    assert!(game.remote.is_none());
                    assert!(matches!(
                        &game.title.as_ref().unwrap().mode,
                        sunrise_client::title::Mode::Multiplayer(multiplayer::Screen::Disconnected {
                            reason, retry: None, ..
                        }) if reason.translation_key() == Some("multiplayer.requiredTexturePrompt.disconnect")
                    ));
                    loop {
                        match peer.recv() {
                            Ok((id, body)) => assert!(!matches!(
                                Serverbound::decode(id, &body).unwrap(),
                                Serverbound::ResourcePack(_)
                            )),
                            Err(e) if e.is_end_of_stream() => break,
                            Err(e) => panic!("expected transport EOF: {e}"),
                        }
                    }
                    assert_eq!(
                        game.server_packs.packs.take_recorded_feedback(),
                        PACK_IDS.map(|id| (id, action::DECLINED))
                    );
                    assert_eq!(game.server_packs.packs.prompt(), Prompt::Pending);
                    assert!(game.server_packs.packs.take_replies().is_empty());
                    assert_ne!(file.metadata().unwrap().modified().unwrap(), old_modified);
                    assert_eq!(std::fs::read(&backup).unwrap(), original_bytes);
                    // Read the saved raw NBT directly to prove Disabled policies were not cleared.
                    let (saved, _) = sunrise_core::nbt::read_named(
                        &std::fs::read(file).unwrap(),
                        sunrise_core::nbt::Limits::default(),
                    )
                    .unwrap();
                    let Some(sunrise_core::nbt::Tag::List(entries)) = saved.tag.get("servers")
                    else {
                        panic!("expected servers list");
                    };
                    assert_eq!(entries.len(), 3);
                    assert_eq!(
                        entries[0].get("acceptTextures"),
                        Some(&sunrise_core::nbt::Tag::Byte(0))
                    );
                    assert_eq!(
                        entries[1].get("acceptTextures"),
                        Some(&sunrise_core::nbt::Tag::Byte(0))
                    );
                    assert_eq!(
                        entries[2].get("acceptTextures"),
                        Some(&sunrise_core::nbt::Tag::Byte(1))
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn optional_decline_replies_on_the_live_wire_and_saves_disabled_policy() {
        in_own_process(
            "game::server_packs::tests::optional_decline_replies_on_the_live_wire_and_saves_disabled_policy",
            || {
                with_pack_prompt(false, |game, peer, file| {
                    let (mut shown, hidden) = servers::load_all(file);
                    shown[0].accept_textures = Some(false);
                    game.answer_pack_prompt(false).unwrap();
                    game.tick_server_packs();
                    game.send(Serverbound::Punch(Empty)); // Fence the outgoing replies.
                    let mut replies = Vec::new();
                    loop {
                        let (id, body) = peer.recv().unwrap();
                        match Serverbound::decode(id, &body).unwrap() {
                            Serverbound::ResourcePack(response) => {
                                replies.push((response.id, response.action));
                            }
                            Serverbound::Punch(_) => break,
                            _ => {}
                        }
                    }
                    assert_eq!(replies, PACK_IDS.map(|id| (id, 1))); // DECLINED
                    assert_eq!(servers::load_all(file), (shown, hidden));
                });
            },
        );
    }

    /// `ServerPackManager.acceptPack` reports from `handleResourcePackPush`, so a pack
    /// the server list allows is ACCEPTED in configuration, before the FinishConfiguration
    /// that the same poll acknowledges.
    #[test]
    #[ignore = "daily"]
    fn an_allowed_pack_is_accepted_before_finish_configuration() {
        in_own_process(
            "game::server_packs::tests::an_allowed_pack_is_accepted_before_finish_configuration",
            || {
                use sunrise_protocol::packets::{configuration as cfg, login};
                const ADDRESS: &str = "pack-order.invalid";
                let dir = tempfile::tempdir().unwrap();
                sunrise_client::logging::set_player_dir(dir.path().to_owned());
                let mut entry = servers::ServerEntry::new("Packs", ADDRESS);
                entry.accept_textures = Some(true);
                servers::save_all(&dir.path().join("servers.dat"), &[entry], &[]).unwrap();
                with_client_game(|game| {
                    let mut peer = Connection::local(logged_client(game));
                    peer.reader
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    game.server_packs_connect(Some(ADDRESS), None, ("SessionReadTest".into(), 42));
                    peer.recv().unwrap(); // Intention.
                    peer.recv().unwrap(); // Hello.
                    let finished = login::Clientbound::LoginFinished(login::LoginFinished {
                        profile: login::GameProfile {
                            uuid: 42,
                            name: "SessionReadTest".into(),
                            properties: vec![],
                        },
                        session: 1,
                    });
                    peer.send(&finished.encode()).unwrap();
                    peer.recv().unwrap(); // Login acknowledged.
                    peer.recv().unwrap(); // Brand.
                    peer.recv().unwrap(); // Client information.
                    let push = cfg::ResourcePackPush {
                        id: PACK_IDS[0],
                        url: "https://pack-order.invalid/server.zip".into(),
                        hash: String::new(),
                        required: false,
                        prompt: None,
                    };
                    for packet in [
                        cfg::Clientbound::ResourcePackPush(push),
                        cfg::Clientbound::FinishConfiguration(Empty),
                    ] {
                        peer.send(&packet.encode()).unwrap();
                    }
                    // Both reach main and are handled in one poll, with no pack tick between.
                    game.acknowledge_test_configuration();
                    let mut sent = Vec::new();
                    loop {
                        let (id, body) = peer.recv().unwrap();
                        match cfg::Serverbound::decode(id, &body).unwrap() {
                            cfg::Serverbound::ResourcePack(r) => sent.push(Some((r.id, r.action))),
                            cfg::Serverbound::FinishConfiguration(_) => {
                                sent.push(None);
                                break;
                            }
                            _ => {}
                        }
                    }
                    assert_eq!(sent, [Some((PACK_IDS[0], action::ACCEPTED)), None]);
                    assert!(game.server_packs.packs.take_replies().is_empty());
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn an_audio_panic_toast_survives_its_mods_hud_veto_but_not_f1() {
        crate::game::in_own_process(
            "game::server_packs::tests::an_audio_panic_toast_survives_its_mods_hud_veto_but_not_f1",
            || {
                use mod_api::{
                    AudioStream, ClientHost, Flow, Hooks, ModEntry, RenderGuiPreEvent, SoundSource,
                    StreamSpec,
                };
                use std::{
                    sync::mpsc,
                    time::{Duration, Instant},
                };
                use winit::keyboard::KeyCode;

                const OWNER: &str = "audio_toast_fixture";
                struct HideHud;
                #[mod_api::__subscriptions]
                impl Hooks for HideHud {
                    fn render_gui_pre(
                        &self,
                        _: &dyn ClientHost,
                        _: &mut RenderGuiPreEvent,
                    ) -> Flow {
                        Flow::Cancel
                    }
                }

                struct BrokenSource(mpsc::Sender<()>);
                impl AudioStream for BrokenSource {
                    fn read(&mut self, _: &mut [i16]) -> usize {
                        panic!("broken pull callback");
                    }
                }
                impl Drop for BrokenSource {
                    fn drop(&mut self) {
                        let _ = self.0.send(());
                    }
                }

                mod_platform::testing::with_mods(
                    vec![ModEntry::new(OWNER, Box::new(HideHud))].into(),
                    || {
                        crate::game::native_entity_refs::tests::with_client_game(|game| {
                            game.title = None;
                            game.screen = crate::game::Screen::Playing;
                            game.audio.backend_live_for_bench();
                            let spec = StreamSpec::new(SoundSource::Voice, 2, 48_000);
                            let (dropped, retired) = mpsc::channel();
                            let _handle = game
                                .audio
                                .open_stream(OWNER, &spec, None, Box::new(BrokenSource(dropped)))
                                .unwrap();
                            game.audio.mix_buffer(&mut [0.0; 2], 2, 48_000);
                            // The worker queues the notice before dropping the failed source.
                            retired.recv_timeout(Duration::from_secs(1)).unwrap();

                            game.add_system_toast(
                                Some(SystemToastId::PackLoadFailure),
                                SystemToast::new(Text::tr("resourcePack.load_fail", &[]), None),
                            );
                            let size = [640, 480];
                            let _ = game.hud(size);
                            let _ = game.hud(size);
                            let [regular, error] = game.toast_manager.visible.as_mut_slice() else {
                                panic!("expected two visible notices")
                            };
                            let Toast::System(Some(SystemToastId::PackLoadFailure), regular_toast) =
                                &mut regular.toast
                            else {
                                panic!("expected ordinary toast")
                            };
                            let Toast::Diagnostic {
                                toast: error_toast, ..
                            } = &mut error.toast
                            else {
                                panic!("expected diagnostic toast")
                            };
                            assert_eq!(regular.slot, 0);
                            assert_ne!(error.slot, 0, "the ordinary toast keeps its slot");
                            let error_y = 32.0 * error.slot as f32;
                            assert_eq!(
                                error_toast.message.as_ref().unwrap().component()["with"][0]["text"],
                                OWNER
                            );
                            let past = Instant::now() - Duration::from_millis(700);
                            regular_toast.slide.origin = Some(past);
                            error_toast.slide.origin = Some(past);

                            let rect = *game
                                .gui
                                .atlas
                                .sprites
                                .entry("toast/system".into())
                                .or_insert(sunrise_client::hud::Rect {
                                    u0: 0.25,
                                    v0: 0.25,
                                    u1: 0.5,
                                    v1: 0.5,
                                    w: 160,
                                    h: 32,
                                });
                            let has_toast = |batch: &HudBatch, y| {
                                batch.vertices.iter().any(|vertex| {
                                    vertex.uv == [rect.u0, rect.v0]
                                        && vertex.pos[1] == y
                                        && vertex.pos[0] < batch.width
                                })
                            };
                            let canceled = game.hud(size);
                            assert!(
                                has_toast(&canceled, 0.0),
                                "the HUD veto does not hide toasts"
                            );
                            assert!(has_toast(&canceled, error_y), "audio error still draws");
                            game.key_pressed(KeyCode::F1, KeyCode::F1).unwrap();
                            game.handle_play_keybinds();
                            assert!(!has_toast(&game.hud(size), error_y), "F1 hides the notice");
                        });
                    },
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn closing_save_reports_reach_the_title_in_the_owning_window() {
        crate::game::in_own_process(
            "game::server_packs::tests::closing_save_reports_reach_the_title_in_the_owning_window",
            || {
                use crate::game::{
                    integrated::{Integrated, Serving},
                    native_entity_refs::tests::with_client_game,
                };
                use mod_api::{ClientHost, Flow, Hooks, ModEntry, ToastAddEvent, ToastKind};

                struct Listener;
                #[mod_api::__subscriptions]
                impl Hooks for Listener {
                    fn toast_add(&self, _: &dyn ClientHost, event: &mut ToastAddEvent<'_>) -> Flow {
                        use mod_api::text::TextContent::Translatable;

                        if let Translatable { key, .. } = &event.title.content
                            && key == "chunk.toast.saveFailure"
                        {
                            assert_eq!(event.kind, ToastKind::System);
                            *event.description = Some(mod_api::Text::literal("Reported on title"));
                        }
                        Flow::Continue
                    }
                }

                let mods = vec![ModEntry::new("chunk_report_probe", Box::new(Listener))];
                mod_platform::testing::with_mods(mods.into(), || {
                    with_client_game(|window| {
                        let serving =
                            Serving::fixture_with_reports(0, window.client_reports.clone());
                        window.integrated = Some(Integrated::fixture_for(&serving));
                        window.begin_closing_save(false).unwrap();
                        assert!(window.pending_save.is_some());
                        let dir = tempfile::tempdir().unwrap();
                        let reporter = sunrise_client::chunk_io::SaveFailureReporter::new(
                            true,
                            dir.path(),
                            "test",
                        );
                        serving.reports().report_save_failure(
                            &reporter,
                            dir.path(),
                            "minecraft:overworld",
                            "chunk",
                            (10, -7),
                            &std::io::Error::other("region write failed"),
                        );
                        drop(reporter);
                        drop(serving);

                        assert!(!window.poll_closing_save().unwrap());
                        assert!(window.integrated.is_none());
                        assert!(window.title.is_some());
                        assert!(
                            window
                                .toast_manager
                                .system_mut(SystemToastId::ChunkIo(2))
                                .is_none()
                        );
                        let mut batch = HudBatch::scratch([640, 480]);
                        window.draw_every_toast(&mut batch);
                        let [entry] = window.toast_manager.visible.as_slice() else {
                            panic!("expected a visible toast")
                        };
                        let Toast::System(Some(SystemToastId::ChunkIo(2)), toast) = &entry.toast
                        else {
                            panic!("expected a save failure toast")
                        };
                        assert_eq!(
                            toast.title.translation_key(),
                            Some("chunk.toast.saveFailure")
                        );
                        assert_eq!(toast.title.component()["with"][0]["text"], "[10, -7]");
                        assert_eq!(toast.message.as_ref().unwrap().plain(), "Reported on title");
                    });
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn scheduled_task_error_toasts_are_admitted_by_the_owning_client() {
        crate::game::in_own_process(
            "game::server_packs::tests::scheduled_task_error_toasts_are_admitted_by_the_owning_client",
            || {
                use crate::game::{
                    integrated::{Integrated, Serving},
                    native_entity_refs::tests::{with_client_game, with_game},
                };
                use mod_api::{
                    ClientHost, Flow, Hooks, ModEntry, ServerHost, ToastAddEvent, ToastKind,
                };
                use std::sync::{Arc, Mutex};

                const ACCEPTED: &str = "toast_accept";
                const CANCELLED: &str = "toast_cancel";
                struct Listener(Arc<Mutex<Vec<String>>>);
                #[mod_api::__subscriptions]
                impl Hooks for Listener {
                    fn toast_add(&self, _: &dyn ClientHost, event: &mut ToastAddEvent<'_>) -> Flow {
                        use mod_api::text::TextContent::{Literal, Translatable};

                        let Some(Translatable { key, arguments, .. }) =
                            event.description.as_ref().map(|text| &text.content)
                        else {
                            return Flow::Continue;
                        };
                        if key != "sunrise.mods.scheduled_task_error.message" {
                            return Flow::Continue;
                        }
                        assert_eq!(event.kind, ToastKind::System);
                        let [argument] = arguments.as_slice() else {
                            panic!("the notice must name one mod");
                        };
                        let Literal(id) = &argument.content else {
                            panic!("the mod ID must be a literal component");
                        };
                        self.0.lock().unwrap().push(id.clone());
                        if id == CANCELLED {
                            Flow::Cancel
                        } else {
                            *event.title = mod_api::Text::literal("Edited task error");
                            Flow::Continue
                        }
                    }
                }
                struct NoHooks;
                #[mod_api::__subscriptions]
                impl Hooks for NoHooks {}

                let observed = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![
                    ModEntry::new(ACCEPTED, Box::new(Listener(observed.clone()))),
                    ModEntry::new(CANCELLED, Box::new(NoHooks)),
                ];
                mod_platform::testing::with_mods(mods.into(), || {
                    with_client_game(|window| {
                        let serving =
                            Serving::fixture_with_reports(0, window.client_reports.clone());
                        window.integrated = Some(Integrated::fixture_for(&serving));
                        with_game(|server| {
                            server.serving = Some(serving);
                            server.native_lifecycle.world_open_ready().unwrap();
                            for id in [ACCEPTED, CANCELLED] {
                                let _owner = mod_api::__private::enter(id, "schedule fixture");
                                let host: &mut dyn ServerHost = &mut *server;
                                host.schedule_task(
                                    0,
                                    Box::new(|_| panic!("scheduled callback panicked")),
                                )
                                .unwrap();
                            }
                            server.native_lifecycle_server_tick_start(true);
                            server.native_lifecycle_server_tick_end(false);
                        });
                        assert!(
                            observed.lock().unwrap().is_empty(),
                            "not admitted on the server"
                        );

                        let mut batch = HudBatch::scratch([640, 480]);
                        with_client_game(|other| {
                            other.draw_every_toast(&mut batch);
                            assert!(other.toast_manager.visible.is_empty());
                            assert!(
                                observed.lock().unwrap().is_empty(),
                                "not sent to another window"
                            );
                        });

                        window.draw_every_toast(&mut batch);
                        let [entry] = window.toast_manager.visible.as_slice() else {
                            panic!("only the accepted notice should reach the queue");
                        };
                        let Toast::System(None, toast) = &entry.toast else {
                            panic!("expected a scheduled task error toast")
                        };
                        assert_eq!(toast.title.plain(), "Edited task error");
                        assert_eq!(
                            toast.message.as_ref().unwrap().component()["with"][0]["text"],
                            ACCEPTED,
                        );
                        window.draw_every_toast(&mut batch);
                        assert_eq!(observed.lock().unwrap().as_slice(), [ACCEPTED, CANCELLED]);
                    });
                });
            },
        );
    }

    fn notifier(total: usize, events: &[download::Progress]) -> PackNotifier {
        let (send, receive) = std::sync::mpsc::channel();
        for event in events {
            send.send(*event).unwrap();
        }
        let mut notifier = PackNotifier::new(receive, total, 0);
        assert!(notifier.poll(|_| true).0);
        notifier
    }
    fn texts(n: &PackNotifier) -> (serde_json::Value, Option<serde_json::Value>) {
        let (title, message) = n.text();
        (title.component(), message.map(|m| m.component()))
    }

    #[test]
    fn the_toast_counts_requests_and_shows_progress() {
        let n = notifier(2, &[RequestStart]);
        let (title, message) = texts(&n);
        assert_eq!(title["translate"], "download.pack.title");
        assert_eq!(title["with"], serde_json::json!([1, 2]));
        assert_eq!(message, None);
        let n = notifier(2, &[RequestStart, DownloadStart(Some(400)), Bytes(100)]);
        assert_eq!(
            texts(&n).1.unwrap(),
            serde_json::json!({"translate": "download.pack.progress.percent", "with": [25]})
        );
        let n = notifier(1, &[RequestStart, DownloadStart(None), Bytes(1280)]);
        assert_eq!(
            texts(&n).1.unwrap()["with"],
            serde_json::json!([{"text": "1.3 KB"}])
        );
    }

    #[test]
    fn a_failed_pack_turns_the_toast_into_the_failure_count() {
        let events = [
            RequestStart,
            RequestFinished(false),
            RequestStart,
            DownloadStart(Some(3)),
            Bytes(3),
        ];
        let (send, receive) = std::sync::mpsc::channel();
        for event in events {
            send.send(event).unwrap();
        }
        let mut n = PackNotifier::new(receive, 2, 0);
        assert!(n.poll(|_| true).0);
        // Not the last request yet: still downloading.
        assert_eq!(texts(&n).0["translate"], "download.pack.title");
        send.send(RequestFinished(true)).unwrap();
        drop(send);
        assert_eq!(
            n.poll(|_| true),
            (false, false),
            "a failure does not force-hide the toast"
        );
        let (title, message) = texts(&n);
        assert_eq!(
            title,
            serde_json::json!({"translate": "download.pack.failed", "with": [1, 2]})
        );
        assert_eq!(message, None);
    }

    #[test]
    fn a_cancelled_download_toast_is_not_retried_on_progress() {
        let (send, receive) = std::sync::mpsc::channel();
        let mut n = PackNotifier::new(receive, 1, 0);
        send.send(RequestStart).unwrap();
        assert_eq!(n.poll(|_| false), (true, false));
        send.send(DownloadStart(Some(100))).unwrap();
        send.send(Bytes(50)).unwrap();
        assert_eq!(
            n.poll(|_| panic!("cancelled toast must not be added again")),
            (true, false)
        );
        send.send(RequestFinished(true)).unwrap();
        drop(send);
        assert_eq!(n.poll(|_| panic!("no toast on completion")), (false, true));
    }

    #[test]
    fn a_finished_batch_without_a_toast_is_dropped() {
        let (send, receive) = std::sync::mpsc::channel::<download::Progress>();
        drop(send);
        assert_eq!(
            PackNotifier::new(receive, 1, 0).poll(|_| panic!("no toast without progress")),
            (false, false)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn hook_panic_toast_cannot_be_vetoed_or_rewritten_by_toast_add() {
        crate::game::in_own_process(
            "game::server_packs::tests::hook_panic_toast_cannot_be_vetoed_or_rewritten_by_toast_add",
            || {
                use mod_api::{
                    ClientHost, ClientTickPreEvent, Flow, Hooks, ModEntry, RenderGuiPreEvent,
                    ToastAddEvent,
                };
                use sunrise_client::lang;

                struct VetoingPanicker;
                #[mod_api::__subscriptions]
                impl Hooks for VetoingPanicker {
                    fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
                        panic!("boom from vetoing mod");
                    }
                    fn toast_add(&self, _: &dyn ClientHost, event: &mut ToastAddEvent<'_>) -> Flow {
                        *event.title = mod_api::Text::literal("rewritten");
                        *event.description = Some(mod_api::Text::literal("hidden"));
                        Flow::Cancel
                    }
                    fn render_gui_pre(
                        &self,
                        _: &dyn ClientHost,
                        _: &mut RenderGuiPreEvent,
                    ) -> Flow {
                        Flow::Cancel
                    }
                }
                let mods: mod_api::Mods =
                    vec![ModEntry::new("veto_mod", Box::new(VetoingPanicker))].into();
                mod_platform::testing::with_mods(mods, || {
                    super::super::native_entity_refs::tests::with_client_game(|game| {
                        // Ordinary system toasts are still vetoed by toast_add.
                        assert!(!game.add_system_toast(
                            None,
                            SystemToast::new(Text::literal("ordinary"), None)
                        ));
                        game.native_client_tick_pre();
                        // Dedicated/server instances do not drain the client toast queue.
                        game.options.dedicated = Some(Vec::new());
                        game.add_hook_panic_toasts();
                        assert!(game.toast_manager.queued.is_empty());
                        // The client drains and queues the toast directly without firing toast_add.
                        game.options.dedicated = None;
                        let _ = lang::scoped(lang::Language::load(&[], lang::DEFAULT), || {
                            game.add_hook_panic_toasts();
                        });
                        assert_eq!(game.toast_manager.queued.len(), 1);
                        let Some(Toast::Diagnostic { toast, .. }) =
                            game.toast_manager.queued.front()
                        else {
                            panic!("expected a queued diagnostic")
                        };
                        assert_eq!(
                            toast.title.component()["translate"],
                            "sunrise.mods.hook_panic.title"
                        );
                        let message = toast.message.as_ref().unwrap();
                        assert_eq!(
                            message.component(),
                            serde_json::json!({
                                "translate": "sunrise.mods.hook_panic.description",
                                "with": [{"text": "veto_mod"}, {"text": "client_tick_pre"}],
                            })
                        );
                        assert!(message.plain().contains("veto_mod"));
                        let mut gui_canceled = false;
                        game.native_render_gui_pre(&mut gui_canceled);
                        assert!(gui_canceled);
                        // Advance the existing slide past its entrance without sleeping.
                        let mut first = HudBatch::scratch([640, 480]);
                        game.draw_every_toast(&mut first);
                        game.draw_every_toast(&mut first);
                        let entry = &mut game.toast_manager.visible[0];
                        assert_eq!(entry.slot, 0);
                        let Toast::Diagnostic { toast, .. } = &mut entry.toast else {
                            panic!("expected a visible diagnostic")
                        };
                        toast.slide.origin =
                            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
                        let mut visible = HudBatch::scratch([640, 480]);
                        game.draw_every_toast(&mut visible);
                        let mut drawn_text = visible.vertices.iter().filter(|v| {
                            v.text == 1
                                && (0.0..visible.width).contains(&v.pos[0])
                                && (0.0..32.0).contains(&v.pos[1])
                        });
                        assert!(
                            drawn_text.clone().any(|v| {
                                v.color == sunrise_client::hud::rgba(255, 85, 85, 255)
                            }),
                            "the error title draws inside its slot despite the HUD veto"
                        );
                        assert!(
                            drawn_text.any(|v| v.color == u32::MAX),
                            "the message draws too"
                        );

                        game.set_hud_mode(mod_api::HudMode::NoHud);
                        assert!(game.ui.hide_gui);
                        let mut mod_hidden = HudBatch::scratch([640, 480]);
                        game.draw_every_toast(&mut mod_hidden);
                        assert!(
                            !mod_hidden.vertices.is_empty(),
                            "NoHud does not hide diagnostics"
                        );

                        game.toggle_hide_gui();
                        game.set_hud_mode(mod_api::HudMode::AllVisible);
                        assert!(game.ui.player_hides_gui);
                        let mut player_hidden = HudBatch::scratch([640, 480]);
                        game.draw_every_toast(&mut player_hidden);
                        assert!(
                            player_hidden.vertices.iter().any(|v| {
                                v.text == 1
                                    && v.color == sunrise_client::hud::rgba(255, 85, 85, 255)
                            }),
                            "the hook failure diagnostic still draws with the vanilla HUD hidden"
                        );
                    });
                });
            },
        );
    }
}
