#[path = "shared/book_screen.rs"]
mod book_screen;
#[cfg(test)]
#[path = "shared/item_fall_on_tests.rs"]
mod item_fall_on_tests;
#[path = "shared/level_helpers.rs"]
mod level_helpers;
#[cfg(test)]
#[path = "shared/particle_count_tests.rs"]
pub(super) mod particle_count_tests;
#[cfg(test)]
use level_helpers::block_entity_for_block;
#[cfg(test)]
#[path = "shared/audio_math_tests.rs"]
mod audio_math_tests;
#[cfg(test)]
#[path = "shared/block_property_tests.rs"]
mod block_property_tests;
#[cfg(test)]
#[path = "shared/gameplay_hook_tests.rs"]
mod gameplay_hook_tests;
#[path = "shared/item_fallthrough.rs"]
mod item_fallthrough;
#[cfg(test)]
#[path = "shared/label_tests.rs"]
mod label_tests;
#[cfg(test)]
#[path = "shared/live_damage_tests.rs"]
mod live_damage_tests;
#[path = "shared/loot.rs"]
mod loot;
#[cfg(test)]
#[path = "shared/packet_tests.rs"]
mod packet_tests;
#[cfg(test)]
#[path = "shared/runtime_environment_tests.rs"]
mod runtime_environment_tests;
#[cfg(test)]
#[path = "shared/screen_position_tests.rs"]
mod screen_position_tests;
#[cfg(test)]
#[path = "shared/slot_icon_tests.rs"]
mod slot_icon_tests;
#[cfg(test)]
#[path = "shared/slot_tooltip_tests.rs"]
mod slot_tooltip_tests;
#[path = "shared/system_helpers.rs"]
mod system_helpers;
#[path = "shared/world_helpers.rs"]
mod world_helpers;

use super::screen_build::ObservedScreen;
use crate::game::api_values::{TryIntoApi, TryIntoEngine};
use screen_chars::chars_to_type;
use sunrise_client::containers::ModBlockEntity as EngineBlockEntity;

struct ContentsChangedGuard<'a, 'game> {
    game: &'a mut Game<'game>,
}

impl Drop for ContentsChangedGuard<'_, '_> {
    fn drop(&mut self) {
        self.game.contents_changing.pop();
    }
}

/// Write back a `menu_button` cursor edit without revalidating an untouched stack.
fn apply_menu_button_cursor(
    items: &sunrise_client::items::Items,
    original: &Option<mod_api::ItemStack>,
    cursor: &Option<mod_api::ItemStack>,
    carried: &mut Option<ItemStack>,
) {
    if cursor == original {
        return;
    }
    match cursor.try_into_engine(items) {
        Ok(updated) => *carried = updated,
        Err(error) => {
            log::warn!("mod menu_button write refused: {error:?}");
        }
    }
}

#[path = "shared/screen_view.rs"]
mod screen_view;
use screen_view::screen_view_with_id;

/// What `decide_screen` says the hooks answered to a change of screen.
enum ScreenDecision {
    /// The change goes ahead.
    Open,
    /// `Cancel`: the current screen stays.
    Keep,
    /// `ModScreen`: this mod's own screen opens instead.
    ModScreen(&'static str),
    /// `Vanilla`: this vanilla screen opens instead.
    Vanilla(ScreenTarget),
}

/// The title screen a `ScreenOpening::Vanilla` target is (its id), or None
/// for a world's screen.
fn title_target_id(target: ScreenTarget) -> Option<&'static str> {
    match target {
        ScreenTarget::Title => Some("title"),
        ScreenTarget::WorldSelect => Some("world_select"),
        ScreenTarget::WorldCreate => Some("world_create"),
        ScreenTarget::Multiplayer => Some("multiplayer"),
        ScreenTarget::AddServer => Some("add_server"),
        ScreenTarget::DirectConnect => Some("direct_connect"),
        _ => None,
    }
}

pub(super) fn observed_overlay_id(overlays: &[(bool, &'static str)]) -> Option<&'static str> {
    overlays.iter().find_map(|(open, id)| open.then_some(*id))
}

impl Game<'_> {
    pub(super) fn observed_screen_view(&self) -> Option<ScreenView<'_>> {
        let book = self.book.as_ref().map(super::books::BookScreen::id);
        let overlay = observed_overlay_id(&[
            (self.command_prompt.is_some(), "command_prompt"),
            (self.link_prompt.is_some(), "link_prompt"),
            (self.code_of_conduct.is_some(), "code_of_conduct"),
            (self.server_packs.prompt.is_some(), "server_pack_prompt"),
            (self.dialog_open(), "dialog"),
            (book.is_some(), book.unwrap_or("book_view")),
            (self.sign_edit.is_some(), "sign_editor"),
        ]);
        let Some(id) = overlay else {
            return self.screen_view_now();
        };
        let renderer = self.renderer.as_ref()?;
        let batch = self.interface_batch([renderer.config.width, renderer.config.height]);
        Some(ScreenView::from(mod_api::__private::built::ScreenView {
            id,
            menu: None,
            title: None,
            rect: [0.0, 0.0, batch.width, batch.height],
            slots: 0,
            mouse: self.cursor(),
            partial_tick: self.clock.fraction() as f32,
            regions: &[],
        }))
    }

    /// Opens `screen` (`Gui.setScreen`; `Screen::Playing` is vanilla's null):
    /// the one place the game's screen changes. A different screen runs
    /// `screen_opening` first (`ScreenEvent.Opening`: a mod may keep the
    /// current screen or open its own), then `screen_closing` for the screen
    /// it replaces. A game with no mod listening to either, and a server's
    /// or a guest's `Game`, assign the screen directly. A new native menu
    /// can take over the already shown `Inventory` screen in place.
    pub(super) fn set_screen(&mut self, screen: Screen) {
        self.set_screen_with(screen, |_| {});
    }
    /// `set_screen`, with `before` run once the hooks let the change go ahead and
    /// before it applies: what a screen's own `onClose` does only when it really
    /// closes (a container screen's close packet).
    pub(super) fn set_screen_with(&mut self, screen: Screen, before: impl FnOnce(&mut Self)) {
        let opening_native =
            screen == Screen::Inventory && self.active_native_screen().is_some_and(|s| !s.shown);
        if self.screen == screen {
            if opening_native {
                self.apply_screen_with_mod_book(screen, None);
            }
            return;
        }
        let mut screen = screen;
        let mut owner = None;
        if self.screen_hooks_heard() {
            let decision = {
                let current = self.screen_change_view(self.screen);
                let new = self.screen_change_view(screen);
                self.decide_screen(&current, &new, self.screen != Screen::Playing)
            };
            match decision {
                ScreenDecision::Keep => {
                    // Reject only the opening just staged, never the menu
                    // that an unrelated veto is keeping on screen.
                    if opening_native {
                        self.remote_close_container();
                    }
                    return;
                }
                ScreenDecision::ModScreen(mod_id) => {
                    screen = Screen::Mod;
                    owner = Some(mod_id);
                }
                ScreenDecision::Vanilla(target) => {
                    before(self);
                    if opening_native && self.active_native_screen().is_some() {
                        self.remote_close_container();
                    }
                    self.open_screen_target(target);
                    if self.title.is_some() {
                        // A title screen: told here, so the frame's observer
                        // doesn't announce it again.
                        self.screen_state.observed = self.observed_now();
                        self.shown_init_post();
                    }
                    return;
                }
                ScreenDecision::Open => {}
            }
        }
        if self.screen == screen && owner.is_none() {
            return;
        }
        before(self);
        if self.screen == Screen::Options {
            self.ui.options.discard_draw();
        }
        self.apply_screen_with_mod_book(screen, owner);
    }
    /// Whether a change of screen asks the screen hooks: a mod listens to
    /// `screen_opening` or `screen_closing`, on a client's own `Game` (not a
    /// server's, not on a guest's behalf, not the screen a mod's `Vanilla`
    /// answer opens). Otherwise it is a plain assignment.
    pub(in crate::game) fn screen_hooks_heard(&self) -> bool {
        (mod_api::__private::listened(mod_api::HookId::ScreenOpening)
            || mod_api::__private::listened(mod_api::HookId::ScreenClosing))
            && self.options.execution_role.is_client()
            && self.lan_acting.is_none()
            && !self.screen_state.replacing
    }
    /// The hooks' answer to a change from `current` to `new`, for every path
    /// that changes the screen: `screen_change` (Opening, then Closing for
    /// `current` when `closes`), then what `screen_decision` makes of it.
    fn decide_screen(
        &self,
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        closes: bool,
    ) -> ScreenDecision {
        self.screen_decision(self.screen_change(current, new, true, closes))
    }
    /// What a mod's `screen_opening` answer does here: a `ModScreen` only
    /// where a mod's screen can open (`mod_screen_blocked`), a `Vanilla`
    /// target only where it opens (a title screen while the title shows, a
    /// world's screen where a mod's could open in a world), else the change
    /// goes ahead and the refused target is logged. Records whether a mod's
    /// answer changed what shows (`ScreenState::answered`).
    fn screen_decision(&self, answer: Option<Answered<ScreenOpening>>) -> ScreenDecision {
        let decision = match answer {
            Some(Answered {
                answer: ScreenOpening::Cancel,
                ..
            }) => ScreenDecision::Keep,
            Some(Answered {
                answer: ScreenOpening::ModScreen,
                mod_id,
                ..
            }) if !self.mod_screen_blocked() => ScreenDecision::ModScreen(mod_id),
            Some(Answered {
                answer: ScreenOpening::Vanilla(target),
                mod_id,
                ..
            }) => {
                let opens = match title_target_id(target) {
                    // An overlay over the title would stay in front of it.
                    Some(_) => self.title.is_some() && !self.screen_over_playing(),
                    None => self.title.is_none() && !self.mod_screen_blocked(),
                };
                if opens {
                    ScreenDecision::Vanilla(target)
                } else {
                    log::warn!(
                        "Sunrise: mod {mod_id} answered screen_opening with {target:?}, which can't open here; the screen asked for opens"
                    );
                    ScreenDecision::Open
                }
            }
            _ => ScreenDecision::Open,
        };
        self.screen_state
            .answered
            .set(!matches!(decision, ScreenDecision::Open));
        decision
    }
    /// Opens vanilla screen `target` as it opens on its own, in place of the
    /// one asked for (`ScreenOpening::Vanilla`, which `screen_decision` let
    /// through): the change fires no screen hook again.
    fn open_screen_target(&mut self, target: ScreenTarget) {
        self.screen_state.replacing = true;
        match target {
            ScreenTarget::Pause => {
                self.pause_focus = None;
                self.set_screen(Screen::Paused);
                self.pause_menu = true;
            }
            ScreenTarget::Options => self.open_options(false),
            ScreenTarget::Inventory => {
                // A menu the server opened for the screen asked for (a crafting
                // table) closes: the inventory shows the player's own. With
                // only the player's own open, there is nothing to close.
                if self.remote_menu_type().is_some() {
                    self.close_container();
                }
                self.open_player_inventory();
            }
            ScreenTarget::Advancements => self.open_advancements(false),
            ScreenTarget::Stats => self.open_stats(),
            ScreenTarget::Chat => self.begin_chat_screen("", false),
            _ => {
                // A title screen: one of the game's own screens over the title goes.
                self.set_screen(Screen::Playing);
                if let (Some(title), Some(id)) = (self.title.as_mut(), title_target_id(target)) {
                    title.open_by_id(id);
                }
                self.release_mappings_on_screen_open();
                self.build_screen();
            }
        }
        // Every target takes the pointer, even one opened in place of a
        // request that grabbed none (a mod's screen asked for while playing).
        self.capture(false);
        self.screen_state.replacing = false;
    }
    /// Install a screen instance without firing hooks or rebuilding its layout.
    pub(super) fn activate_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.screen_activation = self.screen_activation.wrapping_add(1);
    }

    /// What `set_screen` does once the hooks have answered: `screen` opens,
    /// and the mod `owner`'s own screen when one answered `ModScreen`.
    fn apply_screen(&mut self, screen: Screen, owner: Option<&str>) {
        // `before` may already have closed it. Overlays that keep Inventory
        // underneath don't pass through this replacement path.
        if screen != Screen::Inventory && self.active_native_screen().is_some() {
            self.remote_close_container();
        }
        self.activate_screen(screen);
        if let Some(owner) = owner {
            self.native_mod_screen = Some(owner.to_owned());
        }
        if screen != Screen::Playing {
            self.release_mappings_on_screen_open();
        }
        if screen == Screen::Mod {
            self.clear_device_keys();
            self.set_mapping_held("key.attack", false);
            self.set_mapping_held("key.use", false);
            self.mining = None;
            self.capture(false);
        }
        self.build_screen();
    }
    /// The events of one change of screen, in order, for
    /// `set_screen` and for the overlays `observe_screen` announces: `opens`
    /// fires `screen_opening` (`ScreenEvent.Opening`) from `current` to `new`
    /// (a screen of `playing` is vanilla's null) and returns a mod's answer;
    /// then, unless it cancelled, `closes` fires `screen_closing` for
    /// `current`.
    fn screen_change(
        &self,
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        opens: bool,
        closes: bool,
    ) -> Option<Answered<ScreenOpening>> {
        // Asked before either screen draws: nothing beside it yet.
        let current = &crate::game::screen_backgrounds::without_regions(current);
        let new = &crate::game::screen_backgrounds::without_regions(new);
        let mut outcome = None;
        if opens {
            self.native_screen_opening(current, new, &mut outcome);
        }
        if closes
            && outcome
                .as_ref()
                .is_none_or(|a| a.answer != ScreenOpening::Cancel)
        {
            self.native_screen_closing(current);
        }
        outcome
    }
    /// `screen` as the screen hooks see it while it opens or closes: no
    /// renderer (a test, a window not yet up) leaves its rectangle empty.
    fn screen_change_view(&self, screen: Screen) -> ScreenView<'_> {
        let rect = match &self.renderer {
            Some(renderer) => self.screen_rect(
                &HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.settings.gui_scale_inputs(),
                ),
                screen,
            ),
            None => [0.0; 4],
        };
        let mut view = self.screen_view_at(screen, rect);
        if screen == Screen::Playing {
            view.slots = 0;
        }
        view
    }
    #[cfg(test)]
    pub(super) fn test_observed_title(&self) -> Option<std::sync::Arc<mod_api::Text>> {
        self.observe_view(self.screen_change_view(self.screen))
            .title
    }
    /// A mod's own screen cannot open now: over a book, a sign editor, a
    /// dialog or a prompt, with neither the title nor a player, with a
    /// world loading, or on a guest's behalf.
    fn mod_screen_blocked(&self) -> bool {
        self.lan_acting.is_some()
            || (self.player.is_none() && self.title.is_none())
            || self.screen_over_playing()
            || self.remote_loading()
    }

    /// Opens an overlay, one of the screens kept beside `Screen` (the sign
    /// editor, a book or lectern, a dialog, the command, link,
    /// code-of-conduct and server-pack prompts), as `Gui.setScreen` opens
    /// a screen: `screen_opening` fires first with `id` as the new screen,
    /// then `screen_closing` for an overlay it replaces. A mod's `Cancel`
    /// keeps what is open and `open` never runs; `ModScreen` opens the
    /// mod's own screen instead, unless one can't open now, and `Vanilla` its
    /// vanilla screen. Otherwise `open`
    /// sets the overlay and `screen_init_post` follows. Returns whether
    /// `open` ran. With no mod loaded, on a server's `Game` and on a guest's
    /// behalf, `open` just runs.
    pub(super) fn open_screen_overlay(
        &mut self,
        id: &'static str,
        open: impl FnOnce(&mut Self),
    ) -> bool {
        if !self.screen_hooks_heard() {
            open(self);
            self.release_mappings_on_screen_open();
            self.screen_state.build.reopened();
            self.build_screen();
            return true;
        }
        // What changed earlier this frame is told first, so `screen_state`
        // is what shows now.
        self.observe_screen();
        let decision = {
            let none = self.screen_change_view(Screen::Playing);
            let from = self
                .screen_state
                .observed
                .as_ref()
                .filter(|old| old.overlay);
            let from = from.map(ObservedScreen::view);
            let to = screen_view_with_id(&none, id);
            self.decide_screen(from.as_ref().unwrap_or(&none), &to, from.is_some())
        };
        match decision {
            ScreenDecision::Keep => return false,
            ScreenDecision::ModScreen(mod_id) => {
                self.apply_screen(Screen::Mod, Some(mod_id));
                self.screen_state.observed = self.observed_now();
                return false;
            }
            ScreenDecision::Vanilla(target) => {
                self.open_screen_target(target);
                self.screen_state.observed = self.observed_now();
                self.shown_init_post();
                return false;
            }
            ScreenDecision::Open => {}
        }
        open(self);
        self.release_mappings_on_screen_open();
        self.screen_state.build.reopened();
        self.build_screen();
        // Told here, so the frame's `observe_screen` sees no change to announce.
        self.screen_state.observed = self.observed_now();
        let none = self.screen_change_view(Screen::Playing);
        self.native_screen_init_post(&screen_view_with_id(&none, id));
        true
    }

    /// What `observe_screen` compares with the last frame's.
    fn observed_now(&self) -> Option<ObservedScreen> {
        // Only the title's screens and the overlays show over `Screen::Playing`.
        self.observed_screen_view()
            .map(|view| self.observe_view(view))
    }

    pub(super) fn observe_view(&self, view: ScreenView<'_>) -> ObservedScreen {
        ObservedScreen {
            id: view.id,
            overlay: self.screen == Screen::Playing,
            menu: view.menu.map(str::to_owned),
            title: view
                .title
                .and_then(|_| self.remote.as_ref()?.shared_menu_title_snapshot()),
            rect: view.rect,
            slots: view.slots,
            mouse: view.mouse,
            partial_tick: view.partial_tick,
        }
    }

    /// Opens the title screen `Title::set_mode` asked for, as
    /// `Minecraft.setScreen` opens the title's screens: `screen_opening` fires
    /// from the title screen shown to the new one, then `screen_closing` for
    /// the one it leaves, and the change applies unless a mod answered
    /// `Cancel`; `ModScreen` opens the mod's own screen over the title instead,
    /// and `Vanilla` another title screen. Then `screen_init_post` follows and the
    /// frame's observer is told, so it does not announce it again. Each call
    /// also sets the title's gate: when no mod listens to the screen hooks
    /// (`screen_hooks_heard`), `set_mode` applies at once. Run after each window
    /// event and control command, and each frame.
    pub(super) fn settle_title_mode(&mut self) {
        let gated = self.screen_hooks_heard();
        let Some(title) = self.title.as_mut() else {
            return;
        };
        title.gated = gated;
        let Some(mode) = title.take_requested() else {
            return;
        };
        let (current, id) = (title.mode.id(), mode.id());
        // What changed before is told first.
        let now = self.observed_now();
        self.observe_change(now);
        let decision = {
            let none = self.screen_change_view(Screen::Playing);
            let from = screen_view_with_id(&none, current);
            let to = screen_view_with_id(&none, id);
            self.decide_screen(&from, &to, true)
        };
        match decision {
            ScreenDecision::Keep => return,
            ScreenDecision::ModScreen(mod_id) => self.apply_screen(Screen::Mod, Some(mod_id)),
            ScreenDecision::Vanilla(target) => self.open_screen_target(target),
            ScreenDecision::Open => {
                if let Some(title) = self.title.as_mut() {
                    title.activate_mode(mode);
                }
                self.release_mappings_on_screen_open();
                self.build_screen();
            }
        }
        self.screen_state.observed = self.observed_now();
        self.shown_init_post();
    }
    /// `screen_init_post` for what shows now, once a change was told to the
    /// frame's observer: a title screen, a mod's, or the game's own.
    fn shown_init_post(&self) {
        let view = self.screen_change_view(self.screen);
        let id = match &self.title {
            Some(title) if self.screen == Screen::Playing => title.mode.id(),
            _ => view.id,
        };
        self.native_screen_init_post(&screen_view_with_id(&view, id));
    }

    /// Shows `title`, as `Minecraft.setScreen(new TitleScreen())` (or the
    /// screen it opens on: the multiplayer list, a disconnect, an error)
    /// does at startup and when a world closes. `screen_opening` fires to its
    /// screen, then `screen_closing` for an overlay it replaces, before it
    /// shows; `screen_init_post` follows. With no world open nothing else can
    /// show, so a mod's `Cancel` doesn't keep it shut; `ModScreen` opens the
    /// mod's own screen over it, and `Vanilla` another title screen.
    pub(super) fn show_title(&mut self, mut title: sunrise_client::title::Title) {
        title.set_rule_registry(Arc::new(crate::game::api_values::game_rule_registry()));
        self.show_title_with_registry(title);
    }

    /// Takes the title down as a world opens over it (`setScreen` to the
    /// loading screen, then null): `screen_opening` to no screen, then
    /// `screen_closing` for the title's screen, before it goes. The world
    /// is opening, so a `Cancel` doesn't keep it.
    pub(super) fn hide_title(&mut self) {
        if self.title.is_none() {
            return;
        }
        if self.screen_hooks_heard() {
            self.title_change(None);
        }
        self.title = None;
        self.screen_activation = self.screen_activation.wrapping_add(1);
        self.build_screen();
        self.screen_state.observed = self.observed_now();
    }
    /// The hooks of the title appearing (`to` is its screen) or going
    /// (`None`), from what shows now, and a mod's answer. What changed before
    /// is told first.
    fn title_change(&mut self, to: Option<&'static str>) -> Option<Answered<ScreenOpening>> {
        let now = self.observed_now();
        self.observe_change(now);
        let none = self.screen_change_view(Screen::Playing);
        // The title's screen or an overlay; a `Screen` told its own change.
        let from = self
            .screen_state
            .observed
            .as_ref()
            .filter(|old| old.overlay);
        let from = match (from, self.title.as_ref()) {
            (Some(old), _) => Some(old.view()),
            // No renderer yet (startup, a test): the title is still known.
            (None, Some(title)) => Some(screen_view_with_id(&none, title.mode.id())),
            (None, None) => None,
        };
        let to = to.map(|id| screen_view_with_id(&none, id));
        self.screen_change(
            from.as_ref().unwrap_or(&none),
            to.as_ref().unwrap_or(&none),
            true,
            from.is_some(),
        )
    }

    pub(super) fn observe_screen(&mut self) {
        self.settle_title_mode();
        // Keeps the build on the frame's window size and drops it once nothing
        // shows; an unchanged frame compares and builds nothing.
        self.build_screen();
        // Free with no native mod: the snapshot clones the title and menu.
        if mod_api::__private::mods().is_empty() {
            return;
        }
        let current = self.observed_now();
        self.observe_change(current);
    }
    /// What the frame's screen changed since the last one. The `Screen` list,
    /// the overlays' opening and the title's screens announce their own
    /// changes before they apply (`set_screen`, `open_screen_overlay`,
    /// `settle_title_mode`, `show_title`, `hide_title`); what is left, an
    /// overlay closing, is announced here, through the same `screen_change`
    /// and in the same order, but after the change. `screen_init_post`
    /// follows for every kind, on opening and on a resize.
    fn observe_change(&mut self, current: Option<ObservedScreen>) {
        let changed = match (&self.screen_state.observed, &current) {
            (Some(old), Some(new)) => old.id != new.id || old.menu != new.menu,
            (None, None) => false,
            _ => true,
        };
        if changed {
            let old = self
                .screen_state
                .observed
                .as_ref()
                .filter(|old| old.overlay);
            let opens = current.as_ref().is_some_and(|new| new.overlay)
                || (current.is_none() && old.is_some());
            if old.is_some() || opens {
                let none = self.screen_change_view(Screen::Playing);
                let from = old.map(ObservedScreen::view);
                let to = current.as_ref().map(ObservedScreen::view);
                self.screen_change(
                    from.as_ref().unwrap_or(&none),
                    to.as_ref().unwrap_or(&none),
                    opens,
                    from.is_some(),
                );
            }
            if let Some(new) = &current {
                self.native_screen_init_post(&new.view());
            }
        } else if let (Some(old), Some(new)) = (&self.screen_state.observed, &current)
            && old.rect != new.rect
        {
            self.native_screen_init_post(&new.view());
        }
        self.screen_state.observed = current;
    }
}

