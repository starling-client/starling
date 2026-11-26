//! Starling's part of loading_screen.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// When the next frame of the title menu drawn while the resources load is due, if that is still
/// ahead (`last`: when the last one started). On Android at most 60 a second: it ran uncapped
/// until the game's own frame limit took over, heating the phone while it prepared. Adapted from
/// Vasysik/horizon-android (its commit f5cf0231bc), MIT, copyright (c) 2026 Ludovic Marie and
/// the Horizon contributors.
#[cfg(target_os = "android")]
pub(super) fn loading_frame_due(last: Option<Instant>) -> Option<Instant> {
    let next = last? + Duration::from_nanos(1_000_000_000 / 60);
    (Instant::now() < next).then_some(next)
}

/// When the next frame of the title menu drawn while the resources load is due: at once here
/// (see the Android one).
#[cfg(not(target_os = "android"))]
pub(super) fn loading_frame_due(_last: Option<Instant>) -> Option<Instant> {
    None
}

/// The event loop; on Android, the activity's (sunrise-android).
pub(in super::super) fn event_loop() -> Result<EventLoop<()>> {
    #[cfg(target_os = "android")]
    {
        use winit::platform::android::EventLoopBuilderExtAndroid;
        Ok(EventLoop::builder()
            .with_android_app(crate::android::android_app())
            .build()?)
    }
    #[cfg(not(target_os = "android"))]
    Ok(EventLoop::new()?)
}

/// The world's renderer, made on a thread of its own on the boot renderer's device
/// (`Loading::begin_world_renderer`).
pub(in super::super) enum WorldRenderer {
    Building {
        thread: JoinHandle<std::result::Result<(Renderer, Vec<u8>), String>>,
        /// The resources it is made from (`RenderAssets::generation`): a pack reload on the title
        /// replaces them meanwhile, and this renderer with them.
        generation: u64,
        /// The GUI atlas whose pixels it took (`HudAtlas::revision`), to give them back.
        atlas: Option<u64>,
    },
    /// Made, for the game to take once the window is in front (Android puts it in the background).
    Built(Box<Renderer>),
}

impl Loading {
    pub(super) fn show_startup_console(&self) -> bool {
        cfg!(target_os = "android")
            && self.failure.is_none()
            && !self.setup.starting
            && self.menu.pending.is_none()
            && self
                .options
                .as_ref()
                .is_some_and(|options| !options.offscreen)
    }

    /// The window is going away (Android): its surface goes with it, and the log is flushed
    /// before the system may end the process.
    pub(super) fn suspended(&mut self) {
        #[cfg(target_os = "android")]
        {
            crate::diagnostics::heartbeat("suspended", || serde_json::json!({}));
            crate::diagnostics::request_flush();
        }
        self.suspended = true;
        if let Some(r) = &mut self.renderer {
            r.suspend();
        }
        if let Some(boot) = &mut self.boot_renderer {
            boot.suspend();
        }
        sunrise_client::logging::flush();
    }

    /// Load the resources again (they were imported after a failure): a fresh loader, as at start.
    pub(in super::super) fn retry(&mut self) -> Result<()> {
        #[cfg(target_os = "android")]
        if self.vanilla_root.join("sunrise-resources.json").is_file() && self.show_onboarding() {
            self.failure = None;
            return Ok(());
        }
        let options = self.options.as_ref().ok_or("loading already finished")?;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let (receiver, handle) = startup_menu::spawn_loader(
            options,
            &self.settings,
            self.menu.pack_selection_revision,
            options.execution_role.into(),
            &self.vanilla_root,
            self.started,
            &self.progress,
        )?;
        // Follow the new generation after the failed startup's result was delivered.
        self.follow_loader(receiver, handle);
        self.failure = None;
        log::info!("Sunrise: resources imported, loading again");
        Ok(())
    }

    /// The loading failure and where the resources were looked for, in the embedded Sunrise
    /// font: the vanilla one is part of the resources that did not load.
    pub(super) fn draw_failure(&self, batch: &mut HudBatch, failure: &str) {
        use sunrise_client::ui_theme::{Face, Ui, color};
        let Some(gui) = &self.gui else { return };
        let mut ui = Ui::new(&gui.atlas, batch, (0.0, 0.0));
        let (w, h) = (ui.w, ui.h);
        ui.fill(0.0, 0.0, w, h, color::bg());
        let (left, max) = (32.0, w - 64.0);
        let mut y = h * 0.25;
        ui.text(
            Face::Semibold,
            22.0,
            left,
            y,
            color::text(),
            "Sunrise could not start",
        );
        y += 40.0;
        let detail = if sunrise_client::platform::registered() {
            // Android: the prepared folder, zipped, comes in through "Open with" or the picker.
            format!(
                "{failure}\n\nStarling needs the Minecraft resources sunrise-prepare makes from your \
                 own copy of the game. Zip that folder, then open the zip with Starling from your \
                 file manager, or tap anywhere here to pick it."
            )
        } else {
            format!(
                "{failure}\n\nResources are read from {}: copy the folder sunrise-prepare made there.",
                self.vanilla_root.display()
            )
        };
        for paragraph in detail.lines() {
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let next = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if !line.is_empty() && ui.measure(Face::Regular, 14.0, &next) > max {
                    ui.text(Face::Regular, 14.0, left, y, color::MUTED, &line);
                    y += 20.0;
                    line = word.to_owned();
                } else {
                    line = next;
                }
            }
            ui.text(Face::Regular, 14.0, left, y, color::MUTED, &line);
            y += 20.0;
        }
        ui.finish();
        self.imports.draw(&gui.atlas, batch);
    }
}

