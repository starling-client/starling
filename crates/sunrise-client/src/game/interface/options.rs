//! The options screens bound to the game: drawing, input, applying settings
//! (video, sound, chat, font, language) and the F11 fullscreen toggle.
use super::shader_packs::{TogglePlan, active_pack, dev_pack, plan_shader_toggle};
use super::{Game, ResizeState, Screen};
use serde_json::{Value, json};
use sunrise_client::{
    Result,
    graphics::Graphics,
    hdr_calibration::WindowState,
    hud::HudBatch,
    lang,
    options_ui::{Event, Page, SkinAction},
    text::Text,
    window::{FrameContext, InactivityFpsLimit, framerate_limit},
};
use winit::{event::MouseButton, keyboard::KeyCode, window::Fullscreen};

/// `CommonLinks.ACCESSIBILITY_HELP`, the Accessibility page's footer link.
pub(super) const ACCESSIBILITY_HELP_LINK: &str = "https://aka.ms/MinecraftJavaAccessibility";

pub(super) const CONTROL_HELP: &[&str] = &[
    "options-get [KEY...]           options.txt values by key, with their type, range or choices",
    "options-set KEY VALUE          set an option (enums by name, keys as key.keyboard.w, lists as JSON), apply and save it",
    "graphics-get [KEY...]          Sunrise's graphics settings (graphics.txt) by key",
    "graphics-set KEY VALUE         set a Sunrise graphics setting (resolutionScale 25..200, presentMode, workers...), apply and save it",
];

/// The catalog's shelf for a page's Catalog button.
#[cfg(target_os = "android")]
fn catalog_shelf(shelf: sunrise_client::options_ui::CatalogShelf) -> starling_catalog::Shelf {
    use sunrise_client::options_ui::CatalogShelf;
    match shelf {
        CatalogShelf::Mods => starling_catalog::Shelf::Mods,
        CatalogShelf::ResourcePacks => starling_catalog::Shelf::ResourcePacks,
        CatalogShelf::ShaderPacks => starling_catalog::Shelf::ShaderPacks,
    }
}

/// Starling: on a phone a menu over the world (pause, inventory, ...) draws at most 60 frames a
/// second. A paused game's menu draws its kept backdrop (`menu_blur_cache`) so cheaply that it
/// ran at 300+ FPS, warming the phone for nothing. A computer keeps its limit.
fn menu_frame_cap(limit: i32, has_screen: bool) -> i32 {
    if cfg!(target_os = "android") && has_screen {
        limit.min(60)
    } else {
        limit
    }
}

