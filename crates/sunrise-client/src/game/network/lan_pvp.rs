//! Players hitting players on a server (`Player.attack` on a player). The hit
//! is worked out as the attacker (weapon, attack strength, critical hit,
//! enchantments, a mace's smash) and lands on the victim like any player
//! attack (blocking, armour, knockback), when the `pvp` game rule allows it
//! (`ServerPlayer.canHarmPlayer`, `isPvpAllowed`). A player's arrow on
//! another player lands the same way.
use super::{
    Game, chat::PLAYER_NAME, commands::Target, lan_actions::CombatPlayer, lan_mobs::guest_bounds,
    mace::Spared,
};

use sunrise_client::{
    Result,
    health::HurtSource,
    mobs::Who,
    projectiles::{Hit, Projectile, ProjectileKind},
    text::Text,
};
use sunrise_core::geometry::{Aabb, Vec3};

/// The player projectiles that meet other players: arrows (their hurt), and
/// snowballs, eggs and mods' thrown types, which break on them
/// (`Player.hurtServer` takes no 0 damage). Pearls, potions and wind charges
/// fly past players as before.
pub(super) fn meets_players(kind: &ProjectileKind) -> bool {
    matches!(
        kind,
        ProjectileKind::Arrow { .. }
            | ProjectileKind::Snowball
            | ProjectileKind::Egg
            | ProjectileKind::Thrown(_)
    )
}

/// Who a player's attack lands on: a player by connection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Victim {
    Guest(u32),
}

/// The attacker's side of the hit.
pub(super) struct Strike {
    /// The attacking guest, kept after their turn through `as_guest` ends.
    attacker: Option<u32>,
    /// The attacker's creative mode: their hit passes the victim's
    /// invulnerability (`DamageSource.isCreativePlayer`, #916).
    creative: bool,
    amount: f32,
    scale: f32,
    crit: bool,
    /// `Player.attack`'s `magicBoost > 0`: the enchantments added damage.
    magic: bool,
    from: [f64; 3],
    /// `getSecondsToDisableBlocking` of the attacker's weapon.
    disable_blocking: f32,
    sweep: Option<[f64; 3]>,
    /// `causeExtraKnockback`: its strength and direction (the attacker's
    /// yaw), and whether the hit is a sprinting knockback attack.
    knockback: (f32, f64, f64),
    knockback_attack: bool,
    direct_motion: Option<(f64, bool)>,
    /// The attacker's team, for the victim's `Player.canHarmPlayer`.
    team: Option<sunrise_client::scoreboard::Team>,
    /// The attacker's main-hand item with a custom name, for the death
    /// message's `.item` form.
    item: Option<sunrise_client::text::Text>,
    /// The weapon that actually produced the strike, before either guest moves.
    pub(super) weapon: Option<sunrise_client::items::ItemStack>,
    pub(super) damage_type: String,
}

/// A player's blow on another player as the native hooks name it: the hit's
/// own push and its damage name no attacker yet, so the extra and the sweep's
/// pushes don't either.
const PVP_SOURCE: HurtSource<'static> = HurtSource {
    damage_type: "minecraft:player_attack",
    causing: 0,
    direct: 0,
    position: None,
    direct_type: None,
};

#[cfg(test)]
impl Strike {
    /// A plain blow by guest `attacker`, for the tests that land one.
    pub(super) fn test_blow(attacker: u32) -> Self {
        Self {
            attacker: Some(attacker),
            creative: false,
            amount: 1.0,
            scale: 1.0,
            crit: false,
            magic: false,
            from: [3.0, 64.0, 5.0],
            disable_blocking: 0.0,
            sweep: None,
            knockback: (1.0, 1.0, 0.0),
            knockback_attack: false,
            direct_motion: None,
            team: None,
            item: None,
            weapon: None,
            damage_type: "minecraft:player_attack".into(),
        }
    }
}

/// The attacker's pre-hit inputs retained until a sweep's actual victim is in place.
#[derive(Clone)]
struct SweepDamage {
    weapon: sunrise_client::items::ItemStack,
    attacker: u32,
    base: f32,
    scale: f32,
    damage_type: String,
    direct_motion: Option<(f64, bool)>,
}

/// A sweep attack's reach to the other players (`Player.doSweepAttack`),
/// worked out as the attacker.
#[derive(Clone)]
pub(super) struct Sweep {
    /// The attacking guest (none: the host).
    attacker: Option<u32>,
    /// The attacker's creative mode: their hit passes the victim's
    /// invulnerability (`DamageSource.isCreativePlayer`, #916).
    creative: bool,
    from: [f64; 3],
    /// The damage after the enchantments and the attack strength.
    damage: f32,
    /// Custom effects need the actual victim, so keep their pre-hit input until each landing.
    enchanted_damage: Option<SweepDamage>,
    /// The push along the attacker's yaw.
    knockback: (f64, f64),
    team: Option<sunrise_client::scoreboard::Team>,
    /// The target's box grown by (1, 0.25, 1).
    pub(super) hit_box: Aabb,
    /// The entity id of the player the sweep started from, which it spares.
    pub(super) skip: Option<i32>,
}

