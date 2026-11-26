use super::*;

/// The hook points. Each does nothing in Sunrise.
impl Game<'_> {
    /// `BaseSpawner.isNearPlayer`: whether `spawner` in `dimension` is active this tick.
    /// `active` comes in as vanilla's answer, a player within its range.
    pub(in crate::game) fn native_spawner_active(
        &self,
        dimension: &str,
        spawner: mod_api::Spawner,
        active: &mut bool,
    ) {
        self.fire_spawner_active(dimension, spawner, active);
    }

    /// Vanilla's `isNearPlayer` answer `near` for `spawner`, as a mod's `spawner_active` leaves
    /// it: `near` itself when no mod listens, in which case `spawner` is never built.
    pub(in crate::game) fn spawner_active(
        &self,
        spawner: impl FnOnce() -> mod_api::Spawner,
        near: bool,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::SpawnerActive) {
            return near;
        }
        let mut active = near;
        self.native_spawner_active(self.dim.id(), spawner(), &mut active);
        active
    }
}
