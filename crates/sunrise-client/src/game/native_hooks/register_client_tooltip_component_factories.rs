use super::*;

impl Game<'_> {
    /// Registers client tooltip component factories for custom `TooltipComponentData` types
    /// (NeoForge's `RegisterClientTooltipComponentFactoriesEvent`). Asked once at client startup.
    pub(in crate::game) fn native_register_client_tooltip_component_factories(
        context: &mod_api::RegistrationContext,
        factories: &mut sunrise_client::tooltip_components::TooltipComponentFactories,
    ) {
        Self::fire_register_client_tooltip_component_factories(context, |listen| {
            factories.register_listener(listen);
        });
    }
}
