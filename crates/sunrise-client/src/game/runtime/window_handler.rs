//! winit's `ApplicationHandler` for the game window: window and device events, shutting down,
//! and the idle step (`about_to_wait`).
use super::*;
use sunrise_client::window::PointerSample;

impl Game<'_> {
    /// A run that a person's stray key, click or mouse move would spoil: a screenshot sequence, a
    /// `SUNRISE_TICK_SCRIPT` scenario or a `record-replay`. While one runs, the keyboard, mouse
    /// and pad at this computer don't reach the game (`locks_out`, `poll_pads`), the window's
    /// focus stays as the run found it (`held_focus`) and its pointer grab stays inside the game
    /// (`internal_grab`).
    pub(super) fn scripted_run(&self) -> bool {
        self.screenshot_sequence.is_some() || self.tick_script.playing() || self.repro.replaying()
    }
}

/// A key, a modifier, a click, the wheel, the pointer or the input method. Resize and close are
/// not, so a person can still close the window. A modifier change counts: `reconcile` would
/// release a Shift or Ctrl the run holds.
/// `event`'s key without the modifiers held (winit's modifier supplement). Starling: Android's winit has
/// none, and the logical key is the nearest; a function apart so `window_event`, which mods target, has no
/// cfg in its body (horizon-mixins refuses one).
fn key_without_modifiers(event: &winit::event::KeyEvent) -> winit::keyboard::Key {
    #[cfg(not(target_os = "android"))]
    {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        event.key_without_modifiers()
    }
    #[cfg(target_os = "android")]
    {
        event.logical_key.clone()
    }
}

fn is_input(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::Ime(_)
    )
}

/// A person's input during a scripted run is dropped. What the run itself sends (the control
/// socket, a replay, the game's own) still plays.
fn locks_out(origin: input_trace::InputOrigin, scripted: bool, event: &WindowEvent) -> bool {
    scripted && origin == input_trace::InputOrigin::User && is_input(event)
}

/// A person clicking into the window or away from it during a scripted run. A replay's look and
/// clicks act by the focus and the pointer grab, which that would change mid-run: the run keeps
/// the focus it started with, and the last change applies once it ends (`settle_after_run`).
fn held_focus(
    origin: input_trace::InputOrigin,
    scripted: bool,
    event: &WindowEvent,
) -> Option<bool> {
    match event {
        WindowEvent::Focused(focused) if scripted && origin == input_trace::InputOrigin::User => {
            Some(*focused)
        }
        _ => None,
    }
}

/// What a person's event does at an agent's window no one has clicked yet.
#[derive(Debug, PartialEq)]
enum Unclaimed {
    /// A mouse button pressed inside it: the window is theirs from this press on.
    Claims,
    /// Their keys, wheel, input method and button releases, and the focus the system gives the
    /// window or takes from it: the game the agent drives hears none of them.
    Dropped,
    /// Everything else, and whatever the agent or the game sends.
    Passes,
}

/// Only a press claims the window. The focus doesn't: Windows and X11 can hand it to a window
/// nobody clicked, and the person's keys and mouse would then steer the agent's game (#15449).
/// The pointer's position still passes, so the press that claims a menu lands where it points.
fn at_unclaimed(
    origin: input_trace::InputOrigin,
    unclaimed: bool,
    event: &WindowEvent,
) -> Unclaimed {
    if !unclaimed || origin != input_trace::InputOrigin::User {
        return Unclaimed::Passes;
    }
    match event {
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } => Unclaimed::Claims,
        WindowEvent::CursorMoved { .. } => Unclaimed::Passes,
        WindowEvent::Focused(_) => Unclaimed::Dropped,
        _ if is_input(event) => Unclaimed::Dropped,
        _ => Unclaimed::Passes,
    }
}

