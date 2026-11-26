//! Starling's part of title.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Title {
    /// A finger's scroll (Starling, a phone's drag): the lists scroll as under the wheel, but a cycle
    /// button the finger passes over keeps its value; a tap still cycles it.
    pub fn finger_scroll(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
        self.scroll_worlds_with(delta, cursor, size, false);
    }

    pub fn password_focused(&self) -> bool {
        matches!(&self.mode, Mode::Account(screen) if screen.password_focused())
    }

    /// An account field mirrored by Android's IME, in character indices.
    pub fn account_field_selection(&mut self) -> Option<(usize, usize)> {
        if !matches!(self.mode, Mode::Account(_)) {
            return None;
        }
        let (id, value, max) = self.focused_field()?;
        let mut value = value.clone();
        let mut selection = None;
        self.caret.get_mut().edit(id, &mut value, max, |edit| {
            selection = Some((edit.cursor, edit.highlight));
        });
        selection
    }

    /// Apply the whole IME replacement, including its cursor and selection.
    pub fn set_account_field(&mut self, text: &str, cursor: usize, anchor: usize) {
        if !matches!(self.mode, Mode::Account(_)) {
            return;
        }
        let secret = self.password_focused();
        self.edit_field(|edit| {
            edit.value.clear();
            edit.cursor = 0;
            edit.highlight = 0;
            if secret {
                edit.insert_password(text);
            } else {
                edit.insert_text(text);
            }
            let len = edit.value.chars().count();
            edit.cursor = cursor.min(len);
            edit.highlight = anchor.min(len);
        });
    }

    /// Create New World's game rules while their screen is open, whose boxes `focused_field`
    /// leaves out (the on-screen keyboard's editor on Android reads and sets them).
    pub fn creation_rules(&mut self) -> Option<&mut RulesScreen> {
        self.rules.get_mut().as_mut()
    }

    /// Whether a text field has the focus.
    pub fn text_focused(&mut self) -> bool {
        // The game rules' boxes edit themselves (`focused_field` leaves them out).
        if let Some(rules) = self.rules.get_mut() {
            return rules.typing();
        }
        self.focused_field().is_some()
    }

    /// Whether `(x, y)` (GUI pixels) is on the focused text field, where a tap asks for an
    /// on-screen keyboard.
    pub fn text_field_at(&mut self, (x, y): (f32, f32), size: (f32, f32)) -> bool {
        // The game rules' screen: the tap's click moved the focus to the box under it, or
        // to none.
        if let Some(rules) = self.rules.get_mut() {
            return rules.typing();
        }
        let (w, h) = (size.0.ceil(), size.1.ceil());
        self.focused_rect((w, h))
            .is_some_and(|[fx, fy, fw, fh]| crate::ui::hovered((x, y), fx, fy, fw, fh))
    }
}
