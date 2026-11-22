use super::*;
impl Game<'_> {
    pub(in crate::game) fn native_register_load_conditions(
        context: &mod_api::RegistrationContext,
        conditions: &mut mod_api::LoadConditionDeclarations,
    ) {
        Self::fire_register_load_conditions(context, |listen| conditions.register_listener(listen));
    }
}
