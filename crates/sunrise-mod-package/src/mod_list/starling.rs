//! Starling's part of mod_list.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Whether a compiled native build (Starling's Compile) is the one for the native mods on now.
/// A build that says the SHA-256 of the request it was compiled from (`sent`) is current only for
/// that same request (`request`), so a mod replaced under the same id (a new voice chat of the
/// same id, with buttons the old one lacked) makes it out of date and Compile is offered again.
/// A build from before builds said it, or with no request to compare, goes by the mods' ids.
pub fn native_build_current(
    sent: Option<&str>,
    built: &BTreeSet<String>,
    wanted: &BTreeSet<String>,
    request: Option<&str>,
) -> bool {
    match (sent, request) {
        (Some(sent), Some(request)) => sent == request,
        _ => built == wanted,
    }
}

#[cfg(target_os = "android")]
pub const NOT_BUILT: &str =
    "Installed, not in this game yet: Compile builds the game with it on Starling's build server.";

/// [`NOT_BUILT`] in the player's language, as the Mods page's tooltip says it.
#[cfg(not(target_os = "android"))]
pub const NOT_BUILT_TIP: &str = "sunrise.mods.not_built_tip";

#[cfg(target_os = "android")]
pub const NOT_BUILT_TIP: &str = "starling.mods.native.not_built_tip";

/// Android: the game with native mods as the Mods page says it (game/native_builds.rs): the APK's
/// own or a build with some mods compiled in, then what the player can do about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeGame {
    /// Lines of text, each with whether it is a warning (red).
    pub lines: Vec<(String, bool)>,
    pub buttons: Vec<(String, NativeGameAct)>,
}
