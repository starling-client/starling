// game-module-cfg: target_os = "android"
// Adapted from Horizon Android at fa548eb7abe91c194d3780e4580b9746323cd606 (MIT).
// Copyright (c) 2026 Ludovic Marie and the Horizon contributors.
// See THIRD_PARTY_NOTICES.md and LICENSE.
//! Extra desktop keys, a touch keyboard and menu scrolling on Android.
use super::input_trace::InputOrigin;
use super::*;
use horizon_client::{
    android_control_keyboard::Output,
    android_control_ui::{Action, Gesture},
    debug_keys::KeyAction,
};
use winit::event::{Touch, TouchPhase};
mod mobile_platform;

#[derive(Default)]
pub(super) struct MobileControls {
    pub controls: horizon_client::android_controls::TouchControls,
    pub keypad: horizon_client::keypad::Keypad,
    pub ui: horizon_client::android_control_ui::TouchUi,
    custom_keys: std::collections::HashSet<KeyCode>,
    custom_pressed_at: std::collections::HashMap<String, u64>,
    late_custom_releases: Vec<String>,
    hardware_keys: std::collections::HashSet<KeyCode>,
    pub touch_shield: bool,
    pub touch_aim: Option<[f32; 2]>,
    pub loaded: bool,
    frame_at: Option<Instant>,
    was_in_menu: Option<bool>,
}

