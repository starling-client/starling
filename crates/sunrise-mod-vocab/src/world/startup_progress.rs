//! Mod lines and progress bars on the startup loading screen: NeoForge's
//! `StartupNotificationManager` (`addModMessage`, `addProgressBar`, `popBar`) and `ProgressMeter`.
use std::sync::{Arc, OnceLock};

/// A native startup progress operation could not be completed.
// no wire: startup progress errors are returned in-process.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartupProgressError {
    /// The calling mod already has 64 open bars. Close a bar before retrying.
    TooManyBars,
    /// The game's screen has already been installed.
    AlreadyInstalled,
}
impl std::fmt::Display for StartupProgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyBars => {
                f.write_str("the calling mod already has 64 open startup progress bars")
            }
            Self::AlreadyInstalled => f.write_str("the startup screen is already installed"),
        }
    }
}
impl std::error::Error for StartupProgressError {}

/// The game's side of the startup loading screen. A mod reaches it through
/// [`RegistrationContext::progress`](crate::RegistrationContext::progress) and
/// [`RegistrationContext::message`](crate::RegistrationContext::message), not directly.
pub trait StartupScreen: Send + Sync + 'static {
    /// Open a bar of `steps` steps under the stage bar and return its owned meter.
    /// `mod_id` identifies the registration listener, even on a scoped worker.
    /// The screen borrows `label` for the call; the meter does not borrow it.
    /// Returns `StartupProgressError::TooManyBars` if the calling mod already has 64 open bars.
    fn open(
        self: Arc<Self>,
        mod_id: &'static str,
        label: &str,
        steps: u32,
    ) -> Result<Box<dyn ProgressMeter>, StartupProgressError>;

    /// Show a line on the screen for a few seconds, or log it on a dedicated server.
    fn message(&self, text: &str);
}

/// The game's side of one startup bar. Its owner may move it to or share it with
/// worker threads. Implementors remove their bar when the meter is dropped.
pub trait ProgressMeter: Send + Sync + 'static {
    /// One more step is done, up to the bar's total steps.
    fn step(&self);
    /// `done` of the bar's steps are done, up to its total steps.
    fn set(&self, done: u32);
    /// Replace the bar's label. The new label may outlive `text`.
    fn label(&self, text: &str);
    /// Show a startup line, separate from the bar's label, or log it on a dedicated server.
    fn message(&self, text: &str);
}

static SCREEN: OnceLock<Arc<dyn StartupScreen>> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
std::thread_local! {
    pub(crate) static TEST_SCREEN: std::cell::RefCell<Option<Arc<dyn StartupScreen>>> = const { std::cell::RefCell::new(None) };
}

/// Install the game's startup screen, once, before mods register.
// hidden: engine plumbing, re-exported through __private for the game.
#[doc(hidden)]
pub fn install_startup_screen(screen: Arc<dyn StartupScreen>) -> Result<(), StartupProgressError> {
    SCREEN
        .set(screen)
        .map_err(|_| StartupProgressError::AlreadyInstalled)
}

pub(crate) fn installed() -> Option<Arc<dyn StartupScreen>> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(screen) = TEST_SCREEN.with_borrow(Clone::clone) {
        return Some(screen);
    }
    SCREEN.get().cloned()
}

/// A mod's owned bar on the startup loading screen (NeoForge's `ProgressMeter`), from
/// [`RegistrationContext::progress`](crate::RegistrationContext::progress). The mod may keep it
/// past registration and step it or post messages from any thread, including `client_setup`.
/// Changes are drawn only while the loading screen is being drawn. Synchronous `client_setup`
/// work is not displayed live; updates become visible only after the hook returns and only if
/// a loading frame is drawn afterwards. Slow work may use a mod-owned worker, but that alone
/// does not keep the loading screen open. Dropping the bar removes it. With no game screen
/// (a test or a tool), every call does nothing.
// exhaustive: its field is private, so mods cannot build or destructure it
// no wire: the handle owns a native meter in the same process.
pub struct StartupProgress {
    meter: Option<Box<dyn ProgressMeter>>,
}

