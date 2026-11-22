use super::*;

impl Game<'_> {
    /// Gather attributed block-state definition providers at client startup, before resources load
    /// (NeoForge's `RegisterBlockStateModels`).
    pub(in crate::game) fn native_register_block_state_models(
        context: &mod_api::RegistrationContext,
        definitions: &mut Vec<Answered<BlockStateDefinitionRegistration>>,
    ) {
        Self::fire_register_block_state_models(context, definitions);
    }
}
