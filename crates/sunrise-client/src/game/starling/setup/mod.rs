//! Android: setting the game up on the phone when its resources are not there (or did not
//! load), in place of the loading failure. Sign in with the Microsoft account that owns the game
//! (accounts.rs) or play offline, the official files fetched, then prepared on the phone
//! (preparer.rs); or the resources prepared on a computer, picked as before. Drawn with the
//! embedded Sunrise font: the game's own is part of what is missing. While it waits, water to fish in (fishing.rs), and the
//! game's settings, drawn in the low-budget skin the app carries (packaging/lowbudget).
mod account_form;
pub(super) mod fishing;
#[cfg(target_os = "android")]
mod onboarding;
use account_form::{Form, Kind};

use super::{Audio, Loading, UiResources};
use std::cell::Cell;
use std::time::Instant;
use sunrise_client::{
    accounts::{self, Files, State},
    hud::HudBatch,
    preparer::{self, Stage},
    ui_theme::{Face, Ui, color},
};
use winit::event::{Touch, TouchPhase};

/// What the screen offers now.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    /// Before anything: the Starling folder the title screen offers too (files_offer.rs), so the
    /// game's files are where a player can reach them, and a bug report's log with them.
    #[cfg(target_os = "android")]
    Folder(super::files_offer::Stage),
    /// Horizon Android is on the phone: its worlds and resource packs can come over
    /// (SunriseImport.java). True once its folder was asked for: they are arriving.
    #[cfg(target_os = "android")]
    Sunrise(bool),
    /// Nothing under way: sign in, play offline, or bring resources from a computer.
    Start,
    /// Play offline chosen: whether the player is sure (most servers need a sign-in).
    Offline,
    /// The active provider's request or saved sign-in renewal.
    Talking(String),
    AccountForm,

    /// The player signs in elsewhere with `code`.
    Code {
        code: String,
        link: String,
    },
    /// The official files on their way: files and bytes so far, and in all.
    CheckingFiles,
    Download(accounts::Progress),
    /// Java on its way: bytes so far, and in all.
    Java(u64, u64, Option<std::time::Instant>),
    Unpacking,
    /// sunrise-prepare's last line.
    Preparing(String),
    /// First setup completed: choose what the app opens on subsequent starts.
    #[cfg(target_os = "android")]
    StartupScreen {
        directly: bool,
        error: Option<String>,
    },
    /// Prepared: the game loads.
    Done,
    /// The last attempt failed, and why.
    Failed(String),
    /// The sign-in failed (an account without the game, say): playing offline is offered too.
    SignInFailed(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    /// The Starling folder: Android's setting that allows it, or, allowed, the restart that moves
    /// the files.
    #[cfg(target_os = "android")]
    Folder,
    /// Not now (asked again on the title screen only if not turned down), or Later.
    #[cfg(target_os = "android")]
    FolderLater,
    /// Horizon Android's folder in the system's picker (again, if the player came back without
    /// picking it).
    #[cfg(target_os = "android")]
    Sunrise,
    /// Skip, or Next once its files are on their way: not offered again.
    #[cfg(target_os = "android")]
    SunriseDone,
    SignIn,
    Ely,
    AccountBack,
    SubmitForm,
    Register,
    Reveal,
    CancelSignIn,
    /// Play offline: asks first, then the files are fetched without a sign-in.
    Offline,
    PlayOffline,
    /// Back from the question to the start.
    NotOffline,
    OpenLink,
    CopyCode,
    /// Pick the resources prepared on a computer (as the loading failure did).
    Pick,
    /// After a failure: the step that failed, again.
    Retry,
    /// While it waits: to the water, back from it, or to the game's settings.
    Fish,
    Back,
    Settings,
    Diagnostics,
    #[cfg(target_os = "android")]
    FinishOnboarding(bool),
}

#[derive(Default)]
pub(super) struct Setup {
    /// The saved sign-in renewed, once.
    resumed: bool,
    /// The Starling folder's setting opened, waiting for the player to come back from it.
    #[cfg(target_os = "android")]
    folder_asked: bool,
    /// The Starling folder put off for this start (Later).
    #[cfg(target_os = "android")]
    folder_later: bool,
    /// Whether Horizon Android is installed, asked of Android once.
    #[cfg(target_os = "android")]
    sunrise_installed: Cell<Option<bool>>,
    /// Its folder asked for: its worlds and packs are on their way to the import inbox.
    #[cfg(target_os = "android")]
    sunrise_picked: bool,
    /// Play offline tapped: the question shows.
    offline_asked: bool,
    form: Option<Form>,
    provider: Option<Kind>,
    dismiss_signin_error: bool,
    cancelled: bool,
    offline_saved: bool,
    held: std::collections::HashSet<winit::keyboard::KeyCode>,
    /// Prepared once already: what it made did not load either (the failure says why), so it is
    /// not prepared again unasked.
    prepared: bool,
    /// Prepared and loading: the setup stays up until the menu's font is there (the loading
    /// screen without it draws every letter as an empty box).
    pub(super) starting: bool,
    #[cfg(target_os = "android")]
    onboarding: Option<onboarding::Onboarding>,
    /// The buttons as last drawn (dp): rectangle and what it does.
    buttons: Cell<[Option<([f32; 4], Action)>; 5]>,
    fishing: fishing::Fishing,
    /// The water as last drawn (dp), while there is a wait to pass.
    pond: Cell<Option<[f32; 4]>>,
    /// Out fishing (the water over the screen), not on the card.
    fishing_open: bool,
    /// The low-budget skin put on the menus, once.
    skinned: bool,
    /// The skin loading off the window's thread: its atlas waits for the renderer's word on the
    /// GPU's largest texture (`hud::atlas_limit`), and the renderer is built on this thread.
    skin: Option<std::thread::JoinHandle<Option<SkinLoad>>>,
}