impl Game<'_> {
    /// One of the game's folders (packs, shader packs, mods) in the file manager. On a phone the
    /// Files app shows it through Starling's own provider, wherever the game keeps it. A folder it
    /// can't show from the app's private files gets the offer of the Starling folder instead.
    fn open_game_folder(&mut self, dir: &std::path::Path) {
        let _ = std::fs::create_dir_all(dir);
        #[cfg(target_os = "android")]
        if sunrise_client::platform::open_folder(dir) != Some(true)
            && sunrise_client::logging::game_home() == sunrise_client::logging::private_home()
        {
            self.ask_files_offer();
        }
        #[cfg(not(target_os = "android"))]
        super::packs::open_folder(dir);
    }

    pub(super) fn save_title_options(&mut self) {
        if let Some(title) = &self.title
            && (title.last_server != self.settings.file.string("lastServer")
                || title.skip_multiplayer_warning
                    != self.settings.file.bool("skipMultiplayerWarning"))
        {
            self.settings.file.set(
                "lastServer",
                sunrise_client::options_file::V::Str(title.last_server.clone()),
            );
            self.settings.file.set(
                "skipMultiplayerWarning",
                sunrise_client::options_file::V::Bool(title.skip_multiplayer_warning),
            );
            self.settings.save(&Self::options_path());
        }
    }
    /// `options-get`, `options-set`, `graphics-get` and `graphics-set`; None for other commands.
    pub(super) fn options_control(&mut self, words: &[&str], line: &str) -> Result<Option<Value>> {
        match words[0] {
            "options-get" => {
                let file = self.settings.to_file();
                let wanted: Vec<&str> = if words.len() > 1 {
                    words[1..].to_vec()
                } else {
                    sunrise_client::options_file::keys()
                };
                let mut options = serde_json::Map::new();
                for key in wanted {
                    let described = file.describe(key).ok_or_else(|| {
                        format!("unknown option {key}; options-get lists them all")
                    })?;
                    options.insert(key.to_owned(), described);
                }
                Ok(Some(json!({ "options": options })))
            }
            "options-set" => {
                let key = words.get(1).ok_or("options-set KEY VALUE")?;
                // The value runs to the end of the line (lists, server names).
                let value = line
                    .trim_start()
                    .strip_prefix("options-set")
                    .and_then(|rest| rest.trim_start().strip_prefix(key))
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .ok_or("options-set KEY VALUE: missing the value")?;
                let narrator = self.settings.narrator();
                let reduced = self.settings.reduced_debug_info;
                self.settings = self.settings.with_option(key, value)?;
                if self.settings.reduced_debug_info != reduced {
                    self.debug_entries.invalidate();
                }
                // As the options screen applies a change (OptionInstance.set
                // runs the narrator's onValueUpdate for a new value only).
                let mut events = vec![Event::Changed];
                if self.settings.narrator() != narrator {
                    events.push(Event::Narrator);
                }
                self.handle_option_events(events)?;
                match *key {
                    "lang" => self.load_language(&self.settings.language.clone()),
                    "resourcePacks" | "incompatibleResourcePacks" => self.reload_packs(),
                    _ => {}
                }
                // Labels change (a language, a value): `ui` reads the redrawn screen.
                self.ui_wait.clicked();
                let now = self.settings.to_file().describe(key);
                Ok(Some(json!({ "ok": true, "key": key, "option": now })))
            }
            "graphics-get" => {
                let text = self.settings.sunrise.serialize();
                let all: Vec<(&str, &str)> =
                    text.lines().filter_map(|l| l.split_once(':')).collect();
                if let Some(key) = words[1..]
                    .iter()
                    .find(|k| !all.iter().any(|(a, _)| a == *k))
                {
                    return Err(
                        format!("unknown graphics key {key}; graphics-get lists them all").into(),
                    );
                }
                let graphics: serde_json::Map<String, Value> = all
                    .into_iter()
                    .filter(|(k, _)| words.len() == 1 || words[1..].contains(k))
                    .map(|(k, v)| (k.to_owned(), v.into()))
                    .collect();
                Ok(Some(json!({ "graphics": graphics })))
            }
            "graphics-set" => {
                let (Some(key), Some(_)) = (words.get(1), words.get(2)) else {
                    return Err("graphics-set KEY VALUE".into());
                };
                let value = words[2..].join(" ");
                // graphics.txt's own reading, with the new line last.
                let text = self.settings.sunrise.serialize();
                let line = |text: &str| {
                    text.lines()
                        .find(|l| l.split_once(':').is_some_and(|(k, _)| k == *key))
                        .map(str::to_owned)
                };
                let before = line(&text).ok_or_else(|| {
                    format!("unknown graphics key {key}; graphics-get lists them all")
                })?;
                let graphics = Graphics::parse(&format!(
                    "{text}{key}:{value}
"
                ));
                let after = line(&graphics.serialize()).unwrap_or_default();
                if after == before && after != format!("{key}:{value}") {
                    return Err(format!("{key} does not take {value}; it stays {before}").into());
                }
                self.settings.sunrise = graphics;
                self.handle_option_events(vec![Event::Changed])?;
                let now = after.split_once(':').map_or("", |(_, v)| v).to_owned();
                // The worker pool and the graphics API are made once, at start.
                let restart = matches!(*key, "workers" | "graphicsApi");
                Ok(Some(
                    json!({ "ok": true, "key": key, "value": now, "restart": restart }),
                ))
            }
            _ => Ok(None),
        }
    }
    /// This player's `options.txt`: in the game folder, or in `--player-dir` when several
    /// players share one (`logging::set_player_dir`).
    pub(super) fn options_path() -> std::path::PathBuf {
        sunrise_client::logging::player_dir().join("options.txt")
    }
    /// The game folder (the working directory): what its players share, saves, packs, mods.
    pub(super) fn game_dir() -> std::path::PathBuf {
        std::env::current_dir().unwrap_or_default()
    }
    /// `shaderpacks/` in the game folder.
    pub(super) fn shader_packs_dir() -> std::path::PathBuf {
        Self::game_dir().join(sunrise_client::shader_pack_list::DIR)
    }
    pub(super) fn gui_size(&self) -> (f32, f32) {
        let size = self
            .renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height]);
        let b = self.interface_batch(size);
        (b.width, b.height)
    }
    /// Close the options screens (`OptionsScreen.onClose`) back to where they
    /// were opened: the recipe book's settings, World Options, the title or
    /// the pause menu.
    pub(super) fn leave_options(&mut self) {
        let to_game = self.options_from_title;
        let back = if let Some(screen) = self.panel.recipes_settings_return.take() {
            screen
        } else if std::mem::take(&mut self.options_to_world_options) {
            Screen::WorldOptions
        } else if self.options_from_title {
            Screen::Playing
        } else {
            Screen::Paused
        };
        self.set_screen(back);
        // Vanilla grabs only on `setScreen(null)`: back on the title, which
        // is still shown, the pointer stays free (#1999). The startup handoff
        // can enter a world while its options are open, and closing them then
        // returns to that world, as `capture_loaded_world` grabs.
        if to_game && self.title.is_none() {
            self.capture(true);
        }
        self.dragging = false;
        // Closed from calibration (sunrise-ctl, a lost window): out of it.
        self.show_calibration((0.0, 0.0));
    }
    /// Open the options screens (OptionsScreen) from the title or pause menu.
    pub(super) fn open_options(&mut self, from_title: bool) {
        self.open_options_at(from_title, Page::Main);
    }
    pub(super) fn open_options_at(&mut self, from_title: bool, page: Page) {
        #[cfg(target_os = "android")]
        self.mobile_options_open();
        // Gameplay keys must not stay held behind the options screen, where
        // bindings can also change before the physical key is released.
        self.clear_device_keys();
        self.set_screen(Screen::Options);
        self.options_from_title = from_title;
        let minecart_improvements = self.minecart_improvements();
        // Restore fullbright when leaving another server.
        self.hold_fullbright();
        let ui = &mut self.ui.options;
        // The game puts the skin on (game/own_skin.rs).
        ui.skin_page = true;
        ui.wardrobe_enabled = cfg!(target_os = "android");
        ui.profile_name = sunrise_client::accounts::player_name();
        ui.profile_key = sunrise_client::accounts::player_key();
        ui.skin_cloud = sunrise_client::accounts::skin_cloud();
        ui.offer_catalog();
        ui.open_at(!from_title && self.title.is_none(), page);
        ui.minecart_improvements = minecart_improvements;
        ui.languages = lang::available(&[&self.pack.source]);
        self.refresh_pack_lists();
    }
    /// While the resource pack screen is open, `resourcepacks/` is looked at once a second
    /// and the list found again when its packs changed (`PackSelectionScreen.tick`'s
    /// watcher).
    fn watch_pack_folder(&mut self) {
        if !self.ui.options.resource_packs_open() {
            self.pack_folder = None;
            return;
        }
        let now = std::time::Instant::now();
        if self
            .pack_folder
            .as_ref()
            .is_some_and(|(at, _)| now.duration_since(*at).as_secs_f32() < 1.0)
        {
            return;
        }
        let signature: Vec<(String, u64, Option<std::time::SystemTime>)> =
            std::fs::read_dir(super::packs::packs_dir())
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    let meta = e.metadata().ok();
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        meta.as_ref().map_or(0, |m| m.len()),
                        meta.and_then(|m| m.modified().ok()),
                    )
                })
                .collect();
        let changed = self
            .pack_folder
            .as_ref()
            .is_some_and(|(_, seen)| *seen != signature);
        self.pack_folder = Some((now, signature));
        if changed {
            let infos = self.resource_pack_infos();
            self.ui.options.update_pack_infos(infos);
            self.ui.options.pack_configs = super::packs::pack_configs(&self.vanilla_root());
        }
    }
    pub(super) fn draw_options(&mut self, batch: &mut HudBatch, allowed: bool) {
        if !allowed {
            self.ui.options.discard_draw();
            return;
        }
        self.watch_pack_folder();
        self.poll_diagnostics();
        self.perf_calibrate_page();
        let context = self.controller_settings_context();
        self.show_skin_cloud();
        let mut ui = std::mem::take(&mut self.ui.options);
        if ui.controller_open() {
            ui.set_controller_context(context);
        }
        let cursor = self.options_pointer(&mut ui);
        self.ui.options = ui;
        let output = self
            .renderer
            .as_ref()
            .map_or(Default::default(), |r| r.output());
        self.ui
            .options
            .draw_background(&self.settings, output, batch, &self.gui.atlas);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        self.ui
            .options
            .draw_contents(&self.settings, batch, &self.gui.atlas, cursor);
        #[cfg(target_os = "android")]
        self.draw_wardrobe_preview(batch);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        self.ui
            .options
            .draw_overlays(batch, &self.gui.atlas, cursor);
        self.show_calibration((batch.width, batch.height));
    }
    fn controller_settings_context(&self) -> sunrise_client::controller_ui::Context {
        sunrise_client::controller_ui::Context {
            player: self.options.player_name(),
            device: self.pad.device_or_virtual(),
            detected: detected_controller_style(self.pad.device.as_ref(), self.pad.style),
            glyphs: self.ui.options.pad_glyphs,
        }
    }
    /// HDR calibration's test picture to the renderer, full screen while it runs: the
    /// window goes fullscreen for it and back after, as it was (maximized, say).
    pub(super) fn show_calibration(&mut self, size: (f32, f32)) {
        let picture = (self.screen == Screen::Options)
            .then(|| self.ui.options.calibration_picture(&self.gui.atlas, size))
            .flatten();
        if let Some(r) = self.renderer.as_mut() {
            r.set_hdr_calibration(picture.map(|(step, nits, band)| {
                sunrise_client::gfx::HdrCalibration {
                    step: step.number(),
                    nits,
                    band,
                }
            }));
        }
        let Some(window) = &self.window else {
            return;
        };
        match (picture.is_some(), self.ui.options.calibration_window) {
            (true, None) => {
                // Before entering fullscreen, which un-maximizes a window on Windows.
                let was = WindowState::of(window.fullscreen().is_some(), window.is_maximized());
                if was != WindowState::Fullscreen {
                    let borderless = sunrise_client::window::borderless_fullscreen(window);
                    sunrise_client::window::enter_fullscreen(window, borderless);
                }
                self.ui.options.calibration_window = Some(was);
            }
            (false, Some(was)) => {
                let (leave_fullscreen, maximize) = was.leaving();
                if leave_fullscreen {
                    window.set_fullscreen(None);
                }
                if maximize {
                    window.set_maximized(true);
                }
                self.ui.options.calibration_window = None;
            }
            _ => {}
        }
    }
    pub(super) fn options_mouse(&mut self, pressed: bool, right: bool) -> Result<bool> {
        if !pressed {
            if right {
                return Ok(false);
            }
            let (events, handled) = self.ui.options.release_with_outcome();
            self.dragging = false;
            self.handle_option_events(events)?;
            return Ok(handled);
        }
        if !right {
            self.dragging = true;
        }
        let size = self.gui_size();
        let framebuffer = self.renderer.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        let mut ui = std::mem::take(&mut self.ui.options);
        ui.shift = self.modifiers.shift_key();
        let (events, handled) = ui.click_with_outcome(
            &mut self.settings,
            &self.gui.atlas,
            self.cursor.0,
            self.cursor.1,
            size,
            framebuffer,
            false,
            right,
        );
        self.ui.options = ui;
        self.handle_option_events(events)?;
        Ok(handled)
    }
    pub(super) fn options_drag(&mut self) -> Result<()> {
        let size = self.gui_size();
        let framebuffer = self.renderer.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        let mut ui = std::mem::take(&mut self.ui.options);
        let events = ui.click(
            &mut self.settings,
            &self.gui.atlas,
            self.cursor.0,
            self.cursor.1,
            size,
            framebuffer,
            true,
            false,
        );
        self.ui.options = ui;
        self.handle_option_events(events)
    }
    pub(super) fn options_key(&mut self, key: KeyCode, text: Option<&str>) -> Result<()> {
        let size = self.gui_size();
        let shift = self.has_shift_down();
        let framebuffer = self.renderer.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        self.ui.options.size = size;
        self.ui.options.framebuffer = framebuffer;
        let mut ui = std::mem::take(&mut self.ui.options);
        ui.modifiers = self.modifiers().bits;
        let events = ui.key_modifiers(&mut self.settings, &self.gui.atlas, key, text, shift);
        self.ui.options = ui;
        self.handle_option_events(events)
    }
    /// Ctrl+A/C/V/X in the options' search boxes; false when none has focus.
    pub(super) fn options_shortcut(
        &mut self,
        shortcut: sunrise_client::clipboard::EditShortcut,
    ) -> bool {
        self.ui
            .options
            .shortcut(&self.gui.atlas, shortcut, &mut self.clipboard)
    }
    /// The wheel on the options. A finger's scroll (`finger`) only scrolls: a list moved by a
    /// finger that landed on a cycle button must not cycle it at each step, as a mouse wheel over it
    /// does (`CycleButton.mouseScrolled`).
    pub(super) fn options_scroll(&mut self, amount: f32, finger: bool) -> Result<()> {
        if self.pad_stick_scroll() {
            let size = self.gui_size();
            self.ui
                .options
                .scroll_at(&self.settings, size, self.cursor, amount);
            return Ok(());
        }
        let framebuffer = self.renderer.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        if self.ui.options.video_open() && self.has_control_down() {
            // VideoSettingsScreen.mouseScrolled: Ctrl with the wheel steps the GUI scale.
            let mut ui = std::mem::take(&mut self.ui.options);
            let events = ui.video_wheel_gui_scale(&mut self.settings, framebuffer, amount);
            self.ui.options = ui;
            return self.handle_option_events(events);
        }
        let size = self.gui_size();
        if finger {
            self.ui
                .options
                .scroll_at(&self.settings, size, self.cursor, amount);
            return Ok(());
        }
        let events =
            self.ui
                .options
                .wheel(&mut self.settings, size, self.cursor, framebuffer, amount);
        self.handle_option_events(events)
    }
    /// Mouse buttons pressed while a key is being bound.
    pub(super) fn options_waiting_for_key(&self) -> bool {
        self.ui.options.selected_key.is_some()
    }
    pub(super) fn options_bind_mouse(&mut self, button: MouseButton) -> Result<()> {
        self.ui.options.modifiers = self.modifiers().bits;
        let events = self.ui.options.bind_mouse(&mut self.settings, button);
        self.handle_option_events(events)
    }
    pub(super) fn handle_option_events(&mut self, events: Vec<Event>) -> Result<()> {
        for e in events {
            match e {
                Event::DiagnosticsExport => self.export_diagnostics(),
                Event::DiagnosticsOpen => self.open_diagnostics(),
                Event::DiagnosticsRecord => self.toggle_diagnostic_recording(),
                Event::DiagnosticsGpu => self.toggle_diagnostic_gpu(),
                Event::PerfLab(i) => self.perf_lab_next(i),
                Event::Calibrate(i) => self.perf_calibrate_button(i),
                Event::ButtonClick => self.audio.play_button_click(),
                Event::DebugEntriesChanged => self.debug_entries.invalidate(),
                Event::Changed => {
                    #[cfg(target_os = "android")]
                    self.mobile_options_apply(true);
                    self.settings.save(&Self::options_path());
                    self.settings.keys.sync_action_bindings();
                    self.apply_settings();
                    self.switch_mods(false);
                    self.sync_native_switches();
                    self.sync_pregen_option();
                    self.sync_agent_control();
                    self.ui.options.agent_status = self.agent_status_lines();
                }
                // Every step of a drag: applied, and written once when the slider is let go
                // (`Changed`).
                Event::SliderDrag => {
                    #[cfg(target_os = "android")]
                    self.mobile_options_apply(false);
                    self.apply_settings();
                }
                // Options.createSoundSliderOptionInstance: the preview plays
                // only without a level (on the title screen's options).
                Event::Volume(source, volume) if self.title.is_some() => {
                    self.audio.preview(source, volume);
                }
                Event::Volume(..) => {}
                Event::CopyAgentDetails => {
                    use sunrise_client::clipboard::Clipboard;
                    let text = self.agent_details();
                    self.clipboard.set(&text);
                }
                // The AI Agent page's repro buttons (game/runtime/repro.rs).
                Event::ToggleRecording => self.toggle_recording_key(),
                Event::ExportRepro => self.export_repro_key(),
                // The Music page: the situations' tracks, and playing one now.
                Event::ListMusicTracks => self.ui.options.music_groups = self.music_groups(),
                Event::PlayMusicTrack(event, track) => self.play_music_track(&event, &track),
                Event::Close if self.ui.options.resource_error.is_some() => {}
                Event::Close => {
                    // A drag that ended without a release (`Changed`) still has its touch settings
                    // written.
                    #[cfg(target_os = "android")]
                    self.mobile_options_apply(true);
                    self.settings.save(&Self::options_path());
                    // The mods switched on the Mods page, in one reload (`switch_mods`).
                    self.switch_mods(true);
                    self.leave_options();
                }
                Event::Language(code) => {
                    self.load_language(&code);
                }
                Event::Packs(_) => {
                    self.settings.save(&Self::options_path());
                    self.reload_packs();
                }
                Event::PackOptions { entries, chosen } => {
                    super::packs::save_pack_options(&entries, chosen);
                    self.ui.options.pack_configs = super::packs::pack_configs(&self.vanilla_root());
                    self.reload_packs();
                }
                Event::OpenPackFolder => self.open_game_folder(&super::packs::packs_dir()),
                Event::OpenModFolder => {
                    self.open_game_folder(&Self::game_dir().join(sunrise_client::mod_list::DIR))
                }
                Event::ImportPacks(paths) => {
                    let additional = self.discover_mod_packs(mod_api::PackType::ClientResources);
                    let vanilla = self.vanilla_root();
                    super::packs::import_resource_packs(
                        &mut self.ui.options,
                        &vanilla,
                        &self.server_packs.active,
                        &paths,
                        &additional,
                    );
                }
                // Sunrise's own Add button: the screen picked a hit, the game
                // owns the folder it lands in.
                Event::DownloadPack(i) => {
                    let dir = super::packs::packs_dir();
                    if let Some(screen) = &mut self.ui.options.pack_screen
                        && let Some(browser) = &mut screen.browser
                    {
                        browser.download(i, &dir, sunrise_core::VANILLA_VERSION);
                    }
                }
                #[cfg(target_os = "android")]
                Event::OpenCatalog(shelf) => self.open_catalog_on(catalog_shelf(shelf)),
                #[cfg(not(target_os = "android"))]
                Event::OpenCatalog(_) => {}
                Event::DataPacks(ids) => {
                    let resources = self.vanilla_root();
                    if let Some(title) = &mut self.title {
                        title.offer_data_packs(&resources, ids);
                    }
                }
                Event::OpenDataPackFolder(dir) => self.open_game_folder(&dir),
                // The editor works over the world, or over the title before one: the menus
                // close for it.
                Event::TouchLayout => {
                    self.settings.save(&Self::options_path());
                    self.screen = Screen::Playing;
                    self.dragging = false;
                    self.open_touch_editor();
                }
                // The Mods screen's links go through ConfirmLinkScreen, back to the options.
                Event::OpenLink(url) => self.confirm_untrusted_link(&url, Screen::Options),
                Event::CopyText(text) => {
                    use sunrise_client::clipboard::Clipboard;
                    self.clipboard.set(&text);
                }
                Event::BrowseMods => {
                    let ui = &mut self.ui.options.mod_browser;
                    self.mod_jobs.browse(ui, Self::game_dir());
                }
                Event::InstallMod(id) => {
                    if self.player.is_none() {
                        let ui = &mut self.ui.options.mod_browser;
                        self.mod_jobs.install(ui, Self::game_dir(), id);
                    }
                }
                // The website, through ConfirmLinkScreen as other links.
                Event::OpenModPage(url) => self.confirm_trusted_link(&url, Screen::Options),
                Event::ListShaderPacks => {
                    let (key, _) = sunrise_client::shader_pack_list::resolve(
                        &Self::shader_packs_dir(),
                        &self.settings.sunrise.shader_pack,
                    );
                    if key != self.settings.sunrise.shader_pack {
                        self.settings.sunrise.shader_pack = key;
                        self.settings.save(&Self::options_path());
                    }
                    self.settings.shader_packs =
                        sunrise_client::shader_pack_list::scan(&Self::shader_packs_dir());
                }
                Event::ListMods => self.list_copied_native_mods(),
                Event::ModToggled(i) => self.mod_toggled(i),
                Event::NativeGame(act) => self.native_game_button(act),
                // Saved; the next frame's `shader_pack_inputs` loads the pick.
                Event::ShaderPack => self.settings.save(&Self::options_path()),
                Event::OpenCustomButtons => self.open_custom_buttons(),
                Event::CustomButtons => self.save_custom_buttons(),
                Event::CustomImage(id, pixels) => {
                    if let Err(e) = sunrise_client::own_buttons::save(id, &pixels) {
                        log::warn!("Sunrise: the button image was not saved: {e}");
                    }
                    self.save_custom_buttons();
                }
                Event::ImportButtonImage(id) => self.pick_button_image(id),
                Event::OpenShaderOptions => self.open_shader_options(),
                Event::ShaderOptions => self.save_shader_options(),
                Event::NativePackOptions(i) => {
                    if let Some(pack) = self.settings.shader_packs.get(i) {
                        pack.save_values(&Self::shader_packs_dir());
                    }
                }
                Event::OpenShaderPackFolder => self.open_game_folder(&Self::shader_packs_dir()),
                Event::ImportGpuDriver => {
                    sunrise_client::platform::pick_file();
                }
                Event::RestartGame => {
                    #[cfg(target_os = "android")]
                    crate::android::restart_app();
                }
                Event::NativeSettings(i) => self.native_settings_refresh(i),
                Event::NativeSetting(i, j) => self.native_setting_event(i, j),
                Event::Credits => self.open_win_screen(false, super::win::Finish::Options),
                Event::HudEditor => self.open_hud_editor(true),
                // AccessibilityOptionsScreen's link button: ConfirmLinkScreen
                // back to the options screen.
                Event::AccessibilityHelp => {
                    self.confirm_trusted_link(ACCESSIBILITY_HELP_LINK, Screen::Options)
                }
                Event::Narrator => self.narrator_status_changed(),
                Event::OpenStudioReplay(path) => {
                    let path = path.to_string_lossy().to_string();
                    if let Err(e) = self.open_studio_replay(&path) {
                        log::warn!("Sunrise Studio: could not open {path}: {e}");
                    }
                }
                Event::OpenStudioFolder => {
                    let dir = std::path::PathBuf::from("recordings");
                    let _ = std::fs::create_dir_all(&dir);
                    super::packs::open_folder(&dir);
                }
            }
        }
        for action in self.ui.options.take_skin_actions() {
            match action {
                SkinAction::Name(name) => self.keep_player_name(&name),
                SkinAction::Choice(choice) => self.choose_own_skin(choice),
                SkinAction::Cape(choice) => {
                    #[cfg(target_os = "android")]
                    self.choose_own_cape(choice);
                    #[cfg(not(target_os = "android"))]
                    let _ = choice;
                }
                SkinAction::PickCape => {
                    #[cfg(target_os = "android")]
                    self.pick_wardrobe_png(crate::ha_launcher::skins::library::Kind::Cape);
                }
                SkinAction::LoadAccountCapes =>
                {
                    #[cfg(target_os = "android")]
                    if let Err(error) = self.account_capes(None) {
                        self.ui.options.skin_warning(error);
                    }
                }
                SkinAction::SaveWardrobe { cape, skin } => {
                    #[cfg(target_os = "android")]
                    self.save_wardrobe(cape, skin);
                    #[cfg(not(target_os = "android"))]
                    let _ = (cape, skin);
                }
                SkinAction::OpenFolder => self.open_game_folder(&Self::skins_dir()),
                SkinAction::OpenWardrobeFolder => {
                    #[cfg(target_os = "android")]
                    self.open_game_folder(&Self::wardrobe_root().join("images"));
                }
                SkinAction::PickFile => self.pick_skin_file(),
                SkinAction::ElyBy(on) => self.keep_elyby(on),
                SkinAction::Upload(choice) => {
                    if let Err(error) = self.upload_own_skin(choice) {
                        self.ui.options.skin_warning(error);
                    }
                }
                SkinAction::ElySite => self.open_ely_skins(),
            }
        }
        Ok(())
    }
    /// LanguageManager.setSelected + reload: translations, then re-wrap chat.
    pub(super) fn load_language(&mut self, code: &str) {
        lang::set(lang::Language::load(&[&self.pack.source], code));
        self.refresh_remote_language();
        self.ui.chat.rescale(&self.gui.atlas);
        self.creative_screen.forget_items();
    }
    /// Window.updateFullscreenIfChanged: only a changed option asks the
    /// window to change, so applying settings never undoes a fullscreen
    /// the compositor set up.
    fn apply_fullscreen(&mut self) {
        if let Some(w) = self.window.clone() {
            let monitor = w.current_monitor().or_else(|| w.primary_monitor());
            if let Some(modes) = self
                .video_modes
                .refresh(monitor, sunrise_client::window::monitor_video_modes)
            {
                self.settings.video_modes = modes;
            }
            let in_video_screen = self.screen == Screen::Options && self.ui.options.video_open();
            let wanted = self.settings.fullscreen;
            let exclusive = self.settings.file.bool("exclusiveFullscreen");
            let preferred = self
                .settings
                .file
                .fullscreen_resolution
                .as_deref()
                .and_then(sunrise_client::window::VideoMode::read);
            self.fullscreen
                .apply(&w, wanted, exclusive, preferred, in_video_screen);
        }
    }
    /// Settings that Sunrise applies beyond render distance, volume and GUI scale.
    pub(super) fn apply_ui_settings(&mut self) {
        let scale = self.world_scale();
        sunrise_client::parallel::set_setting(self.settings.sunrise.parallel_tick);
        if sunrise_client_render::mipmap::set_dither_cutouts(
            self.settings.sunrise.dither_cutout_mips,
        ) {
            self.reload_packs();
        }
        self.apply_fullscreen();
        // A failed HDR start that named no driver becomes the window's driver's, and another
        // driver's is dropped, so HDR output is tried again.
        if let Some(driver) = self.renderer.as_ref().and_then(|r| r.hdr_driver())
            && self.settings.sunrise.settle_failed_start(driver)
            && let Some(dir) = Self::options_path().parent()
        {
            self.settings.sunrise.save(dir);
        }
        self.hold_fullbright();
        let s = &self.settings;
        if let Some(r) = self.renderer.as_mut() {
            r.gamma = s.gamma as f32;
            r.improved_transparency = s.improved_transparency;
            r.set_present_mode(s.vsync, s.sunrise.present_mode);
            r.animated_textures = s.sunrise.animated_textures;
            r.screenshot_quality = s.sunrise.screenshot_quality;
            r.set_mipmap_levels(s.mipmap_levels.max(0) as u32);
            r.set_texture_filtering(
                s.file.int("textureFiltering"),
                s.file.int("maxAnisotropyBit"),
            );
            // Dynamic Resolution turned off starts over at the floor.
            if !s.sunrise.dynamic_resolution {
                self.dynamic_resolution.reset();
            }
            r.set_world_post(
                scale,
                s.sunrise.fxaa,
                s.sunrise.taa,
                s.sunrise.upscaling.ratio(),
                s.sunrise.upscaling_sharpness as f32 / 100.0,
            );
            r.set_frame_generation(
                s.sunrise.frame_generation != sunrise_client::graphics::FrameGeneration::Off,
            );
            r.set_hdr_lighting(s.sunrise.hdr_lighting);
            // Where it is offered (`Renderer::gpu_culling_offer`); unchanged, it does nothing.
            // Auto has it on while its measurements say so (`GpuCullingAuto`), and starts over
            // when the choice is another one.
            let gpu_culling = match s.sunrise.gpu_culling {
                sunrise_client::graphics::GpuCullingMode::Off => false,
                sunrise_client::graphics::GpuCullingMode::On => true,
                sunrise_client::graphics::GpuCullingMode::Auto => self.gpu_culling_auto.on(),
            };
            if s.sunrise.gpu_culling != sunrise_client::graphics::GpuCullingMode::Auto {
                self.gpu_culling_auto.reset();
            }
            let _ = r.set_gpu_culling(gpu_culling);
            r.update_output(
                &s.sunrise,
                &Self::options_path().with_file_name(sunrise_client::graphics::HDR_MARKER),
            );
        }
        let mut volumes = s.volumes.map(|gain| gain as f32);
        volumes[0] = 1.0;
        sunrise_client::settings::set_source_volumes(volumes);
        super::narrator::sync(s);
        let chat = s.chat_options();
        let rescale = {
            let o = &self.ui.chat.options;
            o.width != chat.width || o.scale != chat.scale || o.line_spacing != chat.line_spacing
        };
        self.ui.chat.options = chat;
        if rescale {
            self.ui.chat.rescale(&self.gui.atlas);
        }
        let font = &mut self.gui.atlas.font;
        if font.uniform != s.force_unicode || font.japanese != s.japanese_glyphs {
            font.uniform = s.force_unicode;
            font.japanese = s.japanese_glyphs;
            font.reset_cache();
            self.ui.chat.rescale(&self.gui.atlas);
        }
        self.notify_window_resize();
        // The GUI scale is part of the screen's size (`Screen.resize`): a
        // change remakes its build before this frame draws it.
        self.build_screen();
    }
    pub(super) fn resize_state_of(&self, size: winit::dpi::PhysicalSize<u32>) -> ResizeState {
        let viewport = self
            .renderer
            .as_ref()
            .map_or([size.width, size.height], |r| {
                [r.config.width, r.config.height]
            });
        ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new(
                viewport,
                self.settings.gui_scale,
                self.settings.force_unicode,
            )
            .size,
        }
    }
    /// Fires `window_resize` once when the physical or GUI-scaled dimensions change.
    pub(super) fn notify_window_resize(&mut self) {
        let Some(window) = &self.window else { return };
        let now = self.resize_state_of(window.inner_size());
        if now.differs_from(&mut self.resize_state) {
            self.native_window_resize(
                now.size.width,
                now.size.height,
                now.gui_size[0],
                now.gui_size[1],
            );
            self.build_screen();
        }
    }
    /// `Window.updateFullscreenState`: a fullscreen change made by the window
    /// system is adopted by the option (`Minecraft.fullscreenStateChanged`).
    pub(super) fn observe_fullscreen(&mut self) {
        // HDR calibration's own fullscreen is not the player's option: it
        // goes back to what it was when the calibration ends.
        if self.ui.options.calibration_window.is_some() {
            return;
        }
        let Some(w) = &self.window else { return };
        if let Some(now) = self.fullscreen.observe(w.fullscreen().is_some())
            && self.settings.fullscreen != now
        {
            self.settings.fullscreen = now;
            self.settings.save(&Self::options_path());
            self.notify_window_resize();
        }
    }
    /// F11 (key.fullscreen): Window.toggleFullScreen, remembered in options.txt.
    pub(super) fn toggle_fullscreen(&mut self) {
        self.settings.fullscreen = !self.settings.fullscreen;
        self.settings.save(&Self::options_path());
        self.apply_ui_settings();
    }
    /// The Extras page's shader pack key: turns the picked pack off, or asks for the last
    /// pick again. The same persistent pick the Shader Packs screen writes: turning off keeps
    /// the last non-empty pick, resuming restores it, and the off state survives a restart.
    /// With `SUNRISE_SHADER_PACK` set the developer override wins and the key does nothing.
    /// Invalidation stays with the normal selection path: the next frame's `shader_pack_inputs`
    /// sees the changed pick and drops or loads the pack itself, for either pack kind.
    pub(super) fn toggle_shader_pack(&mut self) {
        let present = |key: &str| {
            sunrise_client::shader_pack_list::resolve(&Self::shader_packs_dir(), key)
                .1
                .exists()
        };
        let plan = plan_shader_toggle(
            &self.settings.sunrise.shader_pack,
            &self.settings.extras.last_shader_pack,
            dev_pack().is_some(),
            present,
            |key| !active_pack(key, &self.settings.mods).is_empty(),
        );
        match plan {
            TogglePlan::Noop => {}
            TogglePlan::Hint => {
                self.system_message(Text::tr("sunrise.shaders.noPack", &[]));
            }
            TogglePlan::HintClearLast => {
                self.settings.extras.last_shader_pack.clear();
                self.settings.save(&Self::options_path());
                self.system_message(Text::tr("sunrise.shaders.noPack", &[]));
            }
            TogglePlan::HintModOff(name) => {
                self.system_message(Text::tr("sunrise.shaders.modOff", &[&name]));
            }
            TogglePlan::Off(name) => {
                self.settings.set_shader_pick("");
                self.settings.save(&Self::options_path());
                self.system_message(Text::tr("sunrise.shaders.suspended", &[&name]));
            }
            TogglePlan::On(name) => {
                self.settings.set_shader_pick(&name);
                self.settings.save(&Self::options_path());
                self.system_message(Text::tr("sunrise.shaders.resumed", &[&name]));
            }
        }
    }
    /// `FramerateLimitTracker.getFramerateLimit`: the Max Framerate option,
    /// throttled when minimised, idle, or in a menu without a world.
    pub(super) fn framerate_limit(&self) -> i32 {
        let context = FrameContext {
            inactivity: if self.settings.inactivity_fps_limit == "minimized" {
                InactivityFpsLimit::Minimized
            } else {
                InactivityFpsLimit::Afk
            },
            iconified: self
                .window
                .as_ref()
                .is_some_and(|w| w.is_minimized().unwrap_or(false)),
            focused: self.focused,
            // Window.isExclusiveFullscreen.
            exclusive_fullscreen: self
                .window
                .as_ref()
                .is_some_and(|w| matches!(w.fullscreen(), Some(Fullscreen::Exclusive(_)))),
            now_ms: self.started.elapsed().as_millis() as i64,
            latest_input_ms: self.last_input.duration_since(self.started).as_millis() as i64,
            has_level: self.title.is_none(),
            has_screen: self.title.is_some() || self.screen != super::Screen::Playing,
            has_overlay: false,
        };
        let limit = framerate_limit(self.settings.max_fps as i32, context);
        let limit = self
            .settings
            .sunrise
            .battery_limit(limit, self.title.is_none(), self.focused);
        // Starling: a phone's menu over the world draws at most 60 frames a second
        // (`menu_frame_cap`). No `cfg` here: dynamic_fps_for_rust hooks this function, and
        // horizon-mixins refuses a target body with one.
        let limit = menu_frame_cap(limit, context.has_screen);
        // A native mod may only lower it (`native_framerate_limit`).
        let mut hooked = limit;
        self.native_framerate_limit(
            self.focused,
            context.iconified,
            self.title.is_none(),
            &mut hooked,
        );
        if hooked >= 1 {
            hooked.min(limit)
        } else {
            limit
        }
    }
    /// `Minecraft.runTick`'s frame limiter: `FramerateLimiter.limitDisplayFPS`
    /// below 260 frames per second, sleeping then spin-waiting until the
    /// frame's slot ends (not in scripted frame-count runs).
    pub(super) fn limit_frame(&mut self) {
        if self.options.frames.is_some() {
            return;
        }
        let limit = self.framerate_limit();
        match self.settings.sunrise.pacing {
            sunrise_client::graphics::Pacing::Precise => {
                sunrise_client::window::end_frame(&mut self.limiter, limit, &mut self.frame_clock)
            }
            sunrise_client::graphics::Pacing::Efficient => {
                let mut clock = sunrise_client::window::SleepingClock(self.frame_clock);
                sunrise_client::window::end_frame_on_grid(&mut self.limiter, limit, &mut clock);
            }
        }
    }
}

