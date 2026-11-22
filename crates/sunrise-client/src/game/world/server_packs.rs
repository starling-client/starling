//! A server's resource packs bound to the game (`ClientCommonPacketListenerImpl.
//! handleResourcePackPush` and `handleResourcePackPop`, its `PackConfirmScreen`,
//! and `DownloadedPackSource` as `Minecraft` drives it): the prompt, the
//! server list's setting, downloads into `downloads/`, and a resource reload
//! with the server's packs on top of the player's own.
use super::{Game, packs::ReloadKind};
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

impl ServerPackState {
    /// The `ToastManager` slots these system toasts hold.
    pub(super) fn toast_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.notifiers
            .iter()
            .filter_map(|n| n.toast.as_ref())
            .chain(self.load_fail.as_ref())
            .chain(self.chunk_io.iter().flatten())
            .chain(self.server_errors.iter().flatten())
            .chain(
                self.hook_panics
                    .iter()
                    .filter_map(|(toast, _)| toast.as_ref()),
            )
            .filter_map(|t| t.slot)
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
    /// The account the downloads are made for (the download headers).
    user: (String, u128),
    /// The packs loaded on top of the player's own, bottom first.
    pub(super) active: Vec<packs::Pack>,
    /// The download batches' notifiers, oldest first.
    notifiers: Vec<PackNotifier>,
    /// `SystemToastId.PACK_LOAD_FAILURE`: the toast `Minecraft.clearResourcePacksOnError` adds.
    pub(super) load_fail: Option<SystemToast>,
    /// `SystemToastId.LOW_DISK_SPACE`, `CHUNK_LOAD_FAILURE` and `CHUNK_SAVE_FAILURE`:
    /// the toasts the integrated server's chunk IO reports add (`chunk_io`).
    pub(super) chunk_io: [Option<SystemToast>; 3],
    /// Error reports the integrated server delivered to the window.
    server_errors: Vec<Option<SystemToast>>,
    /// Pull callback failures to show, one toast per panic.
    /// One error toast per `(hook, mod)` whose callback panicked and was disabled for the session.
    pub(super) hook_panics: Vec<(Option<SystemToast>, bool)>,
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
    toast: Option<SystemToast>,
    toast_cancelled: bool,
}
impl PackNotifier {
    fn new(progress: Receiver<download::Progress>, total: usize) -> Self {
        Self {
            progress,
            total,
            count: 0,
            failed: 0,
            size: None,
            bytes: None,
            done: false,
            toast: None,
            toast_cancelled: false,
        }
    }
    /// The batch's events since the last call, with the toast updated once
    /// for all of them. False once the batch has ended and its toast is gone.
    fn poll(&mut self, admit: impl FnOnce(SystemToast) -> Option<SystemToast>) -> bool {
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
            // SystemToast.addOrUpdate.
            match &mut self.toast {
                Some(toast) => toast.reset(title, message),
                None => {
                    self.toast = admit(SystemToast::new(title, message));
                    self.toast_cancelled = self.toast.is_none();
                }
            }
        }
        if hide && let Some(toast) = &mut self.toast {
            toast.force_hide();
        }
        !ended || self.toast.is_some()
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
    pub(super) fn server_packs_connect(&mut self, address: Option<&str>, user: (String, u128)) {
        let state = &mut self.server_packs;
        state.user = user;
        state.prompt = None;
        state.notifiers.clear();
        let Some(address) = address else {
            state.address = None;
            state.packs.local_world();
            return;
        };
        let file = multiplayer::servers_file(&saves());
        // ServerList.get: the shown entries, then the hidden ones.
        let (shown, hidden) = servers::load_all(&file);
        let entry = shown.into_iter().chain(hidden).find(|s| s.ip == address);
        let setting = entry.as_ref().and_then(|e| e.accept_textures);
        state.address = entry.map(|e| e.ip);
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
        shown
            .into_iter()
            .chain(hidden)
            .find(|s| s.ip == address)?
            .accept_textures
    }
    fn send_pack_status(&self, id: u128, status: i32) {
        if let Some(r) = &self.remote {
            r.session.send_pack_status(id, status);
        }
    }

