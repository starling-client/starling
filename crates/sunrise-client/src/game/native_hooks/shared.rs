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
use crate::game::api_values::{TryIntoApi, TryIntoEngine, with_stack_view};
use screen_chars::chars_to_type;

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
use screen_view::{screen_view_for_mod, screen_view_with_id};

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
    pub(super) fn observed_overlay_now(&self) -> Option<&'static str> {
        let book = self.book.as_ref().map(super::books::BookScreen::id);
        observed_overlay_id(&[
            (self.command_prompt.is_some(), "command_prompt"),
            (self.link_prompt.is_some(), "link_prompt"),
            (self.code_of_conduct.is_some(), "code_of_conduct"),
            (self.server_packs.prompt.is_some(), "server_pack_prompt"),
            (self.dialog_open(), "dialog"),
            (book.is_some(), book.unwrap_or("book_view")),
            (self.sign_edit.is_some(), "sign_editor"),
        ])
    }

    pub(super) fn observed_screen_view(&self) -> Option<ScreenView<'_>> {
        let Some(id) = self.observed_overlay_now() else {
            return self.screen_view_now();
        };
        self.renderer_framebuffer()?;
        Some(self.screen_overlay_view(id))
    }

    fn screen_overlay_view(&self, id: &'static str) -> ScreenView<'_> {
        let rect = self.renderer_framebuffer().map_or([0.0; 4], |framebuffer| {
            let batch = self.interface_batch(framebuffer);
            [0.0, 0.0, batch.width, batch.height]
        });
        ScreenView::from(mod_api::__private::built::ScreenView {
            id,
            mod_screen: None,
            menu: None,
            title: None,
            rect,
            slots: 0,
            mouse: self.cursor(),
            partial_tick: self.clock.fraction() as f32,
            regions: &[],
        })
    }

    /// `LocalPlayer.clientSideCloseContainer`: restore the logical inventory
    /// menu, then `Gui.setScreen(null)`. Keep the old screen and its view for
    /// `ScreenEvent.Closing` before running its removal, not its user close.
    pub(super) fn close_screen_to_level(&mut self, show_death_screen: bool) -> super::Result<()> {
        let overlay = self.observed_overlay_now();
        let inventory_screen = overlay.is_none()
            && self.screen == Screen::Inventory
            && self.remote_menu_type().is_none()
            && self.shown_native_screen().is_none();
        let heard = self.screen_hooks_heard();
        let outgoing = heard
            .then(|| {
                if let Some(id) = overlay {
                    Some(self.observe_view(self.screen_overlay_view(id)))
                } else if self.screen != Screen::Playing {
                    Some(self.observe_view(self.screen_change_view(self.screen)))
                } else {
                    self.title
                        .as_ref()
                        .map(|title| self.observe_view(self.screen_overlay_view(title.mode.id())))
                }
            })
            .flatten();
        let detach_native = self.shown_native_screen().is_some();
        self.remote_set_menu(None, None);
        if detach_native && let Some(screen) = self.mod_screen.as_mut() {
            screen.detached = true;
        }
        let has_level = self.remote.as_ref().is_some_and(|r| r.has_level());
        let dead = self
            .player
            .as_ref()
            .is_some_and(|player| player.health.health() <= 0.0);
        let title = if has_level {
            None
        } else {
            Some(
                self.new_title(std::env::current_dir()?.join("saves"))
                    .with_user(|| self.options.player_name()),
            )
        };
        let screen = if has_level && dead && show_death_screen {
            Screen::Dead
        } else {
            if has_level && dead {
                self.respawn_player()?;
            }
            Screen::Playing
        };
        let decision = if heard {
            let none = self.screen_overlay_view(Screen::Playing.id());
            let old = outgoing.as_ref().map(ObservedScreen::view);
            let new = self
                .screen_overlay_view(title.as_ref().map_or(screen.id(), |title| title.mode.id()));
            let answer = self.screen_change(
                old.as_ref().unwrap_or(&none),
                &new,
                title.is_some() || screen != Screen::Playing,
                old.is_some(),
            );
            self.screen_decision_for_context(answer, true, title.is_some() || self.title.is_some())
        } else {
            ScreenDecision::Open
        };
        let (screen, owner, target) = match decision {
            ScreenDecision::Keep => return Ok(()),
            ScreenDecision::ModScreen(owner) => (Screen::Mod, Some(owner), None),
            ScreenDecision::Vanilla(target) => (screen, None, Some(target)),
            ScreenDecision::Open => (screen, None, None),
        };

        self.reset_inventory_clicks();
        self.remove_remote_menu_screen();
        if inventory_screen && let Some(remote) = self.remote.as_mut() {
            // InventoryMenu.removed clears the result, not the client input grid.
            remote.craft_result = None;
        }
        self.remove_sign_editor();
        self.remove_chat_screen(crate::game::chat::ChatExit::Interrupted);
        self.close_dialogs();
        self.book = None;
        self.link_prompt = None;
        self.command_prompt = None;
        self.code_of_conduct = None;
        self.server_packs.prompt = None;
        self.pause_focus = None;
        if self.screen == Screen::Options {
            self.ui.options.discard_draw();
        }
        if screen == Screen::Dead && self.screen != Screen::Dead {
            self.ui.death_time = 0;
            self.ui.death_confirm = None;
        }
        if let Some(mut title) = title {
            title.set_rule_registry(Arc::new(crate::game::api_values::game_rule_registry()));
            self.stage_title(title);
            self.screen = Screen::Playing;
            self.native_mod_screen = None;
        }
        if let Some(target) = target {
            self.open_screen_target(target);
        } else {
            if self.title.is_some() && screen == Screen::Playing {
                self.release_mappings_on_screen_open();
            }
            self.apply_screen_with_mod_book(screen, owner);
        }
        self.capture(
            self.screen == Screen::Playing && self.title.is_none() && !self.screen_over_playing(),
        );
        Ok(())
    }

    /// Opens `screen` (`Gui.setScreen`; `Screen::Playing` is vanilla's null):
    /// the one place the game's screen changes. A different screen runs
    /// `screen_opening` first (`ScreenEvent.Opening`: a mod may keep the
    /// current screen or open its own), then `screen_closing` for the screen
    /// it replaces. A game with no mod listening to either, and a server's
    /// or a guest's `Game`, assign the screen directly. A new native menu or
    /// same-type vanilla container can take over the already shown `Inventory`
    /// screen in place.
    pub(super) fn set_screen(&mut self, screen: Screen) {
        self.set_screen_with(screen, |_| {});
    }
    /// `set_screen`, with `before` run once the hooks let the change go ahead and
    /// before it applies: what a screen's own `onClose` does only when it really
    /// closes (a container screen's close packet).
    pub(super) fn set_screen_with(&mut self, screen: Screen, before: impl FnOnce(&mut Self)) {
        self.transition_screen(screen, false, |game, _| before(game));
    }

    /// `Gui.setScreen(null)`: dismisses the open screen without a vetoable
    /// `ScreenEvent.Opening` (NeoForge only posts `Opening` for a non-null
    /// target), firing `screen_closing` for the screen being left, running
    /// `before`, and returning to `Screen::Playing`.
    pub(in crate::game) fn dismiss_screen_with(&mut self, before: impl FnOnce(&mut Self)) {
        if self.screen == Screen::Playing {
            before(self);
            return;
        }
        if self.screen_hooks_heard() {
            let current = self.screen_change_view(self.screen);
            self.native_screen_closing(&current);
        }
        before(self);
        if self.screen == Screen::Options {
            self.ui.options.discard_draw();
        }
        self.apply_screen_with_mod_book(Screen::Playing, None);
        self.screen_state.observed = self.observed_now();
    }

    /// A new instance can replace the current screen of the same kind. The preparation
    /// sees the accepted screen, or `None` when a replacement opens through its own path.
    pub(in crate::game) fn transition_screen(
        &mut self,
        screen: Screen,
        new_instance: bool,
        before: impl FnOnce(&mut Self, Option<Screen>),
    ) {
        let opening_native =
            screen == Screen::Inventory && self.active_native_screen().is_some_and(|s| !s.shown);
        let new_menu_gen = (screen == Screen::Inventory)
            .then(|| self.remote.as_ref().map(|r| r.menu.generation))
            .flatten();
        let menu_replaced = screen == Screen::Inventory
            && new_menu_gen.is_some()
            && self
                .screen_state
                .observed
                .as_ref()
                .and_then(|o| o.menu_generation)
                != new_menu_gen;
        if self.screen == screen && !new_instance {
            if opening_native || menu_replaced {
                self.apply_screen_with_mod_book(screen, None);
            }
            return;
        }
        let mut screen = screen;
        let mut owner = None;
        if self.screen_hooks_heard() {
            let decision = {
                let observed = new_instance.then(|| self.observed_screen_view()).flatten();
                let current = observed.unwrap_or_else(|| self.screen_change_view(self.screen));
                let new = self.screen_change_view(screen);
                self.decide_screen(&current, &new, current.id != "playing", new_instance)
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
                    if new_instance {
                        self.remove_replaced_overlays();
                    }
                    before(self, None);
                    if opening_native && self.active_native_screen().is_some() {
                        self.remote_close_container();
                    }
                    self.open_screen_target(target);
                    return;
                }
                ScreenDecision::Open => {}
            }
        }
        if self.screen == screen && owner.is_none() && !new_instance {
            return;
        }
        if new_instance {
            self.remove_replaced_overlays();
            self.screen_state.build.reopened();
        }
        before(self, Some(screen));
        if self.screen == Screen::Options {
            self.ui.options.discard_draw();
        }
        self.apply_screen_with_mod_book(screen, owner);
    }

    /// `Screen.removed` on accepted replacement. Do not run onClose actions
    /// such as answering a prompt, following a link, or signing a book.
    fn remove_replaced_overlays(&mut self) {
        self.command_prompt = None;
        self.link_prompt = None;
        self.code_of_conduct = None;
        self.server_packs.prompt = None;
        self.close_dialogs();
        self.book = None;
        self.remove_sign_editor();
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
        removes_overlay: bool,
    ) -> ScreenDecision {
        self.screen_decision(
            self.screen_change(current, new, true, closes),
            removes_overlay,
        )
    }
    /// Like `decide_screen`, for `mod_screen_open`, which clears any active
    /// overlay via `close_replaced_screen_for_mod` before applying the
    /// decision, so `ScreenOpening::ModScreen` and `ScreenOpening::Vanilla`
    /// answers remain eligible even when replacing an overlay.
    fn decide_screen_for_mod_open(
        &self,
        current: &ScreenView<'_>,
        new: &ScreenView<'_>,
        closes: bool,
    ) -> ScreenDecision {
        self.screen_decision(self.screen_change(current, new, true, closes), true)
    }
    /// What a mod's `screen_opening` answer does here: a `ModScreen` only
    /// where a mod's screen can open (`mod_screen_blocked`), a `Vanilla`
    /// target only where it opens (a title screen while the title shows, a
    /// world's screen where a mod's could open in a world), else the change
    /// goes ahead and the refused target is logged. Records whether a mod's
    /// answer changed what shows (`ScreenState::answered`).
    fn screen_decision(
        &self,
        answer: Option<Answered<ScreenOpening>>,
        clears_overlays: bool,
    ) -> ScreenDecision {
        self.screen_decision_for_context(answer, clears_overlays, self.title.is_some())
    }
    fn screen_decision_for_context(
        &self,
        answer: Option<Answered<ScreenOpening>>,
        clears_overlays: bool,
        title_available: bool,
    ) -> ScreenDecision {
        let blocked = self.mod_screen_blocked(clears_overlays, title_available);
        let decision = match answer {
            Some(Answered {
                answer: ScreenOpening::Cancel,
                ..
            }) => ScreenDecision::Keep,
            Some(Answered {
                answer: ScreenOpening::ModScreen,
                mod_id,
                ..
            }) if !blocked => ScreenDecision::ModScreen(mod_id),
            Some(Answered {
                answer: ScreenOpening::Vanilla(target),
                mod_id,
                ..
            }) => {
                let opens = match title_target_id(target) {
                    // An overlay over the title would stay in front of it unless cleared.
                    Some(_) => title_available && (clears_overlays || !self.screen_over_playing()),
                    None => !title_available && !blocked,
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
    /// through): the change fires no screen hook again, and commits once.
    fn open_screen_target(&mut self, target: ScreenTarget) {
        let before_instance = self.screen_state.screen_instance;
        self.screen_state.replacing = true;
        self.screen_state.build.reopened();
        match target {
            ScreenTarget::Pause => {
                self.pause_focus = None;
                self.pause_menu = true;
                self.set_screen(Screen::Paused);
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
            ScreenTarget::Chat => {
                self.begin_chat_screen("", false);
            }
            _ => {
                // A title screen: set the title mode first so `set_screen(Playing)`
                // commits directly into the target title screen without double-init.
                if let (Some(title), Some(id)) = (self.title.as_mut(), title_target_id(target)) {
                    title.open_by_id(id);
                }
                self.release_mappings_on_screen_open();
                self.set_screen(Screen::Playing);
            }
        }
        self.build_screen();
        // Every target takes the pointer, even one opened in place of a
        // request that grabbed none (a mod's screen asked for while playing).
        self.capture(false);
        self.screen_state.replacing = false;
        if self.screen_state.screen_instance == before_instance {
            self.commit_screen_transition();
        }
    }
    /// Commit a screen transition synchronously (`Gui.setScreen` / `Screen.init`):
    /// bumps `screen_instance`, clears `last_click` and outgoing mounted widgets,
    /// rebuilds layout, publishes `screen_state.observed`, and mounts the
    /// incoming screen's widgets immediately.
    pub(super) fn commit_screen_transition(&mut self) {
        self.screen_state.bump_instance();
        self.clear_screen_widgets();
        self.build_screen();
        let observed = self.observed_now();
        self.screen_state.observed = observed.clone();
        if !self.options.execution_role.is_client() || self.lan_acting.is_some() {
            return;
        }
        if let Some(obs) = observed {
            self.init_screen_widgets(&obs.view());
        } else if let Some(obs) = self.headless_screen_snapshot() {
            self.init_screen_widgets(&obs.view());
        }
    }
    fn headless_screen_snapshot(&self) -> Option<ObservedScreen> {
        if self.renderer.is_some() || self.remote_loading() {
            return None;
        }
        if let Some(id) = self.observed_overlay_now() {
            let none = self.screen_change_view(Screen::Playing);
            return Some(self.observe_view(screen_view_with_id(&none, id)));
        }
        if self.screen != Screen::Playing {
            return Some(self.observe_view(self.screen_change_view(self.screen)));
        }
        if let Some(title) = self.title.as_ref() {
            let none = self.screen_change_view(Screen::Playing);
            return Some(self.observe_view(screen_view_with_id(&none, title.mode.id())));
        }
        None
    }
    /// What `set_screen` does once the hooks have answered: `screen` opens,
    /// and the mod `owner`'s own screen when one answered `ModScreen`.
    fn apply_screen(&mut self, screen: Screen, owner: Option<&str>) {
        // A screen's pointer grab ends with the screen, by any route.
        self.pointer_grab_request.set(None);
        if self.screen_grab {
            self.capture(false);
        }
        // `before` may already have closed it. Overlays that keep Inventory
        // underneath don't pass through this replacement path.
        if screen != Screen::Inventory {
            if self.active_native_screen().is_some() {
                self.remote_close_container();
            } else if self.mod_screen.as_ref().is_some_and(|s| s.detached) {
                self.remove_remote_menu_screen();
            }
        }
        if screen != Screen::DemoIntro {
            self.demo_intro = None;
        }
        if self.screen == Screen::Chat && screen != Screen::Chat {
            self.remove_chat_screen(crate::game::chat::ChatExit::Interrupted);
        }
        self.screen = screen;
        // Set again by the Mods screen's Config once its mod screen is open.
        self.native_mod_screen_parent = false;
        if let Some(owner) = owner {
            self.native_mod_screen = Some(owner.to_owned());
        } else if screen != Screen::Mod {
            self.native_mod_screen = None;
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
        self.commit_screen_transition();
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
    fn mod_screen_blocked(&self, removes_overlay: bool, title_available: bool) -> bool {
        self.lan_acting.is_some()
            || (self.player.is_none() && !title_available)
            || (!removes_overlay && self.screen_over_playing())
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
            self.commit_screen_transition();
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
            self.decide_screen(from.as_ref().unwrap_or(&none), &to, from.is_some(), false)
        };
        match decision {
            ScreenDecision::Keep => return false,
            ScreenDecision::ModScreen(mod_id) => {
                self.apply_screen(Screen::Mod, Some(mod_id));
                return false;
            }
            ScreenDecision::Vanilla(target) => {
                self.open_screen_target(target);
                return false;
            }
            ScreenDecision::Open => {}
        }
        open(self);
        self.release_mappings_on_screen_open();
        self.screen_state.build.reopened();
        self.commit_screen_transition();
        true
    }

    /// What `observe_screen` compares with the last frame's.
    pub(super) fn observed_now(&self) -> Option<ObservedScreen> {
        // Only the title's screens and the overlays show over `Screen::Playing`.
        self.observed_screen_view()
            .map(|view| self.observe_view(view))
    }

    /// Snapshot of the receiving screen for screen input hooks, including overlays.
    pub(super) fn observed_input_screen(&self) -> Option<ObservedScreen> {
        let view = self.observed_screen_view()?;
        let regions = view.regions.to_vec();
        let mut snapshot = self.observe_view(view);
        snapshot.regions = regions;
        Some(snapshot)
    }

    /// Runs close cleanup for any screen or overlay being replaced by
    /// `mod_screen_open` once hooks allow the transition, matching vanilla's
    /// `Gui.setScreen` / `Screen.removed` so no container menu, editor, dialog,
    /// or prompt stays open over the new screen.
    fn close_replaced_screen_for_mod(&mut self) {
        let replacing = std::mem::replace(&mut self.screen_state.replacing, true);
        match self.screen {
            Screen::Inventory => self.close_container(),
            Screen::Creative => self.close_creative_menu(),
            Screen::Options => self.ui.options.discard_draw(),
            _ => {}
        }
        if self.book.is_some() {
            self.close_book();
        }
        if self.sign_edit.is_some() {
            self.close_sign_editor();
        }
        if self.dialog_open() {
            self.close_dialogs();
        }
        self.link_prompt = None;
        self.command_prompt = None;
        self.code_of_conduct = None;
        self.server_packs.prompt = None;
        self.screen_state.replacing = replacing;
    }

    pub(super) fn observe_view(&self, view: ScreenView<'_>) -> ObservedScreen {
        let screen_size = if self.renderer.is_some() {
            self.gui_frame_now().1.size.map(|size| size as f32)
        } else {
            [view.rect[2], view.rect[3]]
        };
        ObservedScreen {
            id: view.id,
            overlay: self.screen == Screen::Playing || self.observed_overlay_now().is_some(),
            mod_screen: view.mod_screen.map(str::to_owned),
            menu: view.menu.map(str::to_owned),
            menu_generation: (self.screen == Screen::Inventory && !self.screen_over_playing())
                .then(|| self.remote.as_ref().map(|r| r.menu.generation))
                .flatten(),
            dialog_key: (view.id == "dialog").then(|| match self.dialog_screen_key() {
                (true, _, _) => (true, false, None),
                key => key,
            }),
            title: view
                .title
                .and_then(|_| self.remote.as_ref()?.shared_menu_title_snapshot()),
            rect: view.rect,
            screen_size,
            slots: view.slots,
            mouse: view.mouse,
            partial_tick: view.partial_tick,
            regions: Vec::new(),
        }
    }

    pub(super) fn settle_title_mode(&mut self) {
        let heard = self.screen_hooks_heard();
        let gated = heard
            || (self.options.execution_role.is_client()
                && self.lan_acting.is_none()
                && !self.screen_state.replacing
                && mod_api::__private::listened(mod_api::HookId::ScreenInitPost));
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
        let decision = if heard {
            let none = self.screen_change_view(Screen::Playing);
            let from = screen_view_with_id(&none, current);
            let to = screen_view_with_id(&none, id);
            self.decide_screen(&from, &to, true, false)
        } else {
            ScreenDecision::Open
        };
        match decision {
            ScreenDecision::Keep => (),
            ScreenDecision::ModScreen(mod_id) => self.apply_screen(Screen::Mod, Some(mod_id)),
            ScreenDecision::Vanilla(target) => self.open_screen_target(target),
            ScreenDecision::Open => {
                if let Some(title) = self.title.as_mut() {
                    title.activate_mode(mode);
                }
                self.release_mappings_on_screen_open();
                self.commit_screen_transition();
            }
        }
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
        self.commit_screen_transition();
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
        self.refresh_screen_widgets();
    }

    /// Refresh the receiver's build before another input can reach it.
    pub(super) fn refresh_screen_widgets(&mut self) {
        if self.screen_state.replacing {
            return;
        }
        // Keeps the build on the frame's window size and drops it once nothing
        // shows; an unchanged frame compares and builds nothing.
        self.build_screen();
        if self.screen_state.context_menu.is_some() {
            let _ = self.live_context_menu();
        }
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
    /// follows on opening and when a resize rebuilds the receiver's children.
    fn observe_change(&mut self, current: Option<ObservedScreen>) {
        let changed = match (&self.screen_state.observed, &current) {
            (Some(old), Some(new)) => {
                old.id != new.id
                    || old.menu != new.menu
                    || old.menu_generation != new.menu_generation
                    || old.mod_screen != new.mod_screen
                    || old.dialog_key != new.dialog_key
            }
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
            self.screen_state.bump_instance();
            self.clear_screen_widgets();
            self.screen_state.observed = current.clone();
            if let Some(new) = &current
                && self.options.execution_role.is_client()
                && self.lan_acting.is_none()
            {
                self.init_screen_widgets(&new.view());
            }
            return;
        }
        if let (Some(old), Some(new)) = (&self.screen_state.observed, &current)
            && old.screen_size != new.screen_size
            // Screen.resize calls repositionElements. Screen's default rebuilds
            // widgets; these receivers override it to relayout existing children.
            // A panel rect can also move independently (e.g. opening a recipe book).
            && !matches!(
                new.id,
                "options"
                    | "world_options"
                    | "stats"
                    | "advancements"
                    | "social"
                    | "debug_options"
                    | "dialog"
                    | "dialog_warning"
                    | "confirm_link"
                    | "demo_intro"
                    | "link_prompt"
                    | "code_of_conduct"
                    | "world_select"
                    | "world_create"
                    | "world_create_flat"
                    | "world_create_buffet"
                    | "world_edit"
                    | "world_recover"
                    | "multiplayer"
                    | "multiplayer_warning"
                    | "disconnected"
                    | "onboarding"
            )
        {
            self.clear_screen_widgets();
            self.init_screen_widgets(&new.view());
        }
        self.screen_state.observed = current;
    }
}

/// The pose a native mod asks for the first-person hand, each out-of-range
/// or non-finite component replaced by its identity.
pub(in crate::game) fn hand_pose_matrix(values: [f32; 7]) -> glam::Mat4 {
    let valid = |value: f32, min: f32, max: f32, default: f32| {
        if value.is_finite() && (min..=max).contains(&value) {
            value
        } else {
            default
        }
    };
    let [x, y, z, pitch, yaw, roll, scale] = values;
    glam::Mat4::from_translation(glam::Vec3::new(
        valid(x, -16.0, 16.0, 0.0),
        valid(y, -16.0, 16.0, 0.0),
        valid(z, -16.0, 16.0, 0.0),
    )) * glam::Mat4::from_rotation_x(valid(pitch, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_rotation_y(valid(yaw, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_rotation_z(valid(roll, -3600.0, 3600.0, 0.0).to_radians())
        * glam::Mat4::from_scale(glam::Vec3::splat(valid(scale, 0.01, 100.0, 1.0)))
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
    /// The server closed the logical menu (`LocalPlayer.clientSideCloseContainer`),
    /// while the displayed screen stays whole through `screen_closing` or a veto
    /// until its removal runs (`AbstractContainerScreen` owns its own `menu`).
    detached: bool,
}

/// The menu the screen presents: where its slots are stored, the opener,
/// and the window's live data.
struct OpenNativeMenu {
    key: Pos,
    window: i32,
    generation: u64,
    /// The menu's type, which its screen factory is keyed by.
    name: String,
    /// The server title captured for this screen, kept if the logical menu closes first.
    title: Option<mod_api::Text>,
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

/// How many of a menu's player slots are the main inventory's rows, which
/// come before the hotbar in menu order
/// (`AbstractContainerMenu.addStandardInventorySlots`).
const MAIN_INVENTORY_SLOTS: usize = 27;

/// Where a mod moved the open screen's slots (container slots, then the
/// player's, in menu order) after it opened, over the
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
    /// Where menu slot `slot` will be from the next build: the last move,
    /// pending or committed, else its declaration. `declared` is the screen's
    /// declaration (`OpenNativeScreen::declared_slot`), None for no such slot.
    fn latest(&self, slot: usize, declared: Option<Option<[f32; 2]>>) -> Option<[f32; 2]> {
        let declared = declared?;
        let pending = self.pending.borrow().get(&slot).copied();
        match pending {
            Some(pending) => pending,
            None => self.position(slot, declared),
        }
    }
    /// Moves or activates menu slot `slot` to `pos`, or hides it with None,
    /// from the next build (`Slot.isActive()`). `declared` is as in `latest`.
    /// A refusal or a repeat changes nothing, not even the move count.
    fn set(
        &self,
        slot: usize,
        declared: Option<Option<[f32; 2]>>,
        pos: Option<[f32; 2]>,
    ) -> Result<(), mod_api::ScreenSlotRefusal> {
        use mod_api::ScreenSlotRefusal;
        if declared.is_none() {
            return Err(ScreenSlotRefusal::NoSuchSlot);
        }
        if pos.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite()) {
            return Err(ScreenSlotRefusal::BadPosition);
        }
        if self.latest(slot, declared) != pos {
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
    pub(in crate::game) fn active_native_screen(&self) -> Option<&OpenNativeScreen> {
        let screen = self.mod_screen.as_ref()?;
        let remote = &self.remote.as_ref()?.menu;
        (!screen.detached
            && remote.generation == screen.menu.generation
            && remote.as_ref()?.0 == screen.menu.window)
            .then_some(screen)
    }

    /// The shown native screen whose remote window is still the active logical menu.
    pub(in crate::game) fn shown_active_native_screen(&self) -> Option<&OpenNativeScreen> {
        self.active_native_screen().filter(|s| {
            s.shown && self.screen == Screen::Inventory && self.container == Some(s.menu.key)
        })
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

    /// Where the screen declared menu slot `slot` (container slots, then the
    /// player's main inventory and hotbar, as `AbstractContainerMenu.slots`
    /// holds them): Some(None) for a container slot declared inactive, None
    /// for no such slot.
    fn declared_slot(&self, slot: usize) -> Option<Option<[f32; 2]>> {
        let layout = self.layout();
        if let Some(declared) = layout.slots.get(slot) {
            return Some(declared.as_ref().map(|slot| slot.position));
        }
        let player = slot - layout.slots.len();
        if player >= self.player_slots() {
            return None;
        }
        let (column, row_y) = if player < MAIN_INVENTORY_SLOTS {
            (player % 9, layout.inventory_y + (player / 9) as f32 * 18.0)
        } else {
            (player - MAIN_INVENTORY_SLOTS, layout.inventory_y + 58.0)
        };
        Some(Some([layout.inventory_x + column as f32 * 18.0, row_y]))
    }

    /// Menu slot of the player's inventory slot `slot` (0..9 the hotbar, 9..36
    /// the main inventory), None when the menu has no such player slot.
    fn player_menu_slot(&self, slot: usize) -> Option<usize> {
        if slot >= 9 + MAIN_INVENTORY_SLOTS {
            return None;
        }
        let player = if slot >= 9 {
            slot - 9
        } else {
            MAIN_INVENTORY_SLOTS + slot
        };
        (player < self.player_slots()).then(|| self.layout().slots.len() + player)
    }

    /// Where this frame puts the player's inventory slot `slot`: where the
    /// last build committed it (None once hidden), else its declared place.
    fn player_slot_position(&self, slot: usize) -> Option<[f32; 2]> {
        let menu_slot = self.player_menu_slot(slot)?;
        self.slot_moves
            .position(menu_slot, self.declared_slot(menu_slot)?)
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
        (0..self.player_slots())
            .filter(move |&slot| self.player_slot_position(slot).is_some())
            .chain(self.visible_container_slots())
    }
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

/// Which mouse Post hook owes dispatch (`ScreenMousePost`).
pub(super) enum ScreenMousePostKind {
    Scroll([f64; 2]),
    Drag { button: i32, delta: [f32; 2] },
}

/// The Post a mouse input owes once the screen has handled it, built when
/// its Pre let the input through (`Game::screen_mouse_post`), carrying the
/// receiving screen and modifiers captured at Pre time.
pub(super) struct ScreenMousePost {
    pub(super) receiver: ObservedScreen,
    pub(super) modifiers: u32,
    pub(super) kind: ScreenMousePostKind,
    pub(super) handled: bool,
}

impl ScreenMousePost {
    pub(super) fn new(receiver: ObservedScreen, modifiers: u32, kind: ScreenMousePostKind) -> Self {
        Self {
            receiver,
            modifiers,
            kind,
            handled: false,
        }
    }
}

/// What a drag's Pre decided (`Game::screen_mouse_dragged`).
pub(super) enum ScreenMousePre {
    /// No screen hook ran: no drag, a synthetic move or no screen open.
    NotFired,
    Canceled,
    Passed(ScreenMousePost),
}

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Admission for ordinary toasts. Only an accepted candidate enters its
    /// queue; mandatory callback-panic diagnostics bypass mod admission.
    pub(super) fn admit_toast(
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

    pub(super) fn settle_title_toasts(&mut self) {
        use super::toast_manager::SystemToastId;
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
                if self.title.is_none()
                    || self
                        .toast_manager
                        .system_mut(SystemToastId::Title)
                        .is_some()
                {
                    return None;
                }
                let reason = self.options.offline_reason.take()?;
                Some(
                    sunrise_client::toasts::SystemToast::new(
                        sunrise_client::text::Text::tr("sunrise.title.playing_offline", &[]),
                        Some(sunrise_client::text::Text::literal(reason)),
                    )
                    .with_display_time(sunrise_client::toasts::SystemToast::LONG_DISPLAY_TIME),
                )
            });
        if let Some(toast) = pending {
            self.add_system_toast(Some(SystemToastId::Title), toast);
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
    /// The hearts the local player's health bar draws (`Gui.HeartType.forPlayer`, then
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
        let shown_native = menu_shown.then(|| self.shown_native_screen()).flatten();
        ScreenView::from(mod_api::__private::built::ScreenView {
            id: screen.id(),
            mod_screen: (screen == Screen::Mod).then(|| self.mod_screen()).flatten(),
            menu: self
                .remote_menu_type()
                .filter(|m| menu_shown && !m.is_empty())
                .or_else(|| {
                    shown_native
                        .map(|s| s.menu.name.as_str())
                        .filter(|m| !m.is_empty())
                }),
            title: self
                .remote
                .as_ref()
                .and_then(|r| r.menu_title_snapshot())
                .filter(|_| menu_shown)
                .or_else(|| shown_native.and_then(|s| s.menu.title.as_ref())),
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
            mod_screen: None,
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
    /// `Screen.mouseMoved`, asked before the move's drag: whether a mod cancelled the move.
    /// Nothing is built with no listener. A screen holding the pointer grab hears
    /// `screen_pointer_motion` instead (the grab's own cursor warps are not moves).
    pub(super) fn screen_mouse_moved(&self, delta: [f32; 2]) -> bool {
        if self.pad_synthetic_input()
            || delta == [0.0; 2]
            || self.screen_grab
            || !mod_api::__private::listened(mod_api::HookId::ScreenMouseMovedPre)
        {
            return false;
        }
        let Some(receiver) = self.observed_input_screen() else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_mouse_moved_pre(
            &receiver.view(),
            delta,
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    /// The files of the last drop, fired as one list for the open screen (`Screen.onFilesDrop`):
    /// before the next window event, or when the event-loop pass ends. Nothing is built with no
    /// listener or no screen.
    pub(in crate::game) fn fire_dropped_files(&mut self) {
        if self.dropped_files.is_empty() {
            return;
        }
        let paths = std::mem::take(&mut self.dropped_files);
        if !mod_api::__private::listened(mod_api::HookId::ScreenFilesDropped) {
            return;
        }
        if let Some(receiver) = self.observed_input_screen() {
            self.native_screen_files_dropped(&receiver.view(), &paths);
        }
    }
    /// The grabbing mod screen's raw pointer motion (`screen_pointer_motion`), unscaled.
    pub(in crate::game) fn screen_pointer_motion(&self, delta: [f64; 2]) {
        if delta == [0.0; 2]
            || !self.mod_screen_grabs_pointer()
            || !mod_api::__private::listened(mod_api::HookId::ScreenPointerMotion)
        {
            return;
        }
        if let Some(receiver) = self.observed_input_screen() {
            self.native_screen_pointer_motion(&receiver.view(), delta);
        }
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
        let Some(receiver) = self.observed_input_screen() else {
            return ScreenMousePre::NotFired;
        };
        let button = super::glfw_button(button);
        let modifiers = super::glfw_modifiers(self.modifiers);
        let mut canceled = false;
        self.native_screen_mouse_dragged_pre(
            &receiver.view(),
            button,
            delta,
            modifiers,
            &mut canceled,
        );
        if canceled {
            ScreenMousePre::Canceled
        } else {
            ScreenMousePre::Passed(ScreenMousePost::new(
                receiver,
                modifiers,
                ScreenMousePostKind::Drag { button, delta },
            ))
        }
    }
    /// Button Post sees the original receiver and its answer; return the final handled bit.
    pub(super) fn screen_button_post(
        &self,
        receiver: &ObservedScreen,
        button: i32,
        pressed: bool,
        modifiers: u32,
        double_click: bool,
        handled: bool,
    ) -> bool {
        let screen = receiver.view();
        let mut result = TriState::Default;
        if pressed {
            self.native_screen_mouse_button_pressed_post(
                &screen,
                button,
                modifiers,
                double_click,
                handled,
                &mut result,
            );
        } else {
            self.native_screen_mouse_button_released_post(
                &screen,
                button,
                modifiers,
                handled,
                &mut result,
            );
        }
        result.or(handled)
    }

    /// Unhandled scroll/drag notices go to the receiver captured before handling.
    pub(super) fn screen_mouse_post(&self, post: ScreenMousePost) {
        if post.handled {
            return;
        }
        let screen = post.receiver.view();
        match post.kind {
            ScreenMousePostKind::Scroll(scroll) => {
                self.native_screen_mouse_scrolled_post(&screen, scroll, post.modifiers);
            }
            ScreenMousePostKind::Drag { button, delta } => {
                self.native_screen_mouse_dragged_post(&screen, button, delta, post.modifiers);
            }
        }
    }
    /// Whether a Post of the key `key` over the receiving screen `receiver`
    /// cancelled it (`ScreenEvent.KeyPressed.Post` / `KeyReleased.Post`): asked
    /// where the key neither mounted widgets nor screen arms took reaches the
    /// game's own key handling, which a cancel then skips. False with no
    /// subscriber, no receiving screen, or a key with no vanilla name.
    pub(super) fn screen_key_post_on(
        &self,
        receiver: Option<&ObservedScreen>,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
    ) -> bool {
        let hook = if pressed {
            mod_api::HookId::ScreenKeyPressedPost
        } else {
            mod_api::HookId::ScreenKeyReleasedPost
        };
        if !mod_api::__private::listened(hook) {
            return false;
        }
        let (Some(name), Some(receiver)) = (keybinds::key_name(key), receiver) else {
            return false;
        };
        let modifiers = super::glfw_modifiers(self.modifiers);
        let mut canceled = false;
        let screen = receiver.view();
        if pressed {
            self.native_screen_key_pressed_post(&screen, name, repeat, modifiers, &mut canceled);
        } else {
            self.native_screen_key_released_post(&screen, name, modifiers, &mut canceled);
        }
        canceled
    }
    /// `ScreenEvent.CharacterTyped.Post` for a character that neither mounted
    /// widgets nor any text box took, on the captured receiving screen.
    fn screen_chars_post_on(&self, receiver: Option<&ObservedScreen>, c: char) {
        if !mod_api::__private::listened(mod_api::HookId::ScreenCharTypedPost) {
            return;
        }
        let Some(receiver) = receiver else {
            return;
        };
        let modifiers = super::glfw_modifiers(self.modifiers);
        self.native_screen_char_typed_post(&receiver.view(), c, modifiers);
    }
    /// A screen the native mods' input hooks are for is open: the window has a
    /// renderer, either the title's own (`title`, `world_select`,
    /// `world_create`, `multiplayer`…) or a screen over the world
    /// (`Screen::Playing` behind it, so not a book, a sign editor, a server
    /// dialog or a prompt drawn over gameplay), and no connect or world load
    /// is in progress. The one gate they all ask before they fire.
    pub(super) fn screen_hooks_open(&self) -> bool {
        self.has_renderer()
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
        let framebuffer = self.renderer_framebuffer()?;
        let batch = self.interface_batch(framebuffer);
        if self.screen != Screen::Playing {
            return Some(self.screen_view(&batch, self.screen));
        }
        self.title_screen_view(&batch)
    }
    /// Whether the key press `key` over the receiving screen went no further
    /// (`native_screen_key_pressed_pre`): what `handle_window_event` and the
    /// control socket's `key` command both ask before a screen acts on a key.
    /// False when no receiving screen is open, and for a key with no vanilla name.
    pub(super) fn screen_key_pressed_on(
        &self,
        receiver: Option<&ObservedScreen>,
        key: KeyCode,
        repeat: bool,
    ) -> bool {
        let (Some(name), Some(receiver)) = (keybinds::key_name(key), receiver) else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_key_pressed_pre(
            &receiver.view(),
            name,
            repeat,
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    pub(super) fn screen_key_released_on(
        &self,
        receiver: Option<&ObservedScreen>,
        key: KeyCode,
    ) -> bool {
        let (Some(name), Some(receiver)) = (keybinds::key_name(key), receiver) else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_key_released_pre(
            &receiver.view(),
            name,
            super::glfw_modifiers(self.modifiers),
            &mut canceled,
        );
        canceled
    }
    /// `native_screen_char_typed_pre` for one character on `receiver`, with
    /// the focused box's edit: what the listeners leave of `field` goes back
    /// to the box before the character does. Whether they cancelled the character.
    fn screen_char_canceled_on(&mut self, receiver: Option<&ObservedScreen>, c: char) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::ScreenCharTypedPre) {
            return false;
        }
        let Some(receiver) = receiver else {
            return false;
        };
        let modifiers = super::glfw_modifiers(self.modifiers);
        self.with_focused_text_edit(|game, field| {
            let screen = receiver.view();
            let mut canceled = false;
            game.native_screen_char_typed_pre(&screen, c, modifiers, field, &mut canceled);
            canceled
        })
    }
    /// Text into a text box a character at a time (`Screen.charTyped`): each
    /// printable one snapshots the current receiving screen, is offered to
    /// `native_screen_char_typed_pre`, and `take` gets it unless cancelled,
    /// before the next is offered. A control character goes straight to `take`.
    pub(super) fn screen_text_each(&mut self, text: &str, mut take: impl FnMut(&mut Self, &str)) {
        let mut encoded = [0; 4];
        for c in text.chars() {
            if c.is_control() {
                take(self, c.encode_utf8(&mut encoded));
                continue;
            }
            let receiver = self.observed_input_screen();
            let initial_instance = self.screen_state.screen_instance;
            if !self.screen_char_canceled_on(receiver.as_ref(), c)
                && self.screen_state.screen_instance == initial_instance
            {
                take(self, c.encode_utf8(&mut encoded));
            }
        }
    }
    /// Deliver each printable character of `text` through `CharacterTyped.Pre`,
    /// mounted screen widgets first, then `take` (the vanilla text box sink,
    /// returning whether it handled the character), and finally
    /// `CharacterTyped.Post` on the captured receiving screen ONLY when
    /// unhandled. Snapshots the receiving screen per character (matching
    /// vanilla `KeyboardHandler.charTyped` reading `minecraft.screen` each call)
    /// and never falls through into a replacement screen opened mid-character.
    pub(super) fn screen_chars_dispatch(
        &mut self,
        text: &str,
        mut take: impl FnMut(&mut Self, &str) -> bool,
    ) {
        let modifiers = super::glfw_modifiers(self.modifiers);
        let mut encoded = [0; 4];
        for c in text.chars() {
            if c.is_control() {
                continue;
            }
            if self.observed_screen_view().is_none() && self.headless_screen_snapshot().is_none() {
                break;
            }
            let receiver = self.observed_input_screen();
            let initial_instance = self.screen_state.screen_instance;
            if self.screen_char_canceled_on(receiver.as_ref(), c) {
                continue;
            }
            let mut handled = self.screen_state.screen_instance == initial_instance
                && self.mounted_widget_char_typed(c, modifiers);
            if !handled && self.screen_state.screen_instance == initial_instance {
                handled = take(self, c.encode_utf8(&mut encoded));
            }
            if !handled {
                self.screen_chars_post_on(receiver.as_ref(), c);
            }
        }
    }
    /// A key press's characters (`ScreenEvent.CharacterTyped.Pre`, vanilla's
    /// `KeyboardHandler.charTyped`), dispatched once the screen has acted on
    /// the key: GLFW's char callback comes after its key callback, so the
    /// characters go to the screen that is open now — a key that closed the
    /// screen leaves none to hear them, as vanilla's closed screen hears
    /// nothing, and one that moved the focus types into the box it moved to.
    /// Each printable one snapshots the current screen, runs
    /// `native_screen_char_typed_pre` (when hooks apply), then mounted widgets,
    /// then the focused text box, and unhandled characters fire
    /// `native_screen_char_typed_post` on that character's captured receiver.
    pub(super) fn screen_chars_typed(&mut self, text: Option<&str>) {
        let Some(text) = text else {
            return;
        };
        if self.observed_screen_view().is_none() && self.headless_screen_snapshot().is_none() {
            return;
        }
        // A held edit modifier makes a shortcut of the key, not text.
        let text = chars_to_type(text, self.types_text());
        if text.is_empty() {
            return;
        }
        self.screen_chars_dispatch(&text, |game, c| game.type_chars_into_screen(c).is_ok());
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
    /// to the parts, and the writes a landing (`Block.fallOn`) or step (`Block.stepOn`)
    /// asked for to the level. A write is skipped when the block at its position is no
    /// longer the state it touched, as vanilla never sees a stale one.
    pub(super) fn drain_entity_inside(&mut self) {
        for contact in sunrise_client::block_contact::take_pending() {
            let (state, pos) = match &contact {
                Pending::Landed(landed) => (landed.state, landed.position),
                Pending::MinecartPass(pass) => (pass.state, pass.position),
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
                self.level_destroy_block(level, at, drop, None, None, false);
            }
            // A write a later API adds that this game does not know is not applied.
            _ => {}
        }
    }
}

/// `native_living_use_totem` for the damage code: whether it
/// canceled the totem. The entity and the source are named as the damage hook names them.
pub(super) fn use_totem(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
    hand: Hand,
    stack: &ItemStack,
) -> bool {
    if !mod_api::__private::listened(mod_api::HookId::LivingUseTotem) {
        return false;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut cancel = false;
    HitLevel(level).native_living_use_totem(entity, kind, &source, hand, stack, &mut cancel);
    cancel
}

/// `native_living_death_protection` for the damage code: a mod's protector from outside the
/// hands, once neither hand protected.
pub(super) fn death_protection_source(
    level: DamageLevel<'_>,
    hurting: Hurting<'_>,
) -> Option<ItemStack> {
    if !mod_api::__private::listened(mod_api::HookId::LivingDeathProtection) {
        return None;
    }
    let (entity, kind, source) = hurt_server_args(hurting, level.attack());
    let mut protector = None;
    HitLevel(level).native_living_death_protection(entity, kind, &source, &mut protector);
    protector
}

/// The level a hit or a landing happens in, as its hooks' level host: its blocks, its random and its
/// own attachments. Not `GameLevel`, which a hit can't build: a mob's hit lands in the mob
/// tick, where the world is read by every mob and its random is lent out, and building one
/// per hit would allocate its entity boxes.
pub(super) struct HitLevel<'a>(pub(super) DamageLevel<'a>);

impl<'a> HitLevel<'a> {
    /// The item table a mod's stack is checked against as it comes back into the hit
    /// (`living_death_protection`'s protector): the authoritative world's, as for the hit's reads.
    pub(in crate::game) fn stack_boundary_items(&self) -> Option<&'a sunrise_client::items::Items> {
        self.0.tags()
    }
}

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
    let weapon = attack.weapon_of(source.direct);
    let causing_weapon = (source.causing != 0 && source.causing == attack.attacker)
        .then_some(attack.held[0].as_ref())
        .flatten();
    with_hit_weapons(weapon, causing_weapon, |weapon, causing_weapon| {
        let mut live = LiveDamageContainer(container);
        HitLevel(level.caused_by(source.causing)).native_living_incoming_damage(
            mod_api::__private::LivingIncomingDamageCall {
                entity,
                kind,
                source: &source,
                victim: &victim,
                weapon,
                causing_weapon,
                container: mod_api::DamageContainer::new(&mut live),
                canceled: &mut canceled,
            },
        );
    });
    canceled
}

/// One melee weapon can be both the direct and the causing weapon. Feed both fields the
/// same view so reading either doesn't encode that stack's components again.
fn with_hit_weapons(
    direct: Option<&ItemStack>,
    causing: Option<&ItemStack>,
    dispatch: impl FnOnce(Option<&mod_api::ItemStackView<'_>>, Option<&mod_api::ItemStackView<'_>>),
) {
    with_stack_view(direct, |direct_view| {
        if direct.zip(causing).is_some_and(|(a, b)| std::ptr::eq(a, b)) {
            dispatch(direct_view.as_ref(), direct_view.as_ref());
        } else {
            with_stack_view(causing, |causing_view| {
                dispatch(direct_view.as_ref(), causing_view.as_ref());
            });
        }
    });
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
    HitLevel(level.caused_by(source.causing)).native_living_damage_pre(
        mod_api::__private::LivingDamagePreCall {
            entity,
            kind,
            source: &source,
            container: mod_api::DamageContainer::new(&mut live),
        },
    );
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
    HitLevel(level.caused_by(source.causing)).native_living_damage_post(
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
        use sunrise_client::health::{HitLend, LivingBody, PlayerBody, PlayerHealthRead};
        let (source, state, original_lend) = post.view();
        // The victim as the death or protection callback left it, beside the lent attacker:
        // the player in place, a mob split from the attacker in the same list, or the dragon.
        let dragon_victim = self
            .dragon
            .as_ref()
            .filter(|d| d.id as i32 == entity)
            .is_some_and(|d| !self.target_removed(&crate::game::commands::Target::Dragon(d.id)));
        let victim_mob = self.mobs.iter().position(|m| m.id as i32 == entity);
        // The Post record exists only for a Post subscriber, and names an attacker only if the hit
        // lent one: it carries the setup's decision.
        let lend_mob = original_lend
            .and_then(|id| self.mobs.iter().position(|m| m.id as i32 == id))
            .filter(|&index| Some(index) != victim_mob);
        // A parked guest the hit lent, lent again if its reads still answer from its record.
        let parked = original_lend
            .filter(|_| lend_mob.is_none())
            .and_then(|id| self.native_parked_causer(id));
        let (victim_mob, lend_mob) = match (victim_mob, lend_mob) {
            (Some(v), Some(l)) => {
                let [v, l] = self
                    .mobs
                    .get_disjoint_mut([v, l])
                    .expect("two different mobs");
                (Some(&*v), Some(l))
            }
            (None, Some(l)) => (None, Some(&mut self.mobs[l])),
            (Some(v), None) => (Some(&self.mobs[v]), None),
            (None, None) => (None, None),
        };
        let player_victim = self.player.as_ref().is_some_and(|p| p.entity_id == entity);
        let hub = self.lan.as_ref().map(|lan| &lan.hub);
        let parked = move || HitLend::parked_guest(hub, parked?);
        let (victim, lend) = if player_victim {
            let p = self.player.as_ref().expect("the player victim");
            let body = LivingBody::Player(PlayerBody {
                id: p.entity_id,
                uuid: None,
                health: PlayerHealthRead::Synced(&p.health.data, p.health.ids),
                attributes: Some(&p.attributes),
                effects: Some(&self.potions.effects),
                equipment: acting_equipment!(self),
                attachments: None,
            });
            (Some(body), lend_mob.and_then(HitLend::mob).or_else(parked))
        } else {
            let lend = match lend_mob {
                Some(mob) => HitLend::mob(mob),
                None => original_lend
                    .and_then(|id| {
                        HitLend::attacking_player(
                            self.player.as_mut(),
                            id,
                            &self.potions.effects,
                            acting_equipment!(self),
                        )
                    })
                    .or_else(parked),
            };
            let victim = victim_mob.map(LivingBody::Mob).or_else(|| {
                dragon_victim
                    .then(|| self.dragon.as_ref())
                    .flatten()
                    .map(crate::game::native_living_body::dragon_body)
            });
            (victim, lend)
        };
        let level = crate::game::entity::damage_level_of!(
            self,
            &sunrise_client::health::NO_ATTACK,
            no_push,
            lend
        );
        let level = match victim {
            Some(victim) => level.reborrow().with_victim(victim),
            None => level.reborrow(),
        };
        let live = LiveDamageRead(state);
        HitLevel(level.caused_by(source.causing)).native_living_damage_post(
            entity,
            kind,
            &source,
            mod_api::DamageContainerView::new(&live),
        );
    }

    /// The [`Attack`] of `attacker` (an entity id, 0 for none) for the level a hit lands in
    /// ([`DamageLevel::attacked_by`]). Record the attacker's hand state only if a
    /// subscribed hit, death or loot hook may read it, or when the main hand carries
    /// `minecraft:armor_effectiveness`, which the hit reads as its weapon. Capture before
    /// the level borrows the game so nested damage can't substitute a different attacker's
    /// hands.
    pub(super) fn attack_of(&self, attacker: i32) -> Attack {
        let listens = [
            mod_api::HookId::LivingIncomingDamage,
            mod_api::HookId::LivingKnockback,
            mod_api::HookId::LivingDeath,
            mod_api::HookId::LootDrops,
        ]
        .into_iter()
        .any(mod_api::__private::listened);
        let enchantments = &world_stations!(self).enchantments;
        let main = |stack: Option<&sunrise_client::items::ItemStack>| {
            stack
                .filter(|stack| {
                    listens || enchantments.has_component(stack, "minecraft:armor_effectiveness")
                })
                .cloned()
        };
        let held = if self
            .player
            .as_ref()
            .is_some_and(|p| p.entity_id == attacker)
        {
            [
                main(self.stack_in(self.selected)),
                listens.then(|| self.offhand.clone()).flatten(),
            ]
        } else if let Some(index) = self.mob_index.find_ref(&self.mobs, attacker as u32) {
            let mob = &self.mobs[index];
            [main(mob.extra.main_hand_stack()), None]
        } else if let Some(guest) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.entity_id == attacker))
        {
            [
                main(guest.inventory.get(guest.selected).and_then(Option::as_ref)),
                listens.then(|| guest.offhand.clone()).flatten(),
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
            projectile_type: None,
            direct: None,
            direct_kind: None,
            direct_type: None,
            position: None,
            projectile_weapon: None,
            attacker_uuid: (player_uuid.is_none() && attacker != 0)
                .then(|| self.known_attacker_uuid(attacker as u32))
                .flatten(),
            player_uuid,
            enchantment_attacker: self.enchantment_actor(attacker),
            enchantment_direct: None,
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
    let (level, kill_credit, credit_main) = level.split_knockback_credit(&pushed);
    let (entity, kind, source) = hurt_server_args(pushed, attack);
    let mut canceled = false;
    let [main, off] = &attack.held;
    let read_metadata = |slot: &str, stack: Option<&sunrise_client::items::ItemStack>| {
        let stack = stack?;
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return None;
        }
        match crate::game::api_values::stack_read_metadata(stack) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                log::warn!(
                    "living_knockback on entity {entity} ({kind}): omitting unrepresentable {slot} '{}' (count {}): {error:?}",
                    stack.item,
                    stack.count,
                );
                None
            }
        }
    };
    let main_metadata = read_metadata("attacker_main", main.as_ref());
    let off_metadata = read_metadata("attacker_off", off.as_ref());
    let credit_metadata = read_metadata("kill_credit_main", credit_main);
    let project_main = |metadata| {
        main.as_ref()
            .and_then(|stack| crate::game::api_values::project_stack(stack, metadata).ok())
    };
    let project_off = |metadata| {
        off.as_ref()
            .and_then(|stack| crate::game::api_values::project_stack(stack, metadata).ok())
    };
    let project_credit = |metadata| {
        credit_main.and_then(|stack| crate::game::api_values::project_stack(stack, metadata).ok())
    };
    let attacker_main =
        main_metadata.map(|metadata| mod_api::ItemStackView::resident(metadata, &project_main));
    let attacker_off =
        off_metadata.map(|metadata| mod_api::ItemStackView::resident(metadata, &project_off));
    let kill_credit_main =
        credit_metadata.map(|metadata| mod_api::ItemStackView::resident(metadata, &project_credit));
    // NeoForge `LivingEntity.knockback` posts `(float) strength` and widens the result back.
    let mut strength = knock.strength as f32;
    HitLevel(level).native_living_knockback(mod_api::__private::LivingKnockbackCall {
        entity,
        kind,
        source: &source,
        attacker,
        attacker_main,
        attacker_off,
        kill_credit,
        kill_credit_main,
        strength: &mut strength,
        x: &mut knock.x,
        z: &mut knock.z,
        canceled: &mut canceled,
    });
    knock.strength = f64::from(strength);
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
    protector: Some(death_protection_source),
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
                    &crate::game::native_surface::FrictionView(level, None),
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
        inside_fire: |level, dimension, entity, kind, state, pos, precise, effects| {
            HitLevel(level.reborrow()).native_entity_inside(
                crate::game::native_hooks::fire::EntityInsideCall {
                    dimension,
                    entity,
                    kind,
                    state,
                    pos,
                    precise,
                    effects,
                },
            );
        },
        state_to_api: |id| id.try_into_api().ok(),
        state_to_engine: |state| state.try_into_engine(&()).ok(),
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
    positions: &[[f64; 3]],
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
        && positions.iter().flatten().all(|v| v.is_finite())
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
/// Both game modes let FAIL and PASS go to the item. SUCCESS_SERVER
/// consumes without a swing on the predicting client.
pub(super) fn block_use_result(
    answer: mod_api::InteractionResult,
    side: LogicalSide,
) -> Option<sunrise_world::level::InteractionResult> {
    use mod_api::InteractionResult as Answer;
    use sunrise_world::level::InteractionResult;
    match answer {
        Answer::SuccessServer if side == LogicalSide::Client => Some(InteractionResult::Consume),
        Answer::Success | Answer::SuccessServer => Some(InteractionResult::Success),
        Answer::Consume => Some(InteractionResult::Consume),
        Answer::TryWithEmptyHand => None,
        _ => Some(InteractionResult::Pass),
    }
}

/// The captured target or hand changed during client block-use prediction.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) struct StaleBlockClick;

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

    pub(in crate::game) fn placement_canceled(
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
        let placer = by_player.then(|| self.native_clicker().1.cast_unsigned());
        self.with_level(|b, level, ticks| b.report_placement(level, ticks, &placed, placer));
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
    ) -> Result<Option<mod_api::InteractionResult>, StaleBlockClick> {
        if let Some(answer) =
            self.use_item_on_block(hit, hand, Some(stack), UsePhase::ItemBeforeBlock)
        {
            return Ok(Some(answer));
        }
        if !mod_api::__private::item_listens(mod_api::HookId::OnItemUseFirst, &stack.item) {
            return Ok(None);
        }
        let Ok(snapshot) = stack.try_into_api() else {
            return Ok(None);
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let click = self.block_click(hit, hand, Some(&snapshot));
        self.with_block_click_part(&click, state, Some(stack), |game, answer| {
            game.native_on_item_use_first(&snapshot, &click, answer);
        })
    }

    /// Run the captured block's selected behaviour, only if that key subscribes.
    pub(super) fn predict_block_part(
        &mut self,
        hit: &super::Hit,
        state: BlockStateId,
        hand: Hand,
        held: Option<&ItemStack>,
        branch: sunrise_world::level::UseBranch,
    ) -> Result<Option<mod_api::InteractionResult>, StaleBlockClick> {
        use sunrise_world::level::UseBranch;
        let hook = match branch {
            UseBranch::ItemOn => mod_api::HookId::UseItemOn,
            UseBranch::WithoutItem => mod_api::HookId::UseWithoutItem,
        };
        if !super::api_values::block_state::block_listens(hook, state) {
            return Ok(None);
        }
        let Ok(snapshot) = held.try_into_api() else {
            return Ok(None);
        };
        let click = self.block_click(hit, hand, snapshot.as_ref());
        self.with_block_click_part(&click, state, held, |game, answer| match branch {
            UseBranch::ItemOn => game.native_use_item_on(state, &click, answer),
            UseBranch::WithoutItem => game.native_use_without_item(state, &click, answer),
        })
    }

    /// A refused payload defers to the template. A stale decision must not
    /// be applied or retried against a different block or hand. The server's
    /// Parts may write and deliberately do not have this prediction guard.
    pub(in crate::game) fn with_block_click_part(
        &mut self,
        click: &BlockClick<'_>,
        state: BlockStateId,
        held: Option<&ItemStack>,
        dispatch: impl FnOnce(&mut Self, &mut Option<mod_api::InteractionResult>),
    ) -> Result<Option<mod_api::InteractionResult>, StaleBlockClick> {
        let predicting = click.side == LogicalSide::Client;
        if predicting
            && (!click.hit.point.into_iter().all(f64::is_finite)
                || !Direction::ALL
                    .into_iter()
                    .any(|face| face.normal() == click.hit.face))
        {
            return Ok(None);
        }
        let [x, y, z] = click.pos;
        let selected = self.selected;
        let epoch = self.native_lifecycle.epoch();
        let current = |game: &Self| {
            game.remote.is_some()
                && game.native_lifecycle.epoch() == epoch
                && game.dim.level() == click.dimension
                && game.world.block(x, y, z) == state
                && match click.hand {
                    Hand::Main => game.selected == selected && game.stack_in(selected) == held,
                    Hand::Off => game.offhand.as_ref() == held,
                }
        };
        if predicting && !current(self) {
            return Err(StaleBlockClick);
        }
        let mut answer = None;
        dispatch(self, &mut answer);
        if predicting && !current(self) {
            return Err(StaleBlockClick);
        }
        Ok(answer)
    }
    /// `GameRuleChangedEvent` once the server set the rule `name` to `value`
    /// (`GameRules.set` with a server).
    pub(super) fn game_rule_changed(
        &mut self,
        name: &str,
        value: GameRuleValue,
        actor: Option<PlayerRef>,
    ) {
        // MinecraftServer.onGameRuleChanged refreshes the chunk source for this
        // rule, but not when spawn_mobs itself changes.
        if name == "minecraft:spawn_monsters" {
            self.update_mob_spawning_flags();
        }
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
    pub(super) fn admit_new_mobs(&mut self, from: usize) {
        // Preserve saved or constructed UUIDs. Insertion paths without an
        // identity still need one before their join listeners run.
        for mob in self.mobs.iter().skip(from) {
            self.persist
                .uuids
                .get_or_insert_with(mob.id, sunrise_client::save::random_uuid);
        }
        self.flush_pending_mob_parts();
        self.join_spawn_tags(from);
        self.native_joins_flush(false, 0);
    }
    /// The acting player's hit landed on mob `index`, holding `stack`.
    pub(super) fn native_hurt_enemy_event(&mut self, stack: &ItemStack, index: usize) {
        if !mod_api::__private::item_listens(mod_api::HookId::HurtEnemy, &stack.item) {
            return;
        }
        let mob = &self.mobs[index];
        let entity = mob.id;
        // MobTypes retains the canonical registry id for vanilla and content types alike.
        let kind = mob.kind.clone();
        with_stack_view(Some(stack), |view| {
            if let Some(view) = view {
                self.native_hurt_enemy(view, entity, &kind.id);
            }
        });
    }
    /// `Inventory.tick` for the acting player: each carried stack goes through
    /// `native_inventory_tick`.
    pub(super) fn native_inventory_ticks(&mut self) {
        if self.player.is_none()
            || !mod_api::__private::has_part_hook(mod_api::HookId::InventoryTick)
        {
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
        let Some(stack) = self.native_carried_slot(slot).and_then(|s| s.as_ref()) else {
            return;
        };
        if stack.count > 0
            && mod_api::__private::item_listens(mod_api::HookId::InventoryTick, &stack.item)
            && let Some(item) = mod_api::__private::item_key(&stack.item)
        {
            let place = mod_api::ItemPlace::PlayerSlot {
                player: self.native_acting_id(),
                slot,
            };
            self.native_inventory_tick(item, &place, slot, selected);
        }
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
                if !mod_api::__private::is_content_block(&state.name) {
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
    /// (`native_block_entity_container`).
    pub(super) fn native_containers(&self) -> FxHashMap<u32, Arc<ModContainer>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModContainer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if !mod_api::__private::is_content_block(&state.name) {
                continue;
            }
            let container = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut container = None;
                self.native_block_entity_container(&state.name, &mut container);
                container.map(|mut c| {
                    let n = c.slots;
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
    /// (`native_block_entity_fluids`).
    pub(super) fn native_fluid_tables(&self) -> FxHashMap<u32, Arc<ModFluidTanks>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModFluidTanks>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if !mod_api::__private::is_content_block(&state.name) {
                continue;
            }
            let tanks = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut fluids = None;
                self.native_block_entity_fluids(&state.name, &mut fluids);
                fluids.map(Arc::new)
            });
            if let Some(t) = tanks {
                out.insert(i, Arc::clone(t));
            }
        }
        out
    }
    /// The energy buffers of the mod blocks' block entities, by block state
    /// (`native_block_entity_energy`).
    pub(super) fn native_energy_tables(&self) -> FxHashMap<u32, Arc<ModEnergyBuffer>> {
        let registry = self.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModEnergyBuffer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if !mod_api::__private::is_content_block(&state.name) {
                continue;
            }
            let buffer = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut energy = None;
                self.native_block_entity_energy(&state.name, &mut energy);
                energy.map(Arc::new)
            });
            if let Some(b) = buffer {
                out.insert(i, Arc::clone(b));
            }
        }
        out
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
    /// when no mod screen is showing for that active remote window.
    pub(super) fn native_screen_data(&mut self, window: i32, index: usize, value: i32) -> bool {
        if self.shown_active_native_screen().is_none() {
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
    /// Where the open mod screen puts the player's inventory slot `slot` (0..9
    /// the hotbar, 9..36 the main inventory) this frame; None when hidden or
    /// when no mod screen is shown.
    pub(in crate::game) fn native_screen_player_slot(&self, slot: usize) -> Option<(f32, f32)> {
        let [x, y] = self.shown_native_screen()?.player_slot_position(slot)?;
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
    /// Where the open screen of mod menu type `menu` puts menu slot `slot`
    /// (container slots, then the player's) from the next frame: the mod's last
    /// move, else the screen's declaration; None for an inactive/hidden slot,
    /// another menu type's screen or none.
    pub(in crate::game) fn screen_slot(&self, menu: &str, slot: usize) -> Option<[f32; 2]> {
        let s = self.shown_native_screen().filter(|s| s.menu.name == menu)?;
        s.slot_moves.latest(slot, s.declared_slot(slot))
    }
    /// Moves or activates menu slot `slot` (container slots, then the player's)
    /// of the open screen of menu type
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
        s.slot_moves.set(slot, s.declared_slot(slot), pos)
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
        // Numeric container IDs can be reused. Send only for the active opening that heard the press.
        if !self
            .shown_active_native_screen()
            .is_some_and(|screen| (screen.menu.window, screen.menu.generation) == opening)
            || !self.remote_button_click(button)
        {
            return false;
        }
        self.play_audio_button_click();
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
            row.native = Some(NativePage {
                mappings,
                settings,
                own_screen: mod_api::__private::has_config_screen(&row.id),
            });
            self.settings.mods.installed.push(row);
        }
    }
    /// The settings mod `mod_id` registered, with their live values from the
    /// authoritative client settings store, as its page shows them.
    fn native_page_settings(&self, mod_id: &str) -> Vec<NativeSetting> {
        mod_api::__private::settings()
            .iter()
            .filter(|(owner, _)| *owner == mod_id)
            .map(|(owner, spec)| {
                let value = mod_settings::ui_setting_value(spec, self.live_setting(owner, spec));
                NativeSetting {
                    key: spec.key.clone(),
                    label: spec.label.clone(),
                    tooltip: spec.tooltip.clone(),
                    value,
                    category: mod_api::__private::setting_category(mod_id, &spec.key)
                        .map(str::to_owned),
                }
            })
            .collect()
    }
    /// Setting `j` of native mod `i`'s page (its index in Options > Mods) changed on the page:
    /// committed through the single authoritative client settings path, which persists,
    /// updates live state, and notifies the owning mod (`setting_changed`).
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
        let Some(handle) = mod_api::__private::setting_key_by_name(&id, &key) else {
            return;
        };
        let Some((owner, spec)) =
            mod_api::__private::setting_entry(handle).map(|(owner, spec)| (*owner, spec))
        else {
            return;
        };
        let root = self.native_settings_root();
        if self
            .apply_native_setting_write(&root, owner, spec, value)
            .is_err()
        {
            // Keep the Options > Mods widget aligned with the live store if persistence fails.
            let live = self.live_setting(owner, spec);
            if let Some(setting) = self
                .settings
                .mods
                .installed
                .get_mut(i)
                .and_then(|row| row.native.as_mut())
                .and_then(|page| page.settings.get_mut(j))
            {
                setting.value = mod_settings::ui_setting_value(spec, live);
            }
        }
    }
    /// Native mod `i`'s page opened: refresh its live values from disk through
    /// the single authoritative settings store (notifying on external changes)
    /// and rebuild its page rows from the store.
    pub(super) fn native_settings_refresh(&mut self, i: usize) {
        let Some(id) = self.settings.mods.installed.get(i).map(|m| m.id.clone()) else {
            return;
        };
        let root = self.native_settings_root();
        self.refresh_native_settings_owner(&root, &id);
        let settings = self.native_page_settings(&id);
        let row = self.settings.mods.installed.get_mut(i);
        if let Some(page) = row.and_then(|m| m.native.as_mut()) {
            page.settings = settings;
        }
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
    let current = registry.state(state)?;
    if current.properties.get(key)? == value {
        return Some(state);
    }
    let mut s = current.clone();
    s.properties.insert(key.into(), value.into());
    registry.resolve(&s).ok()
}

/// The state a mod's property write changes `state` to; none when the block
/// has no such property or value, or already has it: `LevelChunk.setBlockState`
/// returns null for the same state, so `Level.setBlock` returns false.
pub(super) fn property_change(
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

/// What `mod_screen_open(mod_id)` does before running a screen transition
/// (`unavailable`: no renderer, acting for a guest, neither a player nor a
/// title screen, or a world/connection is loading; `owner`: the mod whose
/// `Screen::Mod` is currently shown on top, if any): `Some(false)` refuses
/// without changing state, `Some(true)` keeps `mod_id`'s already open screen,
/// and `None` proceeds through the normal screen-transition path.
pub(super) fn mod_screen_opens(
    screen: Screen,
    unavailable: bool,
    owner: Option<&str>,
    mod_id: &str,
) -> Option<bool> {
    if unavailable {
        return Some(false);
    }
    if screen == Screen::Mod && owner == Some(mod_id) {
        return Some(true);
    }
    None
}

fn client_packet_view(packet: &sunrise_protocol::packets::play::Clientbound) -> ClientPacket<'_> {
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
fn serverbound_packet_view(
    packet: &sunrise_protocol::packets::play::Serverbound,
) -> ClientPacket<'_> {
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
fn packet_view<'a>(
    types: &'static [mod_api::Identifier],
    id: i32,
    payload: Option<&'a sunrise_protocol::packets::configuration::CustomPayload>,
) -> ClientPacket<'a> {
    use mod_api::Identifier;
    use std::sync::LazyLock;

    static UNKNOWN: LazyLock<Identifier> =
        LazyLock::new(|| Identifier::parse("sunrise:unknown").unwrap());
    let id = usize::try_from(id)
        .ok()
        .and_then(|id| types.get(id))
        .unwrap_or(&UNKNOWN);
    let payload = payload.map(|p| ClientPayload::new(p.channel.as_str(), p.data.as_slice()));
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
                    mod_screen: None,
                    menu: None,
                    menu_generation: None,
                    dialog_key: None,
                    title: None,
                    rect: [0.0; 4],
                    screen_size: [0.0; 2],
                    slots: 0,
                    mouse: [0.0; 2],
                    partial_tick: 0.0,
                    regions: Vec::new(),
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
    fn no_stack_is_hovered_without_a_window() {
        crate::game::in_own_process(
            "game::native_hooks::tests::no_stack_is_hovered_without_a_window",
            || {
                use super::super::{Audio, Game, Options, Progress, Settings, load_resources};

                let root =
                    std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
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
                assert_eq!(game.screen_hovered_stack(), None);
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn declared_stock_storage_honors_shapes_and_rates() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        use mod_api::{self, *};
        use std::sync::Arc;
        use sunrise_client::{chunk::Chunk, containers::BlockEntity, save};

        const CHILD: &str = "SUNRISE_STOCK_STORAGE_CHILD";
        if run_fresh_cases(
            "game::native_hooks::tests::declared_stock_storage_honors_shapes_and_rates",
            CHILD,
            &["run"],
        ) {
            return;
        }

        const VAULT: &str = "stock_test:vault";
        // A stock container is not bounded by the menu protocol's signed-short slot index.
        const VAULT_SLOT: usize = sunrise_mod_package::content::MAX_MENU_SLOTS as usize;
        const TANK: &str = "stock_test:tank";
        const BATTERY: &str = "stock_test:battery";

        struct StockMod;
        #[mod_api::__subscriptions]
        impl Hooks for StockMod {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.block_entity_type("vault").blocks(&[VAULT]).build();
                r.block_entity_type("tank").blocks(&[TANK]).build();
                r.block_entity_type("battery").blocks(&[BATTERY]).build();

                let vault = r.block_key(VAULT);
                let mut container_faces: [Vec<usize>; 6] = Default::default();
                container_faces[Direction::Up as usize] = vec![VAULT_SLOT];
                r.block_container(
                    &vault,
                    ModContainer {
                        slots: VAULT_SLOT + 1,
                        faces: container_faces,
                        inputs: vec![VAULT_SLOT],
                        outputs: vec![VAULT_SLOT],
                        insert_rules: Vec::new(),
                        menu: None,
                    },
                );

                let tank = r.block_key(TANK);
                let mut fluids = ModFluidTanks::default();
                fluids.tanks = vec![1000; 17];
                fluids.tanks[0] = 0;
                fluids.tanks[16] = 1_000_001;
                fluids.faces[Direction::Up as usize] = vec![0, 16];
                fluids.inputs = vec![0, 16];
                fluids.outputs = vec![0, 16];
                r.block_fluids(&tank, fluids);

                let battery = r.block_key(BATTERY);
                let mut extract_faces = [false; 6];
                extract_faces[Direction::East as usize] = true;
                r.block_energy(
                    &battery,
                    ModEnergyBuffer {
                        capacity: 0,
                        max_insert: 500,
                        max_extract: 500,
                        insert_faces: [false; 6],
                        extract_faces,
                    },
                );
            }

            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                for id in [VAULT, TANK, BATTERY] {
                    event
                        .register(mod_api::Block::new(id, "minecraft:stone"))
                        .unwrap();
                }
            }
        }

        let mods: Mods = Arc::from([ModEntry::new("stock_test", Box::new(StockMod))]);
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = super::super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            // Populate stock tables through the game's real declarations -> table path.
            game.tick_block_entities_with(&mut ()).unwrap();

            let level = game.level(game.dim.id()).unwrap();
            let vault_pos = [1, 64, 0];
            let tank_pos = [2, 64, 0];
            let battery_pos = [3, 64, 0];
            for (pos, id) in [(vault_pos, VAULT), (tank_pos, TANK), (battery_pos, BATTERY)] {
                let state = game.pack.assets.registry.default_state(id).unwrap();
                assert_eq!(
                    game.level_set_block(level, pos, state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
            }

            // 1. Storage beyond the menu slot range still exposes and persists its declared slot.
            let items = level
                .capability(game, &ITEMS, vault_pos, Some(Direction::Up))
                .unwrap();
            assert_eq!(items.owner(), "the game");
            assert_eq!(items.size(game), 1);
            let dirt = ItemResource::of(
                &mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:dirt").unwrap(),
                    1,
                )
                .unwrap(),
            );
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>(items.insert_at(host, tx, 0, &dirt, 7))),
                Ok(Ok(7))
            );
            assert_eq!(items.amount(game, 0), 7);
            assert_eq!(items.resource(game, 0), Some(dirt.clone()));
            let vault_key = (vault_pos[0], vault_pos[1], vault_pos[2]);
            let saved = save::block_entity_encode::encode(
                vault_key,
                game.world.block_entities.get(&vault_key).unwrap(),
                VAULT,
                &Default::default(),
            );
            let (saved_pos, restored) = save::decode_block_entity_for_block(&saved, VAULT).unwrap();
            assert_eq!(saved_pos, vault_key);
            let BlockEntity::Mod(reloaded) = &restored else {
                panic!("expected mod block entity for vault");
            };
            assert_eq!(reloaded.items.len(), VAULT_SLOT + 1);
            assert_eq!(
                reloaded.items[VAULT_SLOT].as_ref(),
                Some(&sunrise_client::items::ItemStack::new("minecraft:dirt", 7))
            );
            game.world.block_entities.insert(vault_key, restored);
            assert_eq!(items.amount(game, 0), 7);
            assert_eq!(items.resource(game, 0), Some(dirt));

            // 2. 17-tank declaration exposes tank 16 (capacity 1,000,001), zero-capacity tank 0
            // refuses fill, and a malformed negative saved amount decodes as empty without overflow.
            let fluids = level
                .capability(game, &FLUIDS, tank_pos, Some(Direction::Up))
                .unwrap();
            assert_eq!(fluids.owner(), "the game");
            assert_eq!(fluids.size(game), 2);
            let water = FluidResource::of(&FluidStack::new(
                mod_api::__private::fluid_key("minecraft:water").unwrap(),
                1,
            ));
            assert_eq!(fluids.capacity(game, 0, Some(&water)), 0);
            assert_eq!(fluids.capacity(game, 1, Some(&water)), 1_000_001);
            let tank_key = (tank_pos[0], tank_pos[1], tank_pos[2]);
            if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&tank_key) {
                let mut list = vec![Tag::Compound(Compound::new()); 17];
                list[16] = Tag::compound([
                    ("Fluid", Tag::String("minecraft:water".into())),
                    ("Amount", Tag::Int(i32::MIN)),
                ]);
                entity.nbt.insert("fluids".into(), Tag::List(list));
            } else {
                panic!("expected mod block entity for tank");
            }
            assert_eq!(fluids.amount(game, 1), 0);
            assert_eq!(fluids.resource(game, 1), None);
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>((
                    fluids.insert_at(host, tx, 0, &water, 1000),
                    fluids.insert_at(host, tx, 1, &water, 1_000_001),
                ))),
                Ok((Ok(0), Ok(1_000_001)))
            );
            assert_eq!(fluids.amount(game, 0), 0);
            assert_eq!(fluids.resource(game, 0), None);
            assert_eq!(fluids.amount(game, 1), 1_000_001);
            assert_eq!(fluids.resource(game, 1), Some(water));

            // 3. Zero-capacity energy buffer with saved energy and max_extract > capacity exposes
            // the sided handler and extracts inside the normal transaction.
            let battery_key = (battery_pos[0], battery_pos[1], battery_pos[2]);
            if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&battery_key)
            {
                entity.nbt.insert("energy".into(), Tag::Int(800));
            } else {
                panic!("expected mod block entity for battery");
            }
            assert!(
                level
                    .capability(game, &ENERGY, battery_pos, Some(Direction::West))
                    .is_none()
            );
            let battery = level
                .capability(game, &ENERGY, battery_pos, Some(Direction::East))
                .unwrap();
            assert_eq!(battery.owner(), "the game");
            assert_eq!(battery.capacity(game), 0);
            assert_eq!(battery.amount(game), 800);
            assert_eq!(
                game.transaction(|host, tx| Ok::<_, ()>((
                    battery.insert(host, tx, 100),
                    battery.extract(host, tx, 600),
                ))),
                Ok((Ok(0), Ok(500)))
            );
            assert_eq!(battery.amount(game), 300);
            let abort: Result<(), mod_api::TransactionError<()>> = game.transaction(|host, tx| {
                assert_eq!(battery.extract(host, tx, 200), Ok(200));
                Err(())
            });
            assert!(abort.is_err());
            assert_eq!(battery.amount(game), 300);
        });
    }
}
