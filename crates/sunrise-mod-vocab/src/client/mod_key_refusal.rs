//! Refusals from the registered client key mapping API.

/// Why resolving a mapping with [`mod_key_mapping`](crate::prelude::ClientInputClientHost::mod_key_mapping)
/// or using a [`crate::ModKeyMappingRef`] was refused.
/// A refused [`set_binding`](crate::ModKeyMappingRef::set_binding) or
/// [`reset_binding`](crate::ModKeyMappingRef::reset_binding) leaves the binding and
/// `sunrise-mods.txt` unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ModKeyRefusal {
    /// Resolution found no registered mapping for the given owner and id.
    NotRegistered,
    /// The key is not a key or mouse button name Controls knows (`key.keyboard.z`,
    /// `key.mouse.left`, `key.keyboard.unknown` for none).
    UnknownKey,
    /// The modifiers hold a bit other than shift (1), control (2) and alt (4).
    UnknownModifiers,
    /// No client key mapping backend is available in this host or phase.
    Unavailable,
    /// The handle no longer identifies a row in this client's registration roster.
    /// Resolve its owner and id again on the current client; a rebind or reset alone is not stale.
    Stale,
}

impl std::fmt::Display for ModKeyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotRegistered => "the mod registered no key mapping of that name",
            Self::UnknownKey => "the key is not a key or mouse button name",
            Self::UnknownModifiers => "modifiers are shift (1), control (2) and alt (4) only",
            Self::Unavailable => "client key bindings are unavailable",
            Self::Stale => "stale mod key mapping handle",
        })
    }
}

impl std::error::Error for ModKeyRefusal {}

impl crate::NoBackend for ModKeyRefusal {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
