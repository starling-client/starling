//! Input method (IME) text: committed composition goes to the focused text
//! field through the same sinks the keyboard and `sunrise-ctl type` use.
//! The window only asks the OS for an input method while such a field has
//! focus, so movement keys keep arriving as plain key events.
use super::{Game, Screen, screen_build::ObservedScreen};
use sunrise_client::{
    hud::HudBatch,
    preedit::{self, PreeditEvent},
};

/// The text of the key press that is still held, which the input method may
/// commit again once a field has focus. GLFW's char callback of a key press
/// reaches the screen open when the key was pressed, so the press that opens
/// a text screen (T, or / for chat) never types into it; winit can hand the
/// same character over as `Ime::Commit` after the field enabled the input
/// method.
#[derive(Default)]
pub(super) struct HeldKeyText(Option<(winit::keyboard::KeyCode, String)>);

impl HeldKeyText {
    pub(super) fn pressed(&mut self, key: winit::keyboard::KeyCode, text: Option<&str>) {
        self.0 = text.map(|t| (key, t.to_owned()));
    }

    pub(super) fn released(&mut self, key: winit::keyboard::KeyCode) {
        if self.0.as_ref().is_some_and(|(k, _)| *k == key) {
            self.0 = None;
        }
    }

    /// Whether `committed` is the held key's own character: dropped once.
    pub(super) fn swallows(&mut self, committed: &str) -> bool {
        if self.0.as_ref().is_some_and(|(_, t)| t == committed) {
            self.0 = None;
            return true;
        }
        false
    }
}

