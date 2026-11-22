use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl Game<'_> {
    /// The selected language's text for a translation key (`Component.translatable`'s lookup), `None`
    /// when no language layer defines it.
    pub(in crate::game) fn translate(&self, key: &str) -> Option<String> {
        sunrise_client::lang::has(key).then(|| sunrise_client::lang::get(key))
    }
}