/// `Level.setBlock`'s flags, for the `native_set_block_property` helpers:
/// `flags::UPDATE_CLIENTS` as a vanilla crop grows, `flags::UPDATE_ALL` as a
/// furnace sets its `lit`.
pub(super) use sunrise_world::level::flags;

/// A mod's camera (`native_camera_pose`): each value that isn't finite keeps vanilla's from `view`.
fn posed_camera(
    view: (glam::DVec3, f32, f32),
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
) -> (glam::DVec3, f32, f32) {
    let position = std::array::from_fn(|i| {
        if position[i].is_finite() {
            position[i]
        } else {
            view.0[i]
        }
    });
    (
        glam::DVec3::from_array(position),
        if yaw.is_finite() { yaw } else { view.1 },
        if pitch.is_finite() { pitch } else { view.2 },
    )
}

/// The largest turn a shake gives, in degrees, on each axis.
const SHAKE_MAX_DEGREES: f32 = 10.0;
/// The largest offset a shake gives, in blocks, along each axis.
const SHAKE_MAX_OFFSET: f32 = 0.5;

use camera_pose::bounded_shake;
pub(super) use camera_pose::hand_pose_matrix;

/// The pose a bounded shake multiplies into the projection, after the view
/// bobbing: the offset, then roll, pitch and yaw.
fn shake_pose(shake: Shake) -> glam::Mat4 {
    let [x, y, z] = shake.offset;
    glam::Mat4::from_translation(glam::Vec3::new(-x, -y, -z))
        * glam::Mat4::from_rotation_z(shake.roll.to_radians())
        * glam::Mat4::from_rotation_x(shake.pitch.to_radians())
        * glam::Mat4::from_rotation_y(shake.yaw.to_radians())
}

/// This client's window-owned mod screen and its local slot state.
pub(super) struct OpenNativeScreen {
    /// Fixed for the opening; slot moves have their own revision.
    pub(super) layout_revision: u64,
    slot_moves: SlotMoves,
    screen: mod_api::__private::OwnedMenuScreen,
    menu: OpenNativeMenu,
    /// Installed as the window's screen, not merely staged for ScreenOpening.
    shown: bool,
}

/// The menu the screen presents: where its slots are stored, the opener,
/// and the window's live data.
struct OpenNativeMenu {
    key: Pos,
    window: i32,
    generation: u64,
    /// The menu's type, which its screen factory is keyed by.
    name: String,
    /// The mod block whose matching container opened the menu, if any.
    block: Option<String>,
    pos: Option<[i32; 3]>,
    mod_type: Option<&'static sunrise_client::containers::ModMenuType>,
    data: Vec<i32>,
}

#[cfg(test)]
fn seed_native_menu(game: &mut Game<'_>, window: i32, name: &str) -> u64 {
    if game.remote.is_none() {
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Native screen fixture").unwrap();
    }
    if game.remote.as_ref().unwrap().menu.as_ref().is_some() {
        game.remote_close_local();
    }
    let menu = &mut game.remote.as_mut().unwrap().menu;
    menu.set(Some((window, name.to_owned())));
    menu.generation
}

/// Where a mod moved the open screen's slots after it opened, over the
/// positions its `MenuScreenLayout` declared. Interior state, as the screen callbacks that
/// move slots hold `&self`. A move is pending until the layout build commits
/// it, so drawing and hit-testing read one position within a frame and both
/// take a move together from the next.
#[derive(Default)]
pub(super) struct SlotMoves {
    /// Each moved slot's position, or None for a slot a mod hid.
    committed: std::cell::RefCell<std::collections::HashMap<usize, Option<[f32; 2]>>>,
    pending: std::cell::RefCell<std::collections::HashMap<usize, Option<[f32; 2]>>>,
    /// How many moves changed a position, for the layout build's cache key.
    count: std::cell::Cell<u32>,
}

impl SlotMoves {
    /// Slot `slot`'s position this frame: where the last build committed it
    /// (None once hidden), else `declared`.
    fn position(&self, slot: usize, declared: Option<[f32; 2]>) -> Option<[f32; 2]> {
        self.committed
            .borrow()
            .get(&slot)
            .copied()
            .unwrap_or(declared)
    }
    /// Where slot `slot` of a screen that declared `slots` will be from the
    /// next build: the last move, pending or committed, else its declaration.
    fn latest(&self, slots: &[Option<mod_api::ScreenSlot>], slot: usize) -> Option<[f32; 2]> {
        let declared = slots.get(slot)?.as_ref().map(|slot| slot.position);
        let pending = self.pending.borrow().get(&slot).copied();
        match pending {
            Some(pending) => pending,
            None => self.position(slot, declared),
        }
    }
    /// Moves or activates slot `slot` of a screen that declared `slots` to `pos`,
    /// or hides it with None, from the next build (`Slot.isActive()`). A refusal
    /// or a repeat changes nothing, not even the move count.
    fn set(
        &self,
        slots: &[Option<mod_api::ScreenSlot>],
        slot: usize,
        pos: Option<[f32; 2]>,
    ) -> Result<(), mod_api::ScreenSlotRefusal> {
        use mod_api::ScreenSlotRefusal;
        if slot >= slots.len() {
            return Err(ScreenSlotRefusal::NoSuchSlot);
        }
        if pos.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite()) {
            return Err(ScreenSlotRefusal::BadPosition);
        }
        if self.latest(slots, slot) != pos {
            self.pending.borrow_mut().insert(slot, pos);
            self.count.set(self.count.get().wrapping_add(1));
        }
        Ok(())
    }
    /// Makes the pending moves the positions drawing and hit-testing read.
    fn commit(&self) {
        let mut pending = self.pending.borrow_mut();
        if !pending.is_empty() {
            self.committed.borrow_mut().extend(pending.drain());
        }
    }
}

impl Game<'_> {
    /// The staged or shown native screen belonging to this opening, not a
    /// prior opening of the same numeric window id.
    fn active_native_screen(&self) -> Option<&OpenNativeScreen> {
        let screen = self.mod_screen.as_ref()?;
        let remote = &self.remote.as_ref()?.menu;
        (remote.generation == screen.menu.generation && remote.as_ref()?.0 == screen.menu.window)
            .then_some(screen)
    }

    /// The menu type resolved for this client's opening, including in ScreenOpening.
    pub(in crate::game) fn native_screen_menu_type(
        &self,
        window: i32,
        name: &str,
    ) -> Option<&'static sunrise_client::containers::ModMenuType> {
        let menu = &self.active_native_screen()?.menu;
        (menu.window == window && menu.name == name).then_some(menu.mod_type)?
    }
}

impl OpenNativeScreen {
    fn layout(&self) -> &mod_api::MenuScreenLayout {
        self.screen.layout()
    }

    #[cfg(test)]
    fn layout_mut(&mut self) -> &mut mod_api::MenuScreenLayout {
        self.screen.layout_mut()
    }

    fn frame(
        &self,
        batch: &HudBatch,
        (ox, oy): (f32, f32),
        (x, y): (f32, f32),
    ) -> mod_api::MenuScreenFrame<'_> {
        mod_api::MenuScreenFrame {
            origin: [ox, oy],
            pointer: [x as i32, y as i32],
            gui_size: [batch.width as i32, batch.height as i32],
            at: [x - ox, y - oy],
            data: &self.menu.data,
        }
    }

    /// How many slot moves changed a position, so a layout built before the
    /// last one is rebuilt.
    pub(in crate::game) fn slot_move_count(&self) -> u32 {
        self.slot_moves.count.get()
    }
    /// Makes the slot moves a mod made since the last build the screen's
    /// positions; the layout build calls it before it reads them.
    pub(in crate::game) fn commit_slot_moves(&self) {
        self.slot_moves.commit();
    }
    /// Data slot `index` of window `window`'s menu: false for another window.
    /// The caller resolves the shown opening before passing it a packet.
    fn set_data(&mut self, window: i32, index: usize, value: i32) -> bool {
        if window != self.menu.window {
            return false;
        }
        if self.menu.mod_type.map_or(
            index < usize::from(sunrise_mod_package::content::MAX_SIGNED_DATA_SLOTS),
            |kind| index < kind.data_slots,
        ) {
            if self.menu.data.len() <= index {
                self.menu.data.resize(index + 1, 0);
            }
            self.menu.data[index] = value;
        }
        true
    }

    pub(super) fn container_slot_base(&self) -> usize {
        if self.menu.mod_type.is_some() {
            super::inventory_slots::MOD_MENU_SLOT_BASE
        } else {
            300
        }
    }

    fn player_slots(&self) -> usize {
        self.menu
            .mod_type
            .map_or(36, sunrise_client::containers::ModMenuType::player_slots)
    }

    fn visible_container_slots(&self) -> impl Iterator<Item = usize> + '_ {
        let base = self.container_slot_base();
        self.layout()
            .slots
            .iter()
            .enumerate()
            .filter_map(move |(i, slot)| {
                self.slot_moves
                    .position(i, slot.as_ref().map(|slot| slot.position))?;
                Some(base + i)
            })
    }

    pub(super) fn visible_slots(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.player_slots()).chain(self.visible_container_slots())
    }
}

/// The player as a native mod reads it.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct NativePlayer {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Degrees; 0 faces south, 90 west (`Direction.fromYRot`).
    pub yaw: f32,
    pub pitch: f32,
    /// `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`.
    pub dimension: &'static str,
}

/// How the panel lays out a recipe of `native_panel_recipes`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeRecipeLayout {
    /// A crafting grid of `width` x `height` slots, an arrow and the result.
    Grid,
    /// One input over a flame, an arrow and the result (furnaces, smokers,
    /// campfires).
    Cooking,
    /// One input, an arrow and the result (the stonecutter).
    Single,
    /// Template, base and addition, an arrow and the result.
    Smithing,
    /// A potion and an ingredient, an arrow and the result.
    Brewing,
}

/// One result of a recipe the panel shows (`native_panel_recipes`): a slot
/// cycling through `stacks`, one a second, and the chance in percent the recipe
/// gives it (`Some(25.0)` draws "25%"); `None` gives it every time.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeRecipeResult {
    pub(crate) stacks: Vec<mod_api::Stack>,
    pub(crate) chance: Option<f32>,
}

/// A recipe the panel shows (`native_panel_recipes`). Each slot, and each
/// result, cycles through its stacks, one a second; an empty slot shows
/// nothing. A crafting recipe has one result and no chance.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeRecipeView {
    pub(crate) layout: NativeRecipeLayout,
    /// The grid's size (`Grid` only), at most 3 x 3; slots past it continue
    /// in rows under it, 27 in all.
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) slots: Vec<Vec<mod_api::Stack>>,
    /// Up to four, left to right after the arrow.
    pub(crate) results: Vec<NativeRecipeResult>,
    /// Fluids the recipe takes and gives, `amount` in millibuckets, each in
    /// a well with its amount under it; at most eight each. A well shows the
    /// fluid's client look (`data/<mod>/sunrise/fluid/`: sprite and tint), a
    /// flat square of vanilla's colour for water and lava and else grey when
    /// the fluid has none.
    pub(crate) fluid_inputs: Vec<FluidStack>,
    pub(crate) fluid_outputs: Vec<FluidStack>,
    /// A short line under the recipe (a cooking time), or "".
    pub(crate) note: String,
    /// The action button beside it: none, enabled, or shown greyed.
    pub(crate) action: Option<bool>,
}

/// An item of [`NativeRecipes::items`], by its index there, with a count
/// and the components it has beyond the item's own, in vanilla's item
/// syntax (`[minecraft:potion_contents={potion:"minecraft:swiftness"}]`),
/// or "".
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeRecipeStack {
    pub(crate) item: u32,
    pub(crate) count: u32,
    pub(crate) components: String,
}

/// The station a recipe is made at.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeRecipeKind {
    CraftingShaped,
    CraftingShapeless,
    Smelting,
    Blasting,
    Smoking,
    Campfire,
    Stonecutting,
    Smithing,
    Brewing,
}

