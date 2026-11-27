//! The loading screen while the resources load (`Loading`), the window's app before the game
//! takes over (`App`), and the checks a start makes first: missing or damaged prepared files,
//! an unclean last exit, a fullscreen the window system could not show.
use super::*;
#[path = "loading_screen/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;

/// The window system ended the event loop with a failure (on Linux, a compositor dropping the
/// connection after a protocol error) while fullscreen was on: the next start opens windowed, so a
/// fullscreen the system can't show doesn't crash every launch (#2904).
pub(super) fn leave_fullscreen_after(
    result: &std::result::Result<(), EventLoopError>,
    settings: &mut Settings,
) {
    if matches!(result, Err(EventLoopError::ExitFailure(_))) && settings.fullscreen {
        settings.fullscreen = false;
        settings.save(&Game::options_path());
        log::warn!(
            "the window system closed the game in fullscreen; the next start opens windowed"
        );
    }
}

pub(super) fn run_event_loop(mut game: Game<'_>) -> Result<()> {
    if game.options.headless {
        // No display to open winit's loop on: the game's own loop.
        game.run_headless();
        // narrator::start ran for it too (options.txt's narrator setting).
        narrator::stop();
        let result = write_report(&game).and_then(|()| game.error.take().map_or(Ok(()), Err));
        if game.options.is_client_start() {
            game.native_client_stopped();
            game.native_game_shutting_down();
        }
        return result;
    }
    let result = event_loop()?.run_app(&mut game);
    narrator::stop();
    leave_fullscreen_after(&result, &mut game.settings);
    let result = result
        .map_err(Into::into)
        .and_then(|()| write_report(&game))
        .and_then(|()| game.error.take().map_or(Ok(()), Err));
    if game.options.is_client_start() {
        game.native_client_stopped();
        game.native_game_shutting_down();
    }
    result
}

/// The loading screen's state while the worker loads the resources.
pub(super) struct Loading {
    pub(super) window: Option<Arc<Window>>,
    pub(super) renderer: Option<Renderer>,
<<<<<<< ours
    /// What the loading screen draws with until the world's renderer is built from it
    /// (`finish`): the panorama and the HUD, and none of the world's pipelines.
    pub(super) boot_renderer: Option<sunrise_client::gfx::boot::BootRenderer>,
    /// The world's renderer, made on the boot renderer's device while the title stays up.
    pub(super) world_renderer: Option<WorldRenderer>,
=======
    /// The opened GPU device's `max_texture_dimension_2d`, shared with the background loader.
    pub(super) max_atlas_size: Arc<AtomicU32>,
>>>>>>> theirs
    /// The boot screen's atlas, until the pack's GUI atlas replaces it.
    pub(super) boot: sunrise_client::hud::HudAtlas,
    pub(super) gui: Option<UiResources>,
    pub(super) settings: Settings,
    pub(super) fullscreen: sunrise_client::window::FullscreenState,
    pub(super) video_modes: sunrise_client::window::VideoModeList,
    pub(super) options: Option<Options>,
    pub(super) vanilla_root: PathBuf,
    pub(super) pack_error: Option<String>,
    pub(super) started: Instant,
    pub(super) panorama_rotation: sunrise_client::gfx::panorama::Rotation,
    pub(super) progress: Progress,
    /// The bar as drawn, easing toward `progress` (`LoadingOverlay.currentProgress`).
    pub(super) bar: f32,
    pub(super) receiver: Receiver<LoaderMsg>,
    pub(super) loaded: Option<std::result::Result<Loaded, super::resource_loader::LoaderFailure>>,
    /// The loader's `Done` arrived: its channel closing afterwards is expected.
    pub(super) delivered: bool,
    pub(super) handle: Option<JoinHandle<()>>,
    pub(super) shown: bool,
    /// When the menu bundle arrived (`set_ui`), and whether a title frame with
    /// it was presented since: the world renderer waits for that frame
    /// (`ready_for_handoff`).
    pub(super) ui_at: Option<Instant>,
    pub(super) title_drawn: bool,
    pub(super) menu: startup_menu::StartupMenu,
    /// Why the resources did not load, kept on screen instead of quitting where no terminal
    /// shows the error (Android).
    pub(super) failure: Option<String>,
    /// Android: the game set up on the phone in place of that failure (game/setup.rs).
    pub(super) setup: setup::Setup,
    /// The finger acting as the mouse (sunrise_client::touch::TouchPointer).
    pub(super) touch_pointer: sunrise_client::touch::TouchPointer,
    /// Files handed to the game from outside: prepared resources end the failure.
    pub(super) imports: imports::Imports,
    /// The sound output, opened once the first frame is up so the menu clicks before the game
    /// loads (Game takes it over at the handoff).
    pub(super) audio: Option<Audio>,
    /// The sound events, read on a thread of their own from the start.
    pub(super) sound_registry: Option<JoinHandle<sunrise_client::sound_events::SoundRegistry>>,
    /// The window is in the background (Android): the game waits for it to take over, since
    /// its renderer needs the window's surface (one asked for meanwhile, as a screen recorder's
    /// prompt over the app, ended the start).
    pub(super) suspended: bool,
    pub(super) error: Option<Box<dyn std::error::Error>>,
}
/// How long the handoff waits for the first title frame before building the
/// world renderer anyway (a hidden window may never be asked to redraw).
pub(super) const TITLE_FRAME_WAIT: Duration = Duration::from_secs(1);
impl Loading {
    /// Reconstitute the main-thread error without erasing declaration refusals.
    pub(super) fn take_worker_result(&mut self) -> Option<Result<Loaded>> {
        self.loaded.take().map(|result| result.map_err(Into::into))
    }

