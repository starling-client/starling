// game-module-cfg: target_os = "android"
//! Android: the offer, over the title screen, to keep the game folder in shared storage (a
//! Starling folder any file manager opens). Android's "all files" setting does the allowing, in a
//! task of its own; once allowed, a restart moves the player's files (android.rs
//! `game_folder`). Turned down, it comes back from the folder buttons (Open Pack Folder...).
//!
//! After it, the offer of Horizon Android's worlds and resource packs (HorizonImport.java), for a
//! player who set Starling up before the setup made it (game/setup.rs): on every start while
//! Horizon Android is installed, until they came over.
use super::Game;
use super::setup::{HORIZON_LATER, horizon_answered};
use horizon_client::{
    hud::HudBatch,
    ui_theme::{Face, Ui, color},
};
use std::cell::Cell;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use winit::event::{Touch, TouchPhase};

/// How often the setting is looked at while the player is away in it.
const CHECK: Duration = Duration::from_millis(700);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    /// What the folder is for, and the way to allow it.
    Ask,
    /// Sent to the setting; asked again if they come back without allowing.
    Waiting,
    /// Allowed: a restart moves the files.
    Restart,
}

/// The card over the title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Card {
    Folder(Stage),
    /// Horizon Android's worlds and packs: true once the player picked its folder.
    Horizon(bool),
}

#[derive(Default)]
pub(super) struct FilesOffer {
    stage: Option<Stage>,
    /// Horizon Android's offer, shown when the folder's is not: true once its folder is picked.
    horizon: Option<bool>,
    started: bool,
    /// Made again from a folder button (Open Pack Folder...): it shows over any screen.
    asked: bool,
    checked: Option<Instant>,
    /// The buttons as last drawn (UI units): the primary's and the other's rectangles.
    buttons: Cell<[[f32; 4]; 2]>,
}

/// Written when the player turns the offer down (here or in the setup, game/setup.rs): it is not
/// made again.
pub(super) fn declined() -> PathBuf {
    horizon_client::logging::private_home().join("files-offer-declined")
}

pub(super) fn texts(stage: Stage) -> [&'static str; 4] {
    match stage {
        Stage::Ask => [
            "Keep your files where you can reach them",
            "Starling can keep your worlds, packs, mods and screenshots in a Starling folder on \
             your phone's storage, which any file manager (and a computer over USB) opens. \
             Android will ask you to allow access to files for that.",
            "Set up",
            "Not now",
        ],
        Stage::Waiting => [
            "Allow access to files",
            "Turn on the switch for Starling in the screen that opened, then come back here.",
            "Open it again",
            "Not now",
        ],
        Stage::Restart => [
            "All set",
            "Restart Starling to move your worlds, packs and settings into the Starling folder. \
             It copies them first: nothing is lost.",
            "Restart now",
            "Later",
        ],
    }
}

fn card_texts(card: Card) -> [&'static str; 4] {
    match card {
        Card::Folder(stage) => texts(stage),
        Card::Horizon(false) => [
            "Coming from Horizon Android?",
            "Horizon Android is part of Starling now: bring your worlds and resource packs over. \
             Close Horizon Android first, then, in the folder that opens, tap Use this folder and \
             Allow. They are copied: Horizon Android keeps its own.",
            "Bring them over",
            "Not now",
        ],
        Card::Horizon(true) => [
            "Coming from Horizon Android?",
            "Your worlds and resource packs are on their way: the note at the top says what each \
             one added. Tap Done when they are in.",
            "Done",
            "Pick the folder again",
        ],
    }
}

/// The UI's scale for a window of `w` x `h` pixels (as `Ui::new`).
fn density(w: f32, h: f32) -> f32 {
    (w / 1040.0).min(h / 700.0).clamp(1.0, 4.0)
}

