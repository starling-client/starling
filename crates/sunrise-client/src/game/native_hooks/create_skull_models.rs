use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// Collect the mods' skull models once at client startup, after layer definitions freeze.
    /// API @API:create_skull_models@.
    pub(in crate::game) fn native_create_skull_models(
        context: &mod_api::RegistrationContext,
        models: &mut Vec<SkullModel>,
    ) {
        Self::fire_create_skull_models(context, models);
    }
}
