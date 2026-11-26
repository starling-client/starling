//! The mouse in play: the look it turns, the pointer grab (`MouseHandler.grabMouse` /
//! `releaseMouse`), the cursor icon, and pausing a world whose window loses focus.
use super::*;
#[path = "pointer_capture/starling.rs"]
mod starling;
#[allow(unused_imports)]
pub use self::starling::*;

fn studio_suppresses_focus_pause(replay_active: bool, freecam_active: bool) -> bool {
    replay_active || freecam_active
}

impl Game<'_> {
    /// The options `MouseHandler.turnPlayer` reads.
    pub(super) fn look_options(&self) -> LookOptions {
        LookOptions {
            sensitivity: self.settings.sensitivity,
            smooth_camera: self.smooth_camera,
            scoping: self.scoping(),
            invert_x: self.settings.invert_x,
            invert_y: self.settings.invert_y,
        }
    }
    /// `MouseHandler.handleAccumulatedMovement` and `Entity.turn`, once per frame.
    /// An agent moving the mouse (`mouse-move`) counts as a grabbed pointer in
    /// an active window, as it cannot grab the real one.
    pub(super) fn apply_mouse_look(&mut self) {
        let mut look = self.look_options();
        let time = self.started.elapsed().as_secs_f64();
        // The touch controls turn the view as a grabbed pointer does (game/touch.rs).
        let pointer = self.captured || self.agent_pointer || self.touch_turns();
        let grabbed = pointer
            && self.screen == Screen::Playing
            && (self.player.is_some() || self.studio.flying());
        let active = self.focused || self.agent_pointer || self.touch_turns();
        if active
            && grabbed
            && mod_api::__private::listened(mod_api::HookId::CalculatePlayerTurn)
            && let Some(remote) = &self.remote
        {
            self.native_calculate_player_turn(
                self.own_entity_id(),
                remote.profile,
                &mut look.sensitivity,
                &mut look.smooth_camera,
            );
        }
        if let Some((xo, yo)) = self
            .mouse
            .handle_accumulated_movement(time, active, grabbed, look)
        {
            self.look_turn(xo, yo);
        }
    }
    /// A look from any device, in `Entity.turn` units: the tutorial's "look
    /// around" step hears it before `Entity.turn` applies it, for the mouse
    /// and a controller alike.
    pub(super) fn look_turn(&mut self, x: f64, y: f64) {
        self.tutorial.mouse(x, y);
        self.turn_player(x, y);
    }
    /// `Minecraft.pause`: a single-player game with a pausing screen open.
    pub(super) fn pauses_game(&self) -> bool {
        self.has_singleplayer_server()
            && !self.is_published()
            && self.title.is_none()
            && top_screen_pauses(
                // A dialog (and its warning or waiting screen) is the
                // screen Minecraft set, over whatever was open.
                self.dialog_open().then(|| self.dialogs.pauses_world()),
                self.book.as_ref().map(|book| book.pauses_world()),
                self.screen.is_pause_screen(),
                self.win_screen_pauses(),
            )
    }
    /// A screen open over gameplay while `screen` stays `Playing`: a book or
    /// lectern, a sign, a server dialog, the code of conduct, a link or
    /// server pack prompt (vanilla's `Minecraft.screen` is set for each).
    pub(super) fn screen_over_playing(&self) -> bool {
        self.book.is_some() || self.sign_edit.is_some() || self.prompt_open()
    }
    /// A prompt is up: over the world or over another open screen, it takes the input and is
    /// answered first, in the order of `handle_window_event`'s arms and of what draws on top.
    pub(super) fn prompt_open(&self) -> bool {
        self.dialog_open()
            || self.code_of_conduct.is_some()
            || self.link_prompt.is_some()
            || self.command_prompt.is_some()
            || self.server_packs.prompt.is_some()
    }
    /// `Minecraft.pauseIfInactive`: a window without focus for more than 500 ms
    /// pauses the game. An open screen (books, signs, the pack prompt, a server
    /// dialog) stays instead of pausing (`Gui.setPauseScreen`).
    pub(super) fn pause_if_inactive(&mut self) {
        if self
            .focus_lost
            .is_some_and(|t| t.elapsed() > Duration::from_millis(500))
            && self.control.is_none()
            && self.screen == Screen::Playing
            && !studio_suppresses_focus_pause(self.replay.playing(), self.studio.flying())
            && self.player.is_some()
            && !self.options.no_pause
            && self.settings.file.bool("pauseOnLostFocus")
            && !self.pad_keeps_playing_unfocused()
            && self.book.is_none()
            && self.sign_edit.is_none()
            && self.server_packs.prompt.is_none()
            && !self.dialog_open()
        {
            self.pause_focus = None;
            self.set_screen(Screen::Paused);
        }
    }
    /// `KeyMapping.setAll`: every key physically held is held again, as the
    /// mapping it is bound to, so a key pressed or kept down behind a screen
    /// counts once the game has the keyboard back. A sneak or sprint that
    /// toggles keeps its toggle state (`shouldSetOnIngameFocus`).
    pub(super) fn restore_held_keys(&mut self) {
        let held: Vec<KeyCode> = self.physical_keys.iter().copied().collect();
        let options = self.toggle_options();
        let has_menu_keys = !self.screen_state.context_menu_input.owned_keys_empty();
        for key in held {
            let restore_sources = if has_menu_keys {
                match self.context_menu_restore_sources(key) {
                    Some(0) => continue,
                    other => other,
                }
            } else {
                None
            };
            if let Some(name) = sunrise_client::keybinds::key_name(key) {
                for slot in self.settings.keys.slots_for(name) {
                    let option =
                        sunrise_client::keymapping::toggle_index(slot).is_some_and(|i| options[i]);
                    if self
                        .vanilla_mappings
                        .state(slot)
                        .should_set_on_ingame_focus(option)
                    {
                        self.vanilla_mappings.set_down(slot, true, option);
                    }
                }
            }
            for mapped in self.settings.keys.translate_all(key) {
                match mapped {
                    Ok(KeyCode::ShiftLeft | KeyCode::ControlLeft) => {}
                    Ok(other) => match restore_sources {
                        None => {
                            self.insert_device_key(other);
                        }
                        Some(mask) => {
                            if mask & 1 != 0 {
                                self.press_device_key(
                                    other,
                                    sunrise_client::held::Source::Keyboard,
                                );
                            }
                            if mask & 2 != 0 {
                                self.press_device_key(other, sunrise_client::held::Source::Agent);
                            }
                        }
                    },
                    Err(_) => {}
                }
            }
        }
    }
    /// `MouseHandler.grabMouse` / `releaseMouse`. Both put the pointer at
    /// the window's centre, so a screen opened from gameplay (inventory,
    /// chest, pause, chat, sign, book, death) starts with the cursor in the
    /// middle instead of wherever the system last left it. Releasing when
    /// already released does nothing, so moving from screen to screen keeps
    /// the cursor where it is. Like GLFW's `glfwSetCursorPos`, nothing warps
    /// while the window is unfocused (losing focus releases the pointer).
    pub(super) fn capture(&mut self, capture: bool) {
        // A guest's action (`as_guest`) opens and closes screens on the
        // guest's own client: the window's pointer stays the host's (#687).
        if self.lan_acting.is_some() {
            return;
        }
        // MouseHandler.grabMouse never captures an inactive window, and vanilla
        // grabs only on `setScreen(null)`: the pack prompt and server dialogs
        // are screens over the world that keep the pointer, as does a screen a
        // mod's `screen_opening` answer kept or opened in place of none.
        if capture
            && ((self.screen_state.answered.get() && self.screen != Screen::Playing)
                || (!self.focused && !self.unclaimed_window && !self.scripted_run())
                || self.server_packs.prompt.is_some()
                || self.dialog_open()
                || self.command_prompt.is_some())
        {
            return;
        }
        // The container screen's press state belongs to the screen: a screen
        // that takes the pointer over starts with its own state, as
        // `AbstractContainerScreen` does, so a press whose release never
        // arrived (the pointer left the window) cannot wedge the next
        // screen's quick move and clicks.
        self.reset_inventory_clicks();
        if !capture {
            self.stop_mining();
            self.studio.rmb_down = false;
        }
        // The real pointer takes over from an agent's.
        self.agent_pointer = false;
        if !capture {
            self.screen_grab = false;
        }
        if capture && !self.captured && !self.screen_grab && !cfg!(target_os = "macos") {
            // MouseHandler.grabMouse: KeyMapping.setAll unless
            // InputQuirks.RESTORE_KEY_STATE_AFTER_MOUSE_GRAB is off (macOS).
            self.restore_held_keys();
        }
        let Some(window) = &self.window else { return };
        let size = self.renderer.as_ref().map_or_else(
            || window.inner_size().into(),
            |r| [r.config.width, r.config.height],
        );
        let was_grabbed = self.captured;
        // Android grabs the mouse as the activity's pointer capture, which comes a moment later
        // (a loss the system makes comes back: game/input_mode.rs). Only a mouse player asks:
        // a finger or a controller plays without it.
        #[cfg(target_os = "android")]
        {
            if capture {
                self.mouse.set_ignore_first_move();
            }
            self.captured =
                capture && self.input_mode == sunrise_client::input_mode::InputMode::KeyboardMouse;
            self.capture_mode = None;
            if self.captured != was_grabbed {
                crate::android::capture_pointer(self.captured);
            }
        }
        #[cfg(not(target_os = "android"))]
        let projection = self.pointer_projection(size);
        #[cfg(not(target_os = "android"))]
        let centre = projection.window.center();
        #[cfg(not(target_os = "android"))]
        let warp = |window: &Window| {
            let _ = window.set_cursor_position(centre);
        };
        #[cfg(not(target_os = "android"))]
        if self.internal_grab() {
            // Its grab is internal (an agent turns the view with `mouse-move`). A grab the
            // window already held when a scripted run began stays the system's until the
            // run ends (`settle_after_run`).
            if capture {
                self.mouse.set_ignore_first_move();
            }
            self.captured = capture;
        } else if capture {
            self.mouse.set_ignore_first_move();
            // macOS warps by re-associating the mouse with the pointer,
            // which undoes a Locked grab and leaves the pointer free: warp
            // first there. Wayland moves a pointer only while it is locked,
            // so everywhere else warp after grabbing (Windows does both).
            #[cfg(target_os = "macos")]
            warp(window);
            // Windows clips a Locked pointer to a 1x1 rectangle where it is, and `SetCursorPos`
            // stays inside that clip: warp first, or the pointer stays frozen wherever it was
            // (over another window, on another monitor) instead of the window's centre (#6076).
            #[cfg(windows)]
            if self.focused {
                warp(window);
            }
            self.capture_mode = if window.set_cursor_grab(CursorGrabMode::Locked).is_ok() {
                Some(CursorGrabMode::Locked)
            } else if window.set_cursor_grab(CursorGrabMode::Confined).is_ok() {
                Some(CursorGrabMode::Confined)
            } else {
                None
            };
            self.captured = self.capture_mode.is_some();
            // Windows already warped above, inside the clip's own position.
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            if self.captured && self.focused {
                warp(window);
            }
        } else {
            if self.captured {
                // Wayland moves a pointer only as the locked pointer's
                // position hint, which the compositor applies as the lock
                // goes (sway and Hyprland do): set it before ungrabbing.
                // X11, Windows and macOS warp, so warp again once free
                // (Wayland refuses that one).
                if self.focused {
                    warp(window);
                }
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                if self.focused {
                    warp(window);
                }
            } else {
                let _ = window.set_cursor_grab(CursorGrabMode::None);
            }
            self.captured = false;
            self.capture_mode = None;
        }
        // MouseHandler.grabMouse and releaseMouse: the mouse position is
        // the window's centre whenever the pointer's grab state changes, so
        // a screen opened from gameplay starts with the cursor in the middle
        // and the game starts with it away from the screen's last spot.
        if !self.internal_grab() {
            window.set_cursor_visible(!self.captured);
        }
        #[cfg(not(target_os = "android"))]
        if let Some(pos) = projection
            .window
            .center_after_grab_change(capture, was_grabbed)
        {
            self.set_window_cursor(projection.window.physical_sample(pos), projection);
        }
    }
    /// The open mod screen holds the pointer (`ScreensClientHost::set_pointer_grab`) and the
    /// window has it: the device's motion is the screen's.
    pub(super) fn mod_screen_grabs_pointer(&self) -> bool {
        self.screen_grab && self.captured && self.screen == Screen::Mod
    }
    /// The device's motion with the pointer grabbed (`MouseHandler.onMove`, applied once per
    /// frame): a grabbing mod screen's `screen_pointer_motion`, otherwise the camera's.
    pub(in crate::game) fn grabbed_pointer_motion(&mut self, [x, y]: [f64; 2]) {
        self.note_input(
            input_trace::InputOrigin::User,
            input_trace::TraceKind::Motion { x, y },
        );
        if self.mod_screen_grabs_pointer() {
            self.screen_pointer_motion([x, y]);
        } else {
            self.mouse.on_move(0.0, 0.0, x, y, true, true);
        }
        self.last_input = Instant::now();
    }
    /// Whether the open screen is the calling mod's own (`mod_api::content::crash::current_native_mod`).
    fn calling_mod_owns_screen(&self) -> bool {
        mod_api::content::crash::current_native_mod()
            .is_some_and(|owner| self.mod_screen() == Some(owner))
    }
    /// `ScreensClientHost::set_pointer_grab`: the grab of the calling mod's own open screen,
    /// taken at the next `apply_pointer_grab_request`. Refused (nothing changes) when the open
    /// screen is another mod's or none, or, for a grab, when the window is not focused
    /// (`MouseHandler.grabMouse` never grabs an inactive window; `capture` allows the same
    /// unclaimed and scripted windows).
    pub(in crate::game) fn set_pointer_grab(&self, grab: bool) -> bool {
        if !self.calling_mod_owns_screen() {
            return false;
        }
        if !grab {
            if self.pointer_grabbed() {
                self.pointer_grab_request.set(Some(false));
            }
            return true;
        }
        if !(self.focused || self.unclaimed_window || self.scripted_run())
            || self.has_live_context_menu()
        {
            return false;
        }
        self.pointer_grab_request.set(Some(true));
        true
    }
    /// `ScreensClientHost::pointer_grabbed`: the calling mod's open screen holds the grab, as
    /// asked for if the request is still waiting.
    pub(in crate::game) fn pointer_grabbed(&self) -> bool {
        self.calling_mod_owns_screen()
            && self
                .pointer_grab_request
                .get()
                .unwrap_or_else(|| self.mod_screen_grabs_pointer())
    }
    /// Applies what `set_pointer_grab` asked for, and ends a grab whose screen's mod is off.
    /// The screen's own `capture(true)` skips vanilla's `KeyMapping.setAll` (the screen, not
    /// the game, has the keyboard) and is not refused by an earlier `screen_opening` answer,
    /// which only meant that caller's grab after that change. Released by `capture(false)`,
    /// which every screen change and focus loss run.
    pub(in crate::game) fn apply_pointer_grab_request(&mut self) {
        let request = self.pointer_grab_request.take();
        let owner_off = self.native_mod_screen.as_deref().is_some_and(|owner| {
            mod_api::__private::mod_index(owner)
                .is_some_and(|i| !mod_api::__private::mod_index_on(i))
        });
        if self.screen_grab && (owner_off || request == Some(false)) {
            self.capture(false);
        } else if request == Some(true)
            && !owner_off
            && self.screen == Screen::Mod
            && !self.live_context_menu()
        {
            self.screen_grab = true;
            self.screen_state.answered.set(false);
            self.capture(true);
            if !self.captured {
                self.screen_grab = false;
            }
        }
    }
    /// `MouseHandler.isMouseGrabbed`: whether the window holds the pointer.
    pub(in crate::game) fn is_mouse_grabbed(&self) -> bool {
        self.captured
    }
    /// `MouseHandler.grabMouse` and `releaseMouse` for a mod. Grabbing does nothing when the
    /// window is inactive or already holds the pointer, or where `Gui.setScreen(null)` would
    /// not return to the world: no player in a world, a connecting or loading screen, the
    /// death screen. Otherwise it closes the open screen (a container's menu with it, so the
    /// server is told), grabs through `capture`, whose guards (a screen a mod kept, a prompt,
    /// a dialog, a LAN guest's action) still hold, and sets `missTime` as `grabMouse` does.
    /// Releasing does nothing when the pointer is not grabbed.
    pub(in crate::game) fn set_mouse_grabbed(&mut self, grabbed: bool) {
        if self.lan_acting.is_some() || !self.options.execution_role.is_client() {
            return;
        }
        if !grabbed {
            if self.captured {
                self.capture(false);
            }
            return;
        }
        if self.captured
            || !(self.focused || self.unclaimed_window || self.scripted_run())
            || self.player.is_none()
            || self.remote_screen_up()
            || self.screen == Screen::Dead
        {
            return;
        }
        if self.screen == Screen::Inventory {
            self.close_container();
        }
        self.set_screen(Screen::Playing);
        if self.screen == Screen::Playing && self.title.is_none() && !self.screen_over_playing() {
            self.capture(true);
            if self.captured {
                self.clicks.miss_time = 10000;
            }
        }
    }
    /// The game's pointer grab leaves the system pointer where the person at the computer has
    /// it: a genuinely hidden `--offscreen` run, an agent's window no one has clicked yet, or a
    /// scripted run, which plays the game while the person may be using the computer.
    pub(super) fn internal_grab(&self) -> bool {
        self.options.offscreen || self.unclaimed_window || self.scripted_run()
    }
    /// A person pressed a mouse button inside the agent's window (`at_unclaimed`): from now on
    /// it is a window like any other, and that press takes the pointer
    /// (`MouseHandler.onPress`) instead of attacking. The focus changes it didn't hear while
    /// unclaimed, or an agent's `pad focus`, may have left the game's idea of the focus apart
    /// from the window's: it catches up first, as a `Focused` would have told it. So do the
    /// modifiers the person holds (`held_modifiers`).
    pub(super) fn claim_window(&mut self, event_loop: &dyn EventLoopCtl) {
        self.unclaimed_window = false;
        self.captured = false;
        // A regrab owed since an agent's `pad focus false` would take the pointer as the focus
        // catches up, and the press would then attack.
        self.regrab_on_refocus = false;
        let Some(window) = &self.window else { return };
        let (id, focused) = (window.id(), window.has_focus());
        if focused != self.focused {
            self.handle_window_event(
                event_loop,
                id,
                WindowEvent::Focused(focused),
                input_trace::InputOrigin::User,
            );
        }
        if let Some(modifiers) = self.unclaimed_modifiers.take() {
            self.handle_window_event(
                event_loop,
                id,
                WindowEvent::ModifiersChanged(modifiers),
                input_trace::InputOrigin::User,
            );
        }
    }
    /// Once a scripted run is over, the person's last click into the window or away from it
    /// applies (`held_focus`), and the system pointer follows the grab the run left inside the
    /// game, as if the window had taken or released it now.
    pub(super) fn settle_after_run(&mut self, event_loop: &dyn EventLoopCtl) {
        if self.scripted_run() {
            return;
        }
        if let Some(focused) = self.focus_after_run.take() {
            let id = self
                .window
                .as_ref()
                .map_or_else(WindowId::dummy, |w| w.id());
            self.handle_window_event(
                event_loop,
                id,
                WindowEvent::Focused(focused),
                input_trace::InputOrigin::User,
            );
        }
        // Outside a run, a window whose grab is the system's holds it exactly while the game
        // has it grabbed; only a run leaves the two apart.
        if regrab_after_run(
            cfg!(target_os = "android"),
            self.internal_grab(),
            self.captured,
            self.capture_mode.is_some(),
        ) {
            let grab = self.captured;
            self.captured = self.capture_mode.is_some();
            self.capture(grab);
        }
    }
    /// A Linux window manager can release the native pointer grab while the
    /// window remains focused. Reapply the recorded mode on the next gameplay
    /// click or wheel event, without warping the pointer or consuming input.
    pub(super) fn reassert_pointer_capture(&mut self) {
        #[cfg(target_os = "linux")]
        if self.screen == Screen::Playing
            && self.focused
            && self.captured
            && !self.options.offscreen
            && !self.internal_grab()
            && let (Some(window), Some(mode)) = (&self.window, self.capture_mode)
        {
            let other = match mode {
                CursorGrabMode::Locked => CursorGrabMode::Confined,
                CursorGrabMode::Confined => CursorGrabMode::Locked,
                CursorGrabMode::None => return,
            };
            self.capture_mode = if window.set_cursor_grab(mode).is_ok() {
                Some(mode)
            } else if window.set_cursor_grab(other).is_ok() {
                Some(other)
            } else {
                None
            };
            self.captured = self.capture_mode.is_some();
            window.set_cursor_visible(!self.captured);
        }
    }
    /// `GuiGraphicsExtractor.applyCursor` -> `Window.selectCursor` at the end
    /// of a frame: the cursor the frame's widgets asked for, set on the window
    /// only when it changes.
    pub(super) fn apply_cursor_icon(&mut self) {
        use sunrise_client::widgets::Cursor;
        let wanted = sunrise_client::widgets::take_cursor();
        // The widgets under the keyboard's hidden pointer aren't under the
        // mouse (`handleCursor` asks only for a hovered widget).
        let effective = if self.settings.allow_cursor_changes && !self.key_focus_driving() {
            wanted
        } else {
            Cursor::Arrow
        };
        if effective == self.cursor_icon {
            return;
        }
        if let Some(window) = &self.window {
            window.set_cursor(match effective {
                Cursor::Arrow => winit::window::CursorIcon::Default,
                Cursor::PointingHand => winit::window::CursorIcon::Pointer,
                Cursor::Ibeam => winit::window::CursorIcon::Text,
                Cursor::NotAllowed => winit::window::CursorIcon::NotAllowed,
                Cursor::ResizeNs => winit::window::CursorIcon::NsResize,
                Cursor::ResizeEw => winit::window::CursorIcon::EwResize,
            });
        }
        self.cursor_icon = effective;
    }
    /// LevelLoadingScreen.onClose -> Gui.setScreen(null) -> grabMouse.
    pub(super) fn capture_loaded_world(&mut self) {
        // Loading may finish after focus was lost or another screen opened.
        if self.title.is_none() && self.screen == Screen::Playing {
            self.capture(true);
        }
    }
    /// The window came back into focus after holding the pointer: the system
    /// dropped the grab with the focus, and vanilla's grab outlives it
    /// (`MouseHandler.mouseGrabbed` stays true while GLFW keeps the cursor
    /// mode), so the game takes the pointer again without waiting for a
    /// click. A screen that is open keeps it free, as releasing the pointer
    /// for that screen did: it is the screen, not the focus, that shows it.
    pub(super) fn regrab_on_refocus(&mut self) {
        if self.studio_up() {
            self.regrab_on_refocus = false;
            self.studio.rmb_down = false;
            self.capture(false);
            return;
        }
        if std::mem::take(&mut self.regrab_on_refocus) && self.screen == Screen::Playing {
            self.capture(true);
        }
    }
}