/// The unpacked skin's folder and its menus, or why they did not load.
type SkinLoad = (std::path::PathBuf, Result<UiResources, String>);

fn setup_density(width: u32, height: u32) -> f32 {
    (width as f32 / 1040.0)
        .min(height as f32 / 700.0)
        .clamp(1.0, 4.0)
}

impl Setup {
    pub(in crate::game) fn onboarding_pending(&self) -> bool {
        #[cfg(target_os = "android")]
        return self.onboarding.is_some();
        #[cfg(not(target_os = "android"))]
        false
    }

    pub(in crate::game) fn onboarding_visible(&self) -> bool {
        #[cfg(target_os = "android")]
        return self.onboarding.as_ref().is_some_and(|setup| setup.visible);
        #[cfg(not(target_os = "android"))]
        false
    }

    pub(in crate::game) fn onboarding_already_chosen(&self) -> bool {
        #[cfg(target_os = "android")]
        return self.onboarding.as_ref().is_some_and(|setup| setup.chosen);
        #[cfg(not(target_os = "android"))]
        false
    }

    /// How soon the screen draws again: often while the water or the settings show.
    pub(super) fn frame(&self, settings_open: bool) -> std::time::Duration {
        let lively = self.pond.get().is_some() || settings_open;
        std::time::Duration::from_millis(if lively { 33 } else { 400 })
    }
}

/// Whether the step is a wait worth passing (the water shows).
fn waiting(step: &Step) -> bool {
    matches!(
        step,
        Step::CheckingFiles
            | Step::Download(_)
            | Step::Java(..)
            | Step::Unpacking
            | Step::Preparing(_)
    )
}

fn step(setup: &Setup, failure: &str) -> Step {
    #[cfg(target_os = "android")]
    if let Some(onboarding) = setup.onboarding.as_ref().filter(|setup| setup.visible) {
        return Step::StartupScreen {
            directly: onboarding.directly,
            error: onboarding.error.clone(),
        };
    }
    if setup.starting {
        return Step::Done;
    }
    if setup.prepared && preparer::stage() == Stage::Idle {
        return Step::Failed(format!("The prepared game files did not load: {failure}"));
    }
    match preparer::stage() {
        Stage::Java(done, total, started) => return Step::Java(done, total, Some(started)),
        Stage::Unpacking => return Step::Unpacking,
        Stage::Preparing(line) => return Step::Preparing(line),
        Stage::Done => return Step::Done,
        Stage::Failed(e) => return Step::Failed(e),
        Stage::Idle => {}
    }
    match accounts::files() {
        Files::Checking => return Step::CheckingFiles,
        Files::Downloading(progress) => return Step::Download(progress),
        Files::Failed(e) => return Step::Failed(e),
        // Ready: the preparing starts by itself (poll_setup).
        Files::Ready => return Step::Java(0, 0, None),
        Files::Missing => {}
    }
    if setup.form.is_some() {
        return Step::AccountForm;
    }
    account_step(setup, accounts::state())
}
fn account_step(setup: &Setup, state: State) -> Step {
    if setup.offline_asked {
        return Step::Offline;
    }
    if setup.cancelled
        && matches!(
            state,
            State::Offline | State::OfflineProfile(_) | State::SignedIn(_)
        )
    {
        return Step::Start;
    }
    match state {
        State::Failed(_) if setup.dismiss_signin_error => Step::Start,
        // Play offline tapped, at the start or after a failed sign-in: the question.
        State::Offline | State::OfflineProfile(_) | State::Failed(_) if setup.offline_asked => {
            Step::Offline
        }
        State::Offline | State::OfflineProfile(_) => {
            #[cfg(target_os = "android")]
            if let Some(stage) = folder_offer(setup) {
                return Step::Folder(stage);
            }
            // After the folder: what comes over lands in the game folder that stays.
            #[cfg(target_os = "android")]
            if sunrise_offer(setup) {
                return Step::Sunrise(setup.sunrise_picked);
            }
            Step::Start
        }
        State::Resuming => Step::Talking(tr("restoring")),
        State::Starting => Step::Talking(tr(if setup.provider == Some(Kind::Ely) {
            "ely_working"
        } else {
            "requesting_code"
        })),
        State::Progress(stage) => Step::Talking(sunrise_client::lang::get(stage.key())),
        State::SignedIn(_) => Step::Talking(tr("setup.preparing")),
        State::Code { code, link } => Step::Code { code, link },
        State::Failed(e) => Step::SignInFailed(if e.starts_with("starling.account.") {
            sunrise_client::lang::get(&e)
        } else {
            e
        }),
    }
}

