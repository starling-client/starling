//! Living entities for native server mods, through a `LivingRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): the equipment
//! of mobs, players and armor stands, throwing a stack, and whether a mob
//! is tamed. Reads return `None`, writes false, for a stale handle or on a
//! client; the exception is the reads marked side `both` (`living`, `with_equipment`,
//! `with_hand_item`, `health`, `max_health`, `attribute_value`, `attribute_base`,
//! `visit_attributes` and `visit_attribute_modifiers`), which on a client answer for the
//! local player from the client's own copy; and `effect`/`visit_effects`. Visitor reads
//! refuse with a typed error rather than returning `None`.
use super::Game;
use super::api_values::{StackReadMetadata, projected_stack, stack_read_metadata};
use super::commands::{Target, items::Owner};
use super::living_attributes::AttributeWrite;
use crate::mobs::Who;
use mod_api::client::first_person::Hand;
use mod_api::{
    AttributeModifier, AttributeModifierView, AttributeOperation, Direction, EntityReadError,
    EntityRef, EquipmentSlot, Identifier, ItemStackView, LivingRef, ModifierLifetime,
};
use std::{ops::ControlFlow, sync::Arc};
use sunrise_client::health::{Attack, DamageLevel, HitEffects, Hurting, Knock, NO_ATTACK};
use sunrise_client::items::{EntityTypeTags, ItemStack};
use sunrise_client::mobs::{
    Mob,
    finalize::{self, EquipmentRead},
};
use sunrise_world::attributes::{Modifier, Operation, entity::Catalog};

/// A player's armour slots (`armor`, head first) by equipment slot.
fn armor_index(slot: EquipmentSlot) -> Option<usize> {
    match slot {
        EquipmentSlot::Head => Some(0),
        EquipmentSlot::Chest => Some(1),
        EquipmentSlot::Legs => Some(2),
        EquipmentSlot::Feet => Some(3),
        _ => None,
    }
}

/// A player's stack in `slot`, from its inventory, armour and off hand.
pub(super) fn player_slot<'a>(
    inventory: &'a [Option<ItemStack>],
    selected: usize,
    armor: &'a [Option<ItemStack>; 4],
    offhand: &'a Option<ItemStack>,
    equipment: &'a sunrise_items::equipment::Equipment,
    slot: EquipmentSlot,
) -> Option<&'a ItemStack> {
    match slot {
        EquipmentSlot::MainHand => inventory.get(selected)?.as_ref(),
        EquipmentSlot::OffHand => offhand.as_ref(),
        EquipmentSlot::Body | EquipmentSlot::Saddle => equipment.get(slot as usize),
        _ => armor[armor_index(slot)?].as_ref(),
    }
}

/// Lend one equipment slot to the API, applying overlays and encoding components only on demand.
pub(super) fn lend_equipment(
    stack: EquipmentRead<'_>,
    read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
) -> Option<()> {
    let metadata = stack_read_metadata(stack.base()).ok()?;
    with_equipment_view(&stack, metadata, |view, _| read(view));
    Some(())
}

/// Use checked slot metadata to lend the same lazy view to readers and component editors.
pub(super) fn with_equipment_view<R>(
    stack: &EquipmentRead<'_>,
    metadata: StackReadMetadata,
    read: impl for<'a> FnOnce(ItemStackView<'a>, &'a dyn Fn(&str) -> Option<mod_api::Tag>) -> R,
) -> R {
    use sunrise_core::nbt::{JavaString, Tag};
    let read_override = |id: &str| {
        // The dye is read on the render path: its typed field and removal marker answer
        // directly, without encoding a patch compound to read one integer back.
        match id {
            "minecraft:dyed_color" => {
                return stack.base().extra.as_ref()?.dyed_color.map(Tag::Int);
            }
            "!minecraft:dyed_color" => {
                return stack.base().extra.as_ref()?.other(id).cloned();
            }
            _ => {}
        }
        let bare = id.strip_prefix('!').unwrap_or(id);
        let Tag::Compound(mut patch) = stack.extra_components_matching(|name| name == bare)? else {
            return None;
        };
        patch.remove(&JavaString::from(id))
    };
    let project =
        |metadata| projected_stack(metadata, stack.extra_components_matching(|_| true)).ok();
    read(
        ItemStackView::resident_with_components(metadata, &project, &read_override),
        &read_override,
    )
}

/// The equipment slot of a hand (`LivingEntity.getItemInHand`/`setItemInHand`).
pub(super) fn hand_slot(hand: Hand) -> EquipmentSlot {
    match hand {
        Hand::Main => EquipmentSlot::MainHand,
        Hand::Off => EquipmentSlot::OffHand,
    }
}

/// The `/item` slot name of an equipment slot.
fn slot_name(slot: EquipmentSlot) -> &'static str {
    match slot {
        EquipmentSlot::MainHand => "weapon.mainhand",
        EquipmentSlot::OffHand => "weapon.offhand",
        EquipmentSlot::Head => "armor.head",
        EquipmentSlot::Chest => "armor.chest",
        EquipmentSlot::Legs => "armor.legs",
        EquipmentSlot::Feet => "armor.feet",
        EquipmentSlot::Body => "armor.body",
        EquipmentSlot::Saddle => "saddle",
    }
}

fn world_equipment_slot(slot: EquipmentSlot) -> sunrise_core::equipment_slot::EquipmentSlot {
    use sunrise_core::equipment_slot::EquipmentSlot as WorldSlot;
    match slot {
        EquipmentSlot::MainHand => WorldSlot::MainHand,
        EquipmentSlot::OffHand => WorldSlot::OffHand,
        EquipmentSlot::Head => WorldSlot::Head,
        EquipmentSlot::Chest => WorldSlot::Chest,
        EquipmentSlot::Legs => WorldSlot::Legs,
        EquipmentSlot::Feet => WorldSlot::Feet,
        EquipmentSlot::Body => WorldSlot::Body,
        EquipmentSlot::Saddle => WorldSlot::Saddle,
    }
}

/// `LivingEntity.isAlive`'s health test: above 0, so a NaN health loaded from
/// a save is not alive either.
fn alive_at(health: f32) -> bool {
    health > 0.0
}

fn equipment_stack(stack: Option<ItemStack>) -> Option<ItemStack> {
    stack.filter(|s| s.count > 0 && &*s.item != "minecraft:air")
}

/// Project each modifier without copying its canonical id out of the live map.
pub(super) fn visit_attribute_modifiers(
    modifiers: &[Modifier],
    visit: &mut dyn FnMut(AttributeModifierView<'_>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    modifiers.iter().try_for_each(|m| {
        visit(mod_api::__private::game::attribute_modifier_view(
            m.id(),
            m.amount(),
            match m.operation() {
                Operation::AddValue => AttributeOperation::AddValue,
                Operation::AddMultipliedBase => AttributeOperation::AddMultipliedBase,
                Operation::AddMultipliedTotal => AttributeOperation::AddMultipliedTotal,
            },
        ))
    })
}

/// Use the mob's saved-attribute seed if its map has not been built yet.
pub(super) fn mob_attribute_modifiers<'a>(
    mob: &'a Mob,
    catalog: &Arc<Catalog>,
    name: &str,
) -> Option<&'a [Modifier]> {
    mob.extra.live.modifiers(catalog, &mob.kind.id, name, || {
        sunrise_client::entity_nbt::attribute_entries(mob)
            .into_iter()
            .map(|(_, entry)| entry)
            .collect()
    })
}

