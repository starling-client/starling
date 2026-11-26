//! Narrow, per-victim context prepared before the hit borrows its victim.
use super::{Game, combat::PlayerCredit};
use std::collections::HashMap;
use sunrise_client::{
    health::{Attack, Hurting, KnockbackCredit},
    items::ItemStack,
    mobs::{Damage, Mob, Species, Who},
};

#[derive(Default)]
pub(super) struct CreditEntity {
    id: i32,
    main: Option<ItemStack>,
}

pub(super) struct CreditPlan {
    victim: i32,
    player: Option<(PlayerCredit, CreditEntity)>,
    existing_player_live: bool,
    /// The mob the victim's hurt memory names, by that recorded id, and the
    /// entity the credit resolves to now (`LivingRef::kill_credit`'s).
    previous_mob: (u32, CreditEntity),
    attacking_mob: CreditEntity,
    responsibility: Option<Option<(PlayerCredit, CreditEntity)>>,
    responsibility_live: bool,
    fixed: CreditEntity,
    /// Canonical UUIDs of the causing entity for a villager's hurt gossip.
    hurt_by: Option<HurtBy>,
}

struct HurtBy {
    player: Option<[i32; 4]>,
    attacker: Option<(u32, [i32; 4])>,
    trade_feedback: bool,
}

pub(super) struct CreditLoan<'a> {
    plan: CreditPlan,
    players: &'a mut HashMap<u32, PlayerCredit>,
}

impl CreditPlan {
    pub(super) fn lend(self, players: &mut HashMap<u32, PlayerCredit>) -> CreditLoan<'_> {
        CreditLoan {
            plan: self,
            players,
        }
    }
}

impl KnockbackCredit for CreditLoan<'_> {
    fn accepted_mob_hit(&mut self, mob: &mut Mob) {
        assert_eq!(mob.id as i32, self.plan.victim);
        let interval = mob.ambient_interval();
        if let Some(hurt) = &mut self.plan.hurt_by
            && std::mem::take(&mut hurt.trade_feedback)
            && let Some(pending) = mob
                .extra
                .villager
                .as_mut()
                .and_then(|ext| ext.pending_hurt_player.as_mut())
        {
            // notifyTradeUpdated draws before the victim's own hurt sound or effects.
            pending.trade_pitch = sunrise_client::merchant::trade_update_pitch(
                &mut mob.ambient_timer,
                interval,
                &mut mob.random,
            );
        }
        if let Some(responsibility) = self.plan.responsibility.take() {
            mob.ai.last_hurt_by_player = if responsibility.is_some() { 100 } else { 0 };
            match &responsibility {
                Some((credit, _)) => {
                    self.players.insert(mob.id, *credit);
                }
                None => {
                    self.players.remove(&mob.id);
                }
            }
            self.plan.player = responsibility;
        }
    }

    fn hurt_by_uuid(&self, source: &Damage) -> Option<[i32; 4]> {
        let hurt_by = self.plan.hurt_by.as_ref()?;
        match source.attacker? {
            Who::Player => hurt_by.player,
            Who::Mob(id) => hurt_by
                .attacker
                .filter(|&(attacker, _)| attacker == id)
                .map(|(_, uuid)| uuid),
        }
    }

    fn resolve_player_responsible(&mut self, mob: &mut Mob) -> bool {
        assert_eq!(mob.id as i32, self.plan.victim);
        let updated = self.plan.responsibility.is_some();
        self.accepted_mob_hit(mob);
        if updated {
            self.plan.responsibility_live
        } else {
            self.players.contains_key(&mob.id) && self.plan.existing_player_live
        }
    }

    fn snapshot<'a>(&'a self, pushed: &Hurting<'_>, _: &'a Attack) -> (i32, Option<&'a ItemStack>) {
        let selected = match pushed {
            Hurting::Mob(mob, _) => {
                assert_eq!(mob.id as i32, self.plan.victim);
                if self.players.contains_key(&mob.id) {
                    // An unresolved remembered player still takes precedence over a mob.
                    self.plan.player.as_ref().map(|(_, entity)| entity)
                } else {
                    match mob.recent_hurt_by() {
                        Some(Who::Mob(id)) if id as i32 == self.plan.attacking_mob.id => {
                            Some(&self.plan.attacking_mob)
                        }
                        Some(Who::Mob(id)) if id == self.plan.previous_mob.0 => {
                            Some(&self.plan.previous_mob.1)
                        }
                        _ => None,
                    }
                }
            }
            _ => Some(&self.plan.fixed),
        };
        selected.map_or((0, None), |entity| (entity.id, entity.main.as_ref()))
    }
}