/// The Starling folder's offer where it stands, while the game folder is still the app's own
/// files and the player has not turned it down.
#[cfg(target_os = "android")]
fn folder_offer(setup: &Setup) -> Option<super::files_offer::Stage> {
    use super::files_offer::{Stage, declined};
    let private = sunrise_client::logging::game_home() == sunrise_client::logging::private_home();
    if !private || setup.folder_later || declined().exists() {
        return None;
    }
    Some(if crate::android::file_access_allowed() {
        Stage::Restart
    } else if setup.folder_asked {
        Stage::Waiting
    } else {
        Stage::Ask
    })
}

/// Written once the player answered Horizon Android's offer (Next or Skip here, Done or Not now
/// over the title, game/files_offer.rs): it is not made again. It came back on every start until
/// the worlds came over, and a player keeping Horizon Android installed saw it every time.
#[cfg(target_os = "android")]
pub(super) fn sunrise_answered() -> std::path::PathBuf {
    sunrise_client::logging::private_home().join("sunrise-android-import-answered")
}

/// Answered in this start (the file above may not be writable).
#[cfg(target_os = "android")]
pub(super) static SUNRISE_LATER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether to offer Horizon Android's worlds and packs: it is installed, and not answered yet.
#[cfg(target_os = "android")]
fn sunrise_offer(setup: &Setup) -> bool {
    if sunrise_answered().exists() || SUNRISE_LATER.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    setup.sunrise_installed.get().unwrap_or_else(|| {
        let installed = crate::android::sunrise_android_installed();
        setup.sunrise_installed.set(Some(installed));
        installed
    })
}

fn mb(bytes: u64) -> String {
    format!("{:.0}", bytes as f64 / 1_000_000.0)
}

fn duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{}h {:02}m", seconds / 3600, seconds / 60 % 60)
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

fn transfer_details(
    received: u64,
    remaining: u64,
    elapsed: std::time::Duration,
    stalled: bool,
) -> String {
    let mut details = format!("{} elapsed", duration(elapsed.as_secs()));
    if stalled {
        details.push_str(" · Waiting for the next file to finish");
    } else if received > 0 && elapsed.as_secs() >= 3 {
        let rate = received as f64 / elapsed.as_secs_f64();
        let remaining = duration((remaining as f64 / rate).ceil() as u64);
        details = format!(
            "{:.2} MB/s avg · {details} · ~{remaining} left",
            rate / 1_000_000.0
        );
    }
    details
}

/// What the step says: title, body, and the line under it.
fn texts(step: &Step) -> (&'static str, String, Option<String>) {
    match step {
        #[cfg(target_os = "android")]
        Step::Folder(stage) => {
            let [title, body, ..] = super::files_offer::texts(*stage);
            (title, body.into(), None)
        }
        #[cfg(target_os = "android")]
        Step::Sunrise(false) => (
            "Coming from Horizon Android?",
            "Horizon Android is part of Starling now: bring your worlds and resource packs over. \
             Close Horizon Android first, then, in the folder that opens, tap Use this folder and \
             Allow."
                .into(),
            Some("They are copied: Horizon Android keeps its own.".into()),
        ),
        #[cfg(target_os = "android")]
        Step::Sunrise(true) => (
            "Coming from Horizon Android?",
            "Your worlds and resource packs are on their way: the note at the top says what each \
             one added. Tap Next when they are in."
                .into(),
            None,
        ),
        Step::Start => ("Set up Starling", tr("setup.intro"), None),
        Step::Offline => (
            "Are you sure?",
            "You will not be able to play on most servers! We highly recommend signing in with \
             Microsoft."
                .into(),
            None,
        ),
        Step::Talking(message) => ("Set up Starling", message.clone(), None),
        Step::AccountForm => ("Set up Starling", String::new(), None),
        Step::Code { link, .. } => (
            "Sign in with Microsoft",
            format!("On {link}, enter this code:"),
            Some(tr("waiting_browser")),
        ),
        Step::CheckingFiles => (
            "Checking game files",
            "Checking the version and asset list.".into(),
            None,
        ),
        Step::Download(progress) => (
            "Downloading the game",
            "From Mojang's servers, checked file by file.".into(),
            Some(if let Some(started) = progress.transfer_started {
                let stalled = progress
                    .last_transfer
                    .unwrap_or(started)
                    .elapsed()
                    .as_secs()
                    >= 15;
                let details = transfer_details(
                    progress.downloaded_bytes,
                    progress.bytes_total.saturating_sub(progress.bytes),
                    started.elapsed(),
                    stalled,
                );
                format!(
                    "{} of {} MB · {} of {} files\n{details}",
                    mb(progress.bytes),
                    mb(progress.bytes_total),
                    progress.files,
                    progress.files_total
                )
            } else {
                format!(
                    "Checking existing files · {} of {}",
                    progress.files, progress.files_total
                )
            }),
        ),
        Step::Java(done, total, started) => (
            "Downloading Java",
            "Java is needed to prepare the game files on this device.".into(),
            started.map(|started| {
                format!(
                    "{} of {} MB\n{}",
                    mb(*done),
                    mb(*total),
                    transfer_details(*done, total.saturating_sub(*done), started.elapsed(), false)
                )
            }),
        ),
        Step::Unpacking => ("Preparing the game", "Unpacking Java.".into(), None),
        Step::Preparing(line) => (
            "Preparing the game",
            "This takes a few minutes. Keep Starling open.".into(),
            Some(line.clone()),
        ),
        #[cfg(target_os = "android")]
        Step::StartupScreen { error, .. } => (
            "Starling is ready",
            "Open the launcher or go straight to the game. Starling remembers your choice. Change it anytime in Settings > Launcher."
                .into(),
            error.clone(),
        ),
        Step::Done => (
            "Starting Starling",
            "The game files are ready.".into(),
            None,
        ),
        Step::Failed(e) | Step::SignInFailed(e) => ("Something went wrong", e.clone(), None),
    }
}

