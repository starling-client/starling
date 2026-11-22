use super::*;

impl Game<'_> {
    /// Collect the mods' fluid models before the client bakes its initial or reloaded resources.
    pub(in crate::game) fn native_register_fluid_models(
        context: &mod_api::RegistrationContext,
        models: &mut Vec<mod_api::FluidModel>,
    ) {
        Self::fire_register_fluid_models(context, models);
    }
}
