//! A mob's melee strike for native server mods, through a `MobRef`: `Mob.doHurtTarget` on a
//! target the mod chooses, the strike a melee goal ends with.
use super::native_server::Acting;
use super::{Game, commands::Target};
use mod_api::{AttackRefused, EntityRef, MobRef};
use sunrise_client::mobs::{self, MobEvent, Who, goals::Cx};
use sunrise_gameplay::lan::mob_players::acts_on_player;

impl Game<'_> {
    /// `Mob.doHurtTarget(level, target)` for `mob` on a player or a mob of its level, with each
    /// species' override and no swing: vanilla's `wasHurt`.
    pub(in crate::game) fn mob_do_hurt_target(
        &mut self,
        mob: MobRef,
        target: EntityRef,
    ) -> Result<bool, AttackRefused> {
        let epoch = self.native_lifecycle.epoch();
        let entity = mob.entity();
        if !self.is_logical_server()
            || epoch != Some(entity.epoch())
            || epoch != Some(target.epoch())
        {
            return Err(AttackRefused::Stale);
        }
        if !self.lan_guest_swaps.is_empty()
            || !(self.transactions.is_empty() || self.in_level_tick_boundary_alone())
            || self.fake_action.acting()
            || self.held_out.contains(&entity.id())
        {
            return Err(AttackRefused::Busy);
        }
        let id = match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => id,
            None if self.mob_in_other_level(entity.id()).is_some() => {
                return Err(AttackRefused::OtherLevel);
            }
            // The dragon, which no melee goal strikes with.
            _ => return Err(AttackRefused::Stale),
        };
        let conn = match self.native_entity_target(target) {
            Some(Target::Mob(other)) if other == id => return Err(AttackRefused::Unsupported),
            Some(Target::Mob(other)) => {
                // As a fake player's `perform_action`, beside a level tick listener's
                // transaction.
                return self.beside_listener_journal(|game| {
                    let _acting = Acting::start(&game.fake_action);
                    let (events, answer) = game
                        .mob_strike(id, Who::Mob(other))
                        .ok_or(AttackRefused::Stale)?;
                    game.combat.hit_landed = false;
                    let _ = game.handle_mob_events(events);
                    Ok(answer.unwrap_or(game.combat.hit_landed))
                });
            }
            Some(Target::Guest(conn)) => conn,
            // The player whose turn runs now.
            Some(Target::Player) => return Err(AttackRefused::Busy),
            Some(_) => return Err(AttackRefused::Unsupported),
            None if self.entity_level(target).is_some() => return Err(AttackRefused::OtherLevel),
            None => return Err(AttackRefused::Stale),
        };
        // Players are server-wide records: the guest must stand in the mob's level.
        let in_place = self
            .guest_index(conn)
            .is_some_and(|gi| self.guest_in_place(gi));
        if !in_place {
            return Err(AttackRefused::OtherLevel);
        }
        self.beside_listener_journal(|game| game.mob_strike_on_guest(id, conn))
    }

    /// Mob `id`'s strike on guest `conn`: it sees the guest as its player, and what it does to
    /// them runs as theirs, as the mob tick routes it (game/network/lan_mobs.rs), its other
    /// events first.
    fn mob_strike_on_guest(&mut self, id: u32, conn: u32) -> Result<bool, AttackRefused> {
        let _acting = Acting::start(&self.fake_action);
        let (events, answer) = self
            .as_guest(conn, |g| g.mob_strike(id, Who::Player))
            .flatten()
            .ok_or(AttackRefused::Stale)?;
        let (on_player, events): (Vec<_>, Vec<_>) = events.into_iter().partition(acts_on_player);
        let _ = self.handle_mob_events(events);
        let landed = self.guest_hit_by(conn, |g| {
            g.combat.hit_landed = false;
            for e in on_player {
                g.guest_mob_event(id, e);
            }
            g.combat.hit_landed
        });
        Ok(answer.unwrap_or(landed == Some(true)))
    }

    /// Mob `id`'s `doHurtTarget` on `who`, with the level lent as a goal's tick lends it: the
    /// events the strike made, and vanilla's answer when no hit decides it (a creeper's). None
    /// for a mob no longer here.
    fn mob_strike(&mut self, id: u32, who: Who) -> Option<(Vec<MobEvent>, Option<bool>)> {
        let index = self.mobs.iter().position(|m| m.id == id && !m.removed)?;
        let snapshot: Vec<_> = self
            .mobs
            .iter()
            .map(|m| super::creatures::seen(m, self.level.player_uuid, self.items()))
            .collect();
        let mut lifted = self.mobs.lift();
        let mut lent = sunrise_client::health::LentLevel::take(&mut self.world);
        let mut w = self.mob_world_base(&snapshot);
        let pos = lifted[index].position;
        w.difficulty = self.local_difficulty([pos.x, pos.y, pos.z]);
        let mut events = Vec::new();
        let answer = {
            let mut level = sunrise_client::health::DamageLevel::lent(
                &self.world,
                self.pack.assets.registry,
                &mut lent,
                Some(world_items!(self)),
                Some(&self.pack.block_registries),
                &self.damage_hooks,
            )
            .with_variants(world_variants!(self))
            .with_mob_uuids(&self.persist.uuids)
            .with_reads(crate::game::entity::hit_reads_of!(self));
            let mut cx = Cx {
                w: &w,
                level: &mut level,
                events: &mut events,
            };
            let answer = mobs::combat::do_hurt_target_any(&mut lifted[index], who, &mut cx);
            mobs::apply_brain_events(&mut lifted, &mut level, &mut events, 0, &w.teams);
            answer
        };
        drop(w);
        lent.give_back(&mut self.world);
        self.mobs.restore(
            lifted,
            sunrise_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        Some((events, answer))
    }
}