/// The fraction done, for the bar.
fn fraction(step: &Step) -> Option<f32> {
    match step {
        Step::Download(progress) if progress.bytes_total > 0 => {
            Some((progress.bytes as f64 / progress.bytes_total as f64) as f32)
        }
        Step::Java(bytes, total, _) if *total > 0 => Some((*bytes as f64 / *total as f64) as f32),
        _ => None,
    }
}

/// The buttons, the main one first (drawn on the right).
fn buttons(step: &Step) -> Vec<(String, Action)> {
    let literal = |text: &str, action| (text.to_owned(), action);
    match step {
        #[cfg(target_os = "android")]
        Step::Folder(stage) => {
            let [_, _, primary, other] = super::files_offer::texts(*stage);
            vec![
                literal(primary, Action::Folder),
                literal(other, Action::FolderLater),
            ]
        }
        #[cfg(target_os = "android")]
        Step::Sunrise(false) => vec![
            literal("Bring them over", Action::Sunrise),
            literal("Skip", Action::SunriseDone),
        ],
        #[cfg(target_os = "android")]
        Step::Sunrise(true) => vec![
            literal("Next", Action::SunriseDone),
            literal("Pick the folder again", Action::Sunrise),
        ],
        #[cfg(target_os = "android")]
        Step::StartupScreen { .. } => vec![
            literal("Game", Action::FinishOnboarding(true)),
            literal("Launcher", Action::FinishOnboarding(false)),
        ],
        Step::Start => vec![
            (tr("setup.microsoft"), Action::SignIn),
            (tr("setup.ely_title"), Action::Ely),
            (tr("setup.offline_title"), Action::Offline),
            (tr("setup.import"), Action::Pick),
        ],
        Step::Offline => vec![
            (tr("setup.offline_title"), Action::PlayOffline),
            (tr("back"), Action::NotOffline),
        ],
        Step::Code { .. } => vec![
            (tr("open"), Action::OpenLink),
            (tr("copy"), Action::CopyCode),
            (tr("cancel"), Action::CancelSignIn),
        ],
        Step::Talking(_) => vec![(tr("cancel"), Action::CancelSignIn)],
        Step::Failed(_) => vec![
            literal("Try again", Action::Retry),
            (tr("setup.import"), Action::Pick),
        ],
        Step::SignInFailed(_) => vec![
            literal("Try again", Action::Retry),
            (tr("back"), Action::AccountBack),
            (tr("setup.offline_title"), Action::Offline),
        ],
        step if waiting(step) => vec![
            literal("Go fishing", Action::Fish),
            literal("Settings", Action::Settings),
        ],
        _ => Vec::new(),
    }
}
fn tr(key: &str) -> String {
    sunrise_client::lang::get(&format!("starling.account.{key}"))
}

/// Words of `text` in lines no wider than `max` dp; a word wider than that (a path) is cut where
/// it reaches the edge.
fn wrap(ui: &Ui<'_>, size: f32, text: &str, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        let mut words = Vec::new();
        for word in paragraph.split(' ') {
            let mut piece = String::new();
            for ch in word.chars() {
                piece.push(ch);
                if piece.chars().count() > 1 && ui.measure(Face::Regular, size, &piece) > max {
                    piece.pop();
                    words.push(std::mem::replace(&mut piece, ch.to_string()));
                }
            }
            words.push(piece);
        }
        for word in &words {
            let next = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && ui.measure(Face::Regular, size, &next) > max {
                lines.push(std::mem::replace(&mut line, word.to_owned()));
            } else {
                line = next;
            }
        }
        lines.push(line);
    }
    lines
}

