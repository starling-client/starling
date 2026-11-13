//! Starling: the native hooks and helpers of Horizon before it split them by area
//! (`game/native_hooks.rs`, Starling 0.8.0's), kept beside the new ones so that the mods made for
//! them still build and run: Horizon's native mod registry is full of them. Each old hook is
//! called where its successor is (the HUD after `render_gui_post`, the frame after
//! `render_frame_pre`, keys with the native panel's, the wheel before the hotbar's), the old
//! helpers lead to the new ones, and the old Options > Mods rows (`native_mods`,
//! `native_settings`) are shown on Horizon's page. native_mod_adapt.py points a mod's old targets
//! here; the old types are in `horizon_client::starling_mod_api`.
//!
//! Hook bodies have no `cfg` and no macros: horizon-mixins refuses a target with either.
use super::native_hooks::{NativeRecipeLayout, NativeRecipeResult, NativeRecipeView};
use super::{Game, GameLevel, LevelDimension, lan::LanHost};
use horizon_client::lan::data::component_nbt;
use horizon_client::lan::guest::Guest;
use horizon_client::{
    extras_hud::Corners,
    hud::HudBatch,
    items::ItemStack,
    mod_list::{
        Installed, KeyConflictContext, KeyMapping, NativePage, NativeSetting, NativeSettingValue,
    },
    starling_mod_api::{NativeMod, NativeStack},
    text::Text,
};
use horizon_core::position::BlockPos;
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::registry::BlockStateId;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;
use winit::event::MouseButton;

// The names the old native_hooks.rs had in scope: a mod's helper is written into the hook's body,
// so it names them as they were there (`Corner::TopLeft`, `NativeLifecycleContext`, `mod_api` for
// the old module).
#[allow(unused_imports)]
use super::Screen;
#[allow(unused_imports)]
use ::mod_api::{ServerTickPhase, WorldPhase};
#[allow(unused_imports)]
use horizon_client::starling_mod_api::{
    self as mod_api, NativeCommand, NativeDataMapType, NativeDispensed, NativeFalling,
    NativeHeightmap, NativeLifecycleContext, NativeMinimap, NativeMobRender, NativeModEvent,
    NativePanelEvent, NativePlayerRender, NativeScreenView, NativeServerPlayer, NativeShake,
    NativeZoom,
};
#[allow(unused_imports)]
use horizon_client::{
    extras::Corner, fast_hash::FxHashMap, first_person::Hand, hud::rgba, keybinds, mod_list,
};
#[allow(unused_imports)]
use horizon_core::{geometry::Direction, nbt::Compound};
#[allow(unused_imports)]
use std::{collections::HashMap, fs, path::Path};
#[allow(unused_imports)]
use winit::keyboard::KeyCode;

/// The key an old mod's button `j` has as a setting on Horizon's page.
const OLD_BUTTON: &str = "starling.old-button.";

