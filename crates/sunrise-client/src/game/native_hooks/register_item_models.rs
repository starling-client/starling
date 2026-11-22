use super::*;

impl Game<'_> {
    /// Gather attributed item model node providers at client startup, before resources load
    /// (NeoForge's `RegisterItemModelsEvent`).
    pub(in crate::game) fn native_register_item_models(
        context: &mod_api::RegistrationContext,
        models: &mut Vec<Answered<ItemModelRegistration>>,
    ) {
        Self::fire_register_item_models(context, models);
    }
}
