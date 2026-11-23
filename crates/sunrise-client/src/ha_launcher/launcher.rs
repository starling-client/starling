//! A native Rust installer and version screen, sharing the game's Vulkan renderer.
use crate::ha_launcher::catalog::{Catalog, Page};
use crate::ha_launcher::game::{self, Renderer};
use crate::ha_launcher::skin_head::AvatarHead as _;
#[path = "chooser.rs"]
mod chooser;
#[path = "launcher_gesture.rs"]
mod gesture;
#[path = "login.rs"]
mod login;
#[path = "launcher_pages.rs"]
mod pages;
#[path = "wardrobe.rs"]
mod wardrobe;
use crate::ha_launcher::client::{
    Result,
    hud::{HudAtlas, HudBatch},
    ui_theme::{Face, Ui},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Window, WindowId},
};

pub(super) struct Launcher {
    home: PathBuf,
    app_version: String,
    controller_edges: crate::ha_launcher::client::gamepad::Edges,
    controller_repeat: Option<(crate::ha_launcher::client::gamepad::Button, Instant, bool)>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: Option<game::UiResources>,
    gui_loader: Option<std::thread::JoinHandle<std::result::Result<game::UiResources, String>>>,
    pub(super) audio: Option<crate::ha_launcher::client::audio::Audio>,
    cursor: (f64, f64),
    mouse_down: bool,
    feedback: Option<(pages::Action, Instant)>,
    install: Arc<Mutex<crate::ha_launcher::client::official_install::InstallProgress>>,
    install_paused: Arc<std::sync::atomic::AtomicBool>,
    install_cancelled: Arc<std::sync::atomic::AtomicBool>,
    install_transfer: crate::ha_launcher::catalog::TransferTracker,
    pub(super) anim: pages::AnimState,
    pub(super) control_editor: Option<crate::ha_launcher::client::touch::TouchControls>,
    worker: Option<mpsc::Receiver<std::result::Result<(), String>>>,
    error: Option<String>,
    catalog: Catalog,
    targets: Vec<([f32; 4], pages::Action)>,
    list_targets: Vec<([f32; 4], pages::Action)>,
    list_view: Option<[f32; 3]>,
    pending_focus: Option<pages::Action>,
    drag: Option<gesture::Drag>,
    chooser: Option<chooser::Chooser>,
    exit_confirmation: bool,
    setup_destination_open: bool,
    login: login::Login,
    wardrobe: wardrobe::Wardrobe,
    pub(super) launching: bool,
    pub(super) launch_pending_tick: bool,
    pub(super) diagnostics_modal_open: bool,
    focus: Option<usize>,
    diagnostic_export: Option<mpsc::Receiver<std::result::Result<String, String>>>,
}

impl Launcher {
    pub(super) fn refresh_wardrobe_account(&mut self) {
        if self.catalog.page == Page::Skins {
            self.catalog.scroll = 0.0;
        }
        // The header shows this appearance even when another page is open.
        self.wardrobe.open(
            &self.catalog.config.name,
            &self.catalog.config.microsoft_client_id,
        );
    }

    fn edit_text(&mut self, output: crate::ha_launcher::client::osk::Output) {
        if let Some(tc) = &mut self.control_editor
            && crate::ha_launcher::client::touch_ui::apply_editor_or_preset_output(
                tc.editor.as_mut(),
                None,
                &output,
            )
        {
            let game_home = self.catalog.config.game_home(&self.home);
            let _ = tc.settings.save_to(&game_home.join("sunrise-touch.json"));
            if crate::ha_launcher::client::touch_ui::output_dismisses(&output) {
                crate::ha_launcher::platform::hide();
            }
            return;
        }
        if self.login.edit(self.catalog.editing, &output) {
            if matches!(
                output,
                crate::ha_launcher::client::osk::Output::Enter
                    | crate::ha_launcher::client::osk::Output::Close
            ) {
                self.catalog.editing = None;
                crate::ha_launcher::platform::hide();
            }
        } else {
            self.catalog.edit(output, &self.home);
        }
    }

