//! `Minecraft.createTitle` and `updateTitle`: the window's title, as vanilla's reads.
use super::{Game, Screen};

/// `Minecraft* 26.3`: `ModCheck` reports a client that isn't Mojang's signed jar as modified.
pub(in crate::game) fn base() -> String {
    format!("Minecraft* {}", sunrise_core::VANILLA_VERSION)
}

/// The full title for a session part (`title.singleplayer`...), none while not connected.
fn title(session: Option<&str>) -> String {
    match session {
        Some(key) => format!("{} - {}", base(), sunrise_client::lang::get(key)),
        None => base(),
    }
}

impl Game<'_> {
    /// The session part `createTitle` adds while `getConnection` has a player and its connection is open.
    fn title_session(&self) -> Option<&'static str> {
        let live = self.remote.as_ref().is_some_and(|r| r.session.connected());
        if !live || self.player.is_none() {
            return None;
        }
        Some(if self.integrated.is_some() && !self.is_published() {
            "title.singleplayer"
        } else if self.realm_session {
            "title.multiplayer.realms"
        } else if self.integrated.is_none() && !self.lan_session {
            "title.multiplayer.other"
        } else {
            "title.multiplayer.lan"
        })
    }

    /// `updateTitle`: on a session change (level, publishing), and a new language once back to play.
    pub(in crate::game) fn update_window_title(&mut self) {
        let language = match self.window_title {
            Some((_, shown)) if self.screen != Screen::Playing => shown,
            _ => sunrise_client::lang::generation(),
        };
        let shown = (self.title_session(), language);
        if self.window_title == Some(shown) {
            return;
        }
        if let Some(window) = &self.window {
            window.set_title(&title(shown.0));
            self.window_title = Some(shown);
        }
    }
}
