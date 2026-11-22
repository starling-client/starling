//! `WanderingTraderSpawner`: every 1200 ticks the spawn delay counts down;
//! when it runs out a trader may appear near the player (or the nearest
//! bell), bringing two llamas on leads, and wanders off after two days.
use super::{Dim, Game, creatures::Creatures};
use sunrise_client::save::{read_dat, write_dat};
use sunrise_gameplay::wandering_trader::{TraderLevel, roll};

pub(super) use sunrise_gameplay::wandering_trader::Spawner;

impl Creatures {
    /// A fresh spawner for a newly opened world.
    pub(super) fn reset_trader(&mut self) {
        self.trader = Spawner::default();
    }
}
impl Game<'_> {
    fn trader_path(&self) -> std::path::PathBuf {
        self.world_root
            .join("data")
            .join("minecraft")
            .join("wandering_trader.dat")
    }
    fn load_trader_data(&mut self) {
        if self.creatures.trader.is_loaded() {
            return;
        }
        let tag = read_dat(&self.trader_path());
        self.creatures
            .trader
            .load(tag.as_ref().and_then(|t| t.get("data")));
    }
    pub(super) fn save_wandering_trader(&mut self) {
        let Some(tag) = self.creatures.trader.to_save() else {
            return;
        };
        if write_dat(&self.trader_path(), tag).is_ok() {
            self.creatures.trader.saved();
        }
    }
    /// `WanderingTraderSpawner.tick`.
    pub(super) fn tick_wandering_trader(&mut self) {
        if self.dim != Dim::Overworld
            || !self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::SPAWN_WANDERING_TRADERS)
        {
            return;
        }
        self.load_trader_data();
        let Some(chance) = self.creatures.trader.due() else {
            return;
        };
        if roll(self, chance) {
            self.creatures.trader.spawned();
        }
    }
}
impl TraderLevel for Game<'_> {
    fn motion_top(&self, x: i32, z: i32) -> i32 {
        Game::motion_top(self, x, z)
    }
    fn chunk_biome_has(&self, tag: &str, pos: [i32; 3]) -> bool {
        self.world
            .chunks
            .get(&(pos[0] >> 4, pos[2] >> 4))
            .map(|c| {
                self.res
                    .biomes()
                    .get(c.biome(pos[0], pos[1], pos[2]))
                    .name
                    .clone()
            })
            .is_some_and(|b| self.res.mob_data().biome_has(tag, &b))
    }
}
