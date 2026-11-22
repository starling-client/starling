use crate::game::{Game, api_values::with_live_level_random};
use mod_api::{LevelRef, RandomDraw};
use sunrise_core::random::LegacyRandom;

impl Game<'_> {
    /// Lend the named level's live stream directly.
    pub(in crate::game) fn with_level_random(&mut self, level: LevelRef, draw: RandomDraw<'_>) {
        let Some(random) = self.level_random_mut(level) else {
            return;
        };
        with_live_level_random(random, draw);
    }

    /// Select a current, resident server level's stream without bringing the level into place.
    pub(in crate::game) fn level_random_mut(
        &mut self,
        level: LevelRef,
    ) -> Option<&mut LegacyRandom> {
        if !self.is_logical_server() || !self.level_handle_current(level) {
            return None;
        }
        let world = if level.dimension() == self.dim.key() {
            &mut self.world
        } else {
            self.levels.parked_world_mut_of(level.dimension_id())?
        };
        Some(&mut world.random)
    }
}