impl Loading {
    /// Every frame on the setup: a saved sign-in renewed, the download after a sign-in, the
    /// preparing once the files are there, and the game loaded once they are prepared.
    pub(super) fn poll_setup(&mut self) {
        self.setup.fishing.update(Instant::now());
        if self.setup.form.as_mut().is_some_and(Form::poll) {
            self.setup_action(Action::SubmitForm);
        }
        if self.setup.offline_saved && matches!(accounts::state(), State::OfflineProfile(_)) {
            self.setup.offline_saved = false;
            accounts::download_files();
        }
        // The fishing's sounds, on an output of its own: the game's sounds are not there yet.
        if self.audio.is_none() && self.setup.pond.get().is_some() {
            let mut audio = Audio::with_registry(
                &sunrise_assets::resource_source::ResourceSource::directory(
                    self.vanilla_root.clone(),
                ),
                Default::default(),
            );
            audio.master = self.settings.master_volume as f32;
            self.audio = Some(audio);
        }
        for sound in self.setup.fishing.take_sounds() {
            if let Some(audio) = &mut self.audio {
                audio.play_samples(sound.samples(), fishing::RATE, 0.3);
            }
        }
        if !self.setup.resumed {
            self.setup.resumed = true;
            accounts::resume();
        }
        // The menus in the skin the app carries: their own resources are what is missing.
        if !self.setup.skinned {
            self.setup.skinned = true;
            self.setup.skin = std::thread::Builder::new()
                .name("setup-skin".into())
                .spawn(|| {
                    let root = preparer::skin()?;
                    let source =
                        sunrise_assets::resource_source::ResourceSource::directory(root.clone());
                    let ui = UiResources::load(&source, &root).map_err(|e| e.to_string());
                    Some((root, ui))
                })
                .inspect_err(|e| log::warn!("Sunrise: the setup's skin did not start: {e}"))
                .ok();
        }
        if self.setup.skin.as_ref().is_some_and(|h| h.is_finished())
            && let Some(handle) = self.setup.skin.take()
        {
            match handle.join() {
                Ok(Some((root, Ok(gui)))) => self.set_ui(gui, root),
                Ok(Some((_, Err(e)))) => log::warn!("Sunrise: the setup's skin did not load: {e}"),
                Ok(None) => {}
                Err(_) => log::warn!("Sunrise: the setup's skin load panicked"),
            }
        }
        if preparer::stage() == Stage::Done {
            self.setup.prepared = true;
            preparer::clear();
            // The game opens its sound output with the sounds just prepared.
            self.audio = None;
            match self.retry() {
                Ok(()) => self.setup.starting = true,
                Err(e) => self.failure = Some(e.to_string()),
            }
            return;
        }
        if preparer::running() {
            return;
        }
        match (accounts::files(), accounts::state()) {
            (Files::Ready, _) if preparer::stage() == Stage::Idle && !self.setup.prepared => {
                preparer::start(self.vanilla_root.clone());
            }
            (Files::Missing, State::SignedIn(_))
                if !self.setup.cancelled && self.setup.form.is_none() =>
            {
                accounts::download_files()
            }
            _ => {}
        }
    }

