//! Registered biome identifiers (`BiomeKey`, vanilla `ResourceKey<Biome>`) and scoped,
//! generation-bound biome views (`BiomeView`, vanilla `Holder<Biome>`).
use crate::{Host, Identifier};
use std::{
    collections::BTreeMap,
    fmt,
    hash::{Hash, Hasher},
};

/// A registered biome identifier in `minecraft:worldgen/biome`, matching vanilla's
/// `ResourceKey<Biome>`.
///
/// Unlike [`BiomeView`], a `BiomeKey` carries no world-data generation token: it names a biome
/// persistently across reloads, worlds, worker threads, and wire boundaries. Resolve it against
/// an installed world snapshot with [`BiomeKey::view`] or [`Host::biome`](crate::Host::biome)
/// when generation-bound membership or data-map reads are needed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, sunrise_mod_api_macros::Wire)]
// exhaustive: private identifier wrapper; construct with `BiomeKey::new`, `BiomeKey::parse`, or from a `BiomeView`.
pub struct BiomeKey(Identifier);

impl BiomeKey {
    /// Wrap a validated resource identifier as a biome key.
    pub fn new(id: Identifier) -> Self {
        Self(id)
    }

    /// Parse a biome identifier (`minecraft:plains`, or bare `plains` for the `minecraft`
    /// namespace). Returns [`BiomeKeyError::InvalidIdentifier`] for an invalid resource id.
    pub fn parse(id: &str) -> Result<Self, BiomeKeyError> {
        Identifier::parse(id)
            .map(Self)
            .map_err(|_| BiomeKeyError::InvalidIdentifier)
    }

    /// Construct a biome key from a validated `namespace` and `path`.
    pub fn from_parts(namespace: &str, path: &str) -> Result<Self, BiomeKeyError> {
        Identifier::from_parts(namespace, path)
            .map(Self)
            .map_err(|_| BiomeKeyError::InvalidIdentifier)
    }

    /// The underlying validated resource identifier.
    pub fn identifier(&self) -> &Identifier {
        &self.0
    }

    /// The biome identifier's namespace (`minecraft`, `mymod`, ...).
    pub fn namespace(&self) -> &str {
        self.0.namespace()
    }

    /// The biome identifier's path (`plains`, `deep_dark`, ...).
    pub fn path(&self) -> &str {
        self.0.path()
    }

    /// Unwrap the underlying validated resource identifier.
    pub fn into_identifier(self) -> Identifier {
        self.0
    }

    /// Resolve this biome key in the host's installed world-data generation as a scoped
    /// [`BiomeView`].
    ///
    /// Returns [`BiomeKeyError::Unavailable`] without a server world biome registry (or in a
    /// load phase that forbids world-data lookups) and [`BiomeKeyError::UnknownBiome`] if this
    /// biome is not present in the installed generation.
    pub fn view<'a>(&self, host: &'a dyn Host) -> Result<BiomeView<'a>, BiomeKeyError> {
        crate::backend_of(host)
            .ok_or(BiomeKeyError::Unavailable)?
            .biome_view(self)
    }
}

impl Hash for BiomeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.namespace().hash(state);
        self.0.path().hash(state);
    }
}

impl fmt::Display for BiomeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<Identifier> for BiomeKey {
    fn as_ref(&self) -> &Identifier {
        &self.0
    }
}

impl From<Identifier> for BiomeKey {
    fn from(id: Identifier) -> Self {
        Self(id)
    }
}

impl From<BiomeKey> for Identifier {
    fn from(key: BiomeKey) -> Self {
        key.0
    }
}

impl TryFrom<&str> for BiomeKey {
    type Error = BiomeKeyError;

    fn try_from(id: &str) -> Result<Self, Self::Error> {
        Self::parse(id)
    }
}

impl TryFrom<String> for BiomeKey {
    type Error = BiomeKeyError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::parse(&id)
    }
}

/// A biome entry borrowed from one installed world-data generation, matching vanilla's
/// `Holder<Biome>`.
///
/// The borrow binds this view to the active snapshot: it cannot outlive the host borrow, cross
/// a reload, or cross a wire boundary. Keep [`BiomeView::key`] (a [`BiomeKey`]) to identify the
/// biome persistently and resolve it again later.
// no wire: scoped view borrowing from one installed world-data generation.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct BiomeView<'a> {
    key: &'a BiomeKey,
    id: &'a str,
    generation: u64,
}