impl Game<'_> {
    /// Fullbright as this window may draw it (`fullbright_held`): the renderer's flag and the
    /// graphics screen's row. Each tick (`tick_frame_visuals`) and when settings apply, so
    /// joining or leaving a server needs no call of its own.
    pub(super) fn hold_fullbright(&mut self) {
        let held = fullbright_held(
            self.remote.as_ref().map(|_| self.integrated.is_some()),
            self.replay.playing(),
        );
        self.settings.fullbright_held = held;
        if let Some(r) = self.renderer.as_mut() {
            r.fullbright = self.settings.extras.fullbright && !held;
        }
    }
}

/// Fullbright is held off on another's server, where it would light what the server shows
/// dark: a connection without this window's integrated server, as a multiplayer server
/// or a LAN guest has. Single player, a LAN host and Replay Studio keep it.
/// `connection`: none, or whether it plays on this window's integrated server.
fn fullbright_held(connection: Option<bool>, studio: bool) -> bool {
    connection == Some(false) && !studio
}

/// The named device can be a connected pad that has not driven the game yet.
fn detected_controller_style(
    device: Option<&sunrise_client::controller_ui::Device>,
    fallback: sunrise_client::gamepad::GlyphStyle,
) -> sunrise_client::gamepad::GlyphStyle {
    device.map_or(fallback, |d| {
        sunrise_client::gamepad::GlyphStyle::of_device(d.vendor, d.product)
    })
}