    pub(super) fn draw_setup(&self, batch: &mut HudBatch, failure: &str) {
        let atlas = match &self.gui {
            Some(gui) => &gui.atlas,
            None if self.setup.onboarding_pending() => &self.boot,
            None => {
                // The boot screen holds the window while the setup skin loads.
                let logs = sunrise_client::logging::recent_lines();
                sunrise_client::ui_theme::draw_boot(&self.boot, batch, &logs);
                return;
            }
        };
        let step = step(&self.setup, failure);
        let (title, body, note) = texts(&step);
        let mut ui = Ui::new(atlas, batch, (-1.0, -1.0));
        let (w, h) = (ui.w, ui.h);
        if let Some(form) = &self.setup.form {
            ui.fill(0., 0., w, h, color::bg());
            let mut drawn = [None; 5];
            #[cfg(target_os = "android")]
            let available_h =
                (h - sunrise_client::android::ime_height() as f32 / ui.density).max(320.);
            #[cfg(not(target_os = "android"))]
            let available_h = h;
            form.draw(&mut ui, &mut drawn, available_h);
            let rect = [(w - 160.).max(16.), 8., 144., 32.];
            ui.rounded(rect[0], rect[1], rect[2], rect[3], 8., color::raised());
            ui.text_centered(
                Face::Semibold,
                13.,
                rect[0] + 72.,
                rect[1] + 8.,
                color::text(),
                &sunrise_client::lang::get("starling.diagnostics.title"),
            );
            drawn[4] = Some((rect, Action::Diagnostics));
            ui.finish();
            self.setup.buttons.set(drawn);
            self.setup.pond.set(None);
            return;
        }
        let now = Instant::now();
        let pond = (waiting(&step) && self.setup.fishing_open).then_some([0.0, 0.0, w, h]);
        if pond.is_some() {
            self.setup.fishing.draw_water(&mut ui, w, h, now);
        } else {
            ui.fill(0.0, 0.0, w, h, color::bg());
        }
        // Out fishing, the card only says how far it is; Back is in the corner.
        let actions = if pond.is_some() {
            Vec::new()
        } else {
            buttons(&step)
        };
        // Wider for three buttons, so "Sign in with Microsoft" fits in a third.
        let three = actions.len() >= 3;
        let card_w = (w - 48.0).min(if three { 660.0 } else { 520.0 });
        let pad = 24.0;
        let inner = card_w - 2.0 * pad;
        let mut lines = wrap(&ui, 14.0, &body, inner);
        if lines.len() > 9 {
            lines.truncate(8);
            lines.push("...".into());
        }
        let note = note.map(|n| wrap(&ui, 12.0, &n, inner));
        let code = match &step {
            Step::Code { code, .. } => Some(code.clone()),
            _ => None,
        };
        let bar = fraction(&step);
        let button_h = 40.0;
        let mut card_h = pad + 26.0 + 12.0 + lines.len() as f32 * 21.0;
        if code.is_some() {
            card_h += 52.0;
        }
        if bar.is_some() {
            card_h += 22.0;
        }
        if let Some(note) = &note {
            card_h += 8.0 + note.len() as f32 * 18.0;
        }
        if !actions.is_empty() {
            card_h += 22.0 + button_h;
            if actions.len() > 3 {
                card_h += button_h + 12.;
            }
        }
        card_h += pad;
        // With a wait to pass, the card goes up, over the water.
        let (x, y) = if pond.is_some() {
            ((w - card_w) / 2.0, 16.0)
        } else {
            ((w - card_w) / 2.0, ((h - card_h) / 2.0).max(8.0))
        };
        let card = if pond.is_some() {
            color::with_alpha(color::bg(), 0.88)
        } else {
            color::bg()
        };
        ui.rounded(x, y, card_w, card_h, 14.0, card);
        ui.border(x, y, card_w, card_h, 14.0, color::border());
        ui.text(Face::Semibold, 18.0, x + pad, y + pad, color::text(), title);
        let mut ty = y + pad + 26.0 + 12.0;
        for line in &lines {
            ui.text(Face::Regular, 14.0, x + pad, ty, color::MUTED, line);
            ty += 21.0;
        }
        if let Some(code) = &code {
            ui.text_centered(
                Face::Semibold,
                30.0,
                x + card_w / 2.0,
                ty + 8.0,
                color::text(),
                code,
            );
            ty += 52.0;
        }
        if let Some(done) = bar {
            ty += 6.0;
            ui.rounded(x + pad, ty, inner, 8.0, 4.0, color::raised());
            ui.rounded(
                x + pad,
                ty,
                inner * done.clamp(0.0, 1.0),
                8.0,
                4.0,
                color::ACCENT,
            );
            ty += 16.0;
        }
        if let Some(note) = &note {
            ty += 8.0;
            for line in note {
                ui.text(Face::Regular, 12.0, x + pad, ty, color::MUTED, line);
                ty += 18.0;
            }
        }
        let mut drawn = [None; 5];
        let shown: Vec<_> = actions.iter().collect();
        if !shown.is_empty() {
            let by =
                y + card_h - pad - button_h - if shown.len() > 3 { button_h + 12. } else { 0. };
            let columns = if shown.len() > 3 { 2 } else { shown.len() };
            let bw = (inner - 12.0 * (columns - 1) as f32) / columns as f32;
            for (i, (text, action)) in shown.iter().enumerate() {
                // The first is the main one, on the right.
                let bx = x + card_w - pad - bw - (bw + 12.0) * (i % columns) as f32;
                let rect = [
                    bx,
                    by + (i / columns) as f32 * (button_h + 12.),
                    bw,
                    button_h,
                ];
                let primary = match (&step, action) {
                    #[cfg(target_os = "android")]
                    (Step::StartupScreen { directly, .. }, Action::FinishOnboarding(choice)) => {
                        directly == choice
                    }
                    _ => i == 0,
                };
                let (fill, ink) = if primary {
                    (color::ACCENT, color::ON_ACCENT)
                } else {
                    (color::raised(), color::text())
                };
                ui.rounded(rect[0], rect[1], rect[2], rect[3], 10.0, fill);
                ui.text_centered(
                    Face::Semibold,
                    15.0,
                    rect[0] + rect[2] / 2.0,
                    rect[1] + (rect[3] - 15.0) / 2.0 - 2.0,
                    ink,
                    text,
                );
                drawn[i] = Some((rect, *action));
            }
        }
        // Why the game could not start, small, under the card: for a report. Not on a first start,
        // where the game files are simply not there yet.
        if matches!(step, Step::Start) && self.vanilla_root.exists() {
            for (i, line) in wrap(&ui, 11.0, failure, card_w).iter().take(2).enumerate() {
                let ly = y + card_h + 14.0 + i as f32 * 15.0;
                ui.text(
                    Face::Regular,
                    11.0,
                    x,
                    ly,
                    color::with_alpha(color::MUTED, 0.6),
                    line,
                );
            }
        }
        if pond.is_some() {
            self.setup.fishing.draw(&mut ui, w, h, h - 58.0, now);
            let rect = [16.0, 16.0, 92.0, 36.0];
            ui.rounded(
                rect[0],
                rect[1],
                rect[2],
                rect[3],
                18.0,
                color::with_alpha(color::bg(), 0.88),
            );
            ui.text_centered(
                Face::Semibold,
                15.0,
                rect[0] + rect[2] / 2.0,
                rect[1] + 9.0,
                color::text(),
                "Back",
            );
            drawn[0] = Some((rect, Action::Back));
        }
        // Reports must be reachable even before sign-in or resource preparation.
        let rect = [(w - 160.0).max(16.0), 16.0, 144.0, 36.0];
        ui.rounded(rect[0], rect[1], rect[2], rect[3], 10.0, color::raised());
        ui.text_centered(
            Face::Semibold,
            15.0,
            rect[0] + rect[2] / 2.0,
            rect[1] + 8.0,
            color::text(),
            &sunrise_client::lang::get("starling.diagnostics.title"),
        );
        drawn[4] = Some((rect, Action::Diagnostics));
        ui.finish();
        self.setup.buttons.set(drawn);
        // Fishing under the card: the card itself takes no cast.
        let below = y + card_h + 8.0;
        self.setup
            .pond
            .set(pond.map(|_| [0.0, below, w, h - below]));
        // The inbox's banner over it all, since the setup polls the inbox too: what its file
        // picker, or Horizon Android's folder, added.
        self.imports.draw(atlas, batch);
    }

