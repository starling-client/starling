//! Observe the complete species invulnerability predicate once per living hit attempt.
use super::*;

impl Mob {
    pub(super) fn remember_accepted_hit(
        &mut self,
        level: &DamageLevel<'_>,
        source: &Damage,
        accepted: bool,
    ) {
        if accepted {
            self.death_canceled = false;
            self.accepted_hit = level.accepted_hit(crate::health::Hurting::Mob(self, source));
        }
    }
    /// WitherBoss refuses its friends after asking isInvulnerableTo, not at the caller.
    pub(super) fn attacker_allows_damage(&self, source: &Damage) -> bool {
        !source
            .attacker_species
            .is_some_and(|attacker| wither::shielded_from(self.species(), attacker))
    }

    pub(super) fn invulnerable_to_attempt(
        &self,
        level: &mut DamageLevel<'_>,
        source: &Damage,
    ) -> bool {
        use sunrise_world::damage_immunity::ImmunityTag;

        let bypasses = source.kind.bypasses_invulnerability();
        let reflected = self.reflected_fireball(source, level.attack());
        let items = level.tags();
        let digging =
            self.species() == Species::Warden && warden::digging_or_emerging(self) && !bypasses;
        // Ghast.isInvulnerableTo can short-circuit the shared living predicate.
        let flagged_ghast = self.species() == Species::Ghast && self.is_invulnerable_to(source);
        let vanilla = digging
            || if reflected || flagged_ghast {
                self.removed || flagged_ghast
            } else {
                crate::health::BodyImmunity {
                    removed: self.removed,
                    invulnerable: self.is_invulnerable(),
                    creative_source: self.creative_attacker,
                    fire_immune: self.fire_immune(),
                    fall_immune: items.is_some_and(|items| {
                        items
                            .enchantments
                            .entity_in_tag(&self.kind.id, "minecraft:fall_damage_immune")
                    }),
                    source_has_tag: |tag| match tag {
                        ImmunityTag::BypassesInvulnerability => bypasses,
                        ImmunityTag::Fire => source.is_fire(),
                        ImmunityTag::Fall => match source.kind {
                            DamageKind::Named(named) => named.has(NamedDamage::IS_FALL),
                            kind => matches!(kind, DamageKind::Fall | DamageKind::Stalagmite),
                        },
                        _ => false,
                    },
                }
                .vanilla()
            };
        if level.invulnerability_check(crate::health::Hurting::Mob(self, source), vanilla) {
            return true;
        }
        // LivingEntity adds active equipment immunity after the base predicate.
        // A ghast skips the living predicate for its reflected fireball.
        !reflected
            && items.is_some_and(|items| {
                let attacker = (source.attacker == Some(Who::Player)).then_some("minecraft:player");
                let is_direct = level.attack().is_direct(source);
                let context = crate::enchant::EffectContext {
                    items: Some(items),
                    damage_type: Some(crate::health::source_damage_type(*source)),
                    is_direct,
                    this: Some(&self.kind.id),
                    attacker,
                    direct_attacker: attacker.filter(|_| is_direct),
                    ..Default::default()
                };
                use sunrise_items::equipment::{BODY, SADDLE, SLOT_NAMES};
                // Mob hands and armor keep their effective levels in Gear;
                // body and saddle use their retained stacks.
                let gear = gear::gear(self);
                let equipment = [
                    gear::MAIN,
                    gear::OFF,
                    gear::FEET,
                    gear::LEGS,
                    gear::CHEST,
                    gear::HEAD,
                ]
                .into_iter()
                .map(|slot| gear.map_or(&[][..], |gear| gear.slots[slot].as_slice()))
                .chain([BODY, SADDLE].map(|slot| {
                    self.extra
                        .equipment
                        .get(slot)
                        .map_or(&[][..], crate::items::ItemStack::enchantments)
                }))
                .zip(SLOT_NAMES);
                let enchantments = &items.enchantments;
                enchantments.is_damage_immune(
                    enchantments.damage_immunities(equipment),
                    &context,
                    level.random(),
                )
            })
    }
}