impl Game<'_> {
    /// Whether a vanilla (non-mounted) text box has focus right now.
    pub(super) fn vanilla_text_field_focused(&mut self) -> bool {
        match self.observed_overlay_now() {
            Some("dialog") => return self.dialog_typing(),
            Some("sign_editor" | "book_edit" | "book_sign") => return true,
            Some(_) => return false,
            None => {}
        }
        if self.remote_loading() {
            return false;
        }
        if self.screen == Screen::Playing
            && let Some(title) = self.title.as_mut()
        {
            // A caret field, an append-only one (`focused_field`), or a game
            // rule's box (`has_caret_field` asks the rules screen itself, and
            // `title_edit` types into it).
            return title.has_caret_field() || title.focused_field().is_some();
        }
<<<<<<< HEAD
        if self.sign_edit.is_some() || matches!(self.book, Some(BookScreen::Edit { .. })) {
            return true;
        }
        let focused = match self.screen {
||||||| 46c00106def
        if self.sign_edit.is_some() || matches!(self.book, Some(BookScreen::Edit { .. })) {
            return true;
        }
        match self.screen {
=======
        match self.screen {
>>>>>>> ed6772e2118
            Screen::Chat | Screen::CommandBlock | Screen::Social => true,
            Screen::DebugOptions => self
                .debug_options
                .as_ref()
                .is_some_and(|s| s.search_focused()),
            // The language list's search, the settings search and the pack
            // screen's search (OptionsUi::typing).
            Screen::Options => self.ui.options.typing(),
            Screen::WorldOptions => self.world_options_typing(),
            Screen::Creative => self.mod_browser_typing() || self.creative_screen.tab().search,
            Screen::BlockEditor => self.block_editor.as_ref().is_some_and(|s| s.text_focused()),
            Screen::Inventory => {
                self.mod_browser_typing() || self.recipe_search_focused() || self.station_typing()
            }
            _ => false,
        };
        focused || cfg!(target_os = "android") && self.widget_text_focused()
    }

    /// Whether a text box or input-capturing mounted widget has focus right now
    /// (so OS IME commits and text typing are enabled). Note that `ime_preedit`
    /// separately checks `mounted_widget_preedit`, never `captures_input()`.
    pub(super) fn text_field_focused(&mut self) -> bool {
        if self.context_menu_open() {
            return false;
        }
        if self.screen_state.widgets.has_focus() {
            return self.mounted_widget_captures_input();
        }
        self.vanilla_text_field_focused()
    }

    /// Identity of the text box currently owning IME composition, so switching
    /// directly from one field to another (including synchronous replacement
    /// with an initially focused field) clears the old field's preedit and area.
    fn ime_focus_identity(&mut self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        if self.context_menu_open()
            || (!self.text_field_focused() && self.preset_editor_title().is_none())
        {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.screen_state.screen_instance.hash(&mut h);
        if let Some(id) = self.screen_state.widgets.focused_id() {
            1u8.hash(&mut h);
            id.raw().hash(&mut h);
        } else {
            0u8.hash(&mut h);
            std::mem::discriminant(&self.screen).hash(&mut h);
            self.dialog_open().hash(&mut h);
            self.sign_edit.is_some().hash(&mut h);
            self.book.is_some().hash(&mut h);
            self.mod_browser_typing().hash(&mut h);
            self.recipe_search_focused().hash(&mut h);
            self.station_typing().hash(&mut h);
            if let Some(title) = self.title.as_mut() {
                title.mode.id().hash(&mut h);
                title.focused_field().map(|(id, _, _)| id).hash(&mut h);
            }
        }
        Some(h.finish())
    }

    /// Allow the input method only while a text box has focus, and reset
    /// composition whenever the focused text box itself changes.
    pub(super) fn sync_ime(&mut self) {
<<<<<<< HEAD
        // On Android asking for the input method shows the keyboard: there a tap on the field
        // decides (touch.rs), not its focus.
        if cfg!(target_os = "android") {
            return;
        }
        let want = self.text_field_focused() || self.preset_editor_title().is_some();
||||||| 46c00106def
        let want = self.text_field_focused() || self.preset_editor_title().is_some();
=======
        let focus = self.ime_focus_identity();
        let want = focus.is_some();
        if focus != self.screen_state.ime_focus {
            self.screen_state.ime_focus = focus;
            self.ime_area = None;
            preedit::reset();
        }
>>>>>>> ed6772e2118
        if want != self.ime_allowed
            && let Some(window) = &self.window
        {
            window.set_ime_allowed(want);
            self.ime_allowed = want;
            self.ime_area = None;
            if !want {
                preedit::reset();
            }
        }
    }

    /// The input method consumes composing key presses even if the editor has no built-in caret.
    pub(super) fn ime_suppresses_key(&mut self) -> bool {
        !self.context_menu_open()
            && (self.text_field_focused() || self.preset_editor_title().is_some())
            && preedit::should_suppress_key()
    }

    /// `WindowEvent::Ime(Preedit)`: `KeyboardHandler.textEditing`. The text
    /// still being composed is shown under the focused field's caret.
    pub(super) fn ime_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        self.sync_ime();
        if self.context_menu_open() {
            preedit::reset();
            return;
        }
        let preedit_receiver = (mod_api::__private::listened(mod_api::HookId::ScreenPreeditPre)
            || mod_api::__private::listened(mod_api::HookId::ScreenPreeditPost))
        .then(|| self.observed_input_screen())
        .flatten();
        let initial_instance = self.screen_state.screen_instance;
        let event = PreeditEvent::from_winit(text, cursor);
        // ScreenEvent.Preedit.Pre, then the screen's handler (the focused
        // child takes it), then Preedit.Post on the captured receiver when no
        // child handled it.
        let shown = event.clone().unwrap_or_default();
        let bound_editor = self.preset_editor_title().is_some();
        if self.dispatch_preedit_pre_hook(preedit_receiver.as_ref(), &shown) {
            // A bound preset editor wants committed text, but the platform still owns
            // the composition (and its Enter/Escape) even when Pre cancels it.
            if bound_editor && self.screen_state.screen_instance == initial_instance {
                preedit::set_preedit(event);
            }
            return;
        }
        let taken = if self.screen_state.screen_instance != initial_instance {
            false
        } else if self.screen_state.widgets.has_focus() {
            self.mounted_widget_preedit(event.as_ref())
        } else {
            self.vanilla_text_field_focused()
        };
        preedit::set_preedit(event.filter(|_| {
            taken || (bound_editor && self.screen_state.screen_instance == initial_instance)
        }));
        if !taken {
            self.dispatch_preedit_post_hook(preedit_receiver.as_ref(), &shown);
        }
    }

    /// `native_screen_preedit_pre` for the captured receiving screen: whether
    /// it cancelled. An empty `event` is a composition that ended. Nothing
    /// fires with no mod listening or no screen open.
    fn dispatch_preedit_pre_hook(
        &self,
        receiver: Option<&ObservedScreen>,
        event: &PreeditEvent,
    ) -> bool {
        let mut canceled = false;
        if mod_api::__private::listened(mod_api::HookId::ScreenPreeditPre)
            && let Some(receiver) = receiver
        {
            let view = receiver.view();
            self.native_screen_preedit_pre(
                &view,
                &event.full_text,
                event.caret_position,
                &event.blocks,
                event.focused_block,
                &mut canceled,
            );
        }
        canceled
    }

    /// `native_screen_preedit_post` for the captured receiving screen, as the Pre.
    fn dispatch_preedit_post_hook(&self, receiver: Option<&ObservedScreen>, event: &PreeditEvent) {
        if mod_api::__private::listened(mod_api::HookId::ScreenPreeditPost)
            && let Some(receiver) = receiver
        {
            let view = receiver.view();
            self.native_screen_preedit_post(
                &view,
                &event.full_text,
                event.caret_position,
                &event.blocks,
                event.focused_block,
            );
        }
    }

    /// With the frame's GUI built: draw the composition over it and move the
    /// input method's candidate window to the caret
    /// (`TextInputManager.setTextInputArea`, in window pixels).
    pub(super) fn flush_ime(&mut self, hud: &mut HudBatch) {
        self.touch_text_field(preedit::take_focused_field());
        preedit::draw_overlay(hud, &self.gui.atlas);
        let Some([x0, y0, x1, y1]) = preedit::take_text_input_area() else {
            return;
        };
        let scale = hud.scale;
        let x = (x0 * scale).round() as i32;
        let y = (y0 * scale).round() as i32;
        let width = (((x1 - x0) * scale).round() as i32).max(1);
        let height = (((y1 - y0) * scale).round() as i32).max(1);
        let area = [x, y, width, height];
        if self.ime_area == Some(area) {
            return;
        }
        if let Some(window) = &self.window
            && self.ime_allowed
        {
            window.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(x, y),
                winit::dpi::PhysicalSize::new(width as u32, height as u32),
            );
            self.ime_area = Some(area);
        }
    }

    /// Text the input method committed (`WindowEvent::Ime(Commit)`).
    pub(super) fn ime_commit(&mut self, text: &str) {
        preedit::mark_committed();
        if self.context_menu_open() || text.is_empty() || self.held_key_text.swallows(text) {
            return;
        }
        // ScreenEvent.CharacterTyped.Pre, mounted widgets, focused vanilla
        // field, and CharacterTyped.Post on the captured receiving screen when
        // unhandled.
        self.screen_chars_dispatch(text, |game, c| {
            if game.preset_editor_title().is_some() {
                game.preset_editor_type_text(c)
            } else {
                game.ime_type(c)
            }
        });
    }

    /// A committed character into the focused vanilla text box (mounted
    /// widgets are offered first by `screen_chars_dispatch`). Returns whether
    /// a text box handled the character.
    fn ime_type(&mut self, text: &str) -> bool {
        // Only a dialog has vanilla fields among the prompts. Mounted fields already heard it.
        if (self.prompt_open() && !self.dialog_open())
            || self.screen_state.widgets.has_focus()
            || !self.vanilla_text_field_focused()
        {
            return false;
        }
        if self.dialog_type(text) {
            return true;
        }
        if self.screen == Screen::Playing && self.title.is_some() {
            self.title_edit(Some(text), false);
            true
        } else if self.book.is_some() {
            self.book_input(None, Some(text));
            true
        } else if self.sign_edit.is_some() {
            self.sign_input(None, Some(text));
            true
        } else {
            match self.screen {
                Screen::Chat => self.chat_type(text),
                Screen::CommandBlock => self.command_block_type(text),
                Screen::Social => self.social_type(text),
                Screen::DebugOptions => self.debug_options_type(text),
                Screen::Options => self.ui.options.type_text(&self.gui.atlas, text),
                Screen::WorldOptions => self.world_options_text(Some(text)),
                Screen::BlockEditor => self.block_editor_type(text),
                Screen::Inventory | Screen::Creative if self.mod_browser_typing() => {
                    self.mod_browser_text(Some(text), false)
                }
                Screen::Creative => self.creative_search_edit(Some(text)),
                Screen::Inventory if self.recipe_search_focused() => {
                    self.recipe_text(Some(text), false)
                }
                Screen::Inventory => self.station_text(Some(text), false),
                _ => return false,
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HeldKeyText;
    use winit::keyboard::KeyCode;

    #[test]
    fn the_held_opening_key_is_not_typed_again_by_the_input_method() {
        let mut held = HeldKeyText::default();
        held.pressed(KeyCode::KeyT, Some("t"));
        assert!(held.swallows("t"));
        assert!(!held.swallows("t"), "only the one character of the press");
        held.pressed(KeyCode::KeyT, Some("t"));
        assert!(!held.swallows("h"), "another character is the person's");
        held.released(KeyCode::KeyT);
        assert!(!held.swallows("t"), "a commit after the release is typed");
        held.pressed(KeyCode::Slash, Some("/"));
        held.released(KeyCode::KeyT);
        assert!(held.swallows("/"), "another key's release keeps it");
    }
}