    pub(super) fn new(home: PathBuf) -> Self {
        // Upgrade existing installations and pre-verify structure transforms in the background
        // so pressing Play launches immediately without main-thread I/O stalls.
        let cleanup_home = home.clone();
        std::thread::spawn(move || {
            if let Err(error) =
                crate::ha_launcher::client::official_install::cleanup_completed_downloads(
                    &cleanup_home,
                )
            {
                log::warn!("Could not clean completed Minecraft downloads: {error}");
            }
            if crate::ha_launcher::client::official_install::ready(&cleanup_home) {
                let _ = crate::ha_launcher::client::android_resources::repair_structure_transforms(
                    &cleanup_home.join("engine-data.zip"),
                    &cleanup_home
                        .join(".local/sunrise-client")
                        .join(sunrise_core::VANILLA_VERSION),
                );
            }
        });
        let mut catalog = Catalog::new(&home);
        if crate::ha_launcher::diagnostics::previous_unclean(&home) {
            catalog.status = crate::ha_launcher::diagnostics::summary(&home);
        }
        crate::ha_launcher::client::android_runtime::graphics_startup::configure(home.clone());
        if crate::ha_launcher::client::android_runtime::graphics_startup::recover(&home) {
            catalog.config.renderer =
                crate::ha_launcher::catalog::RendererMode::VulkanCompatibility;
            if let Err(error) = catalog.config.save(&home) {
                log::warn!("Could not save graphics recovery choice: {error}");
            }
            catalog.status = "Previous Vulkan startup was interrupted. Using Vulkan compatibility; you can change Rendering mode in Settings.".into();
        }
        crate::ha_launcher::configure_input_mode(catalog.config.input_host);
        crate::ha_launcher::device_runtime::configure(catalog.config.device);
        crate::ha_launcher::client::ui_theme::set_ui_scale_percent(catalog.config.ui_scale_percent);
        crate::ha_launcher::client::ui_theme::motion::set_launcher_animations_disabled(
            catalog.config.disable_launcher_animations,
        );
        crate::ha_launcher::client::ui_theme::motion::set_game_animations_disabled(
            catalog.config.disable_game_animations,
        );
        // First run: ensure independent MCPE and GoyControl storage directories exist,
        // and initialize MCPE Controls as the default active control system on fresh installs.
        let game_home = catalog.config.game_home(&home);
        crate::ha_launcher::client::touch::Settings::ensure_system_storage(
            &game_home,
            crate::ha_launcher::client::touch::ControlStyle::Mcpe,
        );
        crate::ha_launcher::client::touch::Settings::ensure_system_storage(
            &game_home,
            crate::ha_launcher::client::touch::ControlStyle::GoyControl,
        );
        let touch_file = game_home.join("sunrise-touch.json");
        if !touch_file.exists() {
            let touch = crate::ha_launcher::client::touch::Settings::default_mcpe();
            if let Some(dir) = touch_file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(error) = touch.save_to(&touch_file) {
                log::warn!("Cannot write the factory touch layout: {error}");
            }
        }
        // Load the selected wardrobe and its last head before the first draw:
        // the account pill must have the real face instantly, not after a page visit.
        let mut wardrobe = wardrobe::Wardrobe::new(&home);
        wardrobe.open(&catalog.config.name, &catalog.config.microsoft_client_id);
        Self {
            wardrobe,
            home,
            app_version: crate::android::app_version()
                .map_or_else(String::new, |(version, _)| version),
            controller_edges: Default::default(),
            controller_repeat: None,
            window: None,
            renderer: None,
            gui: None,
            gui_loader: None,
            audio: None,
            cursor: (0.0, 0.0),
            mouse_down: false,
            feedback: None,
            install: Arc::default(),
            install_paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            install_cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            install_transfer: Default::default(),
            anim: Default::default(),
            control_editor: None,
            worker: None,
            error: None,
            catalog,
            targets: vec![],
            list_targets: vec![],
            list_view: None,
            pending_focus: None,
            drag: None,
            chooser: None,
            exit_confirmation: false,
            setup_destination_open: false,
            login: login::Login::new(),
            launching: false,
            launch_pending_tick: false,
            diagnostics_modal_open: false,
            focus: None,
            diagnostic_export: None,
        }
    }

    pub(super) fn configure_screen(&self) {
        crate::ha_launcher::native_ime::configure_screen(
            self.catalog.config.screen.immersive,
            self.catalog.config.screen.avoid_cutout,
        );
        crate::android::apply_display_settings(&self.catalog.config.game_settings(&self.home));
    }
    fn fail(&mut self, error: impl std::fmt::Display) {
        log::error!("{error}");
        self.launching = false;
        self.launch_pending_tick = false;
        crate::ha_launcher::diagnostics::record_error(&error.to_string());
        self.error = Some(error.to_string());
        let _ = std::fs::write(self.home.join("android-error.txt"), error.to_string());
    }

