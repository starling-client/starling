//! Input method (IME) text: committed composition goes to the focused text
//! field through the same sinks the keyboard and `sunrise-ctl type` use.
//! The window only asks the OS for an input method while such a field has
//! focus, so movement keys keep arriving as plain key events.
use super::{Game, Screen, books::BookScreen};
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
    /// Whether a text box has focus right now (the fields `ime_commit` can fill).
    pub(super) fn text_field_focused(&mut self) -> bool {
        if self.dialog_open() {
            return self.dialog_has_text_field();
        }
        if self.title_input_active()
            && let Some(title) = self.title.as_mut()
        {
            // A caret field, an append-only one (`focused_field`), or a game
            // rule's box (`has_caret_field` asks the rules screen itself, and
            // `title_edit` types into it).
            return title.has_caret_field() || title.focused_field().is_some();
        }
        if self.sign_edit.is_some() || matches!(self.book, Some(BookScreen::Edit { .. })) {
            return true;
        }
        let focused = match self.screen {
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

    /// Allow the input method while a text box has focus or a bound preset editor takes text.
    pub(super) fn sync_ime(&mut self) {
        // On Android asking for the input method shows the keyboard: there a tap on the field
        // decides (touch.rs), not its focus.
        if cfg!(target_os = "android") {
            return;
        }
        let want = self.text_field_focused() || self.preset_editor_title().is_some();
        if want != self.ime_allowed
            && let Some(window) = &self.window
        {
            window.set_ime_allowed(want);
            self.ime_allowed = want;
            // The field lost the focus (or a new one took it): the composition
            // and the candidate window's place belong to the old one.
            self.ime_area = None;
            if !want {
                preedit::reset();
            }
        }
    }

    /// The input method consumes composing key presses even if the editor has no built-in caret.
    pub(super) fn ime_suppresses_key(&mut self) -> bool {
        (self.text_field_focused() || self.preset_editor_title().is_some())
            && preedit::should_suppress_key()
    }

    /// `WindowEvent::Ime(Preedit)`: `KeyboardHandler.textEditing`. The text
    /// still being composed is shown under the focused field's caret.
    pub(super) fn ime_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        let taken = self.text_field_focused();
        let bound_editor = self.preset_editor_title().is_some();
        let event = PreeditEvent::from_winit(text, cursor);
        let shown = event.clone().unwrap_or_default();
        let canceled = self.dispatch_preedit_pre_hook(&shown);
        // The editor receives committed text, not an overlay. Remember the input method's
        // composition even when Pre cancels delivery to the screen, as KeyboardHandler does.
        if bound_editor || !canceled {
            preedit::set_preedit(event.filter(|_| taken || bound_editor));
        }
        // Tracking the composition does not mean a focused widget handled the event.
        if !canceled && !taken {
            self.dispatch_preedit_post_hook(&shown);
        }
    }

    /// `native_screen_preedit_pre` for the open screen: whether it cancelled.
    /// An empty `event` is a composition that ended. Nothing fires with no
    /// mod listening or no screen open.
    fn dispatch_preedit_pre_hook(&self, event: &PreeditEvent) -> bool {
        let mut canceled = false;
        if mod_api::__private::listened(mod_api::HookId::ScreenPreeditPre)
            && let Some(view) = self.observed_screen_view()
        {
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

    /// `native_screen_preedit_post` for the open screen, as the Pre.
    fn dispatch_preedit_post_hook(&self, event: &PreeditEvent) {
        if mod_api::__private::listened(mod_api::HookId::ScreenPreeditPost)
            && let Some(view) = self.observed_screen_view()
        {
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
        if text.is_empty() || self.held_key_text.swallows(text) {
            return;
        }
        // Dialogs have their own fields; the other topmost prompts take no text.
        if self.prompt_open() && !self.dialog_open() {
            return;
        }
        if self.preset_editor_title().is_some() {
            self.screen_committed_chars(text);
            return;
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        // ScreenEvent.CharacterTyped.Pre: the input method's committed text
        // reaches a text box by characters, as a key's own text does, so the
        // open screen hears of each of them first and a cancelled one never
        // reaches the box.
        self.screen_text_each(&text, |game, c| game.ime_type(c));
    }

    /// A committed character into the focused text box.
    fn ime_type(&mut self, text: &str) {
        if !self.text_field_focused() {
            return;
        }
        if self.dialog_type(text) {
            return;
        }
        if self.title_input_active() {
            self.title_edit(Some(text), false);
        } else if self.book.is_some() {
            self.book_input(None, Some(text));
        } else if self.sign_edit.is_some() {
            self.sign_input(None, Some(text));
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
                _ => {}
            }
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
