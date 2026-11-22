use super::*;
use crate::game::client_item_extensions::Registration;
impl Game<'_> {
    /// Registers this resource load's item-owned client equipment extensions.
    pub(in crate::game) fn native_register_client_item_extensions(
        context: &mod_api::RegistrationContext,
        extensions: &mut Registration<'_>,
    ) {
        Self::fire_register_client_item_extensions(context, |listen| {
            extensions.register_listener(listen)
        });
    }
}