    /// The prompt's answer (`PackConfirmScreen`'s callback): the server's
    /// setting follows it, declining a required pack disconnects, and every
    /// pack it asked about is pushed.
    pub(super) fn answer_pack_prompt(&mut self, yes: bool) -> Result<()> {
        let Some(prompt) = self.server_packs.prompt.take() else {
            return Ok(());
        };
        if self.player.is_some() && self.screen == super::Screen::Playing {
            self.capture(true);
        }
        let setting = if yes {
            self.server_packs.packs.allow();
            Some(true)
        } else {
            self.server_packs.packs.reject();
            if prompt.required {
                let reason = Text::translate("multiplayer.requiredTexturePrompt.disconnect", &[]);
                return self.leave_server(Some(reason));
            }
            Some(false)
        };
        for (id, url, hash) in prompt.requests {
            self.server_packs.packs.push(id, &url, parse_sha1(&hash));
        }
        // ServerList.saveSingleServer.
        if let Some(address) = &self.server_packs.address {
            let file = multiplayer::servers_file(&saves());
            let _guard = servers::lock();
            let (mut list, mut hidden) = servers::load_all(&file);
            let mut found = false;
            for entry in list.iter_mut().filter(|s| &s.ip == address) {
                entry.accept_textures = setting;
                found = true;
            }
            // Not among the shown entries: the hidden ones (a Direct Connection's).
            if !found {
                for entry in hidden.iter_mut().filter(|s| &s.ip == address) {
                    entry.accept_textures = setting;
                }
            }
            if let Err(e) = servers::save_all(&file, &list, &hidden) {
                log::warn!("Sunrise: could not save {}: {e}", file.display());
            }
        }
        Ok(())
    }

    /// Runs what the packs need (`createUpdateScheduler`): the downloads
    /// that finished, the ones to start, a resource reload when the active
    /// packs change, and the statuses for the server.
    pub(super) fn tick_server_packs(&mut self) {
        let mut notifiers = std::mem::take(&mut self.server_packs.notifiers);
        notifiers.retain_mut(|n| n.poll(|toast| self.add_system_toast(toast)));
        self.server_packs.notifiers = notifiers;
        self.drive_mod_browser();
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
        for (id, status) in self.server_packs.packs.take_replies() {
            self.send_pack_status(id, status);
        }
    }
    fn start_pack_downloads(&mut self, requests: Vec<sunrise_client::server_packs::Request>) {
        let state = &mut self.server_packs;
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
                        .push(PackNotifier::new(events, requests.len()));
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
            .push(PackNotifier::new(events, requests.len()));
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
    }