/// The old hooks. Each does nothing in Starling.
impl Game<'_> {
    /// A real window key event after handling, including repeats and consumed shortcuts, as the
    /// window gave it (Horizon's `input_key` names the key now). Modifier bits use the screen
    /// hooks' GLFW encoding. Not cancellable.
    pub(in crate::game) fn native_input_key_old(
        &mut self,
        event: &winit::event::KeyEvent,
        modifiers: u32,
    ) {
        let _ = (event, modifiers);
    }
    /// Once a tick, in every `Game`: the window's (in single player, on LAN
    /// and on servers), and the server's own (`native_window` is false
    /// there: the single player server thread, a dedicated server).
    pub(in crate::game) fn native_tick(&mut self) {}
    /// Once a frame, in a world or not, after Horizon's extras.
    pub(in crate::game) fn native_frame(&mut self, elapsed: Duration) {
        let _ = elapsed;
    }
    /// The HUD (not drawn under F1), after the minimap: place a box with
    /// `corners.place` (boxes stack away from their edge) and draw it with
    /// `native_fill` and `native_text`. Skip a corner `native_corner_shown`
    /// says is taken.
    pub(in crate::game) fn native_hud(&mut self, batch: &mut HudBatch, corners: &mut Corners) {
        let _ = (batch, corners);
    }
    /// Before one of the HUD's layers, as `native_gui_layer_pre` is, with the game to change: Horizon's
    /// had `&mut self`, the split's has `&self`. Set `canceled` to hide the layer.
    pub(in crate::game) fn native_gui_layer_pre_old(
        &mut self,
        layer: &str,
        draw: &mut HudBatch,
        canceled: &mut bool,
    ) {
        let _ = (layer, draw, canceled);
    }
    /// After one of the HUD's layers, as `native_gui_layer_post` is, with the game to change.
    pub(in crate::game) fn native_gui_layer_post_old(&mut self, layer: &str, draw: &mut HudBatch) {
        let _ = (layer, draw);
    }
    /// A key or mouse button went down or up, by its vanilla name
    /// (`key.keyboard.z`, `key.mouse.left`): keys on any screen (check
    /// `native_playing`), mouse buttons while playing. Repeats are left out.
    pub(in crate::game) fn native_key(&mut self, key: &str, pressed: bool) {
        let _ = (key, pressed);
    }
    /// Wheel steps while playing: set `claimed` to keep them from the hotbar.
    pub(in crate::game) fn native_wheel(&mut self, steps: f32, claimed: &mut bool) {
        let _ = (steps, claimed);
    }
    /// The commands `native_command` runs, by name and without their slash:
    /// push the ones that are yours. Asked when a connection starts and
    /// again when the server sends its own tree, so the chat box colours,
    /// completes and lists them as the game's own; a name the game already
    /// has keeps that command's behaviour.
    pub(in crate::game) fn native_commands(&mut self, commands: &mut Vec<&'static str>) {
        let _ = commands;
    }
    /// A player breaks the block `state` at `pos`, once the game allowed it (after Horizon's
    /// `break_block`): set `canceled` to keep the block. Server-side, for every player of the
    /// world (the helpers read the acting one).
    pub(in crate::game) fn native_destroy_block(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        canceled: &mut bool,
    ) {
        let _ = (pos, state, canceled);
    }
    /// A player right-clicks the entity `entity` of type `kind` with `self.hand`, holding `stack`
    /// (after Horizon's `entity_interact`): set `claimed` to take the click.
    pub(in crate::game) fn native_interact_entity(
        &mut self,
        entity: i32,
        kind: &str,
        stack: Option<&ItemStack>,
        claimed: &mut bool,
    ) {
        let _ = (entity, kind, stack, claimed);
    }
    /// A player right clicks the block at `pos` with `self.hand`, `face` the face clicked, before
    /// Horizon's `right_click_block` (the old one's name, which it took with other parameters):
    /// set `canceled` to stop the click.
    pub(in crate::game) fn native_right_click_block_old(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        canceled: &mut bool,
    ) {
        let _ = (pos, face, canceled);
    }
    /// A player uses `stack` with `self.hand` on the block at `pos`, `face` the face clicked
    /// (`[0, 1, 0]` is the top), once the block itself let the click through: where
    /// `ItemStack.useOn` starts, before Horizon's hooks there (the old one's name is Horizon's
    /// `BlockBehaviour.useItemOn` now). Set `claimed` to take the use.
    pub(in crate::game) fn native_use_item_on_old(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
        claimed: &mut bool,
    ) {
        let _ = (stack, pos, face, claimed);
    }
    /// A player uses `stack` (right click) with `self.hand` in the air or where nothing took the
    /// click: set `claimed` to take the use (Horizon's `use_item` takes a call now).
    pub(in crate::game) fn native_use_item_old(&mut self, stack: &ItemStack, claimed: &mut bool) {
        let _ = (stack, claimed);
    }
    /// A play packet from the server reached the window's game, decoded, just before the game
    /// handles it: Horizon's `native_client_packet`, whose split successor shows only the
    /// packet's id and a custom payload. Read-only.
    pub(in crate::game) fn native_client_packet_old(
        &mut self,
        packet: &horizon_protocol::packets::play::Clientbound,
    ) {
        let _ = packet;
    }
    /// After a successful authoritative server level tick pass of `dimension`
    /// (`minecraft:overworld`); Horizon's `level_tick_post` names the level by a handle now.
    pub(in crate::game) fn native_level_tick_post_old(&mut self, dimension: &str) {
        let _ = dimension;
    }
    /// Options > Mods lists the native mods: push yours.
    pub(in crate::game) fn native_mods(&mut self, mods: &mut Vec<NativeMod>) {
        let _ = mods;
    }
    /// The buttons of mod `mod_id`'s settings page, under its keys: push
    /// their labels when it is yours. Asked again each time the page opens
    /// and after each click.
    pub(in crate::game) fn native_settings(&mut self, mod_id: &str, buttons: &mut Vec<String>) {
        let _ = (mod_id, buttons);
    }
    /// Button `button` of mod `mod_id`'s settings page was clicked.
    pub(in crate::game) fn native_settings_click(&mut self, mod_id: &str, button: usize) {
        let _ = (mod_id, button);
    }
}

