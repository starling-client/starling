//! Starling's part of dialogs.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The focused text input, as the keyboard's editor on Android mirrors it
/// (touch/text_mirror.rs).
pub(in super::super) struct TypedInput {
    /// Which of the dialog's inputs.
    pub(in super::super) index: usize,
    pub(in super::super) value: String,
    /// The cursor and the selection's other end, in characters; None for a
    /// multiline box, which the editor types into at its end.
    pub(in super::super) selection: Option<(usize, usize)>,
    /// The most characters it takes.
    pub(in super::super) max: usize,
    pub(in super::super) multiline: bool,
}

impl Game<'_> {
    /// The text input that has the focus, for the keyboard's editor.
    pub(in super::super) fn dialog_input(&self) -> Option<TypedInput> {
        if !self.dialog_typing() {
            return None;
        }
        let open = self.dialogs.open.as_ref()?;
        let index = open.focus?;
        Some(match open.fields.get(index)?.as_ref()? {
            Field::Line(edit) => TypedInput {
                index,
                value: edit.value.clone(),
                selection: Some((edit.cursor, edit.highlight)),
                max: edit.max_length,
                multiline: false,
            },
            Field::Multi(edit) => {
                let Control::Text { max_length, .. } = &open.dialog.inputs.get(index)?.control
                else {
                    return None;
                };
                TypedInput {
                    index,
                    value: edit.value(),
                    selection: None,
                    max: (*max_length).max(0) as usize,
                    multiline: true,
                }
            }
        })
    }

    /// The focused text input's box when it takes one line.
    pub(in super::super) fn dialog_input_box(&mut self) -> Option<&mut ui::EditBox> {
        let open = self.dialogs.open.as_mut()?;
        match open.fields.get_mut(open.focus?)? {
            Some(Field::Line(edit)) => Some(edit),
            _ => None,
        }
    }

    /// The keyboard's editor's text on the focused multiline input, which it
    /// types into at its end: back to where the two texts part, then the
    /// rest typed, through the box's own filter and limits.
    pub(in super::super) fn dialog_multiline_set(&mut self, text: &str) {
        let font = &self.gui.atlas.font;
        let Some(open) = self.dialogs.open.as_mut() else {
            return;
        };
        let Some(Some(Field::Multi(edit))) = open.focus.and_then(|f| open.fields.get_mut(f)) else {
            return;
        };
        let value = edit.value();
        let same = value
            .chars()
            .zip(text.chars())
            .take_while(|(a, b)| a == b)
            .count();
        // Ctrl+End: the cursor at the end, nothing selected.
        edit.key(font, KeyCode::End, true, false);
        let gone = value.chars().count() - same;
        if gone > 0 {
            edit.delete_text(font, -(gone as isize));
        }
        let rest: String = text.chars().skip(same).collect();
        edit.insert_text(font, &rest);
    }

    /// A dialog the game itself shows, in the server dialogs' look.
    pub(in super::super) fn show_local_dialog(&mut self, tag: &Tag) {
        self.show_dialog(tag);
        if let Some(open) = self.dialogs.open.as_mut() {
            open.local = true;
        }
    }

    /// The platform's browser on `url`; without one (the desktop opens no links) it is copied.
    pub(in super::super) fn open_link(&mut self, url: &str) {
        if sunrise_client::platform::open_link(url) != Some(true) {
            use sunrise_client::clipboard::Clipboard;
            self.clipboard.set(url);
            log::info!("Sunrise: no browser here; {url} copied");
        }
    }
}