/// A recipe of [`NativeRecipes`].
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeRecipe {
    /// `minecraft:oak_planks`.
    pub(crate) id: String,
    pub(crate) kind: NativeRecipeKind,
    /// The recipe book's tab (`crafting_building_blocks`, `furnace_food`),
    /// or "".
    pub(crate) category: String,
    /// Recipes the book shows on one button share a group, or "".
    pub(crate) group: String,
    /// A shaped recipe's grid: `slots` holds its cells row by row. 0 for
    /// the other kinds.
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Each slot's stacks, any one of which fits; an empty list is an empty
    /// cell. Cooking and stonecutting: the input. Smithing: the template,
    /// the base, the addition. Brewing: the bottle, the reagent.
    pub(crate) slots: Vec<Vec<NativeRecipeStack>>,
    /// What it makes; several when that varies (a trim on any armour).
    pub(crate) results: Vec<NativeRecipeStack>,
    /// Cooking: game ticks and experience; 0 for the other kinds.
    pub(crate) cooking_time: u32,
    pub(crate) experience: f32,
    /// The recipe book's id for it (one per way the book shows it), as
    /// `known_recipes` lists them; -1 for the kinds the book doesn't
    /// show (campfire, stonecutting, smithing, brewing).
    pub(crate) book_id: i32,
}

/// An item of [`NativeRecipes::items`].
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeItem {
    /// `minecraft:diamond`, `<mod>:<path>`.
    pub(crate) id: String,
    /// In the player's language.
    pub(crate) name: String,
    /// Its item tags, without the `#`.
    pub(crate) tags: Vec<String>,
}

/// Every item and recipe the game knows (`native_recipes`): `items` in the
/// registry's order, which each [`NativeRecipeStack::item`] indexes; the
/// recipes are the book's in its order, then campfire, stonecutting,
/// smithing and brewing.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeRecipes {
    pub(crate) items: Vec<NativeItem>,
    pub(crate) recipes: Vec<NativeRecipe>,
}

/// A physical press and its original receiver. Activation counters, unlike screen IDs or
/// layout revisions, distinguish same-kind replacements but do not change on resize.
#[derive(Clone, Copy)]
pub(super) struct ScreenPress {
    pub(super) activation: u64,
    pub(super) screen: Screen,
    pub(super) title_mode: Option<u64>,
    pub(super) button: i32,
    pub(super) millis: u64,
}

impl ScreenPress {
    pub(super) fn is_double_click(self, last: Option<Self>) -> bool {
        last.is_some_and(|last| {
            self.activation == last.activation
                && self.screen == last.screen
                && self.title_mode == last.title_mode
                && self.button == last.button
                && self.millis - last.millis < 250 // MouseHandler.DOUBLE_CLICK_THRESHOLD_MS
        })
    }
}

/// The button event and its original receiver, retained across a handler that may replace it.
pub(super) struct ScreenButtonPost {
    pub(super) screen: ObservedScreen,
    pub(super) button: i32,
    pub(super) pressed: bool,
    pub(super) modifiers: u32,
    pub(super) double_click: bool,
}

/// A scroll/drag notice owed after its Pre let the input through.
pub(super) enum ScreenMousePost {
    Scroll([f64; 2]),
    Drag { button: i32, delta: [f32; 2] },
}

/// What a drag's Pre decided (`Game::screen_mouse_dragged`).
pub(super) enum ScreenMousePre {
    /// No screen hook ran: no drag, a synthetic move or no screen open.
    NotFired,
    Canceled,
    Passed(ScreenMousePost),
}

/// What a `ScreenEvent.CharacterTyped.Pre` leaves of `text`, given `canceled`
/// (whether a hook cancelled character `c`): the cancelled printable
/// characters are gone and every other character stays, control characters
/// included — a text box reads Backspace, Delete or Escape as keys and vanilla
/// fires no `charTyped` for them, so they are never offered to the hook.
/// `None` when nothing is left to type.
fn screen_text_hooked(text: &str, mut canceled: impl FnMut(char) -> bool) -> Option<String> {
    let mut left = String::with_capacity(text.len());
    for c in text.chars() {
        if !c.is_control() && canceled(c) {
            continue;
        }
        left.push(c);
    }
    if left.is_empty() && !text.is_empty() {
        return None;
    }
    Some(left)
}

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Admission for ordinary toasts. Only an accepted candidate enters its
    /// queue; mandatory callback-panic diagnostics bypass mod admission.
    pub(super) fn add_toast(
        &self,
        kind: mod_api::ToastKind,
        title: &mut Text,
        description: &mut Option<Text>,
    ) -> bool {
        // Shared advancement/recipe producers also run on logical servers;
        // only their receiving client owns toast presentation and its hook.
        if self.is_logical_server() {
            return false;
        }
        let mut cancel = false;
        self.native_toast_add(kind, title, description, &mut cancel);
        !cancel
    }

    pub(super) fn add_system_toast(
        &self,
        mut toast: sunrise_client::toasts::SystemToast,
    ) -> Option<sunrise_client::toasts::SystemToast> {
        self.add_toast(
            mod_api::ToastKind::System,
            &mut toast.title,
            &mut toast.message,
        )
        .then_some(toast)
    }

    pub(super) fn settle_title_toasts(&mut self) {
        let pending = self
            .title
            .as_mut()
            .and_then(|title| {
                title.poll_backup();
                title.take_pending_toast()
            })
            .or_else(|| {
                // The launcher's "Playing offline" can sit behind this window: say it here too, once
                // the toast slot is free.
                let reason = self
                    .options
                    .offline_reason
                    .take_if(|_| self.title.as_ref().is_some_and(|title| !title.has_toast()))?;
                Some(
                    sunrise_client::toasts::SystemToast::new(
                        sunrise_client::text::Text::tr("sunrise.title.playing_offline", &[]),
                        Some(sunrise_client::text::Text::literal(reason)),
                    )
                    .with_display_time(sunrise_client::toasts::SystemToast::LONG_DISPLAY_TIME),
                )
            });
        if let Some(toast) = pending.and_then(|toast| self.add_system_toast(toast))
            && let Some(title) = self.title.as_mut()
        {
            title.apply_toast(toast);
        }
    }
    /// The milliseconds a client tick lasts at this frame's clock advance
    /// (`Minecraft.getTickTargetMillis`), as a mod's `tick_target_millis` leaves it. An answer
    /// that is not a finite number above zero keeps the clock's own.
    pub(super) fn tick_target_millis(&self) -> f32 {
        let vanilla = self.clock.tick_target_millis();
        let mut millis = vanilla;
        self.native_tick_target_millis(&mut millis);
        if millis.is_finite() && millis > 0.0 {
            millis
        } else {
            vanilla
        }
    }
    /// The hearts the local player's health bar draws (`Hud.HeartType.forPlayer`, then
    /// NeoForge's `PlayerHeartTypeEvent`): poisoned, else withered, else frozen when fully
    /// frozen, else normal, as a mod's `player_heart_type` leaves it. None without a player.
    pub(super) fn player_heart_kind(&mut self) -> Option<mod_api::client::hud_layout::HeartKind> {
        use mod_api::client::hud_layout::HeartKind;
        let p = self.player.as_ref()?;
        let effects = &self.potions.effects;
        let mut kind = if effects.get("poison").is_some() {
            HeartKind::Poisoned
        } else if effects.get("wither").is_some() {
            HeartKind::Withered
        } else if p.is_fully_frozen() {
            HeartKind::Frozen
        } else {
            HeartKind::Normal
        };
        self.native_player_heart_type(&mut kind);
        Some(kind)
    }
    /// Deliver a non-repeat key press to the native panel.
    pub(super) fn native_panel_key_event(&mut self, key: KeyCode, pressed: bool, repeat: bool) {
        if pressed
            && !repeat
            && let Some(name) = keybinds::key_name(key)
        {
            self.native_panel_key(name);
        }
    }
    /// Whether any mod listens to a hook given the open screen's
    /// `ScreenView` (render, input, open and close, room, exclusion): only
    /// then are its regions collected.
    pub(super) fn screen_view_listened(&self) -> bool {
        use mod_api::HookId;
        [
            HookId::ScreenRenderPre,
            HookId::ScreenRenderBackground,
            HookId::ScreenRenderForeground,
            HookId::ScreenRenderPost,
            HookId::ScreenMouseButtonPressedPre,
            HookId::ScreenMouseButtonPressedPost,
            HookId::ScreenMouseButtonReleasedPre,
            HookId::ScreenMouseButtonReleasedPost,
            HookId::ScreenMouseDraggedPre,
            HookId::ScreenMouseDraggedPost,
            HookId::ScreenMouseScrolledPre,
            HookId::ScreenMouseScrolledPost,
            HookId::ScreenKeyPressedPre,
            HookId::ScreenKeyPressedPost,
            HookId::ScreenKeyReleasedPre,
            HookId::ScreenKeyReleasedPost,
            HookId::ScreenCharTypedPre,
            HookId::ScreenCharTypedPost,
            HookId::ScreenOpening,
            HookId::ScreenClosing,
            HookId::ScreenInitPost,
            HookId::ScreenRoom,
            HookId::ScreenExclusionAreas,
        ]
        .into_iter()
        .any(mod_api::__private::listened)
    }
    /// Whether a mod listens to one of the four screen render hooks; nothing
    /// is built to fire them otherwise.
    pub(super) fn screen_render_listened(&self) -> bool {
        use mod_api::HookId;
        [
            HookId::ScreenRenderPre,
            HookId::ScreenRenderBackground,
            HookId::ScreenRenderForeground,
            HookId::ScreenRenderPost,
        ]
        .into_iter()
        .any(mod_api::__private::listened)
    }
    /// `Render.Pre`: whether the screen draws. The screen owns its later phases.
    pub(super) fn screen_render_begin(&self, draw: &mut HudBatch, screen: &ScreenView<'_>) -> bool {
        let mut canceled = false;
        self.native_screen_render_pre(draw, screen, &mut canceled);
        !canceled
    }
    /// `Render.Post`, including when Pre cancelled the screen's drawing.
    pub(super) fn screen_render_end(&self, draw: &mut HudBatch, screen: &ScreenView<'_>) {
        self.native_screen_render_post(draw, screen);
    }
    /// The open screen as a native mod sees it (`ScreenView`): its id,
    /// the open menu's type and title, its slot count, and its panel's
    /// rectangle, with the mouse and the partial tick. `screen` is the screen
    /// drawn this frame, which `ScreenView.id` comes from. Borrowed:
    /// nothing is copied per frame.
    pub(super) fn screen_view(&self, batch: &HudBatch, screen: Screen) -> ScreenView<'_> {
        self.screen_view_at(screen, self.screen_rect(batch, screen))
    }
    /// `screen_view` with the panel `rect` the caller is laying out: the
    /// container build tells a mod the panel it is about to draw, not the one
    /// the previous frame drew.
    pub(super) fn screen_view_at(&self, screen: Screen, rect: [f32; 4]) -> ScreenView<'_> {
        // Only the inventory screen shows a server's menu: a menu staged for
        // a screen a mod replaced is not the pause or chat screen's.
        let menu_shown = screen == Screen::Inventory;
        ScreenView::from(mod_api::__private::built::ScreenView {
            id: screen.id(),
            menu: self
                .remote_menu_type()
                .filter(|m| menu_shown && !m.is_empty()),
            title: self
                .remote
                .as_ref()
                .and_then(|r| r.menu_title_snapshot())
                .filter(|_| menu_shown),
            rect,
            slots: self.slot_count(),
            mouse: self.cursor(),
            partial_tick: self.clock.fraction() as f32,
            regions: self.screen_regions(screen),
        })
    }
    /// The title's own screen as a native mod sees it (`ScreenView`):
    /// its id from the title state machine (`title`, `world_select`,
    /// `world_create`, `multiplayer`, `add_server`…), no menu and no menu
    /// title, the whole window as its rectangle and no slots (no menu is
    /// open). None while no title screen is up.
    pub(super) fn title_screen_view(&self, batch: &HudBatch) -> Option<ScreenView<'_>> {
        let title = self.title.as_ref()?;
        Some(ScreenView::from(mod_api::__private::built::ScreenView {
            id: title.mode.id(),
            menu: None,
            title: None,
            rect: [0.0, 0.0, batch.width, batch.height],
            slots: 0,
            mouse: self.cursor(),
            partial_tick: self.clock.fraction() as f32,
            regions: &[],
        }))
    }
    /// The open screen's container panel in GUI pixels (`screen`'s, the one
    /// this frame draws), or the whole window for a screen with none
    /// (`ScreenView.rect`).
    pub(super) fn screen_rect(&self, batch: &HudBatch, screen: Screen) -> [f32; 4] {
        if screen != Screen::Inventory && screen != Screen::Creative {
            return [0.0, 0.0, batch.width, batch.height];
        }
        // The window the game itself draws (`container_image`): the merchant
        // and the beacon are wider and shifted, creative is its own size.
        let image = self.container_image(batch);
        [
            image.left as f32,
            image.top as f32,
            image.width as f32,
            image.height as f32,
        ]
    }
    pub(super) fn track_drag_button(&mut self, button: MouseButton, pressed: bool) {
        // Any release ends the drag, as vanilla's `MouseHandler.onButton` clears `activeButton`:
        // left down, right down, left up sends no drag while right is still held.
        self.drag_button = pressed.then_some(button);
    }
    pub(super) fn screen_mouse_dragged(&self, delta: [f32; 2]) -> ScreenMousePre {
        if self.pad_synthetic_input() || delta == [0.0; 2] {
            return ScreenMousePre::NotFired;
        }
        let Some(button) = self
            .drag_button
            .filter(|b| self.physical_buttons.contains(b))
        else {
            return ScreenMousePre::NotFired;
        };
        let Some(screen) = self.screen_view_now() else {
            return ScreenMousePre::NotFired;
        };
        let button = super::glfw_button(button);
        let mut canceled = false;
        self.native_screen_mouse_dragged_pre(
            &screen,
            button,
            delta,
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        if canceled {
            ScreenMousePre::Canceled
        } else {
            ScreenMousePre::Passed(ScreenMousePost::Drag { button, delta })
        }
    }
    /// Button Post sees the original receiver and its answer; return the final handled bit.
    pub(super) fn screen_button_post(&self, post: &ScreenButtonPost, handled: bool) -> bool {
        let screen = post.screen.view();
        let mut result = TriState::Default;
        if post.pressed {
            self.native_screen_mouse_button_pressed_post(
                &screen,
                post.button,
                post.modifiers,
                post.double_click,
                handled,
                &mut result,
            );
        } else {
            self.native_screen_mouse_button_released_post(
                &screen,
                post.button,
                post.modifiers,
                handled,
                &mut result,
            );
        }
        result.or(handled)
    }

    /// Scroll/drag Post is a notice to the screen still open after the input.
    pub(super) fn screen_mouse_post(&self, post: ScreenMousePost) {
        use mod_api::HookId;
        let hook = match post {
            ScreenMousePost::Scroll(_) => HookId::ScreenMouseScrolledPost,
            ScreenMousePost::Drag { .. } => HookId::ScreenMouseDraggedPost,
        };
        if !mod_api::__private::listened(hook) {
            return;
        }
        let Some(screen) = self.screen_view_now() else {
            return;
        };
        let modifiers = super::glfw_modifiers(self.modifiers);
        match post {
            ScreenMousePost::Scroll(scroll) => {
                self.native_screen_mouse_scrolled_post(&screen, scroll, modifiers)
            }
            ScreenMousePost::Drag { button, delta } => {
                self.native_screen_mouse_dragged_post(&screen, button, delta, modifiers)
            }
        }
    }
    /// Whether a Post of the key `key` over the open screen cancelled it
    /// (`ScreenEvent.KeyPressed.Post` / `KeyReleased.Post`): asked where the
    /// key no screen arm took reaches the game's own key handling, which a
    /// cancel then skips. False with no subscriber, no screen of the hooks'
    /// open, or a key with no vanilla name.
    pub(super) fn screen_key_post(&self, key: KeyCode, pressed: bool, repeat: bool) -> bool {
        let hook = if pressed {
            mod_api::HookId::ScreenKeyPressedPost
        } else {
            mod_api::HookId::ScreenKeyReleasedPost
        };
        if !mod_api::__private::listened(hook) {
            return false;
        }
        let (Some(name), Some(screen)) = (keybinds::key_name(key), self.screen_view_now()) else {
            return false;
        };
        let modifiers = super::glfw_modifiers(self.modifiers);
        let mut canceled = false;
        if pressed {
            self.native_screen_key_pressed_post(&screen, name, repeat, modifiers, &mut canceled);
        } else {
            self.native_screen_key_released_post(&screen, name, modifiers, &mut canceled);
        }
        canceled
    }
    /// `ScreenEvent.CharacterTyped.Post` for each character of `text` that no
    /// text box took.
    fn screen_chars_post(&self, text: &str) {
        if !mod_api::__private::listened(mod_api::HookId::ScreenCharTypedPost) {
            return;
        }
        let Some(screen) = self.screen_view_now() else {
            return;
        };
        let modifiers = super::glfw_modifiers(self.modifiers);
        for c in text.chars() {
            self.native_screen_char_typed_post(&screen, c, modifiers, &mut false);
        }
    }
    /// A screen the native mods' input hooks are for is open: the window has a
    /// renderer, either the title's own (`title`, `world_select`,
    /// `world_create`, `multiplayer`…) or a screen over the world
    /// (`Screen::Playing` behind it, so not a book, a sign editor, a server
    /// dialog or a prompt drawn over gameplay), and no connect or world load
    /// is in progress. The one gate they all ask before they fire.
    pub(super) fn screen_hooks_open(&self) -> bool {
        self.renderer.is_some()
            && !self.screen_over_playing()
            && !self.remote_loading()
            && (self.screen != Screen::Playing || self.title.is_some())
    }
    /// The open screen as an input hook sees it (`ScreenView`), or None
    /// when `screen_hooks_open` says no screen is. `screen.mouse` is the
    /// pointer the hook's event is at, so a hook takes no position, and
    /// `partial_tick` is the clock's fraction at this event, not a frame's.
    /// The `HudBatch` built here is only what `screen.rect` needs: the
    /// window's GUI scale, no allocation.
    pub(super) fn screen_view_now(&self) -> Option<ScreenView<'_>> {
        if !self.screen_hooks_open() {
            return None;
        }
        let renderer = self.renderer.as_ref()?;
        let batch = self.interface_batch([renderer.config.width, renderer.config.height]);
        if self.screen != Screen::Playing {
            return Some(self.screen_view(&batch, self.screen));
        }
        self.title_screen_view(&batch)
    }
    /// Whether the key press `key` over the open screen went no further
    /// (`native_screen_key_pressed_pre`): what `handle_window_event` and the
    /// control socket's `key` command both ask before a screen acts on a key.
    /// False when no screen of the hooks' is open, and for a key with no
    /// vanilla name.
    pub(super) fn screen_key_pressed(&self, key: KeyCode, repeat: bool) -> bool {
        let Some(name) = keybinds::key_name(key) else {
            return false;
        };
        let Some(screen) = self.screen_view_now() else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_key_pressed_pre(
            &screen,
            name,
            repeat,
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    pub(super) fn screen_key_released(&self, key: KeyCode) -> bool {
        let Some(name) = keybinds::key_name(key) else {
            return false;
        };
        let Some(screen) = self.screen_view_now() else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_key_released_pre(
            &screen,
            name,
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    /// `native_screen_char_typed_pre` for one character, with the focused
    /// box's edit: what the listeners leave of `field` goes back to the box
    /// before the character does. Whether they cancelled the character.
    fn screen_char_canceled(&mut self, c: char) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ScreenCharTypedPre)
            || !self.screen_hooks_open()
        {
            return false;
        }
        let modifiers = super::glfw_modifiers(self.modifiers);
        self.with_focused_text_edit(|game, field| {
            let Some(screen) = game.screen_view_now() else {
                return false;
            };
            let mut canceled = false;
            game.native_screen_char_typed_pre(&screen, c, modifiers, field, &mut canceled);
            canceled
        })
    }
    /// Text on its way to a text box after the open screen's character hooks
    /// (`native_screen_char_typed_pre` per printable character): the cancelled
    /// characters are gone and every other one stays, control characters
    /// included (a text box reads Backspace, Delete and the like as keys).
    /// `None` when none of them is left to type. For a key's text that stays
    /// on the key, which no box with a `field` takes.
    pub(super) fn screen_text_typed(&mut self, text: &str) -> Option<String> {
        screen_text_hooked(text, |c| self.screen_char_canceled(c))
    }
    /// Text into a text box a character at a time (`Screen.charTyped`): each
    /// printable one is offered to `native_screen_char_typed_pre`, and `take`
    /// gets it unless cancelled, before the next is offered. A control
    /// character goes straight to `take`: a text box reads it as a key.
    pub(super) fn screen_text_each(&mut self, text: &str, mut take: impl FnMut(&mut Self, &str)) {
        let mut encoded = [0; 4];
        for c in text.chars() {
            if c.is_control() || !self.screen_char_canceled(c) {
                take(self, c.encode_utf8(&mut encoded));
            }
        }
    }
    /// A key press's characters (`ScreenEvent.CharacterTyped.Pre`, vanilla's
    /// `KeyboardHandler.charTyped`), dispatched once the screen has acted on
    /// the key: GLFW's char callback comes after its key callback, so the
    /// characters go to the screen that is open now — a key that closed the
    /// screen leaves none to hear them, as vanilla's closed screen hears
    /// nothing, and one that moved the focus types into the box it moved to.
    /// Each printable one is offered to `native_screen_char_typed_pre`; the
    /// survivors go to the focused text box, control characters (keys) gone.
    pub(super) fn screen_chars_typed(&mut self, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        // A held edit modifier makes a shortcut of the key, not text.
        let text = chars_to_type(text, self.types_text());
        self.screen_committed_chars(&text);
    }

    /// Characters already committed by the input method or the control socket are not shortcuts.
    pub(in crate::game) fn screen_committed_chars(&mut self, text: &str) {
        if !self.screen_hooks_open() {
            return;
        }
        self.screen_text_each(text, |game, c| {
            if !c.starts_with(char::is_control) && game.type_chars_into_screen(c).is_err() {
                game.screen_chars_post(c);
            }
        });
    }
    pub(super) fn observe_client_packet(
        &mut self,
        packet: &sunrise_protocol::packets::play::Clientbound,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::ClientPacket) {
            return;
        }
        let view = client_packet_view(packet);
        self.native_client_packet(&view);
    }
    /// Whether a mod cancelled `packet` on its way to the server (`client_packet_send`).
    /// Builds the packet's view only when a mod listens.
    pub(super) fn client_packet_send_canceled(
        &self,
        packet: &sunrise_protocol::packets::play::Serverbound,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ClientPacketSend) {
            return false;
        }
        let mut canceled = false;
        self.native_client_packet_send(&serverbound_packet_view(packet), &mut canceled);
        canceled
    }
    /// The living entity an effect decision names: `Some(id)` a mob's
    /// `Entity.getId`, `None` the acting player. `None` when no native
    /// world runs, and the decision falls back to vanilla's.
    pub(super) fn effect_living_ref(&self, entity: Option<i32>) -> Option<LivingRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let id = match entity {
            Some(id) => id as u32,
            None => self.own_entity_id() as u32,
        };
        Some(LivingRef::new(EntityRef::new(epoch, id)))
    }
    /// Deliver what the batch just ticked recorded, in order: `minecart_pass`
    /// to the parts, and the writes a landing, step, or mid-walk `entity_inside`
    /// contact asked for to the level. A write is skipped when the block at
    /// its position is no longer the state it touched, as vanilla never sees
    /// a stale one. The parts' own `entity_inside` contacts fire mid-walk now,
    /// on the shared inside sweep, not here.
    pub(super) fn drain_entity_inside(&mut self) {
        for contact in sunrise_client::block_contact::take_pending() {
            let (state, pos) = match &contact {
                Pending::Landed(landed) => (landed.state, landed.position),
                Pending::MinecartPass(pass) => (pass.state, pass.position),
                Pending::InsideWritten(written) => (written.state, written.position),
            };
            let Some(level) = self.level(self.dim.id()) else {
                continue;
            };
            if self.level_block_state(level, [pos.x, pos.y, pos.z]) != Some(state) {
                continue;
            }
            match contact {
                Pending::MinecartPass(pass) => {
                    if let Some(entity) = u32::try_from(pass.entity)
                        .ok()
                        .and_then(|id| self.entity(id))
                    {
                        self.native_minecart_pass(state, pos, entity);
                    }
                }
                Pending::Landed(landed) => {
                    for write in landed.writes {
                        self.apply_landing_write(level, pos, write);
                    }
                }
                // The mid-walk contacts' own level writes, guarded on the touched
                // state above, as a landing's are.
                Pending::InsideWritten(written) => {
                    for write in written.writes {
                        self.apply_inside_write(level, pos, write);
                    }
                }
            }
        }
    }

    /// Apply one already-validated engine write, at the block the entity landed on.
    /// The original-state guard runs once before the ordered batch, not between writes.
    fn apply_landing_write(
        &mut self,
        level: mod_api::LevelRef,
        pos: BlockPos,
        write: sunrise_world::level::LandingWrite,
    ) {
        let at = [pos.x, pos.y, pos.z];
        match write {
            // `Level.setBlockAndUpdate`: flag 3, the neighbours and the clients.
            sunrise_world::level::LandingWrite::SetBlock(state) => {
                let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
                let _ = self.level_set_block(level, at, state, flags);
            }
            sunrise_world::level::LandingWrite::DestroyBlock { drop } => {
                self.level_destroy_block(level, at, drop, None, false);
            }
            // A write a later API adds that this game does not know is not applied.
            _ => {}
        }
    }
    /// A write an `entity_inside` part asked for, applied as a landing write is: at the
    /// touched block, once the entity's batch has ticked.
    fn apply_inside_write(
        &mut self,
        level: mod_api::LevelRef,
        pos: BlockPos,
        write: sunrise_world::level::InsideWrite,
    ) {
        let at = [pos.x, pos.y, pos.z];
        match write {
            sunrise_world::level::InsideWrite::SetBlock(state) => {
                let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
                let _ = self.level_set_block(level, at, state, flags);
            }
            sunrise_world::level::InsideWrite::DestroyBlock { drop } => {
                self.level_destroy_block(level, at, drop, None, false);
            }
            // A write a later API adds that this game does not know is not applied.
            _ => {}
        }
    }
}

