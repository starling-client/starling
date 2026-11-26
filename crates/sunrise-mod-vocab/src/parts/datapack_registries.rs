use super::*;
use crate::world::datapack_registry::{
    DatapackField, DatapackRegistrationError, DatapackRegistryDeclaration, DatapackRegistrySync,
};

/// The datapack registries mods declared.
pub fn datapack_registry_declarations() -> &'static [DatapackRegistryDeclaration] {
    // A fixture's installed mods own its declarations. Process-wide declarations are
    // only the fallback for connection threads without an inherited fixture snapshot.
    #[cfg(any(test, feature = "testing"))]
    if let Some(installed) = crate::parts::TEST_PARTS.get() {
        return &installed.datapack_registries;
    }
    #[cfg(any(test, feature = "testing"))]
    if let Some(declarations) = PROCESS_DECLARATIONS.get() {
        return declarations;
    }
    installed().map_or(&[], |installed| &installed.datapack_registries)
}

/// Declarations a test process shares with every thread. A test's own installation is per
/// thread, but the connection a server answers a client on runs on threads the test did not
/// start. A thread's scoped installation takes precedence over this fallback.
#[cfg(any(test, feature = "testing"))]
static PROCESS_DECLARATIONS: OnceLock<Vec<DatapackRegistryDeclaration>> = OnceLock::new();

/// Declare `declarations` for every thread of this test process; the first call wins.
#[cfg(any(test, feature = "testing"))]
pub fn install_process_datapack_registry_declarations(
    declarations: Vec<DatapackRegistryDeclaration>,
) {
    let _ = PROCESS_DECLARATIONS.set(declarations);
}

/// Registration of mod-owned datapack registries
/// ([`Registrar::declare_datapack_registry`](DatapackRegistriesRegistrar::declare_datapack_registry)).
pub trait DatapackRegistriesRegistrar {
    /// Declare a datapack registry (NeoForge's `NewDatapackRegistryEvent`): a bare `path`
    /// uses the declaring mod's namespace (`spell` of `example` is `example:spell`),
    /// while a valid explicit `namespace:path` keeps its namespace. The record `fields` an
    /// entry has (scalars, and lists, maps, records and optional fields of them, nested as
    /// deep as the schema says), and how clients receive the entries.
    ///
    /// Entries are JSON files in data packs: entry `addon:fire` of
    /// `example:spell` is `data/addon/example/spell/fire.json`
    /// ([`DatapackRegistryDeclaration::entry_file`]). The game selects the winning
    /// resource before testing its load conditions against the candidate's registry
    /// and tag view. An omitted winner does not expose a lower pack's file. The game
    /// decodes accepted entries against `fields`: an undeclared key, missing required
    /// key or out-of-range value fails the entry. A default never stands in for
    /// invalid data. Transport limits do not apply to local files or candidates.
    ///
    /// Client-only packages cannot declare datapack registries. Server-only
    /// packages may declare [`DatapackRegistrySync::ServerOnly`] and
    /// [`DatapackRegistrySync::Optional`] registries without requiring clients to
    /// install the mod, as long as the package's other content does not require it.
    /// A [`DatapackRegistrySync::Required`] declaration needs a both-side package;
    /// the game refuses the server-only package at load, naming the registry.
    ///
    /// A both-side declarer gets `WrongSide` at registration for a required registry
    /// in a namespace supplied by a server-only package it serves.
    /// Refused, naming the mod: a built-in registry key, a malformed key,
    /// a duplicate (of this mod's or
    /// another's, caught where the declarations join), an empty or repeated
    /// field name, unusable bounds, an optional kind inside a list, map or optional, or a
    /// default that is not a value of its field inside its bounds or whose map names one key
    /// twice, at any depth. Id and reference defaults are normalised at every depth.
    fn declare_datapack_registry(
        &mut self,
        path: &str,
        sync: DatapackRegistrySync,
        fields: Vec<DatapackField>,
    ) -> Result<(), DatapackRegistrationError>;
}
