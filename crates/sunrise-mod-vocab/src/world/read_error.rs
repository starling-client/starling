//! Refusals of entity, level, raid, server and registry reads.

/// Why an `EntityRef` read cannot return its requested state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: this refusal is returned only by in-process entity reads
pub enum EntityReadError {
    /// The handle does not name an entity in the current view.
    Stale,
    /// The requested state is not readable from this side, phase or view.
    Unavailable,
}

impl crate::NoBackend for EntityReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Why a `LevelRef` read cannot return its requested state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LevelReadError {
    /// The handle's level is gone or has been reopened.
    Stale,
    /// The state is not readable from this side, phase or view.
    Unavailable,
}

impl crate::NoBackend for LevelReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Why a raid's members or heroes could not be visited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: this refusal is returned only by in-process raid visitors
pub enum RaidReadError {
    /// The raid no longer exists in its level, or that level was closed or reopened.
    Stale,
    /// The raid data is not readable from this side, phase or view.
    Unavailable,
}

impl From<LevelReadError> for RaidReadError {
    fn from(error: LevelReadError) -> Self {
        match error {
            LevelReadError::Stale => Self::Stale,
            LevelReadError::Unavailable => Self::Unavailable,
        }
    }
}

impl crate::NoBackend for RaidReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Why a server-wide view could not be visited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: this refusal is returned only by in-process server visitors
pub enum ServerReadError {
    /// The view has no backend or the current phase cannot read it.
    Unavailable,
}

impl crate::NoBackend for ServerReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Why the current side's registry could not be visited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: this refusal is returned only to the in-process registry visitor
pub enum RegistryReadError {
    /// No registry has this key, or this side does not expose it.
    UnknownRegistry,
    /// There is no live registry backend or the phase does not allow world access.
    Unavailable,
}

impl crate::NoBackend for RegistryReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
