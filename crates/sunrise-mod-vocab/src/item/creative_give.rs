//! Refusals when a client mod gives the creative player a stack.

/// Why [`PlayerRef::give_creative`](crate::PlayerRef::give_creative) gave nothing. Every
/// refusal leaves the inventory as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CreativeGiveError {
    /// No client world is open, its resources are not loaded yet, or the host has no game
    /// backend.
    Unavailable,
    /// The handle is stale or does not name the local player.
    Stale,
    /// The local player is not in creative mode.
    NotCreative,
    /// The game's item boundary refused the stack: an unknown item, a count above its
    /// effective stack limit, or invalid components.
    Stack(crate::StackWriteError),
    /// The connected server's enabled feature flags do not enable this item.
    FeatureDisabled,
}

impl std::fmt::Display for CreativeGiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => f.write_str("no client world is open"),
            Self::Stale => f.write_str("the player handle is stale or not the local player"),
            Self::NotCreative => f.write_str("the player is not in creative mode"),
            Self::Stack(error) => write!(f, "invalid stack: {error:?}"),
            Self::FeatureDisabled => f.write_str("the item's required feature is disabled"),
        }
    }
}

impl std::error::Error for CreativeGiveError {}

impl From<crate::StackWriteError> for CreativeGiveError {
    fn from(error: crate::StackWriteError) -> Self {
        Self::Stack(error)
    }
}

impl crate::NoBackend for CreativeGiveError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
