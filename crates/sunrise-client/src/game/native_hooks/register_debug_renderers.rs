use super::*;

impl Game<'_> {
    /// Collect world-space debug renderers with their registering mods at each client
    /// renderer-list refresh (NeoForge's `RegisterDebugRenderersEvent`).
    pub(in crate::game) fn native_register_debug_renderers(
        context: &mod_api::RegistrationContext,
        entries: &dyn mod_api::__private::DebugEntryStates,
        renderers: &mut Vec<Answered<DebugRenderer>>,
    ) {
        Self::fire_register_debug_renderers(context, entries, renderers);
    }
}