impl Game<'_> {
    /// What a living handle names now: a player, a mob, an armor stand or the dragon.
    pub(super) fn native_living_target(&self, living: LivingRef) -> Option<Target> {
        self.native_entity_target(living.entity())
            .filter(|t| t.is_living())
    }
    /// A direct, non-damaging call to the short `LivingEntity.knockback` entry.
    pub(super) fn living_knockback(
        &mut self,
        living: LivingRef,
        strength: f64,
        [x, z]: [f64; 2],
    ) -> bool {
        if ![strength, x, z].into_iter().all(f64::is_finite) {
            return false;
        }
        let Some(target) = self.native_living_target(living) else {
            return false;
        };
        const SOURCE: mod_api::HurtSource<'static> = mod_api::HurtSource {
            damage_type: "minecraft:generic",
            causing: 0,
            direct: 0,
            position: None,
            direct_type: None,
        };
        let knock = Knock { strength, x, z };
        let id = living.id();
        match target {
            Target::Player | Target::Guest(_) => self
                .player_knockback_from_mod(living, target, knock, SOURCE)
                .is_some(),
            Target::Mob(_) => {
                let Some(i) = self.mob_index.find(&self.mobs, id) else {
                    return false;
                };
                let plan = self.prepare_knockback_credit(id as i32, None, &NO_ATTACK);
                let mut credit = plan.lend(&mut self.last_hurt_by_player);
                let level =
                    crate::game::entity::damage_level_of!(self, &NO_ATTACK, &mut credit, None);
                self.mobs[i].knockback(
                    level,
                    strength,
                    x,
                    z,
                    &sunrise_client::mobs::Damage::of(sunrise_client::mobs::DamageKind::Generic),
                    0.0,
                )
            }
            Target::Stand(id) => self.knock_back_armor_stand(id, knock, SOURCE, &NO_ATTACK),
            Target::Dragon(_) => {
                let resistance = self.dragon_knockback_resistance();
                let plan = self.prepare_knockback_credit(id as i32, None, &NO_ATTACK);
                let mut credit = plan.lend(&mut self.last_hurt_by_player);
                let level =
                    crate::game::entity::damage_level_of!(self, &NO_ATTACK, &mut credit, None);
                self.dragon
                    .as_mut()
                    .is_some_and(|d| d.knockback(level, knock, SOURCE, resistance))
            }
            _ => false,
        }
    }

    /// Lend the victim's own body and level to the player path without switching turns or levels.
    fn player_knockback_from_mod(
        &mut self,
        living: LivingRef,
        target: Target,
        knock: Knock,
        source: mod_api::HurtSource<'_>,
    ) -> Option<()> {
        let id = living.id();
        // The hook's attacker falls back to the victim's `getLastHurtByMob`, resolved in any
        // level; the source stays unattributed and kill credit is planned on its own.
        let remembered = self
            .living_last_attacker(living)
            .map(|attacker| self.last_attacker_attack(attacker.id()));
        let attack = remembered.as_ref().unwrap_or(&NO_ATTACK);
        let hub = &self.lan.as_ref()?.hub;
        let player = match target {
            Target::Player => {
                hub.guest(self.lan_acting?)?;
                self.player.as_ref()?
            }
            Target::Guest(conn) => hub.guest(conn)?.motion.as_deref()?,
            _ => return None,
        };
        let mut random = player.random.clone();
        let plan = self.prepare_knockback_credit(id as i32, None, attack);
        let dimension = self.resident_target_dimension(target).unwrap_or(self.dim);
        let mut credit = plan.lend(&mut self.last_hurt_by_player);
        let world = if dimension == self.dim {
            &mut self.world
        } else {
            &mut self.levels.parked_mut(dimension)?.world
        };
        let mut level = DamageLevel::of(
            world,
            self.pack.assets.registry,
            Some(world_items!(self)),
            Some(&self.pack.block_registries),
            &self.damage_hooks,
        )
        .attacked_by(attack)
        .with_knockback_credit(&mut credit)
        .with_effects(HitEffects::new(
            &mut self.creatures.lan_sounds,
            Some(hub),
            dimension.id(),
            self.native_lifecycle.epoch(),
            None,
        ));
        let velocity = super::creatures::knockback::player_knockback_velocity(
            player,
            &mut random,
            &mut level,
            knock,
            source,
        );
        // The retries drew from the victim's own random, landed or not.
        self.write_player_random(target, random);
        let velocity = velocity?;
        self.write_player_velocity(target, |_| Some(velocity))
            .then_some(())
    }

    /// [`Self::attack_of`] for a remembered attacker, which may be a mob in another loaded
    /// level: its main-hand stack is read there.
    fn last_attacker_attack(&self, attacker: u32) -> Attack {
        let mut attack = self.attack_of(attacker as i32);
        if let Some((_, mob)) = self.mob_in_other_level(attacker)
            && !mod_api::__private::mods().is_empty()
        {
            attack.held[0] = mob.extra.main_hand_stack().cloned();
        }
        attack
    }

    pub(super) fn knock_back_armor_stand(
        &mut self,
        id: u32,
        mut knock: Knock,
        source: mod_api::HurtSource<'_>,
        attack: &Attack,
    ) -> bool {
        let Some(index) = self.armor_stands.iter().position(|stand| stand.id == id) else {
            return false;
        };
        let resistance = self.armor_stands[index]
            .live
            .read(
                &self.motion.attributes,
                "minecraft:armor_stand",
                "minecraft:knockback_resistance",
                sunrise_world::attribute_map::ValueKind::Computed,
                Vec::new,
            )
            .unwrap_or(0.0);
        let plan = self.prepare_knockback_credit(id as i32, None, attack);
        let mut credit = plan.lend(&mut self.last_hurt_by_player);
        let level = crate::game::entity::damage_level_of!(self, attack, &mut credit, None);
        let pushed = Hurting::ArmorStand(id as i32, source);
        if level.knockback_canceled(pushed, &mut knock) {
            return false;
        }
        let stand = &mut self.armor_stands[index];
        knock.apply(
            &mut stand.velocity,
            resistance,
            stand.ground.on_ground,
            level.random(),
        )
    }

    /// The entity as a living entity (vanilla's `LivingEntity`: a mob, a
    /// player, an armor stand or the dragon), or None for another kind or a
    /// stale handle.
    ///
    /// On a client, only the local player is one: the client keeps the others'
    /// equipment, effects and health only as far as it draws them. API 382.
    pub(super) fn living(&self, entity: EntityRef) -> Option<LivingRef> {
        let living = LivingRef::new(entity);
        self.native_living_read_target(living).map(|_| living)
    }
    /// What a living handle names for a read that works on both sides: the
    /// server's entity, or on a client the local player (`Minecraft.player`),
    /// whose state is the client's copy of what the server sent.
    pub(super) fn native_living_read_target(&self, living: LivingRef) -> Option<Target> {
        if self.is_logical_server() {
            return self.native_living_target(living);
        }
        match self.native_client_entity_target(living.entity())? {
            Target::Player => Some(Target::Player),
            _ => None,
        }
    }
    /// The effective value of an attribute on a living entity
    /// (`AttributeMap.getValue`). None for an unknown or absent attribute, an
    /// unavailable entity, a stale handle, or on a client for any entity but
    /// the local player, whose copy of the synced instances it reads. A mob,
    /// an armor stand and the dragon answer from their live attribute map: the
    /// type's defaults, the saved `attributes`, and the modifiers of what they
    /// wear or hold.
    pub(super) fn living_attribute_value(&self, living: LivingRef, id: &str) -> Option<f64> {
        self.native_living_attribute(
            living,
            id,
            sunrise_world::attribute_map::ValueKind::Computed,
        )
    }

    /// The base value of an attribute, before modifiers
    /// (`AttributeMap.getBaseValue`), read as the effective value is.
    pub(super) fn living_attribute_base(&self, living: LivingRef, id: &str) -> Option<f64> {
        self.native_living_attribute(living, id, sunrise_world::attribute_map::ValueKind::Base)
    }

    /// Resolve the visitor's target. Only the local player has these reads on the client.
    pub(super) fn native_living_visit_target(
        &self,
        living: LivingRef,
    ) -> Result<Target, EntityReadError> {
        let target = self.native_entity_visit_target(living.entity())?;
        if !matches!(target, Target::Player) && !self.is_logical_server() {
            return Err(EntityReadError::Unavailable);
        }
        Ok(target)
    }

    /// Lend the entity type's attribute ids in registry order.
    pub(super) fn living_visit_attributes(
        &self,
        living: LivingRef,
        visit: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let entity = match self.native_living_visit_target(living)? {
            Target::Player | Target::Guest(_) => "minecraft:player",
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .ok_or(EntityReadError::Stale)?
                .kind
                .id
                .as_str(),
            Target::Stand(_) => "minecraft:armor_stand",
            Target::Dragon(_) => "minecraft:ender_dragon",
            _ => return Err(EntityReadError::Unavailable),
        };
        Ok(self
            .motion
            .attributes
            .attribute_names_for(entity)
            .try_for_each(visit))
    }

    /// Lend resident modifiers from the same attribute instances the value reads use.
    pub(super) fn living_visit_attribute_modifiers(
        &self,
        living: LivingRef,
        id: &str,
        visit: &mut dyn FnMut(AttributeModifierView<'_>) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let catalog = &self.motion.attributes;
        let modifiers = match self.native_living_visit_target(living)? {
            Target::Player => self
                .player
                .as_ref()
                .ok_or(EntityReadError::Unavailable)?
                .attributes
                .modifiers(id),
            Target::Guest(conn) => self
                .command_guest(conn)
                .and_then(|guest| guest.motion.as_deref())
                .ok_or(EntityReadError::Unavailable)?
                .attributes
                .modifiers(id),
            Target::Mob(mob) => {
                let m = self
                    .mobs
                    .iter()
                    .find(|m| m.id == mob && !m.removed)
                    .ok_or(EntityReadError::Stale)?;
                mob_attribute_modifiers(m, catalog, id)
            }
            Target::Stand(stand) => self
                .armor_stands
                .iter()
                .find(|a| a.id == stand)
                .ok_or(EntityReadError::Stale)?
                .live
                .modifiers(catalog, "minecraft:armor_stand", id, Vec::new),
            Target::Dragon(dragon) => self
                .dragon
                .as_ref()
                .filter(|d| d.id == dragon)
                .ok_or(EntityReadError::Stale)?
                .live
                .modifiers(catalog, "minecraft:ender_dragon", id, Vec::new),
            _ => None,
        }
        .ok_or(EntityReadError::Unavailable)?;
        Ok(visit_attribute_modifiers(modifiers, visit))
    }

    /// Adds an attribute modifier (`AttributeInstance.addTransientModifier`
    /// or `addPermanentModifier`). A permanent one is saved with the entity;
    /// a transient one lasts until it unloads. It goes where `/attribute`
    /// writes (`Game::write_attribute`), so gameplay and the entity's clients
    /// see it. False when the entity already has a modifier with that id on
    /// the attribute (vanilla throws), for an attribute the entity type does
    /// not have, a non-finite amount, the dragon (whose attributes nothing
    /// reads yet), a stale handle, or on a client.
    pub(super) fn living_add_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
    ) -> bool {
        self.native_living_add_modifier(living, attribute, modifier, lifetime, false)
    }

    /// Adds an attribute modifier in place of the one with the same id, if
    /// any (`addOrUpdateTransientModifier`, `addOrReplacePermanentModifier`):
    /// the new one's lifetime wins. False as `add_attribute_modifier` is,
    /// except that a taken id is replaced.
    pub(super) fn living_replace_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
    ) -> bool {
        self.native_living_add_modifier(living, attribute, modifier, lifetime, true)
    }

    /// Removes the modifier with id `id` from the attribute
    /// (`AttributeInstance.removeModifier`), whatever its lifetime. False when
    /// there was none, and in the cases `add_attribute_modifier` is.
    pub(super) fn living_remove_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        id: &Identifier,
    ) -> bool {
        self.native_living_write(living, attribute, AttributeWrite::Remove(id.to_string()))
    }

    /// Sets the attribute's base value (`AttributeInstance.setBaseValue`);
    /// the value stays within the attribute's range. False for a non-finite
    /// base, and in the cases `add_attribute_modifier` is.
    pub(super) fn living_set_attribute_base(
        &mut self,
        living: LivingRef,
        attribute: &str,
        base: f64,
    ) -> bool {
        base.is_finite()
            && self.native_living_write(
                living,
                attribute,
                AttributeWrite::Base(self.mod_base_in_range(attribute, base)),
            )
    }

    /// A mod's base clamped to the attribute's range, which `/attribute base set` and the entity's own code skip.
    pub(in crate::game) fn mod_base_in_range(&self, attribute: &str, base: f64) -> f64 {
        self.motion
            .attributes
            .definition(attribute)
            .map_or(base, |d| d.sanitize(base))
    }

    fn native_living_add_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
        replace: bool,
    ) -> bool {
        let operation = match modifier.operation {
            AttributeOperation::AddValue => Operation::AddValue,
            AttributeOperation::AddMultipliedBase => Operation::AddMultipliedBase,
            AttributeOperation::AddMultipliedTotal => Operation::AddMultipliedTotal,
            _ => unreachable!("AttributeOperation: unhandled variant"),
        };
        let permanent = match lifetime {
            ModifierLifetime::Transient => false,
            ModifierLifetime::Permanent => true,
            _ => unreachable!("ModifierLifetime: unhandled variant"),
        };
        if !modifier.amount.is_finite() {
            return false;
        }
        AttributeWrite::add(
            &modifier.id.to_string(),
            modifier.amount,
            operation,
            permanent,
            replace,
        )
        .is_some_and(|write| self.native_living_write(living, attribute, write))
    }

    fn native_living_write(
        &mut self,
        living: LivingRef,
        name: &str,
        write: AttributeWrite,
    ) -> bool {
        self.native_living_target(living)
            .and_then(|t| self.write_attribute(t, name, write))
            .unwrap_or(false)
    }

    fn native_living_attribute(
        &self,
        living: LivingRef,
        name: &str,
        kind: sunrise_world::attribute_map::ValueKind,
    ) -> Option<f64> {
        self.native_target_attribute(self.native_living_read_target(living)?, name, kind)
    }

    pub(super) fn native_target_attribute(
        &self,
        target: Target,
        name: &str,
        kind: sunrise_world::attribute_map::ValueKind,
    ) -> Option<f64> {
        super::native_living_body::body_attribute(
            self.native_living_body(target)?,
            &self.motion.attributes,
            name,
            kind,
        )
    }

    /// `Entity.canUsePortal`: the decision the game's own portal trip runs —
    /// a player's (alive, nobody's passenger, not asleep: the engine's own
    /// `player_can_use_portal`, the one gate the client's portal entry, the
    /// guest trip and this read answer), a mob's walk gate
    /// (`mob_can_use_portal`, #1275's read: not a boss that never travels,
    /// not a creaking bound to its heart, not asleep in its bed, riding
    /// nothing, carrying no one this side holds back). None for a stale
    /// handle, an armor stand or the dragon: the portal walk carries
    /// neither as its own traveller.
    pub(super) fn living_can_use_portal(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            // The engine's own gate, the one the client's portal entry runs
            // (`player_can_use_portal`, #943).
            Target::Player => Some(self.player_can_use_portal()),
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                let p = g.motion.as_deref()?;
                Some(self.can_use_portal_of(p, g.vehicle.is_some(), g.sleeping))
            }
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                Some(self.mob_portal_eligible(m))
            }
            _ => None,
        }
    }

    /// `Mob.isWithinMeleeAttackRange`: the melee goal's own reach check — the
    /// mob's box inflated by sqrt(2.04) − 0.6 over the target's box, or an
    /// active item's attack range for a mob (a spear's reach) — whether this
    /// living entity's next melee strike reaches `target`. The check is a
    /// mob's; a player's attack reach is another rule (the interaction range
    /// attribute), so a player answers None.
    pub(super) fn living_in_melee_range(
        &self,
        living: LivingRef,
        target: EntityRef,
    ) -> Option<bool> {
        let Target::Mob(id) = self.native_living_read_target(living)? else {
            return None;
        };
        let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        let [min, max] = self.entity_bounding_box(target)?;
        let bounds = sunrise_core::geometry::Aabb::new(
            sunrise_core::geometry::Vec3::new(min[0], min[1], min[2]),
            sunrise_core::geometry::Vec3::new(max[0], max[1], max[2]),
        );
        let w = self.mob_world_base(&[]);
        Some(m.melee_reaches(&w, bounds))
    }

    /// `LivingEntity.canFreeze`: the decision the freeze tick itself runs — a
    /// mob's (`ai::can_freeze`: not `#freeze_immune_entity_types`, no
    /// `#freeze_immune_wearables` worn), a player's (`PlayerMotion::can_freeze`:
    /// not a spectator, no immune armor). None for a stale handle, an armor
    /// stand or the dragon.
    pub(super) fn living_can_freeze(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                let w = self.mob_world_base(&[]);
                Some(sunrise_client::mobs::ai::can_freeze(m, &w))
            }
            t => Some(self.native_player_body(t)?.can_freeze()),
        }
    }

    /// Membership in the live `#can_breathe_under_water` tag for a mob.
    /// The tag read only, not the whole drown decision. None for a stale handle,
    /// an armor stand or the dragon.
    pub(super) fn living_can_breathe_underwater(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Mob(id) => {
                let mob = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                Some(self.items().entity_type_tags.has_bit(
                    mob.kind.flags.entity_tag_slot,
                    EntityTypeTags::CAN_BREATHE_UNDER_WATER,
                ))
            }
            Target::Player | Target::Guest(_) => Some(false),
            _ => None,
        }
    }

    /// The bed a sleeper lies in, as its block faces
    /// (`LivingEntity.getBedOrientation` over `BedBlock.getBedOrientation`):
    /// the block at the sleeping position, its `facing` where that block is
    /// one of `#minecraft:beds`. None where the sleeper keeps no position,
    /// the block is not a bed, or the handle is stale.
    pub(super) fn living_bed_orientation(&self, living: LivingRef) -> Option<Direction> {
        let pos = self.living_sleeping_pos(living)?;
        let w = self.mob_world_base(&[]);
        let name = w.block_name(pos[0], pos[1], pos[2]);
        if !w.has_tag("minecraft:beds", name) {
            return None;
        }
        match w.prop(pos[0], pos[1], pos[2], "facing") {
            Some("north") => Some(Direction::North),
            Some("south") => Some(Direction::South),
            Some("west") => Some(Direction::West),
            Some("east") => Some(Direction::East),
            _ => None,
        }
    }

    /// The entity's health (`LivingEntity.getHealth`; Paper's
    /// `Damageable#getHealth`). During a player's turn only that player
    /// resolves: the hub is out for their action, and other players'
    /// handles read `None` until it is back.
    pub(super) fn living_health(&self, living: LivingRef) -> Option<f32> {
        let target = self.native_living_read_target(living)?;
        super::native_living_body::body_health(self.native_living_body(target)?)
    }
    /// The entity's maximum health (`LivingEntity.getMaxHealth`; Paper's
    /// `Damageable#getMaxHealth`). Players and guests read their attribute
    /// map; a mob, an armor stand and the dragon their own fields (the
    /// armor stand the fixed 20 the game seeds it with, the dragon its
    /// 200) until live attribute maps cover them.
    pub(super) fn living_max_health(&self, living: LivingRef) -> Option<f32> {
        let target = self.native_living_read_target(living)?;
        super::native_living_body::body_max_health(self.native_living_body(target)?)
    }
    /// Whether the entity is alive (`LivingEntity.isAlive`; Paper's
    /// `Entity#isDead` is its inverse). A dying mob, mid death
    /// animation, is not.
    pub(super) fn living_alive(&self, living: LivingRef) -> Option<bool> {
        self.living_health(living).map(alive_at)
    }

    /// Sets the entity's health (`LivingEntity.setHealth`; Paper's
    /// `Damageable#setHealth`), cut to its maximum health. No heal or damage
    /// hook runs. False for a stale handle, a non-finite value, an armor stand
    /// or the dragon.
    pub(super) fn living_set_health(&mut self, living: LivingRef, health: f32) -> bool {
        if !health.is_finite() {
            return false;
        }
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| {
                    let max = g.player.as_mut()?.attribute("minecraft:max_health").ok()? as f32;
                    let health = health.min(max);
                    let revived = g.player.as_ref()?.health.health() <= 0.0 && health > 0.0;
                    g.journal_player_health();
                    g.player.as_mut()?.set_health(health, false).ok()?;
                    if revived
                        && let Some(conn) = g.lan_acting
                        && let Some(host) = g.lan.as_mut()
                        && let Some(gi) =
                            host.hub.guests.iter().position(|guest| guest.conn == conn)
                        && host.hub.guests[gi].death_canceled
                    {
                        host.hub.guests.update(gi, |guest| {
                            guest.death_canceled = false;
                            guest.death_time = 0;
                        });
                        g.player.as_mut()?.health.dead = false;
                    }
                    Some(())
                })
                .flatten()
                .is_some(),
            Some(Target::Mob(id)) => {
                self.journal_mob_health(id);
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                let revived =
                    m.death_canceled && m.health <= 0.0 && health.min(m.attrs.max_health) > 0.0;
                m.health = health.clamp(0.0, m.attrs.max_health);
                m.needs_sync = true;
                if revived {
                    m.death_canceled = false;
                    m.death_time = 0;
                    self.died.remove(&id);
                }
                true
            }
            _ => false,
        }
    }
    /// Heals the entity by `amount` (`LivingEntity.heal`), through the
    /// `living_heal` hook, which may change or cancel it. A dead or dying
    /// entity isn't healed, and health stops at the maximum. False for a stale
    /// handle, a non-finite amount, a zero-health entity, an armor stand or the dragon.
    pub(super) fn living_heal(&mut self, living: LivingRef, amount: f32) -> bool {
        if !amount.is_finite()
            || !self
                .living_health(living)
                .is_some_and(|health| health > 0.0)
        {
            return false;
        }
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| {
                    g.apply_effect_action(sunrise_client::effects::EffectAction::Heal(amount));
                })
                .is_some(),
            Some(Target::Mob(id)) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
                    return false;
                };
                m.heal(
                    &self.loop_context,
                    &|| crate::game::entity::hit_reads_of!(self),
                    amount,
                );
                m.needs_sync = true;
                true
            }
            _ => false,
        }
    }

    /// `LivingEntity.getKillCredit`: the remembered player takes precedence,
    /// even if the player cannot be resolved; otherwise the last hurting mob,
    /// in whichever loaded level it is now (`credited_entity`).
    pub(super) fn living_kill_credit(&self, living: LivingRef) -> Option<EntityRef> {
        let credit = match self.native_living_target(living)? {
            Target::Player => self.combat.player_credit(self.world.game_time),
            Target::Guest(conn) => self
                .guest_combat
                .get(&conn)
                .and_then(|combat| combat.player_credit(self.world.game_time)),
            Target::Mob(id) => self.last_hurt_by_player.get(&id).copied(),
            _ => None,
        };
        if let Some(credit) = credit {
            let conn = self.resolve_player_credit(credit)?;
            let id = self.player_entity_id_by_connection(conn)?;
            return self.entity(u32::try_from(id).ok()?);
        }
        self.living_last_attacker(living)
    }

    /// `LivingEntity.getLastHurtByMob`: the entity that last hurt this one,
    /// within a hundred ticks of the hurt (the window of
    /// `CombatTracker.getKiller`'s credit and a mob's hurt-by memory, as
    /// `armadillo.rs` reads it). None for a stale handle, a window that has
    /// passed, an attacker that is gone, an armor stand, the dragon or a
    /// client.
    pub(super) fn living_last_attacker(&self, living: LivingRef) -> Option<EntityRef> {
        match self.native_living_target(living)? {
            Target::Player => self
                .combat
                .kill_credit(self.world.game_time)
                .and_then(|id| self.credited_entity(id)),
            Target::Guest(conn) => self
                .guest_combat
                .get(&conn)?
                .kill_credit(self.world.game_time)
                .and_then(|id| self.credited_entity(id)),
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                let who = m.recent_hurt_by()?;
                let attacker = match who {
                    Who::Mob(attacker) => {
                        return self.credited_entity(attacker);
                    }
                    // A player's hurt carries no id in `Who`: the hit's
                    // record names the player (`last_hurt_by_player`), the
                    // hub the guest's entity. The acting guest is resolved
                    // without the hub, which their turn takes out; the
                    // host's own record is displaced for that turn — the
                    // guest's body stands in as `player`, so the host's id
                    // is out of reach (the same None the other players'
                    // handles read while the hub is out).
                    Who::Player => {
                        let conn =
                            self.resolve_player_credit(*self.last_hurt_by_player.get(&id)?)?;
                        match conn {
                            None if self.lan_acting.is_some() => None,
                            None => Some(self.player.as_ref()?.entity_id as u32),
                            Some(conn) if self.lan_acting == Some(conn) => {
                                Some(self.lan_acting_entity?)
                            }
                            Some(conn) => Some(
                                self.lan
                                    .iter()
                                    .flat_map(|h| h.hub.guests.iter())
                                    .find(|g| g.conn == conn)?
                                    .entity_id as u32,
                            ),
                        }
                    }
                }?;
                self.entity(attacker)
            }
            _ => None,
        }
    }

    pub(super) fn living_last_hurt_mob(&self, living: LivingRef) -> Option<(EntityRef, i32)> {
        let (id, timestamp) = self.living_last_hurt_record(living)?;
        let target = self.entity(id)?;
        self.last_hurt_target_alive(id)
            .then_some((target, timestamp))
    }

    /// The holder's `lastHurtMob` id and `lastHurtMobTimestamp`, its target unresolved.
    pub(in crate::game) fn living_last_hurt_record(&self, living: LivingRef) -> Option<(u32, i32)> {
        match self.native_living_target(living)? {
            Target::Player => self.player.as_ref()?.health.last_hurt_mob,
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                match &guest.motion {
                    Some(body) => body.health.last_hurt_mob,
                    None => guest.carried_last_hurt_mob,
                }
            }
            Target::Mob(id) => {
                let (_, join) = self.last_hurt_living_target(id)?;
                self.mobs[self.mobs.locate(join)?].last_hurt_mob
            }
            Target::Stand(id) => {
                let (_, join) = self.last_hurt_living_target(id)?;
                self.armor_stands[self.armor_stands.locate(join)?].last_hurt_mob
            }
            Target::Dragon(id) => self.dragon.as_ref().filter(|d| d.id == id)?.last_hurt_mob,
            _ => None,
        }
    }

    /// `EntityReference.getEntity` for a kill credit, which looks through
    /// every level (`getEntityInAnyDimension`): the entity `id` in the level
    /// in place, else [`Self::credited_elsewhere`].
    pub(super) fn credited_entity(&self, id: u32) -> Option<EntityRef> {
        self.entity(id).or_else(|| self.credited_elsewhere(id))
    }

    /// A credited mob that is not in the level in place under `id`: the same
    /// id in another loaded level (a vehicle keeps it), or the id the
    /// dimension changes since gave that same mob (`Levels::travelled`), in
    /// whichever loaded level it is now. Never another entity holding its UUID.
    pub(in crate::game) fn credited_elsewhere(&self, id: u32) -> Option<EntityRef> {
        let epoch = self.native_lifecycle.epoch()?;
        let mut id = id;
        loop {
            if self.mob_in_other_level(id).is_some() {
                return Some(EntityRef::new(epoch, id));
            }
            id = self.levels.travelled(id)?;
            if let Some(found) = self.entity(id) {
                return Some(found);
            }
        }
    }

    /// Resolve one live equipment slot without taking a copy or reading any other slot.
    pub(super) fn living_equipment_stack(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<EquipmentRead<'_>> {
        self.equipment_stack_for(self.native_living_read_target(living)?, slot)
    }

    /// Read one equipment slot of an already resolved target.
    pub(super) fn equipment_stack_for(
        &self,
        target: Target,
        slot: EquipmentSlot,
    ) -> Option<EquipmentRead<'_>> {
        let body = match target {
            // A removed mob still lends what it wears.
            Target::Mob(id) => {
                sunrise_client::health::LivingBody::Mob(self.mobs.iter().find(|m| m.id == id)?)
            }
            target => self.native_living_body(target)?,
        };
        super::native_living_body::body_equipment(body, slot)
    }

    /// Lend the stack in `slot` (`LivingEntity.getItemBySlot`).
    pub(super) fn living_equipment(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        lend_equipment(self.living_equipment_stack(living, slot)?, read)
    }

    /// `Mob.getDropChances().byEquipment(slot)` of a mob of the level in place.
    pub(super) fn living_equipment_drop_chance(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<f32> {
        if !self.is_logical_server() {
            return None;
        }
        let Target::Mob(id) = self.native_living_target(living)? else {
            return None;
        };
        let mob = self.mobs.iter().find(|m| m.id == id)?;
        let slot = match slot {
            EquipmentSlot::MainHand => "mainhand",
            EquipmentSlot::OffHand => "offhand",
            EquipmentSlot::Feet => "feet",
            EquipmentSlot::Legs => "legs",
            EquipmentSlot::Chest => "chest",
            EquipmentSlot::Head => "head",
            EquipmentSlot::Body => "body",
            EquipmentSlot::Saddle => "saddle",
        };
        Some(sunrise_client::mobs::drops::drop_chance(mob, slot))
    }

    /// Damage an equipped stack without an equip/unequip notification.
    pub(super) fn living_hurt_and_break(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        amount: i32,
    ) -> Option<ItemStack> {
        let target = self.native_living_target(living)?;
        if matches!(target, Target::Player | Target::Guest(_)) {
            return self
                .for_player(target, |game| game.damage_player_equipment(slot, amount))
                .flatten();
        }
        let stack = self
            .equipment_stack_for(target, slot)?
            .project()
            .into_owned();
        let wearer = super::durability::Wearer::Living(living.id());
        let (updated, broken) = self.damage_stack_worn(&stack, amount, wearer);
        if updated.as_ref() == Some(&stack) {
            return None;
        }
        if self
            .living_equipment_stack(living, slot)?
            .project()
            .as_ref()
            != &stack
        {
            return None;
        }
        match target {
            Target::Mob(id) => {
                self.mobs
                    .iter_mut()
                    .find(|mob| mob.id == id)?
                    .set_equipment_stack(slot as usize, updated);
            }
            Target::Stand(id) => {
                self.armor_stands
                    .iter_mut()
                    .find(|stand| stand.id == id)?
                    .equipment
                    .set(slot as usize, updated);
            }
            Target::Dragon(id) => {
                self.dragon
                    .as_mut()
                    .filter(|dragon| dragon.id == id)?
                    .equipment
                    .set(slot as usize, updated);
            }
            _ => return None,
        }
        if broken.is_some() {
            let event =
                sunrise_client::mobs::item_break::equipment_break_event(world_equipment_slot(slot));
            // Non-player equipment writes above are unjournaled and take effect immediately; keep
            // the coupled break event on the same immediate side of the rollback boundary.
            self.broadcast_entity_event_immediate(living.entity(), event as u8);
        }
        broken
    }

    pub(super) fn damage_player_equipment(
        &mut self,
        slot: EquipmentSlot,
        amount: i32,
    ) -> Option<ItemStack> {
        let stack = player_slot(
            &self.inventory,
            self.selected,
            &self.armor,
            &self.offhand,
            &self.equipment,
            slot,
        )?
        .clone();
        let (updated, broken) = self.damage_stack(&stack, amount, true);
        if updated.as_ref() == Some(&stack) {
            return None;
        }
        let cell = match slot {
            EquipmentSlot::MainHand => &mut self.inventory[self.selected],
            EquipmentSlot::OffHand => &mut self.offhand,
            EquipmentSlot::Body | EquipmentSlot::Saddle => {
                if self.equipment.get(slot as usize) != Some(&stack) {
                    return None;
                }
                self.equipment.set(slot as usize, updated);
                if let Some(ref broken) = broken {
                    self.award_stat("broken", &broken.item, 1);
                    self.player_equipment_broke(world_equipment_slot(slot), broken);
                    self.player_item_destroyed(&stack, None);
                }
                return broken;
            }
            _ => &mut self.armor[armor_index(slot)?],
        };
        if cell.as_ref() != Some(&stack) {
            return None;
        }
        *cell = updated;
        if let Some(ref broken) = broken {
            self.award_stat("broken", &broken.item, 1);
            self.player_equipment_broke(world_equipment_slot(slot), broken);
            let hand = match slot {
                EquipmentSlot::MainHand => Some(Hand::Main),
                EquipmentSlot::OffHand => Some(Hand::Off),
                _ => None,
            };
            self.player_item_destroyed(&stack, hand);
            if armor_index(slot).is_some() {
                self.apply_equipment();
            }
        }
        broken
    }

    /// Lend the stack in `hand` (`LivingEntity.getItemInHand`).
    pub(super) fn living_hand_item(
        &self,
        living: LivingRef,
        hand: Hand,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        self.living_equipment(living, hand_slot(hand), read)
    }
    /// Puts `stack` in hand `hand` (`LivingEntity.setItemInHand`): the main
    /// hand's slot or the off hand's, written as `set_equipment`
    /// writes them, with its equip events and sound. None, air or a zero count
    /// empties the hand. False means unsupported or stale.
    pub(super) fn living_set_hand_item(
        &mut self,
        living: LivingRef,
        hand: Hand,
        stack: Option<ItemStack>,
    ) -> bool {
        self.living_set_equipment(living, hand_slot(hand), stack)
    }
    /// Puts `stack` in `slot` (`LivingEntity.setItemSlot`); None, air or a
    /// zero count empties it. Players, guests, mobs, armor stands and the
    /// dragon store all eight slots. An armor stand stores body and saddle
    /// too: `ArmorStand.canUseSlot` only stops interaction, and
    /// `LivingEntity.setItemSlot` keeps whatever it is given. False means
    /// unsupported or stale.
    /// This is a direct write: it takes effect and is final immediately, even
    /// inside `host.transaction`. A mob's write makes any equip/unequip event
    /// and equip sound immediately; a stand's or the dragon's makes neither. API 382.
    pub(super) fn living_set_equipment(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
    ) -> bool {
        self.write_living_equipment(living, slot, stack, false)
    }
    /// The write `native_living_set_equipment` makes (#932): with `defer`,
    /// its equip sound and game event wait for the root commit of the open
    /// transaction — an abort stays quiet — while the equipment and the
    /// attribute map change at once, as later steps of the same transaction
    /// read them.
    pub(super) fn write_living_equipment(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
        defer: bool,
    ) -> bool {
        // A client's copy changes only when the server says so.
        if self.remote.is_some() {
            return false;
        }
        let Some(t) = self.native_living_target(living) else {
            return false;
        };
        let stack = equipment_stack(stack);
        let equippable = stack.as_ref().and_then(|s| self.equippable_slot_sound(s));
        if let Target::Mob(id) = t {
            let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else {
                return false;
            };
            let changed = {
                let old = if (slot as usize) <= finalize::HEAD {
                    finalize::item_stack(mob, slot as usize).map(std::borrow::Cow::Owned)
                } else {
                    mob.extra
                        .slot_stack(slot as usize)
                        .map(std::borrow::Cow::Borrowed)
                };
                match (old.as_deref(), &stack) {
                    (None, None) => false,
                    (Some(a), Some(b)) => !a.same_item_same_components(b),
                    _ => true,
                }
            };
            if !mob.set_equipment_stack(slot as usize, stack) {
                return false;
            }
            if changed && !mob.first_tick {
                let at = mob.position;
                let sound = equippable.as_ref().and_then(|(equip_slot, sound)| {
                    (!mob.silent && *equip_slot as usize == slot as usize)
                        .then(|| {
                            super::dispenser::equip_sound(mob, *equip_slot, Some(sound.clone()))
                        })
                        .flatten()
                });
                if defer {
                    self.after_commit(super::transactions::After::Equip {
                        at: [at.x, at.y, at.z],
                        sound,
                        equips: equippable.is_some(),
                        who: id,
                    });
                } else {
                    let ctx = self.who_context(Some(sunrise_client::mobs::Who::Mob(id)), None);
                    self.equip_item_effects([at.x, at.y, at.z], sound, equippable.is_some(), ctx);
                }
            }
            // The stack's attribute modifiers go on the live map at once, so
            // the next read — a hit's armor, a `LivingRef`'s — sees them (#941).
            let before = if defer {
                self.snapshot_mob_modifiers(id)
            } else {
                None
            };
            self.collect_mob_equipment_attributes(id, before);
            return true;
        }
        // `LivingEntity.onEquipItem` for the entities that keep no equip state
        // of their own: the stand and the dragon (not silent, never spectators).
        let equip_event_at = match t {
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map(|a| (a.equipment.get(slot as usize), a.position, a.first_tick)),
            Target::Dragon(id) => self
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| (d.equipment.get(slot as usize), d.position, d.first_tick())),
            _ => None,
        }
        .and_then(|(old, at, first_tick)| {
            let same = match (old, stack.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_item_same_components(b),
                _ => false,
            };
            (!first_tick && !same).then_some(at)
        });
        let written = match self
            .slots_named(&Owner::Entity(t), slot_name(slot))
            .as_slice()
        {
            // This path owns onEquipItem below, including immediate writes.
            // The generic stand slot writer must not emit it a second time.
            [s] => self.slot_set_with_effects(s, stack, false),
            _ => false,
        };
        if let (true, Some(at)) = (written, equip_event_at) {
            let sound = equippable.as_ref().and_then(|(equip_slot, sound)| {
                (*equip_slot as usize == slot as usize).then(|| sound.clone())
            });
            let (Target::Stand(id) | Target::Dragon(id)) = t else {
                unreachable!("only a stand or the dragon gets here")
            };
            if defer {
                self.after_commit(super::transactions::After::Equip {
                    at: [at.x, at.y, at.z],
                    sound,
                    equips: equippable.is_some(),
                    who: id,
                });
            } else {
                let ctx_who = Some(sunrise_client::mobs::Who::Mob(id));
                let ctx = self.who_context(ctx_who, None);
                self.equip_item_effects([at.x, at.y, at.z], sound, equippable.is_some(), ctx);
            }
        }
        written
    }

    /// The slot and sound `stack`'s `equippable` component names
    /// (`Equippable.slot`, `equipSound`).
    pub(super) fn equippable_slot_sound(
        &self,
        stack: &ItemStack,
    ) -> Option<(super::dispenser::Slot, String)> {
        let name = sunrise_client::item_sync::equippable_slot(stack, self.items())?;
        let equip_slot = super::dispenser::Slot::from_name(&name)?;
        let sound = match stack.data().and_then(|d| d.other("minecraft:equippable")) {
            Some(component) => match component.get("equip_sound") {
                Some(sunrise_core::nbt::Tag::String(sound)) => Some(sound.to_string_lossy()),
                _ => None,
            },
            None => self
                .items()
                .get(&stack.item)
                .and_then(|i| i.equip_sound.clone()),
        }
        .unwrap_or_else(|| "minecraft:item.armor.equip_generic".into());
        Some((equip_slot, sound))
    }

    /// The effects of `LivingEntity.onEquipItem` once the guards passed (server
    /// side, not a spectator, not the first tick, another item): the equip
    /// sound when there is one, and the `equip` (the stack is equippable) or
    /// `unequip` game event.
    pub(in crate::game) fn equip_item_effects(
        &mut self,
        at: [f64; 3],
        sound: Option<String>,
        equips: bool,
        ctx: sunrise_client::sculk::Context,
    ) {
        if let Some(sound) = sound {
            self.play_audio(&sound, Some(at), 1.0, 1.0);
        }
        self.post_game_event(
            if equips {
                "minecraft:equip"
            } else {
                "minecraft:unequip"
            },
            at,
            ctx,
        );
    }
    /// Throws `stack`, which the mod holds, from the eyes (`LivingEntity.drop`,
    /// `ServerPlayer.drop`), never through a slot: along the view with a
    /// main-hand swing, or with `throw_randomly` scattered with none, as a
    /// death's drops. `include_thrower_name` sets the thrower, and a player's
    /// throw along the view then counts in its stats and fires `item_toss`.
    /// None for an empty stack, a stand or the dragon, a stale handle, or an
    /// item a mod kept out. API @API:living_refs@.
    pub(super) fn living_drop(
        &mut self,
        living: LivingRef,
        stack: ItemStack,
        throw_randomly: bool,
        include_thrower_name: bool,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        let target = self.native_living_target(living)?;
        let stack = equipment_stack(Some(stack))?;
        let mut item = None;
        match target {
            Target::Mob(id) => {
                if !throw_randomly {
                    self.living_swing(living, Hand::Main, true);
                }
                item = self.native_mob_drop(id, stack, throw_randomly, include_thrower_name);
            }
            Target::Player | Target::Guest(_) => {
                self.native_as_living_player(target, |g| {
                    item = g.native_player_drop(stack, throw_randomly, include_thrower_name);
                });
            }
            _ => {}
        }
        Some(EntityRef::new(living.entity().epoch(), item?))
    }
    /// `TamableAnimal.isTame`, `AbstractHorse.isTamed`: None for a kind that
    /// can't be tamed or a stale handle. API @API:living_refs@.
    pub(super) fn living_is_tamed(&self, living: LivingRef) -> Option<bool> {
        self.native_tamable(living).map(|m| m.extra.tame)
    }
    /// The owner's UUID (`OwnableEntity.getOwnerReference`): None for a wild
    /// mob, a kind that can't be tamed, or a stale handle. API @API:living_refs@.
    pub(super) fn living_owner(&self, living: LivingRef) -> Option<u128> {
        let owner = self.native_tamable(living)?.extra.owner?;
        Some(sunrise_client::lan::playerdata::ints_uuid(owner))
    }
    /// The living entity as a player, with its UUID: the player in place (a guest's while it
    /// acts) or a guest. Resolved through [`Self::player`], so it names the same player
    /// `host.player(uuid)` does, and only when that player is this entity. None for any other
    /// living entity, a fake player acting, a stale handle or on a client.
    pub(super) fn living_player(&self, living: LivingRef) -> Option<mod_api::PlayerRef> {
        if !self.is_logical_server() {
            return None;
        }
        let uuid = match self.native_living_target(living)? {
            Target::Player => self.lan_acting_uuid.or_else(|| {
                self.level
                    .player_uuid
                    .map(sunrise_client::lan::playerdata::ints_uuid)
            })?,
            Target::Guest(conn) => self.command_guest(conn)?.uuid,
            _ => return None,
        };
        self.player(uuid)
            .filter(|player| player.entity() == living.entity())
    }
    /// The mob a handle names, when it is a `TamableAnimal` or an `AbstractHorse`.
    fn native_tamable(&self, living: LivingRef) -> Option<&Mob> {
        let Target::Mob(id) = self.native_living_target(living)? else {
            return None;
        };
        self.mobs.iter().find(|m| m.id == id && m.is_tamable())
    }
    /// Runs `f` as the player `target` names: the one acting now, or a guest
    /// in the level in place through `as_guest` (`native_as_player`). False
    /// when it didn't run.
    fn native_as_living_player(&mut self, target: Target, f: impl FnOnce(&mut Self)) -> bool {
        let uuid = match target {
            Target::Guest(conn) => match self.command_guest(conn) {
                Some(g) if g.dimension == self.dim.id() => g.uuid,
                _ => return false,
            },
            _ => self.native_acting_id(),
        };
        self.native_as_player(uuid, f)
    }
    /// The acting player's half of `living_drop`: the id of the item entity.
    fn native_player_drop(
        &mut self,
        stack: ItemStack,
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        if randomly {
            let eye = self.eye_position()?;
            self.throw_from_eyes(stack, eye, true, thrown_from_hand)
        } else if thrown_from_hand {
            self.player_drop(stack)
        } else {
            self.drop_plain(stack)
        }
    }
    /// A mob's half of `living_drop`: the id of the item entity.
    fn native_mob_drop(
        &mut self,
        id: u32,
        stack: ItemStack,
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        let m = self.mobs.iter_mut().find(|m| m.id == id)?;
        let v = sunrise_client::entities::drop_motion(&mut m.random, m.yaw, m.head_pitch, randomly);
        let at = [m.position.x, m.eye_y() - f64::from(0.3f32), m.position.z];
        let thrower = thrown_from_hand.then(|| self.uuid_for(id));
        self.spawn_item(stack, at, v, 40);
        let item = self.just_spawned_item()?;
        item.thrower = thrower;
        Some(item.id)
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::super::api_values::TryIntoApi;
    use super::*;
    use sunrise_core::geometry::Vec3;

    /// A worn item's dye answers from its typed field, its removal marker and absence, and
    /// every other component still reads through the patch.
    #[test]
    fn a_worn_items_dye_override_reads_without_a_patch_projection() {
        use mod_api::{Tag, prelude::ItemsHost};
        use std::num::NonZeroU32;

        let mut host = mod_api::testing::FakeHost::new();
        let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
        host.registries.resolve_items([id.clone()]).unwrap();
        let item = host.item_key(&id).unwrap();
        let metadata = (item, NonZeroU32::MIN, None);
        let probe = |stack: &ItemStack, ids: &[&str]| {
            with_equipment_view(&EquipmentRead::resident(stack), metadata, |_, read| {
                ids.iter().map(|id| read(id)).collect::<Vec<_>>()
            })
        };
        let dyed_id = ["minecraft:dyed_color", "!minecraft:dyed_color"];
        let mut stack = ItemStack::new("minecraft:leather_chestplate", 1);
        assert_eq!(probe(&stack, &dyed_id), [None, None], "absence");
        stack.data_mut().dyed_color = Some(0x12_34_56);
        assert_eq!(
            probe(&stack, &dyed_id),
            [Some(Tag::Int(0x12_34_56)), None],
            "the typed field"
        );
        let mut removed = ItemStack::new("minecraft:leather_chestplate", 1);
        removed.data_mut().set_other(
            "!minecraft:dyed_color",
            Some(Tag::Compound(Default::default())),
        );
        assert_eq!(
            probe(&removed, &dyed_id),
            [None, Some(Tag::Compound(Default::default()))],
            "the removal marker"
        );
    }

    /// The mod API keeps a written base in range; vanilla's own `setBaseValue` (`/attribute base set`) does not.
    #[test]
    #[ignore = "daily"]
    fn a_mod_base_write_stays_in_range_and_a_vanilla_one_does_not() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for (id, entity) in [(55, "minecraft:pig"), (56, "minecraft:slime")] {
                let ty = game.res.mob_types().get(entity).unwrap().clone();
                game.mobs.push(Mob::new(id, ty, Vec3::ZERO, 0.0));
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert!(game.living_set_attribute_base(living, "minecraft:max_health", 2000.0));
                assert_eq!(
                    game.living_attribute_base(living, "minecraft:max_health"),
                    Some(1024.0),
                    "{entity}: the mod's write"
                );
                let write = AttributeWrite::Base(2000.0);
                assert_eq!(
                    game.write_attribute(Target::Mob(id), "minecraft:max_health", write),
                    Some(true)
                );
                assert_eq!(
                    game.living_attribute_base(living, "minecraft:max_health"),
                    Some(2000.0),
                    "{entity}: /attribute base set"
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn durability_handle_and_held_path_share_rng_and_damage() {
        use mod_api::ServerHost;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::random::RandomSource;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let living = game.living(game.entity(20).unwrap()).unwrap();
            let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
            stack.data_mut().enchantments = vec![("minecraft:unbreaking".into(), 3)];
            game.inventory[game.selected] = Some(stack.clone());
            let random = game.world.random.clone();
            // Before the migration, damage_held called wear and added its result.
            let old_loss = game.wear(&stack, 20);
            let old_next_random = game.world.random.next_float();
            game.world.random = random.clone();
            game.damage_held(20);
            let held_result = game.inventory[game.selected].clone();
            let next_random = game.world.random.next_float();
            assert_eq!(held_result.as_ref().unwrap().damage, old_loss);
            assert_eq!(next_random, old_next_random);
            game.world.random = random;
            game.inventory[game.selected] = Some(stack);
            let host: &mut dyn ServerHost = game;
            assert!(
                living
                    .hurt_and_break(host, EquipmentSlot::MainHand, 20)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected], held_result);
            assert_eq!(game.world.random.next_float(), next_random);

            let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
            stack.damage = 10;
            game.inventory[game.selected] = Some(stack.clone());
            let mut repair_random = game.world.random.clone();
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, -4)
                    .is_none()
            );
            assert_eq!(game.world.random.next_float(), repair_random.next_float());
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 6);
            game.creative = true;
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, 100)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 6);
            game.creative = false;
            let mut unbreakable = stack.clone();
            unbreakable.data_mut().set_other(
                "minecraft:unbreakable",
                Some(sunrise_core::nbt::Tag::Byte(1)),
            );
            game.inventory[game.selected] = Some(unbreakable.clone());
            assert!(
                living
                    .hurt_and_break(game, EquipmentSlot::MainHand, 100)
                    .is_none()
            );
            assert_eq!(game.inventory[game.selected], Some(unbreakable));
            let max = game.items().get(&stack.item).unwrap().max_damage;
            stack.damage = max - 1;
            game.inventory[game.selected] = Some(stack);
            let broken = living
                .hurt_and_break(game, EquipmentSlot::MainHand, 1)
                .unwrap();
            assert_eq!(broken.damage(), max);
            assert!(game.inventory[game.selected].is_none());

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
            let mob = game.living(game.entity(21).unwrap()).unwrap();
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.damage = max - 1;
            assert!(
                mob.set_equipment(
                    game,
                    EquipmentSlot::MainHand,
                    Some(sword.try_into_api().unwrap())
                )
                .unwrap()
            );
            let broken = mob
                .hurt_and_break(game, EquipmentSlot::MainHand, 1)
                .unwrap();
            assert_eq!(broken.damage(), max);
            assert!(
                mob.with_equipment(game, EquipmentSlot::MainHand, |_| ())
                    .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn armor_hit_matches_old_draws_and_break_order() {
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::random::RandomSource;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.audio.engine.loaded = true;
            game.audio.engine.registry.add_file("minecraft", br#"{"entity.item.break":{"sounds":["test/item"]},"block.anvil.break":{"sounds":["test/anvil"]}}"#, &|_, _| true);
            let source = sunrise_client::health::NO_ATTACK.player_source("minecraft:generic", 0);
            let mut armor: [Option<ItemStack>; 4] = std::array::from_fn(|_| None);
            for (i, name) in ["minecraft:diamond_helmet", "minecraft:diamond_chestplate"]
                .into_iter()
                .enumerate()
            {
                let mut stack = ItemStack::new(name, 1);
                stack.damage = game.items().get(name).unwrap().max_damage - 1;
                let sound = ["minecraft:entity.item.break", "minecraft:block.anvil.break"][i];
                stack.data_mut().set_other(
                    "minecraft:break_sound",
                    Some(sunrise_core::nbt::Tag::String(sound.into())),
                );
                armor[i] = Some(stack);
            }
            let mut leggings = ItemStack::new("minecraft:diamond_leggings", 1);
            leggings.data_mut().enchantments = vec![("minecraft:unbreaking".into(), 3)];
            armor[2] = Some(leggings);
            for guest in [false, true] {
                game.armor = armor.clone();
                game.lan_acting = guest.then_some(1);
                game.audio.capture = Some(Vec::new());
                game.audio.record_heard = true;
                game.audio.engine.notifications.clear();
                game.audio.heard.clear();
                game.guest_equipment_breaks.clear();
                let random = game.world.random.clone();
                // Player.hurtArmor supplies FEET, LEGS, CHEST, HEAD;
                // LivingEntity.doHurtEquipment finishes hurtAndBreak (including
                // its effects/RNG) before the next slot's durability draw.
                let mut broken = Vec::new();
                for index in [3, 2, 1, 0] {
                    let Some(mut stack) = game.armor[index].clone() else {
                        continue;
                    };
                    let max = game.items().get(&stack.item).unwrap().max_damage;
                    let loss = game.wear(&stack, 1);
                    if loss == 0 {
                        continue;
                    }
                    stack.damage = stack.damage.saturating_add(loss).min(max);
                    if stack.damage >= max {
                        game.armor[index] = None;
                        game.player_equipment_broke(super::super::armor_slot(index), &stack);
                        broken.push(index);
                    } else {
                        game.armor[index] = Some(stack);
                    }
                }
                let expected_armor = game.armor.clone();
                let expected_events = game.guest_equipment_breaks.clone();
                let expected_sounds = game.audio.heard.clone();
                let expected_sound_order: Vec<_> = game
                    .audio
                    .engine
                    .notifications
                    .iter()
                    .map(|event| event.identifier.clone())
                    .collect();
                let expected_random = game.world.random.next_float();
                assert_eq!(broken, [1, 0]);
                if guest {
                    assert_eq!(expected_events, [50, 49]);
                } else {
                    assert_eq!(expected_sounds.get("minecraft:entity.item.break"), Some(&1));
                    assert_eq!(expected_sounds.get("minecraft:block.anvil.break"), Some(&1));
                    assert_eq!(
                        expected_sound_order,
                        ["minecraft:block.anvil.break", "minecraft:entity.item.break"]
                    );
                }
                game.armor = armor.clone();
                game.guest_equipment_breaks.clear();
                game.audio.capture = Some(Vec::new());
                game.audio.heard.clear();
                game.audio.engine.notifications.clear();
                game.world.random = random;
                game.hurt_armor(&source, 4.0);
                assert_eq!(game.armor, expected_armor);
                assert_eq!(game.guest_equipment_breaks, expected_events);
                assert_eq!(game.audio.heard, expected_sounds);
                assert_eq!(
                    game.audio
                        .engine
                        .notifications
                        .iter()
                        .map(|event| event.identifier.clone())
                        .collect::<Vec<_>>(),
                    expected_sound_order
                );
                assert_eq!(game.world.random.next_float(), expected_random);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn immutable_host_attribute_read_refreshes_live_cache() {
        use mod_api::Host;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        use sunrise_world::attributes::{Modifier, Operation};
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let living = game.living(game.entity(20).unwrap()).unwrap();
            let attributes = &mut game.player.as_mut().unwrap().attributes;
            let key = attributes.key("minecraft:max_health").unwrap();
            let instance = attributes.get_instance(Some(key.clone())).unwrap().unwrap();
            assert_eq!(attributes.instance(&instance).unwrap().value(), 20.0);
            attributes
                .edit(&instance, |a, o| {
                    a.add_transient(
                        Modifier::new("test:bonus", 5.0, Operation::AddValue).unwrap(),
                        o,
                    )
                })
                .unwrap();
            assert_eq!(attributes.cached_value(Some(&key)).unwrap(), 20.0);
            let host: &dyn Host = game;
            assert_eq!(
                living.attribute_base(host, "minecraft:max_health"),
                Some(20.0)
            );
            assert_eq!(
                game.player
                    .as_ref()
                    .unwrap()
                    .attributes
                    .cached_value(Some(&key))
                    .unwrap(),
                20.0
            );
            assert_eq!(
                living.attribute_value(host, "minecraft:max_health"),
                Some(25.0)
            );
            assert_eq!(living.max_health(host), Some(25.0));
            assert_eq!(living.attribute_value(host, "missing:attribute"), None);
            assert_eq!(
                game.player
                    .as_ref()
                    .unwrap()
                    .attributes
                    .cached_value(Some(&key))
                    .unwrap(),
                25.0
            );
        });
    }

    /// The reads a handle answers about an entity's attributes: the type's
    /// attribute ids, and one attribute's modifiers through the handle's own
    /// add, replace and remove writes.
    #[test]
    #[ignore = "daily"]
    fn a_handle_lists_its_attributes_and_reads_its_modifiers() {
        use crate::game::native_entity_refs::tests::fixture_resident_column;
        use mod_api::{AttributeOperation, AttributeOperation::*, Identifier, ModifierLifetime::*};
        use sunrise_client::entities::ArmorStand;
        use sunrise_client::mobs::Mob;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        use sunrise_items::equipment::HEAD;
        let modifier = |id: &str, amount: f64, operation: AttributeOperation| {
            AttributeModifier::new(Identifier::parse(id).unwrap(), amount, operation)
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let zombie_kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie_kind, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let stand = game.living(game.entity(13).unwrap()).unwrap();

            let read_names = |game: &dyn mod_api::Host, living: LivingRef| {
                let mut names = Vec::new();
                let result = living.visit_attributes(game, &mut |name| {
                    names.push(name.to_owned());
                    ControlFlow::Continue(())
                });
                assert_eq!(result, Ok(ControlFlow::Continue(())));
                names
            };
            let read_modifiers = |game: &dyn mod_api::Host,
                                  living: LivingRef,
                                  name: &str|
             -> Result<Vec<_>, EntityReadError> {
                let mut modifiers = Vec::new();
                assert_eq!(
                    living.visit_attribute_modifiers(game, name, &mut |m| {
                        modifiers.push((m.id.to_owned(), m.amount, m.operation));
                        ControlFlow::Continue(())
                    })?,
                    ControlFlow::Continue(())
                );
                Ok(modifiers)
            };
            // Zombie.createAttributes supports these; Attributes registers them in this order.
            let known = [
                "minecraft:armor",
                "minecraft:attack_damage",
                "minecraft:max_health",
                "minecraft:movement_speed",
            ];
            let names = read_names(game, zombie);
            let matching: Vec<_> = names
                .iter()
                .map(String::as_str)
                .filter(|id| known.contains(id))
                .collect();
            assert_eq!(matching, known);
            let mut first_name = Vec::new();
            assert_eq!(
                zombie.visit_attributes(game, &mut |id| {
                    first_name.push(id.to_owned());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first_name, ["minecraft:air_drag_modifier".to_owned()]);
            assert!(
                !read_names(game, stand)
                    .iter()
                    .any(|id| id == "minecraft:attack_damage")
            );

            // A known attribute with no modifiers completes; an unknown or unsupported one refuses.
            let speed = "minecraft:movement_speed";
            assert_eq!(read_modifiers(game, zombie, speed), Ok(Vec::new()));
            assert_eq!(
                zombie.visit_attribute_modifiers(game, "missing:attribute", &mut |_| panic!(
                    "unknown attribute visited"
                )),
                Err(EntityReadError::Unavailable)
            );
            assert_eq!(
                stand.visit_attribute_modifiers(game, "minecraft:attack_damage", &mut |_| panic!(
                    "unsupported attribute visited"
                )),
                Err(EntityReadError::Unavailable)
            );

            // The add, replace and remove writes the reads follow.
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 1.0, AddMultipliedBase),
                Permanent
            ));
            assert!(zombie.add_attribute_modifier(
                game,
                speed,
                modifier("test:boost", 0.5, AddMultipliedTotal),
                Transient
            ));
            assert_eq!(
                read_modifiers(game, zombie, speed),
                Ok(vec![
                    ("test:fast".to_owned(), 1.0, AddMultipliedBase),
                    ("test:boost".to_owned(), 0.5, AddMultipliedTotal),
                ])
            );
            let mut first_modifier = Vec::new();
            assert_eq!(
                zombie.visit_attribute_modifiers(game, speed, &mut |m| {
                    first_modifier.push((m.id.to_owned(), m.amount, m.operation));
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(
                first_modifier,
                [("test:fast".to_owned(), 1.0, AddMultipliedBase)]
            );

            // Replacing puts the new one at the end, its lifetime changed.
            assert!(zombie.replace_attribute_modifier(
                game,
                speed,
                modifier("test:fast", 2.0, AddValue),
                Transient
            ));
            let boost = Identifier::parse("test:boost").unwrap();
            assert!(zombie.remove_attribute_modifier(game, speed, &boost));
            assert_eq!(
                read_modifiers(game, zombie, speed),
                Ok(vec![("test:fast".to_owned(), 2.0, AddValue)])
            );

            // A worn helmet's modifier is read off the stand's live map.
            game.armor_stands[0]
                .equipment
                .set(HEAD, Some(ItemStack::new("minecraft:diamond_helmet", 1)));
            game.refresh_stand_and_dragon_equipment_attributes();
            let armor = read_modifiers(game, stand, "minecraft:armor").unwrap();
            assert!(armor.iter().any(|m| m.1 == 3.0), "{armor:?}");

            // A player's own map answers the same read.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let health = "minecraft:max_health";
            assert_eq!(read_modifiers(game, player, health), Ok(Vec::new()));
            assert!(player.add_attribute_modifier(
                game,
                health,
                modifier("test:hearty", 0.5, AddMultipliedBase),
                Permanent
            ));
            assert_eq!(
                read_modifiers(game, player, health),
                Ok(vec![("test:hearty".to_owned(), 0.5, AddMultipliedBase)])
            );

            // A stale handle refuses before calling either visitor.
            game.mobs.clear();
            game.armor_stands.clear();
            assert_eq!(
                zombie.visit_attributes(game, &mut |_| panic!("stale attributes visited")),
                Err(EntityReadError::Stale)
            );
            assert_eq!(
                zombie.visit_attribute_modifiers(game, speed, &mut |_| panic!(
                    "stale modifier visited"
                )),
                Err(EntityReadError::Stale)
            );
            assert_eq!(
                stand.visit_attribute_modifiers(game, "minecraft:armor", &mut |_| panic!(
                    "stale modifier visited"
                )),
                Err(EntityReadError::Stale)
            );
        });
    }

    /// `LivingRef::equipment_drop_chance` (#5935): a mob's default chance, then the one it
    /// records, slot by slot; a handle of no living entity reads None.
    #[test]
    #[ignore = "daily"]
    fn a_mobs_equipment_drop_chance_reads_its_recorded_chance_per_slot() {
        use sunrise_client::chunk::Chunk;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let id = game
                .spawn_mob(
                    "minecraft:zombie",
                    [2.5, 64.0, 2.5],
                    true,
                    super::super::creatures::SpawnSite::Finalizing(
                        sunrise_client::mobs::spawn_rules::Reason::Command,
                    ),
                )
                .unwrap();
            let mob = game.living(game.entity(id).unwrap()).unwrap();
            let zombie = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            zombie.extra.drop_chances = None;
            assert_eq!(
                mob.equipment_drop_chance(game, EquipmentSlot::Head),
                Some(0.085)
            );
            assert_eq!(
                mob.equipment_drop_chance(game, EquipmentSlot::Saddle),
                Some(2.0)
            );
            // `EQUIPMENT_SLOTS` order: mainhand, offhand, feet, legs, chest, head, body, saddle.
            let zombie = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            zombie.extra.drop_chances = Some(Box::new([0.1, 0.2, 0.3, 0.4, 0.5, 2.0, 0.7, 0.0]));
            for (slot, chance) in [
                (EquipmentSlot::MainHand, 0.1),
                (EquipmentSlot::OffHand, 0.2),
                (EquipmentSlot::Feet, 0.3),
                (EquipmentSlot::Legs, 0.4),
                (EquipmentSlot::Chest, 0.5),
                (EquipmentSlot::Head, 2.0),
                (EquipmentSlot::Body, 0.7),
                (EquipmentSlot::Saddle, 0.0),
            ] {
                assert_eq!(
                    mob.equipment_drop_chance(game, slot),
                    Some(chance),
                    "{slot:?}"
                );
            }
            let gone = LivingRef::new(EntityRef::new(mob.entity().epoch(), 999));
            assert_eq!(gone.equipment_drop_chance(game, EquipmentSlot::Head), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn living_reads_answer_the_vanilla_defaults_and_a_gone_entity_reads_none() {
        use sunrise_client::entities::ArmorStand;
        use sunrise_client::mobs::Mob;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            // The eight dragon parts must not overlap the player at ID 20.
            dragon.id = 40;
            game.dragon.replace(dragon);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            // The vanilla attribute defaults: a player and a zombie 20, an
            // armor stand the 20 the game seeds it with, the dragon its 200.
            for (id, health, max, alive) in [
                (20u32, 20.0, 20.0, true),
                (12, 20.0, 20.0, true),
                (13, 20.0, 20.0, true),
                (40, 200.0, 200.0, true),
            ] {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                assert_eq!(living.health(game), Some(health), "health of {id}");
                assert_eq!(living.max_health(game), Some(max), "max of {id}");
                assert_eq!(living.alive(game), Some(alive), "alive of {id}");
            }
            // A dead dragon: isAlive is health above zero.
            let dragon_handle = game.living(game.entity(40).unwrap()).unwrap();
            game.dragon.as_mut().unwrap().health = 0.0;
            assert_eq!(dragon_handle.health(game), Some(0.0));
            assert_eq!(dragon_handle.alive(game), Some(false));
            // The entities gone: every read answers None.
            game.player = None;
            game.lan_acting_uuid = None;
            game.lan_acting_entity = None;
            game.mobs[0].removed = true;
            game.armor_stands.clear();
            game.dragon.discard();
            for id in [20u32, 12, 13, 40] {
                let handle = game.entity(id).and_then(|er| game.living(er));
                assert!(handle.is_none(), "handle of {id} after it is gone");
            }
            assert_eq!(dragon_handle.health(game), None);
            assert_eq!(dragon_handle.max_health(game), None);
            assert_eq!(dragon_handle.alive(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn kill_credit_prefers_the_player_reference_and_never_falls_back_when_unresolved() {
        use sunrise_client::mobs::{Damage, Mob};
        use sunrise_core::geometry::Vec3;

        super::super::native_guest_turn_tests::with_guests(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let wolf = game.res.mob_types().get("minecraft:wolf").unwrap().clone();
            game.mobs.push(Mob::new(100, pig, Vec3::ZERO, 0.0));
            let mut wolf = Mob::new(101, wolf, Vec3::ZERO, 0.0);
            wolf.extra.tame = true;
            wolf.extra.owner = Some(sunrise_client::lan::playerdata::uuid_ints(12));
            game.mobs.push(wolf);
            game.mobs[0].ai.last_hurt_by = Some(Who::Mob(101));
            game.mobs[0].ai.last_hurt_by_time = game.mobs[0].age as i32;
            let living = game.living(game.entity(100).unwrap()).unwrap();
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(101));

            // A player hit followed by a mob hit keeps the player credit.
            game.as_guest(12, |game| game.remember_mob_player_credit(100))
                .unwrap();
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            // A tame wolf's hit also records its owner's reference.
            game.resolve_player_responsible(100, Damage::mob(101, [0.0; 3]));
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            {
                let value = "minecraft:the_nether".into();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.dimension = value);
            }
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(12));
            {
                let value = false;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.placed = value);
            }
            assert_eq!(living.kill_credit(game), None);

            // Once the player reference expires, the mob can be credited.
            game.last_hurt_by_player.remove(&100);
            assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(101));
            // A mob that has left this level is no longer resolvable by its old ID.
            game.mobs[1].removed = true;
            assert_eq!(living.kill_credit(game), None);
            game.mobs[1].removed = false;
            game.mobs[0].age += 101;
            assert_eq!(living.kill_credit(game), None);
        });
    }

    /// The UUID the transfer tests give their attacker, as `summon` takes it.
    pub(in crate::game) const ZOMBIE_UUID: [i32; 4] = [7, 8, 9, 10];
    const ZOMBIE_NBT: &str = "{NoAI:1b,UUID:[I;7,8,9,10]}";

    pub(in crate::game) fn summon_mob(game: &mut super::Game<'_>, kind: &str, nbt: &str) -> u32 {
        match game
            .summon_with_nbt(kind, [0.5, 70.0, 0.5], nbt)
            .unwrap()
            .unwrap()
        {
            Target::Mob(id) => id,
            other => panic!("{kind} summoned as {other:?}"),
        }
    }

    /// A pig hurt by a zombie of [`ZOMBIE_UUID`] in the Overworld, with the
    /// Nether's spawn chunk forced loaded so it stays parked beside it.
    pub(in crate::game) fn hurt_pig(game: &mut super::Game<'_>) -> (u32, u32) {
        use crate::game::Dim;
        use std::sync::Arc;
        game.world_root = game.save.root.clone();
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
            .unwrap();
        game.build_registries();
        assert!(game.as_console(|game| game.run_command("forceload add 0 0")));
        game.enter_level(Dim::Overworld).unwrap();
        let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
        let zombie = summon_mob(game, "minecraft:zombie", ZOMBIE_NBT);
        let hit = sunrise_client::mobs::Damage::mob(zombie, [0.5, 70.0, 0.5]);
        assert!(game.mob_hits_mob(pig, 1.0, hit));
        let living = game.living(game.entity(pig).unwrap()).unwrap();
        assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(zombie));
        (pig, zombie)
    }

    pub(in crate::game) fn send_to_the_nether(game: &mut super::Game<'_>, mob: u32) {
        use crate::game::{Dim, vehicle_teleport::TeleportMotion};
        assert!(
            game.teleport_vehicle_across(
                Target::Mob(mob),
                Dim::Nether,
                [0.5, 70.0, 0.5],
                None,
                false,
                TeleportMotion::Stop,
            )
            .unwrap()
        );
        assert_eq!(game.dim, Dim::Overworld);
    }

    /// `EntityReference.getEntity` looks through every level: the zombie that
    /// hurt the pig keeps the credit after it crossed into the Nether under a
    /// new id, even with another mob holding its UUID in the pig's level.
    #[test]
    #[ignore = "daily"]
    fn kill_credit_follows_a_mob_that_changed_dimension() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let (pig, zombie) = hurt_pig(game);
            send_to_the_nether(game, zombie);
            let living = game.living(game.entity(pig).unwrap()).unwrap();
            let bystander = summon_mob(game, "minecraft:zombie", ZOMBIE_NBT);
            // What a mod reads from the pig's level, the Nether still parked.
            let credit = living.kill_credit(game).expect("credit kept");
            assert!(![zombie, bystander, pig].contains(&credit.id()));
            assert_eq!(living.last_attacker(game), Some(credit));
            assert_eq!(
                credit.kind(game),
                mod_api::__private::resolved_entity_type_key("minecraft:zombie")
            );
            assert_eq!(
                credit.uuid(game),
                Some(sunrise_client::lan::playerdata::ints_uuid(ZOMBIE_UUID))
            );
            let nether = game.level("minecraft:the_nether");
            assert_eq!(credit.level(game), nether);
            // The same reads from a mod goal of the pig's own tick.
            let mut actor = game.mobs.iter().find(|m| m.id == pig).unwrap().clone();
            game.in_goal_view(&mut actor, |view| {
                assert_eq!(living.kill_credit(view), Some(credit));
                assert_eq!(
                    credit.kind(view),
                    mod_api::__private::resolved_entity_type_key("minecraft:zombie")
                );
                // `uuid` takes a `ServerHost`, which a goal view is not.
                assert_eq!(credit.level(view), nether);
            });
        });
    }

    /// A Nether mob already holds the zombie's UUID, so the zombie's copy is
    /// refused residency there: the credit names no one, not that other mob.
    #[test]
    #[ignore = "daily"]
    fn a_refused_arrival_hands_the_credit_to_no_other_mob() {
        use crate::game::Dim;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (pig, zombie) = hurt_pig(game);
            game.enter_level(Dim::Nether).unwrap();
            summon_mob(game, "minecraft:zombie", ZOMBIE_NBT);
            game.enter_level(Dim::Overworld).unwrap();
            send_to_the_nether(game, zombie);
            let living = game.living(game.entity(pig).unwrap()).unwrap();
            assert_eq!(living.kill_credit(game), None);
            assert_eq!(living.last_attacker(game), None);
        });
    }

    /// `last_hurt_by_mob` keeps naming a credited mob that is in another
    /// loaded level: saved through the hundredth tick, not the next, and read
    /// back to the mob there.
    #[test]
    #[ignore = "daily"]
    fn a_saved_credit_names_a_mob_in_another_level_for_a_hundred_ticks() {
        use sunrise_core::nbt::Tag;
        super::super::native_entity_refs::tests::with_game(|game| {
            let (pig, zombie) = hurt_pig(game);
            send_to_the_nether(game, zombie);
            let i = game.mobs.iter().position(|m| m.id == pig).unwrap();
            let mut saved = |since: i32| {
                let m = &mut game.mobs[i];
                m.age = (m.ai.last_hurt_by_time + since) as u32;
                let mut tag = Tag::Compound(Default::default());
                game.put_last_hurt_by_mob(&game.mobs[i], &mut tag);
                tag
            };
            let last = saved(100);
            assert_eq!(
                last.get("last_hurt_by_mob"),
                Some(&Tag::IntArray(ZOMBIE_UUID.to_vec()))
            );
            assert_eq!(
                last.get("ticks_since_last_hurt_by_mob"),
                Some(&Tag::Int(100))
            );
            assert_eq!(saved(101).get("last_hurt_by_mob"), None);
            let nether = game.level("minecraft:the_nether");
            for (since, credited) in [(100, true), (101, false)] {
                let nbt = format!(
                    "{{NoAI:1b,last_hurt_by_mob:[I;7,8,9,10],ticks_since_last_hurt_by_mob:{since}}}"
                );
                let read_back = summon_mob(game, "minecraft:pig", &nbt);
                game.resolve_last_hurt_by_refs();
                let living = game.living(game.entity(read_back).unwrap()).unwrap();
                let credit = living.kill_credit(game);
                assert_eq!(credit.is_some(), credited, "{since} ticks after the hurt");
                if let Some(credit) = credit {
                    assert_eq!(credit.level(game), nether);
                    assert_ne!(credit.id(), zombie);
                }
            }
        });
    }

    /// `living_knockback`'s kill credit for a push that leaves the pig's credit
    /// as it was: the zombie now in the Nether, and the sword it holds there.
    #[test]
    #[ignore = "daily"]
    fn a_push_names_the_credited_mob_in_another_level_and_its_main_hand() {
        use sunrise_client::health::NO_ATTACK;
        use sunrise_client::items::ItemStack;
        use sunrise_client::mobs::{Damage, DamageKind};
        let mut credited = 0;
        let heard = super::super::native_knockback_tests::heard(|game, _| {
            let (pig, zombie) = hurt_pig(game);
            game.mobs
                .iter_mut()
                .find(|m| m.id == zombie)
                .unwrap()
                .extra
                .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_sword", 1)));
            send_to_the_nether(game, zombie);
            let living = game.living(game.entity(pig).unwrap()).unwrap();
            credited = living.kill_credit(game).unwrap().id() as i32;
            let plan = game.prepare_knockback_credit(pig as i32, None, game.combat.attack());
            let mut credit = plan.lend(&mut game.last_hurt_by_player);
            let i = game.mobs.iter().position(|m| m.id == pig).unwrap();
            let mob = &mut game.mobs[i];
            mob.knockback(
                crate::game::entity::damage_level_of!(game, &NO_ATTACK, &mut credit, None),
                0.4,
                1.0,
                0.0,
                &Damage::of(DamageKind::Generic),
                0.0,
            );
        });
        assert_eq!(
            heard.credits.last(),
            Some(&(credited, Some("minecraft:iron_sword".to_owned())))
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_mobs_last_attacker_names_a_mob_the_host_and_none_when_displaced() {
        use sunrise_client::mobs::Mob;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, pig, Vec3::ZERO, 0.0));
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(15, zombie, Vec3::ZERO, 0.0));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            // The host's own turn: the uuid is acting, no guest connection.
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let pig_living = game.living(game.entity(12).unwrap()).unwrap();
            // A mob's hit names the mob's own id.
            game.mobs[0].ai.last_hurt_by = Some(Who::Mob(15));
            game.mobs[0].ai.last_hurt_by_time = game.mobs[0].age as i32;
            assert_eq!(pig_living.last_attacker(game).map(|a| a.id()), Some(15));
            // The host's hit, read in the host's own turn: the host's handle.
            game.mobs[0].ai.last_hurt_by = Some(Who::Player);
            game.last_hurt_by_player
                .insert(12, super::super::combat::PlayerCredit(123));
            assert_eq!(pig_living.last_attacker(game).map(|a| a.id()), Some(20));
            // The same record inside a guest's turn: the host's displaced
            // body is out of reach, so the attacker reads None.
            game.lan_acting = Some(7);
            game.lan_acting_uuid = Some(456);
            assert_eq!(pig_living.last_attacker(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn set_health_can_store_zero() {
        use sunrise_client::mobs::Mob;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(living.set_health(game, 8.0));
            assert_eq!(living.health(game), Some(8.0));
            let accepted = living.set_health(game, 0.0);
            assert_eq!((accepted, living.health(game)), (true, Some(0.0)));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn set_health_journal_restores_nested_player_and_mob_writes() {
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            game.mobs[0].needs_sync = false;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            super::super::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let mob = game.living(game.entity(12).unwrap()).unwrap();
            let initial = (player.health(game), mob.health(game));
            let dirty = game.player.as_ref().unwrap().health.data.is_dirty();
            let events = game.player.as_ref().unwrap().events.len();
            game.storage_scope(|game, _| {
                assert!(player.set_health(game, 12.0));
                assert!(mob.set_health(game, 8.0));
                game.storage_scope(|game, _| {
                    assert!(player.set_health(game, 0.0));
                    assert!(mob.set_health(game, 0.0));
                    ((), false)
                });
                assert_eq!(
                    (player.health(game), mob.health(game)),
                    (Some(12.0), Some(8.0))
                );
                game.storage_scope(|game, _| {
                    assert!(player.set_health(game, 0.0));
                    assert!(mob.set_health(game, 0.0));
                    ((), true)
                });
                ((), false)
            });
            assert_eq!((player.health(game), mob.health(game)), initial);
            assert_eq!(game.player.as_ref().unwrap().health.data.is_dirty(), dirty);
            assert_eq!(game.player.as_ref().unwrap().events.len(), events);
            assert!(!game.mobs[0].needs_sync);
            assert!(!game.player.as_ref().unwrap().health.dead);
            assert!(!game.mobs[0].removed);
            game.mobs[0].needs_sync = true;
            game.storage_scope(|game, _| {
                assert!(mob.set_health(game, 0.0));
                ((), false)
            });
            assert_eq!(mob.health(game), initial.1);
            assert!(
                game.mobs[0].needs_sync,
                "abort preserves preexisting sync dirtiness"
            );
            game.storage_scope(|game, _| {
                assert!(player.set_health(game, 0.0));
                assert!(mob.set_health(game, 0.0));
                ((), true)
            });
            assert_eq!(
                (player.health(game), mob.health(game)),
                (Some(0.0), Some(0.0))
            );
            assert!(game.mobs[0].needs_sync);
            assert!(game.player.as_ref().unwrap().events.len() > events);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn set_health_journal_restores_zero_health_guest_outside_its_turn() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let guest = game.player(12).unwrap();
            let living = LivingRef::new(guest.entity());
            let before = living.health(game).unwrap();
            game.storage_scope(|game, _| {
                assert!(living.set_health(game, before - 2.0));
                game.storage_scope(|game, _| {
                    assert!(living.set_health(game, 0.0));
                    assert_eq!(living.health(game), Some(0.0));
                    ((), false)
                });
                assert_eq!(living.health(game), Some(before - 2.0));
                game.storage_scope(|game, _| {
                    assert!(living.set_health(game, 0.0));
                    ((), true)
                });
                ((), false)
            });
            assert_eq!(living.health(game), Some(before));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn heal_uses_the_vanilla_signed_sum() {
        use sunrise_client::mobs::Mob;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, zombie, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(living.set_health(game, 10.0));
            assert!(living.heal(game, 2.0));
            assert_eq!(living.health(game), Some(12.0));
            let _ = living.heal(game, -4.0);
            assert_eq!(living.health(game), Some(8.0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn absorbed_player_hit_still_remembers_the_living_attacker() {
        use sunrise_client::mobs::Mob;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(15, zombie, Vec3::ZERO, 0.0));
            let source = game
                .motion
                .health
                .damage
                .id("minecraft:mob_attack")
                .unwrap();
            let mut attackers = Vec::new();
            let mut health = Vec::new();
            for absorption in [0.0, 8.0] {
                game.combat = Default::default();
                let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
                player.entity_id = 20;
                player
                    .set_modifier(
                        "minecraft:max_absorption",
                        "test:absorption",
                        8.0,
                        "add_value",
                        true,
                    )
                    .unwrap();
                player.set_health(absorption, true).unwrap();
                game.player = Some(player);
                crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
                game.note_attacker("minecraft:mob_attack", 15);
                let rules = game.world.health_rules;
                let level = super::super::entity::damage_level_of!(
                    game,
                    &sunrise_client::health::NO_ATTACK,
                    no_push,
                    None
                );
                let player = game.player.as_mut().unwrap();
                assert!(
                    player
                        .hurt_caused_by(
                            level,
                            &game.motion.health,
                            rules,
                            source,
                            4.0,
                            None,
                            true,
                            15,
                            false
                        )
                        .unwrap()
                        .hurt
                );
                let events = std::mem::take(&mut player.events);
                game.record_combat(&events);
                let living = game.living(game.entity(20).unwrap()).unwrap();
                health.push(living.health(game));
                let host: &dyn mod_api::Host = game;
                attackers.push(living.last_attacker(host).map(|attacker| attacker.id()));
            }
            assert_eq!(
                health,
                vec![Some(16.0), Some(20.0)],
                "damage/absorption control"
            );
            assert_eq!(attackers, vec![Some(15), Some(15)]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn raw_health_write_can_change_a_dead_entity_but_heal_cannot() {
        use sunrise_client::mobs::Mob;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, pig, Vec3::ZERO, 0.0));
            // Dying, mid death animation: not removed, but not alive.
            game.mobs[0].health = 0.0;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let mob_living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(!mob_living.heal(game, 10.0));
            assert_eq!(mob_living.health(game), Some(0.0));
            assert!(mob_living.set_health(game, 10.0));
            assert_eq!(mob_living.health(game), Some(10.0));
            // A raw write also reaches a zero-health player.
            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            let player_living = game.living(game.entity(20).unwrap()).unwrap();
            assert!(!player_living.heal(game, 10.0));
            assert_eq!(player_living.health(game), Some(0.0));
            assert!(player_living.set_health(game, 10.0));
            assert_eq!(player_living.health(game), Some(10.0));
        });
    }

    /// `set_hand_item` writes the hand's equipment slot, as vanilla's
    /// `setItemInHand` does: a player's selected slot or off hand, a mob's
    /// main or off hand; a stale handle writes nothing.
    #[test]
    #[ignore = "daily"]
    fn set_hand_item_writes_the_hands_slot() {
        use mod_api::ServerHost;
        use sunrise_client::mobs::Mob;
        use sunrise_client::motion::PlayerMotion;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.selected = 3;
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let sword = ItemStack::new("minecraft:iron_sword", 1);
            let shield = ItemStack::new("minecraft:shield", 1);
            let host: &mut dyn ServerHost = game;
            assert!(
                player
                    .set_hand_item(host, Hand::Main, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
            assert!(
                player
                    .set_hand_item(host, Hand::Off, Some(shield.try_into_api().unwrap()))
                    .unwrap()
            );
            assert_eq!(game.inventory[3], Some(sword.clone()));
            assert_eq!(game.offhand, Some(shield.clone()));
            assert_eq!(
                player.with_hand_item(game, Hand::Main, |stack| (
                    stack.item().id(game),
                    stack.count()
                )),
                Some((Some("minecraft:iron_sword"), 1))
            );
            assert_eq!(
                player.with_hand_item(game, Hand::Off, |stack| (
                    stack.item().id(game),
                    stack.count()
                )),
                Some((Some("minecraft:shield"), 1))
            );
            assert!(player.set_hand_item(game, Hand::Off, None).unwrap());
            assert_eq!(game.offhand, None);
            assert_eq!(game.inventory[3], Some(sword.clone()));

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            assert!(
                zombie
                    .set_hand_item(game, Hand::Off, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
            assert_eq!(
                zombie
                    .with_hand_item(game, Hand::Off, |stack| stack.project())
                    .flatten(),
                Some(sword.try_into_api().unwrap())
            );
            assert!(zombie.with_hand_item(game, Hand::Main, |_| ()).is_none());

            game.mobs.clear();
            assert!(
                !zombie
                    .set_hand_item(game, Hand::Main, Some(sword.try_into_api().unwrap()))
                    .unwrap()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_equipment_writes_emit_vanilla_equip_events() {
        use sunrise_client::mobs::Mob;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(12).unwrap()).unwrap();
            game.audio.capture = Some(Vec::new());
            super::super::dispense_fixture::start_recording();
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet.clone())));
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, None));
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.mobs[0].first_tick = false;
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet.clone())));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, Some(helmet)));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            game.mobs[0].silent = true;
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Head,
                Some(ItemStack::new("minecraft:diamond_helmet", 1))
            ));
            assert!(game.living_set_equipment(living, EquipmentSlot::Head, None));
            game.mobs[0].silent = false;
            // Equippable, but its head slot does not match chest: event, no sound.
            assert!(game.living_set_equipment(
                living,
                EquipmentSlot::Chest,
                Some(ItemStack::new("minecraft:iron_helmet", 1))
            ));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            let events = super::super::dispense_fixture::stop_recording();
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.ends_with("minecraft:equip"))
                    .count(),
                3
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|e| e.ends_with("minecraft:unequip"))
                    .count(),
                1
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn stand_and_dragon_equipment_writes_run_on_equip_item() {
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let mut stand = sunrise_client::entities::ArmorStand::new(16, Vec3::ZERO, 0.0);
            stand.id = 16;
            game.armor_stands.push(stand);
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 17;
            game.dragon.replace(dragon);
            let stand = game.living(game.entity(16).unwrap()).unwrap();
            let dragon = game.living(game.entity(17).unwrap()).unwrap();
            game.audio.capture = Some(Vec::new());
            super::super::dispense_fixture::start_recording();
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            // Before its first tick an entity makes no sound and no event.
            assert!(game.living_set_equipment(stand, EquipmentSlot::Head, Some(helmet.clone())));
            assert!(game.living_set_equipment(stand, EquipmentSlot::Head, None));
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            game.tick_armor_stands().unwrap();
            game.dragon.as_mut().unwrap().ticks = 1;
            // An equippable item in its own slot: sound and `equip`; the same
            // item again changes nothing.
            for _ in 0..2 {
                assert!(game.living_set_equipment(
                    stand,
                    EquipmentSlot::Head,
                    Some(helmet.clone())
                ));
            }
            // A stored body armor: sound and `equip`, body is storage only.
            let body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            assert!(game.living_set_equipment(stand, EquipmentSlot::Body, Some(body)));
            // Not equippable: `unequip` and no sound.
            let stone = ItemStack::new("minecraft:stone", 1);
            assert!(game.living_set_equipment(stand, EquipmentSlot::Saddle, Some(stone.clone())));
            // The dragon the same.
            let chest = ItemStack::new("minecraft:iron_chestplate", 1);
            assert!(game.living_set_equipment(dragon, EquipmentSlot::Chest, Some(chest)));
            assert!(game.living_set_equipment(dragon, EquipmentSlot::Saddle, Some(stone)));
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 3);
            let events = super::super::dispense_fixture::stop_recording();
            let count = |name: &str| events.iter().filter(|e| e.ends_with(name)).count();
            assert_eq!(count("minecraft:equip"), 3);
            assert_eq!(count("minecraft:unequip"), 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mob_equipment_writes_keep_full_stacks() {
        use sunrise_client::mobs::gear;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let expect_slot = |game: &Game<'_>, living, slot, expected: Option<&ItemStack>| {
                assert_eq!(
                    game.living_equipment_stack(living, slot)
                        .map(|stack| stack.project())
                        .as_deref(),
                    expected,
                    "{slot:?}"
                );
            };
            for (id, kind) in [
                (12, "minecraft:zombie"),
                (13, "minecraft:horse"),
                (14, "minecraft:pillager"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let horse = game.living(game.entity(13).unwrap()).unwrap();
            let captain = game.living(game.entity(14).unwrap()).unwrap();
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 2);
            helmet.damage = 7;
            helmet.data_mut().enchantments = vec![("minecraft:protection".into(), 3)];
            assert_eq!(
                zombie.set_equipment(
                    game,
                    EquipmentSlot::Head,
                    Some(helmet.try_into_api().unwrap())
                ),
                Err(mod_api::StackWriteError::CountExceeded { maximum: 1 })
            );
            assert!(game.living_set_equipment(zombie, EquipmentSlot::Head, Some(helmet.clone())));
            let trim: gear::Trim = ("minecraft:gold".into(), "minecraft:coast".into());
            gear::set(
                &mut game.mobs[0],
                gear::HEAD,
                vec![("minecraft:protection".into(), 4)],
            );
            gear::set_trim(&mut game.mobs[0], gear::HEAD, Some(trim.clone()));
            let mut expected = helmet.clone();
            expected.data_mut().enchantments = vec![("minecraft:protection".into(), 4)];
            expected.data_mut().trim = Some(trim);
            assert_eq!(
                zombie.with_equipment(game, EquipmentSlot::Head, |stack| {
                    (stack.count(), stack.damage(), stack.project())
                }),
                Some((2, 7, Some(expected.try_into_api().unwrap())))
            );
            assert_eq!(
                game.mobs[0]
                    .extra
                    .slot_stack(sunrise_items::equipment::HEAD),
                Some(&helmet),
                "reading overlays must not write into the resident slot"
            );
            let mut shield = ItemStack::new("minecraft:shield", 2);
            shield.damage = 11;
            assert!(game.living_set_equipment(
                zombie,
                EquipmentSlot::OffHand,
                Some(shield.clone())
            ));
            expect_slot(game, zombie, EquipmentSlot::OffHand, Some(&shield));
            assert!(game.living_set_equipment(horse, EquipmentSlot::Head, Some(helmet.clone())));
            let mut body = ItemStack::new("minecraft:diamond_horse_armor", 3);
            body.damage = 9;
            body.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("armor".into()));
            assert!(game.living_set_equipment(horse, EquipmentSlot::Body, Some(body.clone())));
            expect_slot(game, horse, EquipmentSlot::Head, Some(&helmet));
            expect_slot(game, horse, EquipmentSlot::Body, Some(&body));
            for slot in [
                EquipmentSlot::MainHand,
                EquipmentSlot::Chest,
                EquipmentSlot::Legs,
                EquipmentSlot::Feet,
            ] {
                let stack = ItemStack::new("minecraft:stone", 4);
                assert!(game.living_set_equipment(zombie, slot, Some(stack.clone())));
                expect_slot(game, zombie, slot, Some(&stack));
                assert!(game.living_set_equipment(
                    zombie,
                    slot,
                    Some(ItemStack::new("minecraft:stone", 0))
                ));
                expect_slot(game, zombie, slot, None);
            }
            let mut saddle = ItemStack::new("minecraft:saddle", 3);
            saddle.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("saddle".into()));
            assert!(game.living_set_equipment(horse, EquipmentSlot::Saddle, Some(saddle.clone())));
            expect_slot(game, horse, EquipmentSlot::Saddle, Some(&saddle));
            sunrise_client::mobs::raider::set_patrol_leader(&mut game.mobs[2], true);
            sunrise_client::mobs::raider::wear_banner(&mut game.mobs[2]);
            assert!(sunrise_client::mobs::raider::is_captain(&game.mobs[2]));
            assert!(game.living_set_equipment(captain, EquipmentSlot::Head, Some(helmet.clone())));
            expect_slot(game, captain, EquipmentSlot::Head, Some(&helmet));
            assert!(!sunrise_client::mobs::raider::is_captain(&game.mobs[2]));
            assert!(game.living_set_equipment(
                horse,
                EquipmentSlot::Body,
                Some(ItemStack::new("minecraft:air", 1))
            ));
            expect_slot(game, horse, EquipmentSlot::Body, None);
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            // Reserve the dragon's eight consecutive part IDs away from the stand.
            dragon.id = 40;
            game.dragon.replace(dragon);
            let dragon = game.living(game.entity(40).unwrap()).unwrap();
            // The dragon and an armor stand store all eight slots, full
            // stacks included, and save them through the real codecs.
            let mut stand = sunrise_client::entities::ArmorStand::new(16, Vec3::ZERO, 0.0);
            stand.id = 16;
            game.armor_stands.push(stand);
            let stand = game.living(game.entity(16).unwrap()).unwrap();
            for living in [dragon, stand] {
                for slot in EquipmentSlot::ALL {
                    let mut stack = ItemStack::new("minecraft:stone", 3);
                    stack.damage = 5 + slot as u32;
                    stack.data_mut().custom_name =
                        Some(sunrise_core::nbt::Tag::String(slot_name(slot).into()));
                    assert!(game.living_set_equipment(living, slot, Some(stack.clone())));
                    expect_slot(game, living, slot, Some(&stack));
                }
            }
            let saved = game
                .dragon
                .as_ref()
                .unwrap()
                .to_nbt([0; 4], None, &Default::default());
            let loaded = super::super::dragon::Dragon::from_nbt(&saved, 0).unwrap();
            assert_eq!(loaded.equipment, game.dragon.as_ref().unwrap().equipment);
            let stored = game.armor_stands.last().unwrap();
            let saved = sunrise_client::entity_nbt::armor_stand_to_nbt(
                stored,
                [0; 4],
                None,
                &Default::default(),
            );
            let loaded = sunrise_client::entity_nbt::armor_stand_from_nbt(&saved, 16).unwrap();
            assert_eq!(loaded.equipment, stored.equipment);
            for living in [dragon, stand] {
                assert!(game.living_set_equipment(living, EquipmentSlot::Saddle, None));
                expect_slot(game, living, EquipmentSlot::Saddle, None);
            }
            // An empty dragon saves no `equipment` key, a stale one included.
            for slot in EquipmentSlot::ALL {
                assert!(game.living_set_equipment(dragon, slot, None));
            }
            let stale =
                game.dragon
                    .as_ref()
                    .unwrap()
                    .to_nbt([0; 4], Some(&saved), &Default::default());
            assert!(stale.get("equipment").is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_host_players_body_and_saddle_are_slots_and_saved_in_equipment() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let owner = Owner::Entity(Target::Player);
            let mut body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            body.damage = 4;
            body.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("armor".into()));
            let saddle = ItemStack::new("minecraft:saddle", 1);
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.audio.capture = Some(Vec::new());
            for (name, stack) in [("armor.body", &body), ("saddle", &saddle)] {
                let slots = game.slots_named(&owner, name);
                assert_eq!(slots.len(), 1, "{name} is one slot");
                assert!(game.slot_set(&slots[0], Some(stack.clone())));
            }
            assert_eq!(
                (game.equipment.get(6), game.equipment.get(7)),
                (Some(&body), Some(&saddle))
            );
            // onEquipItem: each item in its own slot plays its equip sound.
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 2);
            game.sync_level();
            let tag = game.level.player_tag(&Default::default());
            let equipment = tag.get("equipment").unwrap();
            assert!(equipment.get("body").is_some() && equipment.get("saddle").is_some());
            let mut read = sunrise_client::save::Level::default();
            read.read_player(&tag);
            // Loading the file back into a Game restores both, components too.
            game.equipment = Default::default();
            let level = read;
            game.load_saved_inventory(&level);
            assert_eq!(
                (game.equipment.get(6), game.equipment.get(7)),
                (Some(&body), Some(&saddle))
            );
        });
    }

    #[test]
    fn alive_is_health_above_zero_as_in_vanilla() {
        assert!(alive_at(0.5));
        assert!(!alive_at(0.0));
        assert!(!alive_at(-1.0));
        assert!(!alive_at(f32::NAN));
    }

    #[test]
    #[ignore = "daily"]
    fn armor_stand_first_tick_equipment_change_fires_after_base_tick() {
        use mod_api::{self, Hooks, LivingEquipmentChangeEvent, ModEntry, ServerHost};
        struct Reequip;
        #[mod_api::__subscriptions]
        impl Hooks for Reequip {
            fn living_equipment_change(
                &self,
                host: &mut dyn ServerHost,
                event: &mut LivingEquipmentChangeEvent<'_>,
            ) {
                if event.entity.id() == 18 && event.slot == EquipmentSlot::MainHand {
                    assert!(
                        event
                            .entity
                            .set_equipment(
                                host,
                                EquipmentSlot::Head,
                                Some(
                                    ItemStack::new("minecraft:iron_helmet", 1)
                                        .try_into_api()
                                        .unwrap()
                                ),
                            )
                            .unwrap()
                    );
                }
            }
        }
        mod_platform::testing::with_mods(
            vec![ModEntry::new("first_tick_equipment", Box::new(Reequip))].into(),
            || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    let mut stand = sunrise_client::entities::ArmorStand::new(
                        18,
                        Vec3::new(0.5, 70.0, 0.5),
                        0.0,
                    );
                    stand.no_gravity = true;
                    stand.equipment[0] = Some(ItemStack::new("minecraft:stone", 1));
                    game.armor_stands.push(stand);
                    game.audio.capture = Some(Vec::new());
                    super::super::dispense_fixture::start_recording();
                    game.tick_armor_stands().unwrap();
                    let events = super::super::dispense_fixture::stop_recording();
                    assert_eq!(
                        game.armor_stands[0].equipment[5],
                        Some(ItemStack::new("minecraft:iron_helmet", 1)),
                        "real listener/handle write control",
                    );
                    assert!(!game.armor_stands[0].first_tick);
                    let sounds = game.audio.capture.as_ref().unwrap().len();
                    let equips = events
                        .iter()
                        .filter(|e| e.ends_with("minecraft:equip"))
                        .count();
                    assert_eq!((sounds, equips), (1, 1));
                });
            },
        );
    }

    #[test]
    fn air_and_zero_count_are_empty_on_write() {
        assert!(equipment_stack(Some(ItemStack::new("minecraft:air", 1))).is_none());
        assert!(equipment_stack(Some(ItemStack::new("minecraft:stone", 0))).is_none());
        assert!(equipment_stack(Some(ItemStack::new("minecraft:stone", 1))).is_some());
    }

    #[test]
    fn a_players_slots_come_from_its_inventory_armour_and_off_hand() {
        let stack = |id: &str| Some(ItemStack::new(id, 1));
        let inventory = vec![stack("minecraft:stone"), stack("minecraft:dirt")];
        let armor = [
            stack("minecraft:iron_helmet"),
            None,
            None,
            stack("minecraft:iron_boots"),
        ];
        let offhand = stack("minecraft:shield");
        let mut equipment = sunrise_items::equipment::Equipment::default();
        equipment.set(sunrise_items::equipment::SADDLE, stack("minecraft:saddle"));
        let at = |slot| player_slot(&inventory, 1, &armor, &offhand, &equipment, slot);
        let id = |slot| at(slot).map(|s| s.item.as_ref());
        assert!(std::ptr::eq(
            at(EquipmentSlot::MainHand).unwrap(),
            inventory[1].as_ref().unwrap()
        ));
        assert_eq!(id(EquipmentSlot::MainHand), Some("minecraft:dirt"));
        assert_eq!(id(EquipmentSlot::OffHand), Some("minecraft:shield"));
        assert_eq!(id(EquipmentSlot::Head), Some("minecraft:iron_helmet"));
        assert_eq!(id(EquipmentSlot::Feet), Some("minecraft:iron_boots"));
        assert_eq!(id(EquipmentSlot::Chest), None);
        assert_eq!(id(EquipmentSlot::Body), None);
        assert_eq!(id(EquipmentSlot::Saddle), Some("minecraft:saddle"));
    }

    #[test]
    #[ignore = "daily"]
    fn a_wolf_throws_and_reads_tamed_and_owner() {
        use sunrise_client::{chunk::Chunk, entities::ArmorStand};
        use sunrise_core::geometry::Vec3;

        super::super::native_entity_refs::tests::with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
            let id = game
                .spawn_mob(
                    "minecraft:wolf",
                    [2.5, 64.0, 2.5],
                    true,
                    super::super::creatures::SpawnSite::Finalizing(
                        sunrise_client::mobs::spawn_rules::Reason::Command,
                    ),
                )
                .unwrap();
            let wolf = game.living(game.entity(id).unwrap()).unwrap();
            assert_eq!(game.living_is_tamed(wolf), Some(false));
            assert_eq!(game.living_owner(wolf), None);
            let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            (m.extra.tame, m.extra.owner) = (true, Some([1, 2, 3, 4]));
            assert_eq!(game.living_is_tamed(wolf), Some(true));
            assert_eq!(
                game.living_owner(wolf),
                Some(0x1_0000_0002_0000_0003_0000_0004)
            );

            let bone = ItemStack::new("minecraft:bone", 1);
            assert!(
                game.living_drop(wolf, ItemStack::new("minecraft:air", 1), false, true)
                    .is_none()
            );
            let item = game.living_drop(wolf, bone.clone(), false, true).unwrap();
            assert_eq!(
                game.entity_kind(item)
                    .and_then(|kind| kind.id(game))
                    .as_deref(),
                Some("minecraft:item")
            );
            let wolf_uuid = game.uuid_for(id);
            let thrown = game.dropped.last().unwrap();
            let eye = game.mobs.iter().find(|m| m.id == id).unwrap().eye_y();
            assert_eq!(thrown.position.y, eye - f64::from(0.3f32));
            assert_eq!((thrown.pickup_delay, thrown.thrower), (40, Some(wolf_uuid)));
            // The drop swung: one tick on, that swing goes on.
            let m = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
            assert!(m.ai.swinging);
            m.ai.swing_time = 1;
            assert!(!game.living_swing(wolf, Hand::Main, true));
            let scattered = game.living_drop(wolf, bone, true, false).unwrap();
            assert_ne!(scattered, item);
            assert_eq!(game.dropped.last().unwrap().velocity.y, f64::from(0.2f32));
            assert_eq!(game.dropped.last().unwrap().thrower, None);

            game.armor_stands
                .push(ArmorStand::new(90, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let stand = game.living(game.entity(90).unwrap()).unwrap();
            assert_eq!(game.living_is_tamed(stand), None);
            let apple = ItemStack::new("minecraft:apple", 1);
            assert!(game.living_drop(stand, apple, false, false).is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_mod_drop_and_drop_key_have_equivalent_motion() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.yaw = 37.0;
            game.pitch = -21.0;
            *game.world.random = sunrise_core::random::LegacyRandom::new(12345);
            let item = ItemStack::new("minecraft:apple", 1);
            game.inventory[game.selected] = Some(item.clone());
            let player_random = game.player.as_ref().unwrap().random.clone();
            let (_, seeds) =
                sunrise_client::random_source::record_seeds(|| game.drop_held_item(true));
            let key_velocity = game.dropped.last().unwrap().velocity;
            *game.world.random = sunrise_core::random::LegacyRandom::new(12345);
            crate::game::native_entity_refs::tests::bind_local_player(game, 42, 123);
            let player = game.living(game.entity(42).unwrap()).unwrap();
            game.player.as_mut().unwrap().random = player_random;
            let id = sunrise_client::random_source::replay_seeds(seeds.clone(), || {
                game.living_drop(player, item, false, true).unwrap().id()
            });
            assert!(
                seeds.complete(),
                "both drop routes allocate the same random streams"
            );
            let mod_velocity = game
                .dropped
                .iter()
                .find(|entity| entity.id == id)
                .unwrap()
                .velocity;
            assert_eq!(key_velocity, mod_velocity);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_drop_spread_uses_the_players_random_even_when_cancelled() {
        use mod_api::{Flow, Hooks, ItemTossEvent, ModEntry, ServerHost};
        use sunrise_core::{
            geometry::trig::{cos, sin},
            random::RandomSource,
        };
        struct Veto;
        #[mod_api::__subscriptions]
        impl Hooks for Veto {
            fn item_toss(&self, _: &mut dyn ServerHost, _: &mut ItemTossEvent) -> Flow {
                Flow::Cancel
            }
        }
        for (randomly, cancelled) in [(false, false), (true, false), (false, true)] {
            let mods: mod_api::Mods = if cancelled {
                vec![ModEntry::new("veto", Box::new(Veto))].into()
            } else {
                Vec::<ModEntry>::new().into()
            };
            mod_platform::testing::with_mods(mods, || {
                super::super::native_entity_refs::tests::with_game(|game| {
                    game.player = Some(
                        sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap(),
                    );
                    super::super::native_entity_refs::tests::bind_local_player(game, 42, 123);
                    game.yaw = 0.0;
                    game.pitch = 0.0;
                    game.player.as_mut().unwrap().random.set_seed(12345);
                    let mut expected = game.player.as_ref().unwrap().random.clone();
                    // Independent vanilla oracle at yaw=pitch=0. The item's
                    // constructor owns a separate stream, not these floats.
                    let velocity = if randomly {
                        let strength = expected.next_float() * 0.5;
                        let direction = f64::from(expected.next_float() * std::f32::consts::TAU);
                        [
                            f64::from(-sin(direction) * strength),
                            f64::from(0.2f32),
                            f64::from(cos(direction) * strength),
                        ]
                    } else {
                        let direction = f64::from(expected.next_float() * std::f32::consts::TAU);
                        let strength = f64::from(expected.next_float() * 0.02);
                        let jitter = (expected.next_float() - expected.next_float()) * 0.1;
                        [
                            sunrise_core::jmath::cos(direction) * strength,
                            f64::from(0.1 + jitter),
                            f64::from(0.3f32) + sunrise_core::jmath::sin(direction) * strength,
                        ]
                    };
                    let level = game.world.random.state();
                    let before = game.dropped.len();
                    let id = game.native_player_drop(
                        ItemStack::new("minecraft:apple", 1),
                        randomly,
                        true,
                    );
                    assert_eq!(
                        game.player.as_ref().unwrap().random.state(),
                        expected.state(),
                        "randomly={randomly} cancelled={cancelled}"
                    );
                    assert_eq!(game.world.random.state(), level, "no level random draws");
                    if cancelled {
                        assert!(id.is_none());
                        assert_eq!(game.dropped.len(), before);
                    } else {
                        let drop = game.dropped.iter().find(|d| Some(d.id) == id).unwrap();
                        assert_eq!(
                            [drop.velocity.x, drop.velocity.y, drop.velocity.z],
                            velocity
                        );
                    }
                });
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn armor_stand_drop_does_not_swing() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.armor_stands
                .push(sunrise_client::entities::ArmorStand::new(
                    90,
                    sunrise_core::geometry::Vec3::ZERO,
                    0.0,
                ));
            let stand = game.living(game.entity(90).unwrap()).unwrap();
            let before = game.swings_started;
            assert!(
                game.living_drop(stand, ItemStack::new("minecraft:apple", 1), false, false)
                    .is_none()
            );
            assert_eq!(game.swings_started, before);
            assert!(game.dropped.is_empty());
        });
    }

    #[test]
    fn every_equipment_slot_has_its_own_item_command_name() {
        let names = EquipmentSlot::ALL.map(slot_name);
        for (i, n) in names.iter().enumerate() {
            assert!(!names[..i].contains(n), "{n} named twice");
        }
    }
    /// The living reads of #943 answer the engine's own decisions.
    #[test]
    #[ignore = "daily"]
    fn the_engines_own_portal_melee_freeze_and_air_answers() {
        use mod_api::Host;
        use sunrise_client::mobs::Mob;
        use sunrise_client::motion::PlayerMotion;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            // Two zombies an arm's length apart, and one a stride away.
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(30, zombie.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(31, zombie.clone(), Vec3::new(1.5, 64.0, 0.5), 0.0));
            game.mobs
                .push(Mob::new(32, zombie, Vec3::new(5.5, 64.0, 0.5), 0.0));
            let near = game.living(game.entity(30).unwrap()).unwrap();
            let beside = game.entity(31).unwrap();
            let far = game.entity(32).unwrap();

            // The walk gate's own answers: alive and travelling, until dead.
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), Some(true));
            }
            game.mobs[0].health = 0.0;
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), Some(false));
            }
            game.mobs[0].health = 20.0;

            // `LivingEntity.canUsePortal`'s sleep half: a villager asleep in
            // its bed does not travel, awake again it does.
            let villager = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(35, villager, Vec3::new(11.5, 64.0, 0.5), 0.0));
            let sleeper = game.living(game.entity(35).unwrap()).unwrap();
            game.mobs.last_mut().unwrap().extra.ints[1] = 1;
            {
                let host: &dyn Host = game;
                assert_eq!(sleeper.can_use_portal(host), Some(false));
            }
            game.mobs.last_mut().unwrap().extra.ints[1] = 0;
            {
                let host: &dyn Host = game;
                assert_eq!(sleeper.can_use_portal(host), Some(true));
            }

            // The melee goal's own reach: beside it, yes; a stride away, no.
            // The freeze tick's decision, and a freeze-immune wearable on it.
            // A zombie's type is in the vanilla water-breathing tag.
            {
                let host: &dyn Host = game;
                assert_eq!(near.in_melee_range(host, beside), Some(true));
                assert_eq!(near.in_melee_range(host, far), Some(false));
                assert_eq!(near.can_freeze(host), Some(true));
                assert_eq!(near.can_breathe_underwater(host), Some(true));
            }
            game.mobs[0].extra.equipment.set(
                sunrise_items::equipment::FEET,
                Some(sunrise_client::items::ItemStack::new(
                    "minecraft:leather_boots",
                    1,
                )),
            );
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_freeze(host), Some(false));
            }

            // A player's own answers: alive and awake, freezable, drowning.
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_use_portal(host), Some(true));
                assert_eq!(player.can_freeze(host), Some(true));
                assert_eq!(player.can_breathe_underwater(host), Some(false));
            }
            game.player.as_mut().unwrap().sleeping = true;
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_use_portal(host), Some(false));
            }
            game.player.as_mut().unwrap().sleeping = false;
            game.player.as_mut().unwrap().freeze_immune = true;
            {
                let host: &dyn Host = game;
                assert_eq!(player.can_freeze(host), Some(false));
            }

            let squid = game.res.mob_types().get("minecraft:squid").unwrap().clone();
            game.mobs
                .push(Mob::new(33, squid, Vec3::new(7.5, 64.0, 0.5), 0.0));
            let squid = game.living(game.entity(33).unwrap()).unwrap();
            let golem_kind = game
                .res
                .mob_types()
                .get("minecraft:copper_golem")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(34, golem_kind, Vec3::new(9.5, 64.0, 0.5), 0.0));
            let golem = game.living(game.entity(34).unwrap()).unwrap();
            {
                let host: &dyn Host = game;
                // A squid and a copper golem (#can_breathe_under_water)
                // breathe water.
                assert_eq!(squid.can_breathe_underwater(host), Some(true));
                assert_eq!(golem.can_breathe_underwater(host), Some(true));
            }
            game.mobs.clear();
            {
                let host: &dyn Host = game;
                assert_eq!(near.can_use_portal(host), None);
                assert_eq!(near.in_melee_range(host, beside), None);
                assert_eq!(near.can_freeze(host), None);
                assert_eq!(near.can_breathe_underwater(host), None);
            }
        });
    }

    /// A sleeper's bed orientation is the bed block's own facing (#943):
    /// the block at the sleeping position, where it is one of `#minecraft:beds`.
    #[test]
    #[ignore = "daily"]
    fn a_sleepers_bed_orientation() {
        use mod_api::{Direction, Host};
        use sunrise_client::mobs::Mob;
        use sunrise_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            // A floor with a red bed on it, facing north.
            for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
                let mut chunk = sunrise_client::chunk::Chunk::empty(x, z);
                chunk.lit = true;
                game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
            }
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in -8..8 {
                for z in -8..8 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let bed = game
                .pack
                .assets
                .registry
                .default_state("minecraft:red_bed")
                .unwrap();
            game.world.set(2, 63, 2, bed).unwrap();
            game.world.set(3, 63, 2, bed).unwrap();

            let villager = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(30, villager, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let sleeper = game.living(game.entity(30).unwrap()).unwrap();
            let host: &dyn Host = game;
            // Awake: no sleeping position, no bed.
            assert_eq!(sleeper.bed_orientation(host), None);
            // Asleep in the bed: the block's own facing, the engine's own
            // sleeping position naming the bed.
            game.mobs[0].extra.ints[1] = 1;
            sunrise_client::mobs::villager::ext_mut(&mut game.mobs[0]).sleeping_pos =
                Some([2, 63, 2]);
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), Some(Direction::North));
            assert_eq!(sleeper.can_use_portal(host), Some(false));
            // A sleeping position that is not a bed answers none.
            sunrise_client::mobs::villager::ext_mut(&mut game.mobs[0]).sleeping_pos =
                Some([0, 63, 0]);
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), None);
            // A player asleep in the same bed answers the same facing.
            game.player =
                Some(sunrise_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.sleeping = Some([2, 63, 2]);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(player.bed_orientation(host), Some(Direction::North));
            // Fox sleeping overrides LivingEntity.isSleeping. These species
            // may all travel awake; the fox sleep flag vetoes portal use.
            for name in ["minecraft:fox", "minecraft:bat", "minecraft:cat"] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                let mut mob = Mob::new(31, kind, Vec3::ZERO, 0.0);
                assert!(sunrise_client::mobs::mob_can_use_portal(&mob));
                if name == "minecraft:fox" {
                    mob.extra.ints[0] |= sunrise_client::mobs::fox::SLEEPING;
                    assert!(!sunrise_client::mobs::mob_can_use_portal(&mob));
                }
            }
            assert!(!game.mob_portal_eligible(&game.mobs[0]));
            game.mobs.clear();
            let host: &dyn Host = game;
            assert_eq!(sleeper.bed_orientation(host), None);
        });
    }

    /// #943, review: a guest answers the same one portal rule over their
    /// body and vehicle (`can_use_portal_of`), not a hand-rolled copy.
    #[test]
    #[ignore = "daily"]
    fn a_guest_answers_the_one_portal_rule() {
        use mod_api::Host;
        super::super::native_guest_turn_tests::with_guests(|game| {
            // The fixture leaves the guest without a body: give one, as a
            // joined player's server record keeps.
            game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                g.motion = Some(Box::new(
                    sunrise_client::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::new(3.0, 64.0, 5.0),
                    )
                    .unwrap(),
                ))
            });
            let living = game.living(game.entity(12).unwrap()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(true));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = Some(99));
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = None);
            // Asleep: the rule's sleep half answers, over their body.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.sleeping = true);
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
            // A passenger crosses with its vehicle, not as its own
            // traveller: the rule's riding half answers false.
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.sleeping = false);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| g.vehicle = Some(60));
            let host: &dyn Host = game;
            assert_eq!(living.can_use_portal(host), Some(false));
        });
    }
}
