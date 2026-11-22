//! Fault injection available only to scoped engine tests.
use super::DatapackRegistrySet;
impl DatapackRegistrySet {
    /// A set whose `registry` holds exactly `entries`, for engine tests of reads against a
    /// world's loaded datapack registries.
    // hidden: fault injection for engine tests, gated by the testing feature.
    #[doc(hidden)]
    pub fn with_entries_for_testing(registry: &str, entries: &[&str]) -> Self {
        use crate::world::datapack_registry::DatapackEntry;
        Self {
            registries: std::collections::BTreeMap::from([(
                registry.to_owned(),
                entries
                    .iter()
                    .map(|entry| ((*entry).to_owned(), DatapackEntry { values: Vec::new() }))
                    .collect(),
            )]),
            tags: std::collections::BTreeMap::new(),
            received_registries: None,
        }
    }
}
