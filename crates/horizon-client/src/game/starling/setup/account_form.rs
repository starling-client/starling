//! Resource-independent account fields, using the setup card and Android's existing IME.
use super::*;
use horizon_client::ui::EditBox;
use winit::keyboard::KeyCode;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Ely,
    Offline,
}
pub(super) struct Form {
    pub kind: Kind,
    fields: [EditBox; 3],
    focus: Option<usize>,
    show_password: bool,
    generation: i32,
    pub rects: Cell<[[f32; 4]; 3]>,
}
impl Form {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            fields: std::array::from_fn(|i| {
                let mut field = EditBox::new(0., 0., 200., 32.);
                field.max_length = if kind == Kind::Offline {
                    16
                } else {
                    [254, 256, 6][i]
                };
                field
            }),
            focus: None,
            show_password: false,
            generation: 1,
            rects: Cell::new([[0.; 4]; 3]),
        }
    }
    fn count(&self) -> usize {
        if self.kind == Kind::Ely { 3 } else { 1 }
    }
    fn secret(&self) -> bool {
        self.kind == Kind::Ely && self.focus.is_some_and(|i| i > 0)
    }
    fn sync(&self, show: bool) {
        #[cfg(target_os = "android")]
        if let Some(i) = self.focus {
            let f = &self.fields[i];
            horizon_client::android::text_mirror(
                show,
                &f.value,
                f.cursor as i32,
                f.highlight as i32,
                f.max_length as i32,
                (6 << 8) | if self.secret() { 4 } else { 0 },
                self.generation,
            );
        }
        #[cfg(not(target_os = "android"))]
        let _ = show;
    }
    pub fn focus(&mut self, i: usize) {
        for (at, field) in self.fields.iter_mut().enumerate() {
            field.focused = at == i;
        }
        self.focus = Some(i);
        self.generation += 1;
        self.fields[i].move_cursor_to_end(false);
        self.sync(true);
    }
    pub fn hide(&mut self) {
        self.focus = None;
        for field in &mut self.fields {
            field.focused = false;
        }
        self.generation += 1;
        #[cfg(target_os = "android")]
        horizon_client::android::hide_text_mirror();
    }
    pub fn reveal(&mut self) {
        self.show_password = !self.show_password;
    }
    #[cfg(any(target_os = "android", test))]
    pub fn replace(&mut self, text: &str, cursor: usize, anchor: usize, generation: i32) {
        if generation != self.generation {
            return;
        }
        let secret = self.secret();
        if let Some(i) = self.focus {
            let f = &mut self.fields[i];
            f.value.clear();
            f.cursor = 0;
            f.highlight = 0;
            if secret {
                f.insert_password(text)
            } else {
                f.insert_text(text)
            }
            let n = f.value.chars().count();
            f.cursor = cursor.min(n);
            f.highlight = anchor.min(n);
            if f.value != text {
                self.sync(false);
            }
        }
    }
    pub fn key(&mut self, key: KeyCode, text: Option<&str>, ctrl: bool, shift: bool) {
        if key == KeyCode::Tab {
            let i = self.focus.unwrap_or(0);
            self.focus((i + if shift { self.count() - 1 } else { 1 }) % self.count());
            return;
        }
        let secret = self.secret();
        if let Some(i) = self.focus {
            let f = &mut self.fields[i];
            if ctrl {
                use horizon_client::clipboard::{Clipboard, EditShortcut, SystemClipboard};
                let mut board = SystemClipboard::default();
                match key {
                    KeyCode::KeyV => {
                        let text = board.get();
                        if secret {
                            f.insert_password(&text)
                        } else {
                            f.insert_text(&text)
                        }
                    }
                    KeyCode::KeyA => {
                        f.shortcut(EditShortcut::SelectAll, &mut board);
                    }
                    KeyCode::KeyC | KeyCode::KeyX if secret => {}
                    _ => {
                        f.key(key, ctrl, shift);
                    }
                }
            } else {
                f.key(key, false, shift);
                if let Some(text) = text {
                    if secret {
                        f.insert_password(text)
                    } else {
                        f.insert_text(text)
                    }
                }
            }
            self.sync(false);
        }
    }
    pub fn poll(&mut self) -> bool {
        #[cfg(target_os = "android")]
        for event in horizon_client::android::take_mirror_events() {
            use horizon_client::android::MirrorEvent;
            match event {
                MirrorEvent::Edit {
                    text,
                    cursor,
                    anchor,
                    generation,
                } => self.replace(&text, cursor, anchor, generation),
                MirrorEvent::Action => {
                    self.hide();
                    return true;
                }
                MirrorEvent::Key(67) => self.key(KeyCode::Backspace, None, false, false),
                MirrorEvent::Key(61) => self.key(KeyCode::Tab, None, false, false),
                MirrorEvent::Key(111) => self.hide(),
                _ => {}
            }
        }
        false
    }
    pub fn submit(&mut self) -> bool {
        if self.fields[0].value.trim().is_empty()
            || (self.kind == Kind::Ely && self.fields[1].value.is_empty())
        {
            return false;
        }
        self.hide();
        if self.kind == Kind::Ely {
            accounts::sign_in_ely(
                self.fields[0].value.clone(),
                std::mem::take(&mut self.fields[1].value),
                std::mem::take(&mut self.fields[2].value),
            );
        } else {
            accounts::add_offline(self.fields[0].value.clone());
        }
        true
    }
    pub fn draw(
        &self,
        ui: &mut Ui,
        actions: &mut [Option<([f32; 4], Action)>; 5],
        available_h: f32,
    ) {
        let w = ui.w;
        let cw = (w - 32.).min(520.);
        let ch = if self.kind == Kind::Ely { 352. } else { 206. };
        let x = (w - cw) / 2.;
        let y = ((available_h - ch) / 2.).max(8.);
        let inner = cw - 32.;
        ui.rounded(x, y, cw, ch, 14., color::bg());
        ui.border(x, y, cw, ch, 14., color::border());
        ui.text(
            Face::Semibold,
            18.,
            x + 16.,
            y + 14.,
            color::text(),
            &tr(if self.kind == Kind::Ely {
                "setup.ely_title"
            } else {
                "setup.offline_title"
            }),
        );
        let mut rects = [[0.; 4]; 3];
        for (i, rect) in rects.iter_mut().enumerate().take(self.count()) {
            let fy = y + 48. + i as f32 * 58.;
            *rect = [x + 16., fy + 18., inner, 34.];
            ui.text(
                Face::Regular,
                12.,
                x + 16.,
                fy,
                color::MUTED,
                &tr(if self.kind == Kind::Offline {
                    "name"
                } else {
                    ["login", "password", "otp"][i]
                }),
            );
            ui.rounded(rect[0], rect[1], rect[2], rect[3], 6., color::raised());
            if self.focus == Some(i) {
                ui.border(rect[0], rect[1], rect[2], rect[3], 6., color::ACCENT);
            }
            let raw = &self.fields[i].value;
            let shown = if self.kind == Kind::Ely && (i == 2 || (i == 1 && !self.show_password)) {
                "*".repeat(raw.chars().count())
            } else {
                raw.clone()
            };
            let mut chars: Vec<char> = shown.chars().collect();
            while ui.measure(Face::Regular, 14., &chars.iter().collect::<String>()) > inner - 18.
                && !chars.is_empty()
            {
                chars.remove(0);
            }
            ui.text(
                Face::Regular,
                14.,
                rect[0] + 8.,
                rect[1] + 8.,
                color::text(),
                &chars.iter().collect::<String>(),
            );
            if self.focus == Some(i) && self.fields[i].cursor_visible() {
                let skipped = shown.chars().count() - chars.len();
                let cursor = self.fields[i]
                    .cursor
                    .saturating_sub(skipped)
                    .min(chars.len());
                let before: String = chars.iter().take(cursor).collect();
                let cx = rect[0] + 8. + ui.measure(Face::Regular, 14., &before);
                ui.fill(cx, rect[1] + 8., 1., 16., color::text());
            }
        }
        self.rects.set(rects);
        let specs = if self.kind == Kind::Ely {
            vec![
                (tr("log_in"), Action::SubmitForm),
                (tr("back"), Action::AccountBack),
                (tr("register"), Action::Register),
                (
                    tr(if self.show_password {
                        "hide_password"
                    } else {
                        "show_password"
                    }),
                    Action::Reveal,
                ),
            ]
        } else {
            vec![
                (tr("save"), Action::SubmitForm),
                (tr("back"), Action::AccountBack),
            ]
        };
        for (i, (label, action)) in specs.into_iter().enumerate() {
            let row = i / 2;
            let right = i % 2 == 0;
            let bw = (inner - 10.) / 2.;
            let rect = [
                x + 16. + if right { bw + 10. } else { 0. },
                y + ch - 48. - row as f32 * 44.,
                bw,
                36.,
            ];
            ui.rounded(
                rect[0],
                rect[1],
                rect[2],
                rect[3],
                8.,
                if i == 0 {
                    color::ACCENT
                } else {
                    color::raised()
                },
            );
            ui.text_centered(
                Face::Semibold,
                13.,
                rect[0] + bw / 2.,
                rect[1] + 9.,
                if i == 0 {
                    color::ON_ACCENT
                } else {
                    color::text()
                },
                &label,
            );
            actions[i] = Some((rect, action));
        }
    }
}
impl Drop for Form {
    fn drop(&mut self) {
        self.hide();
    }
}
fn tr(key: &str) -> String {
    horizon_client::lang::get(&format!("starling.account.{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_password_ime_updates_cannot_edit_the_next_field() {
        let mut f = Form::new(Kind::Ely);
        f.focus = Some(1);
        f.generation = 4;
        f.replace("Secret§9", 8, 8, 4);
        assert_eq!(f.fields[1].value, "Secret§9");
        f.focus = Some(2);
        f.generation = 5;
        f.replace("old-password", 12, 12, 4);
        assert!(f.fields[2].value.is_empty());
        f.replace("123456", 6, 6, 5);
        assert_eq!(f.fields[2].value, "123456");
    }
    #[test]
    fn empty_form_does_not_start_authentication() {
        let mut f = Form::new(Kind::Ely);
        assert!(!f.submit());
        f.fields[0].value = "name".into();
        assert!(!f.submit());
    }
}
