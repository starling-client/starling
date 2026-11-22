use super::*;

impl Game<'_> {
    /// The custom animation targets and interpolations a mod adds (NeoForge's
    /// `RegisterJsonAnimationTypesEvent`), bound to their callbacks with the registering mod.
    /// Asked once at start on the client, before resources load.
    pub(in crate::game) fn native_register_json_animation_types(
        context: &mod_api::RegistrationContext,
        types: &mut Vec<Answered<JsonAnimationTypeRegistration>>,
    ) {
        Self::fire_register_json_animation_types(context, types);
    }
}
