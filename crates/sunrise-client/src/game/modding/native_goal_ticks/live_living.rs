//! Reads from the actual mob loaned to the active goal, or a disjoint live mob.
use super::super::native_living_refs::{hand_slot, lend_equipment};
use super::{GoalCx, Resolved};
use crate::mobs::Mob;
use mod_api::{ActiveEffect, EffectKey, EquipmentSlot, Hand, ItemStackView, LivingRef, Pose};
use sunrise_client::{
    items::EntityTypeTags,
    mobs::{finalize, villager},
};

impl GoalCx<'_, '_> {
    fn live_living_mob(&self, living: LivingRef) -> Option<&Mob> {
        match self.resolve(living.entity())? {
            Resolved::Own => Some(self.m),
            Resolved::Live(mob) => Some(mob),
            _ => None,
        }
    }

    pub(super) fn r_living_effect(
        &self,
        living: LivingRef,
        effect: EffectKey,
    ) -> Option<ActiveEffect> {
        let index = mod_api::__private::effect_index(effect)?;
        let mob = self.live_living_mob(living)?;
        let e = mob
            .extra
            .effects
            .iter()
            .find(|e| e.registry_index == Some(index))?;
        Some(super::super::native_living_effects::mob_effect(mob, e))
    }

    pub(super) fn r_living_effects(&self, living: LivingRef) -> Option<Vec<ActiveEffect>> {
        let mob = self.live_living_mob(living)?;
        Some(
            mob.extra
                .effects
                .iter()
                .map(|e| super::super::native_living_effects::mob_effect(mob, e))
                .collect(),
        )
    }

    pub(super) fn r_living_potion_affected(&self, living: LivingRef, effect: EffectKey) -> bool {
        let Some(index) = mod_api::__private::effect_index(effect) else {
            return false;
        };
        self.live_living_mob(living).is_some_and(|mob| {
            mob.extra
                .effects
                .iter()
                .any(|e| e.registry_index == Some(index))
        })
    }

    pub(super) fn r_living_gliding_ticks(&self, living: LivingRef) -> Option<i32> {
        Some(self.live_living_mob(living)?.extra.fall_fly_ticks)
    }

    pub(super) fn r_living_is_gliding(&self, living: LivingRef) -> Option<bool> {
        Some(self.live_living_mob(living)?.extra.fall_flying)
    }

    pub(super) fn r_living_is_sleeping(&self, living: LivingRef) -> Option<bool> {
        Some(villager::sleeping(self.live_living_mob(living)?))
    }

    pub(super) fn r_living_sleeping_pos(&self, living: LivingRef) -> Option<[i32; 3]> {
        villager::sleeping_pos(self.live_living_mob(living)?)
    }

    pub(super) fn r_living_is_jumping(&self, living: LivingRef) -> Option<bool> {
        self.live_living_mob(living).map(|_| false)
    }

    pub(super) fn r_living_baby(&self, living: LivingRef) -> Option<bool> {
        Some(self.live_living_mob(living)?.extra.age < 0)
    }

    pub(super) fn r_living_equipment(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        lend_equipment(
            finalize::equipment_slot(self.live_living_mob(living)?, slot)?,
            read,
        )
    }

    pub(super) fn r_living_hand_item(
        &self,
        living: LivingRef,
        hand: Hand,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        self.r_living_equipment(living, hand_slot(hand), read)
    }

    pub(super) fn r_living_is_tamed(&self, living: LivingRef) -> Option<bool> {
        let mob = self.live_living_mob(living)?;
        mob.is_tamable().then_some(mob.extra.tame)
    }

    pub(super) fn r_living_owner(&self, living: LivingRef) -> Option<u128> {
        let mob = self.live_living_mob(living)?;
        if !mob.is_tamable() {
            return None;
        }
        Some(sunrise_client::lan::playerdata::ints_uuid(mob.extra.owner?))
    }

    pub(super) fn r_living_dimensions_for_pose(
        &self,
        living: LivingRef,
        pose: Pose,
    ) -> Option<[f64; 3]> {
        let mob = self.live_living_mob(living)?;
        if pose == Pose::Sleeping {
            return Some([f64::from(0.2_f32); 3]);
        }
        let (w, h, e) = mob.dims_in_pose(pose);
        Some([w, h, e])
    }

    pub(super) fn r_living_can_freeze(&self, living: LivingRef) -> Option<bool> {
        Some(sunrise_client::mobs::ai::can_freeze(
            self.live_living_mob(living)?,
            self.w,
        ))
    }

    pub(super) fn r_living_can_breathe_underwater(&self, living: LivingRef) -> Option<bool> {
        let mob = self.live_living_mob(living)?;
        Some(
            self.w
                .entity_tag_bit(&mob.kind, EntityTypeTags::CAN_BREATHE_UNDER_WATER),
        )
    }
    pub(super) fn r_living_has_line_of_sight(
        &self,
        living: LivingRef,
        target: mod_api::EntityRef,
    ) -> Option<bool> {
        let read_eye = |entity| {
            if self.is_game_entity(entity) {
                mod_api::__private::backend_read(self.host, |backend| {
                    backend.entity_eye_position(entity)
                })
                .flatten()
            } else {
                self.r_entity_eye_position(entity)
            }
        };
        let from_entity = living.entity();
        let feet = if self.is_game_entity(from_entity) {
            mod_api::__private::backend_read(self.host, |backend| {
                backend.entity_position(from_entity)
            })
            .flatten()?
        } else {
            self.r_entity_position(from_entity)?
        };
        let from = read_eye(from_entity)?;
        let to = read_eye(target)?;
        Some(self.w.line_of_sight_for(
            Some(feet[1]),
            sunrise_core::geometry::Vec3::new(from[0], from[1], from[2]),
            sunrise_core::geometry::Vec3::new(to[0], to[1], to[2]),
        ))
    }