/// `native_living_use_totem` for the damage code: whether it
/// canceled the totem. The entity and the source are named as the damage hook names them.
pub(super) fn use_totem(level: DamageLevel<'_>, hurting: Hurting<'_>, off_hand: bool) -> bool {
    if !mod_api::__private::listened(mod_api::HookId::LivingUseTotem) {
        return false;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let hand = if off_hand { Hand::Off } else { Hand::Main };
    let mut cancel = false;
    HitLevel(level).native_living_use_totem(
        entity,
        kind,
        &source,
        hand,
        "minecraft:totem_of_undying",
        &mut cancel,
    );
    cancel
}

/// The level a hit or a landing happens in, as its hooks' level host: its blocks, its random and its
/// own attachments. Not `GameLevel`, which a hit can't build: a mob's hit lands in the mob
/// tick, where the world is read by every mob and its random is lent out, and building one
/// per hit would allocate its entity boxes.
pub(super) struct HitLevel<'a>(pub(super) DamageLevel<'a>);

/// Let listeners override the vanilla invulnerability decision for this hit.
pub(super) fn invulnerability_check(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    vanilla: bool,
) -> bool {
    if !mod_api::__private::listened(mod_api::HookId::EntityInvulnerabilityCheck) {
        return vanilla;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut invulnerable = vanilla;
    HitLevel(level).native_entity_invulnerability_check(
        entity,
        kind,
        &source,
        vanilla,
        &mut invulnerable,
    );
    invulnerable
}

fn engine_reduction(reduction: mod_api::Reduction) -> sunrise_world::living_damage::Reduction {
    match reduction {
        mod_api::Reduction::Invulnerability => {
            sunrise_world::living_damage::Reduction::Invulnerability
        }
        mod_api::Reduction::Armor => sunrise_world::living_damage::Reduction::Armor,
        mod_api::Reduction::Enchantments => sunrise_world::living_damage::Reduction::Enchantments,
        mod_api::Reduction::MobEffects => sunrise_world::living_damage::Reduction::MobEffects,
        mod_api::Reduction::Absorption => sunrise_world::living_damage::Reduction::Absorption,
        mod_api::Reduction::InnateResistance => {
            sunrise_world::living_damage::Reduction::InnateResistance
        }
        _ => unreachable!("unknown API reduction"),
    }
}

struct LiveDamageRead<'a>(&'a sunrise_world::living_damage::DamageState);

impl mod_api::__private::DamageContainerRead for LiveDamageRead<'_> {
    fn original(&self) -> f32 {
        self.0.original()
    }
    fn new_damage(&self) -> f32 {
        self.0.new_damage()
    }
    fn blocked(&self) -> f32 {
        self.0.blocked()
    }
    fn shield_damage(&self) -> f32 {
        self.0.shield_damage()
    }
    fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.0.post_attack_invulnerability_ticks()
    }
    fn should_cause_side_effects(&self) -> bool {
        self.0.should_cause_side_effects()
    }
    fn inflicted(&self) -> f32 {
        self.0.inflicted()
    }
    fn health_damage(&self) -> f32 {
        self.0.health_damage()
    }
    fn reduction(&self, reduction: mod_api::Reduction) -> f32 {
        self.0.reduction(engine_reduction(reduction))
    }
}

struct LiveDamageContainer<'a>(&'a mut DamageContainer);

impl mod_api::__private::DamageContainerRead for LiveDamageContainer<'_> {
    fn original(&self) -> f32 {
        self.0.original()
    }
    fn new_damage(&self) -> f32 {
        self.0.new_damage()
    }
    fn blocked(&self) -> f32 {
        self.0.blocked()
    }
    fn shield_damage(&self) -> f32 {
        self.0.shield_damage()
    }
    fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.0.post_attack_invulnerability_ticks()
    }
    fn should_cause_side_effects(&self) -> bool {
        self.0.should_cause_side_effects()
    }
    fn inflicted(&self) -> f32 {
        self.0.inflicted()
    }
    fn health_damage(&self) -> f32 {
        self.0.health_damage()
    }
    fn reduction(&self, reduction: mod_api::Reduction) -> f32 {
        self.0.reduction(engine_reduction(reduction))
    }
}

impl mod_api::__private::DamageContainerBackend for LiveDamageContainer<'_> {
    fn set_new_damage(&mut self, value: f32) {
        self.0.record_new_damage(value);
    }
    fn set_post_attack_invulnerability_ticks(&mut self, value: i32) {
        self.0.record_post_attack_invulnerability_ticks(value);
    }
    fn add_reduction_modifier(
        &mut self,
        reduction: mod_api::Reduction,
        owner: Option<&'static str>,
        modifier: Box<dyn FnMut(mod_api::DamageContainerView<'_>, f32) -> f32>,
    ) {
        let mut wrapped = mod_api::__private::wrap_reduction_modifier(owner, modifier);
        self.0.add_reduction_modifier(
            engine_reduction(reduction),
            Box::new(move |engine_container: &DamageContainer, amount: f32| {
                wrapped(
                    mod_api::DamageContainerView::new(&LiveDamageRead(engine_container)),
                    amount,
                )
            }),
        );
    }
}

/// `native_living_incoming_damage` for the damage code: whether it
/// canceled the hit.
pub(super) fn incoming_damage(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    victim: Victim,
    container: &mut DamageContainer,
) -> bool {
    let listens_incoming = mod_api::__private::listened(mod_api::HookId::LivingIncomingDamage);
    if listens_incoming
        || mod_api::__private::listened(mod_api::HookId::LivingDamagePre)
        || mod_api::__private::listened(mod_api::HookId::LivingDamagePost)
    {
        container.enable_live_reductions();
    }
    if !listens_incoming {
        return false;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut canceled = false;
    let attack = level.attack();
    let Ok(weapon) = attack.weapon_of(source.direct).try_into_api() else {
        return false;
    };
    let causing_weapon = (source.causing != 0 && source.causing == attack.attacker)
        .then_some(attack.held[0].as_ref())
        .flatten();
    let Ok(causing_weapon) = causing_weapon.try_into_api() else {
        return false;
    };
    let mut live = LiveDamageContainer(container);
    HitLevel(level).native_living_incoming_damage(mod_api::__private::LivingIncomingDamageCall {
        entity,
        kind,
        source: &source,
        victim: &victim,
        weapon: weapon.as_ref(),
        causing_weapon: causing_weapon.as_ref(),
        container: mod_api::DamageContainer::new(&mut live),
        canceled: &mut canceled,
    });
    canceled
}

/// `native_living_damage_pre` for the damage code.
pub(super) fn damage_pre(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    container: &mut DamageContainer,
) {
    if !mod_api::__private::listened(mod_api::HookId::LivingDamagePre) {
        return;
    }
    container.enable_live_reductions();
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut live = LiveDamageContainer(container);
    HitLevel(level).native_living_damage_pre(mod_api::__private::LivingDamagePreCall {
        entity,
        kind,
        source: &source,
        container: mod_api::DamageContainer::new(&mut live),
    });
}

/// `native_living_damage_post` for the damage code.
pub(super) fn damage_post(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    container: &DamageContainer,
) {
    if !mod_api::__private::listened(mod_api::HookId::LivingDamagePost) {
        return;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let live = LiveDamageRead(container);
    HitLevel(level).native_living_damage_post(
        entity,
        kind,
        &source,
        mod_api::DamageContainerView::new(&live),
    );
}

/// `native_living_shield_block` for the damage code. Its one
/// caller. The hurt entity and the source are named as the damage hook names them.
pub(super) fn shield_block(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    damage: f32,
    original_blocked: bool,
    block: &mut ShieldBlock,
) {
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    HitLevel(level).native_living_shield_block(
        entity,
        kind,
        &source,
        damage,
        original_blocked,
        block,
    );
}

impl Game<'_> {
    /// Finish a lethal or totem-protected hit only once the owning level has settled it.
    pub(in crate::game) fn finish_damage_post(
        &mut self,
        entity: i32,
        kind: &str,
        post: std::sync::Arc<sunrise_client::health::DamagePost>,
    ) {
        let (source, state, original_lend) = post.view();
        let (player, mobs) = (&mut self.player, &mut self.mobs);
        let lend = original_lend.and_then(|id| {
            sunrise_client::health::HitLend::attacking_player(player.as_mut(), id)
                .or_else(|| sunrise_client::health::HitLend::attacking_mob(mobs.iter_mut(), id))
        });
        let level = crate::game::entity::damage_level_of!(
            self,
            &sunrise_client::health::NO_ATTACK,
            no_push,
            lend
        );
        let live = LiveDamageRead(state);
        HitLevel(level.reborrow()).native_living_damage_post(
            entity,
            kind,
            &source,
            mod_api::DamageContainerView::new(&live),
        );
    }

    /// The [`Attack`] of `attacker` (an entity id, 0 for none) for the level a hit lands in
    /// ([`DamageLevel::attacked_by`]). While a native mod listens it also records what the
    /// attacker holds (the player's selected and off-hand stacks, a mob's main hand) for
    /// `living_knockback` and the weapon of `living_incoming_damage`; without one it is the
    /// id alone, two `None`s and no allocation. Taken before the level's borrows of the game.
    pub(super) fn attack_of(&self, attacker: i32) -> Attack {
        let held = if mod_api::__private::mods().is_empty() {
            [None, None]
        } else if self
            .player
            .as_ref()
            .is_some_and(|p| p.entity_id == attacker)
        {
            [self.stack_in(self.selected).cloned(), self.offhand.clone()]
        } else if let Some(mob) = self.mobs.iter().find(|m| m.id as i32 == attacker) {
            [mob.extra.main_hand_stack().cloned(), None]
        } else if let Some(guest) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.entity_id == attacker))
        {
            [
                guest.inventory.get(guest.selected).cloned().flatten(),
                guest.offhand.clone(),
            ]
        } else {
            [None, None]
        };
        let player_uuid = if self
            .player
            .as_ref()
            .is_some_and(|p| p.entity_id == attacker)
        {
            self.player_uuid_of(self.lan_acting)
        } else {
            self.lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.entity_id == attacker))
                .map(|g| sunrise_client::lan::playerdata::uuid_ints(g.uuid))
        };
        Attack {
            attacker,
            held,
            projectile: None,
            projectile_kind: sunrise_client::health::DirectEntity::Other,
            direct: None,
            direct_kind: None,
            position: None,
            projectile_weapon: None,
            player_uuid,
        }
    }
    /// [`Self::attack_of`] for the attacker a damage source names: the player, a mob, or (none)
    /// the attack the enclosing [`Self::with_attack`] lends.
    pub(super) fn attack_of_who(&self, attacker: Option<Who>) -> Attack {
        match attacker {
            Some(Who::Player) => self.attack_of(self.player.as_ref().map_or(0, |p| p.entity_id)),
            Some(Who::Mob(id)) => self.attack_of(id as i32),
            None => self.current_attack(),
        }
    }
}

