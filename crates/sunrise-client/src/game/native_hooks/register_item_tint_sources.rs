use super::*;

impl Game<'_> {
    /// Gather attributed item tint source providers at client startup, before resources load
    /// (NeoForge's `RegisterColorHandlersEvent.ItemTintSources`).
    pub(in crate::game) fn native_register_item_tint_sources(
        context: &mod_api::RegistrationContext,
        sources: &mut Vec<Answered<ItemTintSourceRegistration>>,
    ) {
        Self::fire_register_item_tint_sources(context, sources);
    }
}
