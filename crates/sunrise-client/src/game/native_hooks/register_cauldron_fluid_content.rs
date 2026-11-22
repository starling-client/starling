use super::*;
impl Game<'_> {
    /// The fluid contents mods give their cauldron blocks, once the block registry is resolved.
    pub(in crate::game) fn native_register_cauldron_fluid_content(
        context: &mod_api::RegistrationContext,
        contents: &mut mod_api::CauldronFluidContentRegistry,
    ) {
        Self::fire_register_cauldron_fluid_content(context, |listen| {
            contents.register_listener(listen)
        });
    }
}