    fn open(&mut self, events: &ActiveEventLoop) -> Result<()> {
        self.configure_screen();
        crate::ha_launcher::client::ui_theme::set_android_dpi(
            crate::ha_launcher::platform::display_density(),
        );
        crate::ha_launcher::client::ui_theme::set_ui_scale_percent(
            self.catalog.config.ui_scale_percent,
        );
        crate::ha_launcher::client::android_runtime::configure_gpu(false);
        if let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) {
            if let Some(size) = crate::ha_launcher::display::prepare(window)? {
                renderer.resize(size[0], size[1]);
            }
            renderer.recreate_surface()?;
            let size = window.inner_size();
            renderer.resize(size.width, size.height);
            return Ok(());
        }
        let window =
            Arc::new(events.create_window(Window::default_attributes().with_title("Starling"))?);
        crate::ha_launcher::display::prepare(&window)?;
        // Starling's prepared resources, when there are any yet: the launcher shows before the
        // first preparing, with the theme's own faces (the boot screen's atlas).
        let resources = crate::ha_launcher::client::logging::private_home().join("resources");
        let source = resources
            .join("assets/minecraft/textures/gui")
            .is_dir()
            .then(|| {
                crate::ha_launcher::client::resource_source::ResourceSource::directory(resources)
            });
        let mut renderer = game::loading_renderer(
            window.clone(),
            false,
            None,
            None,
            None,
            if self.catalog.config.renderer == crate::ha_launcher::catalog::RendererMode::OpenGl {
                crate::ha_launcher::client::graphics::GraphicsApi::OpenGl
            } else {
                crate::ha_launcher::client::graphics::GraphicsApi::Auto
            },
        )?;
        let panorama = Arc::new(crate::ha_launcher::client::gfx::panorama::Faces::colour([
            0, 0, 0, 255,
        ]));
        let gui = game::UiResources {
            atlas: HudAtlas::boot(),
            panorama: panorama.clone(),
            sounds: None,
        };
        self.gui_loader = source.and_then(|source| {
            std::thread::Builder::new()
                .name("launcher-assets".into())
                .spawn(move || {
                    Ok(game::UiResources {
                        atlas: HudAtlas::build(&source).map_err(|error| error.to_string())?,
                        panorama,
                        sounds: Some(
                            crate::ha_launcher::client::sound_events::SoundRegistry::load(&source),
                        ),
                    })
                })
                .inspect_err(|error| {
                    log::warn!("Launcher: could not start UI asset loading: {error}")
                })
                .ok()
        });
        renderer.set_loading_ui(&gui.atlas, gui.panorama.as_ref())?;
        self.renderer = Some(renderer);
        self.window = Some(window);
        self.gui = Some(gui);
        Ok(())
    }

    fn poll_gui_assets(&mut self) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if !renderer.has_presented()
            || self
                .gui_loader
                .as_ref()
                .is_none_or(|worker| !worker.is_finished())
        {
            return;
        }
        let Some(worker) = self.gui_loader.take() else {
            return;
        };
        match worker.join() {
            Ok(Ok(gui)) => match renderer.set_loading_ui(&gui.atlas, gui.panorama.as_ref()) {
                Ok(()) => {
                    crate::ha_launcher::client::ui_theme::reset_atlas();
                    self.gui = Some(gui);
                }
                Err(error) => log::warn!("Launcher: could not use the prepared GUI atlas: {error}"),
            },
            Ok(Err(error)) => {
                log::warn!("Launcher: could not load the prepared GUI atlas: {error}")
            }
            Err(_) => log::warn!("Launcher: GUI asset loading stopped unexpectedly"),
        }
    }

    fn request_back(&mut self) {
        if crate::ha_launcher::platform::active() && self.control_editor.is_some() {
            crate::ha_launcher::platform::hide();
            return;
        }
        crate::ha_launcher::platform::hide();
        if let Some(tc) = &mut self.control_editor {
            let game_home = self.catalog.config.game_home(&self.home);
            if tc.request_editor_back() {
                let _ = tc.settings.save_to(&game_home.join("sunrise-touch.json"));
                self.control_editor = None;
            }
            return;
        }
        if !self.anim.intro_done {
            self.anim.intro_done = true;
            return;
        }
        if self.diagnostics_modal_open {
            self.diagnostics_modal_open = false;
        } else if self.setup_destination_open {
            self.setup_destination_open = false;
        } else if self.exit_confirmation {
            self.exit_confirmation = false;
        } else if self.catalog.fm_modal.take().is_some() {
            self.catalog.editing = None;
        } else if self.catalog.project_modal.take().is_some() {
            self.catalog.scroll = 0.0;
        } else if self.login.visible {
            self.login.hide();
        } else if self.chooser.take().is_some() {
            // Back dismisses the current dialog, including the exit question.
        } else if self.catalog.page == Page::Installed
            && self.catalog.installed_sub_tab
                == crate::ha_launcher::catalog::InstalledSubTab::FileManager
            && self.catalog.fm_back(&self.home)
        {
            // Back in File Manager pops the directory history stack.
        } else if self.catalog.page == Page::Settings
            && self.catalog.settings_category
                != crate::ha_launcher::catalog::SettingsCategory::Overview
        {
            self.catalog.settings_category = if self.catalog.settings_category
                == crate::ha_launcher::catalog::SettingsCategory::Theme
            {
                crate::ha_launcher::catalog::SettingsCategory::Launcher
            } else {
                crate::ha_launcher::catalog::SettingsCategory::Overview
            };
            self.catalog.scroll = 0.0;
            self.anim.trigger_page_transition();
        } else if self.catalog.editing.take().is_none() {
            self.exit_confirmation = true;
        }
        self.focus = None;
        self.pending_focus = self.exit_confirmation.then_some(pages::Action::Dismiss);
        self.targets.clear();
        self.list_targets.clear();
        self.drag = None;
        if let Err(error) = self.catalog.config.save(&self.home) {
            self.fail(error);
        }
    }

    fn action(&mut self, _events: &ActiveEventLoop) -> Result<()> {
        if self.exit_confirmation
            || self.setup_destination_open
            || self.chooser.is_some()
            || self.worker.is_some()
            || self.launching
            || self.login.pending()
        {
            return Ok(());
        }
        if !crate::ha_launcher::client::official_install::ready(&self.home)
            && matches!(
                self.catalog.config.onboarding,
                crate::ha_launcher::catalog::OnboardingState::Unseen
                    | crate::ha_launcher::catalog::OnboardingState::Pending
            )
        {
            self.focus = None;
            self.setup_destination_open = true;
            return Ok(());
        }
        crate::ha_launcher::diagnostics::heartbeat(
            "play_requested",
            || serde_json::json!({"renderer":self.catalog.config.renderer.label(),"resolution_percent":self.catalog.config.game_render_scale(&self.home).percent()}),
        );
        self.launching = true;
        self.error = None;
        self.launch_pending_tick = true;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    fn launch_game(&mut self) -> Result<()> {
        let installed = crate::ha_launcher::client::official_install::ready(&self.home);
        crate::ha_launcher::configure_input_mode(self.catalog.config.input_host);
        crate::ha_launcher::device_runtime::configure(self.catalog.config.device);
        crate::ha_launcher::platform::hide();
        self.catalog.config.save(&self.home)?;
        if installed {
            self.catalog.config.apply_experimental_to_game(&self.home);
            let game_home = self.catalog.config.game_home(&self.home);
            std::fs::create_dir_all(&game_home)?;
            // Migrate the earlier launcher preset once; subsequent edits belong to Key Binds.
            let keypad_path = game_home.join("sunrise-keypad.json");
            if self.catalog.config.keypad_controls && !keypad_path.exists() {
                crate::ha_launcher::client::keypad::Settings {
                    enabled: true,
                    ..Default::default()
                }
                .save(&keypad_path)?;
            }
            self.catalog.config.keypad_controls = false;
            if crate::ha_launcher::account::Accounts::load()
                .selected()
                .is_none()
            {
                crate::ha_launcher::appearance::play_offline(&self.catalog.config.name);
            }
            // Use local choices and any appearance already in memory. The game renews the
            // selected account itself, so a background wardrobe request never blocks Play.
            if let Err(error) = self
                .wardrobe
                .prepare_game(&self.catalog.config.name, &game_home)
            {
                log::warn!("Launcher: could not apply wardrobe choice: {error}");
            }
            self.controller_edges = Default::default();
            self.controller_repeat = None;
        }
        crate::ha_launcher::play(installed)?;
        let _ = std::fs::remove_file(self.home.join("android-error.txt"));
        Ok(())
    }
}