/// The Shift, Ctrl or Alt a person holds at an agent's window no one has clicked yet. The game
/// doesn't hear of them, but winit tells of modifiers only as they change: the last state is
/// kept for the claim, where a Ctrl held since before the click would otherwise read as up.
fn held_modifiers(
    origin: input_trace::InputOrigin,
    unclaimed: bool,
    event: &WindowEvent,
) -> Option<winit::event::Modifiers> {
    match event {
        WindowEvent::ModifiersChanged(modifiers)
            if unclaimed && origin == input_trace::InputOrigin::User =>
        {
            Some(*modifiers)
        }
        _ => None,
    }
}

impl Game<'_> {
    /// winit's `window_event`. The control socket and the pad menu call it too, with the
    /// input they make up.
    pub(super) fn handle_window_event(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        event: WindowEvent,
        origin: input_trace::InputOrigin,
    ) {
        self.handle_window_event_with_sample(event_loop, id, event, origin, None);
    }

    pub(super) fn handle_window_event_with_sample(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        event: WindowEvent,
        origin: input_trace::InputOrigin,
        retained_sample: Option<PointerSample>,
    ) {
        let input = matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::Ime(_)
        );
        self.with_user_input(input, |game| {
            game.handle_window_event_as(event_loop, id, event, origin, retained_sample);
        });
    }

    pub(super) fn handle_window_event_as(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        mut event: WindowEvent,
        origin: input_trace::InputOrigin,
        retained_sample: Option<PointerSample>,
    ) {
        // A headless game has no window: its events (the frame, the control socket's input)
        // carry `WindowId::dummy()`.
        let ours = match &self.window {
            Some(w) => w.id() == id,
            None => self.options.headless,
        };
        if !ours {
            return;
        }
        if let Some(modifiers) = held_modifiers(origin, self.unclaimed_window, &event) {
            self.unclaimed_modifiers = Some(modifiers);
        }
        if locks_out(origin, self.scripted_run(), &event) {
            return;
        }
        if let Some(focused) = held_focus(origin, self.scripted_run(), &event) {
            self.focus_after_run = Some(focused);
            return;
        }
        self.save_when_focus_leaves(&event);
        #[cfg(target_os = "android")]
        self.sync_interface_cursor(self.touch_size());
        if self.touch_event(event_loop, id, &event) {
            return;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && self.touch_back_key(event)
        {
            return;
        }
        match at_unclaimed(origin, self.unclaimed_window, &event) {
            Unclaimed::Claims => self.claim_window(event_loop),
            Unclaimed::Dropped => return,
            Unclaimed::Passes => {}
        }
        follow_keymap(&mut event);
        // MacosUtil.setCtrlClickEmulatesRightClick: Control + left click is a right click.
        if let WindowEvent::MouseInput { state, button, .. } = &mut event
            && *button == MouseButton::Left
            && self.right_click_emulation.is_right_click(
                *state == ElementState::Pressed,
                self.modifiers.control_key(),
                MACOS_PLATFORM && self.settings.ctrl_click_emulates_right_click,
            )
        {
            *button = MouseButton::Right;
        }
        let cursor_sample = if let WindowEvent::CursorMoved { position, .. } = &event {
            Some(retained_sample.unwrap_or_else(|| {
                // Only the coordinate scale matters here, not the screen extent.
                let window = self.pointer_space([1, 1]);
                if origin == input_trace::InputOrigin::User {
                    window.os_sample(*position)
                } else {
                    window.physical_sample(*position)
                }
            }))
        } else {
            None
        };
        // Who caused it, before the event is handled: the trace wants the
        // window's own keys, never the ones the socket made up.
        self.trace_window_event(origin, &event, cursor_sample);
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let PhysicalKey::Code(key) = event.physical_key
        {
            if event.state == ElementState::Pressed {
                self.physical_keys.insert(key);
                sunrise_client::keybinds::learn_layout(event);
            } else {
                self.physical_keys.remove(&key);
            }
        }
        if matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
        ) && !self.input_made_pointer_event()
        {
            self.pad.keyboard_took_over = true;
        }
        if let WindowEvent::MouseInput { state, button, .. } = &event {
            if *state == ElementState::Pressed {
                self.physical_buttons.insert(*button);
                self.physical_presses += 1;
            } else {
                self.physical_buttons.remove(button);
            }
            // A container hook's press latch ends here, before any hook can cancel the release.
            self.container_owned_up = *state == ElementState::Released
                && !self.pad_synthetic_input()
                && self.container_button_up(*button);
        }
        let mouse = if !self.pad_synthetic_input()
            && let WindowEvent::MouseInput { state, button, .. } = &event
        {
            Some((
                glfw_button(*button),
                *state == ElementState::Pressed,
                glfw_modifiers(self.modifiers),
            ))
        } else {
            None
        };
        if let Some((button, pressed, modifiers)) = mouse {
            let mut canceled = false;
            self.native_mouse_button_pre(button, pressed, modifiers, &mut canceled);
            // This is release cleanup, not gameplay dispatch. Even a consumed release
            // must not leave a mod's mapping down; presses wait for the screen outcome.
            if let WindowEvent::MouseInput {
                state: ElementState::Released,
                button,
                ..
            } = &event
            {
                self.mod_mappings_button(*button, false);
            }
            if canceled {
                return;
            }
        }
        let resized = matches!(event, WindowEvent::Resized(_));
        let mouse_press_millis = (origin == input_trace::InputOrigin::User
            && mouse.is_some_and(|(_, pressed, _)| pressed))
        .then(|| self.millis());
        let title_settled =
            self.handle_window_event_inner(event_loop, event, cursor_sample, mouse_press_millis);
        if resized {
            self.notify_window_resize();
        }
        // Button receivers settle before their Post; other events settle here.
        if !title_settled {
            self.settle_title_mode();
        }
        if let Some((button, pressed, modifiers)) = mouse {
            self.native_mouse_button_post(button, pressed, modifiers);
        }
    }

    /// The loop ended: close the pads' output, save the world.
    pub(super) fn shut_down(&mut self) {
        self.close_client_tasks();
        if self.error.is_none()
            && let Some(renderer) = &mut self.renderer
        {
            renderer.drop_hdr_marker();
        }
        self.end_screenshot_sequence(Err("cancelled: the game is closing".into()));
        self.pad_close_output();
        if let Err(e) = self.save_for_exit() {
            log::warn!("Sunrise: failed to save the world on exit: {e}");
        }
        self.finish_profile();
        // The client's last moment: its world is left and saved, its options and
        // resources are still held. A dedicated server skips it, as `client_setup` does.
        if self.options.is_client_start() {
            self.native_client_stopping();
        }
    }

    /// winit's `about_to_wait`: what happens between frames (stop requests, the closing
    /// save, world entry, the control socket, pads), then the next frame.
    pub(super) fn idle(&mut self, event_loop: &dyn EventLoopCtl) {
        if sunrise_threads::stop_requests() > 0
            && self.pending_save.as_ref().is_none_or(|save| !save.quit)
        {
            // SIGTERM, SIGINT or SIGHUP: the shutdown hook's server halt, as a window close.
            if let Err(error) = self.begin_closing_save(true) {
                self.error = Some(error);
                event_loop.exit();
                return;
            }
        }
        if self.pending_save.is_some() {
            match self.poll_closing_save() {
                Ok(true) => event_loop.exit(),
                Ok(false) => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    // Yield between polls even when disk I/O is slow.
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(4),
                    ));
                }
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                }
            }
            return;
        }
        if let Err(error) = sunrise_client::spans::time("world_entry", || self.resume_world_entry())
        {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        // The world a backup made before its upgrade opens (EditWorldScreen's callback).
        if let Some(folder) = self.title.as_mut().and_then(|t| t.take_open_request())
            && let Err(error) = self.open_world(&folder, None, None, false, None)
        {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        // A Realm's address came back (GetServerDetailsTask hands over to ConnectScreen).
        if let Some(action) = self.title.as_mut().and_then(|t| t.take_connect_request())
            && let Err(error) = self.title_action(action)
        {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        sunrise_client::spans::time("server_packs", || self.tick_server_packs());
        sunrise_client::spans::time("control", || self.poll_control(event_loop));
        if self.pending_save.is_some() {
            return;
        }
        // The mouse and the keyboard as Android's input hook saw them (game/input_mode.rs).
        #[cfg(target_os = "android")]
        self.android_input_frame(event_loop);
        self.touch_frame(event_loop);
        self.poll_imports();
        if let Some(folder) = &self.content_record
            && world_content::record(folder, &world_content::running())
        {
            self.content_record = None;
        }
        #[cfg(target_os = "android")]
        {
            self.poll_files_offer();
            self.poll_update_offer();
            self.poll_native_builds();
            self.poll_catalog();
        }
        sunrise_client::spans::time("pads", || self.poll_pads(event_loop));
        self.settle_after_run(event_loop);
        self.pause_if_inactive();
        if let Some(limit) = self.options.time_limit()
            && self.started.elapsed() > limit
        {
            self.error = Some(format!("run timed out after {} s", limit.as_secs()).into());
            event_loop.exit();
            return;
        }
        if self.renderer.as_ref().is_some_and(|r| r.detached) {
            // No window to draw in (Android in the background): wake now and then for the
            // control socket instead of spinning.
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));
            return;
        }
        if let Some(w) = &self.window {
            if self.options.offscreen {
                // Offscreen frames don't wait for the window to be drawn: an
                // occluded window (a locked macOS session) gets no redraws.
                let id = w.id();
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::RedrawRequested,
                    input_trace::InputOrigin::Game,
                );
            } else {
                w.request_redraw();
            }
        } else if self.options.headless {
            // The headless loop paces the frames itself (`run_headless`).
            self.handle_window_event(
                event_loop,
                WindowId::dummy(),
                WindowEvent::RedrawRequested,
                input_trace::InputOrigin::Game,
            );
            return;
        }
        // Consume the last frame's result: a hidden compositor may stop sending
        // redraws altogether, leaving a successful result stale indefinitely.
        event_loop.set_control_flow(event_loop::redraw_flow(
            &mut self.frame_drawn,
            self.options.offscreen,
            Instant::now(),
        ));
    }
}

