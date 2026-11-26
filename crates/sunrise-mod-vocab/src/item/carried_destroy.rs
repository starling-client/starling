//! Refusals when a client mod destroys the creative player's carried stack.

/// Why [`MenuRef::destroy_carried`](crate::MenuRef::destroy_carried) destroyed nothing. Every
/// refusal leaves the cursor and the inventory as they were and sends nothing, whether or not
/// a stack was carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CarriedDestroyError {
    /// No client world is logged in, its resources are not loaded yet, or the host has no game
    /// backend.
    Unavailable,
    /// The handle is stale or does not name the local player's open menu.
    Stale,
    /// The local player is not in creative mode.
    NotCreative,
    /// The creative inventory screen is not visible, or a server container is active even
    /// though that screen is still visible. The server owns the menu's cursor; no vanilla
    /// packet clears it in place (`SetCreativeModeSlot` slot `-1` only drops a stack).
    ServerOwnedMenu,
}

impl std::fmt::Display for CarriedDestroyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "no client world is ready",
            Self::Stale => "the menu handle is stale or not the local player's open menu",
            Self::NotCreative => "the player is not in creative mode",
            Self::ServerOwnedMenu => "the open menu's cursor is held by the server",
        })
    }
}

impl std::error::Error for CarriedDestroyError {}

impl crate::NoBackend for CarriedDestroyError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