#[cfg(test)]
mod tests {
    use mod_api::{AttackRefused, EntityRef};
    use sunrise_world::food::Difficulty;

    /// `doHurtTarget` from a mod, on normal difficulty: a zombie's 3 attack damage takes a
    /// connected player from 20 to 17 and a pig from 10 to 7, answering true; a creeper's
    /// override answers true and hurts nothing; a zombie can't strike itself.
    #[test]
    #[ignore = "daily"]
    fn do_hurt_target_strikes_a_player_or_a_mob_with_the_species_override() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        use super::super::native_server::fake_action_tests::fake;
        if run_fresh_cases(
            "game::native_mob_attack::tests::do_hurt_target_strikes_a_player_or_a_mob_with_the_species_override",
            "SUNRISE_MOB_HURT_TARGET_CASE",
            &["strike"],
        ) {
            return;
        }
        with_game(|game| {
            let (_, _, conn) = fake(game, None, 0);
            // A player whose client hasn't loaded the world takes no hurt
            // (`ServerPlayer.isInvulnerableTo`'s `hasClientLoaded`).
            let host = game.lan.as_mut().unwrap();
            host.hub.update_guest(conn, |g| g.load_timeout = 0);
            game.world.health_rules.difficulty = Difficulty::Normal;
            let epoch = game.native_lifecycle.epoch().unwrap();
            let spawn = |game: &mut super::Game<'_>, kind: &str, id: u32, x: f64| {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(sunrise_client::mobs::Mob::new(
                    id,
                    kind,
                    sunrise_core::geometry::Vec3::new(x, 64.0, 0.5),
                    0.0,
                ));
                EntityRef::new(epoch, id)
            };
            let zombie = spawn(game, "minecraft:zombie", 5000, 1.5);
            let pig = spawn(game, "minecraft:pig", 5001, 2.5);
            let creeper = spawn(game, "minecraft:creeper", 5002, 3.5);
            let player = game.player(1).unwrap().entity();
            let mob = |game: &super::Game<'_>, entity| game.mob(entity).unwrap();
            let health =
                |game: &super::Game<'_>, id| game.mobs.iter().find(|m| m.id == id).unwrap().health;
            let striker = mob(game, zombie);
            assert_eq!(striker.do_hurt_target(game, player), Ok(true));
            assert_eq!(
                game.as_guest(conn, |g| g.player.as_ref().unwrap().health.health()),
                Some(17.0)
            );
            assert_eq!(striker.do_hurt_target(game, pig), Ok(true));
            assert_eq!(health(game, 5001), 7.0);
            assert_eq!(mob(game, creeper).do_hurt_target(game, pig), Ok(true));
            assert_eq!(health(game, 5001), 7.0);
            assert_eq!(
                striker.do_hurt_target(game, zombie),
                Err(AttackRefused::Unsupported)
            );
        });
    }
}