#[cfg(test)]
mod resize_state_tests {
    use super::ResizeState;
    use winit::dpi::PhysicalSize;

    fn state(width: u32, unicode: bool) -> ResizeState {
        ResizeState {
            size: PhysicalSize::new(width, 720),
            gui_size: sunrise_client::window::GuiSize::new([width, 720], 3, unicode).size,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn viewport_resize_notifications_follow_each_player() {
        // Winit permits only one event loop per process, even after it is dropped.
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::options::resize_state_tests::viewport_resize_notifications_follow_each_player",
            "SUNRISE_RESIZE_STATE_CHILD",
            &["window"],
        ) {
            return;
        }
        use mod_api::{ClientHost, Hooks, ModEntry, WindowResizeEvent};
        use mod_platform::testing::with_mods;
        use std::sync::{Arc, Mutex};
        struct Listener(Arc<Mutex<Vec<(u32, u32, u32, u32)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn window_resize(&self, _: &mut dyn ClientHost, event: &mut WindowResizeEvent) {
                self.0.lock().unwrap().push((
                    event.width,
                    event.height,
                    event.gui_width,
                    event.gui_height,
                ));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut builder = winit::event_loop::EventLoop::builder();
        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11().with_any_thread(true);
        }
        let event_loop = builder.build().unwrap();
        #[allow(deprecated)]
        let window = Arc::new(
            event_loop
                .create_window(
                    winit::window::Window::default_attributes()
                        .with_visible(false)
                        .with_inner_size(PhysicalSize::new(1920, 1080)),
                )
                .unwrap(),
        );
        assert_eq!(window.inner_size(), PhysicalSize::new(1920, 1080));
        let mods = vec![ModEntry::new("resize", Box::new(Listener(events.clone())))].into();
        with_mods(mods, || {
            for (scale, expected) in [(0, (320, 360)), (2, (480, 540))] {
                super::super::native_entity_refs::tests::with_client_game(|game| {
                    game.options.offscreen = true;
                    game.window = Some(window.clone());
                    game.renderer = Some(game.new_renderer(window.clone()).unwrap());
                    game.renderer.as_mut().unwrap().resize(960, 1080);
                    game.settings.gui_scale = scale;
                    game.settings.force_unicode = false;
                    game.notify_window_resize();
                    game.notify_window_resize();
                    assert_eq!(
                        events.lock().unwrap().pop(),
                        Some((1920, 1080, expected.0, expected.1))
                    );
                    assert!(events.lock().unwrap().is_empty());
                    game.renderer.as_mut().unwrap().resize(1920, 1080);
                    game.notify_window_resize();
                    let gui = sunrise_client::window::GuiSize::new([1920, 1080], scale, false).size;
                    assert_eq!(
                        events.lock().unwrap().pop(),
                        Some((1920, 1080, gui[0], gui[1]))
                    );
                    game.notify_window_resize();
                    assert!(events.lock().unwrap().is_empty());
                });
            }
        });
    }

    /// One resize event that also flips fullscreen is observed twice (by the fullscreen
    /// observer and by the resize handler) but reported once.
    #[test]
    fn overlapping_observers_report_a_change_once() {
        let mut last = Some(state(640, false));
        let now = state(800, true);
        assert!(now.differs_from(&mut last));
        assert!(!now.differs_from(&mut last));
    }

    /// A settings-only change at an unchanged physical size still reports.
    #[test]
    fn a_settings_only_change_reports() {
        let mut last = Some(state(960, false));
        assert!(!state(960, false).differs_from(&mut last));
        assert!(state(960, true).differs_from(&mut last));
    }
    #[test]
    fn settings_that_leave_dimensions_unchanged_do_not_report() {
        let size = PhysicalSize::new(1280, 960);
        let before = ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new([1280, 960], 2, false).size,
        };
        let mut last = Some(before);
        let unicode = ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new([1280, 960], 2, true).size,
        };
        assert!(!unicode.differs_from(&mut last));
        let automatic = ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new([1280, 960], 0, false).size,
        };
        assert!(automatic.differs_from(&mut last));
        let larger_limit = ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new([1280, 960], 100, false).size,
        };
        assert!(!larger_limit.differs_from(&mut last));
    }
}

