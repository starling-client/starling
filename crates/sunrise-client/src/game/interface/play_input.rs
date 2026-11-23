//! Key and mouse mappings in play: toggled sneak and sprint, keys after rebinding, keys held
//! across screens, the hotbar's wheel and bumper steps, and mouse buttons while playing.
use super::*;

fn studio_replay_shortcut(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::Escape
            | KeyCode::KeyP
            | KeyCode::KeyF
            | KeyCode::KeyT
            | KeyCode::KeyM
            | KeyCode::Home
            | KeyCode::KeyL
            | KeyCode::KeyK
            | KeyCode::Delete
    )
}

fn studio_freecam_movement_key(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::KeyW
            | KeyCode::KeyA
            | KeyCode::KeyS
            | KeyCode::KeyD
            | KeyCode::KeyE
            | KeyCode::KeyQ
            | KeyCode::Space
            | KeyCode::ControlRight
    )
}

impl Game<'_> {
    pub(super) fn toggle_options(&self) -> [bool; 4] {
        [
            self.settings.toggle_crouch,
            self.settings.toggle_sprint,
            self.settings.toggle_attack,
            self.settings.toggle_use,
        ]
    }
    pub(super) fn reset_toggle_keys(&mut self) {
        self.vanilla_mappings.reset_toggle_keys();
    }

    pub(super) fn smooth_camera_action(&mut self) {
        self.smooth_camera = !self.smooth_camera;
    }

    pub(super) fn held_mapping(&self, action: sunrise_client::actions::Action) -> bool {
        self.mapped_key_down(action.name())
    }

    pub(super) fn press_device_key(&mut self, key: KeyCode, source: Source) -> bool {
        let edge = self.keys.press(key, source);
        self.sync_device_key(key, true);
        edge
    }

    pub(super) fn release_device_key(&mut self, key: &KeyCode, source: Source) -> bool {
        let up = self.keys.release(key, source);
        self.sync_device_key(*key, !up);
        up
    }

    pub(super) fn insert_device_key(&mut self, key: KeyCode) -> bool {
        let edge = self.keys.insert(key);
        self.sync_device_key(key, true);
        edge
    }

    pub(super) fn remove_device_key(&mut self, key: &KeyCode) -> bool {
        let held = self.keys.remove(key);
        self.sync_device_key(*key, false);
        held
    }

    pub(super) fn clear_device_keys(&mut self) {
        self.keys.clear();
        self.vanilla_mappings.set_keyboard_movement_up();
    }

    fn sync_device_key(&mut self, key: KeyCode, down: bool) {
        if let Some(slot) = sunrise_client::keybinds::VanillaMappingSlot::by_default_key(key) {
            self.vanilla_mappings.state_mut(slot).is_down = down;
        }
    }

    pub(super) fn update_modifiers(&mut self, state: winit::keyboard::ModifiersState) {
        self.modifiers = state;
        if let Some(title) = &mut self.title {
            title.set_alt_down(state.alt_key());
        }
        let released = sunrise_client_input::modifiers::reconcile(
            self.modifiers,
            &mut self.physical_keys,
            &self.settings.keys,
            &mut self.vanilla_mappings,
            [self.settings.toggle_crouch, self.settings.toggle_sprint],
            [
                self.pad.holds_action("key.sneak"),
                self.pad.holds_action("key.sprint"),
            ],
        );
        for (slot, key) in [(SNEAK, KeyCode::ShiftLeft), (SPRINT, KeyCode::ControlLeft)] {
            if released[slot] {
                self.keys.resume_device_input(&key);
            }
        }
    }
    /// Let a held sneak, sprint or modifier key go when the keyboard reports
    /// its modifier up. A release does not always reach the mapping: a screen
    /// reads presses only, and an unfocused window is given no release at all,
    /// so the latch would keep the player sneaking or sprinting until the key
    /// is tapped again. The modifier state is what the keyboard itself
    /// asserts, so it settles the question. A mapping that toggles, or one a
    /// pad latched, keeps its state: `set_down(false, needs_toggle)` leaves
    /// those alone, and a toggle the player asked for is meant to outlive the
    /// key. A mapping rebound away from its modifier is left alone too: the
    /// modifier says nothing about it, and so is one a controller holds.
    ///
    /// Only `physical_keys` is cleared directly here; the logical `self.keys`
    /// entries for sneak and sprint are left to the toggle loop below, because
    /// they stand for the mapping and not the physical Shift or Control.
    /// `Screen.hasShiftDown` and `hasControlDown` read the physical state
    /// (vanilla's `InputConstants.isKeyDown`), so a Shift no one is holding
    /// makes every inventory click a shift-click and every throw a whole-stack
    /// drop. Those do not care what sneak and sprint are bound to. A modifier
    /// says nothing about which side is down, so one being up means both
    /// Shifts, or both Controls, are.
    pub(super) fn heal_hold_latches(&mut self) {
        self.update_modifiers(self.modifiers);
    }
    /// Whether the other device still holds the sneak or sprint mapping `source` released.
    pub(super) fn toggle_held_elsewhere(&self, slot: usize, key: KeyCode, source: Source) -> bool {
        match source {
            // A pad's toggle-sneak isn't holding it: it has no release that would end it.
            Source::Keyboard => {
                !(slot == SNEAK && self.settings.controller.toggle_sneak)
                    && self.pad.holds_action(["key.sneak", "key.sprint"][slot])
            }
            Source::Pad => self.physical_keys.iter().any(|physical| {
                self.settings
                    .keys
                    .translate_all(*physical)
                    .contains(&Ok(key))
            }),
            Source::Agent => false,
        }
    }
    /// Apply a physical key to every mapping bound to it.
    pub(super) fn mapped_key_event(
        &mut self,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
    ) -> Result<()> {
        let action = match (pressed, repeat) {
            (false, _) => KeyAction::Release,
            (_, false) => KeyAction::Press,
            (_, true) => KeyAction::Repeat,
        };
        if let Some(name) = sunrise_client::keybinds::key_name(key) {
            let screen_open = self.mod_mappings_screen_open() || self.replay.playing();
            self.vanilla_mappings
                .key_event(&self.settings.keys, name, action, screen_open);
        }
        for mapped in self.settings.keys.translate_all(key) {
            match mapped {
                Ok(k) => {
                    self.mapped_key(k, pressed, repeat, key, Source::Keyboard)?;
                }
                Err(button) => {
                    if let Some(pressed) = keyboard_button(
                        gameplay_has_keyboard(
                            self.screen,
                            self.title.is_some(),
                            self.screen_over_playing(),
                        ),
                        action,
                    ) {
                        if button != MouseButton::Middle {
                            self.mouse_mapping(button, pressed, Source::Keyboard)?;
                        }
                    } else if container_key_clones(button, action) {
                        self.inventory_pick_key();
                    }
                }
            }
        }
        Ok(())
    }
    /// Apply a physical mouse button to every mapping bound to it.
    pub(super) fn mapped_mouse_event(
        &mut self,
        button: MouseButton,
        input: &str,
        pressed: bool,
    ) -> Result<()> {
        let screen_open = self.mod_mappings_screen_open();
        self.vanilla_mappings
            .mouse_event(&self.settings.keys, input, pressed, screen_open);
        for mapped in self.settings.keys.translate_mouse_all(button) {
            match mapped {
                Err(MouseButton::Middle) => {}
                Err(b) => self.mouse_mapping(b, pressed, Source::Keyboard)?,
                Ok(k) => self.mapped_key(k, pressed, false, k, Source::Keyboard)?,
            }
        }
        Ok(())
    }
    /// A key event after rebinding, as the default key of its mapping (the
    /// keyboard and controllers both end here). `raw` is the physical key
    /// the event started from (`key` where the caller started from no window
    /// key): the screens act in that keyspace (`Screen.keyPressed`), where
    /// `key`, the vanilla default the mapping translated to, may itself be
    /// another mapping's rebound key. `source` is the device the event came
    /// from: a key stays held while any device holds it.
    /// Starling: a key from the touch controls, the on-screen keyboard or a custom binding. These
    /// come with no window event, which queues a tick mapping's click by its slot
    /// (`mapped_key_event`), so the click of the key's mapping (drop, swap hands, inventory, the
    /// hotbar...) is queued here, as `key_pressed` did for them before the sync with Sunrise.
    pub(in crate::game) fn mobile_mapped_key(
        &mut self,
        key: KeyCode,
        down: bool,
        raw: KeyCode,
    ) -> Result<()> {
        self.mapped_key(key, down, false, raw, Source::Keyboard)?;
        if down
            && !self.mod_mappings_screen_open()
            && !self.replay.playing()
            && let Some(slot) = sunrise_client::keybinds::VanillaMappingSlot::by_default_key(key)
            && Self::tick_mapping(slot.definition().0)
        {
            self.vanilla_mappings.click(slot);
        }
        Ok(())
    }

    pub(super) fn mapped_key(
        &mut self,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        raw: KeyCode,
        source: Source,
    ) -> Result<()> {
        if self.mobile_holds_key(key, pressed) {
            return Ok(());
        }
        if self.screen == Screen::Playing && self.replay.playing() {
            if !pressed {
                self.release_device_key(&key, source);
                return Ok(());
            }
            if studio_replay_shortcut(key) {
                if !repeat {
                    self.key_pressed(key, raw)?;
                }
                return Ok(());
            }
            if !studio_freecam_movement_key(key) || !self.studio.flying() {
                return Ok(());
            }
        }
        // Sneak and sprint are ToggleKeyMappings: a press (or a key
        // repeat) flips them when toggling; only releases reach them
        // behind a screen (KeyboardHandler.keyPress).
        let toggle = match key {
            KeyCode::ShiftLeft => Some((0, self.settings.toggle_crouch)),
            KeyCode::ControlLeft => Some((1, self.settings.toggle_sprint)),
            _ => None,
        };
        if let Some((slot, option)) = toggle {
            let action = match (pressed, repeat) {
                (false, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            self.keys.resume_device_input(&key);
            // One device letting go doesn't let go for the other still holding it.
            if !pressed && self.toggle_held_elsewhere(slot, key, source) {
                return Ok(());
            }
            let screen_open = !gameplay_has_keyboard(
                self.screen,
                self.title.is_some(),
                self.screen_over_playing(),
            );
            self.vanilla_mappings
                .toggle_state_mut(slot)
                .key(action, screen_open, option);
            if !pressed {
                return Ok(());
            }
        } else if !pressed {
            self.release_device_key(&key, source);
            return Ok(());
        } else if !self.mod_mappings_screen_open() || self.replay.playing() {
            self.press_device_key(key, source);
            if let Some(slot) = sunrise_client::keybinds::VanillaMappingSlot::by_default_key(key)
                && slot.definition().2 == "movement"
            {
                self.vanilla_mappings.click(slot);
            }
        }
        // The window events already queued their mapping clicks by resolved slot. Screen
        // keys still see the immediately delivered input; controller/socket edges queue here.
        if source == Source::Keyboard {
            self.immediate_key_pressed(key, raw)
        } else if !repeat || key == KeyCode::KeyQ {
            self.key_pressed(key, raw)
        } else {
            Ok(())
        }
    }
    /// `KeyMapping.releaseAll` when a screen opens and
    /// `restoreToggleStatesOnScreenClosed` when it closes.
    pub(super) fn track_toggle_screens(&mut self) {
        let playing = !self.mod_mappings_screen_open();
        if playing == self.was_playing {
            return;
        }
        self.was_playing = playing;
        if playing {
            let options = self.toggle_options();
            self.vanilla_mappings
                .restore_toggles_on_screen_closed(options);
        } else {
            self.release_screen_mappings();
        }
    }
    /// Immediately release gameplay mappings when a screen opens, even if it closes
    /// again before the redraw's screen tracking.
    pub(super) fn release_screen_mappings(&mut self) {
        self.was_playing = false;
        self.keys.clear();
        let options = self.toggle_options();
        self.vanilla_mappings.release_all(options);
    }
    /// `MouseHandler.onScroll`: the hotbar (or the spectator menu) takes the wheel only when no
    /// screen is open; a book, lectern, sign or prompt drawn over gameplay is one.
    pub(super) fn hotbar_takes_wheel(&self) -> bool {
        self.screen == Screen::Playing
            && !self.screen_over_playing()
            && self.sleeping.is_none()
            && self.player.is_some()
    }
    /// A mod's `MouseScrolling` cancels the step about to turn the hotbar.
    pub(super) fn hotbar_scroll_canceled(&mut self, scroll: [f64; 2], steps: [i32; 2]) -> bool {
        let mut canceled = false;
        self.native_mouse_scrolling(
            scroll,
            steps,
            [
                self.physical_buttons.contains(&MouseButton::Left),
                self.physical_buttons.contains(&MouseButton::Middle),
                self.physical_buttons.contains(&MouseButton::Right),
            ],
            [self.cursor.0, self.cursor.1],
            glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    /// What one whole wheel step does in play (`MouseHandler.onScroll`): a spectator's menu or
    /// flying speed, else the selected hotbar slot.
    pub(super) fn hotbar_wheel_step(&mut self, wheel: i32, vertical_steps: i32) {
        if self.spectator() && self.spectator_gui.is_menu_active() {
            self.spectator_scroll(-wheel);
        } else if self.spectator() {
            if let Some(player) = &mut self.player {
                player.flying_speed = sunrise_client::mouse::spectator_flying_speed(
                    player.flying_speed,
                    vertical_steps,
                );
            }
        } else {
            self.selected = sunrise_client::mouse::ScrollWheelHandler::next_selection(
                f64::from(wheel),
                self.selected as i32,
                9,
            ) as usize;
        }
    }
    /// A step of the hotbar from a controller's bumper: one wheel notch, down for the next slot,
    /// through everything the wheel goes through.
    pub(super) fn hotbar_step(&mut self, next: bool) {
        let wheel = if next { -1 } else { 1 };
        if self.hotbar_takes_wheel()
            && !self.hotbar_scroll_canceled([0.0, f64::from(wheel)], [0, wheel])
        {
            self.hotbar_wheel_step(wheel, wheel);
        }
    }
    /// A mouse mapping pressed or let go by `source`. A release waits while another device holds
    /// the mapping, so letting go of one can't end what the other still presses, as for keys
    /// (`HeldKeys`).
    pub(super) fn mouse_mapping(
        &mut self,
        button: MouseButton,
        pressed: bool,
        source: Source,
    ) -> Result<()> {
        if self.mod_mappings_screen_open() {
            return Ok(());
        }
        if !pressed && self.mouse_mapping_held_elsewhere(button, source) {
            // The other device's hold takes over again, even after a direct setDown(false).
            // In toggle mode a release does not change the mapping.
            let toggled = match button {
                MouseButton::Left => self.settings.toggle_attack,
                MouseButton::Right => self.settings.toggle_use,
                _ => return Ok(()),
            };
            if !toggled {
                self.set_mapping_held(
                    if button == MouseButton::Left {
                        "key.attack"
                    } else {
                        "key.use"
                    },
                    true,
                );
            }
            return Ok(());
        }
        self.play_mouse(button, pressed)
    }
    /// Whether a device other than `source` holds what `button` stands for. Asks each device's own
    /// live state rather than keeping a count: a screen takes a mouse release without telling the
    /// mappings, and a count would then hold the attack for good. A script keeps no such state, so
    /// nothing waits for a script's mouse hold.
    pub(super) fn mouse_mapping_held_elsewhere(&self, button: MouseButton, source: Source) -> bool {
        let mapping = match button {
            MouseButton::Left => "key.attack",
            MouseButton::Right => "key.use",
            _ => return false,
        };
        match source {
            Source::Keyboard | Source::Agent => self.input_holds(mapping),
            Source::Pad => {
                let keys = &self.settings.keys;
                self.physical_buttons
                    .iter()
                    .any(|held| keys.translate_mouse_all(*held).contains(&Err(button)))
                    || self
                        .physical_keys
                        .iter()
                        .any(|held| keys.translate_all(*held).contains(&Err(button)))
            }
        }
    }
    /// Two mappings with one default button are separate mappings all the same
    /// (`Minecraft.handleKeybinds`): whether the middle-button action a physical `input` translated
    /// to is one of those it is bound to, for this player now.
    pub(super) fn shared_default_acts(&self, button: MouseButton, input: Option<&str>) -> bool {
        button != MouseButton::Middle
            || input.is_none_or(|input| self.settings.keys.middle_acts(input, self.spectator()))
    }
    /// A mouse button while playing (window or control socket).
    pub(super) fn play_mouse(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        if self.mobile_holds_mouse(button, pressed) {
            return Ok(());
        }
        if pressed {
            self.pvp_click(button);
        }
        if self.sleeping.is_some() {
            return Ok(());
        }
        let name = match button {
            MouseButton::Left => "key.attack",
            MouseButton::Right => "key.use",
            MouseButton::Middle if self.spectator() => "key.spectatorHotbar",
            MouseButton::Middle => "key.pickItem",
            _ => return Ok(()),
        };
        let slot =
            sunrise_client::keybinds::VanillaMappingSlot::by_name(name).expect("known mapping");
        let enabled = sunrise_client::keymapping::toggle_index(slot)
            .is_some_and(|i| self.toggle_options()[i]);
        self.vanilla_mappings
            .state_mut(slot)
            .button(pressed, false, enabled);
        Ok(())
    }
}

impl Game<'_> {
    /// Asks the mods about each gameplay mapping a physical input (a key's or a mouse button's
    /// name) is bound to, before the game acts on it: true when one cancels a press. Only while
    /// playing, and nothing is asked with no mod listening.
    pub(super) fn key_mapping_canceled(&mut self, input: Option<&str>, pressed: bool) -> bool {
        let Some(input) = input else {
            return false;
        };
        if !mod_api::__private::listened(mod_api::HookId::KeyMappingPre)
            || !gameplay_has_keyboard(self.screen, self.title.is_some(), self.dialog_open())
        {
            return false;
        }
        let names: Vec<&'static str> = self.settings.keys.translated_mappings(input).collect();
        let mut canceled = false;
        for name in names {
            let mut this = false;
            self.native_key_mapping_pre(name, pressed, &mut this);
            canceled |= this;
        }
        canceled && pressed
    }
    /// `key_mapping_canceled` for a key event, and what a cancel leaves of the key's hold: a first
    /// press that is cancelled never happened, so `restore_held_keys` must not hold it (nor its
    /// repeats); a cancelled repeat only drops its own action and keeps the hold an accepted
    /// press made.
    pub(super) fn key_press_canceled(&mut self, key: KeyCode, pressed: bool, repeat: bool) -> bool {
        if !repeat {
            self.rejected_keys.remove(&key);
        }
        let name = sunrise_client::keybinds::key_name(key);
        if !self.key_mapping_canceled(name, pressed) {
            return false;
        }
        if !repeat {
            self.rejected_keys.insert(key);
        }
        if self.rejected_keys.contains(&key) {
            self.physical_keys.remove(&key);
        }
        true
    }
}

/// The input source (docs/layers.md, "Controllers, replays and UI pages"): what
/// gameplay asks of a device beyond the keyboard and mouse, and tells it. Gameplay calls these,
/// never the controller modules; today `pad` answers.
impl Game<'_> {
    /// The analog move vector (`[left, forward]`), after direct mapping writes.
    pub(super) fn input_move(&self) -> Option<[f32; 2]> {
        self.pad_move()
            .map(|stick| self.apply_movement_writes(stick))
    }

    fn apply_movement_writes(&self, [left, forward]: [f32; 2]) -> [f32; 2] {
        let axis = |value: f32, positive, negative| {
            let component = |key, value| match self.keys.override_for(&key) {
                Some(true) => 1.0,
                Some(false) => 0.0,
                None => value,
            };
            component(positive, value.max(0.0)) - component(negative, (-value).max(0.0))
        };
        [
            axis(left, KeyCode::KeyA, KeyCode::KeyD),
            axis(forward, KeyCode::KeyW, KeyCode::KeyS),
        ]
    }

    /// KeyboardInput.tick: classify the original stick first; a direct write changes only
    /// its own flag and analog component, never a different direction's classification.
    pub(super) fn sample_movement_keys(
        &self,
        stick: Option<[f32; 2]>,
    ) -> sunrise_client::sprint::Keys {
        use sunrise_client::sprint::Keys;
        let mut keys = Keys {
            forward: self.mapped_key_down("key.forward"),
            backward: self.mapped_key_down("key.back"),
            left: self.mapped_key_down("key.left"),
            right: self.mapped_key_down("key.right"),
            jump: self.mapped_key_down("key.jump"),
            shift: self.mapped_key_down("key.sneak"),
            sprint: self.mapped_key_down("key.sprint"),
            ..Keys::default()
        }
        .with_stick(stick);
        for (down, key) in [
            (&mut keys.forward, KeyCode::KeyW),
            (&mut keys.backward, KeyCode::KeyS),
            (&mut keys.left, KeyCode::KeyA),
            (&mut keys.right, KeyCode::KeyD),
        ] {
            if let Some(value) = self.keys.override_for(&key) {
                *down = value;
            }
        }
        keys.analog = stick.map(|stick| self.apply_movement_writes(stick));
        keys
    }

    /// Whether a device holds the key mapping named `mapping` (`key.use`).
    pub(super) fn input_holds(&self, mapping: &str) -> bool {
        self.pad.holds_action(mapping)
    }

    /// Turns the view by what the devices give this frame (`Entity.turn`).
    pub(super) fn input_look(&mut self, seconds: f32) {
        self.pad_look(seconds);
    }

    /// The keyboard or mouse acted in play (a key or button pressed): it has control again, though
    /// only moving the mouse takes a pad out of use.
    pub(super) fn input_keyboard_used(&mut self) {
        self.pad.keyboard_took_over = true;
    }

    /// Whether the pointer event being handled is one a device made (a pad's menu cursor
    /// clicking), not the mouse's.
    pub(super) fn input_made_pointer_event(&self) -> bool {
        self.pad.menu.synthetic
    }

    /// A new player entity took over (spawn, respawn, a server joined, a dimension change).
    pub(super) fn input_player_replaced(&mut self) {
        self.pad_player_replaced();
    }

    /// Whether the drop about to happen is a device's whole-stack drop (it has no Control key to
    /// hold). Asking answers once.
    pub(super) fn input_takes_stack_drop(&mut self) -> bool {
        std::mem::take(&mut self.pad.drop_stack)
    }

    /// The HUD starts a frame: what a device drew on the last one is gone.
    pub(super) fn input_hud_begin(&mut self) {
        self.pad.guide_shown.clear();
    }

    /// Something happened to the player that a device may let them feel.
    pub(super) fn input_feedback(&mut self, event: sunrise_client::input_source::Feedback) {
        self.pad_rumble(event.into());
    }

    /// Whether `key` is held, a movement key by the analog move vector too: as the keys it holds
    /// (`eight_direction_keys`), whatever moves the player. For readers of the keys that aren't
    /// the movement itself.
    pub(super) fn movement_key_held(&self, key: KeyCode) -> bool {
        if let Some(down) = self.keys.override_for(&key) {
            return down;
        }
        if sunrise_client::keybinds::VanillaMappingSlot::by_default_key(key)
            .is_some_and(|slot| self.vanilla_mappings.is_down(slot))
        {
            return true;
        }
        let Some(stick) = self.pad_move() else {
            return false;
        };
        let [forward, backward, left, right] = sunrise_client::gamepad::eight_direction_keys(stick);
        match key {
            KeyCode::KeyW => forward,
            KeyCode::KeyS => backward,
            KeyCode::KeyA => left,
            KeyCode::KeyD => right,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{studio_freecam_movement_key, studio_replay_shortcut};
    use winit::keyboard::KeyCode;

    #[test]
    fn replay_shortcuts_are_distinct_from_freecam_movement() {
        for key in [
            KeyCode::Escape,
            KeyCode::KeyP,
            KeyCode::KeyF,
            KeyCode::KeyT,
            KeyCode::KeyM,
            KeyCode::Home,
            KeyCode::KeyL,
            KeyCode::KeyK,
            KeyCode::Delete,
        ] {
            assert!(studio_replay_shortcut(key));
        }
        for key in [
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::KeyE,
            KeyCode::KeyQ,
        ] {
            assert!(studio_freecam_movement_key(key));
            assert!(!studio_replay_shortcut(key));
        }
        assert!(!studio_freecam_movement_key(KeyCode::KeyI));
    }
}