impl ApplicationHandler for Game<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            // Back from the background (Android): the same window, a new native surface.
            let size = window.inner_size();
            if let Some(r) = &mut self.renderer
                && let Err(e) = r.resume(size.width, size.height)
            {
                log::warn!("Sunrise: the window's surface did not come back: {e}");
            }
            self.audio.resume_output();
            window.request_redraw();
            // The next time it goes, it saves again.
            self.saved_for_background = false;
            return;
        }
        debug_keys::warm_debug_cpu();
        // The Window constructor: created fullscreen when options.txt or --fullscreen says so.
        let fullscreen = self.settings.fullscreen || self.options.fullscreen;
        let size = self.options.window_size.map(f64::from);
        self.fullscreen = sunrise_client::window::FullscreenState::new(fullscreen);
        let attributes = sunrise_client::window::with_icon(
            Window::default_attributes().with_title(super::window_title::base()),
        );
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "sunrise", "sunrise",
        );
        // A game an agent started opens without the focus and
        // the person keeps their keyboard and pointer until they click it (`unclaimed_window`).
        // X11 activates it anyway, and Windows can too: the focus claims nothing.
        let agent_started = self.options.control.is_some();
        self.unclaimed_window = agent_started && !self.options.offscreen;
        // `--offscreen` renders into a texture: its window would only show white.
        let window_step = startup::step("window.create");
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.options.offscreen)
                .with_active(!agent_started),
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
        let pending = self.pending_initial_models.take();
        let renderer = match &pending {
            Some(start) => self.renderer_for(window.clone(), &start.models.pack, &start.models.gui),
            None => self.new_renderer(window.clone()),
        };
        self.resize_state = Some(self.resize_state_of(window.inner_size()));
        self.window = Some(window);
        if let Some(pending) = pending {
            match self.apply_pending_client_start(*pending, renderer.map(Some)) {
                Ok(true) => {
                    self.settings.file.set(
                        "startedCleanly",
                        sunrise_client::options_file::V::Bool(true),
                    );
                    self.settings.save(&Game::options_path());
                }
                Ok(false) => {
                    event_loop.exit();
                    return;
                }
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                    return;
                }
            }
        } else {
            match renderer {
                Ok(renderer) => {
                    self.renderer = Some(renderer);
                    self.world_renderer_ready = true;
                }
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                    return;
                }
            }
        }
        self.ime_allowed = false;
        self.ime_area = None;
        self.apply_settings();
        self.previous = Instant::now();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let key = if self.window.as_ref().is_some_and(|w| w.id() == id)
            && !self.scripted_run()
            && !self.unclaimed_window
            && let WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } = &event
        {
            Some((event.clone(), glfw_modifiers(self.modifiers)))
        } else {
            None
        };
        self.handle_window_event(event_loop, id, event, input_trace::InputOrigin::User);
        if let Some((event, modifiers)) = key {
            let name = match event.physical_key {
                PhysicalKey::Code(code) => sunrise_client::keybinds::key_name(code),
                PhysicalKey::Unidentified(_) => None,
            };
            let layout_key = sunrise_client::keybinds::layout_key_name(
                event.physical_key,
                &key_without_modifiers(&event),
            );
            let raw_code = sunrise_client::keybinds::raw_key_code(event.physical_key);
            let action = match (event.state, event.repeat) {
                (winit::event::ElementState::Released, _) => mod_api::KeyAction::Release,
                (_, false) => mod_api::KeyAction::Press,
                (_, true) => mod_api::KeyAction::Repeat,
            };
            // Still the key's handling: a mod's key hook may reach the clipboard.
            self.with_user_input(true, |game| {
                game.key_input(name, layout_key, raw_code, action, modifiers);
            });
        }
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        // Offscreen runs do not use native pointer motion.
        if self.pending_save.is_none()
            && !self.scripted_run()
            && !self.unclaimed_window
            && self.captured
            && self.focused
            && !self.options.offscreen
            && self.screen == Screen::Playing
            && let DeviceEvent::MouseMotion { delta: (x, y) } = event
        {
            if self.pad_radial_captures() {
                return;
            }
            if x != 0.0 || y != 0.0 {
                self.pad_yield_input(event_loop);
            }
            self.grabbed_mouse_motion(x, y);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Every event-loop pass, drawn or not: a window that stops redrawing (a lost
        // device, an occluded window) must still end after a server crash.
        self.exit_after_server_crash();
        self.idle(event_loop);
    }
    /// Android took the window away (the app went to the background, the screen went off): its
    /// surface goes now, the sound stops, and the world is saved unless losing the focus saved it
    /// already, since the system may end the process at any time from here on. Android's UI
    /// thread waits for this to return, so nothing here waits for the world's writes.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "android")]
        {
            crate::diagnostics::heartbeat("suspended", || serde_json::json!({}));
            crate::diagnostics::request_flush();
        }
        if let Some(r) = &mut self.renderer {
            // Pipelines made since the start (a world's, a shader pack's) are kept too: Android
            // may end the process in the background without another word.
            r.save_pipeline_cache();
            r.suspend();
        }
        self.audio.suspend_output();
        self.touch_release_all();
        self.save_for_background();
    }
    /// The application is ending. On macOS, Quit (Cmd+Q) ends it through
    /// applicationWillTerminate without a CloseRequested; vanilla's GLFW
    /// window turns it into a window close, so Minecraft.stop saves the world.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.shut_down();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_trace::InputOrigin;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::{Ime, Modifiers, TouchPhase};
    use winit::keyboard::ModifiersState;

    // A key's `KeyEvent` can't be made outside winit; these are the person's other inputs.
    fn inputs() -> Vec<WindowEvent> {
        let device_id = DeviceId::dummy();
        vec![
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            WindowEvent::MouseWheel {
                device_id,
                delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                phase: TouchPhase::Moved,
            },
            WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(10.0, 20.0),
            },
            WindowEvent::ModifiersChanged(Modifiers::from(ModifiersState::empty())),
            WindowEvent::Ime(Ime::Commit("a".into())),
        ]
    }

    #[test]
    fn a_person_is_locked_out_of_a_scripted_run_only() {
        for event in inputs() {
            assert!(locks_out(InputOrigin::User, true, &event), "{event:?}");
            assert!(!locks_out(InputOrigin::User, false, &event), "{event:?}");
        }
    }

    #[test]
    fn what_the_run_sends_still_plays() {
        for event in inputs() {
            assert!(!locks_out(InputOrigin::Agent, true, &event), "{event:?}");
            assert!(!locks_out(InputOrigin::Game, true, &event), "{event:?}");
        }
    }

    #[test]
    fn a_person_can_still_close_the_window() {
        for event in [
            WindowEvent::CloseRequested,
            WindowEvent::Resized(PhysicalSize::new(854, 480)),
        ] {
            assert!(!locks_out(InputOrigin::User, true, &event), "{event:?}");
            assert_eq!(
                held_focus(InputOrigin::User, true, &event),
                None,
                "{event:?}"
            );
        }
    }

    #[test]
    fn only_a_persons_press_claims_an_agents_window() {
        let device_id = DeviceId::dummy();
        let press = WindowEvent::MouseInput {
            device_id,
            state: ElementState::Pressed,
            button: MouseButton::Left,
        };
        assert_eq!(
            at_unclaimed(InputOrigin::User, true, &press),
            Unclaimed::Claims
        );
        // The focus alone claims nothing, and the game doesn't hear of it.
        for focused in [true, false] {
            let event = WindowEvent::Focused(focused);
            assert_eq!(
                at_unclaimed(InputOrigin::User, true, &event),
                Unclaimed::Dropped
            );
        }
        let release = WindowEvent::MouseInput {
            device_id,
            state: ElementState::Released,
            button: MouseButton::Left,
        };
        for event in inputs().into_iter().skip(1).chain([release]) {
            let want = match event {
                WindowEvent::CursorMoved { .. } => Unclaimed::Passes,
                _ => Unclaimed::Dropped,
            };
            assert_eq!(
                at_unclaimed(InputOrigin::User, true, &event),
                want,
                "{event:?}"
            );
        }
        for event in [
            WindowEvent::CloseRequested,
            WindowEvent::Resized(PhysicalSize::new(854, 480)),
        ] {
            assert_eq!(
                at_unclaimed(InputOrigin::User, true, &event),
                Unclaimed::Passes,
                "{event:?}"
            );
        }
    }

    #[test]
    fn a_persons_modifiers_are_kept_for_the_claim() {
        let shift = Modifiers::from(ModifiersState::SHIFT);
        let event = WindowEvent::ModifiersChanged(shift);
        assert_eq!(held_modifiers(InputOrigin::User, true, &event), Some(shift));
        assert_eq!(held_modifiers(InputOrigin::User, false, &event), None);
        assert_eq!(held_modifiers(InputOrigin::Agent, true, &event), None);
        assert_eq!(
            held_modifiers(InputOrigin::User, true, &WindowEvent::Focused(true)),
            None
        );
    }

    #[test]
    fn the_agent_still_drives_its_unclaimed_window() {
        let events = inputs().into_iter().chain([WindowEvent::Focused(true)]);
        for event in events {
            for origin in [InputOrigin::Agent, InputOrigin::Game] {
                assert_eq!(
                    at_unclaimed(origin, true, &event),
                    Unclaimed::Passes,
                    "{event:?}"
                );
            }
        }
    }

    #[test]
    fn an_ordinary_window_hears_everything() {
        let events = inputs().into_iter().chain([WindowEvent::Focused(true)]);
        for event in events {
            assert_eq!(
                at_unclaimed(InputOrigin::User, false, &event),
                Unclaimed::Passes,
                "{event:?}"
            );
        }
    }

    #[test]
    fn a_persons_focus_change_waits_for_the_run_to_end() {
        for focused in [true, false] {
            let event = WindowEvent::Focused(focused);
            assert_eq!(held_focus(InputOrigin::User, true, &event), Some(focused));
            assert_eq!(held_focus(InputOrigin::User, false, &event), None);
            assert!(!locks_out(InputOrigin::User, true, &event));
        }
    }
}
