//! Starling's part of options.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The catalog's shelf for a page's Catalog button.
#[cfg(target_os = "android")]
pub(super) fn catalog_shelf(
    shelf: sunrise_client::options_ui::CatalogShelf,
) -> starling_catalog::Shelf {
    use sunrise_client::options_ui::CatalogShelf;
    match shelf {
        CatalogShelf::Mods => starling_catalog::Shelf::Mods,
        CatalogShelf::ResourcePacks => starling_catalog::Shelf::ResourcePacks,
        CatalogShelf::ShaderPacks => starling_catalog::Shelf::ShaderPacks,
    }
}

/// Starling: the focus native mods' frame rate hook sees. A phone's game in the foreground is the
/// one being played even when a chat bubble or the keyboard holds Android's window focus, and
/// Dynamic FPS then held it at its unfocused rate; leaving the app is `minimized`. Outside the
/// hooked function: horizon-mixins refuses a target body with a `cfg`.
pub(super) fn mod_focus(focused: bool) -> bool {
    cfg!(target_os = "android") || focused
}

/// Starling: on a phone a menu over the world (pause, inventory, ...) draws at most 60 frames a
/// second. A paused game's menu draws its kept backdrop (`menu_blur_cache`) so cheaply that it
/// ran at 300+ FPS, warming the phone for nothing. A computer keeps its limit.
pub(super) fn menu_frame_cap(limit: i32, has_screen: bool) -> i32 {
    if cfg!(target_os = "android") && has_screen {
        limit.min(60)
    } else {
        limit
    }
}

impl Game<'_> {
    /// One of the game's folders (packs, shader packs, mods) in the file manager. On a phone the
    /// Files app shows it through Starling's own provider, wherever the game keeps it. A folder it
    /// can't show from the app's private files gets the offer of the Starling folder instead.
    pub(super) fn open_game_folder(&mut self, dir: &std::path::Path) {
        let _ = std::fs::create_dir_all(dir);
        #[cfg(target_os = "android")]
        if sunrise_client::platform::open_folder(dir) != Some(true)
            && sunrise_client::logging::game_home() == sunrise_client::logging::private_home()
        {
            self.ask_files_offer();
        }
        #[cfg(not(target_os = "android"))]
        super::super::packs::open_folder(dir);
    }
}
