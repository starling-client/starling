//! What a platform does for the game that std cannot: show a folder in its file manager, ask the
//! player for a file to import, the system clipboard, open a page in its browser, seal secrets
//! with its keystore. sunrise-android registers Android's (through the activity); elsewhere
//! nothing is registered and each caller keeps its desktop way.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub struct Hooks {
    /// Show the folder in the system's file manager; false when none could.
    pub open_folder: fn(&Path) -> bool,
    /// Ask the player for a file; it arrives in the import inbox (`import::inbox_dir`). False when
    /// no picker could open.
    pub pick_file: fn() -> bool,
    /// The system clipboard's text (None: empty or unreachable).
    pub clipboard_get: fn() -> Option<String>,
    /// Put text on the system clipboard; false when it could not.
    pub clipboard_set: fn(&str) -> bool,
    /// Open an https:// page in the system's browser; false when none could.
    pub open_link: fn(&str) -> bool,
    /// Seal bytes with a key the platform's keystore holds and never gives out (Android
    /// Keystore); None when it refused.
    pub seal: fn(&[u8]) -> Option<Vec<u8>>,
    /// Open what `seal` sealed on this device; None when it cannot (changed, or another key).
    pub open: fn(&[u8]) -> Option<Vec<u8>>,
    /// The folder the app's libraries are installed in, the one place the platform lets it run
    /// programs from (the Java launchers that prepare the game files); None when unknown.
    pub library_dir: fn() -> Option<PathBuf>,
    /// A file the app carries (the APK's `assets/`), whole; None when it has none of the name.
    pub asset: fn(&str) -> Option<Vec<u8>>,
    /// Keep the game running in the foreground while it waits for `reason` (`line` says so in
    /// the system's notification), or let that reason go: a phone may cut a background app's
    /// internet, or end it, while the player signs in in the browser.
    pub keep_working: fn(&str, &str, bool),
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();

/// Registers the platform's hooks, once, before the game starts.
pub fn set(hooks: Hooks) {
    sunrise_client_input::clipboard::set_platform((hooks.clipboard_get, hooks.clipboard_set));
    let _ = HOOKS.set(hooks);
}

/// Whether a platform registered its hooks (Android).
pub fn registered() -> bool {
    HOOKS.get().is_some()
}

/// The platform's file manager on `dir`: None without one registered (use the desktop's).
pub fn open_folder(dir: &Path) -> Option<bool> {
    HOOKS.get().map(|h| (h.open_folder)(dir))
}

/// The platform's file picker; false without one.
pub fn pick_file() -> bool {
    HOOKS.get().is_some_and(|h| (h.pick_file)())
}

/// The platform's clipboard: None without one registered, Some(None) when it holds no text.
pub fn clipboard_get() -> Option<Option<String>> {
    HOOKS.get().map(|h| (h.clipboard_get)())
}

/// The platform's clipboard: None without one registered.
pub fn clipboard_set(text: &str) -> Option<bool> {
    HOOKS.get().map(|h| (h.clipboard_set)(text))
}

/// Holds the game in the foreground for `reason` (or lets it go); nothing without a platform.
pub fn keep_working(reason: &str, line: &str, on: bool) {
    if let Some(h) = HOOKS.get() {
        (h.keep_working)(reason, line, on);
    }
}

/// The platform's browser on `url`: None without one registered (the desktop shows the link).
pub fn open_link(url: &str) -> Option<bool> {
    HOOKS.get().map(|h| (h.open_link)(url))
}

/// Whether secrets can be sealed here (a platform keystore is registered).
pub fn can_seal() -> bool {
    HOOKS.get().is_some()
}

/// `data` sealed by the platform's keystore: None without one, or when it refused.
pub fn seal(data: &[u8]) -> Option<Vec<u8>> {
    HOOKS.get().and_then(|h| (h.seal)(data))
}

/// What [`seal`] sealed on this device.
pub fn open(sealed: &[u8]) -> Option<Vec<u8>> {
    HOOKS.get().and_then(|h| (h.open)(sealed))
}

/// The folder the app's libraries are installed in: None without a platform registered.
pub fn library_dir() -> Option<PathBuf> {
    HOOKS.get().and_then(|h| (h.library_dir)())
}

/// A file the app carries: None without a platform registered, or without the file.
pub fn asset(name: &str) -> Option<Vec<u8>> {
    HOOKS.get().and_then(|h| (h.asset)(name))
}