impl ApplicationHandler for Launcher {
    fn resumed(&mut self, events: &ActiveEventLoop) {
        self.configure_screen();
        if let Err(error) = self.open(events) {
            self.fail(error);
            events.exit();
        }
    }
    fn suspended(&mut self, _events: &ActiveEventLoop) {
        crate::ha_launcher::diagnostics::heartbeat("suspended", || serde_json::json!({}));
        self.drag = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.suspend_surface();
        }
    }
    fn window_event(&mut self, events: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if crate::ha_launcher::documents::handle_event(&event) {
            return;
        }
        if matches!(event, WindowEvent::Resized(_)) {
            let _ = crate::ha_launcher::native_ime::viewport_bottom();
        }
        if matches!(event, WindowEvent::Focused(true)) {
            self.configure_screen();
        }
        let result = match event {
            WindowEvent::CloseRequested => {
                self.request_back();
                Ok(())
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                Ok(())
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.focus = None;
                self.pending_focus = None;
                self.cursor = (position.x, position.y);
                Ok(())
            }
            WindowEvent::Touch(touch) => self.touch_event(events, touch),
            WindowEvent::Focused(false) => {
                crate::ha_launcher::diagnostics::heartbeat("launcher_background", || {
                    serde_json::json!({})
                });
                crate::ha_launcher::client::android_runtime::clear_gamepads();
                self.controller_repeat = None;
                self.mouse_down = false;
                self.drag = None;
                Ok(())
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Pressed {
                    self.mouse_down = true;
                    Ok(())
                } else if std::mem::take(&mut self.mouse_down) {
                    self.activate(events)
                } else {
                    Ok(())
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.exit_confirmation {
                    return;
                }
                let y = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y * 42.0,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / self.ui_density(),
                };
                if let Some(chooser) = &mut self.chooser {
                    chooser.scroll = (chooser.scroll - y).max(0.0);
                } else {
                    self.catalog.scroll = (self.catalog.scroll - y).max(0.0);
                }
                Ok(())
            }
            WindowEvent::RedrawRequested => self.draw_pages(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if event.physical_key
                    == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Escape)
                    || event.logical_key
                        == winit::keyboard::Key::Named(winit::keyboard::NamedKey::BrowserBack)
                {
                    self.type_key(event);
                    Ok(())
                } else if self.navigate(&event) {
                    Ok(())
                } else if self.catalog.editing.is_none()
                    && !event.repeat
                    && matches!(
                        event.logical_key,
                        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter)
                    )
                {
                    self.activate(events)
                } else {
                    self.type_key(event);
                    Ok(())
                }
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                self.edit_text(crate::ha_launcher::client::osk::Output::Text(text));
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.fail(error);
        }
    }
    fn device_event(&mut self, events: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        if matches!(event, DeviceEvent::Removed) {
            self.mouse_down = false;
            self.drag = None;
        }
        let _ = (events, id);
    }
    fn about_to_wait(&mut self, events: &ActiveEventLoop) {
        crate::ha_launcher::diagnostics::heartbeat("launcher", || serde_json::json!({}));
        self.poll_gui_assets();
        if let Some(result) = self
            .diagnostic_export
            .as_ref()
            .and_then(|r| r.try_recv().ok())
        {
            self.diagnostic_export = None;
            self.catalog.status = match result {
                Ok(name) => {
                    // The share sheet, as the game's Diagnostics opens it: the report is in the
                    // app's own files, which the player can't reach otherwise.
                    let _ = crate::android::share_diagnostic(&name);
                    format!(
                        "Saved {name} in Reports. Review the ZIP before sharing; nothing was uploaded."
                    )
                }
                Err(error) => format!("Diagnostic export failed: {error}"),
            };
        }
        crate::ha_launcher::platform::poll_gamepad_connections();
        if let Err(error) = self.controller_frame(events) {
            self.fail(error);
        }
        if let (Some(window), Some(renderer)) = (&self.window, &mut self.renderer)
            && let Err(error) = crate::ha_launcher::display::apply(window, renderer)
        {
            crate::ha_launcher::display::set_scale(
                crate::ha_launcher::display::RenderScale::default(),
            );
            self.fail(error);
        }
        if let Some(result) = self.worker.as_ref().and_then(|w| w.try_recv().ok()) {
            self.worker = None;
            if let Err(error) = result {
                let message = error.to_string();
                if message.contains("Dns Failed") || message.contains("lookup address") {
                    self.fail("Minecraft download interrupted: DNS could not resolve the download host. Check your connection, VPN or Private DNS, then retry Download. Completed files are kept.");
                } else {
                    self.fail(error);
                }
            }
        }
        if let Some(tc) = &mut self.control_editor
            && let Some(ed) = &mut tc.editor
            && std::mem::take(&mut ed.keyboard_request)
        {
            let text = if ed.editing_save_desc {
                ed.save_desc_input.clone()
            } else {
                ed.save_name_input.clone()
            };
            crate::ha_launcher::platform::show(&text);
        }
        for out in crate::ha_launcher::platform::poll() {
            if matches!(out, crate::ha_launcher::client::osk::Output::Enter) {
                crate::ha_launcher::platform::hide();
            }
            self.edit_text(out);
        }
        if let Some(audio) = &mut self.audio {
            audio.tick(false, &Default::default());
        }
        self.catalog.poll(&self.home);
        self.wardrobe.poll();
        if let Some(notice) = crate::ha_launcher::documents::poll() {
            self.wardrobe.reload();
            self.catalog.status = notice.text;
            if notice.changed {
                self.catalog.refresh(&self.home);
                self.catalog.show_installed(notice.kind);
            }
        }
        for result in crate::ha_launcher::gallery::poll() {
            self.catalog.status = result.unwrap_or_else(|e| format!("File export failed: {e}"));
        }
        if self.login.poll() {
            self.refresh_wardrobe_account();
        }

        if self.launch_pending_tick {
            self.launch_pending_tick = false;
            if let Err(error) = self.launch_game() {
                self.fail(error);
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        let frame_ms = if self.worker.is_some()
            || self.launching
            || self.catalog.busy()
            || self.drag.is_some()
            || self.login.pending()
            || self.control_editor.is_some()
            || self
                .catalog
                .project_modal
                .as_ref()
                .is_some_and(|m| m.loading)
            || self.anim.is_active()
            || (self.catalog.page == Page::Skins && self.wardrobe.is_animating())
            || self
                .feedback
                .is_some_and(|(_, t)| t.elapsed() < Duration::from_millis(180))
        {
            16
        } else {
            33
        };
        events.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(frame_ms),
        ));
    }
    fn exiting(&mut self, _events: &ActiveEventLoop) {}
}

