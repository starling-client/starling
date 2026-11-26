//! Starling's parts above Sunrise's engine. Each is registered through one of Sunrise's platform
//! registries (`sunrise_platform`, docs/layers.md "Platform registries"), as sunrise-official
//! registers Sunrise's own parts, so that no engine file carries them and a sync with Sunrise
//! meets none of them. Starling's Android entry (crates/sunrise-client/src/android_lib.rs
//! `register_parts`) calls [`register`] before the game starts.
//!
//! For now this is Starling's text: the keys of Starling's screens and options, and Starling's
//! translations of them, in `assets/lang/<code>.json`. Sunrise's own lang files stay as Sunrise
//! has them. A key Sunrise's files have as well would lose to the engine's text (and the lang
//! tests refuse it), so every key here is Starling's alone.
use sunrise_platform::lang;

/// Starling's lang files, one per language, `en_us` the reference.
pub static LANG: &lang::Files = include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Registers Starling's parts with the game. Before the game starts: a later call is refused.
pub fn register() -> [Result<(), sunrise_platform::hooks::TooLate>; 1] {
    [lang::register(LANG)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_is_registered_and_english_is_the_reference() {
        assert!(LANG.len() > 50, "only {} lang files", LANG.len());
        let english = LANG.iter().find(|(code, _)| *code == "en_us").expect("en_us.json");
        assert!(english.1.contains("\"starling.credits.sunrise\""));
        for (code, json) in LANG {
            assert!(json.trim_start().starts_with('{'), "{code}.json is no JSON object");
        }
    }
}
