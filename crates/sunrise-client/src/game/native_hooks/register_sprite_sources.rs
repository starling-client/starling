use super::*;

impl Game<'_> {
    /// Gather attributed atlas sprite source providers at client startup, before resources load
    /// (NeoForge's `RegisterSpriteSourcesEvent`).
    pub(in crate::game) fn native_register_sprite_sources(
        context: &mod_api::RegistrationContext,
        sources: &mut Vec<Answered<SpriteSourceRegistration>>,
    ) {
        Self::fire_register_sprite_sources(context, sources);
    }
}
