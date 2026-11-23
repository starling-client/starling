//! The mod's own crate. Its `[[native.mixin]]` helpers (`src/mixins/`) are compiled into copies of the
//! game functions they patch, not into this crate.
#[derive(Default)]
struct Mod;

#[mod_api::sunrise_mod]
impl mod_api::Hooks for Mod {}

/// Called by a helper: a helper may use its mod's crate.
pub fn note(what: &str) {
    eprintln!("fnr: alpha {what}");
}