impl Game<'_> {
    /// The attacking entity's canonical UUID, already assigned on creation or admission.
    pub(super) fn known_attacker_uuid(&self, id: u32) -> Option<[i32; 4]> {
        self.known_resident_entity_uuid_at(self.dim.key(), id)
            .or_else(|| self.persist.uuids.get(&id).copied())
    }
    /// The credited entity and its main hand: in the level in place, or a mob
    /// a kill credit found in another loaded level.
    fn knockback_credit_entity(&self, id: i32) -> CreditEntity {
        if id == 0 {
            return CreditEntity::default();
        }
        if self.entity(id as u32).is_some() {
            let [main, _] = self.attack_of(id).held;
            return CreditEntity { id, main };
        }
        match self.mob_in_other_level(id as u32) {
            Some((_, mob)) => CreditEntity {
                id,
                main: mob.extra.main_hand_stack().cloned(),
            },
            None => CreditEntity::default(),
        }
    }

    fn knockback_player_credit(&self, credit: PlayerCredit) -> (PlayerCredit, CreditEntity) {
        let id = self
            .resolve_player_credit(credit)
            .and_then(|connection| self.player_entity_id_by_connection(connection))
            .unwrap_or(0);
        (credit, self.knockback_credit_entity(id))
    }

    /// Capture only this victim's possible credit holders, not a table of every entity.
    /// UUID responsibility is committed by the loan at the accepted-hit memory point.
    pub(super) fn prepare_knockback_credit(
        &self,
        victim: i32,
        attacker: Option<Who>,
        attack: &Attack,
    ) -> CreditPlan {
        let observed = mod_api::__private::listened(mod_api::HookId::LivingKnockback);
        let mob = observed
            .then(|| self.mobs.iter().find(|mob| mob.id as i32 == victim))
            .flatten();
        let existing_player = self.last_hurt_by_player.get(&(victim as u32)).copied();
        let existing_player_live =
            existing_player.is_some_and(|credit| self.resolve_player_credit(credit).is_some());
        let player = mob.and(existing_player);
        let responsibility = match attacker {
            Some(Who::Player) => attack.player_uuid.map(|uuid| {
                Some(PlayerCredit(sunrise_client::lan::playerdata::ints_uuid(
                    uuid,
                )))
            }),
            Some(Who::Mob(id)) => self
                .mobs
                .iter()
                .find(|mob| mob.id == id && mob.species() == Species::Wolf && mob.extra.tame)
                .map(|wolf| {
                    wolf.extra
                        .owner
                        .map(|uuid| PlayerCredit(sunrise_client::lan::playerdata::ints_uuid(uuid)))
                }),
            None => None,
        };
        let responsibility_live = responsibility
            .flatten()
            .is_some_and(|credit| self.resolve_player_credit(credit).is_some());
        let entity = |id| {
            if observed {
                self.knockback_credit_entity(id)
            } else {
                CreditEntity::default()
            }
        };
        let credited_player = |credit| {
            if observed {
                self.knockback_player_credit(credit)
            } else {
                (credit, CreditEntity::default())
            }
        };
        let previous_mob = mob
            .and_then(|mob| mob.ai.last_hurt_by)
            .and_then(|who| match who {
                Who::Mob(id) => Some(id),
                Who::Player => None,
            })
            .map(|id| {
                let credited = self.credited_entity(id).map_or(0, |e| e.id() as i32);
                (id, entity(credited))
            });
        let hurt_by = self
            .mobs
            .iter()
            .any(|m| m.id as i32 == victim && m.species() == Species::Villager)
            .then(|| HurtBy {
                // The hit's own player (a guest's cloud's owner), not whoever acts now.
                player: attack.player_uuid,
                attacker: match attacker {
                    Some(Who::Mob(id)) => self.known_attacker_uuid(id).map(|uuid| (id, uuid)),
                    _ => None,
                },
                trade_feedback: attacker == Some(Who::Player)
                    && attack
                        .player_uuid
                        .is_some_and(|uuid| self.villager_trade_will_notify(victim as u32, uuid)),
            });
        let fixed = if observed && mob.is_none() {
            self.entity(victim as u32)
                .and_then(|entity| self.living(entity))
                .and_then(|living| self.living_kill_credit(living))
                .map(|credit| entity(credit.id() as i32))
                .unwrap_or_default()
        } else {
            CreditEntity::default()
        };
        CreditPlan {
            victim,
            player: player.map(credited_player),
            existing_player_live,
            previous_mob: previous_mob.unwrap_or_default(),
            attacking_mob: match attacker {
                Some(Who::Mob(id)) => entity(id as i32),
                _ => CreditEntity::default(),
            },
            responsibility: responsibility.map(|credit| credit.map(credited_player)),
            responsibility_live,
            fixed,
            hurt_by,
        }
    }
}
