use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// `IShearable.isShearable` as the entity's entity parts answer it, returning the
    /// answering part's registration slot alongside `shearable`. Both sides.
    pub(in crate::game) fn native_is_shearable(
        &mut self,
        entity: EntityRef,
        player: Option<PlayerRef>,
        shears: mod_api::ItemStackView<'_>,
        shearable: &mut Option<bool>,
        entity_kind: &str,
    ) -> Option<(usize, usize)> {
        mod_api::dispatch_is_shearable(
            crate::game::execution_role::host_side(self),
            entity,
            player,
            shears,
            shearable,
            entity_kind,
        )
    }
}