impl Game<'_> {
    /// Whether the player a hit landed on stands on the ground.
    pub(super) fn pvp_victim_on_ground(&self, victim: Victim) -> bool {
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        match victim {
            Victim::Guest(conn) => host
                .hub
                .guest(conn)
                .and_then(|g| g.motion.as_deref())
                .is_some_and(|p| p.ground.on_ground),
        }
    }

    /// The player `id` that guest `conn` attacks, if it is one.
    pub(super) fn pvp_victim(&self, conn: u32, id: i32) -> Option<Victim> {
        let host = self.lan.as_ref()?;
        host.hub
            .guests
            .iter()
            .find(|g| g.entity_id == id && g.conn != conn && host.hub.here(g) && !g.dead)
            .map(|g| Victim::Guest(g.conn))
    }

    /// `Player.attack`'s `causeFoodExhaustion(0.1F)`, charged to the attacker
    /// (the host, or a guest swapped in) only once the hit hurt the victim.
    pub(super) fn pvp_exhaustion(&mut self) {
        if let Some(p) = self.player.as_mut()
            && !self.creative
        {
            p.cause_food_exhaustion(0.1);
        }
    }

    /// The attacker's side of `Player.attack` (the host, or a guest swapped
    /// in): the damage, the charge reset and the swing.
    pub(super) fn pvp_strike(&mut self, victim: u32) -> Option<Strike> {
        let p = self.player.as_ref()?;
        let from = [p.position.x, p.position.y, p.position.z];
        let (amount, scale, crit, magic) =
            self.melee_damage("minecraft:player", true, Some(victim));
        let sweep = self.sweep_attack_position(scale, crit, "minecraft:player");
        let strength = self.player_attack_knockback();
        let knockback_attack = scale > 0.9 && self.player.as_ref().is_some_and(|p| p.sprinting);
        let (xd, zd) = self.extra_knockback_direction();
        self.attack_ticks = 0;
        self.swing_arm();
        // LivingEntity.getSecondsToDisableBlocking: the held weapon's value,
        // none while another item is in use.
        let disable_blocking = if self.using.is_some() {
            0.0
        } else {
            self.held()
                .map_or(0.0, |stack| self.items().weapon_disable_blocking(stack))
        };
        let weapon = self.held().cloned();
        let motion = self
            .player
            .as_ref()
            .map_or((0.0, false), |p| (p.fall_distance, p.fall_flying));
        let damage_type = self.stack_melee_damage_type(weapon.as_ref(), motion, true);
        Some(Strike {
            attacker: self.lan_acting,
            creative: self.creative,
            amount,
            scale,
            crit,
            magic,
            from,
            disable_blocking,
            sweep,
            knockback: (strength, xd, zd),
            knockback_attack,
            direct_motion: Some(motion),
            team: self.player_team().cloned(),
            item: self.main_hand_custom_name(),
            weapon,
            damage_type,
        })
    }

    /// The guests by connection, with their entity ids.
    pub(super) fn lan_guest_entities(&self) -> Vec<(u32, u32)> {
        self.lan.as_ref().map_or_else(Vec::new, |host| {
            host.hub
                .guests
                .iter()
                .map(|g| (g.conn, g.entity_id as u32))
                .collect()
        })
    }

    /// The player a player's projectile hit, when it hit one.
    pub(super) fn projectile_pvp_victim(&self, p: &Projectile, hit: Hit) -> Option<Victim> {
        if p.owner != Some(Who::Player) || !meets_players(&p.kind) {
            return None;
        }
        let Hit::Entity { who, .. } = hit else {
            return None;
        };
        match who {
            Who::Player => None,
            Who::Mob(id) => self
                .lan
                .as_ref()?
                .hub
                .guests
                .iter()
                .find(|g| g.entity_id as u32 == id)
                .map(|g| Victim::Guest(g.conn)),
        }
    }

    /// `ServerPlayer.canHarmPlayer` of the players `attacker` and `victim` (entity ids) on
    /// their teams: the `pvp` game rule as it stands, handed to
    /// [`can_harm_player`](sunrise_client::health::can_harm_player), which decides.
    pub(in crate::game) fn player_may_harm(
        &self,
        attacker: i32,
        victim: i32,
        attacker_team: Option<&sunrise_client_core::scoreboard::Team>,
        victim_team: Option<&sunrise_client_core::scoreboard::Team>,
    ) -> bool {
        let answer = sunrise_client::health::can_harm_player(
            attacker,
            victim,
            self.ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::PVP),
            attacker_team,
            victim_team,
        );
        #[cfg(test)]
        let answer = super::rule_replacements::harm(attacker, victim).unwrap_or(answer);
        let _ = (attacker, victim);
        answer
    }

    /// [`player_may_harm`](Self::player_may_harm) for a hit on the guest `victim` by the guest
    /// `attacker` (connections), or by the player in place when none: a player's projectile or
    /// trident, which the victim's hurt refuses when it answers false. A guest no longer here
    /// is entity -1, on no team.
    pub(in crate::game) fn guest_may_harm(&self, attacker: Option<u32>, victim: u32) -> bool {
        let board = &self.score.board;
        let guest = |conn: u32| {
            self.lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .map_or((-1, None), |g| (g.entity_id, board.players_team(&g.name)))
        };
        let (attacker, attacker_team) = match attacker {
            Some(conn) => guest(conn),
            None => (self.own_entity_id(), self.player_team()),
        };
        let (victim, victim_team) = guest(victim);
        self.player_may_harm(attacker, victim, attacker_team, victim_team)
    }

    /// A player's projectile on a player: the hit runs as the victim, whose
    /// hurt refuses it when the `pvp` game rule or their teams forbid it
    /// (`ServerPlayer.canHarmPlayer`; `AbstractArrow.onHitEntity` then turns
    /// the arrow back).
    pub(super) fn projectile_hits_player(
        &mut self,
        p: &Projectile,
        hit: Hit,
        victim: Victim,
    ) -> Result<()> {
        let Hit::Entity { at, .. } = hit else {
            return Ok(());
        };
        if self.lan.is_none() {
            return self.projectile_hit(p, hit);
        }
        let refused = match victim {
            Victim::Guest(conn) => !self.guest_may_harm(p.guest, conn),
        };
        let on_player = Hit::Entity {
            who: Who::Player,
            at,
        };
        let shooter = self.player_attacker(p.guest);
        let effect_attacker = self.player_entity_id_by_connection(p.guest);
        let Victim::Guest(conn) = victim;
        let mut landed_arrow = None;
        self.guest_hit_by(conn, |g| {
            g.combat.refuse_player_hit = refused;
            g.combat.projectile_attacker = shooter;
            let saved_effect_attacker = std::mem::replace(
                &mut g.combat.enchantment_projectile_attacker,
                effect_attacker,
            );
            let displaced = effect_attacker.is_some_and(|id| {
                matches!(
                    g.combat_player(id as u32),
                    Some(CombatPlayer::Displaced(..))
                )
            });
            let result = g.projectile_hit_deferring_player_arrow(
                p,
                on_player,
                displaced.then_some(&mut landed_arrow),
            );
            g.combat.refuse_player_hit = false;
            g.combat.projectile_attacker = None;
            g.combat.enchantment_projectile_attacker = saved_effect_attacker;
            result
        })
        .unwrap_or(Ok(()))?;
        if let Some(landed) = landed_arrow {
            self.resume_guest_arrow_hit(conn, landed);
        }
        Ok(())
    }

    /// `AbstractArrow.onHitEntity`: an arrow that hurt a player tells its owner, a player other
    /// than the one hit, with `PLAY_ARROW_HIT_SOUND`, unless the arrow is silent.
    pub(super) fn arrow_hit_sound_to_shooter(&self, p: &Projectile) {
        let (Some(host), Some(shooter)) = (self.lan.as_ref(), p.guest) else {
            return;
        };
        if self.lan_acting == Some(shooter) || self.entity_silent(p.id) {
            return;
        }
        host.hub.send_to(
            shooter,
            &sunrise_protocol::packets::play::Clientbound::GameEvent(
                sunrise_protocol::packets::play::GameEvent {
                    event: sunrise_client::lan::hub::game_event::PLAY_ARROW_HIT_SOUND,
                    param: 0.0,
                },
            ),
        );
    }

    pub(super) fn player_attacker(&self, conn: Option<u32>) -> Option<(u32, Text)> {
        match conn {
            Some(conn) => {
                let g = self.lan.as_ref()?.hub.guest(conn)?;
                Some((g.entity_id as u32, self.player_text(&g.name, g.uuid)))
            }
            None => None,
        }
    }

    /// The hit landing on the victim with its extra knockback, then the
    /// attacker's attack sound. A landed hit returns whether it stopped the attacker's sprint.
    pub(super) fn pvp_land(&mut self, victim: Victim, s: &Strike) -> Option<bool> {
        self.lan.as_ref()?;
        if s.knockback_attack {
            self.play_audio(
                "minecraft:entity.player.attack.knockback",
                Some(s.from),
                1.0,
                1.0,
            );
        }
        let host = self.lan.as_ref()?;
        // CombatTracker: the attacking player is the death message's killer.
        // Without it the message is built with one argument and a template
        // that names a killer ("%1$s was slain by %2$s") shows as written.
        let attacker_note = s.attacker.and_then(|conn| {
            host.hub
                .guest(conn)
                .map(|a| (a.entity_id, self.player_text(&a.name, a.uuid)))
        });
        let attacker_id = self.player.as_ref().map_or(0, |p| p.entity_id);
        let attacker_id = attacker_note.as_ref().map_or(attacker_id, |(id, _)| *id);
        // The blow's attacker as `canHarmPlayer` asks it: the host with no attacking guest,
        // and a guest no longer here as entity -1 (with the blow's own team), as
        // `guest_may_harm` asks it.
        let harm_attacker = match (s.attacker, &attacker_note) {
            (Some(_), Some((id, _))) => *id,
            (Some(_), None) => -1,
            (None, _) => attacker_id,
        };
        let (victim_id, team) = match victim {
            Victim::Guest(conn) => host.hub.guest(conn).map_or((-1, None), |g| {
                (g.entity_id, self.score.board.players_team(&g.name))
            }),
        };
        let harm = self.player_may_harm(harm_attacker, victim_id, s.team.as_ref(), team);
        // Player.attack: the hurt (with its knockback away from the
        // attacker), then causeExtraKnockback along the attacker's yaw.
        // Taken here, before the victim is swapped in, so the attacker's hands are found.
        let attack = self.attack_of(attacker_id);
        let land = |g: &mut Self| {
            g.with_attack(attack.clone(), |g| {
                g.combat.player_attacker = Some((s.from, s.disable_blocking));
                if let Some((id, name)) = attacker_note.clone() {
                    g.note_player_attacker(&s.damage_type, id as u32, name, s.item.clone());
                }
                let hurt = g.mob_hits_player_from(s.amount, s.from, &s.damage_type, s.creative);
                g.combat.player_attacker = None;
                if !hurt {
                    return None;
                }
                let (base, xd, zd) = s.knockback;
                let source = attack.player_source(&s.damage_type, attack.attacker);
                let mut ctx = attack.enchantment_context(source, "minecraft:player");
                ctx.direct_motion = s.direct_motion;
                ctx.attacker_motion = s.direct_motion;
                let strength = s.weapon.as_ref().map_or(base, |weapon| {
                    g.evaluate_weapon_value(
                        weapon,
                        "minecraft:knockback",
                        Some(g.own_entity_id() as u32),
                        ctx,
                        base,
                    )
                });
                let strength = strength * 0.5 + if s.knockback_attack { 0.5 } else { 0.0 };
                if strength > 0.0 {
                    g.knock_back_player_full(
                        f64::from(strength),
                        xd,
                        zd,
                        PVP_SOURCE,
                        &attack,
                        s.amount,
                        true,
                    );
                }
                Some(strength > 0.0)
            })
        };
        let landed = harm
            .then(|| match victim {
                Victim::Guest(conn) => self.guest_struck_by(conn, land).flatten(),
            })
            .flatten();
        let hurt = landed.is_some();
        if hurt && (s.crit || s.magic) {
            let target = match victim {
                Victim::Guest(conn) => self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .and_then(|g| g.motion.as_deref().map(|p| (g.entity_id, p))),
            };
            if let Some((id, p)) = target {
                let d = p.dimensions;
                let (feet, width, height) = (
                    [p.position.x, p.position.y, p.position.z],
                    f64::from(d.width),
                    f64::from(d.height),
                );
                if s.crit {
                    self.critical_hit(s.attacker, id, feet, width, height, false);
                }
                if s.magic {
                    self.critical_hit(s.attacker, id, feet, width, height, true);
                }
            }
        }
        let sound = match (hurt, s.crit, s.scale > 0.9) {
            (false, ..) => "minecraft:entity.player.attack.nodamage",
            (true, true, _) => "minecraft:entity.player.attack.crit",
            (true, false, true) => "minecraft:entity.player.attack.strong",
            (true, false, false) => "minecraft:entity.player.attack.weak",
        };
        let swept = s.sweep.filter(|_| hurt);
        if let Some(at) = swept {
            // The sweep around the player that was hit, as its attacker.
            let (area, id) = match victim {
                Victim::Guest(conn) => {
                    let g = self.lan.as_ref().and_then(|host| host.hub.guest(conn));
                    (g.map(guest_bounds), g.map_or(-1, |g| g.entity_id))
                }
            };
            let target = area.map(|area| (area, Spared::Player(id)));
            match s.attacker {
                None => {
                    // The host is checked out here: collect the sweep for the players.
                    self.combat.sweeps = Some(Vec::new());
                    self.sweep_attack(at, Some(s.from), target, s.scale);
                    for sweep in self.combat.sweeps.take().unwrap_or_default() {
                        self.sweep_players(&sweep);
                    }
                }
                Some(conn) => {
                    self.as_guest(conn, |g| g.sweep_attack(at, Some(s.from), target, s.scale));
                }
            }
        }
        // A critical hit a mod let sweep has both sounds (attackVisualEffects).
        if swept.is_none() || s.crit {
            self.play_audio(sound, Some(s.from), 1.0, 1.0);
        }
        landed
    }

    /// `ServerPlayer.die`'s `killer.awardKillScore(this, source)` for a guest,
    /// whatever source killed them (`LivingEntity.getKillCredit`), then
    /// `createWitherRose(killer)`: the killer's kill counts and `player_kills`,
    /// and the rose a wither's kill leaves where they fell. A mob credits no
    /// score.
    pub(super) fn guest_award_kill_score(&mut self, victim: u32, at: [f64; 3]) {
        let Some(id) = self
            .guest_combat_record(victim)
            .and_then(|c| c.kill_credit(self.world.game_time))
        else {
            return;
        };
        // `awardKillScore` excludes the victim, even when that body is in place.
        if let Some(killer) = self.combat_player(id)
            && let Some(name) = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(victim))
                .filter(|g| g.entity_id as u32 != id)
                .map(|g| g.name.clone())
        {
            match killer {
                CombatPlayer::InPlace => {
                    self.award_custom_stat("player_kills", 1);
                    self.score_player_killed_named(Some(name), true);
                }
                CombatPlayer::Displaced(depth, _) => {
                    let frame = &self.lan_displaced_players[depth];
                    let killer = frame.name.as_deref().unwrap_or(PLAYER_NAME).to_owned();
                    self.award_displaced_stat(depth, "player_kills", 1);
                    self.score_player_kill_for(&killer, Some(&name), true);
                }
                CombatPlayer::Parked(guest) => {
                    self.as_guest(guest.conn, |g| {
                        g.award_custom_stat("player_kills", 1);
                        g.score_player_killed_named(Some(name), true);
                    });
                }
            }
        }
        self.create_wither_rose_by(at, Some(id));
    }

    /// The sweep of the player in place, for the players it reaches: the
    /// damage (`1 + ratio * baseDamage`, plus the enchantments against a
    /// player, scaled by the attack strength) and who hits. The box is set
    /// by the caller.
    pub(super) fn player_sweep(&mut self, base: f32, scale: f32) -> Sweep {
        let from = self
            .player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]);
        let enchanted_damage = self.held().zip(self.player.as_ref()).map(|(held, player)| {
            let weapon = self.item_combat_stack(held);
            let motion = (player.fall_distance, player.fall_flying);
            let damage_type = self.stack_melee_damage_type(Some(&weapon), motion, true);
            SweepDamage {
                weapon,
                attacker: player.entity_id as u32,
                base,
                scale,
                damage_type,
                direct_motion: Some(motion),
            }
        });
        let damage = if enchanted_damage.is_some() {
            0.0
        } else {
            base * scale
        };
        Sweep {
            attacker: self.lan_acting,
            creative: self.creative,
            from,
            damage,
            enchanted_damage,
            knockback: self.extra_knockback_direction(),
            team: self.player_team().cloned(),
            hit_box: Aabb::new(Vec3::ZERO, Vec3::ZERO),
            skip: None,
        }
    }

    /// A sweep from a mob hit reaches the other players. Inside a guest's
    /// turn the host holds the other players (`as_guest`), so it waits for
    /// the turn to end.
    pub(super) fn sweep_other_players(&mut self, sweep: Sweep) {
        if let Some(collect) = self.combat.sweeps.as_mut() {
            collect.push(sweep);
        } else if self.lan_acting.is_some() {
            self.creatures.sweeps.push(sweep);
        } else {
            self.sweep_players(&sweep);
        }
    }

    /// The sweeps guests' turns put off.
    pub(super) fn flush_sweeps(&mut self) {
        for sweep in std::mem::take(&mut self.creatures.sweeps) {
            self.sweep_other_players(sweep);
        }
    }

    /// `Player.doSweepAttack` on the players: each other player in the box,
    /// closer than 3 blocks (`distanceToSqr < 9`) and not an ally
    /// (`isAlliedTo`), is hurt (`ServerPlayer.canHarmPlayer`: the `pvp` rule)
    /// and pushed 0.4 along the attacker's yaw.
    pub(super) fn sweep_players(&mut self, sweep: &Sweep) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let from = Vec3::new(sweep.from[0], sweep.from[1], sweep.from[2]);
        let mut victims = Vec::new();
        for g in &host.hub.guests {
            if host.hub.here(g)
                && g.loaded
                && !g.dead
                && !g.spectator()
                && sweep.attacker != Some(g.conn)
                && guest_bounds(g).intersects(sweep.hit_box)
                && Vec3::new(g.position[0], g.position[1], g.position[2]).distance_squared(from)
                    < 9.0
            {
                let team = self.score.board.players_team(&g.name).cloned();
                victims.push((Victim::Guest(g.conn), g.entity_id, team));
            }
        }
        let (xd, zd) = sweep.knockback;
        // The sweep uses the attacking player's source, even for a local body.
        let attacker = match sweep.attacker {
            Some(conn) => self.player_attacker(Some(conn)),
            None => self
                .player
                .as_ref()
                .map(|p| (p.entity_id as u32, self.target_name(Target::Player))),
        };
        let attacker_id = self.player.as_ref().map_or(0, |p| p.entity_id);
        let attack = self.attack_of(attacker.as_ref().map_or(attacker_id, |(id, _)| *id as i32));
        let land = |g: &mut Self| {
            let damage = if let Some(damage) = &sweep.enchanted_damage {
                let victim = g.player.as_ref().map(|player| player.entity_id as u32);
                g.weapon_enchanted_damage(
                    &damage.weapon,
                    victim.map(|victim| (damage.attacker, victim)),
                    "minecraft:player",
                    &damage.damage_type,
                    damage.base,
                    damage.direct_motion,
                ) * damage.scale
            } else {
                sweep.damage
            };
            let damage_type = sweep
                .enchanted_damage
                .as_ref()
                .map_or("minecraft:player_attack", |damage| {
                    damage.damage_type.as_str()
                });
            g.with_attack(attack.clone(), |g| {
                if let Some((id, name)) = &attacker {
                    g.note_named_attacker(damage_type, *id, name.clone());
                }
                let hurt = g.mob_hits_player_from(damage, sweep.from, damage_type, sweep.creative);
                if hurt {
                    g.knock_back_player(0.4, xd, zd, PVP_SOURCE, &attack, damage);
                }
                hurt
            })
        };
        let attacker_id = attacker.as_ref().map_or(attacker_id, |(id, _)| *id as i32);
        for (victim, id, team) in victims {
            if sweep.skip == Some(id)
                || sunrise_client::scoreboard::teams_allied(team.as_ref(), sweep.team.as_ref())
                || !self.player_may_harm(attacker_id, id, sweep.team.as_ref(), team.as_ref())
            {
                continue;
            }
            let Victim::Guest(conn) = victim;
            if self.guest_struck_by(conn, land).unwrap_or_default() {
                let damage_type = sweep
                    .enchanted_damage
                    .as_ref()
                    .map_or("minecraft:player_attack", |damage| {
                        damage.damage_type.as_str()
                    });
                let source = attack.player_source(damage_type, attack.attacker);
                let weapon = sweep.enchanted_damage.as_ref().map(|damage| &damage.weapon);
                self.post_attack(
                    id as u32,
                    "minecraft:player",
                    &attack,
                    source,
                    weapon,
                    false,
                );
            }
        }
    }
}