/// Set at the end of an old hook's body: a mod's injection that left it early (its cancel) never
/// gets there. A static, not a thread-local: horizon-mixins refuses the macros of an injection in a
/// file that invokes a macro at its top.
static WENT_ON: AtomicBool = AtomicBool::new(false);

/// The end of an old hook's body.
fn went_on() {
    WENT_ON.store(true, Ordering::Relaxed);
}

/// Runs an old hook: whether it got to its end, no mod's injection having stopped it.
fn ran_through(call: impl FnOnce()) -> bool {
    WENT_ON.store(false, Ordering::Relaxed);
    call();
    WENT_ON.swap(false, Ordering::Relaxed)
}

/// The old LAN hooks, which had the host as a parameter (it is in the game now): called with the
/// host taken out for them, as it was then.
impl Game<'_> {
    /// A LAN guest's command, before the server runs it: leave early (a cancellable injection) to
    /// stop it.
    pub(in crate::game) fn guest_command_old(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        command: &str,
    ) {
        let _ = (host, conn, command);
        went_on();
    }
    /// A LAN guest's turn starts: its state is about to be swapped in.
    pub(in crate::game) fn enter_guest_old(&mut self, host: &mut LanHost, conn: u32) {
        let _ = (host, conn);
        went_on();
    }
    /// A LAN guest's vehicle ticks: leave early to stop it.
    pub(in crate::game) fn guest_vehicle_tick_old(&mut self, host: &mut LanHost, conn: u32) {
        let _ = (host, conn);
        went_on();
    }
    /// `guest_command_old` with the host out for it: false when a mod stopped the command.
    pub(in crate::game) fn old_guest_command_went_on(&mut self, conn: u32, command: &str) -> bool {
        let Some(mut host) = self.lan.take() else {
            return true;
        };
        let on = ran_through(|| self.guest_command_old(&mut host, conn, command));
        self.lan = Some(host);
        on
    }
    /// `enter_guest_old` with the host out for it.
    pub(in crate::game) fn old_enter_guest(&mut self, conn: u32) {
        if let Some(mut host) = self.lan.take() {
            self.enter_guest_old(&mut host, conn);
            self.lan = Some(host);
        }
    }
    /// `guest_vehicle_tick_old` with the host out for it: false when a mod stopped the tick.
    pub(in crate::game) fn old_guest_vehicle_tick_went_on(&mut self, conn: u32) -> bool {
        let Some(mut host) = self.lan.take() else {
            return true;
        };
        let on = ran_through(|| self.guest_vehicle_tick_old(&mut host, conn));
        self.lan = Some(host);
        on
    }
}

/// A guest of the hub as the old helpers gave a player, at `permission`.
fn old_server_player(g: &Guest, permission: u8) -> NativeServerPlayer {
    ::mod_api::__private::built::ServerPlayer {
        uuid: g.uuid,
        name: g.name.clone(),
        permission,
        dimension: g.dimension.clone(),
        position: g.position,
        game_mode: g.game_mode,
        health: g.health,
        yaw: g.yaw,
        pitch: g.pitch,
        entity_id: g.entity_id,
        address: g.address.ip().to_string(),
    }
    .into()
}

/// The command line being run, for the old `native_perform_command`: Horizon's typed commands no
/// longer pass it on, so it is kept where a line comes in (`set_command_line`).
static COMMAND_LINE: Mutex<String> = Mutex::new(String::new());

/// The command line about to be run (a slash in front or not).
pub(in crate::game) fn set_command_line(line: &str) {
    let mut kept = COMMAND_LINE.lock().unwrap_or_else(PoisonError::into_inner);
    kept.clear();
    kept.push_str(line.trim_start_matches('/'));
}

/// The old level hooks.
impl GameLevel<'_> {
    /// A piston at `piston` is about to move `push` and break `destroy` in `direction` (after
    /// Horizon's `piston_pre`): set `canceled` to stop it.
    pub(in crate::game) fn native_piston_move(
        &self,
        dimension: LevelDimension,
        piston: BlockPos,
        direction: [i32; 3],
        push: &[BlockPos],
        destroy: &[BlockPos],
        canceled: &mut bool,
    ) {
        let _ = (dimension, piston, direction, push, destroy, canceled);
    }
}

