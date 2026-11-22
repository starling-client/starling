//! `Game` as the `CustomSpawnerLevel` the custom spawners run on: each service forwards to the game's own method.
use super::{Game, creatures::SpawnSite};
use std::collections::HashSet;
use sunrise_client::mobs::{Species, raider, spawn_rules};
use sunrise_client_core::gamerules::RuleIndex;
use sunrise_gameplay::custom_spawner::{CustomSpawnerLevel, SpawnerParts, SpawnerPlayer};

impl CustomSpawnerLevel for Game<'_> {
    fn parts(&mut self) -> SpawnerParts<'_> {
        SpawnerParts {
            world: &mut self.world,
            mobs: &mut self.mobs,
            facts: self.res.facts(),
        }
    }
    fn overworld(&self) -> bool {
        self.dim == super::Dim::Overworld
    }
    fn rule(&self, rule: RuleIndex) -> bool {
        self.ui.rules.bool_at(rule)
    }
    fn players(&self) -> Vec<SpawnerPlayer> {
        self.level_players_iter()
            .map(|p| SpawnerPlayer {
                uuid: p.uuid,
                position: p.position,
                alive: p.alive,
                spectator: p.spectator,
            })
            .collect()
    }
    fn village_centers(&self) -> HashSet<(i32, i32, i32)> {
        Game::village_centers(self)
    }
    fn is_spawn_position_ok(&self, kind: &str, species: Species, pos: [i32; 3]) -> bool {
        spawn_rules::is_spawn_position_ok(
            kind,
            species,
            &self.mob_world_base(&[]),
            pos[0],
            pos[1],
            pos[2],
        )
    }
    fn spawn_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        reason: spawn_rules::Reason,
        yaw: Option<f32>,
    ) -> Option<u32> {
        let site = SpawnSite::Finalizing(reason);
        match yaw {
            Some(y) => self.spawn_mob_facing(kind, at, persistent, site, y),
            None => Game::spawn_mob(self, kind, at, persistent, site),
        }
    }
    fn spawn_event_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        leader: bool,
        reason: raider::Reason,
    ) -> Option<u32> {
        Game::spawn_event_mob(self, kind, at, leader, reason)
    }
    fn structure_tag(&self, tag: &str) -> Vec<String> {
        Game::structure_tag(self, tag)
    }
    fn structures_at(&self, pos: [i32; 3]) -> Vec<String> {
        Game::structures_at(self, pos)
    }
}
