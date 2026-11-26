//! Shared `uses.json` metadata available without the source scanner (`scan` feature).

/// The file a mod carries its uses table in, beside its `sunrise-mod.toml`.
pub const USES_FILE: &str = "uses.json";

/// The largest table `UsesTable::read` reads, in bytes.
pub const MAX_USES_BYTES: usize = 4 << 20;

/// What a mod's manifest says that the list depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifestFacts {
    /// Capability keys and hooks it names.
    pub names: Vec<String>,
}