/// Where the old hooks are called from.
impl Game<'_> {
    /// A mouse button while playing, for `native_key` (Horizon's extras pass it on).
    pub(in crate::game) fn native_mouse_event(&mut self, button: MouseButton, pressed: bool) {
        let name = horizon_client::keybinds::mouse_name(button);
        self.native_key(&name, pressed);
    }
    /// Wheel steps while playing: whether a mod made for the old hooks took them.
    pub(in crate::game) fn native_wheel_event(&mut self, steps: f32) -> bool {
        let mut claimed = false;
        self.native_wheel(steps, &mut claimed);
        claimed
    }
    /// Options > Mods' rows for the mods made for the old hooks (`native_mods`), after Horizon's
    /// (`list_native_mods`): their keys, and their buttons as settings (`old_native_settings`).
    pub(in crate::game) fn list_old_native_mods(&mut self) {
        let mut mods = Vec::new();
        self.native_mods(&mut mods);
        if mods.is_empty() {
            return;
        }
        self.settings
            .mods
            .installed
            .retain(|row| !mods.iter().any(|m| m.id == row.id));
        for m in mods {
            let settings = self.old_native_settings(&m.id);
            let mappings = m
                .keys
                .iter()
                .map(|[name, label, key]| KeyMapping {
                    id: name.clone(),
                    label: label.clone(),
                    key: key.clone(),
                    modifiers: 0,
                    context: KeyConflictContext::Universal,
                    category: String::new(),
                })
                .collect();
            self.settings.mods.installed.push(Installed {
                id: m.id,
                title: m.title,
                description: m.description,
                problem: None,
                dev: false,
                native: Some(NativePage { mappings, settings }),
            });
        }
    }
    /// An old mod's buttons as settings on Horizon's page: a label ending `: ON` or `: OFF` (the
    /// usual `Shown: ON`) as a switch, any other `label: value` as that label and value, and a
    /// click on either as the old click (`old_native_setting_event`).
    fn old_native_settings(&mut self, mod_id: &str) -> Vec<NativeSetting> {
        let mut buttons = Vec::new();
        self.native_settings(mod_id, &mut buttons);
        buttons
            .into_iter()
            .enumerate()
            .map(|(j, label)| old_button(j, label))
            .collect()
    }
    /// Setting `j` of the mod in row `i` changed on the page: when it is an old mod's button, the
    /// old click, and the page's labels asked again. Whether it was one.
    pub(in crate::game) fn old_native_setting_event(&mut self, i: usize, j: usize) -> bool {
        let Some(row) = self.settings.mods.installed.get(i) else {
            return false;
        };
        let button = row
            .native
            .as_ref()
            .and_then(|page| page.settings.get(j))
            .and_then(|setting| setting.key.strip_prefix(OLD_BUTTON))
            .and_then(|n| n.parse().ok());
        let Some(button) = button else {
            return false;
        };
        let id = row.id.clone();
        self.native_settings_click(&id, button);
        self.old_native_settings_refresh(i);
        true
    }
    /// The page of the mod in row `i` opened: when it is an old mod's, its buttons asked again.
    /// Whether it was one.
    pub(in crate::game) fn old_native_settings_refresh(&mut self, i: usize) -> bool {
        let Some(row) = self.settings.mods.installed.get(i) else {
            return false;
        };
        let old = row.native.as_ref().is_some_and(|page| {
            page.settings
                .iter()
                .any(|setting| setting.key.starts_with(OLD_BUTTON))
        });
        if !old {
            return false;
        }
        let id = row.id.clone();
        let settings = self.old_native_settings(&id);
        if let Some(page) = self.settings.mods.installed[i].native.as_mut() {
            page.settings = settings;
        }
        true
    }
}

/// An old mod's button `j`, labelled `label`, as a setting of Horizon's page.
fn old_button(j: usize, label: String) -> NativeSetting {
    let (name, value) = match label.rsplit_once(": ") {
        Some((name, value)) => (name.to_owned(), value.to_owned()),
        None => (label, String::new()),
    };
    let value = match value.as_str() {
        "ON" => NativeSettingValue::Toggle(true),
        "OFF" => NativeSettingValue::Toggle(false),
        _ => NativeSettingValue::Choice {
            options: vec![value],
            index: 0,
        },
    };
    NativeSetting {
        key: format!("{OLD_BUTTON}{j}"),
        label: name,
        tooltip: None,
        value,
    }
}

