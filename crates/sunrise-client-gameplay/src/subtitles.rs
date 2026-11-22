//! `Hud.extractRenderState`'s subtitle rule.

/// Where `Hud.extractRenderState` draws the subtitles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subtitles {
    /// Hidden with the rest of the HUD: F1 with no screen over the game.
    Hidden,
    /// In the HUD's own pass, under any screen that draws later.
    Now,
    /// An in-game screen draws them from `Screen.extractBackground`, after its dimming
    /// and before its widgets. Without a screen, `Gui.extractRenderState` draws them last.
    Deferred,
}

/// `Hud.extractRenderState`'s subtitle rule. `screen` is the open screen's `isInGameUi`
/// answer, or `None` when no screen is open.
pub fn subtitles(show_hud: bool, screen: Option<bool>) -> Subtitles {
    match screen {
        Some(true) => Subtitles::Deferred,
        _ if !show_hud => Subtitles::Hidden,
        Some(false) => Subtitles::Now,
        None => Subtitles::Deferred,
    }
}
