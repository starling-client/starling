use super::*;

impl Game<'_> {
    /// Collect F3 text declarations and renderer toggles with their registering mods, once at
    /// client startup (NeoForge's `RegisterDebugEntriesEvent`).
    pub(in crate::game) fn native_register_debug_entries(
        context: &mod_api::RegistrationContext,
        entries: &mut Vec<Answered<DebugEntry>>,
    ) {
        Self::fire_register_debug_entries(context, entries);
    }
}
