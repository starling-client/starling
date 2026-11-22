use super::*;

impl Game<'_> {
    /// Items' default components are bound on this side (NeoForge's
    /// `DefaultDataComponentsBoundEvent`), right before `native_tags_updated`.
    pub(in crate::game) fn native_default_components_bound(
        &mut self,
        cause: mod_api::TagsUpdateCause,
    ) {
        Self::fire_default_components_bound(self, cause);
    }
}
