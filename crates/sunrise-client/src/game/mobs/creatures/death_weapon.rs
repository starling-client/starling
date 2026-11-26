//! Recorded projectile weapons and the death-loot producer.
use super::*;

impl Game<'_> {
    /// `DamageSources.arrow` / `trident`: type and tags follow the actual projectile.
    pub(in crate::game) fn projectile_damage_kind(&self, shot: &Projectile) -> DamageKind {
        match shot.kind {
            ProjectileKind::Arrow { .. } => DamageKind::Arrow,
            ProjectileKind::Trident => DamageKind::Named(
                sunrise_client::mobs::NamedDamage::registered(
                    &self.health_resources.damage,
                    "minecraft:trident",
                )
                .expect("registered vanilla trident damage"),
            ),
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                DamageKind::Fireball
            }
            _ => DamageKind::Thrown,
        }
    }
    pub(in crate::game) fn projectile_attack(&self, shot: &Projectile) -> Attack {
        let mut attack = match shot.owner {
            Some(Who::Player) => self
                .player_entity_id_by_connection(shot.guest)
                .or(self.combat.enchantment_projectile_attacker)
                .map_or_else(Attack::default, |entity| self.attack_of(entity)),
            Some(Who::Mob(entity)) => self.attack_of(entity as i32),
            None => Attack::default(),
        };
        self.set_projectile_attack(&mut attack, shot);
        attack
    }
    /// The actual direct projectile's family and saved weapon, also for explicit damage sources.
    pub(in crate::game) fn set_projectile_attack(&self, attack: &mut Attack, shot: &Projectile) {
        Self::set_projectile_source(attack, shot);
        let weapon = shot.arrow.as_ref().and_then(|arrow| {
            if matches!(shot.kind, ProjectileKind::Trident) {
                Some(&arrow.pickup_item)
            } else {
                arrow.fired_from_weapon.as_ref()
            }
        });
        self.set_projectile_weapon(attack, weapon);
    }
    /// Classify the actual direct entity independently of the world's weapon rules.
    pub(in crate::game) fn set_projectile_source(attack: &mut Attack, shot: &Projectile) {
        attack.projectile = Some(shot.id as i32);
        attack.enchantment_direct = Some(sunrise_client::health::EnchantmentActor {
            id: shot.id as i32,
            kind: shot.entity_type().into(),
            motion: None,
        });
        attack.projectile_kind = Self::projectile_direct_entity(&shot.kind);
        attack.projectile_type = Some(shot.entity_type());
    }
    /// Classify a projectile's vanilla `DamageSource.getDirectEntity()` family.
    pub(in crate::game) fn projectile_direct_entity(
        kind: &ProjectileKind,
    ) -> sunrise_client::health::DirectEntity {
        match kind {
            ProjectileKind::Arrow { .. } | ProjectileKind::Trident => {
                sunrise_client::health::DirectEntity::Arrow
            }
            ProjectileKind::LargeFireball { .. } => {
                sunrise_client::health::DirectEntity::LargeFireball
            }
            _ => sunrise_client::health::DirectEntity::Projectile,
        }
    }
    /// Preserve a projectile weapon for armor effectiveness and the hooks that expose it.
    pub(in crate::game) fn set_projectile_weapon(
        &self,
        attack: &mut Attack,
        weapon: Option<&ItemStack>,
    ) {
        if mod_api::__private::listened(mod_api::HookId::LivingIncomingDamage)
            || mod_api::__private::listened(mod_api::HookId::LivingDeath)
            || mod_api::__private::listened(mod_api::HookId::LootDrops)
            || weapon.is_some_and(|stack| {
                world_stations!(self)
                    .enchantments
                    .has_component(stack, "minecraft:armor_effectiveness")
            })
        {
            attack.projectile_weapon = weapon.cloned();
        }
    }
    /// Hurt and settle a living target before the projectile resumes its on-hit work.
    /// The weapon is committed only when hurtServer accepts this projectile's source.
    pub(in crate::game) fn damage_projectile_with_weapon(
        &mut self,
        projectile: &Projectile,
        id: u32,
        amount: f32,
        source: Damage,
        aim: Option<sunrise_client::mobs::Aim>,
    ) -> bool {
        if self.combat.attack().projectile != Some(projectile.id as i32) {
            let attack = self.projectile_attack(projectile);
            return self.with_attack(attack, |game| {
                game.damage_projectile_with_weapon(projectile, id, amount, source, aim)
            });
        }
        let Some(index) = self
            .mob_index
            .find(&self.mobs, id)
            .filter(|&i| !self.mobs[i].removed)
        else {
            return false;
        };
        let plan = self.prepare_knockback_credit(id as i32, source.attacker, self.combat.attack());
        // The projectile's shooter, the hit's causing entity.
        let shooter = self.combat.attack().attacker;
        let decision = crate::game::entity::causer_decision!(self, shooter);
        let mut credit = plan.lend(&mut self.last_hurt_by_player);
        let (victim, lend) =
            crate::game::entity::victim_and_causer!(self, index, shooter, decision);
        let hurt = victim.damage_projectile(
            crate::game::entity::damage_level_of!(self, self.combat.attack(), &mut credit, lend),
            amount,
            source,
            aim,
            [projectile.velocity.x, projectile.velocity.z],
        );
        let Some((index, hurt)) = self.complete_mob_hurt(index, hurt) else {
            return false;
        };
        self.finish_mob_damage_at(index);
        self.settle_creaking_hurt(id);
        hurt
    }
    /// Evaluate weapon `minecraft:damage` modifier for `shot` without drawing crit RNG.
    pub(in crate::game) fn shot_damage_base(
        &mut self,
        shot: &Projectile,
        victim: Option<&str>,
        who: Who,
    ) -> f64 {
        let trident = matches!(shot.kind, ProjectileKind::Trident);
        let base_f64 = match shot.kind {
            ProjectileKind::Arrow { base_damage, .. } => base_damage,
            _ => 8.0,
        };
        let weapon = shot.arrow.as_ref().and_then(|a| {
            if trident {
                Some(&a.pickup_item)
            } else {
                a.fired_from_weapon.as_ref()
            }
        });
        let Some(weapon) = weapon else {
            return base_f64;
        };
        let player = self.own_entity_id() as u32;
        let attack = self.projectile_attack(shot);
        let causing = if attack.attacker != 0 {
            attack.attacker
        } else {
            shot.id as i32
        };
        let source = attack.player_source(
            if trident {
                "minecraft:trident"
            } else {
                "minecraft:arrow"
            },
            causing,
        );
        let mut ctx = attack.enchantment_context(source, victim.unwrap_or(""));
        ctx.this = victim;
        let victim = match who {
            Who::Player => player,
            Who::Mob(id) => id,
        };
        let victim = if victim >= crate::game::dragon::SPECIAL {
            self.dragon.as_ref().map_or(victim, |dragon| dragon.id)
        } else {
            victim
        };
        f64::from(self.evaluate_weapon_damage(weapon, Some(victim), ctx, base_f64 as f32))
    }
    /// What an arrow or a thrown trident hits an entity of type `victim`
    /// for (`AbstractArrow.onHitEntity`, `ThrownTrident.onHitEntity`): a
    /// trident's 8 through `EnchantmentHelper.modifyDamage` of the trident
    /// (Impaling's `minecraft:damage` on `#sensitive_to_impaling`).
    pub(in crate::game) fn shot_damage(
        &mut self,
        shot: &mut Projectile,
        victim: Option<&str>,
        who: Who,
    ) -> f32 {
        let base = self.shot_damage_base(shot, victim, who);
        if matches!(shot.kind, ProjectileKind::Trident) {
            base as f32
        } else {
            shot.arrow_damage_with_base(base) + shot.bonus.damage
        }
    }
    pub(super) fn drop_death_loot(
        &mut self,
        m: &mut Mob,
        player_uuid: Option<u128>,
        player_killed: bool,
        killer: sunrise_world::loot::LootEntity,
        consumed: bool,
        pos: [f64; 3],
        source: Option<(&'static str, u32)>,
    ) {
        let orbs = self.drop_death_loot_items(
            m,
            player_uuid,
            player_killed,
            killer,
            consumed,
            pos,
            source,
        );
        if let Some(amount) = orbs {
            self.spawn_orbs(pos, amount as u32);
        }
    }

    /// `EnchantmentHelper.processMobExperience`: the killer's equipment, the killed mob's random.
    pub(super) fn mob_kill_experience(&mut self, killed: &mut Mob, amount: i32) -> i32 {
        let killer = self.mob_damage_source(killed).1;
        let ctx = sunrise_items::enchant::EffectContext {
            this: Some(&killed.kind.id),
            this_motion: Some((killed.fall_distance, killed.extra.fall_flying)),
            ..Default::default()
        };
        self.kill_experience(killer, ctx, &mut killed.random, amount)
    }

    pub(in crate::game) fn kill_experience(
        &mut self,
        killer: u32,
        ctx: sunrise_items::enchant::EffectContext<'_>,
        killed_random: &mut sunrise_core::random::LegacyRandom,
        amount: i32,
    ) -> i32 {
        use crate::game::{commands::Target, lan_actions::CombatPlayer};
        use sunrise_client::mobs::finalize::EquipmentRead;

        if killer == 0 {
            return amount;
        }
        let (target, displaced) = match self.combat_player(killer) {
            Some(CombatPlayer::InPlace) => (Some(Target::Player), None),
            Some(CombatPlayer::Parked(guest)) => (Some(Target::Guest(guest.conn)), None),
            Some(CombatPlayer::Displaced(depth, _)) => (None, Some(depth)),
            None => (self.level_entity(killer as i32), None),
        };
        if target.is_none() && displaced.is_none() {
            return amount;
        }
        const COMPONENT: &str = "minecraft:mob_experience";
        let enchantments = self.stations().enchantments.clone();
        let mut value = amount as f32;
        for slot in mod_api::EquipmentSlot::ALL {
            let stack = if let Some(depth) = displaced {
                let frame = &self.lan_displaced_players[depth];
                self.command_guest(frame.record_conn).and_then(|guest| {
                    crate::game::native_living_refs::player_slot(
                        &guest.inventory,
                        frame.selected,
                        &guest.armor,
                        &guest.offhand,
                        &guest.equipment,
                        slot,
                    )
                    .map(EquipmentRead::resident)
                })
            } else {
                target.and_then(|target| self.equipment_stack_for(target, slot))
            };
            let Some(stack) = stack else { continue };
            let stack = stack.project();
            let slot = sunrise_items::equipment::SLOT_NAMES[slot as usize];
            let mut levels = stack.all_enchantments().into_owned();
            levels.retain(|(id, _)| {
                enchantments.get(id).is_some_and(|enchantment| {
                    enchantment.effects.get(COMPONENT).is_some() && enchantment.matches_slot(slot)
                })
            });
            if levels.is_empty() {
                continue;
            }
            value = enchantments.modify_entity_value(
                &levels,
                COMPONENT,
                &ctx,
                &mut self.world.random,
                killed_random,
                value,
            );
        }
        value as i32
    }

    /// `LivingEntity.dropExperience` for a dying mob: the reward (drawn from
    /// the mob's random after its loot rolls), heard by the mods
    /// (`living_experience_drop`). The orbs to spawn; none when the mob drops
    /// no experience or a sculk catalyst took it.
    fn death_experience(
        &mut self,
        m: &mut Mob,
        player_uuid: Option<u128>,
        player_killed: bool,
        consumed: bool,
    ) -> Option<i32> {
        let mob_drops = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::MOB_DROPS);
        let experience =
            sunrise_client::mobs::drops::experience_reward(m, player_killed, mob_drops);
        let reward = experience.filter(|_| !consumed)?;
        let reward = self.mob_kill_experience(m, reward);
        let kind = crate::game::native_hooks::mob_kind(m);
        Some(self.living_experience_drop(m.id, &kind, player_uuid, reward))
    }

    /// The snapshot-dependent item half of a death. As NeoForge's
    /// `dropAllDeathLoot`, the experience is heard (`death_experience`) after
    /// the loot rolls and before the mods hear the item list
    /// (`living_drops`); the item entities still join the level before the
    /// orbs, which the caller spawns from the returned amount.
    fn drop_death_loot_items(
        &mut self,
        m: &mut Mob,
        player_uuid: Option<u128>,
        player_killed: bool,
        killer: sunrise_world::loot::LootEntity,
        consumed: bool,
        pos: [f64; 3],
        source: Option<(&'static str, u32)>,
    ) -> Option<i32> {
        use sunrise_client::mobs::{Who, drops};
        use sunrise_world::loot::LootEntity;
        let accepted = m
            .accepted_hit
            .take()
            .filter(|hit| hit.belongs_to(m.last_damage));
        if let Some(body) = self.mobs.iter_mut().find(|body| body.id == m.id) {
            body.accepted_hit = None;
        }
        let weapon = accepted.as_ref().and_then(|hit| hit.weapon.as_ref());
        let weapon = weapon.and_then(|stack| stack.try_into_api().ok());
        let Some(loot) = self.server_loot.get() else {
            return self.death_experience(m, player_uuid, player_killed, consumed);
        };
        let player = || killer.clone();
        let damage = m.last_damage.map(|(d, _)| {
            let source = match d.attacker {
                Some(Who::Player) => Some(player()),
                Some(Who::Mob(id)) => self
                    .mobs
                    .iter()
                    .any(|mob| mob.id == id)
                    .then(|| self.target_loot_entity(crate::game::commands::Target::Mob(id)))
                    // A charged creeper's blast kills after the creeper left the list.
                    .or_else(|| {
                        self.combat
                            .charged_exploders
                            .contains(&id)
                            .then(|| LootEntity {
                                powered: true,
                                ..LootEntity::new("minecraft:creeper")
                            })
                    }),
                None => None,
            };
            loot_damage(d, source)
        });
        let vehicle = m
            .extra
            .mount
            .and_then(|id| self.mobs.iter().find(|o| o.id == id))
            .map(|o| LootEntity::new(&o.kind.id));
        let kind = crate::game::native_hooks::mob_kind(m);
        let death_variants = matches!(m.species(), Species::Chicken | Species::Frog)
            .then(|| self.world_variants().clone());
        let death = drops::Death {
            this_nbt: if loot.copies_entity_nbt() {
                self.command_target_nbt(crate::game::commands::Target::Mob(m.id))
                    .map(std::sync::Arc::new)
            } else {
                None
            },
            loot: &loot,
            damage,
            player: player_uuid.is_some().then(player),
            player_killed,
            luck: 0.0,
            vehicle,
            mob_drops: self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::MOB_DROPS),
            world_seed: self.level.seed,
            variants: death_variants.as_ref(),
        };
        let raider = sunrise_client::mobs::raider::is_raider(m.species());
        let to_stack = |stack: &sunrise_world::loot::LootStack| {
            if raider && stack.item == sunrise_client::mobs::raider::BANNER {
                sunrise_client::mobs::raider::ominous_banner()
            } else {
                from_loot(stack)
            }
        };
        // LootContextParams.ORIGIN of a death: the mob's position.
        let place = self.loot_place_at([m.position.x, m.position.y, m.position.z], None);
        let hurt = accepted.as_ref().map(|hit| hit.source()).or_else(|| {
            source.map(|(damage_type, causing)| {
                let mut hit = death_hurt_source(damage_type, causing);
                hit.position = m.last_damage.and_then(|(damage, _)| damage.from);
                hit
            })
        });
        let enchanted_loot = hurt
            .as_ref()
            .filter(|_| mod_api::__private::listened(mod_api::HookId::EnchantedEntityLoot))
            .zip(self.effect_living_ref(Some(m.id as i32)));
        let (items, skulls) = sunrise_world::loot::with_native_loot_place(place, || {
            let rolled = match enchanted_loot {
                Some((source, entity)) => {
                    let mut rolls =
                        crate::game::loot_rolls::ListenedRolls::new(self, entity, source);
                    drops::roll_death(m, &death, &mut rolls)
                }
                None => {
                    let mut rolls = drops::PlainRolls {
                        sequences: &mut self.rand_sequences,
                        level_random: &mut self.world.random,
                    };
                    drops::roll_death(m, &death, &mut rolls)
                }
            };
            let Some(hurt) = &hurt else {
                return drops::finish_death(m, &death, rolled, None);
            };
            // GetEquipmentDropChanceEvent, asked for each slot just before its roll.
            let id = m.id;
            let mut choose = |choice: drops::EquipmentChoice<'_>| {
                let stack = match choice.stack {
                    drops::Drop::Item(stack) => to_stack(stack),
                    drops::Drop::Stack(stack) => stack.clone(),
                    _ => return choice.chance,
                };
                let mut chance = choice.chance;
                let Ok(stack) = stack.try_into_api() else {
                    return chance;
                };
                self.native_get_equipment_drop_chance(
                    mod_api::__private::GetEquipmentDropChanceCall {
                        entity: id,
                        kind: &kind,
                        slot: EquipmentSlot::ALL[choice.slot],
                        source: hurt,
                        recently_hit: player_killed,
                        stack: &stack,
                        original_chance: choice.original,
                        preserved: choice.preserved,
                        chance: &mut chance,
                    },
                );
                chance
            };
            drops::finish_death(m, &death, rolled, Some(&mut choose))
        });
        if skulls && let Some(Who::Mob(id)) = m.last_damage.and_then(|(d, _)| d.attacker) {
            // Creeper.droppedSkulls: one head per charged explosion.
            self.combat.charged_exploders.retain(|c| *c != id);
            if let Some(creeper) = self.mobs.iter_mut().find(|o| o.id == id) {
                creeper.extra.charged = false;
            }
        }
        // The item drops come before the carried block and the orbs, so the
        // mods get them all at once and they still spawn in that order.
        let mut stacks = Vec::new();
        let mut rest = Vec::new();
        for d in items {
            match d {
                drops::Drop::Item(stack) => {
                    stacks.push(to_stack(&stack));
                }
                drops::Drop::Stack(stack) => {
                    stacks.push(stack);
                }
                other => rest.push(other),
            }
        }
        let table = drops::death_loot_table(m, death.mob_drops).filter(|t| loot.has_table(t));
        let context = LootDrops::from(mod_api::__private::built::LootDrops {
            damage_source: hurt,
            weapon: weapon.as_ref(),
            table: table.as_deref().unwrap_or_default(),
            dimension: self.dim.id(),
            origin: [m.position.x, m.position.y, m.position.z],
            block: None,
            entity: Some((m.id, &kind)),
            player: player_uuid,
            tool: None,
            explosion: None,
            used: false,
        });
        self.native_loot_drops(&context, &mut stacks);

        // Stack-only modifiers may replace or reorder table results. Custom
        // item-entity state stays on complete records outside that list.
        let mut stacks: Vec<(ItemStack, [f64; 3], i32)> = stacks
            .into_iter()
            .filter(|stack| stack.count > 0 && &*stack.item != "minecraft:air")
            .map(|stack| (stack, pos, 0))
            .collect();
        for d in rest {
            match d {
                drops::Drop::CarriedBlock(state) => {
                    // Enderman.dropCustomDeathLoot: spawnAtLocation for each
                    // of the carried block's drops using `enderman_loot_drop`.
                    use sunrise_client::mobs::enderman;
                    let at = [m.position.x, m.position.y, m.position.z];
                    let special = self.local_difficulty(at).special();
                    let tool = enderman::carried_block_tool(
                        &world_stations!(self).enchantments,
                        world_items!(self),
                        &mut m.random,
                        special,
                    );
                    let drops = self.roll_block_loot(
                        at.map(|c| c.floor() as i32),
                        state,
                        Some(m.id),
                        enderman::carried_block_roll(&tool),
                    );
                    stacks.extend(
                        drops
                            .into_iter()
                            .filter(enderman::carried_block_drop_kept)
                            .filter(|stack| stack.count > 0 && &*stack.item != "minecraft:air")
                            .map(|stack| (stack, pos, 0)),
                    );
                }
                drops::Drop::Aged(stack, age) => {
                    let stack = to_stack(&stack);
                    if stack.count > 0 && &*stack.item != "minecraft:air" {
                        stacks.push((stack, pos, age));
                    }
                }
                drops::Drop::Item(_) | drops::Drop::Stack(_) | drops::Drop::Experience(_) => {}
            }
        }
        if m.species() == Species::CopperGolem {
            // CopperGolem.dropEquipment: the preserved equipment is part of
            // dropAllDeathLoot, so the mods hear it with the rest. It is
            // `spawnAtLocation` with no offset: at the golem's feet.
            let feet = [m.position.x, m.position.y, m.position.z];
            stacks.extend(
                Self::golem_preserved_equipment(m)
                    .into_iter()
                    .map(|stack| (stack, feet, 0)),
            );
        }
        // Every death drop is its own ItemEntity, and the drop entity's own
        // random (not the level's) gives it the constructor's horizontal push
        // (its motion otherwise stays [0, 0.2, 0] and the drops land stacked
        // at the mob's centre, without disturbing the level random that the
        // orbs' merge ids and block ticks read next).
        let dying = DyingDrops {
            entity: m.id,
            kind: &kind,
            player: None,
            source: hurt,
            recently_hit: player_killed,
        };
        let orbs = self.death_experience(m, player_uuid, player_killed, consumed);
        self.drop_death_items(&dying, stacks, 10, |_| None);
        orbs
    }
}