#[cfg(test)]
mod fullbright_tests {
    use super::fullbright_held;

    #[test]
    fn fullbright_is_held_only_on_another_servers_world() {
        // The title screen, with no connection.
        assert!(!fullbright_held(None, false));
        // Single player and a LAN host play their own save.
        assert!(!fullbright_held(Some(true), false));
        // A multiplayer server, or another player's world opened to LAN.
        assert!(fullbright_held(Some(false), false));
        // Replay Studio plays a recording, not a server.
        assert!(!fullbright_held(Some(false), true));
    }
}

#[cfg(test)]
mod controller_context_tests {
    use super::detected_controller_style;
    use sunrise_client::{controller_ui::Device, gamepad::GlyphStyle};

    #[test]
    fn detected_style_follows_the_named_device_even_before_it_is_used() {
        let mut device = Device {
            name: "pad".into(),
            vendor: Some(0x054c),
            product: None,
        };
        assert_eq!(
            detected_controller_style(Some(&device), GlyphStyle::Xbox),
            GlyphStyle::PlayStation
        );
        device.vendor = Some(0x057e);
        assert_eq!(
            detected_controller_style(Some(&device), GlyphStyle::PlayStation),
            GlyphStyle::Nintendo
        );
        assert_eq!(
            detected_controller_style(None, GlyphStyle::PlayStation),
            GlyphStyle::PlayStation
        );
    }
}

#[cfg(test)]
mod menu_frame_cap_tests {
    use super::menu_frame_cap;

    /// A phone's menu over the world draws at most 60 FPS; the world itself, and a computer's
    /// menu, keep the limit.
    #[test]
    fn a_menu_caps_the_frame_rate_on_a_phone_only() {
        let capped = if cfg!(target_os = "android") { 60 } else { 300 };
        assert_eq!(menu_frame_cap(300, true), capped);
        assert_eq!(menu_frame_cap(300, false), 300);
        assert_eq!(menu_frame_cap(30, true), 30);
    }
}