/// `native_living_knockback` for the knockback code: whether it
/// canceled the push. Its one caller. The pushed entity and the source are named as the
/// damage hook names them.
pub(super) fn knockback(level: DamageLevel<'_>, pushed: Hurting<'_>, knock: &mut Knock) -> bool {
    if !mod_api::__private::listened(mod_api::HookId::LivingKnockback) {
        return false;
    }
    let attack = level.attack();
    let attacker = match &pushed {
        Hurting::Mob(mob, damage) => match damage.attacker.or(mob.ai.last_hurt_by) {
            Some(Who::Mob(id)) => id as i32,
            _ => attack.attacker,
        },
        Hurting::Player(_, source)
        | Hurting::Dragon(_, source)
        | Hurting::ArmorStand(_, source)
        | Hurting::Entity(_, _, source) => match source.causing {
            0 => attack.attacker,
            causing => causing,
        },
    };
    let (kill_credit, credit_main) = level.knockback_credit(&pushed);
    let Ok(credit_main) = credit_main.map(TryIntoApi::try_into_api).transpose() else {
        return false;
    };
    let (entity, kind, source) = hurt_server_args(pushed, level.attack());
    let mut canceled = false;
    let [main, off] = &attack.held;
    let (Ok(main), Ok(off)) = (main.try_into_api(), off.try_into_api()) else {
        return false;
    };
    let entry = level.knockback_entry();
    HitLevel(level).native_living_knockback(mod_api::__private::LivingKnockbackCall {
        entity,
        kind,
        source: &source,
        attacker,
        attacker_main: main.as_ref(),
        attacker_off: off.as_ref(),
        kill_credit,
        kill_credit_main: credit_main.as_ref(),
        entry,
        strength: &mut knock.strength,
        x: &mut knock.x,
        z: &mut knock.z,
        canceled: &mut canceled,
    });
    canceled
}

/// The damage hooks every level of the game lends to its hits (`Game.damage_hooks`).
pub(super) const DAMAGE_HOOKS: DamageHooks = DamageHooks {
    invulnerable: Some(invulnerability_check),
    incoming: Some(incoming_damage),
    pre: Some(damage_pre),
    post: Some(damage_post),
    owner_post_listened: Some(|| mod_api::__private::listened(mod_api::HookId::LivingDamagePost)),
    knockback: Some(knockback),
    totem: Some(use_totem),
    shield: Some(shield_block),
    accepted: Some(living_death::capture_hit),
};
/// The contacts of a simulating entity with a block, as the simulation's own code (client-lib)
/// reports them.
pub(super) const BLOCK_CONTACT_HOOKS: sunrise_client::block_contact::Hooks =
    sunrise_client::block_contact::Hooks {
        step_listens: |state| {
            crate::game::api_values::block_state::block_listens(mod_api::HookId::StepOn, state)
        },
        step_on: |level, step, movement| {
            use self::bounce_restitution::StepOnView;
            use sunrise_client::block_contact::StepLevel;
            match level {
                StepLevel::Server(level) => StepOnView(mod_api::LevelRandomSide::Server(
                    &mut HitLevel(level.reborrow()),
                ))
                .native_step_on(step, movement),
                StepLevel::Client(level) => StepOnView(mod_api::LevelRandomSide::Client(
                    &crate::game::native_surface::FrictionView(level),
                ))
                .native_step_on(step, movement),
            }
        },
        any_inside: || mod_api::__private::has_scoped_hook(mod_api::HookId::EntityInside),
        inside_listens: |state| {
            crate::game::api_values::block_state::block_listens(
                mod_api::HookId::EntityInside,
                state,
            )
        },
        pass_listens: |state| {
            crate::game::api_values::block_state::block_listens(
                mod_api::HookId::MinecartPass,
                state,
            )
        },
        fall_listens: |state| {
            crate::game::api_values::block_state::block_listens(mod_api::HookId::FallOn, state)
        },
        bounce_listens: |state| {
            crate::game::api_values::block_state::block_listens(mod_api::HookId::BounceOn, state)
        },
        inside_fire: |level, dimension, entity, kind, state, pos, precise, staged, writes| {
            // The whole callback is staged: a panic is contained and the walk
            // keeps the caller's collector and write list as they were (#2247's
            // rule: a mod's panic never breaks the model).
            let fired = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                HitLevel(level.reborrow()).native_entity_inside(
                    crate::game::native_hooks::fire::EntityInsideCall {
                        dimension,
                        entity,
                        kind,
                        state,
                        pos,
                        precise,
                        effects: staged,
                        writes,
                    },
                );
            }));
            if let Err(payload) = fired {
                if let Some(text) = payload.downcast_ref::<&str>() {
                    eprintln!("an entity_inside part panicked at {text:?}");
                } else if let Some(text) = payload.downcast_ref::<String>() {
                    eprintln!("an entity_inside part panicked at {text:?}");
                }
            }
        },
        fall_on: |dimension, level, c, distance, damage_multiplier, writes| {
            HitLevel(level.reborrow()).native_fall_on(
                crate::game::native_hooks::fire::FallOnCall {
                    dimension,
                    entity: c.entity,
                    kind: c.kind,
                    state: c.state,
                    pos: c.position,
                    distance,
                    damage_multiplier,
                    writes,
                },
            );
        },
        bounce_on: |dimension, level, c, fall_distance, movement| {
            let mut answer = [movement.x, movement.y, movement.z];
            HitLevel(level.reborrow()).native_bounce_on(
                crate::game::native_hooks::fire::BounceOnCall {
                    dimension,
                    entity: c.entity,
                    kind: c.kind,
                    state: c.state,
                    pos: c.position,
                    fall_distance,
                    movement: &mut answer,
                },
            );
            *movement = sunrise_core::geometry::Vec3::new(answer[0], answer[1], answer[2]);
        },
    };
pub(super) use living_death::hurt_server_args;
pub(super) use sunrise_client::health::mob_source;

/// Whether a native mod's particles can be sent. Block particle types need
/// a known state (`state_ok`), item particles a validated stack; trail and vibration
/// aren't supported by the network dispatch. A mod's own type must be namespaced.
pub(super) fn mod_particles_ok(
    options: &ParticleOptions,
    state: Option<BlockStateId>,
    state_ok: impl Fn(BlockStateId) -> bool,
    at: [f64; 3],
    spread: [f64; 3],
    speed: f64,
) -> bool {
    use ParticleType as T;
    // A native mod's own type: the name has to be namespaced, and a client
    // draws it only when its pack has the sprite set.
    if let Some(name) = &options.mod_particle {
        let namespaced = name
            .split_once(':')
            .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty());
        if !namespaced {
            return false;
        }
    }
    let supported = match options.ty {
        T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
            state.is_some_and(state_ok)
        }
        T::Item | T::Trail | T::Vibration => crate::game::item_particle_send::supported(options),
        T::Geyser | T::GeyserPlume | T::GeyserBase | T::GeyserPoof => options
            .geyser
            .is_some_and(|(_, impulse)| impulse.is_finite()),
        _ => true,
    };
    supported
        && at.into_iter().all(f64::is_finite)
        && spread
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= f32::MAX as f64)
        && speed.is_finite()
        && speed.abs() <= f32::MAX as f64
        && options.color.into_iter().all(f32::is_finite)
        && options.to_color.into_iter().all(f32::is_finite)
        && options.scale.is_finite()
}

fn dimension_id(dimension: LevelDimension) -> &'static str {
    match dimension {
        LevelDimension::Overworld => "minecraft:overworld",
        LevelDimension::Nether => "minecraft:the_nether",
        LevelDimension::End => "minecraft:the_end",
        LevelDimension::Custom(key) => key.id(),
    }
}

/// A mob's kind as the hook points name it (`minecraft:zombie`).
pub(super) fn mob_kind(mob: &sunrise_client::mobs::Mob) -> String {
    crate::game::native_namespaced_id::namespaced(&mob.kind.id)
}

/// The level behaviour's answer, or `None` to try `useWithoutItem`.
/// `ServerPlayerGameMode.useItemOn` lets both FAIL and PASS go to the item.
/// Both success swing sources swing on this server path.
pub(super) fn block_use_result(
    answer: mod_api::InteractionResult,
) -> Option<sunrise_world::level::InteractionResult> {
    use sunrise_world::level::InteractionResult;
    match answer {
        mod_api::InteractionResult::Success | mod_api::InteractionResult::SuccessServer => {
            Some(InteractionResult::Success)
        }
        mod_api::InteractionResult::Consume => Some(InteractionResult::Consume),
        mod_api::InteractionResult::TryWithEmptyHand => None,
        _ => Some(InteractionResult::Pass),
    }
}

/// What the mods answered a right click on a block (`right_click_block`).
pub(super) enum BlockClickAnswer {
    /// Cancelled: this answers the click instead of the block and the item.
    Canceled(mod_api::InteractionResult),
    /// The block's use and the item's `useOn` run as these say.
    Go {
        use_block: TriState,
        use_item: TriState,
    },
}

/// What `BonemealEvent` settled for one use of bone meal.
pub(super) struct BonemealAsk {
    /// `Some(successful)` when a mod canceled the event: `growCrop`'s answer.
    pub crop: Option<bool>,
    /// `isValidBonemealTarget` as the event asked it, when a mod listens:
    /// `growCrop` takes it instead of asking again.
    pub check: Option<BonemealCheck>,
}