/// The projectiles that name themselves as the causing entity when they have no owner; other
/// ownerless projectiles (TNT, potions, snowballs, rockets...) name none, so their blow is not
/// direct.
const OWN_CAUSE: [&str; 5] = [
    "minecraft:arrow",
    "minecraft:spectral_arrow",
    "minecraft:trident",
    "minecraft:fireball",
    "minecraft:small_fireball",
];

/// A dying mob's last damage as its loot sees it (`DamageSource`): the type,
/// the attacker `source` (`getEntity`) and the entity that dealt the blow
/// (`getDirectEntity`), the attacker itself unless the damage recorded
/// another, such as a thrown trident or a splash potion.
fn loot_damage(
    d: Damage,
    source: Option<sunrise_world::loot::LootEntity>,
) -> sunrise_world::loot::LootDamage {
    // The entity that landed the blow when it was not the attacker, as the hit recorded it. A
    // blow with no recorded one (a `/damage` of an arrow's type) has none: the damage source's
    // direct entity is its causing entity, or no entity at all.
    let projectile = d.direct.map(|kind| sunrise_world::loot::LootEntity {
        kind: kind.into(),
        ..Default::default()
    });
    // An arrow, a thrown trident or a fireball with no owner is its own causing entity
    // (`damageSources().arrow(this, owner == null ? this : owner)`, and the trident's and
    // fireballs' alike): both `source` and `direct`, so the blow is direct.
    let own_cause = d.attacker.is_none() && d.direct.is_some_and(|kind| OWN_CAUSE.contains(&kind));
    let source = if own_cause {
        projectile.clone()
    } else {
        source
    };
    let is_direct = projectile.is_none() || own_cause;
    let direct = projectile.or_else(|| source.clone());
    sunrise_world::loot::LootDamage {
        kind: source_damage_type(d).into(),
        source,
        direct,
        is_direct,
    }
}

#[cfg(test)]
#[path = "death_weapon_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "death_weapon_review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "death_loot_direct_tests.rs"]
mod direct_tests;