impl<'a> BiomeView<'a> {
    /// The persistent registered biome key (`Holder.unwrapKey`).
    pub fn key(&self) -> &'a BiomeKey {
        self.key
    }

    /// This entry's canonical namespaced id (`minecraft:plains`), borrowed from the installed
    /// world-data snapshot.
    pub fn id(&self) -> &'a str {
        self.id
    }

    /// Whether this view resolves the given registered biome key (`Holder.is(ResourceKey)`).
    pub fn is(&self, key: &BiomeKey) -> bool {
        self.key == key
    }
}

impl PartialEq for BiomeView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation && self.id == other.id
    }
}

impl Eq for BiomeView<'_> {}

/// Why a biome key cannot be parsed or resolved against a host's world-data generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum BiomeKeyError {
    /// The string is not a valid resource identifier.
    InvalidIdentifier,
    /// The biome is not registered in this installed world-data generation.
    UnknownBiome,
    /// The scoped view belongs to another world or world-data generation.
    WrongGeneration,
    /// No server world biome registry is installed on this host, or the current phase forbids
    /// world-data lookups.
    Unavailable,
}

impl fmt::Display for BiomeKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentifier => "invalid biome identifier",
            Self::UnknownBiome => "biome is not registered in the installed world generation",
            Self::WrongGeneration => "biome view belongs to a different world generation",
            Self::Unavailable => "server world biome registry is unavailable",
        })
    }
}

impl std::error::Error for BiomeKeyError {}

impl crate::NoBackend for BiomeKeyError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// Parse a canonical, namespaced id at the biome registry's load or lookup boundary.
pub fn parse_registered_biome_key(name: &str) -> Result<BiomeKey, BiomeKeyError> {
    let (namespace, path) = name
        .split_once(':')
        .filter(|(namespace, _)| !namespace.is_empty())
        .ok_or(BiomeKeyError::InvalidIdentifier)?;
    BiomeKey::from_parts(namespace, path)
}

/// Engine-owned biome registry snapshot for one installed world-data generation.
// no wire: engine-owned snapshot storage, never passed to mods.
#[non_exhaustive]
#[derive(Default)]
pub struct BiomeKeys {
    generation: u64,
    entries: BTreeMap<BiomeKey, String>,
}

impl BiomeKeys {
    /// Prepare the engine's unique world-data generation from its registry before installation.
    /// The engine must supply a fresh generation for every candidate, across worlds and reloads.
    pub fn new(generation: u64, keys: impl IntoIterator<Item = BiomeKey>) -> Self {
        Self {
            generation,
            entries: keys
                .into_iter()
                .map(|key| {
                    let name = key.to_string();
                    (key, name)
                })
                .collect(),
        }
    }

    /// Resolve a canonical namespaced id (`minecraft:plains`) in this installed generation.
    pub fn view(&self, id: &str) -> Result<BiomeView<'_>, BiomeKeyError> {
        self.view_key(&parse_registered_biome_key(id)?)
    }

    /// Resolve a registered [`BiomeKey`] in this installed generation.
    pub fn view_key(&self, key: &BiomeKey) -> Result<BiomeView<'_>, BiomeKeyError> {
        let (stored, id) = self
            .entries
            .get_key_value(key)
            .ok_or(BiomeKeyError::UnknownBiome)?;
        Ok(BiomeView {
            key: stored,
            id: id.as_str(),
            generation: self.generation,
        })
    }

    /// Validate a scoped [`BiomeView`] against this installed generation and borrow its id.
    pub fn id(&self, view: BiomeView<'_>) -> Result<&str, BiomeKeyError> {
        if view.generation != self.generation {
            return Err(BiomeKeyError::WrongGeneration);
        }
        self.entries
            .get(view.key)
            .map(String::as_str)
            .ok_or(BiomeKeyError::WrongGeneration)
    }
}

#[cfg(test)]
#[path = "biome_key_tests.rs"]
mod tests;