    /// Admit new server reports in the receiving window. `SystemToast.addOrUpdate`
    /// resets an existing chunk toast instead of admitting a second one.
    fn add_server_report_toasts(&mut self) {
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
            if let Some(shown) = &mut self.server_packs.chunk_io[index] {
                shown.reset_from(new);
            } else {
                self.server_packs.chunk_io[index] = self.add_system_toast(new);
            }
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
            if let Some(toast) = self.add_system_toast(notice) {
                self.server_packs.server_errors.push(Some(toast));
            }
        }
    }

    /// Queued hook-listener panics from any thread, each shown once per
    /// `(hook, mod)` as a red-titled system toast. Enqueued directly on the
    /// client toast queue rather than through `toast_add` so a mod cannot veto
    /// or rewrite its own failure notice; headless servers log the error and do
    /// not drain the client queue.
    fn add_hook_panic_toasts(&mut self) {
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
            self.server_packs
                .hook_panics
                .push((Some(toast), hide_with_f1));
        }
    }

    /// System toasts use the normal free 32-pixel slots and time out even when hidden.
    /// Mandatory diagnostics obey presentation settings but not a mod's HUD veto.
    pub(super) fn draw_pack_toasts(
        &mut self,
        batch: &mut HudBatch,
        hud_hidden: bool,
        presentation_hidden: bool,
    ) {
        let multiplier = self.settings.notification_display_time;
        self.add_server_report_toasts();
        self.add_hook_panic_toasts();
        let mut used = self.toast_slots_used();
        let f1 = self.ui.player_hides_gui;
        let packs = &mut self.server_packs;
        for (slot_toast, hidden) in
            packs
                .notifiers
                .iter_mut()
                .map(|n| &mut n.toast)
                .chain(std::iter::once(&mut packs.load_fail))
                .chain(packs.chunk_io.iter_mut())
                .chain(packs.server_errors.iter_mut())
                .map(|toast| (toast, hud_hidden))
                .chain(packs.hook_panics.iter_mut().map(|(toast, hide_with_f1)| {
                    (toast, presentation_hidden || *hide_with_f1 && f1)
                }))
        {
            let Some(toast) = slot_toast else {
                continue;
            };
            if toast.slot.is_none() {
                let Some(slot) = (0..Self::TOAST_SLOTS).find(|s| used & (1 << s) == 0) else {
                    continue;
                };
                used |= 1 << slot;
                toast.slot = Some(slot);
            }
            let (sound, finished) = toast.update(multiplier);
            if let Some(show) = sound {
                let event = if show { "ui.toast.in" } else { "ui.toast.out" };
                self.audio
                    .play(&format!("minecraft:{event}"), None, 1.0, 1.0);
            }
            if finished {
                *slot_toast = None;
                continue;
            }
            if let (false, Some(slot)) = (hidden, toast.slot) {
                toast.draw(&self.gui.atlas, batch, 32.0 * slot as f32);
            }
        }
        packs.server_errors.retain(Option::is_some);
        packs.hook_panics.retain(|(toast, _)| toast.is_some());
        self.draw_controller_notice(batch);
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
                self.audio.play_button_click();
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
    use download::Progress::*;

    #[test]
    #[ignore = "daily"]
    fn an_audio_panic_toast_survives_its_mods_hud_veto_but_not_f1() {
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
            fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
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

                    game.server_packs.load_fail = Some(SystemToast::new(
                        Text::tr("resourcePack.load_fail", &[]),
                        None,
                    ));
                    let size = [640, 480];
                    let _ = game.hud_layers(size);
                    let regular = game.server_packs.load_fail.as_mut().unwrap();
                    let error = game.server_packs.hook_panics[0].0.as_mut().unwrap();
                    assert_eq!(regular.slot, Some(0));
                    let error_slot = error.slot.unwrap();
                    assert_ne!(error_slot, 0, "the ordinary toast keeps its slot");
                    let error_y = 32.0 * error_slot as f32;
                    assert_eq!(
                        error.message.as_ref().unwrap().component()["with"][0]["text"],
                        OWNER
                    );
                    let past = Instant::now() - Duration::from_millis(700);
                    regular.slide.origin = Some(past);
                    error.slide.origin = Some(past);

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
                    let canceled = game.hud_layers(size);
                    assert!(!has_toast(&canceled, 0.0), "ordinary toast is canceled");
                    assert!(has_toast(&canceled, error_y), "audio error still draws");
                    game.key_pressed(KeyCode::F1, KeyCode::F1).unwrap();
                    game.handle_play_keybinds();
                    assert!(
                        !has_toast(&game.hud_layers(size), error_y),
                        "F1 hides the notice"
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn closing_save_reports_reach_the_title_in_the_owning_window() {
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
                let serving = Serving::fixture_with_reports(0, window.client_reports.clone());
                window.integrated = Some(Integrated::fixture_for(&serving));
                window.begin_closing_save(false).unwrap();
                assert!(window.pending_save.is_some());
                let dir = tempfile::tempdir().unwrap();
                let reporter =
                    sunrise_client::chunk_io::SaveFailureReporter::new(true, dir.path(), "test");
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
                assert!(window.server_packs.chunk_io[2].is_none());
                let mut batch = HudBatch::scratch([640, 480]);
                window.draw_pack_toasts(&mut batch, true, false);
                let toast = window.server_packs.chunk_io[2].as_ref().unwrap();
                assert_eq!(
                    toast.title.translation_key(),
                    Some("chunk.toast.saveFailure")
                );
                assert_eq!(toast.title.component()["with"][0]["text"], "[10, -7]");
                assert_eq!(toast.message.as_ref().unwrap().plain(), "Reported on title");
                assert!(toast.slot.is_some());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn scheduled_task_error_toasts_are_admitted_by_the_owning_client() {
        use crate::game::{
            integrated::{Integrated, Serving},
            native_entity_refs::tests::{with_client_game, with_game},
        };
        use mod_api::{ClientHost, Flow, Hooks, ModEntry, ServerHost, ToastAddEvent, ToastKind};
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
                let serving = Serving::fixture_with_reports(0, window.client_reports.clone());
                window.integrated = Some(Integrated::fixture_for(&serving));
                with_game(|server| {
                    server.serving = Some(serving);
                    server.native_lifecycle.world_open_ready().unwrap();
                    for id in [ACCEPTED, CANCELLED] {
                        let _owner = mod_api::__private::enter(id, "schedule fixture");
                        let host: &mut dyn ServerHost = &mut *server;
                        host.schedule_task(0, Box::new(|_| panic!("scheduled callback panicked")))
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
                    other.draw_pack_toasts(&mut batch, true, false);
                    assert!(other.server_packs.server_errors.is_empty());
                    assert!(
                        observed.lock().unwrap().is_empty(),
                        "not sent to another window"
                    );
                });

                window.draw_pack_toasts(&mut batch, true, false);
                let [Some(toast)] = window.server_packs.server_errors.as_slice() else {
                    panic!("only the accepted notice should reach the queue");
                };
                assert_eq!(toast.title.plain(), "Edited task error");
                assert!(toast.slot.is_some());
                assert_eq!(
                    toast.message.as_ref().unwrap().component()["with"][0]["text"],
                    ACCEPTED,
                );
                window.draw_pack_toasts(&mut batch, true, false);
                assert_eq!(observed.lock().unwrap().as_slice(), [ACCEPTED, CANCELLED]);
            });
        });
    }

    fn notifier(total: usize, events: &[download::Progress]) -> PackNotifier {
        let (send, receive) = std::sync::mpsc::channel();
        for event in events {
            send.send(*event).unwrap();
        }
        let mut notifier = PackNotifier::new(receive, total);
        assert!(notifier.poll(Some));
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
        let mut n = PackNotifier::new(receive, 2);
        assert!(n.poll(Some));
        // Not the last request yet: still downloading.
        assert_eq!(texts(&n).0["translate"], "download.pack.title");
        send.send(RequestFinished(true)).unwrap();
        drop(send);
        assert!(n.poll(Some), "the failure toast stays until it slides out");
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
        let mut n = PackNotifier::new(receive, 1);
        send.send(RequestStart).unwrap();
        assert!(n.poll(|_| None));
        assert!(n.toast.is_none());
        send.send(DownloadStart(Some(100))).unwrap();
        send.send(Bytes(50)).unwrap();
        assert!(n.poll(|_| panic!("cancelled toast must not be added again")));
        assert!(n.toast.is_none());
        send.send(RequestFinished(true)).unwrap();
        drop(send);
        assert!(!n.poll(|_| panic!("no toast on completion")));
    }

    #[test]
    fn a_finished_batch_without_a_toast_is_dropped() {
        let (send, receive) = std::sync::mpsc::channel::<download::Progress>();
        drop(send);
        assert!(!PackNotifier::new(receive, 1).poll(Some));
    }

    #[test]
    #[ignore = "daily"]
    fn hook_panic_toast_cannot_be_vetoed_or_rewritten_by_toast_add() {
        use mod_api::{
            ClientHost, ClientTickPreEvent, Flow, Hooks, ModEntry, RenderGuiPreEvent, ToastAddEvent,
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
            fn render_gui_pre(&self, _: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
                Flow::Cancel
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("veto_mod", Box::new(VetoingPanicker))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                // Ordinary system toasts are still vetoed by toast_add.
                assert!(
                    game.add_system_toast(SystemToast::new(Text::literal("ordinary"), None))
                        .is_none()
                );
                game.native_client_tick_pre();
                // Dedicated/server instances do not drain the client toast queue.
                game.options.dedicated = Some(Vec::new());
                game.add_hook_panic_toasts();
                assert!(game.server_packs.hook_panics.is_empty());
                // The client drains and queues the toast directly without firing toast_add.
                game.options.dedicated = None;
                let _ = lang::scoped(lang::Language::load(&[], lang::DEFAULT), || {
                    game.add_hook_panic_toasts();
                });
                assert_eq!(game.server_packs.hook_panics.len(), 1);
                let toast = game.server_packs.hook_panics[0]
                    .0
                    .as_ref()
                    .expect("hook panic toast queued");
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
                game.draw_every_toast(&mut first, false, gui_canceled);
                let toast = game.server_packs.hook_panics[0].0.as_mut().unwrap();
                assert_eq!(toast.slot, Some(0));
                toast.slide.origin =
                    Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
                let mut visible = HudBatch::scratch([640, 480]);
                game.draw_every_toast(&mut visible, false, gui_canceled);
                let mut drawn_text = visible.vertices.iter().filter(|v| {
                    v.text == 1
                        && (0.0..visible.width).contains(&v.pos[0])
                        && (0.0..32.0).contains(&v.pos[1])
                });
                assert!(
                    drawn_text
                        .clone()
                        .any(|v| { v.color == sunrise_client::hud::rgba(255, 85, 85, 255) }),
                    "the error title draws inside its slot despite the HUD veto"
                );
                assert!(
                    drawn_text.any(|v| v.color == u32::MAX),
                    "the message draws too"
                );

                game.set_hud_mode(mod_api::HudMode::NoHud);
                assert!(game.ui.hide_gui);
                let mut mod_hidden = HudBatch::scratch([640, 480]);
                game.draw_every_toast(&mut mod_hidden, false, false);
                assert!(
                    !mod_hidden.vertices.is_empty(),
                    "NoHud does not hide diagnostics"
                );

                game.toggle_hide_gui();
                game.set_hud_mode(mod_api::HudMode::AllVisible);
                assert!(game.ui.player_hides_gui);
                let mut player_hidden = HudBatch::scratch([640, 480]);
                game.draw_every_toast(&mut player_hidden, false, false);
                assert!(
                    player_hidden.vertices.iter().any(|v| {
                        v.text == 1 && v.color == sunrise_client::hud::rgba(255, 85, 85, 255)
                    }),
                    "the hook failure diagnostic still draws with the vanilla HUD hidden"
                );
            });
        });
    }
}

impl ServerPackState {
    /// Starling: the prompt shows: it takes the back key and the touch controls' screen state.
    pub(super) fn prompting(&self) -> bool {
        self.prompt.is_some()
    }
}