impl Game<'_> {
    pub(in crate::game) fn mobile_keypad_key(
        &mut self,
        events: &dyn super::event_loop::EventLoopCtl,
        event: &winit::event::KeyEvent,
    ) -> bool {
        if !self.settings.keypad.enabled {
            return false;
        }
        let Some(key) = horizon_client::keypad::resolve_key(event.physical_key, &event.logical_key)
        else {
            return false;
        };
        if self.options_waiting_for_key() || mobile_platform::active() || self.text_field_focused()
        {
            return false;
        }
        if self.pad_in_menu() {
            if let Some(button) = self.settings.keypad.menu_button(key) {
                if !event.repeat {
                    self.pad.in_use = true;
                    self.pad_menu_button(events, button, event.state == ElementState::Pressed)
                        .unwrap_or_else(|e| log::warn!("Keypad menu: {e}"));
                }
                return true;
            }
            if event.state != ElementState::Released {
                return false;
            }
        }
        self.mobile.keypad.configure(&self.settings.keypad);
        self.mobile
            .keypad
            .key(key, event.state == ElementState::Pressed)
    }

    fn mobile_rocket_boost(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.fall_flying)
            && self
                .held()
                .into_iter()
                .chain(self.offhand.as_ref())
                .any(|s| s.item.as_ref() == "minecraft:firework_rocket")
    }
    fn mobile_hold_uses_item(&self) -> bool {
        self.mobile_rocket_boost()
            || self
                .held()
                .into_iter()
                .chain(self.offhand.as_ref())
                .any(|s| {
                    !horizon_client::android_controls::item_uses_on_tap(&s.item)
                        && self
                            .items()
                            .get(&s.item)
                            .is_some_and(super::remote::item_has_use)
                })
    }
    pub(in crate::game) fn mobile_controls_enabled(&self) -> bool {
        cfg!(target_os = "android")
    }
    pub(in crate::game) fn mobile_pad_end(&mut self, released: bool) {
        if released {
            self.mobile.touch_aim = None;
        }
    }
    pub(in crate::game) fn mobile_back_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{KeyCode, PhysicalKey};
        if event.state != ElementState::Pressed {
            return false;
        }
        if event.physical_key == PhysicalKey::Code(KeyCode::Escape)
            || event.logical_key
                == winit::keyboard::Key::Named(winit::keyboard::NamedKey::BrowserBack)
        {
            if mobile_platform::active() {
                mobile_platform::hide();
                return true;
            }
            if self.dialog_open() {
                if let Err(e) = self.dialog_key(KeyCode::Escape, None, None) {
                    log::warn!("Dialog Back: {e}");
                }
                return true;
            }
            if self.mobile.controls.editor.is_some() {
                self.mobile.controls.request_editor_back();
                return true;
            }
            if self.mobile.ui.floating_menu {
                self.mobile.ui.floating_menu = false;
                return true;
            }
        }
        if let PhysicalKey::Code(key) = event.physical_key {
            if self.mobile.controls.editor.is_some()
                && self.mobile.controls.capture_custom_key_code(key)
            {
                return true;
            }
            if let Some(token) = self.mobile.ui.capture_custom_input_key(key) {
                self.android_custom_token_tap(&token);
                return true;
            }
        }
        false
    }
    pub(in crate::game) fn mobile_shift_down(&self) -> bool {
        (matches!(self.screen, Screen::Inventory | Screen::Creative)
            && self.mobile.controls.inventory_shift)
            || self.mobile.ui.modifier_latched(KeyCode::ShiftLeft)
            || self.mobile.custom_keys.contains(&KeyCode::ShiftLeft)
            || self.mobile.custom_keys.contains(&KeyCode::ShiftRight)
    }
    pub(in crate::game) fn mobile_control_down(&self) -> bool {
        self.mobile.ui.modifier_latched(KeyCode::ControlLeft)
            || self.mobile.custom_keys.contains(&KeyCode::ControlLeft)
            || self.mobile.custom_keys.contains(&KeyCode::ControlRight)
    }
    fn mobile_click_feedback(&mut self) {
        if std::mem::take(&mut self.mobile.controls.clicked) {
            self.audio.play_button_click();
            self.android_touch_haptic(10);
        }
    }
    pub(in crate::game) fn mobile_options_apply(&mut self) {
        self.mobile.controls.settings = self.settings.touch.clone();
        self.mobile.ui.menu_direct_pointer = self.settings.touch.virtual_mouse_hidden;
        self.input_used(self.input_mode);
        if let Err(e) = self.mobile.controls.settings.save() {
            log::warn!("Controls settings: {e}");
        }
    }
    pub(in crate::game) fn mobile_options_open(&mut self) {
        self.mobile_init();
        self.settings.touch = self.mobile.controls.settings.clone();
    }
    fn mobile_init(&mut self) {
        if self.mobile.loaded {
            return;
        }
        self.mobile.loaded = true;
        self.input_used(self.input_mode);
        if let Some(w) = &self.window {
            horizon_client::android_control_theme::set_android_dpi(w.scale_factor() as f32);
        }
        self.mobile.controls.settings = self.settings.touch.clone();
        self.settings.touch.virtual_mouse_hidden = true;
        self.mobile.controls.settings.virtual_mouse_hidden = true;
        self.mobile.ui.menu_direct_pointer = true;
        self.mobile.ui.goy_in_game_mouse = false;
        self.mobile
            .ui
            .refresh_presets_cache(self.mobile.controls.settings.control_style);
    }
    pub(in crate::game) fn mobile_keyboard_active(&self) -> bool {
        mobile_platform::active()
    }
    pub(in crate::game) fn mobile_open_editor(&mut self) {
        self.mobile_init();
        self.android_release_touch_keys(true);
        self.mobile.controls.open_editor();
        self.capture(false);
    }
    pub(in crate::game) fn mobile_frame(&mut self, events: &dyn super::event_loop::EventLoopCtl) {
        self.mobile_init();
        self.mobile.ui.hub_visible = self.mobile.controls.settings.show_control_hub;
        self.mobile.ui.utility_offset_y = if self.screen == Screen::Playing
            && horizon_client::accounts::in_game()
            && self
                .title
                .as_ref()
                .is_some_and(|t| matches!(t.mode, horizon_client::title::Mode::Main))
        {
            24.
        } else {
            0.
        };
        self.android_poll_keyboard();
        let now = Instant::now();
        let dt = self
            .mobile
            .frame_at
            .replace(now)
            .map_or(0., |at| now.duration_since(at).as_secs_f32().min(0.2));
        let in_menu = self.pad_in_menu();
        if self
            .mobile
            .was_in_menu
            .replace(in_menu)
            .is_some_and(|old| old != in_menu)
        {
            self.android_release_touch_keys(true);
            self.pad.set_touch_state(Default::default());
        }
        let shown = self.touch_shown() || self.settings.keypad.enabled;
        let context_use = self.touch_context_use();
        let rocket_boost = self.mobile_rocket_boost();
        let item_use = self.mobile_hold_uses_item();
        let entity_use = self.touch_entity_accepts_use();
        let shield_available = self
            .held()
            .into_iter()
            .chain(self.offhand.as_ref())
            .any(|s| self.blocks_attacks(s).is_some());
        let breaking = self.touch_mining_progress();
        let tc = &mut self.mobile.controls;
        tc.auxiliary_only = self.settings.keypad.enabled;
        tc.shield_available = shield_available;
        tc.context_use = tc.settings.contextual_interact && context_use;
        tc.goy_rendered_by_ui = true;
        tc.break_progress = breaking;
        tc.in_menu = in_menu;
        tc.in_inventory = matches!(self.screen, Screen::Inventory | Screen::Creative);
        let (width, height) = (
            self.window.as_ref().map_or(1, |w| w.inner_size().width),
            self.window.as_ref().map_or(1, |w| w.inner_size().height),
        );
        tc.viewport(width, height);
        tc.safe_insets(width, height, crate::android::safe_insets());
        let flying = self.player.as_ref().is_some_and(|p| p.flying);
        if shown && flying && !tc.vertical_movement {
            self.toggles.0[0] = horizon_client::keymapping::KeyState::toggle(true, true);
            self.keys.remove(&KeyCode::ShiftLeft);
        }
        tc.set_vertical_movement(flying);
        tc.swimming = self.player.as_ref().is_some_and(|p| p.swimming);
        tc.mounted = self.riding.is_some();
        tc.sneaking = self.keys.raw(&KeyCode::ShiftLeft);
        tc.toggle_crouch = self.settings.toggle_crouch;
        tc.update_visibility(shown, dt);
        let editing = tc.editor.is_some() || self.mobile.ui.floating_menu;
        let mut state = if in_menu || editing || !self.focused {
            horizon_client::gamepad::PadState::default()
        } else {
            tc.poll_state()
        };
        self.mobile.touch_aim = tc.aim.or(self.mobile.touch_aim);
        self.mobile.touch_shield = shown
            && !in_menu
            && shield_available
            && if tc.settings.shield_button {
                tc.shield_held()
            } else {
                tc.sneaking || state.held(horizon_client::gamepad::Button::RightStick)
            };
        if self.mobile.touch_shield {
            state.triggers[0] = 1.;
        }
        if tc.aim.is_some() && tc.holding && (tc.holding_use || item_use || entity_use) {
            state.triggers = [1., 0.];
            tc.latch_use();
        }
        let tap = tc.take_tap();

        let chat = std::mem::take(&mut tc.chat_requested);
        let perspective = std::mem::take(&mut tc.perspective_requested);
        let debug = std::mem::take(&mut tc.debug_requested);
        let settings = std::mem::take(&mut tc.settings_requested);
        if let Some(held_use) = tap {
            if self.pick_entity_hit().is_some()
                && self.using.is_none()
                && self.screen == Screen::Playing
            {
                self.remote_start_attack();
            } else if !held_use && !rocket_boost {
                state.triggers[0] = 1.;
            }
        }
        self.pad.set_touch_state(state);
        // The chat and camera buttons act as their mappings, not as the D-pad buttons that held
        // them: Horizon's controller layout moved chat to D-pad up and the radial menu to right.
        for (requested, action) in [(chat, "key.chat"), (perspective, "key.togglePerspective")] {
            if requested
                && let Err(e) = self
                    .pad_action(action, true)
                    .and_then(|()| self.pad_action(action, false))
            {
                log::warn!("Starling: touch {action}: {e:#}");
            }
        }
        if debug {
            self.debug_key(KeyCode::F3, KeyAction::Press);
            self.debug_key(KeyCode::F3, KeyAction::Release);
        }
        if settings {
            self.mobile.ui.floating_menu = false;
            self.open_options(self.title.is_some());
            self.capture(false);
        }

        self.android_custom_events();
        self.android_virtual_mouse_clicks(events);
        let ticking = self.title.is_none() && self.screen == Screen::Playing && !self.pause;
        for token in std::mem::take(&mut self.mobile.late_custom_releases) {
            if !ticking || self.mobile.custom_pressed_at.get(&token) != Some(&self.ticks) {
                self.android_dispatch_custom_token(&token, false);
            } else {
                self.mobile.late_custom_releases.push(token);
            }
        }
        self.touch_gyro();
    }
    pub(in crate::game) fn mobile_draw(&self, b: &mut HudBatch) {
        use horizon_client::android_control_ui as ui;
        if !self.has_touch()
            && !self.settings.keypad.enabled
            && self.mobile.controls.editor.is_none()
            && !self.mobile.ui.floating_menu
        {
            return;
        }
        if self.ui.hide_gui
            && self.mobile.controls.editor.is_none()
            && !self.mobile.ui.floating_menu
        {
            return;
        }
        let tc = &self.mobile.controls;
        // Starling: while editing, only Horizon Android's editor draws. `TouchControls::draw`
        // would add Horizon's own editor (Done, Grid, Add buttons...) under its canvas, seen
        // through it but never reached by a touch.
        if tc.editor.is_none() {
            tc.draw(b, &self.gui.atlas);
            ui::draw_goy_hud(
                tc,
                b,
                &self.gui.atlas,
                self.pad_in_menu(),
                self.mobile.ui.virtual_mouse,
            );
            // Out of the camera cutout's strips with the controls (#166), the hub with the keys
            // beside it; `android_aux_touch` and `floating_menu_touch` read them there.
            self.draw_clear_of_cutout(b, |b, _| {
                self.mobile.ui.draw(b, &self.gui.atlas, self.pad_in_menu());
                self.mobile
                    .ui
                    .draw_floating_menu(b, &self.gui.atlas, &tc.settings);
            });
        }
        ui::draw_hud_editor(tc, b, &self.gui.atlas, &tc.settings.preset_id, None);
        self.mobile.ui.draw_keyboard(b, &self.gui.atlas);
        if self.mobile.ui.virtual_mouse {
            ui::draw_pointer(
                b,
                &self.gui.atlas,
                self.cursor,
                tc.settings.virtual_mouse_pointer_scale,
            );
        }
    }
    pub(in crate::game) fn mobile_touch(
        &mut self,
        t: &Touch,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
    ) {
        self.mobile_init();
        let [w, h] = self.touch_size();
        let mut b = HudBatch::new(
            [w, h],
            horizon_client::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        let p = [t.location.x as f32 / b.scale, t.location.y as f32 / b.scale];
        let n = [
            t.location.x as f32 / w.max(1) as f32,
            t.location.y as f32 / h.max(1) as f32,
        ];
        if self.mobile.controls.editor.is_some() {
            horizon_client::android_control_ui::touch_hud_editor(
                &mut self.mobile.controls,
                &mut b,
                &self.gui.atlas,
                t.id,
                n,
                t.phase == TouchPhase::Started,
                matches!(t.phase, TouchPhase::Ended | TouchPhase::Cancelled),
            );
            self.settings.touch = self.mobile.controls.settings.clone();
            self.mobile_click_feedback();
            return;
        }
        if t.phase == TouchPhase::Started
            && let Some(out) = self.mobile.ui.keyboard_touch(&mut b, &self.gui.atlas, p)
        {
            self.android_preset_typing(&out);
            return;
        }
        match self.android_aux_touch(t, events, id) {
            Ok(true) => {
                self.mobile_click_feedback();
                return;
            }
            Err(e) => {
                log::warn!("Touch: {e}");
                return;
            }
            _ => {}
        }
        let in_menu = self.pad_in_menu();
        self.mobile.controls.in_menu =
            in_menu && !(self.screen == Screen::Options && self.options_waiting_for_key());
        // The controls drawn over a container take their touches: every one with the inventory
        // controls on, and else the ones the MCPE style shows there anyway, Sneak (Shift-click)
        // and Inventory (#168, #235: the Shift button was drawn and a tap went through it).
        let shown_over_container = self.mobile.controls.settings.inventory_controls
            || self.mobile.controls.control_at(n).is_some_and(|c| {
                matches!(
                    c,
                    horizon_client::android_controls::Control::Sneak
                        | horizon_client::android_controls::Control::Inventory
                )
            });
        let inventory_control = shown_over_container
            && matches!(self.screen, Screen::Inventory | Screen::Creative)
            && (self.mobile.controls.owns_pointer(t.id)
                || (t.phase == TouchPhase::Started && self.mobile.controls.control_at_point(n)));
        if inventory_control
            && t.phase == TouchPhase::Started
            && self.mobile.controls.custom_at(n).is_none()
            && self.mobile.controls.control_at(n)
                == Some(horizon_client::android_controls::Control::Sneak)
        {
            self.mobile.controls.inventory_shift ^= true;
            self.mobile.controls.ignore(t.id);
            self.audio.play_button_click();
            self.android_touch_haptic(10);
            return;
        }
        let menu_custom = in_menu
            && (self.mobile.controls.owns_pointer(t.id)
                || (t.phase == TouchPhase::Started && self.mobile.controls.custom_at(n).is_some()));
        if (menu_custom || inventory_control)
            && self.mobile.controls.menu_update(
                t.id,
                n,
                t.phase == TouchPhase::Started,
                matches!(t.phase, TouchPhase::Ended | TouchPhase::Cancelled),
            )
        {
            self.android_custom_events();
            self.mobile_click_feedback();
            return;
        }
        if inventory_control
            && t.phase == TouchPhase::Started
            && self.mobile.controls.custom_at(n).is_none()
        {
            use horizon_client::android_controls::Control as C;
            if let Some(c) = self.mobile.controls.control_at(n)
                && !matches!(c, C::Debug | C::Settings | C::Perspective)
            {
                self.mobile.controls.ignore(t.id);
                if matches!(c, C::Inventory | C::Pause) {
                    let _ = self.pad_back();
                }
                return;
            }
        }
        if in_menu
            && self.mobile.controls.consume_ignored(
                t.id,
                matches!(t.phase, TouchPhase::Ended | TouchPhase::Cancelled),
            )
        {
            return;
        }
        if in_menu {
            if !self.mobile.ui.virtual_mouse
                && !self.has_shift_down()
                && self.slot_touch_event(events, id, t)
            {
                return;
            }
            self.android_menu_touch(t, events, id);
            return;
        }
        if t.phase == TouchPhase::Started
            && !self.mobile.controls.control_at_point(n)
            && let Some(slot) = horizon_client::android_controls::hotbar_slot(b.width, b.height, p)
        {
            self.selected = slot;
            self.mobile.controls.ignore(t.id);
            return;
        }
        if !self.captured && self.focused {
            self.capture(true);
        }
        self.mobile.controls.projectile_crosshair = self
            .held()
            .into_iter()
            .chain(self.offhand.as_ref())
            .any(|s| horizon_client::android_controls::projectile_crosshair(&s.item));
        self.mobile.controls.hold_uses_item = self.mobile_hold_uses_item();
        let delta = self.mobile.controls.update(
            t.id,
            n,
            t.phase == TouchPhase::Started,
            matches!(t.phase, TouchPhase::Ended | TouchPhase::Cancelled),
        );
        let turn = horizon_client::android_controls::camera_delta_with_mouse(
            delta,
            w,
            h,
            &self.mobile.controls.settings,
            self.settings.sensitivity,
            self.settings.invert_x,
            self.settings.invert_y,
            self.zoom_turn_scale() as f32,
        );
        if self.mobile.ui.virtual_mouse {
            let speed = self
                .mobile
                .controls
                .settings
                .virtual_mouse_sensitivity
                .clamp(0.25, 4.);
            self.cursor = (
                (self.cursor.0 + delta[0] * b.width * speed).clamp(0., b.width),
                (self.cursor.1 + delta[1] * b.height * speed).clamp(0., b.height),
            );
        }
        if turn != [0., 0.] {
            self.tutorial.mouse(turn[0] as f64, turn[1] as f64);
            self.turn_player(turn[0] as f64 / 0.15, turn[1] as f64 / 0.15);
        }
        self.android_custom_events();
    }
    pub(in crate::game) fn mobile_layout_command(
        &mut self,
        words: &[&str],
    ) -> Result<Option<serde_json::Value>> {
        self.mobile_init();
        match words.get(1).copied() {
            Some("edit") => self.mobile_open_editor(),
            Some("done") => {
                self.mobile.controls.editor = None;
                self.mobile.controls.settings.save()?;
            }
            Some("reset") => {
                self.mobile.controls.settings = Default::default();
                self.mobile.controls.settings.save()?;
            }
            None => {}
            Some(other) => return Err(format!("unknown touch-layout action: {other}").into()),
        }
        self.settings.touch = self.mobile.controls.settings.clone();
        Ok(Some(
            serde_json::json!({"ok":true,"editing":self.mobile.controls.editor.is_some(),"layout":self.mobile.controls.settings}),
        ))
    }

    fn android_show_keyboard(&mut self) {
        if let Some(ed) = &self.mobile.controls.editor {
            mobile_platform::show(if ed.editing_save_desc {
                &ed.save_desc_input
            } else {
                &ed.save_name_input
            });
        } else if let Some(dlg) = &self.mobile.ui.preset_dialog {
            mobile_platform::show(if dlg.editing_desc {
                &dlg.desc_input
            } else {
                &dlg.name_input
            });
        } else {
            self.touch_request_keyboard();
        }
    }
    fn mobile_osk_output(&mut self, out: Output) {
        use horizon_client::osk::Output as O;
        match out {
            Output::Text(s) => self.pad_osk_output(O::Text(s)),
            Output::Backspace => self.pad_osk_output(O::Backspace),
            Output::Enter => self.pad_osk_output(O::Enter),
            Output::Close => self.pad_osk_output(O::Close),
            Output::Key(k) => {
                let _ = self.control_key_down(k, k);
                self.keys.remove(&k);
                self.physical_keys.remove(&k);
            }
        }
    }
    fn android_poll_keyboard(&mut self) {
        self.android_custom_events();
        self.android_sync_pointer();
        if let Some(ed) = &mut self.mobile.controls.editor
            && std::mem::take(&mut ed.keyboard_request)
        {
            self.android_show_keyboard();
        }
        for out in mobile_platform::poll() {
            let captured = if self.mobile.controls.editor.is_some() {
                match &out {
                    Output::Text(t) => self.mobile.controls.capture_custom_text(t),
                    Output::Key(k) => self.mobile.controls.capture_custom_key_code(*k),
                    Output::Backspace => self
                        .mobile
                        .controls
                        .capture_custom_key_code(KeyCode::Backspace),
                    Output::Enter => self.mobile.controls.capture_custom_key_code(KeyCode::Enter),
                    Output::Close => false,
                }
            } else {
                false
            };
            if captured {
                mobile_platform::hide();
                continue;
            }
            let token = match &out {
                Output::Text(t) => self.mobile.ui.capture_custom_input_text(t),
                Output::Key(k) => self.mobile.ui.capture_custom_input_key(*k),
                _ => None,
            };
            if let Some(token) = token {
                self.android_custom_token_tap(&token);
                mobile_platform::hide();
                continue;
            }
            if self.android_preset_typing(&out) {
                continue;
            }
            self.mobile_osk_output(out);
        }
    }

    /// A custom binding still owns this key until its last touch is released.
    pub(super) fn android_custom_key_held(&self, key: KeyCode) -> bool {
        self.mobile
            .controls
            .held_custom_tokens()
            .chain(self.mobile.late_custom_releases.iter().map(String::as_str))
            .any(|token| {
                if let Some(mapping) = token.strip_prefix("action:") {
                    horizon_client::keybinds::key_code(
                        horizon_client::keybinds::KeyBinds::default_of(mapping),
                    ) == Some(key)
                } else {
                    horizon_client::keybinds::key_code(token).is_some_and(|physical| {
                        physical == key
                            || self
                                .settings
                                .keys
                                .translate_all(physical)
                                .contains(&Ok(key))
                    })
                }
            })
    }
    pub(super) fn android_custom_mouse_held(&self, button: MouseButton) -> bool {
        self.mobile
            .controls
            .held_custom_tokens()
            .chain(self.mobile.late_custom_releases.iter().map(String::as_str))
            .any(|token| {
                if let Some(mapping) = token.strip_prefix("action:") {
                    horizon_client::keybinds::mouse_button(
                        horizon_client::keybinds::KeyBinds::default_of(mapping),
                    ) == Some(button)
                } else if horizon_client::keybinds::mouse_button(token) == Some(button) {
                    true
                } else {
                    horizon_client::keybinds::key_code(token).is_some_and(|physical| {
                        self.settings
                            .keys
                            .translate_all(physical)
                            .contains(&Err(button))
                    })
                }
            })
    }
    pub(super) fn android_custom_events(&mut self) {
        for (name, down) in self.mobile.controls.key_events() {
            self.android_custom_token_edge(&name, down);
        }
    }

    pub(super) fn android_custom_token_edge(&mut self, token: &str, down: bool) {
        let ticking = self.title.is_none()
            && self.screen == Screen::Playing
            && !self.pause
            && !self.pad_in_menu();
        if down {
            if let Some(i) = self
                .mobile
                .late_custom_releases
                .iter()
                .position(|t| t == token)
            {
                let prev = self.mobile.late_custom_releases.remove(i);
                self.android_dispatch_custom_token(&prev, false);
            }
            self.mobile
                .custom_pressed_at
                .insert(token.to_owned(), self.ticks);
        } else if ticking && self.mobile.custom_pressed_at.get(token) == Some(&self.ticks) {
            if !self.mobile.late_custom_releases.iter().any(|t| t == token) {
                self.mobile.late_custom_releases.push(token.to_owned());
            }
            return;
        }
        self.android_dispatch_custom_token(token, down);
    }

    pub(super) fn android_custom_token_tap(&mut self, token: &str) {
        self.android_custom_token_edge(token, true);
        if token != "mouse.virtual.toggle"
            && token != "mouse.scroll.up"
            && token != "mouse.scroll.down"
        {
            self.android_custom_token_edge(token, false);
        }
    }

    pub(super) fn android_dispatch_custom_token(&mut self, name: &str, down: bool) {
        self.last_input = Instant::now();
        self.pad.in_use = true;
        if down && !self.pad_in_menu() && !self.captured && self.focused {
            self.capture(true);
        }
        if self.screen == Screen::Options && self.options_waiting_for_key() {
            if !down {
                return;
            }
            if let Some(mapping) = name.strip_prefix("action:") {
                let default = horizon_client::keybinds::KeyBinds::default_of(mapping);
                if let Some(key) = horizon_client::keybinds::key_code(default) {
                    let _ = self.options_key(key, None);
                } else if let Some(button) = horizon_client::keybinds::mouse_button(default) {
                    let _ = self.options_bind_mouse(button);
                }
                return;
            }
            if let Some(button) = horizon_client::keybinds::mouse_button(name) {
                let _ = self.options_bind_mouse(button);
                return;
            }
            if let Some(key) = horizon_client::keybinds::key_code(name) {
                let _ = self.options_key(key, None);
                return;
            }
        }
        if let Some(mapping) = name.strip_prefix("action:") {
            if mapping == "key.sneak" || mapping == "key.sprint" {
                let slot = usize::from(mapping == "key.sprint");
                let key = [KeyCode::ShiftLeft, KeyCode::ControlLeft][slot];
                if down {
                    self.mobile.custom_keys.insert(key);
                    self.physical_keys.insert(key);
                    self.keys.insert(key);
                } else {
                    self.mobile.custom_keys.remove(&key);
                    self.physical_keys.remove(&key);
                    self.keys.remove(&key);
                }
                self.toggles.0[slot].set_down(down, false);
                self.sync_toggle(slot);
                if slot == 1
                    && !down
                    && let Some(player) = &mut self.player
                {
                    player.sprinting = false;
                }
                return;
            }
            if self.pad_in_menu()
                && down
                && mapping == "key.inventory"
                && matches!(self.screen, Screen::Inventory | Screen::Creative)
            {
                let _ = self.pad_back();
                return;
            }
            let default = horizon_client::keybinds::KeyBinds::default_of(mapping);
            let result = if let Some(key) = horizon_client::keybinds::key_code(default) {
                self.mapped_key(
                    key,
                    down,
                    false,
                    key,
                    horizon_client::held::Source::Keyboard,
                )
            } else if let Some(button) = horizon_client::keybinds::mouse_button(default) {
                self.android_custom_mouse(button, down)
            } else {
                Ok(())
            };
            if let Err(error) = result {
                log::warn!("Custom touch action: {error}");
            }
            if down && self.screen == Screen::Chat {
                self.android_show_keyboard();
            }
            return;
        }
        if let Some(button) = horizon_client::keybinds::mouse_button(name) {
            if let Err(error) = self.android_custom_mouse(button, down) {
                log::warn!("Custom mouse button: {error}");
            }
            return;
        }
        if name == "mouse.scroll.up" || name == "mouse.scroll.down" {
            if down {
                let delta = if name == "mouse.scroll.up" { 8.0 } else { 1.0 };
                let next = horizon_client::mouse::ScrollWheelHandler::next_selection(
                    delta,
                    self.selected as i32,
                    9,
                );
                self.selected = next as usize;
            }
            return;
        }
        if name == "mouse.look" {
            return;
        }
        if name == "mouse.virtual.toggle" {
            if down {
                if !self.mobile.controls.settings.virtual_mouse_enabled {
                    return;
                }
                if self.pad_in_menu() {
                    self.mobile.controls.settings.virtual_mouse_hidden ^= true;
                    self.mobile.ui.menu_direct_pointer =
                        self.mobile.controls.settings.virtual_mouse_hidden;
                    self.settings.touch = self.mobile.controls.settings.clone();
                    let _ = self.mobile.controls.settings.save();
                } else {
                    self.mobile.ui.goy_in_game_mouse ^= true;
                    self.mobile.ui.menu_direct_pointer = !self.mobile.ui.goy_in_game_mouse;
                }
                self.android_sync_pointer();
                self.android_touch_haptic(14);
            }
            return;
        }
        let Some(key) = horizon_client::keybinds::key_code(name) else {
            return;
        };
        if down {
            self.mobile.custom_keys.insert(key);
            self.physical_keys.insert(key);
        } else {
            self.mobile.custom_keys.remove(&key);
            self.physical_keys.remove(&key);
        }
        if down
            && horizon_client::keybinds::key_name(key)
                == Some(self.settings.keys.get("key.fullscreen"))
        {
            self.toggle_fullscreen();
            return;
        }
        if down && self.screenshot_shortcut(key, false) {
            return;
        }
        let action = if down {
            KeyAction::Press
        } else {
            KeyAction::Release
        };
        if self.debug_key(key, action) {
            return;
        }
        self.native_panel_key_event(key, down, false);
        if self.screen == Screen::Options {
            if down {
                let _ = self.options_key(key, None);
            }
            return;
        }
        // Raw Shift and Control are the touch Sneak and Sprint buttons:
        // set their toggle state directly so both Hold and Toggle modes work reliably,
        // and also update `self.keys` so inventory Shift-click works.
        if matches!(
            key,
            KeyCode::ShiftLeft | KeyCode::ShiftRight | KeyCode::ControlLeft | KeyCode::ControlRight
        ) {
            if down {
                self.keys.insert(key);
            } else {
                self.keys.remove(&key);
            }
            let slot = usize::from(matches!(key, KeyCode::ControlLeft | KeyCode::ControlRight));
            self.toggles.0[slot].set_down(down, false);
            self.sync_toggle(slot);
            if slot == 1
                && !down
                && let Some(player) = &mut self.player
            {
                player.sprinting = false;
            }
            for mapped in self.settings.keys.translate_all(key) {
                match mapped {
                    Ok(code)
                        if !matches!(
                            code,
                            KeyCode::ShiftLeft
                                | KeyCode::ShiftRight
                                | KeyCode::ControlLeft
                                | KeyCode::ControlRight
                        ) =>
                    {
                        let _ = self.mapped_key(
                            code,
                            down,
                            false,
                            code,
                            horizon_client::held::Source::Keyboard,
                        );
                    }
                    Err(button) => {
                        let _ = self.android_custom_mouse(button, down);
                    }
                    _ => {}
                }
            }
            return;
        }
        if self.pad_in_menu() && down {
            if key == KeyCode::Escape {
                let _ = self.pad_back();
                return;
            }
            if key == KeyCode::KeyE && matches!(self.screen, Screen::Inventory | Screen::Creative) {
                let _ = self.pad_back();
                return;
            }
            if self.screen == Screen::Inventory
                && (self.merchant_key(key) || self.recipe_book_key(key))
            {
                return;
            }
            if self.screen == Screen::Chat && self.title.is_none() {
                self.keys.insert(key);
                self.chat_key_code(key, None);
                return;
            }
            if self.title.is_some() {
                let (w, h) = self.gui_size();
                let shift = self.has_shift_down();
                if let Some(t) = self.title.as_mut() {
                    match key {
                        KeyCode::Escape => {
                            t.escape();
                            self.save_title_options();
                        }
                        KeyCode::F5 => t.refresh_servers(),
                        KeyCode::Tab => t.tab_dir(shift, (w, h)),
                        KeyCode::ArrowLeft
                        | KeyCode::ArrowRight
                        | KeyCode::ArrowUp
                        | KeyCode::ArrowDown => t.arrow_key(key, (w, h), false),
                        KeyCode::Backspace => self.title_edit(None, true),
                        _ => {}
                    }
                }
                return;
            }
        }
        if !down && self.screen == Screen::Chat && self.title.is_none() {
            self.keys.remove(&key);
        }
        if self.screen == Screen::Playing
            && down
            && horizon_client::keybinds::key_name(key)
                .is_some_and(|n| self.settings.keys.get("key.smoothCamera") == n)
        {
            self.smooth_camera = !self.smooth_camera;
        }
        let translated = self.settings.keys.translate_all(key);
        if translated.is_empty() {
            if down && matches!(self.screen, Screen::Inventory | Screen::Creative) {
                let _ = self.key_pressed(key, key);
            }
        } else {
            for mapped in translated {
                let result = match mapped {
                    Ok(code) => self.mapped_key(
                        code,
                        down,
                        false,
                        code,
                        horizon_client::held::Source::Keyboard,
                    ),
                    Err(button) => self.android_custom_mouse(button, down),
                };
                if let Err(error) = result {
                    log::warn!("Custom touch key: {error}");
                }
            }
        }
        if down && matches!(key, KeyCode::KeyT | KeyCode::Slash) && self.screen == Screen::Chat {
            self.android_show_keyboard();
        }
    }

    pub(super) fn android_touch_haptic(&self, millis: u32) {
        if self.mobile.controls.settings.ui_vibrate {
            mobile_platform::vibrate(horizon_client::gamepad::Pulse {
                strong: 0.30,
                weak: 0.55,
                millis,
            });
        }
    }

    pub(super) fn android_set_virtual_mouse(&mut self, enabled: bool) {
        if !enabled {
            self.mobile.ui.cancel_relative();
            for right in self.mobile.ui.release_all_mouse() {
                let button = if right {
                    MouseButton::Right
                } else {
                    MouseButton::Left
                };
                let _ = self.android_custom_mouse(button, false);
            }
            self.mobile.ui.virtual_mouse = false;
            return;
        }
        self.mobile.ui.virtual_mouse = true;
        if let Some(b) = self.gui_scale() {
            self.mobile
                .ui
                .sync_viewport_cursor(&mut self.cursor, [b.width, b.height]);
        }
    }

    /// State-driven Virtual Mouse visibility synchronization across MCPE and GoyControl.
    pub(super) fn android_sync_pointer(&mut self) {
        self.mobile.ui.pointer_off = !self.mobile.controls.settings.virtual_mouse_enabled;
        let in_menu = self.pad_in_menu();
        if let Some(b) = self.gui_scale() {
            self.mobile
                .ui
                .sync_viewport_cursor(&mut self.cursor, [b.width, b.height]);
        }
        let want = self.mobile.ui.compute_virtual_mouse_visibility(
            &self.mobile.controls,
            in_menu,
            self.ui.hide_gui,
            self.touch_shown(),
            self.captured,
        );
        if self.mobile.ui.virtual_mouse != want {
            self.android_set_virtual_mouse(want);
        }
    }

    fn android_custom_mouse(&mut self, button: MouseButton, down: bool) -> Result<()> {
        if self.screen == Screen::Options && self.options_waiting_for_key() {
            if down {
                return self.options_bind_mouse(button);
            }
            return Ok(());
        }
        if self.pad_in_menu() && !matches!(self.screen, Screen::Inventory | Screen::Creative) {
            let right = button == MouseButton::Right;
            if matches!(button, MouseButton::Left | MouseButton::Right) {
                let id = self.window.as_ref().map_or(WindowId::dummy(), |w| w.id());
                return self.android_menu_mouse(&SyntheticLoop, id, right, down);
            }
            return Ok(());
        }
        if !matches!(self.screen, Screen::Inventory | Screen::Creative) {
            if down && !self.captured && self.focused {
                self.capture(true);
            }
            self.extras_mouse(button, down);
            self.native_mouse_button_post(
                match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Back => 3,
                    MouseButton::Forward => 4,
                    MouseButton::Other(i) => i as i32,
                },
                down,
                0,
            );
            if down
                && self.settings.keys.get("key.smoothCamera")
                    == horizon_client::keybinds::mouse_name(button)
            {
                self.smooth_camera = !self.smooth_camera;
            }
            let mapped = self.settings.keys.translate_mouse_all(button);
            if mapped.is_empty() {
                return self.play_mouse(button, down);
            }
            for item in mapped {
                match item {
                    Err(b) => self.play_mouse(b, down)?,
                    Ok(k) => {
                        self.mapped_key(k, down, false, k, horizon_client::held::Source::Keyboard)?
                    }
                }
            }
            return Ok(());
        }
        if let Some(key) = self
            .settings
            .keys
            .container_mouse_key(button, self.creative)
        {
            if down && self.carried.is_none() {
                self.inventory_hotkey(key);
            }
            return Ok(());
        }
        let right = button == MouseButton::Right;
        if !matches!(button, MouseButton::Left | MouseButton::Right) {
            if self.settings.keys.get("key.pickItem")
                == horizon_client::keybinds::mouse_name(button)
            {
                if down {
                    self.inventory_pick_press();
                } else {
                    self.inventory_pick_release();
                }
                if let Some(container) = self.container {
                    self.container_changed(container);
                }
            }
            return Ok(());
        }
        if down {
            if self.screen == Screen::Creative {
                self.click_creative(right);
            } else {
                self.inventory_press(right);
            }
            if let Some(container) = self.container {
                self.container_changed(container);
            }
        } else {
            self.inventory_mouse_release(right);
            if self.screen == Screen::Creative && !right {
                self.creative_screen.release();
            }
            self.station_release();
            self.recipe_book_release();
            self.loom_release();
        }
        Ok(())
    }

    fn android_menu_mouse(
        &mut self,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
        right: bool,
        down: bool,
    ) -> Result<()> {
        let button = if right {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
            self.android_custom_mouse(button, down)
        } else {
            let id = if id == WindowId::dummy() {
                self.window.as_ref().map_or(WindowId::dummy(), |w| w.id())
            } else {
                id
            };
            self.pad_synthetic_event(
                events,
                id,
                WindowEvent::MouseInput {
                    device_id: DeviceId::dummy(),
                    button,
                    state: if down {
                        ElementState::Pressed
                    } else {
                        ElementState::Released
                    },
                },
            );
            if !right && !down {
                self.touch_field_click();
                if self.on_text_field() == Some(true) {
                    self.android_show_keyboard();
                }
            }
            Ok(())
        }
    }
    pub(super) fn android_aux_touch(
        &mut self,
        t: &Touch,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
    ) -> Result<bool> {
        self.mobile.ui.goy_mode = self.mobile.controls.settings.control_style
            == horizon_client::android_controls::ControlStyle::GoyControl;
        if t.phase == TouchPhase::Ended
            && let Some((right, last)) = self.mobile.ui.release_mouse(t.id)
        {
            if last {
                self.android_menu_mouse(events, id, right, false)?;
            }
            return Ok(true);
        }
        if t.phase == TouchPhase::Cancelled {
            self.mobile.ui.cancel();
            self.android_release_touch_keys(true);
            return Ok(true);
        }
        if t.phase == TouchPhase::Ended && self.mobile.ui.owns_key(t.id) {
            if let Some(key) = self.mobile.ui.key_up(t.id) {
                self.android_key_up(key);
            }
            return Ok(true);
        }
        let size = if let Some(r) = self.renderer.as_ref() {
            winit::dpi::PhysicalSize::new(r.config.width, r.config.height)
        } else if let Some(window) = self.window.as_ref() {
            window.inner_size()
        } else {
            return Ok(false);
        };
        let mut b = HudBatch::new(
            [size.width, size.height],
            horizon_client::hud::GuiScale {
                requested: 0,
                unicode: false,
            },
        );
        // The hub as `mobile_draw` lays it out: clear of the camera cutout's strips, beside the
        // utility keys (#166).
        let [left, _, right, _] = self.safe_gui_insets(b.scale);
        b.width = (b.width - left - right).max(0.0);
        let p = [
            t.location.x as f32 / b.scale - left,
            t.location.y as f32 / b.scale,
        ];
        if !self.mobile.ui.owns_relative(t.id)
            && !self.mobile.ui.owns_gesture(t.id)
            && let Some(menu_act) = self.mobile.ui.floating_menu_touch(
                &mut b,
                &self.gui.atlas,
                t.id,
                t.phase,
                p,
                &mut self.mobile.controls.settings,
            )
        {
            use horizon_client::android_control_ui::InGameMenuAction;
            self.mobile.controls.clear();
            self.mobile.controls.ignore(t.id);
            if t.phase == TouchPhase::Started {
                self.audio.play_button_click();
                self.android_touch_haptic(12);
            }
            match menu_act {
                InGameMenuAction::Close => {}
                InGameMenuAction::OpenControlEditor => {
                    self.mobile.controls.open_editor();
                    self.android_sync_pointer();
                }
                InGameMenuAction::SetControlStyle(style) => {
                    self.android_release_touch_keys(true);
                    self.mobile.ui.goy_in_game_mouse = false;
                    self.mobile.controls.clear();
                    self.mobile
                        .controls
                        .settings
                        .switch_control_system(std::path::Path::new("."), style);
                    self.mobile.ui.goy_mode =
                        style == horizon_client::android_controls::ControlStyle::GoyControl;
                    self.mobile.ui.refresh_presets_cache(style);
                    self.android_sync_pointer();
                }
                InGameMenuAction::SelectPreset(preset_id) => {
                    let cur_style = self.mobile.controls.settings.control_style;
                    self.mobile.controls.settings.select_preset(
                        std::path::Path::new("."),
                        cur_style,
                        &preset_id,
                    );
                    self.mobile.ui.refresh_presets_cache(cur_style);
                }
                InGameMenuAction::CopyPreset {
                    source_id,
                    name,
                    description,
                } => {
                    let cur_style = self.mobile.controls.settings.control_style;
                    if let Ok(meta) = horizon_client::android_controls::Settings::copy_preset(
                        std::path::Path::new("."),
                        cur_style,
                        &source_id,
                        &name,
                        &description,
                    ) {
                        self.mobile.controls.settings.select_preset(
                            std::path::Path::new("."),
                            cur_style,
                            &meta.id,
                        );
                        self.mobile.ui.refresh_presets_cache(cur_style);
                    }
                }
                InGameMenuAction::UpdatePresetMeta {
                    id: preset_id,
                    name,
                    description,
                } => {
                    let cur_style = self.mobile.controls.settings.control_style;
                    let _ = self.mobile.controls.settings.update_preset_meta(
                        std::path::Path::new("."),
                        cur_style,
                        &preset_id,
                        &name,
                        &description,
                    );
                    self.mobile.ui.refresh_presets_cache(cur_style);
                }
                InGameMenuAction::ApplyPreset(name) => {
                    self.mobile.controls.settings.apply_preset(name);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::LoadSavedProfile(idx) => {
                    let saved = horizon_client::android_controls::Settings::list_profiles(
                        std::path::Path::new("."),
                    );
                    if let Some(name) = saved.get(idx) {
                        self.mobile.controls.settings =
                            horizon_client::android_controls::Settings::load_profile(
                                std::path::Path::new("."),
                                name,
                            );
                    }
                }
                InGameMenuAction::PresetVisibilityAll => {
                    self.mobile.controls.settings.apply_visibility_preset_all();
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::CycleMovementMode => {
                    use horizon_client::android_controls::Mode;
                    self.mobile.controls.settings.mode = match self.mobile.controls.settings.mode {
                        Mode::Dpad => Mode::Crosshair,
                        Mode::Crosshair => Mode::Tap,
                        Mode::Tap => Mode::Dpad,
                    };
                    if self.mobile.controls.settings.mode != Mode::Crosshair {
                        self.mobile.controls.settings.split_controls = false;
                    }
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::CycleJoystickArea => {
                    self.mobile.controls.settings.joystick_style.move_area = self
                        .mobile
                        .controls
                        .settings
                        .joystick_style
                        .move_area
                        .next();
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::ToggleAnalogMovement => {
                    self.mobile.controls.settings.analog_movement ^= true;
                    self.mobile.controls.settings.save()?;
                }
                InGameMenuAction::ToggleInteractionLook => {
                    self.mobile.controls.settings.attack_button_look ^= true;
                    self.mobile.controls.settings.save()?;
                }
                InGameMenuAction::ToggleSplitTouch => {
                    self.mobile.controls.settings.toggle_split_touch();
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::ToggleInvertY => {
                    let next = !self.mobile.controls.settings.invert_y;
                    self.mobile.controls.settings.invert_y = next;
                    self.settings.invert_y = next;
                    let _ = self.mobile.controls.settings.save();
                    self.settings.save(&Self::options_path());
                }
                InGameMenuAction::ToggleLeftHanded => {
                    self.mobile.controls.settings.left_handed =
                        !self.mobile.controls.settings.left_handed;
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::ToggleSprintMode => {
                    let next = !self.mobile.controls.settings.sprint_toggle;
                    self.mobile.controls.settings.sprint_toggle = next;
                    self.mobile.controls.settings.toggle_sprint = next;
                    self.settings.toggle_sprint = next;
                    let _ = self.mobile.controls.settings.save();
                    self.settings.save(&Self::options_path());
                }
                InGameMenuAction::ToggleInventoryControls => {
                    self.mobile.controls.settings.inventory_controls =
                        !self.mobile.controls.settings.inventory_controls;
                    let _ = self.mobile.controls.settings.save();
                    self.android_sync_pointer();
                }
                InGameMenuAction::ToggleVibrate | InGameMenuAction::ToggleUiVibrate => {
                    let ui = matches!(menu_act, InGameMenuAction::ToggleUiVibrate);
                    let value = if ui {
                        &mut self.mobile.controls.settings.ui_vibrate
                    } else {
                        &mut self.mobile.controls.settings.vibrate
                    };
                    *value = !*value;
                    let next = *value;
                    let _ = self.mobile.controls.settings.save();
                    if next && ui {
                        self.android_touch_haptic(22);
                    } else {
                        mobile_platform::cancel_vibration();
                    }
                }
                InGameMenuAction::ToggleAutoJump => {
                    let next = !self.mobile.controls.settings.auto_jump;
                    self.mobile.controls.settings.auto_jump = next;
                    self.settings.auto_jump = next;
                    if let Some(player) = &mut self.player {
                        player.auto_jump_enabled = next;
                    }
                    let _ = self.mobile.controls.settings.save();
                    self.settings.save(&Self::options_path());
                }
                InGameMenuAction::ToggleKeyboardButton => {
                    self.mobile.controls.settings.keyboard_button ^= true;
                    self.mobile.controls.settings.save()?;
                }
                InGameMenuAction::ToggleOffhandButton => {
                    self.mobile.controls.settings.offhand_button ^= true;
                    self.mobile.controls.settings.save()?;
                }
                InGameMenuAction::ToggleShieldButton => {
                    self.mobile.controls.settings.shield_button ^= true;
                    self.mobile.controls.settings.save()?;
                }
                InGameMenuAction::ToggleGyro => {
                    self.mobile.controls.settings.gyroscope =
                        !self.mobile.controls.settings.gyroscope;
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::ToggleVirtualMouse => {
                    let next = !self.mobile.controls.settings.virtual_mouse_enabled;
                    self.mobile.controls.settings.virtual_mouse_enabled = next;
                    self.mobile.ui.pointer_off = !next;
                    self.mobile.ui.menu_direct_pointer = true;
                    self.mobile.controls.settings.virtual_mouse_hidden = true;
                    self.mobile.ui.goy_in_game_mouse = false;
                    let _ = self.mobile.controls.settings.save();
                    self.android_sync_pointer();
                }
                InGameMenuAction::SetVirtualMouseSpeed(spd) => {
                    self.mobile.controls.settings.virtual_mouse_sensitivity = spd.clamp(0.25, 3.00);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::StepVirtualMouseSpeed(dir) => {
                    self.mobile.controls.settings.virtual_mouse_sensitivity =
                        (self.mobile.controls.settings.virtual_mouse_sensitivity
                            + dir as f32 * 0.25)
                            .clamp(0.25, 3.00);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::SetVirtualMousePointerSize(scale) => {
                    self.mobile.controls.settings.virtual_mouse_pointer_scale =
                        scale.clamp(0.5, 2.0);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::SetGyroSensitivity(val) => {
                    self.mobile.controls.settings.gyro_sensitivity = val.clamp(0.10, 3.00);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::StepGyroSensitivity(dir) => {
                    self.mobile.controls.settings.gyro_sensitivity =
                        (self.mobile.controls.settings.gyro_sensitivity + dir as f32 * 0.20)
                            .clamp(0.20, 3.00);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::SetTouchSensitivity(sens) => {
                    self.mobile.controls.settings.sensitivity = sens.clamp(0.10, 3.00);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::StepTouchSensitivity(dir) => {
                    self.mobile.controls.settings.sensitivity =
                        (self.mobile.controls.settings.sensitivity + dir as f32 * 0.15)
                            .clamp(0.10, 3.0);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::SetButtonScale(val) => {
                    self.mobile.controls.settings.button_scale = val.clamp(0.50, 2.00);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::StepButtonScale(dir) => {
                    self.mobile.controls.settings.button_scale =
                        (self.mobile.controls.settings.button_scale + dir as f32 * 0.10)
                            .clamp(0.5, 2.0);
                    let _ = self.mobile.controls.settings.save();
                }
                InGameMenuAction::TriggerKey(key) => {
                    if !self.debug_key(key, KeyAction::Press) {
                        self.key_pressed(key, key)?;
                    }
                    self.debug_key(key, KeyAction::Release);
                    if matches!(key, KeyCode::KeyT | KeyCode::Slash) && self.screen == Screen::Chat
                    {
                        self.android_show_keyboard();
                    }
                }
                InGameMenuAction::OpenKeyboard => {
                    self.android_show_keyboard();
                }
                InGameMenuAction::ForceKill => {
                    let _ = self.mobile.controls.settings.save();
                    std::process::exit(0);
                }
                InGameMenuAction::OpenLogs => {
                    // Logs are private to the app; the SAF provider shares the Reports
                    // folder the diagnostics exporter writes into.
                    if !mobile_platform::open_documents("reports") {
                        self.system_message(horizon_client::text::Text::literal(
                            "Could not open Reports. Logs are in the game folder's logs/ directory.",
                        ));
                    }
                }
                InGameMenuAction::EditPresetField(description) => {
                    if let Some(dlg) = self.mobile.ui.preset_dialog.as_mut() {
                        dlg.editing_desc = description;
                        mobile_platform::show(if description {
                            &dlg.desc_input
                        } else {
                            &dlg.name_input
                        });
                    }
                }
                InGameMenuAction::SetResolutionScale(pct) => {
                    let clamped = pct.clamp(25, 100);
                    let changed = self.mobile.controls.settings.resolution_scale_percent != clamped
                        || self.settings.horizon.resolution_scale != clamped;
                    self.mobile.controls.settings.resolution_scale_percent = clamped;
                    self.settings.horizon.resolution_scale = clamped;
                    if let Some(r) = self.renderer.as_mut() {
                        r.set_world_post(
                            clamped as f32 / 100.0,
                            self.settings.horizon.fxaa,
                            self.settings.horizon.taa,
                            self.settings.horizon.upscaling.ratio(),
                            self.settings.horizon.upscaling_sharpness as f32 / 100.0,
                        );
                    }
                    if changed {
                        let _ = self.mobile.controls.settings.save();
                        self.settings.save(&Self::options_path());
                    }
                }
                InGameMenuAction::ToggleGameAnimations => {
                    let next = !self.mobile.controls.settings.disable_game_animations;
                    self.mobile.controls.settings.disable_game_animations = next;
                    horizon_client::android_control_theme::motion::set_game_animations_disabled(
                        next,
                    );
                    let _ = self.mobile.controls.settings.save();
                    if let Some(r) = self.renderer.as_mut() {
                        r.animated_textures = self.settings.horizon.animated_textures && !next;
                    }
                }
                InGameMenuAction::SendCustomInput(token) => {
                    self.android_sync_pointer();
                    self.android_custom_token_tap(&token);
                }
            }
            // Hub actions edit the live controls while Options edits its copy.
            // Keep that copy current before another option change applies it.
            self.settings.touch = self.mobile.controls.settings.clone();
            return Ok(true);
        }
        if t.phase == TouchPhase::Moved {
            return Ok(false);
        }
        // The utility keys as `mobile_draw` lays them out, clear of the cutout's strips.
        let [left, _, right, _] = self.safe_gui_insets(b.scale);
        if t.phase == TouchPhase::Started
            && !self.mobile.ui.owns_relative(t.id)
            && !self.mobile.ui.owns_gesture(t.id)
            && let Some(action) = self.mobile.ui.action(
                b.width - left - right,
                b.height,
                [p[0] - left, p[1]],
                self.pad_in_menu(),
            )
        {
            {
                self.mobile.controls.clear();
                self.mobile.controls.ignore(t.id);
                match action {
                    Action::LatchF3 => {
                        let was = self.mobile.ui.key_held(KeyCode::F3);
                        self.mobile.ui.f3_latched = !self.mobile.ui.f3_latched;
                        let now = self.mobile.ui.key_held(KeyCode::F3);
                        if !was && now {
                            self.physical_keys.insert(KeyCode::F3);
                            self.debug_key(KeyCode::F3, KeyAction::Press);
                        }
                        if was && !now {
                            self.android_key_up(KeyCode::F3);
                        }
                    }
                    Action::Pick => {
                        self.play_mouse(MouseButton::Middle, true)?;
                        self.play_mouse(MouseButton::Middle, false)?;
                        self.mobile.ui.expanded = false;
                    }
                    Action::DebugCombo(key) => {
                        let held = self.mobile.ui.key_held(KeyCode::F3);
                        if !held {
                            self.debug_key(KeyCode::F3, KeyAction::Press);
                        }
                        self.debug_key(key, KeyAction::Press);
                        self.debug_key(key, KeyAction::Release);
                        if !held {
                            self.debug_key(KeyCode::F3, KeyAction::Release);
                        }
                        self.mobile.ui.expanded = false;
                    }
                    Action::MouseButton(right) => {
                        if self.mobile.ui.press_mouse(t.id, right) {
                            self.android_menu_mouse(events, id, right, true)?;
                        }
                    }
                    Action::VirtualMouse => {
                        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
                            // Cancel a prior drag without clicking/dropping at
                            // the toggle button, which is outside the inventory.
                            self.inv_clicks = Default::default();
                            self.inv_drag = None;
                            self.creative_screen.release();
                            self.loom_release();
                        }
                        for right in self.mobile.ui.release_all_mouse() {
                            self.android_menu_mouse(events, id, right, false)?;
                        }
                        self.mobile.ui.cancel();
                        self.mobile.ui.menu_direct_pointer = !self.mobile.ui.menu_direct_pointer;
                        self.mobile.controls.settings.virtual_mouse_hidden =
                            self.mobile.ui.menu_direct_pointer;
                        self.settings.touch = self.mobile.controls.settings.clone();
                        self.mobile.controls.settings.save()?;
                        self.android_sync_pointer();
                    }
                    Action::More => self.mobile.ui.expanded = !self.mobile.ui.expanded,
                    Action::Keyboard => {
                        if mobile_platform::active() {
                            mobile_platform::hide();
                        } else {
                            self.android_show_keyboard();
                        }
                    }
                    Action::Key(key) => {
                        if key != KeyCode::F3 && !self.mobile.ui.key_held(KeyCode::F3) {
                            self.mobile.ui.expanded = false;
                        }
                        if key == KeyCode::Escape {
                            mobile_platform::hide();
                            self.pad_back()?;
                        } else if matches!(
                            key,
                            KeyCode::Tab | KeyCode::ShiftLeft | KeyCode::ControlLeft
                        ) {
                            let down = if key == KeyCode::Tab {
                                if self.physical_keys.remove(&key) {
                                    false
                                } else {
                                    self.physical_keys.insert(key);
                                    true
                                }
                            } else {
                                self.mobile.ui.toggle_modifier(key)
                            };
                            if !self.pad_in_menu() {
                                self.mapped_key(
                                    key,
                                    down,
                                    false,
                                    key,
                                    horizon_client::held::Source::Keyboard,
                                )?;
                            }
                        } else {
                            if !self.mobile.ui.key_down(t.id, key) {
                                return Ok(true);
                            }
                            self.physical_keys.insert(key);
                            if !self.debug_key(key, KeyAction::Press) {
                                self.key_pressed(key, key)?;
                            }
                            if matches!(key, KeyCode::KeyT | KeyCode::Slash)
                                && self.screen == Screen::Chat
                            {
                                self.android_show_keyboard();
                            }
                        }
                    }
                }
            }
            return Ok(true);
        }
        let _ = (events, id);
        Ok(false)
    }

    /// Text from the Android keyboard while a preset naming field has focus.
    /// Both naming dialogs (the editor's save-as and the hub's copy) type here
    /// instead of showing a keyboard inside the dialog.
    fn android_preset_typing(&mut self, out: &Output) -> bool {
        let handled = horizon_client::android_control_ui::apply_editor_or_preset_output(
            self.mobile.controls.editor.as_mut(),
            self.mobile.ui.preset_dialog.as_mut(),
            out,
        );
        if handled && horizon_client::android_control_ui::output_dismisses(out) {
            mobile_platform::hide();
        }
        handled
    }

    pub(super) fn android_key_up(&mut self, key: KeyCode) {
        if self.mobile.custom_keys.contains(&key) {
            return;
        }
        self.physical_keys.remove(&key);
        self.keys.remove(&key);
        self.debug_key(key, KeyAction::Release);
    }
    pub(super) fn android_release_touch_keys(&mut self, cancelled: bool) {
        self.mobile.ui.clear_modifiers();
        if cancelled && matches!(self.screen, Screen::Inventory | Screen::Creative) {
            self.inv_clicks = Default::default();
            self.inv_drag = None;
            self.creative_screen.release();
            self.loom_release();
        }
        self.mobile.ui.cancel_relative();
        for right in self.mobile.ui.release_all_mouse() {
            let button = if right {
                MouseButton::Right
            } else {
                MouseButton::Left
            };
            if let Err(error) = self.android_custom_mouse(button, false) {
                log::warn!("Release virtual mouse: {error}");
            }
        }

        if cancelled {
            self.mobile.hardware_keys.clear();
            mobile_platform::cancel_vibration();
            self.mobile.controls.clear();
            self.mobile.touch_aim = None;
        }
        self.android_custom_events();
        for key in self.mobile.ui.release_keys() {
            if cancelled && key == KeyCode::F3 {
                self.physical_keys.remove(&key);
                self.debug_keys.modifier_down = false;
                self.debug_keys.used_debug_key_as_modifier = false;
                if self.screen == Screen::GameModeSwitcher {
                    self.close_gamemode_switcher(false);
                }
            } else {
                self.android_key_up(key);
            }
        }
    }
    fn android_menu_touch_release(&mut self, phase: TouchPhase) {
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.ui.options.release_touch_focus();
            self.cursor = (-1., -1.);
        }
    }
    pub(super) fn android_menu_touch(
        &mut self,
        t: &Touch,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
    ) {
        self.android_sync_pointer();
        if self.mobile.ui.virtual_mouse {
            self.android_virtual_touch(t, events, id);
            return;
        }
        let Some(b) = self.gui_scale() else {
            return;
        };
        let p = [t.location.x as f32 / b.scale, t.location.y as f32 / b.scale];
        if t.phase == TouchPhase::Started && self.screen == Screen::Options {
            // Starling tracks list and slider presses through its window events.
        }
        let track = self.on_scrollbar([t.location.x as f32, t.location.y as f32]);
        let direct = self.mobile.ui.direct_pointer(t.id, p, t.phase, track);
        self.handle_window_event(
            events,
            id,
            WindowEvent::CursorMoved {
                device_id: t.device_id,
                position: t.location,
            },
            InputOrigin::User,
        );
        if direct {
            if t.phase != TouchPhase::Moved {
                self.handle_window_event(
                    events,
                    id,
                    WindowEvent::MouseInput {
                        device_id: t.device_id,
                        button: MouseButton::Left,
                        state: if t.phase == TouchPhase::Started {
                            ElementState::Pressed
                        } else {
                            ElementState::Released
                        },
                    },
                    InputOrigin::User,
                );
            }
            self.keyboard_after_tap(t, [t.location.x as f32, t.location.y as f32]);
            self.android_menu_touch_release(t.phase);
            return;
        }
        let gesture =
            self.mobile
                .ui
                .gesture(t.id, [t.location.x as f32, t.location.y as f32], t.phase);
        // Gesture deltas are physical pixels / 30; lists scroll 18 GUI pixels
        // per wheel step. Convert once, independent of display GUI scale.
        let gesture = match gesture {
            Gesture::Scroll(y) => Gesture::Scroll(y * 30. / (b.scale * 18.)),
            other => other,
        };
        match gesture {
            Gesture::Tap => {
                let field = mobile_platform::field_at(p);
                for state in [ElementState::Pressed, ElementState::Released] {
                    self.handle_window_event(
                        events,
                        id,
                        WindowEvent::MouseInput {
                            device_id: t.device_id,
                            button: MouseButton::Left,
                            state,
                        },
                        InputOrigin::User,
                    );
                }
                self.keyboard_after_tap(t, [t.location.x as f32, t.location.y as f32]);
                if field || self.sign_edit.is_some() {
                    self.android_show_keyboard();
                }
            }
            Gesture::Scroll(y) if self.screen == Screen::Options => {
                self.ui
                    .options
                    .scroll_at(&self.settings, (b.width, b.height), (p[0], p[1]), y);
            }
            Gesture::Scroll(y) => self.handle_window_event(
                events,
                id,
                WindowEvent::MouseWheel {
                    device_id: t.device_id,
                    delta: MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(
                        0.,
                        f64::from(y * 30. / self.settings.mouse_wheel_sensitivity.max(0.01)),
                    )),
                    phase: TouchPhase::Moved,
                },
                InputOrigin::User,
            ),
            Gesture::None => {}
        }
        self.android_menu_touch_release(t.phase);
    }
    pub(super) fn android_virtual_touch(
        &mut self,
        t: &Touch,
        events: &dyn super::event_loop::EventLoopCtl,
        id: WindowId,
    ) {
        let Some(b) = self.gui_scale() else {
            return;
        };
        let position = self.mobile.ui.relative_touch(
            t.id,
            [t.location.x as f32 / b.scale, t.location.y as f32 / b.scale],
            t.phase,
            [self.cursor.0, self.cursor.1],
            [b.width, b.height],
            self.mobile.controls.settings.virtual_mouse_sensitivity,
            !self.mobile.ui.mouse_held(),
        );
        self.cursor = (position[0], position[1]);
        self.handle_window_event(
            events,
            id,
            WindowEvent::CursorMoved {
                device_id: t.device_id,
                position: winit::dpi::PhysicalPosition::new(
                    f64::from(position[0] * b.scale),
                    f64::from(position[1] * b.scale),
                ),
            },
            InputOrigin::User,
        );
        if t.phase == TouchPhase::Ended {
            self.android_virtual_mouse_clicks(events);
        }
    }
    pub(super) fn android_virtual_mouse_clicks(
        &mut self,
        events: &dyn super::event_loop::EventLoopCtl,
    ) {
        if !self.focused || !self.mobile.ui.virtual_mouse {
            self.mobile.ui.cancel_relative();
            for right in self.mobile.ui.release_all_mouse() {
                if let Some(id) = self.window.as_ref().map(|w| w.id())
                    && let Err(error) = self.android_menu_mouse(events, id, right, false)
                {
                    log::warn!("Release virtual mouse: {error}");
                }
            }
            return;
        }
        if !self.pad_in_menu() {
            return;
        }
        let Some(right) = self.mobile.ui.relative_click(true) else {
            return;
        };
        let button = if right {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
            if let Err(e) = self
                .android_custom_mouse(button, true)
                .and_then(|()| self.android_custom_mouse(button, false))
            {
                log::warn!("Virtual mouse click: {e}");
            }
        } else if let Some(id) = self.window.as_ref().map(|w| w.id()) {
            for state in [ElementState::Pressed, ElementState::Released] {
                self.handle_window_event(
                    events,
                    id,
                    WindowEvent::MouseInput {
                        device_id: DeviceId::dummy(),
                        state,
                        button,
                    },
                    InputOrigin::User,
                );
            }
        }
        if !right {
            self.touch_field_click();
            if self.on_text_field() == Some(true) {
                self.android_show_keyboard();
            }
        }
    }
}

struct SyntheticLoop;
impl super::event_loop::EventLoopCtl for SyntheticLoop {
    fn exit(&self) {}
    fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
}

impl Game<'_> {
    pub(super) fn touch_entity_accepts_use(&mut self) -> bool {
        if [
            self.boat_hit().map(|h| h.1),
            self.cart_hit().map(|h| h.1),
            self.armor_stand_hit().map(|h| h.1),
            self.frame_hit().map(|h| h.1),
        ]
        .into_iter()
        .flatten()
        .any(|distance| self.before_block(distance))
        {
            return true;
        }
        self.mob_hit()
            .filter(|(_, distance, _)| self.before_block(*distance))
            .is_some_and(|(i, _, _)| {
                self.remote_mob_would_take(i, horizon_client::first_person::Hand::Main)
                    || self.remote_mob_would_take(i, horizon_client::first_person::Hand::Off)
            })
    }
    pub(super) fn touch_context_use(&mut self) -> bool {
        if self.touch_entity_accepts_use()
            || [
                self.boat_hit().map(|h| h.1),
                self.cart_hit().map(|h| h.1),
                self.armor_stand_hit().map(|h| h.1),
                self.frame_hit().map(|h| h.1),
            ]
            .into_iter()
            .flatten()
            .any(|d| self.before_block(d))
        {
            return true;
        }
        let Some(hit) = self.pick() else {
            return false;
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let b = self.res.behaviours();
        let data = b.data(state);
        let p = b.props;
        let table = &b.table;
        use horizon_world::level::classes::Class;
        data.chain.iter().any(|c| match c {
            Class::Door | Class::TrapDoor => {
                data.set_type.as_ref().is_some_and(|s| s.can_open_by_hand)
            }
            Class::Cake | Class::CandleCake => {
                self.creative
                    || self
                        .player
                        .as_ref()
                        .is_some_and(|p| p.health.food.food < 20)
            }
            Class::Composter => table.int(state, p.level) == 8,
            Class::CaveVines | Class::CaveVinesPlant => table.bool(state, p.berries),
            Class::SweetBerryBush => table.int(state, p.age) > 1,
            Class::Repeater | Class::Comparator | Class::DaylightDetector | Class::RedstoneWire => {
                self.mode != horizon_client::command::args::GameMode::Adventure
            }
            Class::FenceGate
            | Class::Button
            | Class::Lever
            | Class::Note
            | Class::Bell
            | Class::FlowerPot
            | Class::DragonEgg
            | Class::AbstractBed
            | Class::RespawnAnchor
            | Class::AbstractFurnace
            | Class::Anvil
            | Class::Barrel
            | Class::Beacon
            | Class::BrewingStand
            | Class::CartographyTable
            | Class::Chest
            | Class::Crafter
            | Class::CraftingTable
            | Class::Dispenser
            | Class::EnchantingTable
            | Class::EnderChest
            | Class::Grindstone
            | Class::Hopper
            | Class::Loom
            | Class::ShulkerBox
            | Class::Sign
            | Class::SmithingTable
            | Class::Stonecutter
            | Class::Lectern => true,
            _ => false,
        })
    }
}
