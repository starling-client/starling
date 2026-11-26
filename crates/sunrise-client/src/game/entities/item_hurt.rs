//! Shared server damage for dropped items.
use super::{Game, commands::Target};
use sunrise_client::{entities::ItemEntity, health::HurtSource, items::ItemStack, mobs::Who};
use sunrise_core::geometry::Vec3;
use sunrise_world::damage_source::DamageTypeId;

impl Game<'_> {
    /// `ItemEntity.hurtServer` over a caller-owned item row: runs invulnerability,
    /// mob-griefing, fall/fire/stack-immunity checks, float health subtraction,
    /// `entity_damage` game event, and container/bundle content drops on death.
    pub(in crate::game) fn hurt_lent_item_entity(
        &mut self,
        item: &mut sunrise_client::entities::ItemEntity,
        amount: f32,
        damage_type: &str,
        direct: Option<Target>,
        causing: Option<Target>,
        position: Option<[f64; 3]>,
    ) -> bool {
        if item.health <= 0 || !amount.is_finite() || amount < 0.0 {
            return false;
        }
        let id = item.id;
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
            direct_type: None,
        };
        if self.item_damage_kind(id, source, causing).is_none()
            || !self.item_stack_takes_damage(&item.stack, damage_type)
        {
            return false;
        }
        mark_item_hurt(item, amount);
        let at = item.position;
        self.post_item_damage(at, causing);
        if item.health <= 0 {
            if mod_api::__private::item_listens(mod_api::HookId::ItemDestroyed, &item.stack.item) {
                self.item_destroyed_damage = Some((item.id, damage_type.to_owned()));
            } else {
                let contents = item.hurt(self.res.items(), 0).unwrap_or_default();
                self.spawn_destroyed_contents(
                    contents.into_iter().map(|stack| (stack, at)).collect(),
                );
            }
        }
        true
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

    /// The dropped item's `ItemStack.canBeHurtBy` for the source that hit it,
    /// including the item's default `minecraft:damage_resistant` component.
    /// Fire-tagged sources do not also ask `minecraft:in_fire`.
    fn item_stack_takes_damage(&self, stack: &ItemStack, damage_type: &str) -> bool {
        self.stack_can_be_hurt_by(stack, damage_type) != Some(false)
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
        let mut items = self.dropped.lift();
        let hurt = self.hurt_lent_item_entity(
            &mut items[index],
            amount,
            damage_type,
            direct,
            causing,
            position,
        );
        if items[index].health <= 0 {
            let at = items[index].position;
            let deferred = self
                .item_destroyed_damage
                .take_if(|(destroyed_id, _)| *destroyed_id == id)
                .map(|(_, ty)| ty);
            if let Some(ty) = deferred {
                self.put_back_dropped(items);
                let contents = if let Some(k) = self.dropped.iter().position(|item| item.id == id) {
                    self.destroy_item_entity_drops(k, &ty)
                } else {
                    Vec::new()
                };
                if let Some(k) = self.dropped.iter().position(|item| item.id == id) {
                    self.dropped.remove(k);
                }
                self.spawn_destroyed_contents(
                    contents.into_iter().map(|stack| (stack, at)).collect(),
                );
                return hurt;
            }
            items.remove(index);
        }
        self.put_back_dropped(items);
        hurt
    }
}

fn mark_item_hurt(item: &mut ItemEntity, amount: f32) {
    // Java subtracts in float before truncating to int, including fractions
    // and accepted zero-damage hits.
    item.health = (item.health as f32 - amount) as i32;
    item.needs_sync = true;
}
