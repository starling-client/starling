// exports: none (the registrar is generated; declarations are engine-only).
use super::*;

/// Registration operations for the two unsynced loot map-codec registries.
pub trait LootTypesRegistrar {
    /// Register one function codec in the calling manifest's namespace.
    /// Duplicate ids and minecraft/neoforge namespaces refuse mod loading by name.
    fn add_loot_function_type(
        &mut self,
        id: &crate::Identifier,
        codec: Arc<dyn crate::ModLootFunction>,
    );
    /// Register one condition codec in the calling manifest's namespace.
    fn add_loot_condition_type(
        &mut self,
        id: &crate::Identifier,
        codec: Arc<dyn crate::ModLootCondition>,
    );
}
impl dyn Registrar + '_ {
    /// Register a map codec in `Registries.LOOT_FUNCTION_TYPE` before data loads.
    /// Replaces the broadcast loot-function hook.
    pub fn register_loot_function_type(
        &mut self,
        id: &crate::Identifier,
        codec: impl crate::ModLootFunction,
    ) {
        self.add_loot_function_type(id, Arc::new(codec));
    }
    /// Register a map codec in `Registries.LOOT_CONDITION_TYPE` before data loads.
    /// Validation errors refuse the candidate reload; previous loot data stays active.
    pub fn register_loot_condition_type(
        &mut self,
        id: &crate::Identifier,
        codec: impl crate::ModLootCondition,
    ) {
        self.add_loot_condition_type(id, Arc::new(codec));
    }
}

#[derive(Clone)]
pub(crate) struct LootFunctionTypeInfo {
    pub(crate) id: String,
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) codec: Arc<dyn crate::ModLootFunction>,
}
#[derive(Clone)]
pub(crate) struct LootConditionTypeInfo {
    pub(crate) id: String,
    pub(crate) owner: &'static str,
    pub(crate) owner_index: usize,
    pub(crate) codec: Arc<dyn crate::ModLootCondition>,
}

/// Frozen declarations, with callbacks executed in the declaring mod's scope.
// hidden: game integration captures these once, never scans mods while rolling.
#[doc(hidden)]
pub fn loot_type_declarations() -> (
    Vec<(String, Arc<dyn crate::ModLootFunction>)>,
    Vec<(String, Arc<dyn crate::ModLootCondition>)>,
) {
    installed().map_or_else(Default::default, |installed| {
        (
            installed
                .loot_function_types
                .iter()
                .map(|info| {
                    (
                        info.id.clone(),
                        crate::loot_types::owned::function(
                            info.owner,
                            info.owner_index,
                            info.codec.clone(),
                        ),
                    )
                })
                .collect(),
            installed
                .loot_condition_types
                .iter()
                .map(|info| {
                    (
                        info.id.clone(),
                        crate::loot_types::owned::condition(
                            info.owner,
                            info.owner_index,
                            info.codec.clone(),
                        ),
                    )
                })
                .collect(),
        )
    })
}
