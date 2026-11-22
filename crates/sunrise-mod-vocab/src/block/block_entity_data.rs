//! Errors when declaring a mod block entity's live menu data.

/// Why a mod block entity's menu data slots could not be sized.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockEntityDataError {
    /// The handle no longer identifies a loaded entity in the requested level.
    Stale,
    /// The entity is vanilla, not a configured mod holder, or not writable here.
    Unavailable,
}

impl crate::NoBackend for BlockEntityDataError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
