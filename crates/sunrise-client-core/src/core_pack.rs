//! The prepared resources' core data pack: the one pack Sunrise names in `KnownPacks`
//! (`KnownPacksManager.trySelectingPacks`), so a server leaves that pack's entries' data out.

/// Vanilla's namespace and id, versioned by the game version the prepared resources
/// were built for.
pub const CORE_PACK: (&str, &str, &str) = ("minecraft", "core", sunrise_core::VANILLA_VERSION);