/// Where the game calls the hook points.
impl Game<'_> {
    /// `BonemealEvent` for bone meal from `stack` on `pos`, used by the
    /// acting player (`by_player`) or by a dispenser or a villager:
    /// `Some(successful)` when a mod canceled it, the answer `growCrop`
    /// gives in vanilla's place. The caller keeps what the mods did to
    /// `stack`.
    pub(super) fn bonemeal(
        &mut self,
        pos: BlockPos,
        by_player: bool,
        stack: &mut ItemStack,
    ) -> BonemealAsk {
        if !mod_api::__private::listened(mod_api::HookId::Bonemeal) {
            return BonemealAsk {
                crop: None,
                check: None,
            };
        }
        let client = self.remote.is_some();
        let check = self.with_level(|b, level, _| b.is_valid_bonemeal_target(level, pos, client));
        let valid = check.is_valid();
        let state = self.world.block(pos.x, pos.y, pos.z);
        let (player, player_entity_id) = if by_player {
            let (uuid, entity) = self.native_clicker();
            (Some(uuid), Some(entity))
        } else {
            (None, None)
        };
        let (mut successful, mut canceled) = (false, false);
        let mut snapshot = Some(stack.clone());
        self.native_bonemeal(crate::game::native_hooks::fire::BonemealCall {
            dimension: self.dim.level(),
            player,
            player_entity_id,
            pos,
            state,
            source: BonemealSource::Interaction,
            stack: &mut snapshot,
            valid_bonemeal_target: valid,
            successful: &mut successful,
            canceled: &mut canceled,
        });
        match snapshot {
            Some(next) => *stack = next,
            None => stack.count = 0,
        }
        BonemealAsk {
            crop: canceled.then_some(successful),
            check: Some(check),
        }
    }
    /// Whether a mod canceled `placer` writing `state` at `pos` (`entity_place`):
    /// `placer` is an entity id, `placed_against` the block it goes against and
    /// `companion` the second half of a bed, door or tall plant.
    pub(super) fn place_canceled(
        &mut self,
        placer: u32,
        pos: BlockPos,
        state: BlockStateId,
        placed_against: BlockStateId,
        companion: Option<(BlockPos, BlockStateId)>,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::EntityPlace) {
            return false;
        }
        self.placement_canceled(self.entity(placer), pos, state, placed_against, companion)
    }

    fn placement_canceled(
        &mut self,
        placer: Option<EntityRef>,
        pos: BlockPos,
        state: BlockStateId,
        placed_against: BlockStateId,
        extra: impl IntoIterator<Item = (BlockPos, BlockStateId)>,
    ) -> bool {
        let mut extra = extra.into_iter();
        let placement = if let Some(first) = extra.next() {
            let Some(writes) = std::iter::once((pos, state))
                .chain(std::iter::once(first))
                .chain(extra)
                .map(|(at, state)| Some(BlockPlacement::new(at, state.try_into_api().ok()?)))
                .collect()
            else {
                return false;
            };
            Placement::Multi(writes)
        } else {
            Placement::Single
        };
        let mut canceled = false;
        self.native_entity_place(placer, pos, state, placed_against, placement, &mut canceled);
        canceled
    }
    /// Let `entity_place` veto exactly the decision the world will apply. If the
    /// callback changed the inputs, do not choose another target behind its back.
    pub(super) fn place_item_block(
        &mut self,
        ctx: &PlaceContext,
        by_player: bool,
        stack: &ItemStack,
    ) -> crate::Result<Option<sunrise_world::level::AppliedPlacement>> {
        let registry = self.pack.assets.registry;
        let game_master = ctx.can_use_game_master_blocks;
        let stack_state =
            |state| crate::game::block_items::item_block_state(registry, state, stack, game_master);
        let has_part =
            mod_api::__private::item_listens(mod_api::HookId::PlacementContext, &stack.item);
        let placer = (has_part && by_player)
            .then(|| self.placement_placer())
            .flatten();
        let Some((decision, preview)) = self.with_level(|b, level, ticks| {
            b.decide_placement(level, ticks, ctx, stack_state, |level, context| {
                has_part
                    .then(|| level.rewrite_placement_context(stack, context, placer))
                    .flatten()
            })
        }) else {
            return Ok(None);
        };
        let veto = mod_api::__private::listened(mod_api::HookId::EntityPlace);
        if veto {
            let against_pos = decision.placed_against_pos();
            let block = |game: &Game<'_>, pos: BlockPos| game.world.block(pos.x, pos.y, pos.z);
            let against = block(self, against_pos);
            let target_before = block(self, preview.pos);
            // One proposed state per affected cell. A companion at the pre-clear
            // position supersedes the intermediate air or water write.
            let extra = preview.companion.into_iter().chain(
                preview
                    .pre_clear
                    .filter(|(pos, _)| preview.companion.is_none_or(|(at, _)| at != *pos)),
            );
            let hand = if by_player {
                self.held().cloned()
            } else {
                None
            };
            let placer = by_player
                .then(|| self.native_clicker().1.cast_unsigned())
                .and_then(|entity| self.entity(entity));
            // The block revision and journal epoch cover supporting blocks and
            // block-entity shapes without tracing ordinary reads.
            let level = (self.dim, self.world.revision, self.transactions.writes());
            if self.placement_canceled(placer, preview.pos, preview.state, against, extra)
                || level != (self.dim, self.world.revision, self.transactions.writes())
                || block(self, preview.pos) != target_before
                || block(self, against_pos) != against
                || (by_player && self.held() != hand.as_ref())
            {
                return Ok(None);
            }
        }
        let Some(placed) = self
            .with_level(|b, level, ticks| b.apply_placement(level, ticks, decision, stack_state))
        else {
            return Ok(None);
        };
        if placed.matches_candidate {
            let pos = placed.context.pos;
            let target = [pos.x, pos.y, pos.z];
            self.apply_block_item_data(target, placed.state, stack, game_master);
            self.with_level(|b, level, ticks| b.placement_set_placed_by(level, ticks, &placed));
            let placer = by_player.then(|| self.native_acting_player()).flatten();
            self.set_placed_by(target, placer, Some(stack))?;
            self.block_item_placed(stack, placer, &placed.context);
            if by_player {
                self.trigger_block("minecraft:placed_block", target, stack);
            }
        }
        self.with_level(|b, level, ticks| b.report_placement(level, ticks, &placed));
        Ok(Some(placed))
    }
    /// `MobEvent::PlaceBlock`: a mob sets `state` at `pos` unless a mod
    /// cancels `entity_place`. Enderman attempts use `enderman_place_block`.
    pub(super) fn mob_place_block(
        &mut self,
        mob: u32,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> crate::Result<()> {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let against = self.world.block(pos[0], pos[1] - 1, pos[2]);
        let kind = self
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map(|m| m.kind.id.clone());
        if self.place_canceled(mob, at, state, against, None) {
            return Ok(());
        }
        let facts = self.res.facts();
        let from = facts
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let to = facts.get(state).name.clone();
        self.level_set(pos, state)?;
        if let Some(kind) = kind {
            self.native_entity_change_block(mob, &kind, pos, &from, &to);
        }
        Ok(())
    }
    /// The player clicking, as the block-use hooks name them: the acting
    /// player's UUID and entity id on a server, this client's own player on
    /// a client.
    pub(super) fn native_clicker(&mut self) -> (u128, i32) {
        if self.remote.is_some() {
            return (self.own_uuid(), self.own_entity_id());
        }
        let entity = self
            .lan_acting_entity
            .map_or_else(|| self.own_entity_id(), |id| id as i32);
        (self.native_acting_id(), entity)
    }
    /// `PlayerInteractEvent.RightClickBlock` for a click on `hit` with
    /// `hand` holding `stack`: what the mods let it use.
    pub(super) fn right_click_block(
        &mut self,
        hit: &super::Hit,
        hand: Hand,
        stack: Option<&ItemStack>,
    ) -> BlockClickAnswer {
        let (mut use_block, mut use_item) = (TriState::Default, TriState::Default);
        if !mod_api::__private::listened(mod_api::HookId::RightClickBlock) {
            return BlockClickAnswer::Go {
                use_block,
                use_item,
            };
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return BlockClickAnswer::Go {
                use_block,
                use_item,
            };
        };
        let click = self.block_click(hit, hand, snapshot.as_ref());
        let (mut result, mut canceled) = (mod_api::InteractionResult::Pass, false);
        self.native_right_click_block(
            &click,
            &mut use_block,
            &mut use_item,
            &mut result,
            &mut canceled,
        );
        if canceled {
            BlockClickAnswer::Canceled(result)
        } else {
            BlockClickAnswer::Go {
                use_block,
                use_item,
            }
        }
    }
    /// The chosen phase's broadcast event, before its scoped behaviour.
    pub(super) fn use_item_on_block(
        &mut self,
        hit: &super::Hit,
        hand: Hand,
        stack: Option<&ItemStack>,
        phase: UsePhase,
    ) -> Option<mod_api::InteractionResult> {
        if !mod_api::__private::listened(mod_api::HookId::UseItemOnBlock) {
            return None;
        }
        let snapshot = stack.try_into_api().ok()?;
        let click = self.block_click(hit, hand, snapshot.as_ref());
        let (mut result, mut canceled) = (mod_api::InteractionResult::TryWithEmptyHand, false);
        self.native_use_item_on_block(&click, phase, &mut result, &mut canceled);
        if !canceled {
            return None;
        }
        if phase == UsePhase::ItemAfterBlock
            && result.consumes_action()
            && let Some(stack) = stack
        {
            self.with_hand(hand, |game| {
                if game.remote.is_none() {
                    game.award_stat("used", &stack.item, 1);
                }
                game.apply_after_use_components(stack);
            });
        }
        Some(result)
    }
    /// `ItemStack.onItemUseFirst` for a click on a block: the broadcast
    /// event's `ItemBeforeBlock` phase, then the held item's Part. Canceling skips that Part.
    /// Only `Pass` continues to the block.
    pub(super) fn item_use_first(
        &mut self,
        hit: &super::Hit,
        hand: Hand,
        stack: &ItemStack,
    ) -> Option<mod_api::InteractionResult> {
        if let Some(answer) =
            self.use_item_on_block(hit, hand, Some(stack), UsePhase::ItemBeforeBlock)
        {
            return Some(answer);
        }
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnItemUseFirst) {
            return None;
        }
        let snapshot = stack.try_into_api().ok()?;
        let click = self.block_click(hit, hand, Some(&snapshot));
        let mut answer = None;
        self.native_on_item_use_first(&snapshot, &click, &mut answer);
        answer.filter(|answer| *answer != mod_api::InteractionResult::Pass)
    }
    /// `GameRuleChangedEvent` once the server set the rule `name` to `value`
    /// (`GameRules.set` with a server).
    pub(super) fn game_rule_changed(
        &mut self,
        name: &str,
        value: GameRuleValue,
        actor: Option<&GameRuleActor<'_>>,
    ) {
        if mod_api::__private::listened(mod_api::HookId::GameRuleChanged) {
            self.native_game_rule_changed(name, value, actor);
        }
    }
    /// `AnimalTameEvent` for the mob at `index`, whose taming roll by the
    /// player `tamer` (`tamer_entity_id`) succeeded: true when a mod cancels
    /// it, and the taming fails.
    pub(super) fn animal_tame_cancelled(
        &mut self,
        index: usize,
        tamer: u128,
        tamer_entity_id: i32,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::AnimalTame) {
            return false;
        }
        let m = &self.mobs[index];
        let (entity, kind) = (m.id, mob_kind(m));
        let mut cancel = false;
        self.native_animal_tame(tamer, tamer_entity_id, entity, &kind, &mut cancel);
        cancel
    }
    /// `entity_tame` for the pet at `index`, just tamed by its owner.
    pub(super) fn pet_tamed(&mut self, index: usize) {
        let m = &self.mobs[index];
        let (entity, kind) = (m.id, mob_kind(m));
        let owner = m
            .extra
            .owner
            .map(sunrise_client::lan::playerdata::ints_uuid);
        self.native_entity_tame(entity, &kind, owner);
    }
    /// The acting player as the hook points name one: the guest whose turn
    /// it is, else the host.
    pub(super) fn native_acting_id(&mut self) -> u128 {
        sunrise_client::lan::playerdata::ints_uuid(self.acting_player_uuid())
    }

    /// Tells the mods of the chunks that came into this level.
    pub(super) fn native_chunks_loaded(&mut self) {
        self.fire_pending_block_entity_data_loads();
        let loaded = std::mem::take(&mut self.world.loaded_chunks);
        if self.remote.is_some() {
            return;
        }
        let dimension = self.dim.id();
        for (x, z, new) in loaded {
            self.native_chunk_load(dimension, [x, z], new);
        }
    }
    /// Tells the mods of the chunks leaving this level, after their last save.
    pub(super) fn native_chunks_unloaded(
        &mut self,
        unloaded: &[std::sync::Arc<sunrise_client::chunk::Chunk>],
    ) {
        if self.remote.is_some() {
            return;
        }
        let dimension = self.dim.id();
        for chunk in unloaded {
            self.native_chunk_and_block_entities_unload(dimension, [chunk.x, chunk.z]);
        }
    }
    /// The players still here leave with the server (`PlayerList.removeAll`):
    /// the mods hear it once, whichever path closes it.
    pub(super) fn native_players_leaving(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if std::mem::replace(&mut host.players_told_left, true) {
            return;
        }
        let players: Vec<(u128, String)> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.participates_online())
            .map(|g| (g.uuid, g.name.clone()))
            .collect();
        for (player, name) in players {
            self.native_player_logged_out(player, &name);
        }
    }
    /// The mobs from index `from` on are new in the world.
    pub(super) fn native_mobs_joined(&mut self, from: usize, natural: bool) {
        self.join_spawn_tags(from);
        // A mod can keep a mob out; it is then taken out and not told of.
        self.native_joins_flush(false, 0);
        for i in from..self.mobs.len() {
            let m = &self.mobs[i];
            let (kind, pos) = (mob_kind(m), [m.position.x, m.position.y, m.position.z]);
            self.native_mob_spawned(self.dim.id(), &kind, pos, natural);
        }
    }
    /// The acting player's blow killed mob `index`.
    pub(super) fn native_kill_event(&mut self, index: usize) {
        let (entity, kind) = (self.mobs[index].id, mob_kind(&self.mobs[index]));
        let player = self.native_acting_id();
        self.native_mob_killed(player, entity, &kind);
    }
    /// The acting player's hit landed on mob `index`, holding `stack`.
    pub(super) fn native_hurt_enemy_event(&mut self, stack: &ItemStack, index: usize) {
        let mob = &self.mobs[index];
        let kind = ["minecraft:", mob.kind.id.trim_start_matches("minecraft:")].concat();
        self.native_hurt_enemy(stack, mob.id, &kind);
    }
    /// `Inventory.tick` for the acting player: each carried stack goes through
    /// `native_inventory_tick`.
    pub(super) fn native_inventory_ticks(&mut self) {
        if self.player.is_none() {
            return;
        }
        for i in 0..self.inventory.len() {
            self.native_slot_tick(i as i32, i == self.selected);
        }
        // Head first here; `Inventory` saves feet as 100 and head as 103.
        for i in 0..self.armor.len() {
            self.native_slot_tick(103 - i as i32, false);
        }
        self.native_slot_tick(-106, false);
    }
    fn native_slot_tick(&mut self, slot: i32, selected: bool) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::InventoryTick) {
            return;
        }
        let Some(stack) = self.native_carried_slot(slot).and_then(|s| s.clone()) else {
            return;
        };
        self.native_inventory_tick(&stack, slot, selected);
        if let Some(stack) = self.native_carried_slot(slot)
            && stack.as_ref().is_some_and(|s| s.count == 0)
        {
            *stack = None;
        }
    }
    /// The slot a player's carried stack is in, by `native_inventory_tick`'s
    /// numbers: none for another number.
    pub(super) fn native_carried_slot(&mut self, slot: i32) -> Option<&mut Option<ItemStack>> {
        match slot {
            0.. if (slot as usize) < self.inventory.len() => self.inventory.get_mut(slot as usize),
            100..=103 => self.armor.get_mut(103 - slot as usize),
            -106 => Some(&mut self.offhand),
            _ => None,
        }
    }
    /// `EntityBlock.newBlockEntity` of block `name`: Sunrise's own, else a
    /// native mod's, made as a write of the level creates it
    /// (`GameLevel::new_block_entity`). Only a declared mod type builds the
    /// level.
    #[cfg(test)]
    pub(super) fn new_block_entity(&mut self, name: &str) -> Option<BlockEntity> {
        if sunrise_client::block_entity_types::type_for(name).is_none() {
            return block_entity_for_block(
                self.pack.assets.registry,
                self.res.facts(),
                name,
                Compound::default(),
            );
        }
        self.with_level(|_, level, _| level.new_block_entity(name))
    }
    /// Compile static state-factory ownership once per level, including on clients.
    pub(super) fn ensure_stateful_entity_table(&mut self) {
        if self.world.block_entities.has_stateful_entity_table() {
            return;
        }
        let registry = self.pack.assets.registry;
        let mut blocks = FxHashMap::default();
        let states = (0..registry.len() as u32)
            .map(|id| {
                let Some(state) = registry.state(BlockStateId(id)) else {
                    return false;
                };
                if state.name.starts_with("minecraft:") {
                    return false;
                }
                *blocks.entry(state.name.as_str()).or_insert_with(|| {
                    let mut stateful = false;
                    self.native_stateful_block_entity(&state.name, &mut stateful);
                    stateful
                })
            })
            .collect();
        self.world.block_entities.set_stateful_entities(states);
    }
    /// Create a block entity from an actual state using the level's creation hook.
    pub(super) fn new_block_entity_for_state(
        &mut self,
        state: BlockStateId,
    ) -> Option<BlockEntity> {
        self.with_level(|_, level, _| level.new_block_entity_for_state(state))
    }
    /// The containers of the mod blocks' block entities, by block state
    /// (`native_block_entity_container`), checked.
    pub(super) fn native_containers(&self) -> FxHashMap<u32, Arc<ModContainer>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModContainer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if state.name.starts_with("minecraft:") {
                continue;
            }
            let container = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut container = None;
                self.native_block_entity_container(&state.name, &mut container);
                container.map(|mut c| {
                    c.slots = c
                        .slots
                        .min(usize::from(sunrise_mod_package::content::MAX_MENU_SLOTS));
                    let n = c.slots;
                    for list in c.faces.iter_mut().chain([&mut c.inputs, &mut c.outputs]) {
                        list.retain(|&s| s < n);
                    }
                    c.insert_rules.truncate(n);
                    c.menu = c
                        .menu
                        .filter(|m| mod_menu(m).is_some_and(|(_, size)| size == n));
                    Arc::new(c)
                })
            });
            if let Some(c) = container {
                out.insert(i, Arc::clone(c));
            }
        }
        out
    }
    /// The fluid tanks of the mod blocks' block entities, by block state
    /// (`native_block_entity_fluids`), checked as the containers are.
    pub(super) fn native_fluid_tables(&self) -> FxHashMap<u32, Arc<ModFluidTanks>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModFluidTanks>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if state.name.starts_with("minecraft:") {
                continue;
            }
            let tanks = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut fluids = None;
                self.native_block_entity_fluids(&state.name, &mut fluids);
                fluids.map(|mut f| {
                    f.tanks.truncate(16);
                    // Clamped, not dropped: the face and fill lists name
                    // tanks by number, which must not shift.
                    for c in &mut f.tanks {
                        *c = (*c).clamp(1, 1_000_000);
                    }
                    let n = f.tanks.len();
                    for list in f.faces.iter_mut().chain([&mut f.inputs, &mut f.outputs]) {
                        list.retain(|&t| t < n);
                    }
                    Arc::new(f)
                })
            });
            if let Some(t) = tanks {
                out.insert(i, Arc::clone(t));
            }
        }
        out
    }
    /// The energy buffers of the mod blocks' block entities, by block state
    /// (`native_block_entity_energy`), checked as the containers are.
    pub(super) fn native_energy_tables(&self) -> FxHashMap<u32, Arc<ModEnergyBuffer>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModEnergyBuffer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if state.name.starts_with("minecraft:") {
                continue;
            }
            let buffer = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut energy = None;
                self.native_block_entity_energy(&state.name, &mut energy);
                energy.filter(|e| e.capacity >= 1).map(|mut e| {
                    e.max_insert = e.max_insert.clamp(0, e.capacity);
                    e.max_extract = e.max_extract.clamp(0, e.capacity);
                    Arc::new(e)
                })
            });
            if let Some(b) = buffer {
                out.insert(i, Arc::clone(b));
            }
        }
        out
    }
    /// Whether a native mod's block `name` has a ticking block entity.
    pub(in crate::game) fn native_ticks(&self, name: &str) -> bool {
        let mut ticks = false;
        self.native_block_entity_ticks(name, &mut ticks);
        ticks
    }
    /// A mod block's container at `key` opens its menu: its slots grown to
    /// the menu's. False for any other block.
    pub(super) fn open_mod_menu(&mut self, key: Pos) -> bool {
        let state = self.world.block(key.0, key.1, key.2).0;
        let Some(c) = self.world.block_entities.mod_container(state) else {
            return false;
        };
        let slots = c.slots;
        if c.menu.is_none() {
            return false;
        }
        let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&key) else {
            return false;
        };
        if m.items.len() < slots {
            m.items.resize(slots, None);
        }
        true
    }
    /// The menu type of the mod container at `key`, if it has one.
    pub(super) fn mod_menu_at(&self, key: Pos) -> Option<&'static str> {
        let state = self.world.block(key.0, key.1, key.2).0;
        let c = self.world.block_entities.mod_container(state)?;
        mod_menu(c.menu.as_deref()?).map(|(menu, _)| menu)
    }
    /// A data slot of window `window`'s menu (`ContainerSetData`); false
    /// when no mod screen is showing for that window.
    pub(super) fn native_screen_data(&mut self, window: i32, index: usize, value: i32) -> bool {
        if self.shown_native_screen().is_none() {
            return false;
        }
        self.mod_screen
            .as_mut()
            .is_some_and(|s| s.set_data(window, index, value))
    }
    /// Where the open mod screen puts container slot `slot` this frame: where
    /// a mod last moved it, else where the screen declared it; None when
    /// inactive/hidden.
    pub(super) fn native_screen_slot(&self, slot: usize) -> Option<(f32, f32)> {
        let s = self.shown_native_screen()?;
        let declared = s
            .layout()
            .slots
            .get(slot)?
            .as_ref()
            .map(|slot| slot.position);
        let [x, y] = s.slot_moves.position(slot, declared)?;
        Some((x, y))
    }
    pub(in crate::game) fn native_screen_slot_icon(&self, slot: usize) -> Option<&str> {
        self.shown_native_screen()?
            .layout()
            .slots
            .get(slot)?
            .as_ref()?
            .no_item_icon
            .as_deref()
    }
    /// Where the open screen of mod menu type `menu` puts container slot
    /// `slot` from the next frame: the mod's last move, else the screen's
    /// declaration; None for an inactive/hidden slot, another menu type's screen or none.
    pub(in crate::game) fn screen_slot(&self, menu: &str, slot: usize) -> Option<[f32; 2]> {
        let s = self.shown_native_screen().filter(|s| s.menu.name == menu)?;
        s.slot_moves.latest(&s.layout().slots, slot)
    }
    /// Moves or activates container slot `slot` of the open screen of menu type
    /// `menu` to `pos` (relative to the screen's origin), or hides it with None,
    /// from the next frame (`SlotMoves::set`).
    pub(in crate::game) fn set_screen_slot(
        &self,
        menu: &str,
        slot: usize,
        pos: Option<[f32; 2]>,
    ) -> Result<(), mod_api::ScreenSlotRefusal> {
        let s = self
            .shown_native_screen()
            .filter(|s| s.menu.name == menu)
            .ok_or(mod_api::ScreenSlotRefusal::NoScreen)?;
        s.slot_moves.set(&s.layout().slots, slot, pos)
    }
    /// The internal container slot base of the open mod screen (`MOD_MENU_SLOT_BASE`
    /// for a `ModMenuType`, `300` for a stock menu).
    pub(in crate::game) fn native_screen_container_slot_base(&self) -> usize {
        self.shown_native_screen()
            .map_or(300, OpenNativeScreen::container_slot_base)
    }
    /// The container slots the open mod screen shows this frame.
    pub(super) fn native_screen_slots(&self) -> Option<Vec<usize>> {
        Some(
            self.shown_native_screen()?
                .visible_container_slots()
                .collect(),
        )
    }
    /// The first row of the player's inventory on the open mod screen.
    pub(super) fn native_screen_inventory_y(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.layout().inventory_y)
    }
    /// The open mod screen's inventory's left column.
    pub(super) fn native_screen_inventory_x(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.layout().inventory_x)
    }
    /// The open mod screen's width.
    pub(super) fn native_screen_width(&self) -> Option<f32> {
        self.shown_native_screen().map(|s| s.layout().width)
    }
    /// The open mod screen: a plain panel with its slots' frames, the mod's
    /// drawing over it, then the title and the inventory's label. False when
    /// none is showing.
    pub(super) fn draw_native_screen(&self, batch: &mut HudBatch, ox: f32, oy: f32) -> bool {
        let Some(s) = self.shown_native_screen() else {
            return false;
        };
        let atlas = &self.gui.atlas;
        if !s.layout().own_background {
            let (w, h) = (s.layout().width, s.layout().height);
            batch.fill(atlas, ox, oy, w, h, rgba(0, 0, 0, 255));
            batch.fill(
                atlas,
                ox + 1.0,
                oy + 1.0,
                w - 2.0,
                h - 2.0,
                rgba(198, 198, 198, 255),
            );
            for i in s.visible_slots() {
                let (x, y) = self.slot_position(i);
                let (x, y) = (ox + x - 1.0, oy + y - 1.0);
                batch.fill(atlas, x, y, 18.0, 18.0, rgba(55, 55, 55, 255));
                batch.fill(
                    atlas,
                    x + 1.0,
                    y + 1.0,
                    17.0,
                    17.0,
                    rgba(255, 255, 255, 255),
                );
                batch.fill(
                    atlas,
                    x + 1.0,
                    y + 1.0,
                    16.0,
                    16.0,
                    rgba(139, 139, 139, 255),
                );
            }
        }
        if s.screen.has_draw() {
            let frame = s.frame(batch, (ox, oy), self.cursor);
            let mut draw = crate::game::host::draw::GameDrawContext::new(self, batch);
            s.screen.draw(self, frame, &mut draw);
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        if s.layout().own_labels {
            return true;
        }
        let grey = rgba(64, 64, 64, 255);
        if let Some(title) = self.remote_menu_title() {
            let [x, y] = s.layout().title_label.unwrap_or([8.0, 6.0]);
            batch.text_styled(atlas, &title, ox + x, oy + y, grey, false);
        }
        if s.player_slots() != 0 {
            let inventory = self.translate_with_fallback("container.inventory", "Inventory");
            let [x, y] = s
                .layout()
                .inventory_label
                .unwrap_or([8.0, s.layout().height - 94.0]);
            batch.text(atlas, &inventory, ox + x, oy + y, grey, false);
        }
        true
    }
    /// Menu button `id` on a mod menu (`clickMenuButton`), with the acting
    /// player's cursor stack (`carried`, read and written). A block-backed
    /// menu lends its live resident block entity through `BlockEntityScope`;
    /// a blockless menu lends no view while keeping `host()` and
    /// `transaction(...)` usable.
    pub(super) fn mod_menu_button(
        &mut self,
        menu: &str,
        context: mod_api::item::menu::MenuButtonContext,
        block: Option<&str>,
        backing: Option<(i32, i32, i32)>,
        carried: &mut Option<ItemStack>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuButton) {
            return;
        }
        let Ok(mut api_cursor) = carried.try_into_api() else {
            return;
        };
        let original_api = api_cursor.clone();
        let target_pos = backing.filter(|&pos| self.mod_menu_at(pos) == Some(menu));
        let target = target_pos.and_then(|pos| self.resident_block_entity_target(pos));
        let items_changed = {
            let mut access = match target {
                Some((handle, state)) => ResidentBlockEntityAccess::for_handle(self, handle, state),
                None => ResidentBlockEntityAccess::blockless(self),
            };
            Game::native_menu_button(
                menu,
                block,
                &context,
                mod_api::BlockEntityScope::new(&mut access),
                &mut api_cursor,
            );
            access.items_changed()
        };
        if items_changed
            && let Some((handle, _)) = target
            && let Some(pos) = self.native_block_entity_write_key(handle)
        {
            self.after_commit(crate::game::transactions::After::Changed(pos));
        }
        apply_menu_button_cursor(self.items(), &original_api, &api_cursor, carried);
    }
    /// A press consumed by the shown screen goes to the server as a menu button click.
    pub(super) fn native_screen_press(&mut self, right: bool) -> bool {
        let Some(r) = self.renderer.as_ref() else {
            return false;
        };
        let batch = HudBatch::new(
            [r.config.width, r.config.height],
            self.settings.gui_scale_inputs(),
        );
        self.native_screen_press_at(&batch, right)
    }
    /// Dispatch a press in the given GUI viewport, with inventory slots taking priority.
    fn native_screen_press_at(&mut self, batch: &HudBatch, right: bool) -> bool {
        let Some(screen) = self.shown_native_screen() else {
            return false;
        };
        if self.slot_at(batch, self.cursor.0, self.cursor.1).is_some() {
            return false;
        }
        let opening = (screen.menu.window, screen.menu.generation);
        let frame = screen.frame(batch, self.inventory_origin(batch), self.cursor);
        let Some(button) = screen.screen.click(self, frame, right) else {
            return false;
        };
        // Numeric container IDs can be reused. Send only for the opening that heard the press.
        if !self
            .shown_native_screen()
            .is_some_and(|screen| (screen.menu.window, screen.menu.generation) == opening)
            || !self.remote_button_click(button)
        {
            return false;
        }
        self.audio.play_button_click();
        true
    }
    /// The open mod screen's tooltip under the cursor.
    pub(super) fn draw_native_screen_tooltip(&mut self, batch: &mut HudBatch) {
        // Over no item: an empty slot still gets the mod's hint (which fuel it takes).
        // A carried stack hides only slot tooltips, as AbstractContainerScreen's;
        // the screen's other parts keep theirs, as a vanilla widget's tooltip does.
        let slot = self.slot_at(batch, self.cursor.0, self.cursor.1);
        if slot.is_some_and(|i| self.slot_view(i).is_some()) {
            return;
        }
        let mut lines = Vec::new();
        self.native_screen_tooltip_lines(batch, slot, &mut lines);
        self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
    }
    /// The screen-specific line override, shared by filled-slot and empty-space tooltips.
    /// Item images and component gathering remain on the stack's normal drawing path.
    pub(super) fn native_screen_tooltip_lines(
        &self,
        batch: &HudBatch,
        slot: Option<usize>,
        lines: &mut Vec<(sunrise_client::text::Text, u32)>,
    ) {
        let Some(screen) = self
            .shown_native_screen()
            .filter(|s| s.screen.has_tooltip())
        else {
            return;
        };
        use crate::game::api_values::{TryIntoApi, TryIntoEngine};
        // Keep the original tooltip if the callback or either conversion fails.
        // An item's text can fail on every frame, so do not log conversion refusals here.
        let Ok(mut snapshot) = lines.try_into_api() else {
            return;
        };
        let menu_slot = slot.and_then(|slot| {
            Self::menu_slot_index(&screen.menu.name, slot).map(|slot| slot as u32)
        });
        let frame = screen.frame(batch, self.inventory_origin(batch), self.cursor);
        if screen.screen.tooltip(self, frame, menu_slot, &mut snapshot)
            && let Ok(replacement) = snapshot.try_into_engine(&())
        {
            *lines = replacement;
        }
    }
    /// Options > Mods' rows for loaded code mods, after the content-only rows.
    pub(super) fn list_native_mods(&mut self) {
        let registered = super::mods::collect_key_mappings(
            self.options.execution_role,
            mod_api::__private::key_mappings(),
        );
        let native = self.mods.native.clone();
        let mut rows: Vec<_> = self
            .settings
            .mods
            .installed
            .iter()
            .filter(|row| {
                mod_api::__private::mod_index(&row.id).is_some()
                    && !native.built().iter().any(|m| m.id == row.id)
            })
            .cloned()
            .collect();
        rows.extend(native.packages().iter().map(|package| {
            let m = &package.manifest;
            self.settings
                .mods
                .info
                .insert(m.id.clone(), sunrise_client::mod_list::ModInfo::of(m, None));
            Installed {
                id: m.id.clone(),
                title: format!("{} {}", m.name, m.version),
                description: m.description.clone(),
                problem: None,
                dev: false,
                native: None,
            }
        }));
        self.settings
            .mods
            .installed
            .retain(|row| !rows.iter().any(|code| code.id == row.id));
        for mut row in rows {
            let settings = self.native_page_settings(&row.id);
            let mappings: Vec<KeyMapping> = registered
                .iter()
                .filter(|(owner, _)| *owner == row.id)
                .map(|(_, k)| KeyMapping {
                    id: k.id.clone(),
                    label: k.label.clone(),
                    key: k.key.clone(),
                    modifiers: k.modifiers.bits,
                    context: crate::game::api_values::TryIntoEngine::try_into_engine(
                        &k.context,
                        &(),
                    )
                    .unwrap(),
                    category: k.category.clone(),
                })
                .collect();
            row.native = Some(NativePage { mappings, settings });
            self.settings.mods.installed.push(row);
        }
    }
    /// The settings mod `mod_id` registered, with their kept values, as its page shows them.
    fn native_page_settings(&self, mod_id: &str) -> Vec<NativeSetting> {
        mod_api::__private::settings()
            .iter()
            .filter(|(owner, _)| *owner == mod_id)
            .map(|(_, spec)| {
                let kept = self.stored_setting(mod_id, &spec.key);
                let value = match (&spec.kind, spec.read(kept.as_deref())) {
                    (_, mod_api::SettingValue::Toggle(on)) => NativeSettingValue::Toggle(on),
                    (
                        mod_api::SettingKind::Choice { options, .. },
                        mod_api::SettingValue::Choice(index),
                    ) => NativeSettingValue::Choice {
                        options: options.clone(),
                        index,
                    },
                    (
                        mod_api::SettingKind::Range { min, max, step, .. },
                        mod_api::SettingValue::Range(value),
                    ) => NativeSettingValue::Range {
                        min: *min,
                        max: *max,
                        step: *step,
                        value,
                    },
                    (_, mod_api::SettingValue::Color(rgb)) => NativeSettingValue::Color(rgb),
                    _ => NativeSettingValue::Toggle(false),
                };
                NativeSetting {
                    key: spec.key.clone(),
                    label: spec.label.clone(),
                    tooltip: spec.tooltip.clone(),
                    value,
                }
            })
            .collect()
    }
    /// Setting `j` of native mod `i`'s page (its index in Options > Mods) changed on the page:
    /// kept as the mod registered it, then the mod is told (`setting_changed`).
    pub(super) fn native_setting_event(&mut self, i: usize, j: usize) {
        let Some(row) = self.settings.mods.installed.get(i) else {
            return;
        };
        let Some(setting) = row.native.as_ref().and_then(|page| page.settings.get(j)) else {
            return;
        };
        let (id, key) = (row.id.clone(), setting.key.clone());
        let value = match &setting.value {
            NativeSettingValue::Toggle(on) => mod_api::SettingValue::Toggle(*on),
            NativeSettingValue::Choice { index, .. } => mod_api::SettingValue::Choice(*index),
            NativeSettingValue::Range { value, .. } => mod_api::SettingValue::Range(*value),
            NativeSettingValue::Color(rgb) => mod_api::SettingValue::Color(*rgb),
        };
        let kept = mod_api::__private::setting(&id, &key).and_then(|spec| spec.store(value));
        let Some(kept) = kept else {
            return;
        };
        if self.store_setting(&id, &key, &kept) {
            self.native_setting_changed(&id, &key);
        }
    }
    /// Native mod `i`'s page opened: its settings' kept values read again.
    pub(super) fn native_settings_refresh(&mut self, i: usize) {
        let Some(id) = self.settings.mods.installed.get(i).map(|m| m.id.clone()) else {
            return;
        };
        let settings = self.native_page_settings(&id);
        let row = self.settings.mods.installed.get_mut(i);
        if let Some(page) = row.and_then(|m| m.native.as_mut()) {
            page.settings = settings;
        }
    }
    /// A client mod's command (the `command` hook); a server never asks.
    pub(super) fn client_mod_command(&mut self, command: &str) -> bool {
        if self.lan_acting.is_some() {
            return false;
        }
        let mut ran = false;
        self.native_command(command, &mut ran);
        ran
    }
    /// The zoom in effect while playing (a native mod's); the spyglass's own
    /// view wins.
    pub(super) fn native_zoom_now(&self) -> Option<mod_api::Zoom> {
        if !self.playing() || self.scoping() {
            return None;
        }
        let mut zoom = None;
        self.native_zoom(&mut zoom);
        let z = zoom?;
        let factor = finite(z.factor, 1.0, 10.0);
        (factor > 1.0).then_some(mod_api::Zoom {
            factor,
            smooth: z.smooth,
            turn: if z.turn.is_finite() {
                z.turn.clamp(0.05, 1.0)
            } else {
                1.0
            },
        })
    }
    /// The pose of native mods' camera shake in this frame (`native_camera_shake`),
    /// none while not playing.
    pub(super) fn native_shake_matrix(&self, partial: f32) -> glam::Mat4 {
        if !self.playing() {
            return glam::Mat4::IDENTITY;
        }
        let mut shake = Shake::default();
        self.native_camera_shake(partial, &mut shake);
        if shake == Shake::default() {
            return glam::Mat4::IDENTITY;
        }
        shake_pose(bounded_shake(shake, self.screen_effect_scale()))
    }

    /// The camera native mods draw this frame from (`native_camera_pose`), starting from
    /// `view`, vanilla's position, yaw and pitch.
    pub(super) fn native_camera_pose_now(
        &mut self,
        partial: f32,
        view: (glam::DVec3, f32, f32),
    ) -> (glam::DVec3, f32, f32) {
        let (mut position, mut yaw, mut pitch) = (view.0.to_array(), view.1, view.2);
        self.native_camera_pose(partial, &mut position, &mut yaw, &mut pitch);
        let posed = posed_camera(view, position, yaw, pitch);
        if self.apply_requested_camera_type()
            && let Some(refreshed) = self.camera_view(partial)
        {
            // Refresh vanilla components, not the listener's explicit overrides.
            return (
                if posed.0 == view.0 {
                    refreshed.0
                } else {
                    posed.0
                },
                if posed.1 == view.1 {
                    refreshed.1
                } else {
                    posed.1
                },
                if posed.2 == view.2 {
                    refreshed.2
                } else {
                    posed.2
                },
            );
        }
        posed
    }

    /// What native mods hand shader packs in this frame (`native_shader_params`),
    /// with values that aren't finite as 0.
    pub(super) fn native_shader_params_now(&self, partial: f32) -> [[f32; 4]; 8] {
        let mut params = [[0.0; 4]; 8];
        self.native_shader_params(partial, &mut params);
        finite_params(params)
    }
}

