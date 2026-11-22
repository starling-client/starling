use super::*;

impl Game<'_> {
    /// Gather attributed model loader callbacks at client startup, before resources load
    /// (NeoForge's `ModelEvent.RegisterLoaders`).
    pub(in crate::game) fn native_register_model_loaders(
        context: &mod_api::RegistrationContext,
        loaders: &mut Vec<Answered<ModelLoaderRegistration>>,
    ) {
        Self::fire_register_model_loaders(context, loaders);
    }
}