    pub(super) fn setup_key_event(&mut self, event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{KeyCode, PhysicalKey};
        let PhysicalKey::Code(key) = event.physical_key else {
            return self.setup.form.is_some();
        };
        if event.state != winit::event::ElementState::Pressed {
            self.setup.held.remove(&key);
            return self.setup.form.is_some();
        }
        self.setup.held.insert(key);
        if self.setup.form.is_some() {
            match key {
                KeyCode::Escape => self.setup_action(Action::AccountBack),
                KeyCode::Enter | KeyCode::NumpadEnter => self.setup_action(Action::SubmitForm),
                _ => {
                    let ctrl = self.setup.held.contains(&KeyCode::ControlLeft)
                        || self.setup.held.contains(&KeyCode::ControlRight);
                    let shift = self.setup.held.contains(&KeyCode::ShiftLeft)
                        || self.setup.held.contains(&KeyCode::ShiftRight);
                    self.setup
                        .form
                        .as_mut()
                        .unwrap()
                        .key(key, event.text.as_deref(), ctrl, shift);
                }
            }
            return true;
        }
        if key == KeyCode::Escape
            && matches!(
                accounts::state(),
                State::Starting | State::Progress(_) | State::Code { .. }
            )
        {
            self.setup_action(Action::CancelSignIn);
            return true;
        }
        false
    }

    /// A finger lifted on a button acts.
    pub(super) fn setup_touch(&mut self, touch: &Touch) {
        let Some(window) = &self.window else { return };
        let size = window.inner_size();
        let density = setup_density(size.width, size.height);
        let p = [
            touch.location.x as f32 / density,
            touch.location.y as f32 / density,
        ];
        let inside = |r: [f32; 4]| {
            p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
        };
        let on_button = self
            .setup
            .buttons
            .get()
            .into_iter()
            .flatten()
            .any(|(r, _)| inside(r));
        // The water: a finger down casts, lifted it reels in.
        match touch.phase {
            TouchPhase::Started if !on_button && self.setup.pond.get().is_some_and(inside) => {
                self.setup.fishing.press(Instant::now(), p);
                return;
            }
            TouchPhase::Ended | TouchPhase::Cancelled => self.setup.fishing.release(Instant::now()),
            _ => {}
        }
        if touch.phase != TouchPhase::Ended {
            return;
        }
        if touch.phase == TouchPhase::Ended
            && !on_button
            && let Some(form) = &mut self.setup.form
        {
            if let Some(i) = form
                .rects
                .get()
                .iter()
                .position(|r| r[2] > 0. && inside(*r))
            {
                form.focus(i);
            } else {
                form.hide();
            }
            return;
        }
        self.setup_action_at(&inside);
    }