/// Whether F3's debug screen, open when `debug`, covers `corner`
/// (`DebugScreenOverlay.extractRenderState`): its left and right columns
/// the top corners, and its charts the bottom corners `charts` (bottom left,
/// bottom right) says they are drawn in.
pub(super) fn debug_takes(corner: Corner, debug: bool, charts: [bool; 2]) -> bool {
    debug
        && match corner {
            Corner::TopLeft | Corner::TopRight => true,
            Corner::BottomLeft => charts[0],
            Corner::BottomRight => charts[1],
        }
}

/// A supported type for a block's container menu. A transient container is never block-backed.
pub(super) fn mod_menu(name: &str) -> Option<(&'static str, usize)> {
    MOD_MENUS
        .into_iter()
        .find(|(menu, _)| *menu == name)
        .or_else(|| {
            sunrise_client::containers::mod_menu_type(name)
                .filter(|menu| menu.owned_slots.is_none())
                .map(|menu| (menu.id, menu.slots))
        })
}

/// `state` with its property `key` set to `value`; none when the block has
/// no such property or value.
pub(super) fn with_property(
    registry: &BlockRegistry,
    state: BlockStateId,
    key: &str,
    value: &str,
) -> Option<BlockStateId> {
    let mut s = registry.state(state)?.clone();
    match s.properties.get(key) {
        Some(v) if v == value => return Some(state),
        Some(_) => {}
        None => return None,
    }
    s.properties.insert(key.into(), value.into());
    registry.resolve(&s).ok()
}

/// The state a mod's property write changes `state` to; none when the block
/// has no such property or value, or already has it: `LevelChunk.setBlockState`
/// returns null for the same state, so `Level.setBlock` returns false.
fn property_change(
    registry: &BlockRegistry,
    state: BlockStateId,
    key: &str,
    value: &str,
) -> Option<BlockStateId> {
    with_property(registry, state, key, value).filter(|&id| id != state)
}

/// `config/` in the game folder, beside `options.txt`, made when missing.
pub(super) fn config_dir(game_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = game_dir.join("config");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// An item entity as `native_item_entities` lists it.
/// Read by mods only.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(super) struct NativeItemEntity {
    /// The entity's ID.
    pub id: u32,
    pub position: [f64; 3],
    pub stack: ItemStack,
    /// Ticks before a player may pick it up (`ItemEntity.pickupDelay`);
    /// hoppers take it whatever this is.
    pub pickup_delay: u32,
}

/// `AABB.intersects`: the boxes overlap, touching faces excluded.
fn box_meets(b: &sunrise_core::geometry::Aabb, min: [f64; 3], max: [f64; 3]) -> bool {
    let (lo, hi) = (b.min_position(), b.max_position());
    lo.x < max[0]
        && hi.x > min[0]
        && lo.y < max[1]
        && hi.y > min[1]
        && lo.z < max[2]
        && hi.z > min[2]
}

/// What `mod_screen_open(mod_id)` does over `screen` (`blocked`: an
/// overlay, the title screen, no window or no player; `owner`'s own screen
/// open): `None` opens it, `Some` answers without changing anything (true:
/// `mod_id`'s is already open).
pub(super) fn mod_screen_opens(
    screen: Screen,
    blocked: bool,
    owner: Option<&str>,
    mod_id: &str,
) -> Option<bool> {
    if screen == Screen::Mod {
        return Some(owner == Some(mod_id));
    }
    if screen != Screen::Playing || blocked {
        return Some(false);
    }
    None
}

fn client_packet_view(packet: &sunrise_protocol::packets::play::Clientbound) -> ClientPacket {
    use mod_api::Identifier;
    use std::sync::LazyLock;
    use sunrise_protocol::packets::play::Clientbound;

    static TYPES: LazyLock<Vec<Identifier>> =
        LazyLock::new(|| packet_types(sunrise_protocol::ids::play::clientbound::NAMES));
    let payload = match packet {
        Clientbound::CustomPayload(payload) => Some(payload),
        _ => None,
    };
    packet_view(&TYPES, packet.id(), payload)
}

/// `client_packet_send`'s view of a packet the game sends: as `client_packet_view`.
fn serverbound_packet_view(packet: &sunrise_protocol::packets::play::Serverbound) -> ClientPacket {
    use mod_api::Identifier;
    use std::sync::LazyLock;
    use sunrise_protocol::packets::play::Serverbound;

    static TYPES: LazyLock<Vec<Identifier>> =
        LazyLock::new(|| packet_types(sunrise_protocol::ids::play::serverbound::NAMES));
    let payload = match packet {
        Serverbound::CustomPayload(payload) => Some(payload),
        _ => None,
    };
    packet_view(&TYPES, packet.id(), payload)
}

