use super::*;

impl Game<'_> {
    /// Collect the mods' pipeline modifier registrations once at startup.
    pub(in crate::game) fn native_register_pipeline_modifiers(
        context: &mod_api::RegistrationContext,
        modifiers: &mut Vec<mod_api::PipelineModifier>,
    ) {
        Self::fire_register_pipeline_modifiers(context, modifiers);
    }
}
