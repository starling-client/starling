//! Profile selection in Starling's existing title UI.
use super::{centered, wrapped};
use crate::{
    accounts::{self, Files, State},
    clipboard::FieldId,
    hud::{HudAtlas, HudBatch, rgba},
};
use starling_account::{Player, Provider};
fn tr(key: &str) -> String {
    crate::lang::get(&format!("starling.account.{key}"))
}
fn with(key: &str, value: &str) -> String {
    tr(key).replacen("%s", value, 1)
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Page {
    #[default]
    List,
    Add,
    Signing,
    Files,
    Offline,
    Ely,
    Remove,
}
#[derive(Clone, Default)]
pub struct Screen {
    page: Page,
    offset: usize,
    focus: usize,
    username: String,
    password: String,
    otp: String,
    remove: Option<Player>,
    dismiss_error: bool,
    show_password: bool,
    microsoft: bool,
}
impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountScreen")
            .field("page", &self.page)
            .field("credentials", &"<hidden>")
            .finish()
    }
}
pub(super) fn main_label() -> String {
    if matches!(
        accounts::state(),
        State::Resuming | State::Starting | State::Progress(_) | State::Code { .. }
    ) {
        return tr("working");
    }
    if matches!(accounts::state(), State::Failed(_)) {
        return tr("title");
    }
    accounts::player_name()
        .map(|n| with("active", &n))
        .unwrap_or_else(|| tr("title"))
}
fn rows(h: f32) -> usize {
    (((h - 136.0) / 24.0).floor() as usize).clamp(1, 8)
}
fn label(p: &Player) -> String {
    let provider = match p {
        Player::Offline(_) => tr("offline"),
        Player::Account(a) => match a.provider {
            Provider::Microsoft => "Microsoft".into(),
            Provider::Ely => "Ely.by".into(),
        },
    };
    format!("{} ({provider})", p.name())
}
impl Screen {
    fn status(&self) -> Option<String> {
        self.status_for(&accounts::state())
    }
    fn status_for(&self, state: &State) -> Option<String> {
        match state {
            State::Starting => Some(tr(if self.microsoft {
                "requesting_code"
            } else {
                "ely_working"
            })),
            State::Resuming => Some(tr("restoring")),
            State::Progress(stage) => Some(crate::lang::get(stage.key())),
            State::Code { code, link } => Some(format!(
                "{}\n{link}\n\n{code}\n\n{}",
                tr("device_code"),
                tr("waiting_browser")
            )),
            State::SignedIn(account) if self.page == Page::Signing => {
                Some(with("signed_in", &account.name))
            }
            State::OfflineProfile(name) if self.page == Page::Signing => Some(with("active", name)),
            State::Failed(e) if !self.dismiss_error => {
                Some(if e.starts_with("starling.account.") {
                    crate::lang::get(e)
                } else {
                    e.clone()
                })
            }
            _ => None,
        }
    }
    pub(super) fn field(&mut self) -> Option<(FieldId, &mut String, usize)> {
        if self.status().is_some() {
            return None;
        }
        match (self.page, self.focus) {
            (Page::Offline, 0) => Some((("account-name", 0), &mut self.username, 16)),
            (Page::Ely, 0) => Some((("account-login", 0), &mut self.username, 254)),
            (Page::Ely, 1) => Some((("account-password", 0), &mut self.password, 256)),
            (Page::Ely, 2) => Some((("account-otp", 0), &mut self.otp, 6)),
            _ => None,
        }
    }
    pub(super) fn field_rect(&self, w: f32) -> Option<[f32; 4]> {
        matches!(self.page, Page::Offline | Page::Ely).then(|| {
            [
                (w / 2.0).floor() - 100.0,
                50.0 + 38.0 * self.focus as f32,
                200.0,
                20.0,
            ]
        })
    }
    pub(super) fn password_focused(&self) -> bool {
        self.status().is_none() && self.page == Page::Ely && matches!(self.focus, 1 | 2)
    }
    pub(super) fn password_masked(&self) -> bool {
        self.password_focused() && (self.focus == 2 || !self.show_password)
    }
    pub(super) fn focus_at(&mut self, (x, y): (f32, f32), w: f32) {
        if (x - w / 2.0).abs() > 100.0 {
            return;
        }
        let n = match self.page {
            Page::Ely => 3,
            Page::Offline => 1,
            _ => 0,
        };
        for i in 0..n {
            if (50.0 + 38.0 * i as f32..70.0 + 38.0 * i as f32).contains(&y) {
                self.focus = i;
            }
        }
    }
    pub(super) fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut crate::ui::Caret,
        started: std::time::Instant,
    ) {
        let (w, h) = (batch.width.ceil(), batch.height.ceil());
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        centered(atlas, batch, &tr("title"), 20.0, u32::MAX);
        if let Some(message) = self.status() {
            let mut y = 50.0;
            for text in message.lines() {
                for line in wrapped(atlas, text, (w - 50.0).min(320.0)) {
                    centered(atlas, batch, &line, y, u32::MAX);
                    y += 10.0;
                }
            }
            if matches!(
                accounts::state(),
                State::Starting | State::Progress(_) | State::Resuming | State::Code { .. }
            ) {
                let phase = started.elapsed().as_millis() / 400 % 4;
                centered(
                    atlas,
                    batch,
                    &".".repeat(phase as usize + 1),
                    156.0,
                    rgba(170, 170, 170, 255),
                );
            }
            return;
        }
        match self.page {
            Page::Signing => {}
            Page::Offline | Page::Ely => {
                let fields = if self.page == Page::Offline {
                    vec![("name", &self.username, 16, "account-name")]
                } else {
                    vec![
                        ("login", &self.username, 254, "account-login"),
                        ("password", &self.password, 256, "account-password"),
                        ("otp", &self.otp, 6, "account-otp"),
                    ]
                };
                for (i, (key, value, max, id)) in fields.into_iter().enumerate() {
                    let y = 50.0 + 38.0 * i as f32;
                    centered(atlas, batch, &tr(key), y - 12.0, rgba(170, 170, 170, 255));
                    let field = ((id, 0), value.as_str(), max);
                    let rect = [(w / 2.0).floor() - 100.0, y, 200.0, 20.0];
                    if (key == "password" && !self.show_password) || key == "otp" {
                        caret.draw_password(batch, atlas, field, rect, self.focus == i, started);
                    } else {
                        caret.draw(batch, atlas, field, rect, self.focus == i, started);
                    }
                }
            }
            Page::Remove => {
                if let Some(p) = &self.remove {
                    centered(
                        atlas,
                        batch,
                        &with("remove_confirm", p.name()),
                        60.0,
                        u32::MAX,
                    );
                }
            }
            Page::Files => {
                let message = match accounts::files() {
                    Files::Missing => tr("files_missing"),
                    Files::Checking => tr("files_checking"),
                    Files::Ready => tr("files_ready"),
                    Files::Failed(e) => e,
                    Files::Downloading(p) => format!(
                        "{}: {}/{}; {:.1}/{:.1} MB",
                        tr("downloading"),
                        p.files,
                        p.files_total,
                        p.bytes as f64 / 1_000_000.0,
                        p.bytes_total as f64 / 1_000_000.0
                    ),
                };
                for (i, line) in wrapped(atlas, &message, (w - 50.0).min(320.0))
                    .iter()
                    .enumerate()
                {
                    centered(atlas, batch, line, 60.0 + i as f32 * 10.0, u32::MAX);
                }
            }
            Page::Add => centered(atlas, batch, &tr("choose_type"), 50.0, u32::MAX),
            Page::List => {
                let list = accounts::profiles();
                let message = if list.players().is_empty() {
                    tr("empty")
                } else {
                    list.player()
                        .map(|p| with("active", p.name()))
                        .unwrap_or_else(|| tr("choose"))
                };
                centered(atlas, batch, &message, 34.0, rgba(170, 170, 170, 255));
            }
        }
    }
    pub(super) fn buttons(&self, w: f32, h: f32) -> Vec<(f32, f32, f32, String, bool, u8)> {
        self.buttons_for(w, h, &accounts::state())
    }
    fn buttons_for(&self, w: f32, h: f32, state: &State) -> Vec<(f32, f32, f32, String, bool, u8)> {
        let x = (w / 2.0).floor() - 150.0;
        let b = |dx, y, width, label: String, enabled, id| (x + dx, y, width, label, enabled, id);
        if self.status_for(state).is_some() {
            return match state {
                State::Code { .. } => vec![
                    b(0.0, h - 52.0, 148.0, tr("open"), true, 62),
                    b(152.0, h - 52.0, 148.0, tr("copy"), true, 63),
                    b(0.0, h - 28.0, 148.0, tr("cancel"), true, 61),
                    b(152.0, h - 28.0, 148.0, tr("done"), false, 69),
                ],
                State::Failed(_) => vec![b(50.0, h - 28.0, 200.0, tr("back"), true, 70)],
                State::SignedIn(_) | State::OfflineProfile(_) => {
                    vec![b(50.0, h - 28.0, 200.0, tr("done"), true, 69)]
                }
                _ => vec![
                    b(0.0, h - 28.0, 148.0, tr("cancel"), true, 61),
                    b(152.0, h - 28.0, 148.0, tr("done"), false, 69),
                ],
            };
        }
        match self.page {
            Page::Signing => vec![b(50.0, h - 28.0, 200.0, tr("done"), true, 69)],
            Page::Add => vec![
                b(50.0, 80.0, 200.0, "Microsoft".into(), true, 60),
                b(50.0, 104.0, 200.0, "Ely.by".into(), true, 76),
                b(50.0, 128.0, 200.0, tr("offline"), true, 77),
                b(50.0, h - 28.0, 200.0, tr("back"), true, 70),
            ],
            Page::Offline | Page::Ely => {
                let valid = !self.username.trim().is_empty()
                    && (self.page == Page::Offline || !self.password.is_empty());
                let mut out = vec![
                    b(
                        if self.page == Page::Ely { 0.0 } else { 50.0 },
                        h - 52.0,
                        if self.page == Page::Ely { 148.0 } else { 200.0 },
                        tr(if self.page == Page::Ely {
                            "log_in"
                        } else {
                            "save"
                        }),
                        valid,
                        71,
                    ),
                    b(50.0, h - 28.0, 200.0, tr("back"), true, 70),
                ];
                if self.page == Page::Ely {
                    out.push(b(
                        50.0,
                        158.0,
                        200.0,
                        tr(if self.show_password {
                            "hide_password"
                        } else {
                            "show_password"
                        }),
                        true,
                        108,
                    ));
                    out.insert(1, b(152.0, h - 52.0, 148.0, tr("register"), true, 72));
                }
                out
            }
            Page::Files => vec![
                b(
                    50.0,
                    h - 52.0,
                    200.0,
                    tr(
                        if matches!(accounts::files(), Files::Checking | Files::Downloading(_)) {
                            "stop_files"
                        } else {
                            "files"
                        },
                    ),
                    true,
                    64,
                ),
                b(50.0, h - 28.0, 200.0, tr("back"), true, 70),
            ],
            Page::Remove => vec![
                b(0.0, h - 28.0, 148.0, tr("remove"), true, 73),
                b(152.0, h - 28.0, 148.0, tr("cancel"), true, 70),
            ],
            Page::List => {
                let list = accounts::profiles();
                let players = list.players();
                let n = rows(h);
                let offset = self.offset.min(players.len().saturating_sub(1) / n * n);
                let mut out = Vec::new();
                for (i, p) in players.iter().skip(offset).take(n).enumerate() {
                    let selected = list.player().is_some_and(|a| a.key() == p.key());
                    out.push(b(
                        0.0,
                        48.0 + i as f32 * 24.0,
                        274.0,
                        format!("{}{}", if selected { "> " } else { "" }, label(p)),
                        !selected,
                        80 + i as u8,
                    ));
                    out.push(b(
                        278.0,
                        48.0 + i as f32 * 24.0,
                        22.0,
                        tr("remove_short"),
                        true,
                        100 + i as u8,
                    ));
                }
                out.extend([
                    b(0.0, h - 76.0, 148.0, tr("previous"), offset > 0, 74),
                    b(
                        152.0,
                        h - 76.0,
                        148.0,
                        tr("next"),
                        offset + n < players.len(),
                        75,
                    ),
                    b(50.0, h - 52.0, 200.0, tr("add"), true, 78),
                    b(0.0, h - 28.0, 148.0, tr("files"), true, 79),
                    b(152.0, h - 28.0, 148.0, tr("done"), true, 69),
                ]);
                out
            }
        }
    }
    pub(super) fn click(&mut self, id: u8, h: f32) -> bool {
        match id {
            69 => {
                self.clear();
                return true;
            }
            60 => {
                self.clear();
                self.dismiss_error = false;
                self.microsoft = true;
                self.page = Page::Signing;
                accounts::sign_in();
            }
            61 => {
                self.clear();
                accounts::cancel();
            }
            62 | 63 => {
                if let State::Code { code, link } = accounts::state() {
                    if id == 62 {
                        let _ = crate::platform::open_link(&link);
                    } else {
                        crate::clipboard::Clipboard::set(
                            &mut crate::clipboard::SystemClipboard::default(),
                            &code,
                        );
                    }
                }
            }
            64 => {
                if matches!(accounts::files(), Files::Checking | Files::Downloading(_)) {
                    accounts::cancel_download();
                } else {
                    accounts::download_files();
                }
            }
            70 => {
                self.clear();
                self.dismiss_error = true;
            }
            71 => {
                self.dismiss_error = false;
                if self.page == Page::Offline {
                    accounts::add_offline(self.username.clone());
                } else {
                    accounts::sign_in_ely(
                        self.username.clone(),
                        std::mem::take(&mut self.password),
                        std::mem::take(&mut self.otp),
                    );
                }
                self.page = Page::Signing;
                self.show_password = false;
                self.microsoft = false;
            }
            108 => self.show_password = !self.show_password,
            72 => {
                let _ = crate::platform::open_link(if self.page == Page::Ely {
                    "https://account.ely.by/register"
                } else {
                    "https://signup.live.com/"
                });
            }
            73 => {
                if let Some(p) = self.remove.take() {
                    accounts::remove(p.key());
                }
                self.page = Page::List;
                self.dismiss_error = false;
            }
            74 => self.offset = self.offset.saturating_sub(rows(h)),
            75 => self.offset += rows(h),
            76 | 77 => {
                self.clear();
                self.page = if id == 76 { Page::Ely } else { Page::Offline };
            }
            79 => {
                self.clear();
                self.page = Page::Files;
            }
            78 => {
                self.clear();
                self.page = Page::Add;
            }
            80..=87 | 100..=107 => {
                let list = accounts::profiles().players();
                let n = rows(h);
                let offset = self.offset.min(list.len().saturating_sub(1) / n * n);
                if let Some(p) =
                    list.get(offset + usize::from(if id >= 100 { id - 100 } else { id - 80 }))
                {
                    if id >= 100 {
                        self.remove = Some(p.clone());
                        self.page = Page::Remove;
                    } else {
                        self.dismiss_error = false;
                        accounts::select(p.key());
                    }
                }
            }
            _ => {}
        }
        false
    }
    pub(super) fn back(&mut self) -> bool {
        if matches!(
            accounts::state(),
            State::Starting | State::Resuming | State::Progress(_) | State::Code { .. }
        ) {
            accounts::cancel();
            self.clear();
            return true;
        }
        if self.page != Page::List {
            self.clear();
            true
        } else {
            false
        }
    }
    pub(super) fn tab(&mut self, reverse: bool) -> bool {
        let n = match self.page {
            Page::Offline => 1,
            Page::Ely => 3,
            _ => return false,
        };
        self.focus = (self.focus + if reverse { n - 1 } else { 1 }) % n;
        true
    }
    pub(super) fn submit(&mut self, h: f32) -> bool {
        if matches!(self.page, Page::Offline | Page::Ely)
            && !self.username.trim().is_empty()
            && (self.page == Page::Offline || !self.password.is_empty())
        {
            self.click(71, h);
            true
        } else {
            false
        }
    }
    fn clear(&mut self) {
        self.show_password = false;
        self.microsoft = false;
        self.password.clear();
        self.otp.clear();
        self.username.clear();
        self.page = Page::List;
        self.focus = 0;
        self.remove = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signing_stays_on_its_result_and_enables_done_only_after_completion() {
        let mut screen = Screen {
            page: Page::Signing,
            microsoft: true,
            ..Default::default()
        };
        let done_enabled = |state: State| {
            screen
                .buttons_for(427.0, 267.0, &state)
                .iter()
                .any(|b| b.5 == 69 && b.4)
        };
        assert!(!done_enabled(State::Starting));
        assert!(!done_enabled(State::Code {
            code: "TEST-CODE".into(),
            link: "https://www.microsoft.com/link".into()
        }));
        assert!(!done_enabled(State::Progress(
            starling_account::SignInStage::Ownership
        )));
        let account = starling_account::Account {
            provider: Provider::Microsoft,
            uuid: "0123456789abcdef0123456789abcdef".into(),
            name: "TestPlayer".into(),
            skin: None,
            cape: None,
        };
        let state = State::SignedIn(account);
        assert!(done_enabled(state.clone()));
        assert!(screen.status_for(&state).is_some());
        let buttons = screen.buttons_for(427.0, 267.0, &state);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].5, 69);
        assert!(screen.click(69, 267.0));
        assert_eq!(screen.page, Page::List);
    }
    #[test]
    fn reveal_keeps_the_password_secret_for_input_and_resets_on_leaving() {
        let mut screen = Screen {
            page: Page::Ely,
            focus: 1,
            password: "Secret§9".into(),
            ..Default::default()
        };
        assert!(screen.password_masked());
        screen.click(108, 267.0);
        assert!(!screen.password_masked());
        assert!(screen.password_focused());
        assert!(!format!("{screen:?}").contains("Secret"));
        screen.focus = 2;
        assert!(screen.password_masked());
        screen.clear();
        assert!(!screen.show_password);
        assert!(screen.password.is_empty());
    }
}
