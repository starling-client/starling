use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `LivingVisibilityEvent` (`LivingEntity.getVisibilityPercent`): how
    /// visible `entity` is to `looking`, which scales the range a targeting
    /// check sees it from. `visibility` comes in as vanilla's percent.
    pub(in crate::game) fn native_living_visibility(
        &self,
        entity: LivingRef,
        looking: Option<EntityRef>,
        visibility: &mut f64,
    ) {
        self.fire_living_visibility(entity, looking, visibility);
    }
}
