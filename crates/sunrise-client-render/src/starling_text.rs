//! Starling's text in this crate's tests. The game's entry installs every crate's lang files
//! (`sunrise_hooks::lang`, crates/sunrise-client/src/entry.rs) before a language loads, Starling's
//! (crates/starling-platform) among them; a test that reads Sunrise's screens with Starling's rows
//! on them installs Starling's files the same way first.
use std::sync::OnceLock;

/// Starling's lang files, installed as the game installs them. Only the first install counts, so
/// every test of the process reads the same tables.
pub(crate) fn install() {
    static FILES: OnceLock<[&'static [(&'static str, &'static str)]; 1]> = OnceLock::new();
    crate::lang::install_crate_files(FILES.get_or_init(|| [starling_platform::LANG]));
}
