//! Shared server damage for dropped items.
use super::{Game, commands::Target};
use sunrise_client::{entities::ItemEntity, health::HurtSource, items::ItemStack, mobs::Who};
use sunrise_core::geometry::Vec3;
use sunrise_world::damage_source::DamageTypeId;

impl Game<'_> {
    /// `ItemEntity.hurtServer`: the item branch reached by `DamageCommand.damage`.
    pub(in crate::game) fn hurt_item_entity(
        &mut self,
        id: u32,
        amount: f32,
        damage_type: &str,
        direct: Option<Target>,
        causing: Option<Target>,
        position: Option<[f64; 3]>,
    ) -> bool {
        let Some(index) = self.dropped.iter().position(|item| item.id == id) else {
            return false;
        };
        let entity_id = |target| match target {
            Some(Target::Player) => self.own_entity_id(),
            Some(Target::Guest(conn)) => self.command_guest(conn).map_or(0, |g| g.entity_id),
            Some(target) => {
                crate::game::native_entities::target_id(target).map_or(0, |id| id as i32)
            }
            None => 0,
        };
        let causing_id = entity_id(causing);
        let source = HurtSource {
            damage_type,
            causing: causing_id,
            direct: entity_id(direct),
            position,
        };
        let Some(kind) = self.item_damage_kind(id, source, causing) else {
            return false;
        };
        if !self.item_stack_takes_damage(&self.dropped[index].stack, kind, damage_type) {
            return false;
        }
        let item = &mut self.dropped[index];
        mark_item_hurt(item, amount);
        let at = item.position;
        self.post_item_damage(at, causing);
        if let Some(contents) = self.dropped[index].hurt(0) {
            self.dropped.remove(index);
            self.spawn_destroyed_contents(contents.into_iter().map(|stack| (stack, at)).collect());
        }
        true
    }

    /// Block contact damage during the tick, while the item belongs to its mover.
    pub(super) fn hurt_moving_item(
        &mut self,
        item: &mut ItemEntity,
        damage_type: &str,
        amount: f32,
    ) {
        let source = HurtSource {
            damage_type,
            causing: 0,
            direct: 0,
            position: None,
        };
        let Some(kind) = self.item_damage_kind(item.id, source, None) else {
            return;
        };
        if !self.item_stack_takes_damage(&item.stack, kind, source.damage_type) {
            return;
        }
        mark_item_hurt(item, amount);
        let at = item.position;
        self.post_item_damage(at, None);
        if let Some(contents) = item.hurt(0) {
            self.spawn_destroyed_contents(contents.into_iter().map(|stack| (stack, at)).collect());
        }
    }

    fn item_damage_kind(
        &mut self,
        id: u32,
        source: HurtSource<'_>,
        causing: Option<Target>,
    ) -> Option<DamageTypeId> {
        let damage_type = source.damage_type;
        let kind = self.health_resources.damage.id(damage_type)?;
        let vanilla = self.resident_item_or_orb_immune(
            id,
            "minecraft:item",
            kind,
            self.creative_source_of(causing),
        );
        if crate::game::entity::damage_level_of!(self, self.combat.attack(), no_push, None)
            .invulnerability_check(
                sunrise_client::health::Hurting::Entity(id as i32, "minecraft:item", source),
                vanilla,
            )
            || !self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::MOB_GRIEFING)
                && matches!(causing, Some(Target::Mob(_)))
        {
            return None;
        }
        Some(kind)
    }

    fn item_stack_takes_damage(
        &self,
        stack: &ItemStack,
        kind: DamageTypeId,
        damage_type: &str,
    ) -> bool {
        self.stack_can_be_hurt_by(stack, damage_type) != Some(false)
            && (!self
                .health_resources
                .damage
                .in_tag(kind, "minecraft:is_fire")
                || self.stack_can_be_hurt_by(stack, "minecraft:in_fire") != Some(false))
    }

    fn post_item_damage(&mut self, at: Vec3, causing: Option<Target>) {
        self.sculk_prepare();
        let who = match causing {
            Some(Target::Player) => Some(Who::Player),
            Some(Target::Mob(id)) => Some(Who::Mob(id)),
            _ => None,
        };
        let context = self.who_context(who, None);
        self.post_game_event("minecraft:entity_damage", [at.x, at.y, at.z], context);
    }
}

fn mark_item_hurt(item: &mut ItemEntity, amount: f32) {
    // Java subtracts in float before truncating to int, including fractions
    // and accepted zero-damage hits.
    item.health = (item.health as f32 - amount) as i32;
    item.needs_sync = true;
}
