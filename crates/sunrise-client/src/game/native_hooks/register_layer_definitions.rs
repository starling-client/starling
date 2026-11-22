use super::*;

impl Game<'_> {
    /// Gather attributed layer suppliers at client startup, before the layer table is frozen.
    /// API @API:model_layers@.
    pub(in crate::game) fn native_register_layer_definitions(
        context: &mod_api::RegistrationContext,
        layers: &mut Vec<Answered<LayerDefinitionRegistration>>,
    ) {
        Self::fire_register_layer_definitions(context, layers);
    }
}