    pub(super) fn r_living_in_melee_range(
        &self,
        living: LivingRef,
        target: mod_api::EntityRef,
    ) -> Option<bool> {
        let mob = self.live_living_mob(living)?;
        let [min, max] = if self.is_game_entity(target) {
            mod_api::__private::backend_read(self.host, |backend| {
                backend.entity_bounding_box(target)
            })
            .flatten()?
        } else {
            self.r_entity_bounding_box(target)?
        };
        let bounds = sunrise_core::geometry::Aabb::new(
            sunrise_core::geometry::Vec3::new(min[0], min[1], min[2]),
            sunrise_core::geometry::Vec3::new(max[0], max[1], max[2]),
        );
        Some(mob.melee_reaches(self.w, bounds))
    }

    pub(super) fn r_living_bed_orientation(&self, living: LivingRef) -> Option<mod_api::Direction> {
        let pos = self.r_living_sleeping_pos(living)?;
        let name = self.w.block_name(pos[0], pos[1], pos[2]);
        if !self.w.has_tag("minecraft:beds", name) {
            return None;
        }
        match self.w.prop(pos[0], pos[1], pos[2], "facing") {
            Some("north") => Some(mod_api::Direction::North),
            Some("south") => Some(mod_api::Direction::South),
            Some("west") => Some(mod_api::Direction::West),
            Some("east") => Some(mod_api::Direction::East),
            _ => None,
        }
    }
    pub(super) fn r_living_attribute_value(&self, living: LivingRef, id: &str) -> Option<f64> {
        self.live_mob_attribute(
            living,
            id,
            sunrise_world::attribute_map::ValueKind::Computed,
        )
    }

    pub(super) fn r_living_attribute_base(&self, living: LivingRef, id: &str) -> Option<f64> {
        self.live_mob_attribute(living, id, sunrise_world::attribute_map::ValueKind::Base)
    }

    fn live_mob_attribute(
        &self,
        living: LivingRef,
        id: &str,
        kind: sunrise_world::attribute_map::ValueKind,
    ) -> Option<f64> {
        let m = self.live_living_mob(living)?;
        m.extra
            .live
            .read(self.w.attributes, &m.kind.id, id, kind, || {
                sunrise_client::entity_nbt::attribute_entries(m)
                    .into_iter()
                    .map(|(_, entry)| entry)
                    .collect()
            })
    }

    pub(super) fn r_living_attributes(
        &self,
        living: LivingRef,
    ) -> Option<Vec<mod_api::Identifier>> {
        let m = self.live_living_mob(living)?;
        let catalog = self.w.attributes;
        Some(
            catalog
                .attributes()
                .filter(|(id, _)| catalog.default_base(&m.kind.id, *id).is_some())
                .filter_map(|(_, a)| mod_api::Identifier::parse(&a.name).ok())
                .collect(),
        )
    }

    pub(super) fn r_living_attribute_modifiers(
        &self,
        living: LivingRef,
        id: &str,
    ) -> Option<Vec<mod_api::AttributeModifier>> {
        use sunrise_world::attributes::Operation;
        let m = self.live_living_mob(living)?;
        m.extra
            .live
            .modifiers(self.w.attributes, &m.kind.id, id, || {
                sunrise_client::entity_nbt::attribute_entries(m)
                    .into_iter()
                    .map(|(_, entry)| entry)
                    .collect()
            })
            .map(|list| {
                list.iter()
                    .filter_map(|modifier| {
                        Some(mod_api::AttributeModifier::new(
                            mod_api::Identifier::parse(modifier.id()).ok()?,
                            modifier.amount(),
                            match modifier.operation() {
                                Operation::AddValue => mod_api::AttributeOperation::AddValue,
                                Operation::AddMultipliedBase => {
                                    mod_api::AttributeOperation::AddMultipliedBase
                                }
                                Operation::AddMultipliedTotal => {
                                    mod_api::AttributeOperation::AddMultipliedTotal
                                }
                            },
                        ))
                    })
                    .collect()
            })
    }

    pub(super) fn r_living_used_item_hand(&self, living: LivingRef) -> Option<Hand> {
        let mob = self.live_living_mob(living)?;
        sunrise_client::mobs::spear_use::is_using_item(mob).then_some(if mob.extra.using_off_hand {
            Hand::Off
        } else {
            Hand::Main
        })
    }
    pub(super) fn r_living_is_climbing(&self, living: LivingRef) -> Option<bool> {
        let mob = self.live_living_mob(living)?;
        Some(self.w.goal_host?.mob_is_climbing(mob))
    }

    pub(super) fn r_living_last_attacker(&self, living: LivingRef) -> Option<mod_api::EntityRef> {
        let mob = self.live_living_mob(living)?;
        let id = match mob.recent_hurt_by()? {
            crate::mobs::Who::Mob(id) => {
                return self
                    .lookup_entity(id)
                    .or_else(|| self.w.goal_host?.mob_attacker_elsewhere(id));
            }
            crate::mobs::Who::Player => self.w.goal_host?.mob_player_credit(mob)??,
        };
        self.lookup_entity(id)
    }

    pub(super) fn r_living_kill_credit(&self, living: LivingRef) -> Option<mod_api::EntityRef> {
        let mob = self.live_living_mob(living)?;
        if let Some(credit) = self.w.goal_host?.mob_player_credit(mob) {
            return self.lookup_entity(credit?);
        }
        self.r_living_last_attacker(living)
    }
    pub(super) fn r_living_can_use_portal(&self, living: LivingRef) -> Option<bool> {
        Some(
            self.w
                .goal_host?
                .mob_can_use_portal(self.live_living_mob(living)?),
        )
    }
}