impl StartupProgress {
    /// Show a startup message through this retained handle, including during `client_setup`.
    /// Like bar updates, it appears only on a subsequent loading frame; a dedicated server logs it.
    pub fn message(&self, text: &str) {
        if let Some(meter) = &self.meter {
            meter.message(text);
        }
    }

    /// One more step is done.
    pub fn step(&self) {
        if let Some(meter) = &self.meter {
            meter.step();
        }
    }

    /// `done` of the bar's steps are done; more than its steps reads as all of them.
    pub fn set(&self, done: u32) {
        if let Some(meter) = &self.meter {
            meter.set(done);
        }
    }

    /// The bar reads `text` from now on.
    pub fn label(&self, text: &str) {
        if let Some(meter) = &self.meter {
            meter.label(text);
        }
    }
}

impl crate::RegistrationContext {
    /// Open an owned bar of `steps` steps labelled `label` on the startup loading screen,
    /// under the game's own stage bar, newest first (NeoForge's `addProgressBar`). The bar
    /// may be opened on a scoped worker, updated from any thread, and is removed when dropped.
    /// On a dedicated server,
    /// the label and progress in 10% increments are logged instead.
    /// Returns `TooManyBars` when the calling mod already has 64 open bars; no bar is created.
    pub fn progress(
        &self,
        label: &str,
        steps: u32,
    ) -> Result<StartupProgress, StartupProgressError> {
        let meter = match (self.listener, &self.startup) {
            (Some(id), Some(screen)) => Some(screen.clone().open(id, label, steps)?),
            _ => None,
        };
        Ok(StartupProgress { meter })
    }

    /// Show `text` on the startup loading screen for a few seconds (NeoForge's
    /// `addModMessage`); a dedicated server logs it.
    pub fn message(&self, text: &str) {
        if let Some(screen) = &self.startup {
            screen.message(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Record(Mutex<Vec<String>>);
    impl Record {
        fn push(&self, call: String) {
            self.0.lock().unwrap().push(call);
        }
    }
    impl StartupScreen for Record {
        fn open(
            self: Arc<Self>,
            mod_id: &'static str,
            label: &str,
            steps: u32,
        ) -> Result<Box<dyn ProgressMeter>, StartupProgressError> {
            self.push(format!("open {mod_id} {label} {steps}"));
            Ok(Box::new(RecordingMeter(self)))
        }
        fn message(&self, text: &str) {
            self.push(format!("screen message {text}"));
        }
    }

    struct RecordingMeter(Arc<Record>);
    impl ProgressMeter for RecordingMeter {
        fn step(&self) {
            self.0.push("step".into());
        }
        fn set(&self, done: u32) {
            self.0.push(format!("set {done}"));
        }
        fn label(&self, text: &str) {
            self.0.push(format!("label {text}"));
        }
        fn message(&self, text: &str) {
            self.0.push(format!("meter message {text}"));
        }
    }
    impl Drop for RecordingMeter {
        fn drop(&mut self) {
            self.0.push("close".into());
        }
    }

    #[test]
    fn registration_returns_a_meter_that_outlives_the_context() {
        let screen = Arc::new(Record::default());
        let mut context = crate::RegistrationContext::new().for_mod("test");
        context.startup = Some(screen.clone());
        context.message("starting");
        let bar = context.progress("Baking", 10).unwrap();
        drop(context);

        std::thread::scope(|scope| {
            scope.spawn(|| bar.step());
        });
        std::thread::scope(|scope| {
            scope.spawn(move || {
                bar.label("Baked");
                bar.set(10);
                bar.message("done");
                drop(bar);
            });
        });
        assert_eq!(
            *screen.0.lock().unwrap(),
            [
                "screen message starting",
                "open test Baking 10",
                "step",
                "label Baked",
                "set 10",
                "meter message done",
                "close",
            ]
        );

        // A context without a game screen lets the same mod code run without progress UI.
        let mut none = crate::RegistrationContext::new();
        none.startup = None;
        none.progress("idle", 1).unwrap().step();
        none.message("unheard");
    }
}