#[cfg(test)]
pub(in crate::game) mod arrow_hit_sound_tests {
    use super::{Victim, Who};
    use std::fmt::Write as _;
    use std::path::PathBuf;
    use std::sync::{Arc, atomic::AtomicUsize, mpsc};
    use sunrise_client::{
        command::args::GameMode,
        gamerules::Value,
        lan::{Out, PlayerSender},
        motion::PlayerMotion,
        projectiles::{Hit, Projectile, ProjectileKind},
    };
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::play::Clientbound;

    fn load_witness() -> Witness {
        let value = std::env::var_os("SUNRISE_ARROW_HIT_DING_WITNESS").unwrap_or_else(|| {
            panic!(
                "SUNRISE_ARROW_HIT_DING_WITNESS is unset; run tools/test_arrow_hit_ding_parity.py, which records the witness"
            )
        });
        let raw_path = PathBuf::from(value);
        let path = if raw_path.is_dir() {
            raw_path.join("arrow_hit_ding.json")
        } else {
            raw_path
        };
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "failed to read witness {} (record via tools/test_arrow_hit_ding_parity.py): {e}",
                path.display()
            )
        });
        serde_json::from_str(&raw).expect("valid 26.3 arrow-hit witness JSON")
    }

    #[derive(serde::Deserialize)]
    struct Witness {
        minecraft_version: String,
        cases: Vec<WitnessCase>,
    }

    #[derive(serde::Deserialize)]
    struct WitnessCase {
        name: String,
        shooter: String,
        victim: String,
        silent: bool,
        pvp: bool,
        victim_mode: String,
        victim_hurt: bool,
        victim_health_before_bits: String,
        victim_health_after_bits: String,
        alice_game_events: Vec<WireGameEvent>,
        bob_game_events: Vec<WireGameEvent>,
    }

    #[derive(Debug, PartialEq, Eq, serde::Deserialize)]
    struct WireGameEvent {
        event: u8,
        param_bits: String,
        payload_hex: String,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CaseOutcome {
        victim_hurt: bool,
        victim_health_before_bits: String,
        victim_health_after_bits: String,
        alice_game_events: Vec<WireGameEvent>,
        bob_game_events: Vec<WireGameEvent>,
    }

    fn hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            write!(out, "{b:02x}").unwrap();
        }
        out
    }

    fn float_bits_hex(value: f32) -> String {
        format!("{:08x}", value.to_bits())
    }

    fn player_conn(name: &str) -> u32 {
        match name {
            "Alice" => 12,
            "Bob" => 13,
            other => panic!("unexpected witness player: {other}"),
        }
    }

    /// Wire `ClientboundGameEventPacket` records sent to a guest connection.
    fn game_events(receiver: &mpsc::Receiver<Out>) -> Vec<WireGameEvent> {
        let mut events = Vec::new();
        for sent in receiver.try_iter() {
            let bodies = match sent {
                Out::Packet(p) => vec![p],
                Out::Packets(p) => p,
                Out::Disconnect(_) => panic!("unexpected disconnect"),
            };
            for body in bodies {
                let mut reader = sunrise_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                if let Ok(Clientbound::GameEvent(e)) = Clientbound::decode(id, reader.rest()) {
                    events.push(WireGameEvent {
                        event: e.event,
                        param_bits: float_bits_hex(e.param),
                        payload_hex: hex(&body),
                    });
                }
            }
        }
        events
    }

    /// Run one recorded vanilla 26.3 arrow-hit case between guests 12 (Alice) and 13 (Bob).
    fn run_case(case: &WitnessCase) -> CaseOutcome {
        let shooter = player_conn(&case.shooter);
        let victim = player_conn(&case.victim);
        let victim_mode = match case.victim_mode.as_str() {
            "survival" => GameMode::Survival,
            "creative" => GameMode::Creative,
            other => panic!("unexpected witness victim_mode: {other}"),
        };
        let mut outcome = None;
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let mut receivers = Vec::new();
            for conn in [12, 13] {
                let (tx, rx) = mpsc::channel();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |g| {
                        g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
                    })
                    .unwrap();
                receivers.push(rx);
            }
            let mut body = PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0)).unwrap();
            body.entity_id = victim as i32;
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(victim, |g| {
                    g.motion = Some(Box::new(body));
                    g.load_timeout = 0;
                })
                .unwrap();
            assert!(game.ui.rules.set("pvp", Value::Bool(case.pvp)));
            game.as_guest(victim, |g| {
                g.set_game_mode(GameMode::Creative);
                if victim_mode != GameMode::Creative {
                    g.set_game_mode(victim_mode);
                }
            })
            .unwrap();
            for g in &game.lan.as_ref().unwrap().hub.guests {
                g.sender.flush();
            }
            for rx in &receivers {
                while rx.try_recv().is_ok() {}
            }
            let health_before = game.lan.as_ref().unwrap().hub.guest(victim).unwrap().health;
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(1.0, 65.0, 5.0),
                Some(Who::Player),
            );
            arrow.guest = Some(shooter);
            arrow.velocity = Vec3::new(0.0, -1.0, 0.0);
            if case.silent {
                game.persist.raw.insert(
                    arrow.id,
                    sunrise_core::nbt::Tag::compound([("Silent", sunrise_core::nbt::Tag::Byte(1))]),
                );
            }
            let hit = Hit::Entity {
                who: Who::Player,
                at: Vec3::new(2.7, 65.0, 5.0),
            };
            game.projectile_hits_player(&arrow, hit, Victim::Guest(victim))
                .unwrap();
            let health_after = game.lan.as_ref().unwrap().hub.guest(victim).unwrap().health;
            for g in &game.lan.as_ref().unwrap().hub.guests {
                g.sender.flush();
            }
            outcome = Some(CaseOutcome {
                victim_hurt: health_after < health_before,
                victim_health_before_bits: float_bits_hex(health_before),
                victim_health_after_bits: float_bits_hex(health_after),
                alice_game_events: game_events(&receivers[0]),
                bob_game_events: game_events(&receivers[1]),
            });
        });
        outcome.unwrap()
    }

    /// Vanilla 26.3 witness recorded by `tools/test_arrow_hit_ding_parity.py`: exact wire `ClientboundGameEventPacket` and victim health bits on player arrow hits.
    #[test]
    #[ignore = "run by tools/test_arrow_hit_ding_parity.py"]
    fn an_arrow_that_hurts_another_player_dings_its_shooter() {
        run_an_arrow_that_hurts_another_player_dings_its_shooter();
    }

    pub(in crate::game) fn run_an_arrow_that_hurts_another_player_dings_its_shooter() {
        let witness = load_witness();
        assert_eq!(witness.minecraft_version, "26.3");
        assert!(!witness.cases.is_empty());
        for case in witness.cases {
            let actual = run_case(&case);
            let expected = CaseOutcome {
                victim_hurt: case.victim_hurt,
                victim_health_before_bits: case.victim_health_before_bits,
                victim_health_after_bits: case.victim_health_after_bits,
                alice_game_events: case.alice_game_events,
                bob_game_events: case.bob_game_events,
            };
            assert_eq!(actual, expected, "26.3 witness case {}", case.name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::{
        bind_local_player, fixture_resident_column, run_fresh_cases, run_with_game_stack,
    };
    use crate::game::native_guest_turn_tests::with_guests;
    use sunrise_client::{
        command::args::GameMode, gamerules::Value, items::ItemStack, mobs::Mob,
        motion::PlayerMotion,
    };

    fn ready_victim(game: &mut Game<'_>, with_full_armor: bool) {
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |guest| {
                guest.loaded = true;
                guest.load_timeout = 0;
            })
            .unwrap();
        game.as_guest(13, |game| {
            game.set_mode(GameMode::Survival);
            let mut chest = ItemStack::new("minecraft:diamond_chestplate", 1);
            chest.set_crafting_enchantments(vec![("minecraft:thorns".into(), 255)]);
            game.armor[1] = Some(chest);
            if with_full_armor {
                game.armor[0] = Some(ItemStack::new("minecraft:diamond_helmet", 1));
                game.armor[2] = Some(ItemStack::new("minecraft:diamond_leggings", 1));
                game.armor[3] = Some(ItemStack::new("minecraft:diamond_boots", 1));
            }
            game.put_equipment_modifiers();
        })
        .unwrap();
    }

    /// A fresh survival body for guest `conn` (entity `conn`), where `with_guests` stands it.
    fn fresh_body(game: &mut Game<'_>, conn: u32) {
        let mut body =
            sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0))
                .unwrap();
        body.entity_id = conn as i32;
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(conn, |g| {
                g.motion = Some(Box::new(body));
                g.load_timeout = 0;
                g.loaded = true;
            })
            .unwrap();
        game.as_guest(conn, |g| {
            g.set_game_mode(GameMode::Creative);
            g.set_game_mode(GameMode::Survival);
        })
        .unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn pvp_mace_smash_keeps_the_players_breach_and_does_not_retaliate_on_a_bystander() {
        if run_fresh_cases(
            "game::lan_pvp::tests::pvp_mace_smash_keeps_the_players_breach_and_does_not_retaliate_on_a_bystander",
            "SUNRISE_PVP_MACE_SMASH_CHILD",
            &["smash"],
        ) {
            return;
        }
        run_with_game_stack(|| {
            with_guests(|game| {
                fixture_resident_column(&mut game.world, (0, 0));
                ready_victim(game, true);
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Hard;
                let mut mace = ItemStack::new("minecraft:mace", 1);
                mace.enchant("minecraft:breach", 4);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[guest.selected] = Some(mace);
                    })
                    .unwrap();
                let mut strike = Strike::test_blow(12);
                strike.amount = 5.0;
                strike.damage_type = "minecraft:mace_smash".into();
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                game.mobs
                    .push(Mob::new(41, cow, Vec3::new(3.0, 64.0, 5.0), 0.0));
                let bystander_health = game.mobs[0].health;
                assert!(game.pvp_land(Victim::Guest(13), &strike).is_some());
                let victim = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
                // CombatRules.getDamageAfterAbsorb: full diamond blocks 0.75 of a 5-point
                // hit; Breach IV subtracts 0.60 from that fraction, leaving 4.25 damage.
                assert_eq!(victim.health, 15.75);
                assert_eq!(game.mobs[0].health, bystander_health);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn guest_thorns_answers_the_host_that_shot_the_arrow() {
        if run_fresh_cases(
            "game::lan_pvp::tests::guest_thorns_answers_the_host_that_shot_the_arrow",
            "SUNRISE_PVP_ARROW_THORNS_CHILD",
            &["arrow"],
        ) {
            return;
        }
        run_with_game_stack(|| {
            with_guests(|game| {
                fixture_resident_column(&mut game.world, (0, 0));
                ready_victim(game, false);
                let start = Vec3::new(1.0, 65.0, 5.0);
                game.player = Some(PlayerMotion::new(game.motion, start).unwrap());
                bind_local_player(game, 77, 77);
                game.set_mode(GameMode::Survival);
                let before = game.player.as_ref().unwrap().health.health();
                let at = Vec3::new(3.0, 65.0, 5.0);
                let mut arrow = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    at,
                    Some(Who::Player),
                );
                arrow.id = 90;
                arrow.previous = start;
                arrow.velocity = Vec3::new(2.0, 0.0, 0.0);
                game.projectile_hits_player(
                    &arrow,
                    Hit::Entity {
                        who: Who::Mob(13),
                        at,
                    },
                    Victim::Guest(13),
                )
                .unwrap();
                let victim = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
                assert!(victim.health < 20.0, "the arrow did not hurt the guest");
                let host = game.player.as_ref().unwrap();
                assert_eq!(host.entity_id, 77);
                assert!(
                    host.health.health() < before,
                    "Thorns did not hurt the shooter"
                );
            });
        });
    }

    fn health(game: &Game<'_>, conn: u32) -> f32 {
        let host = game.lan.as_ref().unwrap();
        host.hub
            .guest(conn)
            .unwrap()
            .motion
            .as_ref()
            .unwrap()
            .health
            .health()
    }

    /// What each player-on-player rule answers for guest 12 on guest 13, each from a fresh
    /// body: the `PlayerRef::can_harm` read, an arrow (`AbstractArrow.canHitEntity`), another
    /// projectile or a trident (the victim's hurt), a spear's stab, a melee blow, its sweep
    /// (which also spares allies, `doSweepAttack`'s `isAlliedTo`) and a blast of guest 12's
    /// primed TNT.
    fn every_site(game: &mut Game<'_>) -> [bool; 7] {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        fresh_body(game, 13);
        let read = a.can_harm(&*game, b) == Some(true);
        let arrow = game.can_arrow_harm_player(Who::Mob(12), None, Who::Mob(13));
        let projectile = game.guest_may_harm(Some(12), 13);
        let spear = game.stab_players_for(12).iter().any(|p| p.conn == 13);
        let attacker_team = game.score.board.players_team("guest12").cloned();
        let mut blow = Strike::test_blow(12);
        blow.team = attacker_team.clone();
        let melee = game.pvp_land(Victim::Guest(13), &blow).is_some();

        fresh_body(game, 13);
        let sweep = Sweep {
            attacker: Some(12),
            creative: false,
            from: [3.0, 64.0, 5.0],
            damage: 1.0,
            enchanted_damage: None,
            knockback: (0.0, 0.0),
            team: attacker_team,
            hit_box: Aabb::new(Vec3::new(0.0, 60.0, 0.0), Vec3::new(6.0, 70.0, 10.0)),
            skip: None,
        };
        let before = health(game, 13);
        game.sweep_players(&sweep);
        let swept = health(game, 13) < before;

        fresh_body(game, 13);
        game.combat.blast_owner = Some((12, sunrise_client::text::Text::literal("guest12")));
        game.combat.blast_owner_player = true;
        game.combat.blast_guest = Some(12);
        let before = health(game, 13);
        game.server_explode(
            [3.0, 64.0, 8.0],
            2.0,
            super::super::explosion::BlockInteraction::Keep,
            false,
        )
        .unwrap();
        let blasted = health(game, 13) < before;
        game.combat.blast_owner = None;
        game.combat.blast_owner_player = false;
        game.combat.blast_guest = None;
        [read, arrow, projectile, spear, melee, swept, blasted]
    }

    /// The guests of `with_guests`, in an empty chunk the blasts see through.
    fn with_two_guests(test: impl FnOnce(&mut Game<'_>)) {
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            test(game);
        });
    }

    /// `ServerPlayer.canHarmPlayer` is answered in one place,
    /// [`can_harm_player`](sunrise_client::health::can_harm_player), and every
    /// player-on-player rule takes its answer for guest 12 on guest 13, the `pvp` rule first,
    /// then the teams. The sweep alone also spares allies.
    #[test]
    #[ignore = "daily"]
    fn every_player_on_player_rule_takes_can_harm_player() {
        with_two_guests(|game| {
            let mut answers = Vec::new();
            for (pvp, team) in [
                (true, None),
                (false, None),
                (true, Some(false)),
                (true, Some(true)),
            ] {
                game.ui.rules.set("pvp", Value::Bool(pvp));
                if let Some(friendly_fire) = team {
                    let board = &mut game.score.board;
                    board.add_team("red").allow_friendly_fire = friendly_fire;
                    board.add_to_team("guest12", "red");
                    board.add_to_team("guest13", "red");
                }
                let board = &game.score.board;
                let (own, other) = (board.players_team("guest12"), board.players_team("guest13"));
                let allied = sunrise_client::scoreboard::teams_allied(own, other);
                let expected = sunrise_client::health::can_harm_player(12, 13, pvp, own, other);
                answers.push(expected);
                let mut wanted = [expected; 7];
                wanted[5] = expected && !allied;
                assert_eq!(every_site(game), wanted, "pvp {pvp}, team {team:?}");
            }
            assert_eq!(answers, [true, false, false, true]);
        });
    }

    /// A mod's row that changes `canHarmPlayer`'s answer changes it at every site: with the
    /// answer replaced on this thread (`rule_replacements::HARM`), two players on a server with
    /// `pvp` off hurt each other everywhere, and with `pvp` on and no teams nowhere.
    #[test]
    #[ignore = "daily"]
    fn a_replaced_can_harm_player_answer_reaches_every_site() {
        use super::super::rule_replacements::{HARM, replaced};
        with_two_guests(|game| {
            game.ui.rules.set("pvp", Value::Bool(false));
            let sites = replaced(&HARM, |_, _| true, || every_site(game));
            assert_eq!(sites, [true; 7], "pvp off, answered true");
            game.ui.rules.set("pvp", Value::Bool(true));
            let sites = replaced(&HARM, |_, _| false, || every_site(game));
            assert_eq!(sites, [false; 7], "pvp on, answered false");
        });
    }
}
