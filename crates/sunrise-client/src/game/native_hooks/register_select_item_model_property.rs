use super::*;

impl Game<'_> {
    /// Gather attributed categorical `minecraft:select` item model property providers at client
    /// startup, before resources load (NeoForge's `RegisterSelectItemModelPropertyEvent`).
    pub(in crate::game) fn native_register_select_item_model_property(
        context: &mod_api::RegistrationContext,
        properties: &mut Vec<Answered<SelectPropertyRegistration>>,
    ) {
        Self::fire_register_select_item_model_property(context, properties);
    }
}