impl Game<'_> {
    /// Once at start, then while the player is away in the setting.
    pub(super) fn poll_files_offer(&mut self) {
        let offer = &mut self.files_offer;
        if !offer.started {
            offer.started = true;
            let private =
                horizon_client::logging::game_home() == horizon_client::logging::private_home();
            if private && !declined().exists() {
                offer.stage = Some(if crate::android::file_access_allowed() {
                    Stage::Restart
                } else {
                    Stage::Ask
                });
            }
            if !horizon_answered().exists()
                && !HORIZON_LATER.load(std::sync::atomic::Ordering::Relaxed)
                && crate::android::horizon_android_installed()
            {
                offer.horizon = Some(false);
            }
        }
        if matches!(offer.stage, Some(Stage::Ask | Stage::Waiting))
            && offer.checked.is_none_or(|t| t.elapsed() >= CHECK)
        {
            offer.checked = Some(Instant::now());
            if crate::android::file_access_allowed() {
                offer.stage = Some(Stage::Restart);
            }
        }
    }

    /// A folder button while the game folder is still in the app's private files, which no file
    /// manager opens: the offer again, turned down or not.
    pub(super) fn ask_files_offer(&mut self) {
        let _ = std::fs::remove_file(declined());
        let offer = &mut self.files_offer;
        offer.stage = Some(if crate::android::file_access_allowed() {
            Stage::Restart
        } else {
            Stage::Ask
        });
        offer.asked = true;
    }

    pub(super) fn files_offer_shown(&self) -> Option<Card> {
        let offer = &self.files_offer;
        if let Some(stage) = offer.stage.filter(|_| self.title.is_some() || offer.asked) {
            return Some(Card::Folder(stage));
        }
        offer
            .horizon
            .filter(|_| self.title.is_some())
            .map(Card::Horizon)
    }

    /// A card in the middle of the title screen, the rest dimmed.
    pub(super) fn draw_files_offer(&self, batch: &mut HudBatch) {
        let Some(card) = self.files_offer_shown() else {
            return;
        };
        let [title, body, primary, other] = card_texts(card);
        let mut ui = Ui::new(&self.gui.atlas, batch, (-1.0, -1.0));
        let (w, h) = (ui.w, ui.h);
        ui.fill(0.0, 0.0, w, h, color::with_alpha(color::bg(), 0.72));
        let card_w = (w - 48.0).min(470.0);
        let pad = 22.0;
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in body.split(' ') {
            let next = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && ui.measure(Face::Regular, 14.0, &next) > card_w - 2.0 * pad {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = next;
            }
        }
        lines.push(line);
        let button_h = 40.0;
        let card_h = pad + 26.0 + 12.0 + lines.len() as f32 * 21.0 + 22.0 + button_h + pad;
        let (x, y) = ((w - card_w) / 2.0, ((h - card_h) / 2.0).max(8.0));
        ui.rounded(x, y, card_w, card_h, 14.0, color::bg());
        ui.border(x, y, card_w, card_h, 14.0, color::border());
        ui.text(Face::Semibold, 18.0, x + pad, y + pad, color::text(), title);
        let mut ty = y + pad + 26.0 + 12.0;
        for line in &lines {
            ui.text(Face::Regular, 14.0, x + pad, ty, color::MUTED, line);
            ty += 21.0;
        }
        let by = y + card_h - pad - button_h;
        let bw = (card_w - 2.0 * pad - 12.0) / 2.0;
        let primary_rect = [x + card_w - pad - bw, by, bw, button_h];
        let other_rect = [x + pad, by, bw, button_h];
        for (rect, text, fill, ink) in [
            (primary_rect, primary, color::ACCENT, color::ON_ACCENT),
            (other_rect, other, color::raised(), color::text()),
        ] {
            ui.rounded(rect[0], rect[1], rect[2], rect[3], 10.0, fill);
            ui.text_centered(
                Face::Semibold,
                15.0,
                rect[0] + rect[2] / 2.0,
                rect[1] + (rect[3] - 15.0) / 2.0 - 2.0,
                ink,
                text,
            );
        }
        ui.finish();
        self.files_offer.buttons.set([primary_rect, other_rect]);
    }

    /// While the card shows it takes every touch; a lifted finger on a button acts.
    pub(super) fn files_offer_touch(&mut self, touch: &Touch) -> bool {
        let Some(card) = self.files_offer_shown() else {
            return false;
        };
        if touch.phase != TouchPhase::Ended {
            return true;
        }
        let [w, h] = self.touch_size();
        let d = density(w as f32, h as f32);
        let p = [touch.location.x as f32 / d, touch.location.y as f32 / d];
        let inside = |r: [f32; 4]| {
            p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
        };
        let [primary, other] = self.files_offer.buttons.get();
        let button = if inside(primary) {
            0
        } else if inside(other) {
            1
        } else {
            return true;
        };
        let offer = &mut self.files_offer;
        match (card, button) {
            (Card::Folder(Stage::Ask | Stage::Waiting), 0) => {
                crate::android::open_file_access_settings();
                offer.stage = Some(Stage::Waiting);
            }
            (Card::Folder(Stage::Restart), 0) => crate::android::restart_app(),
            (Card::Folder(stage), _) => {
                if stage != Stage::Restart {
                    let _ = std::fs::write(declined(), b"");
                }
                // Later: the next start moves the files anyway.
                offer.stage = None;
                offer.asked = false;
            }
            (Card::Horizon(false), 0) | (Card::Horizon(true), 1) => {
                if crate::android::import_horizon_android() {
                    offer.horizon = Some(true);
                } else {
                    self.imports.say_failed("No folder picker opened".into());
                }
            }
            (Card::Horizon(true), _) => {
                let _ = std::fs::write(horizon_answered(), b"");
                offer.horizon = None;
            }
            (Card::Horizon(false), _) => {
                HORIZON_LATER.store(true, std::sync::atomic::Ordering::Relaxed);
                offer.horizon = None;
            }
        }
        true
    }
}