    fn setup_action_at(&mut self, inside: &impl Fn([f32; 4]) -> bool) {
        let hit = self
            .setup
            .buttons
            .get()
            .into_iter()
            .flatten()
            .find(|(r, _)| inside(*r));
        let Some((_, action)) = hit else { return };
        self.setup_action(action);
    }
    fn setup_action(&mut self, action: Action) {
        match action {
            #[cfg(target_os = "android")]
            Action::FinishOnboarding(directly) => self.finish_onboarding(directly),
            Action::Ely => {
                self.setup.provider = Some(Kind::Ely);
                self.setup.dismiss_signin_error = true;
                self.setup.cancelled = true;
                self.setup.form = Some(Form::new(Kind::Ely));
            }
            Action::SubmitForm => {
                if let Some(form) = &mut self.setup.form {
                    let kind = form.kind;
                    if form.submit() {
                        self.setup.form = None;
                        self.setup.dismiss_signin_error = false;
                        self.setup.cancelled = false;
                        self.setup.provider = Some(kind);
                        self.setup.offline_saved = kind == Kind::Offline;
                    }
                }
            }
            Action::AccountBack => {
                self.setup.form = None;
                self.setup.dismiss_signin_error = true;
                self.setup.offline_asked = false;
                self.setup.cancelled = true;
            }
            Action::Reveal => {
                if let Some(form) = &mut self.setup.form {
                    form.reveal();
                }
            }
            Action::Register => {
                let _ = sunrise_client::platform::open_link("https://account.ely.by/register");
            }
            Action::CancelSignIn => {
                accounts::cancel();
                self.setup.cancelled = true;
                self.setup.dismiss_signin_error = true;
            }
            #[cfg(target_os = "android")]
            Action::Folder => match folder_offer(&self.setup) {
                Some(super::files_offer::Stage::Restart) => {
                    super::files_offer::restart_moving_files()
                }
                _ => {
                    crate::android::open_file_access_settings();
                    self.setup.folder_asked = true;
                }
            },
            #[cfg(target_os = "android")]
            Action::FolderLater => {
                // Not now: as on the title screen, not offered again (Later, once allowed: the
                // next start moves the files anyway).
                if folder_offer(&self.setup) != Some(super::files_offer::Stage::Restart) {
                    let _ = std::fs::write(super::files_offer::declined(), b"");
                }
                self.setup.folder_later = true;
            }
            #[cfg(target_os = "android")]
            Action::Sunrise => {
                if crate::android::import_sunrise_android() {
                    self.setup.sunrise_picked = true;
                } else {
                    log::warn!("Sunrise: no folder picker for Horizon Android's files");
                }
            }
            #[cfg(target_os = "android")]
            Action::SunriseDone => {
                // Brought over or skipped, it is answered: the title doesn't ask on every start.
                let _ = std::fs::write(sunrise_answered(), b"");
                SUNRISE_LATER.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Action::SignIn => {
                self.setup.form = None;
                self.setup.provider = None;
                self.setup.dismiss_signin_error = false;
                self.setup.cancelled = false;
                self.setup.offline_asked = false;
                accounts::sign_in();
            }
            Action::Offline => self.setup.offline_asked = true,
            Action::PlayOffline => {
                self.setup.offline_asked = false;
                // The name the player already has (an offline profile, an account, the
                // launcher's) is in the field: one tap, not a second sign-in.
                let known = match accounts::state() {
                    State::OfflineProfile(name) => name,
                    State::SignedIn(account) => account.name,
                    _ => {
                        crate::ha_launcher::catalog::Config::load(
                            &crate::ha_launcher::backend::data_dir(),
                        )
                        .name
                    }
                };
                self.setup.form = Some(Form::new(Kind::Offline).with_first(known.trim()));
                self.setup.provider = Some(Kind::Offline);
            }
            Action::NotOffline => self.setup.offline_asked = false,
            Action::OpenLink => {
                if let State::Code { link, .. } = accounts::state()
                    && !sunrise_client::platform::open_link(&link).unwrap_or(false)
                {
                    log::warn!("Sunrise: no browser opened {link}");
                }
            }
            Action::CopyCode => {
                if let State::Code { code, .. } = accounts::state() {
                    let _ = sunrise_client::platform::clipboard_set(&code);
                }
            }
            Action::Pick => {
                if !sunrise_client::platform::pick_file() {
                    log::warn!("Sunrise: no file picker to import resources with");
                }
            }
            Action::Fish => self.setup.fishing_open = true,
            Action::Back => {
                self.setup.fishing.release(Instant::now());
                self.setup.fishing_open = false;
            }
            Action::Settings => self.open_menu_options(sunrise_client::options_ui::Page::Main),
            Action::Diagnostics => {
                if let Some(form) = &mut self.setup.form {
                    form.hide();
                }
                #[cfg(target_os = "android")]
                crate::android::open_recovery();
                #[cfg(not(target_os = "android"))]
                self.open_menu_options(sunrise_client::options_ui::Page::Diagnostics);
            }
            Action::Retry => match (preparer::stage(), accounts::files()) {
                (Stage::Failed(_) | Stage::Idle, Files::Ready) => {
                    preparer::start(self.vanilla_root.clone())
                }
                (_, Files::Failed(_)) => accounts::download_files(),
                _ if self.setup.provider == Some(Kind::Ely) => self.setup_action(Action::Ely),
                _ if self.setup.provider == Some(Kind::Offline) => {
                    self.setup.form = Some(Form::new(Kind::Offline));
                }
                _ => self.setup_action(Action::SignIn),
            },
        }
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;
    #[test]
    fn first_start_offers_both_providers_offline_and_import_without_losing_diagnostics_slot() {
        let actions: Vec<_> = buttons(&Step::Start).into_iter().map(|(_, a)| a).collect();
        assert!(actions.contains(&Action::SignIn));
        assert!(actions.contains(&Action::Ely));
        assert!(actions.contains(&Action::Offline));
        assert!(actions.contains(&Action::Pick));
        assert_eq!(actions.len(), 4);
    }
    #[test]
    fn cancelled_setup_does_not_trap_a_saved_profile_in_talking() {
        let setup = Setup {
            cancelled: true,
            ..Default::default()
        };
        assert_eq!(
            account_step(&setup, State::OfflineProfile("Local".into())),
            Step::Start
        );
        let a = accounts::Account {
            provider: accounts::Provider::Ely,
            uuid: "0123456789abcdef0123456789abcdef".into(),
            name: "Player".into(),
            skin: None,
            cape: None,
        };
        assert_eq!(account_step(&setup, State::SignedIn(a)), Step::Start);
    }
    #[test]
    fn setup_keeps_the_actual_provider_progress_and_error() {
        let setup = Setup {
            provider: Some(Kind::Ely),
            ..Default::default()
        };
        assert_eq!(
            account_step(&setup, State::Starting),
            Step::Talking(tr("ely_working"))
        );
        let error = account_step(
            &setup,
            State::Failed("starling.account.error.ely_otp_needed".into()),
        );
        assert!(matches!(error, Step::SignInFailed(_)));
        let actions = buttons(&error);
        assert!(actions.iter().any(|(_, a)| *a == Action::AccountBack));
        let stage = accounts::SignInStage::Ownership;
        assert_eq!(
            account_step(&Setup::default(), State::Progress(stage)),
            Step::Talking(sunrise_client::lang::get(stage.key()))
        );
    }
}
