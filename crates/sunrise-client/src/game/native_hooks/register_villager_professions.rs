use super::*;
impl Game<'_> {
    pub(in crate::game) fn native_register_villager_professions(
        context: &mod_api::RegistrationContext,
        professions: &mut mod_api::ProfessionDeclarations,
    ) {
        Self::fire_register_villager_professions(context, |listen| {
            professions.register_listener(listen)
        });
    }
}
