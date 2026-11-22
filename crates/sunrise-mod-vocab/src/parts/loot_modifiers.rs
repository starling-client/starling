use super::*;

/// Registrar operations for the unsynced global loot modifier serializer registry.
pub trait GlobalLootModifiersRegistrar {
    /// Register one serializer in the calling mod's owned namespace.
    /// Reserved namespaces and duplicates refuse registration, naming both owners.
    fn add_global_loot_modifier_serializer(
        &mut self,
        name: &str,
        serializer: Arc<dyn crate::GlobalLootModifierSerializer>,
    );
}

impl dyn Registrar + '_ {
    /// Register a type in `neoforge:global_loot_modifier_serializers` once at
    /// startup. Its id is owned by the calling manifest; `neoforge:` is reserved
    /// for built-ins. Data files under `data/<namespace>/loot_modifiers/` enable
    /// prepared instances; a registered type with no file does not run.
    ///
    /// Reload decodes new instances without recompiling code. An in-progress roll keeps
    /// using the generation it started with.
    pub fn register_global_loot_modifier_serializer<T: crate::GlobalLootModifierSerializer>(
        &mut self,
        name: &str,
        serializer: T,
    ) {
        self.add_global_loot_modifier_serializer(name, Arc::new(serializer));
    }
}

#[derive(Clone)]
pub(crate) struct LootModifierSerializerInfo {
    pub(crate) id: Box<str>,
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) serializer: Arc<dyn crate::GlobalLootModifierSerializer>,
}

/// Registration declarations for the game to freeze into its static registry.
pub(crate) fn loot_modifier_serializer_list()
-> Vec<(String, Arc<dyn crate::GlobalLootModifierSerializer>)> {
    installed().map_or_else(Vec::new, |installed| {
        installed
            .loot_modifier_serializers
            .iter()
            .map(|info| {
                (
                    info.id.to_string(),
                    crate::loot_modifiers::owned_serializer(
                        info.owner,
                        info.owner_index,
                        info.serializer.clone(),
                    ),
                )
            })
            .collect()
    })
}
