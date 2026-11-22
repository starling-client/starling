use super::*;
use crate::game::cauldron_interactions::Registrar;
impl Game<'_> {
    /// Declare cauldron interactions before publishing the resource snapshot.
    pub(in crate::game) fn native_register_cauldron_interaction(
        context: &mod_api::RegistrationContext,
        interactions: &mut Registrar<'_>,
    ) {
        Self::fire_register_cauldron_interaction(context, |listen| {
            interactions.register_listener(listen)
        });
    }
}