/// The old helpers, by their old names and shapes, on the new ones. Only mods call them.
#[allow(dead_code)]
impl Game<'_> {
    /// The width of `text` in GUI pixels, as `native_text` draws it (`Font.width`); Horizon's
    /// `native_text_width` takes the batch it draws on first now.
    pub(in crate::game) fn native_text_width_old(&self, text: &str) -> f32 {
        HudBatch::text_width(&self.gui.atlas, text)
    }
    /// The most recently extracted world frame's camera (Horizon's `native_camera`, the split's
    /// `camera`): during the GUI the current frame's, bob, spin and FOV included.
    pub(in crate::game) fn native_camera(&self) -> Option<::mod_api::Camera> {
        self.camera()
    }
    /// Whether Shift is held (live modifier state).
    pub(in crate::game) fn native_has_shift_down(&self) -> bool {
        self.has_shift_down()
    }
    /// Whether Control is held (live modifier state).
    pub(in crate::game) fn native_has_control_down(&self) -> bool {
        self.has_control_down()
    }
    /// Whether Alt is held (live modifier state).
    pub(in crate::game) fn native_has_alt_down(&self) -> bool {
        self.has_alt_down()
    }
    /// Whether a physical key is held now.
    pub(in crate::game) fn native_is_key_down(&self, key: KeyCode) -> bool {
        self.physical_keys.contains(&key)
    }
    /// The text of language key `key`, or `fallback` when the language has none: Horizon's
    /// `Game::translate`, which takes only the key now and answers `None`.
    pub(in crate::game) fn translate_or(&self, key: &str, fallback: &str) -> String {
        horizon_client::lang::get_or(key, fallback)
    }
    /// `AbstractContainerMenu.clicked` on the open menu: Horizon's `Game::menu_click`, which the
    /// split calls `click_open_inventory` (its own `menu_click` takes a menu reference now).
    pub(in crate::game) fn menu_click_old(
        &mut self,
        slot: Option<usize>,
        button: i32,
        input: horizon_client::menu::Input,
    ) -> bool {
        self.click_open_inventory(slot, button, input)
    }
    /// The inventory screen's slot of menu `menu`'s slot `slot`: Horizon's `Game::menu_slot`,
    /// the split's `inventory_screen_slot`.
    pub(in crate::game) fn menu_slot_old(menu: &str, slot: i32) -> Option<usize> {
        Self::inventory_screen_slot(menu, slot)
    }
    /// Draws GUI sprite `name`'s `src` part into `dst`, untinted: Horizon's `native_sprite`
    /// before it took a tint. False when the sprite isn't in the atlas.
    pub(in crate::game) fn native_sprite_old(
        &self,
        batch: &mut HudBatch,
        name: &str,
        src: [f32; 4],
        dst: [f32; 4],
    ) -> bool {
        self.native_sprite(batch, name, src, dst, None)
    }
    /// Plays sound event `sound` for this player only, at `at`, after `delay` ticks (0 to 1200):
    /// false, with nothing played, on a server or for a value out of range. Horizon's
    /// `native_play_local_sound` (API 83), which the split left out.
    pub(in crate::game) fn native_play_local_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        delay: i32,
    ) -> bool {
        let fine = crate::game::native_hooks::sound_id(sound)
            && at.iter().all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
            && volume.is_finite()
            && volume >= 0.0
            && pitch.is_finite()
            && pitch >= 0.0
            && (0..=1200).contains(&delay);
        if !self.native_window() || !fine {
            return false;
        }
        self.audio.play_after(sound, at, volume, pitch, delay);
        true
    }
    /// Asks for the blur behind the open screen this frame; false when it was already asked.
    pub(in crate::game) fn native_apply_screen_blur(&self) -> bool {
        self.apply_screen_blur()
    }
    /// Sends `message` to the server as the player's chat.
    pub(in crate::game) fn native_send_chat(&mut self, message: &str) -> Result<(), String> {
        self.send_chat(message)
    }
    /// The world's clock: its game time and its time of day, while in a world.
    pub(in crate::game) fn native_clock(&self) -> Option<::mod_api::game::Clock> {
        self.clock()
    }
    /// The rain and thunder levels of the world in place, each 0.0 to 1.0.
    pub(in crate::game) fn native_weather(&self) -> (f32, f32) {
        self.weather_levels(1.0)
    }
    /// The block light and sky light at `pos` (0 to 15), 0 where it isn't loaded.
    pub(in crate::game) fn native_light(&self, pos: [i32; 3]) -> (u8, u8) {
        if !self.world.loaded(pos[0], pos[2]) {
            return (0, 0);
        }
        let light = self.world.light(pos[0], pos[1], pos[2]);
        (light & 15, light >> 4)
    }
    /// Binds mod `mod_id`'s key `name` to `key` (none: back to its default), kept with the
    /// options; whether it changed.
    pub(in crate::game) fn native_set_mod_key(
        &mut self,
        mod_id: &str,
        name: &str,
        key: Option<&str>,
    ) -> bool {
        if !self.native_window()
            || mod_id.is_empty()
            || name.is_empty()
            || mod_id.len() > 256
            || name.len() > 256
            || key.is_some_and(|k| k.len() > 512)
        {
            return false;
        }
        let old = self.settings.mods.binding(mod_id, name, ("", 0)).0;
        if key == Some(old.as_str()) || (key.is_none() && old.is_empty()) {
            return false;
        }
        match key {
            Some(key) => self.settings.mods.set_binding(mod_id, name, key, 0),
            None => {
                if let Some(keys) = self.settings.mods.keys.get_mut(mod_id) {
                    keys.remove(name);
                }
            }
        }
        self.settings.save(&Self::options_path());
        true
    }
    /// Sets this client's action bar, with the normal timer; no server command or packet.
    pub(in crate::game) fn native_client_action_bar(&mut self, text: Text, animate: bool) {
        if self.native_window() {
            self.titles.set_overlay(text, animate);
        }
    }
    /// A movement or interaction mapping (`key.forward`, `key.jump`, `key.attack`...): whether it
    /// is held, and the name of the key it is bound to.
    pub(in crate::game) fn native_mapping(&mut self, name: &str) -> Option<(bool, String)> {
        let mapping = self.mapping(name).ok()??;
        let label = keybinds::display_name(self.settings.keys.get(name)).plain();
        Some((mapping.down, label))
    }
    /// The size of a GUI atlas sprite in its own pixels, if present.
    pub(in crate::game) fn native_sprite_size(&self, name: &str) -> Option<[f32; 2]> {
        self.gui_sprite_pixels(name)
    }
    /// Gives the player `count` of `item`, as the recipe book's give does.
    pub(in crate::game) fn native_give(&mut self, item: &str, count: u32) -> Result<(), String> {
        self.recipe_give(item, count)
    }
    /// The items mod `mod_id`'s panel shows; an item the game doesn't know is left out.
    pub(in crate::game) fn native_panel_items(&mut self, mod_id: &str, items: Vec<NativeStack>) {
        let items = self.api_stacks(items);
        self.panel_items(mod_id, items);
    }
    /// The recipes mod `mod_id`'s panel shows, as Horizon's `native_panel_recipes` took them before
    /// a result could have a chance and a recipe fluids: stacks by item id, each result alone in
    /// its slot. An item the game doesn't know is left out.
    pub(in crate::game) fn native_panel_recipes_old(
        &mut self,
        mod_id: &str,
        title: &str,
        tabs: Vec<NativeStack>,
        tab: u32,
        views: Vec<NativeRecipeViewOld>,
    ) {
        let tabs = self.api_stacks(tabs);
        let views = views
            .into_iter()
            .map(|v| NativeRecipeView {
                layout: v.layout,
                width: v.width,
                height: v.height,
                slots: v.slots.into_iter().map(|s| self.api_stacks(s)).collect(),
                results: v
                    .results
                    .into_iter()
                    .map(|s| NativeRecipeResult {
                        stacks: self.api_stacks(vec![s]),
                        chance: None,
                    })
                    .collect(),
                fluid_inputs: Vec::new(),
                fluid_outputs: Vec::new(),
                note: v.note,
                action: v.action,
            })
            .collect();
        self.native_panel_recipes(mod_id, title, tabs, tab, views);
    }
    /// Old stacks by item id as the mod API's; an item the game doesn't know is left out.
    fn api_stacks(&self, stacks: Vec<NativeStack>) -> Vec<::mod_api::Stack> {
        stacks
            .into_iter()
            .filter_map(|stack| {
                let id = ::mod_api::Identifier::parse(&stack.item).ok()?;
                Some(::mod_api::Stack {
                    item: self.item_key(&id)?,
                    count: stack.count,
                    components: stack.components,
                })
            })
            .collect()
    }
    /// The most of `item` one stack holds.
    pub(in crate::game) fn native_max_stack_size(&self, item: &str) -> Option<u32> {
        let id = ::mod_api::Identifier::parse(item).ok()?;
        self.max_stack_size(self.item_key(&id)?)
    }
    /// The players on this server, in the order they joined, as the old helper listed them
    /// (Horizon's `native_server_players` gives handles now). Empty on a client and in a world
    /// nobody else can join.
    pub(in crate::game) fn native_server_players_old(&self) -> Vec<NativeServerPlayer> {
        let Some(host) = &self.lan else {
            return Vec::new();
        };
        host.hub
            .online_players()
            .map(|g| old_server_player(g, g.permission))
            .collect()
    }
    /// The online player `uuid` as the old helpers gave a player, at `permission`.
    fn old_server_player_by(&self, uuid: u128, permission: u8) -> Option<NativeServerPlayer> {
        let host = self.lan.as_ref()?;
        let g = host.hub.online_players().find(|g| g.uuid == uuid)?;
        Some(old_server_player(g, permission))
    }
    /// A player's command on the server, without its slash, for a command a mod registered and
    /// the player's level allows (`native_register_commands`): set `ran` when it is yours. The
    /// player's state is swapped in for the call, so the helpers read that player.
    pub(in crate::game) fn native_perform_command(
        &mut self,
        player: &NativeServerPlayer,
        command: &str,
        ran: &mut bool,
    ) {
        let _ = (player, command, ran);
    }
    /// The old command hook for the line being run by `uuid` at `permission`: whether a mod made
    /// for it ran it.
    pub(in crate::game) fn old_perform_command(&mut self, uuid: u128, permission: u8) -> bool {
        let line = COMMAND_LINE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if line.is_empty() {
            return false;
        }
        let Some(player) = self.old_server_player_by(uuid, permission) else {
            return false;
        };
        let mut ran = false;
        self.native_perform_command(&player, &line, &mut ran);
        ran
    }
    /// A system line in that player's chat, also inside a player's hook for the acting player;
    /// false when they aren't on.
    pub(in crate::game) fn native_tell(&mut self, uuid: u128, line: Text) -> bool {
        let content = component_nbt(&line.component());
        let packet = C::SystemChat(play::SystemChat {
            content,
            overlay: false,
        });
        self.native_send(uuid, packet)
    }
    /// A line over that player's hotbar (the action bar), also inside a player's hook for the
    /// acting player; false when they aren't on.
    pub(in crate::game) fn native_action_bar(&mut self, uuid: u128, line: Text) -> bool {
        let content = component_nbt(&line.component());
        let packet = C::SystemChat(play::SystemChat {
            content,
            overlay: true,
        });
        self.native_send(uuid, packet)
    }
    /// Whether the player left mod `mod_id` on in Options > Mods.
    pub(in crate::game) fn native_on(&self, mod_id: &str) -> bool {
        self.on(mod_id)
    }
    /// Playing, with nothing over the world: keys and zoom apply.
    pub(in crate::game) fn native_playing(&self) -> bool {
        self.playing()
    }
    /// A mod's setting `key`.
    pub(in crate::game) fn native_setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.setting(mod_id, key)
    }
    /// Keeps a mod's setting `key`.
    pub(in crate::game) fn native_set_setting(&self, mod_id: &str, key: &str, value: &str) {
        self.set_setting(mod_id, key, value);
    }
    /// The key the player chose for a mod's key `name`, else `default` (`key.keyboard.z`).
    pub(in crate::game) fn native_mod_key(
        &self,
        mod_id: &str,
        name: &str,
        default: &str,
    ) -> String {
        self.settings.mods.binding(mod_id, name, (default, 0)).0
    }
    /// Opens mod `mod_id`'s own screen; whether it opened.
    pub(in crate::game) fn native_mod_screen_open(&mut self, mod_id: &str) -> bool {
        self.mod_screen_open(mod_id)
    }
    /// The mod whose own screen is open.
    pub(in crate::game) fn native_mod_screen(&self) -> Option<&str> {
        self.mod_screen()
    }
    /// Closes mod `mod_id`'s own screen when it is open.
    pub(in crate::game) fn native_mod_screen_close(&mut self, mod_id: &str) {
        self.mod_screen_close(mod_id);
    }
    /// The open screen's title.
    pub(in crate::game) fn native_screen_title(&self) -> Option<String> {
        self.screen_title()
    }
    /// Sends `data` on `channel` to the server.
    pub(in crate::game) fn native_net_send_to_server(
        &mut self,
        channel: &str,
        data: &[u8],
    ) -> Result<(), String> {
        self.net_send_to_server(channel, data)
    }
    /// Sends `data` on `channel` to `player`.
    pub(in crate::game) fn native_net_send_to_player(
        &mut self,
        player: u128,
        channel: &str,
        data: &[u8],
    ) -> Result<(), String> {
        self.net_send_to_player(player, channel, data)
    }
    /// An image for the mod's own screen.
    pub(in crate::game) fn native_mod_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> bool {
        self.mod_image(width, height, rgba)
    }
    /// The world's folder, when one is open here.
    pub(in crate::game) fn native_world_dir(&self) -> Option<PathBuf> {
        self.world_dir()
    }
    /// The instance (game) folder.
    pub(in crate::game) fn native_instance_dir(&self) -> PathBuf {
        self.instance_dir()
    }
    /// The pointer in GUI pixels.
    pub(in crate::game) fn native_cursor(&self) -> [f32; 2] {
        self.cursor()
    }
    /// The recipe book's recipes the player knows.
    pub(in crate::game) fn native_known_recipes(&mut self) -> Arc<[i32]> {
        self.known_recipes()
    }
    /// Places recipe `book_id` in the open crafting grid.
    pub(in crate::game) fn native_place_recipe(
        &mut self,
        book_id: i32,
        all: bool,
    ) -> Result<(), String> {
        self.place_recipe(book_id, all)
    }
    /// A block's collision boxes.
    pub(in crate::game) fn native_block_collision(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        self.block_collision(pos)
    }
    /// Sends a block entity's data to the players.
    pub(in crate::game) fn native_block_entity_sync(&mut self, pos: [i32; 3]) {
        self.block_entity_sync(pos);
    }
    /// Sets a block's property with `Level.setBlock`'s `flags`.
    pub(in crate::game) fn native_set_block_property_with(
        &mut self,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool {
        self.set_block_property_with(pos, key, value, flags)
    }
    /// Pushes the player by `kick`.
    pub(in crate::game) fn native_push_player(&mut self, kick: [f32; 3]) {
        self.push_player(kick);
    }
    /// The items mod `mod_id` handles the use of itself.
    pub(in crate::game) fn native_claim_items(&mut self, mod_id: &str, ids: &[String]) {
        self.claim_items(mod_id, ids);
    }
    /// The tab list's players, each with the skin this client draws for them (Horizon's
    /// `native_client_profiles`): empty off a server and in a server's own game.
    pub(in crate::game) fn native_client_profiles(&mut self) -> Vec<NativeClientProfile> {
        if !self.native_window() {
            return Vec::new();
        }
        let Some((players, _, _)) = self.remote_tab_players() else {
            return Vec::new();
        };
        players
            .into_iter()
            .map(|p| NativeClientProfile {
                skin: self.face_skin(p.uuid, &p.skin),
                uuid: p.uuid,
                name: p.name,
            })
            .collect()
    }
    /// The health HUD's state (Horizon's `native_health_hud_state`), for a mod answering
    /// `native_health_hud`: `layout` draws its rows and moves their frame state on (the hearts'
    /// blink, the bubble pop), so call it at most once a frame.
    pub(in crate::game) fn native_health_hud_state(
        &mut self,
    ) -> &mut horizon_client::hud_layout::HealthHud {
        &mut self.health_hud
    }
    /// Where the next row above the hotbar would go, in GUI pixels up from the view's bottom: the
    /// larger of `Hud.leftHeight` and `rightHeight`, which Horizon's health HUD kept until 26.3's.
    /// The status rows' top as the HUD keeps room for them, 39 (the first row's) without rows.
    pub(in crate::game) fn status_rows_height_old(&self) -> i32 {
        let room = &self.ui.room;
        let rows = room.height() - room.bottom();
        if rows > 0.0 {
            rows.round() as i32 + 10
        } else {
            39
        }
    }
}

/// A recipe the panel shows as Horizon's API had it before results could have a chance and recipes
/// fluids (`native_panel_recipes_old`): stacks by item id, one per result. Only mods build one.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeRecipeViewOld {
    pub(crate) layout: NativeRecipeLayout,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) slots: Vec<Vec<NativeStack>>,
    pub(crate) results: Vec<NativeStack>,
    pub(crate) note: String,
    pub(crate) action: Option<bool>,
}

/// A player of the tab list as Horizon's `native_client_profiles` gives one.
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(in crate::game) struct NativeClientProfile {
    pub uuid: u128,
    pub name: String,
    /// The skin this client draws for the player.
    pub skin: String,
}

#[allow(dead_code)]
impl super::Dim {
    /// The number a dimension had when the three were all there was (`dim as u8`): 0, 1 and 2,
    /// and 3 for one past them.
    pub(in crate::game) fn old_number(self) -> u8 {
        match self {
            Self::Overworld => 0,
            Self::Nether => 1,
            Self::End => 2,
            Self::Extra(_) => 3,
        }
    }
}