    /// The handoff and, offscreen, the world renderer's build run on the main
    /// thread, so either before the title is on the screen holds the title back.
    /// Wait for one title frame first, unless a world is waiting to open.
    pub(super) fn ready_for_handoff(&self) -> bool {
        self.title_drawn
            || (self.show_startup_console() && self.shown)
            || self.menu.pending.is_some()
            || self.ui_at.is_none_or(|at| at.elapsed() >= TITLE_FRAME_WAIT)
    }
    pub(super) fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.suspended = false;
        if let Some(window) = &self.window {
            // Back from the background (Android), as Game::resumed.
            let size = window.inner_size();
            if let Some(r) = &mut self.renderer
                && let Err(e) = r.resume(size.width, size.height)
            {
                log::warn!("Sunrise: the window's surface did not come back: {e}");
            }
            if let Some(boot) = &mut self.boot_renderer
                && let Err(e) = boot.resume(size.width, size.height)
            {
                log::warn!("Sunrise: the window's surface did not come back: {e}");
            }
            window.request_redraw();
            return;
        }
        // The Window constructor: created fullscreen when options.txt or --fullscreen says so.
        let fullscreen =
            self.settings.fullscreen || self.options.as_ref().is_some_and(|o| o.fullscreen);
        let size = self
            .options
            .as_ref()
            .map_or([1280, 720], |o| o.window_size)
            .map(f64::from);
        self.fullscreen = sunrise_client::window::FullscreenState::new(fullscreen);
        let attributes = sunrise_client::window::with_icon(
            Window::default_attributes().with_title(super::window_title::base()),
        );
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "sunrise", "sunrise",
        );
        // `--offscreen` renders into a texture: its window would only show white.
        let window_step = startup::step("window.create");
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.options.as_ref().is_some_and(|o| o.offscreen)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
                return;
            }
        };
        sunrise_client::window::settle_created_fullscreen(&window, fullscreen);
        #[cfg(target_os = "windows")]
        if let Some(hwnd) = hwnd(&window) {
            sunrise_threads::system_menu::suppress(hwnd);
        }
        drop(window_step);
        // The renderer takes seconds to build and winit's window sits white
        // until the first frame presents, so paint the boot screen on it and
        // keep repainting it as the console's lines arrive; the wgpu frames
        // redraw the same design when they start. GDI is unsafe:
        // sunrise-threads holds it (this crate forbids unsafe code).
        #[cfg(target_os = "windows")]
        // In the fonts given to `menu_style`; with none, the window stays as it is.
        let boot_paint_stop = hwnd(&window)
            .zip(sunrise_client::menu_style::get().fonts)
            .map(|(hwnd, fonts)| {
                let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let repaint_stop = stop.clone();
                std::thread::spawn(move || {
                    while !repaint_stop.load(std::sync::atomic::Ordering::Relaxed) {
                        sunrise_threads::boot_paint::paint(
                            hwnd,
                            fonts.semibold,
                            fonts.mono,
                            &sunrise_client::logging::recent_lines(),
                        );
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                });
                stop
            });
        let offscreen = self.options.as_ref().is_some_and(|o| o.offscreen);
        let boot_step = startup::step("renderer.boot");
<<<<<<< ours
        // The menu bundle may have come before the window (Android's setup puts its skin on
        // while the surface is still on its way; `set_ui` then had no window to build for):
        // its atlas, not the boot screen's, is the one its screens' text and sprites name.
        let (atlas, panorama) = match &self.gui {
            Some(gui) => (&gui.atlas, Some(&gui.panorama)),
            None => (&self.boot, None),
        };
        // Offscreen has no window for a loading screen, so it keeps the old path and builds the
        // world's renderer at once; everything else draws with the boot renderer and hands it the
        // open device at `finish`.
        let failed = if offscreen {
            match loading_renderer(
                window.clone(),
                offscreen,
                Some(atlas),
                panorama,
                None,
                self.settings.sunrise.api,
            ) {
                Ok(r) => {
                    self.renderer = Some(r);
                    None
                }
                Err(e) => Some(e),
            }
        } else {
            match boot_renderer(
                window.clone(),
                Some(atlas),
                panorama,
                self.settings.sunrise.api,
            ) {
                Ok(boot) => {
                    self.boot_renderer = Some(boot);
                    None
                }
                Err(e) => Some(e),
            }
        };
        if self.gui.is_some() {
            self.ui_at = Some(Instant::now());
        }
        match failed {
            None => {}
            Some(e) => {
=======
        match loading_renderer(
            window.clone(),
            offscreen,
            Some(&self.boot),
            None,
            None,
            self.settings.sunrise.api,
        ) {
            Ok(r) => {
                self.max_atlas_size.store(
                    r.device.limits().max_texture_dimension_2d,
                    Ordering::Relaxed,
                );
                self.renderer = Some(r);
            }
            Err(e) => {
>>>>>>> theirs
                #[cfg(target_os = "windows")]
                if let Some(stop) = &boot_paint_stop {
                    stop.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        drop(boot_step);
        // The wgpu frames take the window over: stop the boot paint thread
        // before the first one presents.
        #[cfg(target_os = "windows")]
        if let Some(stop) = &boot_paint_stop {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        self.window = Some(window);
    }
    pub(super) fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        mut event: WindowEvent,
    ) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        #[cfg(target_os = "android")]
        if self.onboarding_input(&event) {
            return;
        }
        if self.show_startup_console()
            && matches!(
                event,
                WindowEvent::Touch(_)
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::KeyboardInput { .. }
            )
        {
            return;
        }
        if self.failure.is_some()
            && !self.menu.options_open
            && let WindowEvent::KeyboardInput { event, .. } = &event
            && self.setup_key_event(event)
        {
            return;
        }
        if self.menu_touch_editor_event(&event) {
            return;
        }
        if let WindowEvent::Touch(touch) = &event
            && self.credits_touch(touch)
        {
            return;
        }
        // The failure screen: the setup's buttons (Android; its settings are the menu's), else a
        // tap picks the prepared resources to import.
        if self.failure.is_some()
            && !self.menu.options_open
            && let WindowEvent::Touch(touch) = &event
        {
            if sunrise_client::platform::registered() {
                self.setup_touch(touch);
            } else if touch.phase == winit::event::TouchPhase::Ended
                && !sunrise_client::platform::pick_file()
            {
                log::warn!("Sunrise: no file picker to import resources with");
            }
            return;
        }
        follow_keymap(&mut event);
        let events = match &event {
            WindowEvent::Touch(touch) => {
                let size = self.window.as_ref().map_or([1, 1], |w| {
                    let s = w.inner_size();
                    [s.width, s.height]
                });
                let slop = sunrise_client::touch::slop(size[0] as f32, size[1] as f32);
                // Lists follow the finger one to one at this GUI scale.
                self.touch_pointer.set_gui_scale(
                    HudBatch::new(size, self.settings.gui_scale_inputs()).scale,
                    self.settings.mouse_wheel_sensitivity as f32,
                );
                // A finger on an option's button scrolls whichever way it moves, as in game.
                if touch.phase == winit::event::TouchPhase::Started
                    && !self.touch_pointer.down()
                    && self.menu_options_button_at(touch)
                {
                    self.touch_pointer.prefer_scroll();
                } else if touch.phase == winit::event::TouchPhase::Started
                    && !self.touch_pointer.down()
                    && self.menu_options_slider_at(touch)
                {
                    self.touch_pointer.prefer_slider();
                }
                self.touch_pointer.event(touch, slop, Instant::now())
            }
            _ => vec![event],
        };
        for event in events {
            if let Err(error) = self.menu_event(event_loop, event) {
                self.error = Some(error);
                event_loop.exit();
                return;
            }
        }
    }
    /// The menu bundle arrived: rebuild the loading screen with the real atlas
    /// and title panorama.
    pub(super) fn set_ui(
        &mut self,
        mut gui: UiResources,
        root: PathBuf,
        source: sunrise_assets::resource_source::ResourceSource,
    ) {
        log::info!(
            "Sunrise: loading the title artwork after {} ms",
            self.started.elapsed().as_millis()
        );
        if let Some(renderer) = &self.renderer
            && let Err(error) = gui.stitch_for_device(
                &source,
                &self.vanilla_root,
                renderer.device.limits().max_texture_dimension_2d,
            )
        {
            self.error = Some(error);
            return;
        }
        let additional = self
            .options
            .as_ref()
            .map_or_else(Vec::new, |options| pack_finders::data_packs(&options.mods));
        self.menu.set_root(
            root,
            gui.source.clone(),
            &self.settings,
            additional.into(),
            &gui.splashes,
        );
        self.gui = Some(gui);
        self.setup.starting = false;
        self.title_drawn = false;
        sunrise_client::ui_theme::reset_atlas();
        self.apply_menu_settings();
        let Some(window) = self.window.clone() else {
            return;
        };
        let offscreen = self.options.as_ref().is_some_and(|o| o.offscreen);
        let bundle = self.gui.as_ref().expect("the menu bundle was just set");
        let _step = startup::step("renderer.title_artwork");
        // The boot renderer keeps its device: the bundle's atlas and panorama are two pipelines
        // on it, where the whole renderer was built again here before.
        if let Some(boot) = &mut self.boot_renderer {
            if let Err(e) = boot_set_ui(boot, &bundle.atlas, &bundle.panorama) {
                self.error = Some(e);
            }
        } else {
            if let Some(renderer) = &mut self.renderer {
                renderer.release_surface();
            }
            match loading_renderer(
                window,
                offscreen,
                Some(&bundle.atlas),
                Some(&bundle.panorama),
                self.renderer.as_ref(),
                self.settings.sunrise.api,
            ) {
                Ok(r) => {
                    self.renderer = Some(r);
                }
                Err(e) => self.error = Some(e),
            }
        }
        // The title frame's wait starts once its renderer is built.
        self.ui_at = Some(Instant::now());
    }
    pub(super) fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The loop may run once more after exit(): keep the first error, the
        // one that ended loading.
        if self.error.is_some() {
            event_loop.exit();
            return;
        }
        if sunrise_threads::stop_requests() > 0 {
            // No world is open yet: nothing to save.
            event_loop.exit();
            return;
        }
        if self.setup.onboarding_visible() {
            #[cfg(target_os = "android")]
            if self.finish_saved_onboarding() {
                return;
            }
            // Draw the first frame explicitly, then redraw only for input and window events.
            if !self.shown
                && !self.suspended
                && let Some(window) = &self.window
            {
                window.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        if self.failure.is_some() {
            // Prepared resources handed in (Android's "Open with" or the picker): load again.
            if self.imports.poll(&self.vanilla_root)
                == Some(sunrise_client::import::Kind::Resources)
                && let Err(e) = self.retry()
            {
                self.failure = Some(e.to_string());
            }
            self.play_import_sounds();
            if self.failure.is_some() && sunrise_client::platform::registered() {
                self.poll_setup();
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            // The import banner moves: every frame while it shows.
            let frame = if self.imports.animating() {
                Duration::from_millis(16)
            } else {
                self.setup
                    .frame(self.menu.options_open || self.menu_touch_editing())
            };
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + frame));
            return;
        }
        if let Err(error) = self.poll_menu_resources() {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        if self.renderer.as_ref().is_some_and(|r| r.detached)
            || self.boot_renderer.as_ref().is_some_and(|b| b.detached)
        {
            // In the background (Android), as Game::about_to_wait.
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));
            return;
        }
        self.poll_menu_realms(event_loop);
        if self.show_startup_console() && self.shown {
            let console_due = self
                .menu
                .diagnostic_frame
                .is_none_or(|last| last.elapsed() >= Duration::from_millis(200));
            // Poll the loader without competing for the device while the game renderer builds.
            if self.world_renderer.is_none()
                && console_due
                && let Some(w) = &self.window
            {
                w.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(16),
            ));
            return;
        }
        if let Some(next) = loading_frame_due(self.menu.diagnostic_frame) {
            event_loop.set_control_flow(ControlFlow::WaitUntil(next));
            return;
        }
        if let Some(w) = &self.window {
            if self.options.as_ref().is_some_and(|o| o.offscreen) {
                self.draw();
            } else {
                w.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
    pub(super) fn draw(&mut self) {
        let console = self.show_startup_console();
        let onboarding = self.setup.onboarding_visible();
        if onboarding && self.setup.onboarding_already_chosen() {
            return;
        }
        // Android may still deliver a redraw queued just before the renderer started building.
        if console && !onboarding && self.shown && self.world_renderer.is_some() {
            return;
        }
        let _frame = startup::step("loading.frame");
        let diagnostic_started = Instant::now();
        let diagnostic_elapsed = self
            .menu
            .diagnostic_frame
            .replace(diagnostic_started)
            .map(|at| diagnostic_started.duration_since(at).as_secs_f32() * 1000.0);
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
            .or_else(|| self.boot_renderer.as_ref().map(|b| b.size()))
        else {
            return;
        };
        let mut batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        if onboarding || (console && self.setup.onboarding_pending()) {
            self.draw_setup(&mut batch, "");
        } else if self.failure.is_some() && self.menu.options_open {
            // The setup's settings: the menu's own screens, in the skin it put on.
            self.draw_menu(&mut batch);
        } else if let Some(failure) = &self.failure {
            if sunrise_client::platform::registered() {
                self.draw_setup(&mut batch, failure);
            } else {
                self.draw_failure(&mut batch, failure);
            }
        } else if self.setup.starting {
            // Prepared: the setup stays until the menu's own font has come.
            self.draw_setup(&mut batch, "");
        } else if console {
            let atlas = self.gui.as_ref().map_or(&self.boot, |gui| &gui.atlas);
            let logs = sunrise_client::logging::recent_lines();
            sunrise_client::ui_theme::draw_boot(atlas, &mut batch, &logs);
        } else {
            self.draw_menu(&mut batch);
        }
        self.draw_menu_touch_editor(&mut batch, size);
        self.menu.frame_drawn((batch.width, batch.height));
        let yaw = self.panorama_rotation.update(
            self.started.elapsed().as_secs_f32(),
            self.settings.file.double("panoramaScrollSpeed") as f32,
        );
<<<<<<< ours
        let diagnostic_prepared = diagnostic_started.elapsed();
        let drawn = if let Some(boot) = &mut self.boot_renderer {
            match boot.render(&batch, yaw, !console && !onboarding) {
                Ok(drawn) => drawn,
                Err(e) => {
                    log::warn!("Sunrise: loading screen frame failed: {e}");
                    false
                }
            }
        } else if let Some(renderer) = &mut self.renderer {
            let mut frame = loading_frame(&batch);
            frame.yaw = yaw;
            match renderer.render(&frame) {
                Ok(drawn) => drawn,
                Err(e) => {
                    log::warn!("Sunrise: loading screen frame failed: {e}");
                    false
=======
        if let Some(renderer) = &mut self.renderer {
            match renderer.render(&frame, None) {
                Ok(true) if self.gui.is_some() && self.menu.pending.is_none() => {
                    self.title_drawn = true;
>>>>>>> theirs
                }
            }
        } else {
            false
        };
        if drawn && self.gui.is_some() && self.menu.pending.is_none() {
            self.title_drawn = true;
        }
        let diagnostic_spans = if sunrise_client::spans::recording_here() {
            sunrise_client::spans::take_raw()
        } else {
            Vec::new()
        };
        #[cfg(target_os = "android")]
        crate::diagnostics::heartbeat(
            if self.failure.is_some() {
                "setup_failure"
            } else {
                "startup_menu"
            },
            || {
                // While the loading screen draws, the adapter is the boot renderer's: a crash or
                // an ANR here is still reported against the GPU that was being drawn with.
                let info = self
                    .renderer
                    .as_ref()
                    .map(|r| r.info.clone())
                    .or_else(|| self.boot_renderer.as_ref().map(|b| b.info()));
                serde_json::json!({
                    "gpu": info.as_ref().map(|i| &i.name),
                    "backend": info.as_ref().map(|i| format!("{:?}", i.backend)),
                })
            },
        );
        if let Some(renderer) = &mut self.renderer {
            renderer.set_pass_timing(crate::performance::gpu_profile());
            if crate::performance::active() {
                if let Some(elapsed) = diagnostic_elapsed {
                    let cpu = diagnostic_started
                        .elapsed()
                        .saturating_sub(renderer.last_wait)
                        .as_secs_f32()
                        * 1000.0;
                    let prepared = diagnostic_prepared.as_secs_f32() * 1000.0;
                    crate::performance::slow_frame(cpu, &diagnostic_spans);
                    if let Some(mut row) = crate::performance::record(
                        elapsed,
                        cpu,
                        &renderer.gpu_times,
                        [0.0, 0.0, prepared, (cpu - prepared).max(0.0)],
                    ) {
                        row["gpu_passes"] = renderer.diagnostic_pass_times();
                        crate::performance::submit(
                            row,
                            serde_json::json!({"phase":"startup_menu", "gpu":renderer.info.name,
                            "backend":format!("{:?}",renderer.info.backend), "resolution":size}),
                        );
                    }
                }
                renderer.gpu_times.clear();
            }
        }
        if drawn && !self.shown {
            self.shown = true;
            startup::mark("loading_screen.shown");
            log::info!(
                "Sunrise: loading screen shown after {} ms",
                self.started.elapsed().as_millis()
            );
            // The system's starting window stayed over the app until now (StarlingActivity).
            #[cfg(target_os = "android")]
            crate::android::first_frame_shown();
            // Not before the first frame: opening the output takes a moment.
            if self.audio.is_none()
                && self.failure.is_none()
                && let Some(registry) = self.sound_registry.take()
            {
                let registry = registry.join().unwrap_or_default();
                let mut audio = Audio::with_registry(
                    &sunrise_assets::resource_source::ResourceSource::directory(
                        self.vanilla_root.clone(),
                    ),
                    registry,
                );
                audio.master = self.settings.master_volume as f32;
                // As Game::apply_ui_settings: the master volume is the engine's own.
                let mut volumes = self.settings.volumes.map(|gain| gain as f32);
                volumes[0] = 1.0;
                sunrise_client::settings::set_source_volumes(volumes);
                audio.sync_volumes();
                self.audio = Some(audio);
            }
        }
    }
    /// What the app does next (`App::about_to_wait`): nothing yet, or the game, made from the
    /// loaded resources and the world's renderer.
    fn poll_handoff(&mut self) -> Option<Result<Option<Game<'static>>>> {
        if self.setup.onboarding_visible() {
            return None;
        }
        // Offscreen there is no boot renderer: the world's is built at the handoff, on the main
        // thread, from the loading renderer's device.
        let threaded = self.boot_renderer.is_some() || self.world_renderer.is_some();
        if threaded && let Err(e) = self.poll_world_renderer() {
            return Some(Err(e));
        }
        // A window in the background has no surface for the game's renderer: the loaded
        // resources wait for it to come back.
        if self.menu.reload_packs || self.suspended || !self.ready_for_handoff() {
            return None;
        }
        // Keep the boot menu responsive while the world tables load. Client
        // model publication needs them even for a start without mods.
        if self.options.as_ref().is_some_and(Options::is_client_start)
            && let Some(Ok(loaded)) = &self.loaded
            && let Some(loader) = &loaded.rest_loader
            && !loader.handle.is_finished()
        {
            return None;
        }
        let renderer = match self.world_renderer.take() {
            Some(WorldRenderer::Built(renderer)) => Some(*renderer),
            building => {
                self.world_renderer = building;
                if threaded && matches!(self.loaded, Some(Ok(_))) {
                    return None;
                }
                None
            }
        };
        Some(match self.loaded.take()? {
            Err(e) => Err(e.into()),
            Ok(loaded) => self.finish(loaded, renderer),
        })
    }
    pub(super) fn finish(
        &mut self,
        mut loaded: Loaded,
        // The world's renderer made on the loading screen's device (`begin_world_renderer`);
        // none builds it here.
        built: Option<Renderer>,
    ) -> Result<Option<Game<'static>>> {
        let handoff_at = Instant::now();
        let mut options = self.options.take().ok_or("loading already finished")?;
        options.resources = loaded.root.clone();
        // StartupMenu owns world selection until it can hand it to Game.
        options.world = None;
        options.server = None;
        let gui = self.gui.take().ok_or("missing menu bundle")?;
        let window = self.window.clone().ok_or("window closed")?;
        let filtering = (
            self.settings.file.int("textureFiltering"),
            self.settings.file.int("maxAnisotropyBit"),
        );
        let renderer_at = Instant::now();
<<<<<<< ours
        let renderer_step = startup::step("handoff.renderer");
        let (renderer, clouds) = match built {
            Some(mut renderer) => {
                // The window keeps the one surface it has: the boot renderer's, its last frame
                // still on it, else (the window came back without one) a new one.
                let size = window.inner_size();
                match self
                    .boot_renderer
                    .take()
                    .and_then(sunrise_client::gfx::boot::BootRenderer::give_surface)
                {
                    Some(surface) => renderer.adopt_surface(surface, size.width, size.height)?,
                    None => renderer.resume(size.width, size.height)?,
                }
                let clouds = finish_renderer(
                    &mut renderer,
                    &loaded.pack,
                    &gui,
                    filtering,
                    (self.settings.vsync, self.settings.sunrise.present_mode),
                )?;
                (renderer, clouds)
            }
            None => {
                // Keep the loader's device and swap in the real textures in one step.
                if let Some(renderer) = &mut self.renderer {
                    renderer.release_surface();
                }
                startup::time("renderer.world", || {
                    build_renderer(
                        window.clone(),
                        options.offscreen,
                        &loaded.pack,
                        &gui,
                        self.renderer.as_ref(),
                        loaded.renderer_cpu.take(),
                        self.settings.sunrise.api,
                        self.settings.sunrise.translucent_batching,
                        self.settings.vsync,
                        &self.settings.sunrise,
                        shader_packs::picked_pack_shape(&self.settings),
                        self.renderer
                            .as_ref()
                            .map(|r| r.display_reading())
                            .unwrap_or_default(),
                        filtering,
                    )
                })?
            }
        };
        drop(renderer_step);
=======
        let mut pick_hdr = shader_packs::PickHdr::default();
        let (renderer, required) = startup::time("renderer.world", || {
            build_renderer(
                window.clone(),
                options.offscreen,
                &loaded.pack,
                &gui,
                self.renderer.as_ref().map_or(
                    sunrise_client::gfx::RendererDevice::New,
                    sunrise_client::gfx::RendererDevice::Reuse,
                ),
                loaded.renderer_cpu,
                self.settings.sunrise.api,
                self.settings.sunrise.translucent_batching,
                self.settings.vsync,
                &self.settings.sunrise,
                shader_packs::startup_pack_shape(&self.settings, &mut pick_hdr),
                self.renderer
                    .as_ref()
                    .map(|r| r.display_reading())
                    .unwrap_or_default(),
                self.settings.texture_atlas_settings(),
            )
        })?;
>>>>>>> theirs
        log::info!(
            "Sunrise: world renderer ready after {} ms",
            self.started.elapsed().as_millis()
        );
        let renderer_ms = renderer_at.elapsed().as_millis();
        // Audio is made on the main thread: the menu's since its first frame, read again from the
        // resource packs' root where they change it, else made now.
        let audio_step = startup::step("handoff.audio");
        let audio = if !options.execution_role.is_client() {
            Audio::silent(&loaded.pack.source)
        } else {
            match self.audio.take() {
                Some(mut audio) => {
                    if options.resources != self.vanilla_root {
                        audio.reload(&loaded.pack.source);
                    }
                    audio
                }
                None => Audio::with_device(&loaded.pack.source, &self.settings.sound_device),
            }
        };
        drop(audio_step);
        // The menu read its language from the prepared root alone (`set_root`) and
        // `load_resources` skips it on this path: read it again with the mods' layers, in the
        // language picked in the menu.
        startup::time("handoff.language", || {
            sunrise_client::lang::set(sunrise_client::lang::Language::load(
                &[&loaded.pack.source],
                &sunrise_client::lang::code(),
            ));
        });
        let pack_error = loaded.pack_error.or_else(|| self.pack_error.take());
        // The game borrows its shared resources for the process's lifetime,
        // like the single copy an integrated server would share.
        let res: &'static Arc<Resources> = Box::leak(Box::new(loaded.resources));
        let game_at = Instant::now();
        let game_step = startup::step("handoff.start_game");
        let Some(mut game) = start_game(
            res,
            loaded.pack,
            gui,
            options,
            pack_error,
            audio,
            self.vanilla_root.clone(),
            self.started,
            loaded.rest_loader,
            self.settings.clone(),
            loaded.game_cpu,
            std::mem::take(&mut loaded.client_applies),
            loaded.entity_models,
            Some((window, renderer)),
        )?
        else {
            return Ok(None);
        };
        drop(game_step);
        let game_ms = game_at.elapsed().as_millis();
        game.fullscreen = self.fullscreen;
        game.panorama_rotation = self.panorama_rotation;
        // The pick's HDR request as the renderer was made for it, with its notes already logged.
        *game.native_shaders.pick_hdr.get_mut() = pick_hdr;
        self.menu.handoff(&mut game);
        game.apply_settings();
        if let Err(error) = required {
            game.selection_reload_failed(&error);
        }
        game.previous = Instant::now();
        // Minecraft.onGameLoadFinished: a start that made it this far is clean.
        game.settings.file.set(
            "startedCleanly",
            sunrise_client::options_file::V::Bool(true),
        );
        game.settings.save(&Game::options_path());
        log::info!(
            "Sunrise: startup handoff {} ms (renderer {} ms, start_game {} ms)",
            handoff_at.elapsed().as_millis(),
            renderer_ms,
            game_ms
        );
        Ok(Some(game))
    }
}

/// The application: the loading screen until the resources are ready, then
/// the game itself, on the same event loop and window.
pub(super) enum App {
    Loading(Box<Loading>),
    Ready(Box<Game<'static>>),
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Loading(loading) => loading.resumed(event_loop),
            App::Ready(game) => game.resumed(event_loop),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match self {
            App::Loading(loading) => loading.window_event(event_loop, id, event),
            App::Ready(game) => game.window_event(event_loop, id, event),
        }
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        if let App::Ready(game) = self {
            game.device_event(event_loop, id, event);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        enum Step {
            Waiting,
            Ready(Box<Game<'static>>),
            Done,
            Failed(Box<dyn std::error::Error>),
        }
        let step = match self {
            App::Ready(game) => {
                game.about_to_wait(event_loop);
                return;
            }
            App::Loading(loading) => {
                #[cfg(target_os = "android")]
                if crate::ha_launcher::launcher_next() {
                    event_loop.exit();
                    return;
                }
                loading.about_to_wait(event_loop);
                if loading.error.is_some() {
                    event_loop.exit();
                    return;
                }
                match loading.poll_handoff() {
                    None => Step::Waiting,
                    Some(Ok(Some(game))) => {
                        // Starling: the prepared resources loaded, the files they came from
                        // can go.
                        sunrise_client::preparer::drop_downloads(&loading.vanilla_root);
                        Step::Ready(Box::new(game))
                    }
                    Some(Ok(None)) => Step::Done,
                    Some(Err(e)) => Step::Failed(e),
                }
            }
        };
        match step {
            Step::Waiting => {}
            Step::Ready(game) => *self = App::Ready(game),
            Step::Done => event_loop.exit(),
            Step::Failed(e) => {
                if let App::Loading(loading) = self {
                    if cfg!(target_os = "android") {
                        log::error!("Sunrise: {e}");
                        loading.failure = Some(e.to_string());
                        return;
                    }
                    loading.error = Some(e);
                }
                event_loop.exit();
            }
        }
    }
    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Loading(loading) => loading.suspended(),
            App::Ready(game) => game.suspended(event_loop),
        }
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        if let App::Ready(game) = self {
            game.exiting(event_loop);
        }
    }
}

/// Every launch reads the prepared game files (`sunrise-prepare`'s output, whose
/// manifest is `sunrise-resources.json`): without them, say where they were
/// looked for and how to get them, before a window or a loader starts. `get` is
/// the page a hook sends the player to (`sunrise_hooks::engine::get_the_game`).
#[cfg(not(target_os = "android"))]
pub(super) fn preflight_resources(
    root: &std::path::Path,
    get: Option<sunrise_hooks::GetTheGame>,
    dedicated: bool,
) -> std::result::Result<sunrise_core::prepared_profile::Profile, String> {
    use sunrise_core::{
        prepared_profile::{MANIFEST, Manifest, Profile},
        version::VERSION_ID,
    };

    if !root.join(MANIFEST).is_file() {
        let launcher = get.filter(|_| !dedicated).map_or(String::new(), |get| {
            format!(
                "Sunrise is started from its launcher: download it at {} (Download). It needs \
                 your own Minecraft: Java Edition; the launcher signs you in and prepares the \
                 game files.\nDevelopers: ",
                get.address
            )
        });
        let shown = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        let profile = if dedicated {
            Profile::Server
        } else {
            Profile::Full
        };
        return Err(format!(
            "Sunrise's prepared game files are missing: no {MANIFEST} in {}\n\
             {launcher}{}, or pass --resources <folder>",
            shown.display(),
            profile.repair(root),
        ));
    }
    let manifest = Manifest::read(root).map_err(|problem| {
        format!(
            "Sunrise's prepared game files at {} are damaged: {problem}",
            root.display(),
        )
<<<<<<< ours
    })
}

/// Which top-level prepared file a killed preparation or an interrupted copy
/// left unusable. The loader parses these before it can start; naming one
/// beats serde's bare "expected value at line 1 column 1".
#[cfg(not(target_os = "android"))]
pub(super) fn damaged_prepared(root: &std::path::Path) -> Option<String> {
    let manifest = root.join("sunrise-resources.json");
    let bytes = std::fs::read(&manifest).ok()?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Some("sunrise-resources.json is empty".into());
=======
    })?;
    let profile = manifest.profile();
    if profile == Profile::Server && !dedicated {
        return Err(format!(
            "{} contains server-only game files, not a full client preparation. Run \
             `sunrise-prepare --prepare --server-jar PATH --output FULL_DIR` with your own \
             client files, then pass --resources FULL_DIR.",
            root.display(),
        ));
>>>>>>> theirs
    }
    let mut problems = manifest.version_problems(root, VERSION_ID);
    if profile == Profile::Server {
        problems.extend(manifest.server_inventory_problems(root));
    }
    // An empty JSON file left by an interrupted export fails with a generic
    // parse error later. Name the file now, before starting a loader or a window.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension() == Some(std::ffi::OsStr::new("json"))
                && std::fs::metadata(&path).is_ok_and(|meta| meta.is_file() && meta.len() == 0)
            {
                problems.push(format!("{} is empty", entry.file_name().to_string_lossy()));
            }
        }
    }
    if problems.is_empty() {
        Ok(profile)
    } else {
        Err(format!(
            "Sunrise's prepared game files in {} are stale or damaged:\n  {}\n\
             Prepare them again with `{}`.",
            root.display(),
            problems.join("\n  "),
            profile.repair(root),
        ))
    }
}

/// `Minecraft.<init>`: `startedCleanly` left false means the previous start
/// died before `onGameLoadFinished`. Vanilla then leaves fullscreen
/// ("resetting fullscreen mode") so a start that died on the display mode can
/// come back; the resource packs are rolled back only when their load fails
/// (`rollbackResourcePacks`), not here. True when the last start was unclean.
pub(super) fn unclean_start_recovery(settings: &mut Settings) -> bool {
    // Vanilla's field defaults true: only an explicit `false` is an unclean
    // start; an options file that never had the key is not one.
    let unclean = matches!(
        settings.file.get("startedCleanly"),
        Some(sunrise_client::options_file::V::Bool(false))
    );
    if unclean {
        settings.fullscreen = false;
        settings.file.fullscreen_resolution = None;
    }
    unclean
}