impl Launcher {
    fn controller_frame(&mut self, events: &ActiveEventLoop) -> Result<()> {
        use crate::ha_launcher::client::gamepad::Button;
        use winit::keyboard::{Key, NamedKey};
        let mut state = crate::ha_launcher::client::android_runtime::gamepad_state();
        for (b, held) in [
            (Button::DPadLeft, state.left[0] < -0.55),
            (Button::DPadRight, state.left[0] > 0.55),
            (Button::DPadUp, state.left[1] > 0.55),
            (Button::DPadDown, state.left[1] < -0.55),
        ] {
            state.set(b, held || state.held(b));
        }
        let now = Instant::now();
        let mut pressed: Vec<_> = self
            .controller_edges
            .update(&state, 0.5)
            .into_iter()
            .filter_map(|(b, down)| down.then_some(b))
            .collect();
        if let Some((button, last, repeating)) = self.controller_repeat {
            if !state.held(button) {
                self.controller_repeat = None;
            } else if now.duration_since(last).as_secs_f32() >= if repeating { 0.09 } else { 0.35 }
            {
                pressed.push(button);
                self.controller_repeat = Some((button, now, true));
            }
        }
        for b in pressed {
            let direction = match b {
                Button::DPadUp => Some(NamedKey::ArrowUp),
                Button::DPadDown => Some(NamedKey::ArrowDown),
                Button::DPadLeft => Some(NamedKey::ArrowLeft),
                Button::DPadRight => Some(NamedKey::ArrowRight),
                _ => None,
            };
            if let Some(key) = direction {
                self.navigate_key(&Key::Named(key));
                if self.controller_repeat.is_none_or(|(old, _, _)| old != b) {
                    self.controller_repeat = Some((b, now, false));
                }
            } else if b == Button::South {
                self.activate(events)?;
            } else if b == Button::East {
                crate::ha_launcher::platform::hide();
                self.catalog.editing = None;
                if self.chooser.take().is_none() {
                    if self.login.visible {
                        self.login.visible = false;
                    } else {
                        self.catalog.page = Page::Play;
                    }
                }
                self.focus = None;
            }
        }
        Ok(())
    }
}
