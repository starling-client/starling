// exports: none (the registrar is generated; declarations are engine-only).
use super::*;

/// Registration operations for the two unsynced loot map-codec registries.
pub trait LootTypesRegistrar {
    /// Register one function codec by its validated identifier.
    /// Built-in loot function types and duplicate ids refuse mod loading by name.
    fn add_loot_function_type(
        &mut self,
        id: &crate::Identifier,
        codec: Arc<dyn crate::ModLootFunctionCodec>,
    );
    /// Register one condition codec by its validated identifier.
    /// Built-in loot condition types and duplicate ids refuse mod loading by name.
    fn add_loot_condition_type(
        &mut self,
        id: &crate::Identifier,
        codec: Arc<dyn crate::ModLootConditionCodec>,
    );
}
impl dyn Registrar + '_ {
    /// Register a map codec in `Registries.LOOT_FUNCTION_TYPE` before data loads.
    /// Replaces the broadcast loot-function hook.
    pub fn register_loot_function_type(
        &mut self,
        id: &crate::Identifier,
        codec: impl crate::ModLootFunctionCodec,
    ) {
        self.add_loot_function_type(id, Arc::new(codec));
    }
    /// Register a map codec in `Registries.LOOT_CONDITION_TYPE` before data loads.
    /// Decode errors refuse the candidate reload; previous loot data stays active.
    pub fn register_loot_condition_type(
        &mut self,
        id: &crate::Identifier,
        codec: impl crate::ModLootConditionCodec,
    ) {
        self.add_loot_condition_type(id, Arc::new(codec));
    }
}

#[derive(Clone)]
pub(crate) struct LootFunctionTypeInfo {
    pub(crate) id: String,
    pub(crate) owner: &'static str,
    pub(crate) callback_owner: crate::__private::game::CallbackOwner,
    pub(crate) codec: Arc<dyn crate::ModLootFunctionCodec>,
}
#[derive(Clone)]
pub(crate) struct LootConditionTypeInfo {
    pub(crate) id: String,
    pub(crate) owner: &'static str,
    pub(crate) callback_owner: crate::__private::game::CallbackOwner,
    pub(crate) codec: Arc<dyn crate::ModLootConditionCodec>,
}

/// Frozen declarations, with callbacks executed in each registration's shared `CallbackOwner`.
// hidden: game integration captures these once, never scans mods while rolling.
#[doc(hidden)]
pub fn loot_type_declarations() -> (
    Vec<(String, Arc<dyn crate::ModLootFunctionCodec>)>,
    Vec<(String, Arc<dyn crate::ModLootConditionCodec>)>,
) {
    installed().map_or_else(Default::default, |installed| {
        (
            installed
                .loot_function_types
                .iter()
                .map(|info| {
                    (
                        info.id.clone(),
                        crate::loot_types::owned::function_codec(
                            info.callback_owner.clone(),
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
                        crate::loot_types::owned::condition_codec(
                            info.callback_owner.clone(),
                            info.codec.clone(),
                        ),
                    )
                })
                .collect(),
        )
    })
}
