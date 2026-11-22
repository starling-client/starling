//! `CatSpawner`: every 1200 ticks a cat may appear near a random player, in a village with enough occupied beds or inside a swamp hut.
use super::{Dim, Game};

pub(super) use sunrise_gameplay::cat_spawner::CatSpawner;

impl Game<'_> {
    /// `CatSpawner.tick` (the overworld's custom spawner).
    pub(super) fn tick_cat_spawner(&mut self) {
        if self.dim != Dim::Overworld || !self.creatures.cat.due() {
            return;
        }
        sunrise_gameplay::cat_spawner::spawn_near_player(self);
    }
    /// `spawnCat`: forwards to the shared flow; only the tests call it.
    #[cfg(test)]
    fn spawn_cat(&mut self, pos: [i32; 3], persistent: bool) {
        sunrise_gameplay::cat_spawner::spawn_cat(self, pos, persistent);
    }
}

#[cfg(test)]
#[path = "tests/cat_spawner_tests.rs"]
mod cat_spawner_tests;