/// One direction's packet type identifiers, by protocol id.
fn packet_types(names: &[&str]) -> Vec<mod_api::Identifier> {
    names
        .iter()
        .map(|name| mod_api::Identifier::parse(name).expect("protocol packet type identifier"))
        .collect()
}

/// A packet's type, `sunrise:unknown` outside the protocol's table, and its custom payload.
fn packet_view(
    types: &[mod_api::Identifier],
    id: i32,
    payload: Option<&sunrise_protocol::packets::configuration::CustomPayload>,
) -> ClientPacket {
    use mod_api::Identifier;
    use std::sync::LazyLock;

    static UNKNOWN: LazyLock<Identifier> =
        LazyLock::new(|| Identifier::parse("sunrise:unknown").unwrap());
    let id = usize::try_from(id)
        .ok()
        .and_then(|id| types.get(id))
        .unwrap_or(&UNKNOWN)
        .clone();
    let payload = payload.map(|p| ClientPayload::new(p.channel.clone(), p.data.clone()));
    ClientPacket::new(id, payload)
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn declared_types_create_tick_and_reload_through_game() {
        use super::super::native_entity_refs::tests::{record_key, run_fresh_cases, with_game_at};
        use mod_api::{self, *};
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, containers::BlockEntity, save};

        const CHILD: &str = "SUNRISE_DECLARED_BE_GAME_CHILD";
        if run_fresh_cases(
            "game::native_hooks::tests::declared_types_create_tick_and_reload_through_game",
            CHILD,
            &["valid", "unknown_block", "unknown_type", "block_not_type"],
        ) {
            return;
        }

        const FIRST: &str = "be_test:press";
        const SECOND: &str = "be_test:ornate_banner";
        const EMPTY: &str = "be_test:blank";
        const PLAIN: &str = "be_test:plain_banner";
        #[derive(Default)]
        struct Keys {
            own: std::sync::OnceLock<BlockEntityTypeKey>,
            shared: std::sync::OnceLock<BlockEntityTypeKey>,
            vanilla: std::sync::OnceLock<BlockEntityTypeKey>,
        }
        trait Probe {
            fn value(&self) -> u32;
        }
        struct Marker(u32);
        impl Probe for Marker {
            fn value(&self) -> u32 {
                self.0
            }
        }
        const PROBE: CapabilityKey<dyn Probe> = CapabilityKey::new("be_test:probe");
        struct Press;
        #[mod_api::sunrise_part]
        impl BlockPart for Press {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for Press {
            fn block_entity(
                &self,
                _: LevelSide<'_>,
                _: &mut BlockEntityEvent<'_>,
            ) -> Option<Compound> {
                Some(Compound::from([("energy".into(), Tag::Long(6300))]))
            }
            fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
                event.subject.with_view(|mut entity| {
                    let key = "energy".into();
                    let Some(Tag::Long(energy)) = entity.nbt().get(&key) else {
                        panic!("factory payload missing");
                    };
                    assert!(entity.set_nbt(key, Some(Tag::Long(energy + 1))));
                });
            }
        }
        struct User {
            keys: Arc<Keys>,
            unknown: Option<&'static str>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for User {
            fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let kind = registrar.block_entity_type_key("be_test:shared");
                registrar
                    .block_entity_part(&kind, Press)
                    .expect("native type");
                registrar.block_entity_capability(&PROBE, &kind, |_, _, _, _, offer| {
                    offer.offer(|| Box::new(Marker(6300)))
                });
                record_key(&self.keys.shared, kind);
                let vanilla = registrar.block_entity_type_key("minecraft:chest");
                registrar.block_entity_capability(&PROBE, &vanilla, |_, _, _, _, offer| {
                    offer.offer(|| Box::new(Marker(26)))
                });
                record_key(&self.keys.vanilla, vanilla);
                if let Some(id) = self.unknown {
                    registrar.block_entity_type_key(id);
                }
            }
        }
        struct TestMod {
            invalid: bool,
            keys: Arc<Keys>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for TestMod {
            fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let kind = registrar
                    .block_entity_type("shared")
                    .blocks(if self.invalid {
                        &["be_test:missing"]
                    } else {
                        &[FIRST, SECOND]
                    })
                    .build();
                record_key(&self.keys.own, kind);
                registrar
                    .block_entity_type("empty")
                    .blocks(&[EMPTY])
                    .build();
            }
            fn blocks(&self, _context: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
                for block in [FIRST, SECOND, EMPTY, PLAIN]
                    .map(|id| mod_api::Block::new(id, "minecraft:white_banner"))
                {
                    event.register(block).unwrap();
                }
            }
        }
        let case = std::env::var(CHILD).unwrap();
        let invalid = case == "unknown_block";
        let unknown = match case.as_str() {
            "unknown_type" => Some("other:missing"),
            "block_not_type" => Some("minecraft:red_shulker_box"),
            _ => None,
        };
        let keys = Arc::new(Keys::default());
        let mods: Mods = Arc::from([
            ModEntry::new(
                "be_user",
                Box::new(User {
                    keys: keys.clone(),
                    unknown,
                }),
            ),
            ModEntry::new(
                "be_test",
                Box::new(TestMod {
                    invalid,
                    keys: keys.clone(),
                }),
            ),
        ]);
        mod_platform::__private::install(mods.clone());
        // `be_user` attaches a Part to the type `be_test` declares, which names `be_test`
        // among its dependencies.
        let package = |id: &str, depends_on: &[&str]| {
            PackageInfo::new(
                id.into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec![id.into()],
                depends_on
                    .iter()
                    .map(|owner| {
                        PackageDependency::new(
                            (*owner).into(),
                            "*".into(),
                            DependencyKind::Required,
                            DependencyOrder::After,
                            Side::Both,
                            None,
                            None,
                        )
                    })
                    .collect(),
            )
        };
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        mod_platform::__private::with_test_package_inventory(
            vec![package("be_test", &[]), package("be_user", &["be_test"])],
            || {
                crate::game::api_values::install_parts(&mods).unwrap();
                let root = super::super::mods::content_root(&base, &Default::default());
                if invalid {
                    let error = root.unwrap_err().to_string();
                    assert!(error.contains("be_test:shared"), "{error}");
                    assert!(error.contains("be_test:missing"), "{error}");
                    // A refused declaration builds no content root. The roots folder's own `.lock`
                    // is made by every start and never deleted, so it is not a root.
                    let built: Vec<_> = std::fs::read_dir(".sunrise-cache/mod-content")
                        .into_iter()
                        .flatten()
                        .map(|entry| entry.unwrap().file_name())
                        .filter(|name| name != ".lock")
                        .collect();
                    assert!(built.is_empty(), "{built:?}");
                    return;
                }
                if let Some(id) = unknown {
                    let error = root.unwrap_err().to_string();
                    assert!(error.contains("be_user"), "{error}");
                    assert!(error.contains("unknown block entity type"), "{error}");
                    assert!(error.contains(id), "{error}");
                    assert!(sunrise_client::block_entity_types::current().is_none());
                    return;
                }
                assert_eq!(keys.shared.get(), keys.own.get());
                with_game_at(root.unwrap().prepared().to_path_buf(), |game| {
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let level = game.level(game.dim.id()).unwrap();
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    for (x, block) in [
                        (1, FIRST),
                        (2, SECOND),
                        (3, EMPTY),
                        (4, PLAIN),
                        (5, "minecraft:chest"),
                    ] {
                        let state = game.pack.assets.registry.default_state(block).unwrap();
                        assert_eq!(
                            game.level_set_block(level, [x, 63, 0], stone, BlockUpdateFlags::NONE),
                            Ok(true)
                        );
                        assert_eq!(
                            game.level_set_block(level, [x, 64, 0], state, BlockUpdateFlags::NONE),
                            Ok(true)
                        );
                        let pos = (x, 64, 0);
                        if block == PLAIN {
                            // No declared type claims it, and no mod adds it to `minecraft:banner`'s
                            // valid blocks, so its template creates no entity (`validateBlockState`).
                            assert!(game.world.block_entities.get(&pos).is_none());
                            assert!(game.block_entity(level, [x, 64, 0]).is_none());
                            continue;
                        }
                        let handle = game.block_entity(level, [x, 64, 0]).unwrap();
                        if block == "minecraft:chest" {
                            assert_eq!(handle.kind(game).as_ref(), keys.vanilla.get());
                            let probe = level.capability(game, &PROBE, [x, 64, 0], None).unwrap();
                            assert_eq!((probe.value(), probe.owner()), (26, "be_user".into()));
                            continue;
                        }
                        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos)
                        else {
                            panic!("declared type did not override its banner template");
                        };
                        if block == EMPTY {
                            assert_eq!(entity.id(), "be_test:empty");
                            assert!(entity.nbt.is_empty());
                            continue;
                        }
                        assert_eq!(entity.id(), "be_test:shared");
                        assert_eq!(entity.nbt.get(&"energy".into()), Some(&Tag::Long(6300)));
                        assert_eq!(handle.kind(game).as_ref(), keys.shared.get());
                        let probe = level.capability(game, &PROBE, [x, 64, 0], None).unwrap();
                        assert_eq!((probe.value(), probe.owner()), (6300, "be_user".into()));
                        game.tick_mod_block_entity(pos);
                        let saved = save::block_entity_encode::encode(
                            pos,
                            game.world.block_entities.get(&pos).unwrap(),
                            block,
                            &Default::default(),
                        );
                        assert_eq!(saved.get("energy"), Some(&Tag::Long(6301)));
                        let (saved_pos, restored) =
                            save::decode_block_entity_for_block(&saved, block).unwrap();
                        assert_eq!(saved_pos, pos);
                        assert_eq!(restored.id(), "be_test:shared");
                        assert!(save::decode_block_entity_for_block(&saved, EMPTY).is_none());
                        game.world.block_entities.insert(pos, restored);
                        game.tick_mod_block_entity(pos);
                        let saved = save::block_entity_encode::encode(
                            pos,
                            game.world.block_entities.get(&pos).unwrap(),
                            block,
                            &Default::default(),
                        );
                        assert_eq!(saved.get("energy"), Some(&Tag::Long(6302)));
                    }

                    // A level hook's `LevelWriteHost::set_block` fires the same hook in its write.
                    let first = game.pack.assets.registry.default_state(FIRST).unwrap();
                    use crate::game::api_values::TryIntoApi;
                    let written = game.with_level(|_, level, ticks| {
                        LevelWriteHost::set_block(
                            &mut super::super::mod_api_host::LevelTicks::new(level, ticks),
                            [5, 64, 0],
                            first.try_into_api().unwrap(),
                            BlockUpdateFlags::NONE,
                        )
                    });
                    assert!(written);
                    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&(5, 64, 0))
                    else {
                        panic!("a level write did not create the declared block entity");
                    };
                    assert_eq!(entity.id(), "be_test:shared");
                    assert_eq!(entity.nbt.get(&"energy".into()), Some(&Tag::Long(6300)));
                });
            },
        );
    }

    #[test]
    fn observed_overlay_prefers_first_open_screen() {
        assert_eq!(
            super::observed_overlay_id(&[(false, "sign"), (false, "book")]),
            None
        );
        assert_eq!(
            super::observed_overlay_id(&[(false, "sign"), (true, "book")]),
            Some("book")
        );
        assert_eq!(
            super::observed_overlay_id(&[(true, "sign"), (true, "book")]),
            Some("sign")
        );
    }

    /// Exercise the gameplay constructor, where name-based entities compete
    /// with a mod block's sign template.
    #[test]
    #[ignore = "daily"]
    fn mod_sign_constructor_prioritizes_templates_over_suffixes() {
        // This fixture adds blocks that are absent from the shared vanilla
        // game's frozen process-wide block definitions.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_hooks::tests::mod_sign_constructor_prioritizes_templates_over_suffixes",
            "SUNRISE_SIGN_CONSTRUCTOR_CHILD",
            &["signs"],
        ) {
            return;
        }
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use sunrise_client::mod_content::{ContentMod, build};
        use sunrise_mod_package::content::{Block, Content};

        let base = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let root = std::env::temp_dir().join(format!(
            "sunrise-sign-constructor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cases = [
            ("ruby:notice_banner", "minecraft:oak_sign", false),
            ("ruby:board_shelf", "minecraft:oak_wall_sign", false),
            ("ruby:lantern_head", "minecraft:oak_hanging_sign", true),
            (
                "ruby:bracket_skull",
                "minecraft:oak_wall_hanging_sign",
                true,
            ),
        ];
        let mut content = Content::declared("ruby");
        let blocks: Vec<_> = cases
            .iter()
            .map(|&(id, template, _)| (id, template))
            .chain([("ruby:road_sign", "minecraft:stone")])
            .map(|(id, template)| Block {
                id: id.into(),
                template: template.into(),
                item: false,
                sign_text_color: (id == "ruby:notice_banner").then(|| "light_blue".into()),
                sign_text_glow: (id == "ruby:notice_banner").then_some(true),
                ..Default::default()
            })
            .collect();
        content.declare(&blocks, &[]).unwrap();
        build(
            &base,
            std::path::Path::new(""),
            &[ContentMod {
                id: "ruby".into(),
                content,
                jars: Vec::new(),
            }],
            &root,
            &sunrise_mod_package::enabled::EnabledMods::new([("ruby".into(), mod_api::Side::Both)]),
        )
        .unwrap();
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            super::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();

        for (name, hanging) in cases
            .map(|(name, _, hanging)| (name, hanging))
            .into_iter()
            .chain([
                ("minecraft:oak_sign", false),
                ("minecraft:oak_hanging_sign", true),
            ])
        {
            match game.new_block_entity(name) {
                Some(super::BlockEntity::Sign(sign)) => {
                    assert_eq!(sign.hanging, hanging, "{name}")
                }
                entity => panic!("{name}: expected a sign, got {entity:?}"),
            }
        }
        // `createDefaultSignText`: a declared colour and glow on both sides of a new sign, black
        // and unlit on one that declares none.
        let mut text = |name| match game.new_block_entity(name) {
            Some(super::BlockEntity::Sign(sign)) => {
                assert_eq!(sign.front.color, sign.back.color);
                assert_eq!(sign.front.glowing, sign.back.glowing);
                (sign.front.color, sign.front.glowing)
            }
            entity => panic!("{name}: expected a sign, got {entity:?}"),
        };
        assert_eq!(text("ruby:notice_banner"), (3, true));
        assert_eq!(text("ruby:board_shelf"), (15, false));
        assert_eq!(text("minecraft:oak_sign"), (15, false));
        assert!(game.new_block_entity("ruby:road_sign").is_none());
        assert!(matches!(
            game.new_block_entity("minecraft:white_banner"),
            Some(super::BlockEntity::Banner(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "daily"]
    fn generated_fire_matches_dispatch() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};
        use mod_api::{ClientHost, Flow, Hooks, ModEntry};
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Listener(usize, Arc<Mutex<Vec<usize>>>);
        #[mod_api::sunrise_mod]
        impl Hooks for Listener {
            fn block_highlight(
                &self,
                _: &dyn ClientHost,
                _: &mut mod_api::BlockHighlightEvent,
            ) -> Flow {
                self.1.lock().unwrap().push(self.0);
                if self.0 == 1 {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn screen_exclusion_areas(
                &self,
                _: &dyn ClientHost,
                event: &mut mod_api::ScreenExclusionAreasEvent<'_>,
            ) {
                self.1.lock().unwrap().push(self.0);
                event.areas.push([self.0 as f32, 0.0, 0.0, 0.0]);
            }
            fn movement_input_update(
                &self,
                _: &dyn ClientHost,
                event: &mut mod_api::MovementInputUpdateEvent<'_>,
            ) {
                self.1.lock().unwrap().push(self.0);
                event.input.move_vector[1] += self.0 as f32;
            }
        }
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mods = (0..3)
            .map(|i| ModEntry::new("listener", Box::new(Listener(i, trace.clone()))))
            .collect::<Vec<_>>();
        mod_platform::testing::with_mods(mods.into(), || {
            let run = |fire: bool| {
                trace.lock().unwrap().clear();
                let mut canceled = false;
                if fire {
                    game.fire_block_highlight(
                        [1, 2, 3],
                        super::BlockStateId(7),
                        0.5,
                        &mut canceled,
                    );
                } else {
                    mod_api::dispatch_block_highlight(
                        &game,
                        [1, 2, 3],
                        crate::game::api_values::TryIntoApi::try_into_api(&super::BlockStateId(7))
                            .unwrap(),
                        0.5,
                        &mut canceled,
                    );
                }
                let cancel_trace = trace.lock().unwrap().clone();
                trace.lock().unwrap().clear();
                let view = super::ObservedScreen {
                    id: "inventory",
                    overlay: false,
                    menu: None,
                    title: None,
                    rect: [0.0; 4],
                    slots: 0,
                    mouse: [0.0; 2],
                    partial_tick: 0.0,
                };
                let mut areas = Vec::new();
                if fire {
                    game.fire_screen_exclusion_areas(&view.view(), &mut areas);
                } else {
                    mod_api::dispatch_screen_exclusion_areas(&game, &view.view(), &mut areas);
                }
                let collect_trace = trace.lock().unwrap().clone();
                trace.lock().unwrap().clear();
                let forward = game.movement_dispatch_fixture(fire);
                (
                    canceled,
                    cancel_trace,
                    areas,
                    collect_trace,
                    forward,
                    trace.lock().unwrap().clone(),
                )
            };
            let old = run(false);
            assert!(old.0);
            assert_eq!(old.1, [0, 1]);
            assert_eq!(old.3, [0, 1, 2]);
            assert_eq!(old.4, 4.0);
            assert_eq!(old, run(true));
        });
    }

    /// With no window to lay a screen out in there is no slot under the pointer.
    #[test]
    #[ignore = "daily"]
    fn no_item_is_hovered_without_a_window() {
        use super::super::{Audio, Game, Options, Progress, Settings, load_resources};

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        let game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        assert_eq!(game.screen_hovered_item(), None);
    }
}