impl Loading {
    /// Build the world's renderer on the boot renderer's device. Backends that can share it
    /// during initialization build on a worker while the title stays responsive.
    pub(super) fn begin_world_renderer(&mut self) -> Result<()> {
        let (Some(boot), Some(gui), Some(Ok(loaded)), Some(window)) = (
            &self.boot_renderer,
            &mut self.gui,
            &mut self.loaded,
            &self.window,
        ) else {
            return Ok(());
        };
        // The atlas's pixels go with the thread and come back with the renderer instead of being
        // copied (tens of megabytes): the loading screen draws from its own texture of them.
        let atlas = (!gui.atlas.pixels.is_empty()).then_some(gui.atlas.revision);
        let pixels = match atlas {
            Some(_) => std::mem::take(&mut gui.atlas.pixels),
            None => gui.atlas.pixels().into_owned(),
        };
        let parts = RendererParts::new(
            window.clone(),
            self.options.as_ref().is_some_and(|o| o.offscreen),
            &loaded.pack,
            gui,
            std::borrow::Cow::Owned(pixels),
            loaded.renderer_cpu.take(),
            self.settings.sunrise.api,
            self.settings.sunrise.translucent_batching,
            &self.settings.sunrise,
            shader_packs::picked_pack_shape(&self.settings),
            // What a renderer reads from the display comes after its first frames.
            Default::default(),
        );
        let lent = boot.lend();
        // EGL serializes operations on the shared context. Slow GLES shader compilation can
        // hold that lock past wgpu's timeout while the boot renderer is drawing on Android.
        #[cfg(target_os = "android")]
        if boot.info().backend == wgpu::Backend::Gl {
            let built = startup::time("renderer.world", || parts.build(None, Some(lent)))
                .map(|(renderer, pixels)| (renderer, pixels.into_owned()));
            // As a build on the thread that fails (`poll_world_renderer`): these resources are
            // done with. Android keeps the loop running after a failure, and would build from
            // them again every frame.
            let (renderer, pixels) = match built {
                Ok(built) => built,
                Err(e) => {
                    self.loaded = None;
                    return Err(e);
                }
            };
            if atlas.is_some() {
                gui.atlas.pixels = pixels;
            }
            self.world_renderer = Some(WorldRenderer::Built(Box::new(renderer)));
            return Ok(());
        }
        let parts = parts.into_owned();
        let generation = loaded.pack.render.generation;
        let thread = std::thread::Builder::new()
            .name("world-renderer".into())
            .spawn(move || {
                startup::time("renderer.world", || parts.build(None, Some(lent)))
                    .map(|(renderer, pixels)| (renderer, pixels.into_owned()))
                    .map_err(|e| e.to_string())
            })?;
        self.world_renderer = Some(WorldRenderer::Building {
            thread,
            generation,
            atlas,
        });
        Ok(())
    }

    /// Starts the world's renderer once the resources are in, and takes it when its thread is
    /// done.
    pub(super) fn poll_world_renderer(&mut self) -> Result<()> {
        if self.world_renderer.is_none() {
            // A pack reload asked for on the title replaces these resources first.
            if !self.menu.reload_packs {
                self.begin_world_renderer()?;
            }
            return Ok(());
        }
        if let Some(WorldRenderer::Building { thread, .. }) = &self.world_renderer
            && thread.is_finished()
            && let Some(WorldRenderer::Building {
                thread,
                generation,
                atlas,
            }) = self.world_renderer.take()
        {
            let built = thread.join().unwrap_or_else(|payload| {
                let text = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("no message");
                Err(format!("the world's renderer panicked: {text}"))
            });
            let made_for = |loaded: &Loaded| loaded.pack.render.generation == generation;
            let current = !self.menu.reload_packs
                && matches!(&self.loaded, Some(Ok(loaded)) if made_for(loaded));
            match built {
                Ok((renderer, pixels)) => {
                    if let Some(gui) = &mut self.gui
                        && atlas == Some(gui.atlas.revision)
                    {
                        gui.atlas.pixels = pixels;
                    }
                    // Else it was made for resources a pack reload replaces: the next ones get
                    // their own.
                    if current {
                        self.world_renderer = Some(WorldRenderer::Built(Box::new(renderer)));
                    }
                }
                // As a build at the handoff that failed: these resources are done with, and only
                // a load again (`retry`) brings others.
                Err(e) if current => {
                    self.loaded = None;
                    return Err(e.into());
                }
                Err(_) => {}
            }
        }
        Ok(())
    }
}