/// Which of the things over gameplay decides `Minecraft.pause`: a dialog is
/// its own `DialogScreen` (Minecraft set it over whatever was open) and
/// comes first, then a book, else `Gui.screen`; the win screen's poem rides
/// with its screen. `None` means that kind of screen is not open.
fn top_screen_pauses(dialog: Option<bool>, book: Option<bool>, screen: bool, win: bool) -> bool {
    dialog.or(book).unwrap_or(screen || win)
}

#[cfg(test)]
mod tests {
    #[path = "../pointer_capture/tests/starling.rs"]
    mod starling;
    #[allow(unused_imports)]
    use self::starling::*;
    use super::{regrab_after_run, studio_suppresses_focus_pause, top_screen_pauses};

    #[test]
    #[ignore = "daily"]
    fn movement_key_held_behind_a_screen_counts_after_pointer_grab() {
        use super::*;
        use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};

        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&ResourceSource::directory(root)),
            None,
        )
        .unwrap();

        game.focused = true;
        game.screen = Screen::Paused;
        game.physical_keys.insert(KeyCode::KeyW);
        assert!(!game.movement_key_held(KeyCode::KeyW));

        game.screen = Screen::Playing;
        game.capture(true);
        assert_eq!(
            game.movement_key_held(KeyCode::KeyW),
            !cfg!(target_os = "macos")
        );
    }

    /// `MouseHandler.releaseMouse` and `grabMouse` through the mod host's pair: releasing drops
    /// the grab, grabbing an inactive window does nothing, and grabbing with a screen open
    /// closes it and takes the pointer.
    #[test]
    #[ignore = "daily"]
    fn mod_host_releases_and_grabs_the_pointer_as_vanilla_does() {
        use super::*;
        use mod_api::prelude::ClientInputClientHost;
        use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};

        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::Client;
        options.offscreen = true;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&ResourceSource::directory(root)),
            None,
        )
        .unwrap();
        let (_event_loop, window) = crate::game::native_entity_refs::tests::test_window([640, 480]);
        game.window = Some(window);
        game.title = None;
        game.screen = Screen::Playing;
        game.player = Some(
            sunrise_client::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::ZERO,
            )
            .unwrap(),
        );

        // Release while playing clears the grab.
        game.focused = true;
        game.captured = true;
        assert!(ClientInputClientHost::is_mouse_grabbed(&game));
        ClientInputClientHost::set_mouse_grabbed(&mut game, false);
        assert!(!ClientInputClientHost::is_mouse_grabbed(&game));

        // Grab while the window is inactive stays released.
        game.focused = false;
        ClientInputClientHost::set_mouse_grabbed(&mut game, true);
        assert!(!ClientInputClientHost::is_mouse_grabbed(&game));

        // Grab with no player in a world does nothing: `setScreen(null)` opens the title.
        game.focused = true;
        game.screen = Screen::Paused;
        let player = game.player.take();
        ClientInputClientHost::set_mouse_grabbed(&mut game, true);
        assert!(!ClientInputClientHost::is_mouse_grabbed(&game));
        assert!(game.screen == Screen::Paused);
        game.player = player;

        // Grab on the death screen does nothing: `setScreen(null)` keeps the death screen.
        game.screen = Screen::Dead;
        ClientInputClientHost::set_mouse_grabbed(&mut game, true);
        assert!(!ClientInputClientHost::is_mouse_grabbed(&game));
        assert!(game.screen == Screen::Dead);

        // Grab with a container screen open closes the container and the screen, takes the
        // pointer, and sets `missTime`.
        game.screen = Screen::Inventory;
        game.craft_size = 3;
        game.clicks.miss_time = 0;
        ClientInputClientHost::set_mouse_grabbed(&mut game, true);
        assert!(game.screen == Screen::Playing);
        assert_eq!(game.craft_size, 2);
        assert!(ClientInputClientHost::is_mouse_grabbed(&game));
        assert_eq!(game.clicks.miss_time, 10000);
    }

    #[test]
    fn studio_replay_and_freecam_never_pause_on_focus_loss() {
        assert!(studio_suppresses_focus_pause(true, false));
        assert!(studio_suppresses_focus_pause(false, true));
        assert!(!studio_suppresses_focus_pause(false, false));
    }

    /// `Minecraft.pause` asks the screen Minecraft set, and each of a dialog,
    /// a book and the win screen is a screen of its own that replaces what
    /// was open: the dialog's `pause`, the book's `BookViewScreen`/
    /// `LecternScreen`, and `WinScreen`'s `!poem` decide, not the screen
    /// underneath.
    #[test]
    fn the_screen_on_top_decides_whether_the_world_pauses() {
        // Nothing replaces the screen: its own isPauseScreen answer, with
        // the win screen's poem rule when it is the win screen.
        assert!(top_screen_pauses(None, None, true, false));
        assert!(top_screen_pauses(None, None, false, true));
        assert!(!top_screen_pauses(None, None, false, false));
        // A dialog over a pause screen: its own pause answers.
        assert!(!top_screen_pauses(Some(false), None, true, false));
        assert!(top_screen_pauses(Some(true), None, false, false));
        // A lectern's book over a pause screen does not pause; a book in
        // hand does.
        assert!(!top_screen_pauses(None, Some(false), true, false));
        assert!(top_screen_pauses(None, Some(true), false, false));
        // A dialog opens over the book.
        assert!(!top_screen_pauses(Some(false), Some(true), false, false));
    }
}
