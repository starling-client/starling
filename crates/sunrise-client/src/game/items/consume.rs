//! Finishing a consumable (`Consumable.onConsume` and the use remainder),
//! its particles and sounds, and `TeleportRandomlyConsumeEffect` (chorus
//! fruit) for players and foxes. The level random stands in for the player's
//! own, as for every other player draw.
use super::{Dim, Game};
use crate::game::api_values::TryIntoApi;
use crate::game::collision_source::Mover;
use std::convert::Infallible;
use sunrise_client::{
    items::{ConsumeEffect, ItemInfo, ItemStack},
    mobs::Who,
    particles::{ParticleOptions, ParticleType},
};
use sunrise_core::{
    geometry::Vec3, position::BlockPos, random::LegacyRandom, random::RandomSource,
};
use {
    sunrise_core::use_animation::UseAnimation,
    sunrise_world::{
        consumable::{Consumable, EmissionWorld},
        teleport_consume::{TeleportConsume, TeleportConsumeWorld},
    },
};

type ItemConsumable = Consumable<String, ConsumeEffect>;

/// Capture once per effect, not once per attempted destination. If the stack
/// cannot cross the API boundary (for example, an unregistered saved item), skip
/// the listener for this consumption and let the vanilla teleport proceed.
fn consumed_snapshot(stack: &ItemStack) -> Option<mod_api::ItemStack> {
    if !mod_api::__private::listened(mod_api::HookId::ItemConsumptionTeleport) {
        return None;
    }
    stack.try_into_api().ok()
}

/// `EntityEvent.USE_ITEM_COMPLETE`: the server finished the player's use.
pub(super) const USE_ITEM_COMPLETE_EVENT: i8 = 9;

impl Game<'_> {
    /// The consuming player, including the local player outside a guest action.
    /// Before a world is published the handle is stale, but the effect still runs.
    pub(super) fn consuming_player_entity(&self) -> mod_api::EntityRef {
        mod_api::EntityRef::new(
            self.native_lifecycle
                .epoch()
                .unwrap_or(mod_api::WorldEpoch::new(0)),
            if self.lan_acting.is_some() {
                self.native_acting_entity()
            } else {
                self.own_entity_id() as u32
            },
        )
    }

    /// `stack.get(DataComponents.CONSUMABLE)`: item prototype, data patch or wire patch.
    pub(super) fn consumable(&self, stack: &ItemStack) -> Option<ItemConsumable> {
        sunrise_client::item_sync::consumable(stack, self.items())
    }

    /// `ServerPlayer.completeUsingItem`: the player's own client is told
    /// first, and completes the use itself (eating the food there too).
    pub(super) fn send_use_item_complete(&mut self) {
        if self.lan_acting.is_some() {
            self.guest_entity_events.push(USE_ITEM_COMPLETE_EVENT);
        }
    }
    /// `Consumable.emitParticlesAndSounds(user.getRandom(), user, stack, count)`.
    pub(super) fn emit_consume(
        &mut self,
        consumable: &ItemConsumable,
        stack: &ItemStack,
        count: i32,
    ) {
        let Ok(()) = consumable.emit(&mut Emitter(self), (), stack.clone(), count);
    }
    /// `LivingEntity.completeUsingItem` for a consumable: `Consumable.onConsume`,
    /// then `ItemStack.applyAfterUseComponentSideEffects`' use remainder.
    pub(super) fn finish_consuming(
        &mut self,
        info: &ItemInfo,
        consumable: &ItemConsumable,
        stack: &ItemStack,
    ) {
        let slot = self.held_slot();
        self.emit_consume(consumable, stack, 16);
        // ServerPlayer: the used statistic and the consume_item trigger.
        self.award_stat("used", &stack.item, 1);
        self.trigger_item("minecraft:consume_item", stack);
        // FoodProperties.onConsume, the player's one ConsumableListener.
        if let Some((nutrition, saturation)) = sunrise_client::item_sync::food(stack, info.food) {
            let at = self.feet();
            let pitch = self.world.random.triangle_float(1.0, 0.4);
            self.audio.play(&consumable.sound, at, 1.0, pitch);
            if let Some(p) = self.player.as_mut() {
                // FoodData.eat(nutrition, saturation)
                let food = &mut p.health.food;
                food.food = (food.food + nutrition as i32).clamp(0, 20);
                food.saturation = (food.saturation + saturation).clamp(0.0, food.food as f32);
            }
            // Mth.randomBetween(random, 0.9, 1.0)
            let pitch = self.world.random.next_float() * (1.0f32 - 0.9f32) + 0.9;
            self.audio
                .play("minecraft:entity.player.burp", self.feet(), 0.5, pitch);
        }
        // SuspiciousStewEffects.onConsume, a ConsumableListener: each entry is
        // added as a `MobEffectInstance(effect, duration)` (amplifier 0).
        if let Some(data) = stack.extra.as_deref() {
            for (id, duration) in &data.stew_effects {
                if let Some(def) = sunrise_client::brewing::effect(id) {
                    self.add_player_effect(sunrise_client::effects::EffectInstance::new(
                        def.id, *duration, 0,
                    ));
                }
            }
        }
        // Consumable.onConsume applies these entries on the logical server only;
        // the client's listener, particles and sounds above still run there.
        self.apply_consumable_effects(stack, &consumable.effects, None);
        let event = if consumable.animation == UseAnimation::Drink {
            "minecraft:drink"
        } else {
            "minecraft:eat"
        };
        self.post_entity_event(event, Who::Player);
        // ItemStack.consume, then UseRemainder.convertIntoRemainder: nothing
        // with infinite materials, the hand when emptied, else the inventory
        // (ServerPlayer.handleExtraItemsCreatedOnUse) or a drop. A listener may
        // replace the stack or change the selected slot; neither is this use.
        if self.held_slot() != slot || self.stack_in(slot) != Some(stack) {
            return;
        }
        self.consume_held();
        self.apply_use_remainder(stack);
    }
    /// `ItemStack.applyAfterUseComponentSideEffects` uses the stack captured
    /// before the item handler, even when that handler changes the hand.
    pub(super) fn apply_after_use_components(&mut self, stack: &ItemStack) {
        self.apply_use_remainder(stack);
        self.apply_use_cooldown(stack);
    }
    fn apply_use_remainder(&mut self, used: &ItemStack) {
        if self.creative || self.held().map_or(0, |stack| stack.count) >= used.count {
            return;
        }
        if let Some(remainder) =
            sunrise_client::item_sync::use_remainder(used, self.items().get(&used.item))
        {
            if self.held().is_none() {
                *self.held_mut() = Some(remainder);
            } else if let Some(rest) = self.add_stack_to_inventory(remainder) {
                self.player_drop(rest);
            }
        }
    }
    fn feet(&self) -> Option<[f64; 3]> {
        self.player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
    }
    /// `TeleportRandomlyConsumeEffect.apply` for the local player.
    pub(super) fn teleport_randomly(
        &mut self,
        teleport: &TeleportConsume,
        consumed: &ItemStack,
    ) -> bool {
        if self.player.is_none() {
            return false;
        }
        let dim = self.dim;
        let consumed = consumed_snapshot(consumed);
        let Ok(result) = teleport.apply(&mut Teleporter(self, consumed.as_ref()), dim);
        result
    }
    /// Apply the paused fox item use with the full host, then leave its tick
    /// continuation to the owning mob loop. Callback writes and apply share a transaction.
    pub(super) fn apply_fox_consumption(
        &mut self,
        decision: &sunrise_client::mobs::fox::Consumption,
    ) -> bool {
        if !self.mobs.iter().any(|m| decision.still_valid(m)) {
            return false;
        }
        let Some(consumer) = self.entity(decision.mob).and_then(|e| self.living(e)) else {
            return false;
        };
        let Some(consumable) = self.consumable(&decision.stack) else {
            return false;
        };
        let failures = mod_api::runtime::isolated_failures();
        let result: Result<bool, ()> = mod_api::run_transaction(self, |game, _| {
            {
                let mut mobs = game.mobs.lift();
                if let Some(mob) = mobs.iter_mut().find(|m| m.id == decision.mob) {
                    let mut events = Vec::new();
                    sunrise_client::mobs::fox::consumed_before_effects(
                        &game.mob_world_base(&[]),
                        mob,
                        &mut events,
                        &decision.stack,
                    );
                    game.mobs
                        .restore(mobs, sunrise_client::join_seq::RestoreOrder::SpawnsFirst);
                    if game.handle_mob_events(events).is_err() {
                        return if mod_api::runtime::isolated_failures() != failures {
                            Err(())
                        } else {
                            Ok(false)
                        };
                    }
                } else {
                    game.mobs
                        .restore(mobs, sunrise_client::join_seq::RestoreOrder::SpawnsFirst);
                    return Ok(false);
                }
                let can_finish = game.apply_consumable_effects(
                    &decision.stack,
                    &consumable.effects,
                    Some((consumer, MobConsumption::Fox(decision))),
                );
                if mod_api::runtime::isolated_failures() != failures {
                    return Err(());
                }
                // A refusal (for example, a listener moved the fox under the
                // pending teleport) still invalidates the original finish.
                if !can_finish {
                    return Ok(false);
                }
                if let Some(mob) = game
                    .mobs
                    .iter()
                    .find(|m| m.id == decision.mob && m.alive() && !m.removed)
                {
                    let event = sunrise_client::mobs::MobEvent::GameEvent {
                        event: if consumable.animation == UseAnimation::Drink {
                            "minecraft:drink"
                        } else {
                            "minecraft:eat"
                        },
                        at: [mob.position.x, mob.position.y, mob.position.z],
                        source: mob.id,
                        state: sunrise_world::registry::BlockStateId(0),
                    };
                    if game.handle_mob_events(vec![event]).is_err() {
                        return if mod_api::runtime::isolated_failures() != failures {
                            Err(())
                        } else {
                            Ok(false)
                        };
                    }
                }
            }
            if mod_api::runtime::isolated_failures() != failures {
                return Err(());
            }
            let Some(m) = game.mobs.iter_mut().find(|m| m.id == decision.mob) else {
                return Ok(false);
            };
            let position = m.position;
            let info = game.res.items().get(&decision.stack.item);
            Ok(decision.finish(m, position, info))
        });
        result.unwrap_or(false)
    }

    /// Run the original user's effective effect list, never re-reading its hand.
    /// If a built-in can no longer act, continue the list but refuse the finish.
    fn apply_consumable_effects(
        &mut self,
        stack: &ItemStack,
        effects: &[ConsumeEffect],
        mob_use: Option<(mod_api::LivingRef, MobConsumption<'_>)>,
    ) -> bool {
        if !self.is_logical_server() || effects.is_empty() {
            return true;
        }
        let Some((consumer, usage)) = mob_use else {
            let user = self.consuming_player_entity();
            for effect in effects {
                self.apply_consume_effect(stack, effect, None, user);
            }
            return true;
        };
        let id = consumer.entity().id();
        let dim = self.dim;
        let consumed = std::cell::OnceCell::new();
        let mut can_finish = true;
        for effect in effects {
            match effect {
                ConsumeEffect::PlaySound(_) | ConsumeEffect::Native { .. } => {
                    can_finish &= self
                        .apply_consume_effect(stack, effect, Some(consumer), consumer.entity())
                        .is_some();
                }
                ConsumeEffect::TeleportRandomly(teleport) => {
                    let consumed = consumed.get_or_init(|| consumed_snapshot(stack));
                    let mut actor = MobTeleporter {
                        game: self,
                        consumer,
                        id,
                        usage,
                        dim,
                        consumed: consumed.as_ref(),
                    };
                    can_finish &= teleport.apply(&mut actor, dim).is_ok();
                }
                _ => {
                    if let Some(mob) = self
                        .mobs
                        .iter_mut()
                        .find(|m| m.id == id && m.alive() && !m.removed)
                    {
                        mob.apply_consume_effect(effect);
                    }
                }
            }
        }
        can_finish
    }

    /// Specialized player drinks have already applied their built-in component listener.
    pub(super) fn apply_drink_effects(&mut self, stack: &ItemStack) {
        if let Some(consumable) = self.consumable(stack) {
            self.apply_consumable_effects(stack, &consumable.effects, None);
        }
    }

    /// A witch or trader applied its ordinary drink behavior during AI. The
    /// owning game now runs the list with the living host, before clearing the hand.
    pub(super) fn finish_mob_drink(&mut self, id: u32, stack: &ItemStack) {
        let usage = MobConsumption::Drink(stack);
        if !self
            .mobs
            .iter()
            .any(|mob| mob.id == id && usage.still_valid(mob, mob.position))
        {
            return;
        }
        let Some(consumer) = self.entity(id).and_then(|entity| self.living(entity)) else {
            return;
        };
        let consumable = self.consumable(stack);
        let failures = mod_api::runtime::isolated_failures();
        let result: Result<(), ()> = mod_api::run_transaction(self, |game, _| {
            if let Some(consumable) = &consumable
                && !game.apply_consumable_effects(
                    stack,
                    &consumable.effects,
                    Some((consumer, usage)),
                )
            {
                return if mod_api::runtime::isolated_failures() != failures {
                    Err(())
                } else {
                    Ok(())
                };
            }
            if mod_api::runtime::isolated_failures() != failures {
                return Err(());
            }
            if let Some(mob) = game
                .mobs
                .iter_mut()
                .find(|mob| mob.id == id && usage.still_valid(mob, mob.position))
            {
                mob.extra.set_main_hand_stack(None);
            }
            Ok(())
        });
        let _ = result;
    }

    /// `LivingEntity.randomTeleport(x, y, z, true, avoid)`: down to the first
    /// block entities can teleport onto, inside the border, in a loaded chunk.
    pub(super) fn random_teleport(&mut self, target: Vec3, avoid: Option<&str>) -> bool {
        let [min_x, min_z, max_x, max_z] = self.border.bounds();
        // WorldBorder.clampVec3ToBound (Mth.clamp, 1.0E-5F inside the far edge).
        let clamp = |v: f64, low: f64, high: f64| if v < low { low } else { v.min(high) };
        let x = clamp(target.x, min_x, max_x - f64::from(1.0e-5_f32));
        let z = clamp(target.z, min_z, max_z - f64::from(1.0e-5_f32));
        let mut y = target.y;
        let mut pos = BlockPos::containing(Vec3::new(x, y, z));
        if !self.world.loaded(pos.x, pos.z) {
            return false;
        }
        while pos.y > self.world.shape.min_y {
            pos.y -= 1;
            let state = self.world.block(pos.x, pos.y, pos.z);
            if self.state_in_tag("minecraft:entities_can_teleport_to", state) {
                return self.check_position_and_teleport(Vec3::new(x, y, z), state, avoid);
            }
            y -= 1.0;
        }
        false
    }
    /// `LivingEntity.checkPositionAndTeleport`, then `ServerPlayer.teleportTo`
    /// (the motion and rotation kept) and entity event 46.
    fn check_position_and_teleport(
        &mut self,
        at: Vec3,
        ground: sunrise_world::registry::BlockStateId,
        avoid: Option<&str>,
    ) -> bool {
        if avoid.is_some_and(|tag| self.state_in_tag(tag, ground)) {
            return false;
        }
        let entities = self.entity_collisions(Mover::Player);
        let Some(player) = self.player.as_mut() else {
            return false;
        };
        let bounds = player.dimensions.bounds(at);
        let side = game_side!(self);
        let free = sunrise_client::motion::MotionWorld {
            side: &side,
            entities: &entities,
            player,
            scene: &mut self.world,
            assets: &self.pack.assets,
            collision_binding: self.collision,
            resources: self.motion,
            health: &self.health_resources,
            loop_context: &self.loop_context,
            damage_hooks: &self.damage_hooks,
            tag_items: Some(world_items!(self)),
            block_registries: Some(&self.pack.block_registries),
        }
        .no_player_collision(bounds)
        .unwrap_or(false);
        // LevelReader.containsAnyLiquid and findBlocksIn: the blocks from the
        // box's minimum corner to the one holding its maximum corner.
        let (lo, hi) = (
            BlockPos::containing(bounds.min_position()),
            BlockPos::containing(bounds.max_position()),
        );
        let blocks = (lo.x..=hi.x)
            .flat_map(|x| (lo.y..=hi.y).flat_map(move |y| (lo.z..=hi.z).map(move |z| (x, y, z))));
        let states: Vec<_> = blocks.map(|(x, y, z)| self.world.block(x, y, z)).collect();
        let liquid = states
            .iter()
            .any(|&s| self.res.facts().get(s).fluid != sunrise_client::blocks::Fluid::None);
        if !free
            || liquid
            || avoid.is_some_and(|tag| states.iter().any(|&s| self.state_in_tag(tag, s)))
        {
            return false;
        }
        if let Some(p) = self.player.as_mut() {
            p.position = at;
        }
        // The position packet moves the old position along (absolute xyz).
        self.previous_position = at;
        // `broadcastEntityEvent(46)` after the teleport. A server-thread window
        // is a remote client, so the packet is what draws the particles there.
        if let Some(id) = self.lan_acting_entity {
            self.lan_entity_event(id as i32, 46, [at.x, at.y, at.z]);
        } else {
            self.portal_particles();
        }
        true
    }
    fn state_in_tag(&self, tag: &str, state: sunrise_world::registry::BlockStateId) -> bool {
        let name = &self.res.facts().get(state).name;
        self.res
            .items()
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(&**name))
    }
    /// `LivingEntity.handleEntityEvent(46)` for this client: 128 portal
    /// particles from the old position to the new one. The local player's
    /// random is the particle engine's.
    fn portal_particles(&mut self) {
        let (from, to, width, height) = {
            let Some(p) = self.player.as_ref() else {
                return;
            };
            (
                [
                    self.previous_position.x,
                    self.previous_position.y,
                    self.previous_position.z,
                ],
                [p.position.x, p.position.y, p.position.z],
                f64::from(p.dimensions.width),
                f64::from(p.dimensions.height),
            )
        };
        self.emit_living_teleport(from, to, width, height);
    }
    /// `LivingEntity.handleEntityEvent(46)` for the entity the packet names:
    /// this player, or a mob the client is drawing.
    pub(super) fn entity_teleport_particles(&mut self, id: i32) {
        if self.remote.as_ref().is_some_and(|r| r.entity_id == id) {
            self.portal_particles();
            return;
        }
        let spawned = {
            let Some(m) = self.mobs.iter_mut().find(|m| m.id == id as u32) else {
                return;
            };
            let from = [m.previous.x, m.previous.y, m.previous.z];
            let to = [m.position.x, m.position.y, m.position.z];
            let (width, height) = (f64::from(m.width()), f64::from(m.height()));
            let mut spawned = Vec::with_capacity(128);
            sunrise_client::particles::living_teleport_particles(
                &mut m.random,
                from,
                to,
                width,
                height,
                |p, v| spawned.push((p, v)),
            );
            spawned
        };
        let options = ParticleOptions::simple(ParticleType::Portal);
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    fn emit_living_teleport(&mut self, from: [f64; 3], to: [f64; 3], width: f64, height: f64) {
        let mut spawned = Vec::with_capacity(128);
        sunrise_client::particles::living_teleport_particles(
            self.particles.random(),
            from,
            to,
            width,
            height,
            |p, v| spawned.push((p, v)),
        );
        let options = ParticleOptions::simple(ParticleType::Portal);
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// `LivingEntity.stopRiding`.
    pub(super) fn stop_riding(&mut self) {
        let Some(id) = self.riding else {
            return;
        };
        let removed_y = self
            .persist
            .removed_vehicles
            .get(&id)
            .map(|vehicle| vehicle.position().y)
            .or_else(|| {
                use super::commands::Target;
                let boat = Target::Boat(id);
                let cart = Target::Cart(id);
                if self.persist.removals.contains(&boat)
                    || self.persist.destroying_vehicles.contains(&boat)
                {
                    self.boats
                        .iter()
                        .find(|vehicle| vehicle.id == id)
                        .map(|vehicle| vehicle.position.y)
                } else if self.persist.removals.contains(&cart)
                    || self.persist.destroying_vehicles.contains(&cart)
                {
                    self.carts
                        .iter()
                        .find(|vehicle| vehicle.id == id)
                        .map(|vehicle| vehicle.position.y)
                } else {
                    None
                }
            });
        if let Some(y) = removed_y {
            self.stop_riding_from_removed_vehicle(id, y);
            return;
        }
        if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            self.dismount(i);
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            self.dismount_cart(i);
        } else if let Some(c) = self.cushions.iter().find(|c| c.id == id).cloned() {
            self.get_up_from_cushion(&c);
        } else if self.seat(id).is_some() {
            self.get_off_seat();
        } else {
            self.dismount_mount();
        }
    }

    pub(super) fn stop_riding_from_removed_vehicle(&mut self, vehicle: u32, y: f64) {
        if self.native_may_leave(vehicle) && self.riding == Some(vehicle) {
            self.dismount_removed_vehicle(y);
            self.release_removed_vehicle(vehicle);
        }
    }
}

/// `Consumable.emitParticlesAndSounds` against the local player.
struct Emitter<'a, 'g>(&'a mut Game<'g>);
impl EmissionWorld for Emitter<'_, '_> {
    type Error = Infallible;
    type Stack = ItemStack;
    type Sound = String;
    type Random = ();
    type Source = LegacyRandom;
    fn random_source(&mut self, (): ()) -> &mut LegacyRandom {
        &mut self.0.world.random
    }
    /// `LivingEntity.spawnItemParticles`.
    fn spawn_item_particles(&mut self, stack: ItemStack, count: i32) -> Result<(), Infallible> {
        self.0.eat_particles(&stack, count.max(0) as u32);
        Ok(())
    }
    fn overrides_consume_sound(&self) -> bool {
        false
    }
    fn consume_sound(&mut self, _: ItemStack) -> Result<String, Infallible> {
        unreachable!("players play the consumable's own sound")
    }
    /// `Player.playSound`: at the player's position, for everyone but the
    /// player, whose own client plays it (`level.playSound(player, ...)`):
    /// a guest's server turn leaves the guest out.
    fn play_sound(&mut self, sound: String, volume: f32, pitch: f32) -> Result<(), Infallible> {
        let at = self.0.feet();
        if !self.0.is_logical_server() {
            self.0.play_client_player_sound(&sound, volume, pitch);
        } else if self.0.lan_acting.is_some() {
            self.0.audio.play_unshared(&sound, at, volume, pitch);
            if let Some(at) = at {
                self.0.guest_player_sounds.push((sound, at, volume, pitch));
            }
        } else {
            self.0.audio.play(&sound, at, volume, pitch);
        }
        Ok(())
    }
}

/// The local player as `TeleportRandomlyConsumeEffect`'s user.
struct Teleporter<'a, 'g>(&'a mut Game<'g>, Option<&'a mod_api::ItemStack>);
impl Teleporter<'_, '_> {
    fn at(&self) -> Vec3 {
        self.0.player.as_ref().map_or(Vec3::ZERO, |p| p.position)
    }
}
impl TeleportConsumeWorld for Teleporter<'_, '_> {
    type Error = Infallible;
    type Level = Dim;
    type Random = ();
    type Source = LegacyRandom;
    fn position_field(&self) -> Vec3 {
        self.at()
    }
    fn get_random(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
    fn random_source(&mut self, (): ()) -> &mut LegacyRandom {
        &mut self.0.world.random
    }
    fn min_y(&mut self, level: Dim) -> Result<i32, Infallible> {
        Ok(self.0.level_shapes.get(level.id()).min_y)
    }
    fn logical_height(&mut self, level: Dim) -> Result<i32, Infallible> {
        Ok(self.0.level_shapes.get(level.id()).logical_height)
    }
    fn is_passenger(&mut self) -> Result<bool, Infallible> {
        Ok(self.0.riding.is_some())
    }
    fn stop_riding(&mut self) -> Result<(), Infallible> {
        self.0.stop_riding();
        Ok(())
    }
    fn position(&mut self) -> Result<Vec3, Infallible> {
        Ok(self.at())
    }
    fn teleport_allowed(&mut self, target: &mut Vec3) -> Result<bool, Infallible> {
        let Some(consumed) = self.1 else {
            return Ok(true);
        };
        let Some(player) = self.0.native_acting_player() else {
            return Ok(true);
        };
        let from = self.at();
        let dimension = self.0.dim;
        let mut to = [target.x, target.y, target.z];
        let mut cancel = false;
        self.0.native_item_consumption_teleport(
            mod_api::LivingRef::new(player.entity()),
            consumed,
            [from.x, from.y, from.z],
            &mut to,
            &mut cancel,
        );
        if self.0.dim != dimension
            || self
                .0
                .player
                .as_ref()
                .is_none_or(|p| p.position != from || p.health.dead_or_dying())
            || self.0.sleeping.is_some()
            || self.0.riding.is_some()
            || self.0.native_acting_player() != Some(player)
        {
            return Ok(false);
        }
        // Keep the mod's spot inside the level as vanilla's own draw is (a
        // huge y would walk down a long way); one that is not a number keeps
        // the draw.
        if to.iter().all(|v| v.is_finite()) {
            let shape = self.0.level_shapes.get(self.0.dim.id());
            let high = shape.min_y + shape.logical_height - 1;
            let y = to[1].clamp(f64::from(shape.min_y), f64::from(high));
            *target = Vec3::new(to[0], y, to[2]);
        }
        Ok(!cancel)
    }
    fn random_teleport(&mut self, target: Vec3) -> Result<bool, Infallible> {
        Ok(self
            .0
            .random_teleport(target, Some("minecraft:consumable_does_not_teleport_to")))
    }
    fn teleport_event(&mut self, _: Dim, old: Vec3) -> Result<(), Infallible> {
        let ctx = self.0.who_context(Some(Who::Player), None);
        self.0
            .post_game_event("minecraft:teleport", [old.x, old.y, old.z], ctx);
        Ok(())
    }
    fn is_fox(&self) -> bool {
        false
    }
    fn teleport_sound(&mut self, _: Dim, at: Vec3, _: bool) -> Result<(), Infallible> {
        self.0.audio.play(
            "minecraft:item.chorus_fruit.teleport",
            Some([at.x, at.y, at.z]),
            1.0,
            1.0,
        );
        Ok(())
    }
    fn block_position(&mut self) -> Result<BlockPos, Infallible> {
        Ok(BlockPos::containing(self.at()))
    }
    fn directional_particles(
        &mut self,
        _: Dim,
        origin: BlockPos,
        offset: i32,
    ) -> Result<(), Infallible> {
        self.0
            .level_event_at(2017, [origin.x, origin.y, origin.z], offset);
        Ok(())
    }
    fn reset_fall_distance(&mut self) -> Result<(), Infallible> {
        if let Some(p) = self.0.player.as_mut() {
            p.fall_distance = 0.0;
        }
        Ok(())
    }
    fn reset_impulse(&mut self) -> Result<(), Infallible> {
        if let Some(p) = self.0.player.as_mut() {
            p.impulse.reset();
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum MobConsumption<'a> {
    Fox(&'a sunrise_client::mobs::fox::Consumption),
    Drink(&'a ItemStack),
}
impl MobConsumption<'_> {
    fn still_valid(self, mob: &sunrise_client::mobs::Mob, from: Vec3) -> bool {
        match self {
            Self::Fox(decision) => decision.still_valid_at(mob, from),
            Self::Drink(stack) => {
                mob.alive()
                    && !mob.removed
                    && mob.position == from
                    && mob.extra.main_hand_stack() == Some(stack)
            }
        }
    }
}

/// A paused mob as the same `TeleportRandomlyConsumeEffect` user as a player.
struct MobTeleporter<'a, 'g> {
    game: &'a mut Game<'g>,
    consumer: mod_api::LivingRef,
    id: u32,
    usage: MobConsumption<'a>,
    dim: Dim,
    consumed: Option<&'a mod_api::ItemStack>,
}
impl MobTeleporter<'_, '_> {
    fn at(&self) -> Result<Vec3, ()> {
        self.game
            .mobs
            .iter()
            .find(|m| m.id == self.id && m.alive() && !m.removed)
            .map(|m| m.position)
            .ok_or(())
    }
}
impl TeleportConsumeWorld for MobTeleporter<'_, '_> {
    type Error = ();
    type Level = Dim;
    type Random = u32;
    type Source = LegacyRandom;
    fn position_field(&self) -> Vec3 {
        // get_random immediately refuses an absent actor before any draw or apply.
        self.at().unwrap_or_default()
    }
    fn get_random(&mut self) -> Result<u32, ()> {
        self.at()?;
        Ok(self.id)
    }
    fn random_source(&mut self, id: u32) -> &mut LegacyRandom {
        &mut self
            .game
            .mobs
            .iter_mut()
            .find(|m| m.id == id)
            .expect("get_random validated the actor without an intervening callback")
            .random
    }
    fn min_y(&mut self, _: Dim) -> Result<i32, ()> {
        Ok(self.game.world.shape.min_y)
    }
    fn logical_height(&mut self, _: Dim) -> Result<i32, ()> {
        Ok(self.game.world.shape.logical_height)
    }
    fn is_passenger(&mut self) -> Result<bool, ()> {
        self.game
            .mobs
            .iter()
            .find(|m| m.id == self.id)
            .map(|m| m.extra.passenger)
            .ok_or(())
    }
    fn stop_riding(&mut self) -> Result<(), ()> {
        self.game.dismount_mob(self.id);
        Ok(())
    }
    fn position(&mut self) -> Result<Vec3, ()> {
        self.at()
    }
    fn teleport_allowed(&mut self, target: &mut Vec3) -> Result<bool, ()> {
        let from = self.at()?;
        let mut to = [target.x, target.y, target.z];
        let mut cancel = false;
        if let Some(consumed) = self.consumed {
            self.game.native_item_consumption_teleport(
                self.consumer,
                consumed,
                [from.x, from.y, from.z],
                &mut to,
                &mut cancel,
            );
        }
        if self.game.dim != self.dim
            || !matches!(self.game.native_living_target(self.consumer),
                Some(super::commands::Target::Mob(id)) if id == self.id)
            || !self
                .game
                .mobs
                .iter()
                .any(|m| m.id == self.id && self.usage.still_valid(m, from))
        {
            return Err(());
        }
        if to.iter().all(|v| v.is_finite()) {
            let shape = self.game.world.shape;
            let high = shape.min_y + shape.logical_height - 1;
            *target = Vec3::new(
                to[0],
                to[1].clamp(f64::from(shape.min_y), f64::from(high)),
                to[2],
            );
        }
        Ok(!cancel)
    }
    fn random_teleport(&mut self, at: Vec3) -> Result<bool, ()> {
        Ok(self.game.living_random_teleport(
            self.consumer,
            [at.x, at.y, at.z],
            Some("minecraft:consumable_does_not_teleport_to"),
        ))
    }
    fn teleport_event(&mut self, _: Dim, old: Vec3) -> Result<(), ()> {
        let ctx = self.game.who_context(Some(Who::Mob(self.id)), None);
        self.game
            .post_game_event("minecraft:teleport", [old.x, old.y, old.z], ctx);
        Ok(())
    }
    fn is_fox(&self) -> bool {
        matches!(self.usage, MobConsumption::Fox(_))
    }
    fn teleport_sound(&mut self, _: Dim, at: Vec3, fox: bool) -> Result<(), ()> {
        self.game.audio.play(
            if fox {
                "minecraft:entity.fox.teleport"
            } else {
                "minecraft:item.chorus_fruit.teleport"
            },
            Some([at.x, at.y, at.z]),
            1.0,
            1.0,
        );
        Ok(())
    }
    fn block_position(&mut self) -> Result<BlockPos, ()> {
        Ok(BlockPos::containing(self.at()?))
    }
    fn directional_particles(&mut self, _: Dim, origin: BlockPos, offset: i32) -> Result<(), ()> {
        self.game
            .level_event_at(2017, [origin.x, origin.y, origin.z], offset);
        Ok(())
    }
    fn reset_fall_distance(&mut self) -> Result<(), ()> {
        let m = self
            .game
            .mobs
            .iter_mut()
            .find(|m| m.id == self.id)
            .ok_or(())?;
        m.fall_distance = 0.0;
        Ok(())
    }
    fn reset_impulse(&mut self) -> Result<(), ()> {
        // The existing mob model has no separate CurrentImpulseContext state.
        Ok(())
    }
}

#[cfg(test)]
mod teleport_hook_tests {
    use super::*;
    use crate::game::api_values::test_snapshot;
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{Flow, Hooks, ItemConsumptionTeleportEvent, ModEntry, ServerHost};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use sunrise_client::motion::PlayerMotion;

    struct ConsumptionProbe(Arc<Mutex<Vec<mod_api::ItemStack>>>, Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for ConsumptionProbe {
        fn item_consumption_teleport(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ItemConsumptionTeleportEvent<'_>,
        ) -> Flow {
            assert_eq!(
                event.consumer.entity(),
                host.players().acting_player().unwrap().entity()
            );
            self.0.lock().unwrap().push(event.consumed.clone());
            assert_eq!(event.consumed.count(), 7);
            assert_eq!(event.consumed.damage(), 3);
            assert!(event.consumed.components().is_some());
            if self.1.load(Ordering::SeqCst) {
                assert!(
                    event
                        .consumer
                        .entity()
                        .teleport_to(host, [3.0, 70.0, 3.0], None)
                );
                return Flow::Continue;
            }
            Flow::Cancel
        }
    }

    #[test]
    fn unregistered_consumed_stack_skips_the_snapshot() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "consumed-stack-proof",
            Box::new(ConsumptionProbe(
                Arc::clone(&heard),
                Arc::new(AtomicBool::new(false)),
            )),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            assert!(mod_api::__private::listened(
                mod_api::HookId::ItemConsumptionTeleport
            ));
            let stack = ItemStack::new("missing:saved_fruit", 1);
            assert!(consumed_snapshot(&stack).is_none());
            assert_eq!(stack.item.as_ref(), "missing:saved_fruit");
            assert_eq!(stack.count, 1);
            assert!(heard.lock().unwrap().is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn consumption_hook_keeps_the_stack_before_use_and_cancellation_stops_attempts() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let moved = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "consumed-stack-proof",
            Box::new(ConsumptionProbe(Arc::clone(&heard), Arc::clone(&moved))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let at = Vec3::new(0.0, 65.0, 0.0);
                game.player = Some(PlayerMotion::new(game.motion, at).unwrap());
                // This body belongs to the guest record installed below, never
                // to a separately admitted local player with the same runtime ID.
                game.player.as_mut().unwrap().entity_id = 2;
                game.lan_acting = Some(12);
                game.lan_acting_uuid = Some(7);
                game.lan_acting_name = Some("ConsumedStackTest".into());
                game.lan_acting_entity = Some(2);
                game.lan_acting_permission = Some(4);
                crate::game::native_entity_refs::tests::bind_acting_guest(game);
                game.creative = false;
                let mut consumed = ItemStack::new("minecraft:chorus_fruit", 7);
                consumed.damage = 3;
                consumed.data_mut().set_loot_name(
                    "minecraft:custom_name",
                    &serde_json::json!("Captured fruit"),
                );
                let snapshot = test_snapshot(&consumed);
                game.inventory[game.selected] = Some(consumed.clone());
                let info = game.items().get(&consumed.item).unwrap().clone();
                let consumable = consumed.consumable(Some(&info)).unwrap().clone();
                game.finish_consuming(&info, &consumable, &consumed);
                assert_eq!(
                    heard.lock().unwrap().as_slice(),
                    std::slice::from_ref(&snapshot)
                );
                assert_eq!(
                    game.held().unwrap().count,
                    6,
                    "veto stops teleport, not consumption"
                );
                assert_eq!(game.player.as_ref().unwrap().position, at);

                // The source stack is explicit, even if the hand has since changed.
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 1));
                let mut expected_random = game.world.random.clone();
                for _ in 0..3 {
                    expected_random.next_double();
                }
                assert!(!game.teleport_randomly(
                    &TeleportConsume {
                        diameter: 16.0,
                        directional_particles: true,
                    },
                    &consumed
                ));
                assert_eq!(*heard.lock().unwrap(), [snapshot.clone(), snapshot]);
                assert_eq!(
                    game.world.random.next_long(),
                    expected_random.next_long(),
                    "veto ends the effect after the first candidate's three draws"
                );
                assert_eq!(game.player.as_ref().unwrap().position, at);

                moved.store(true, Ordering::SeqCst);
                expected_random = game.world.random.clone();
                for _ in 0..3 {
                    expected_random.next_double();
                }
                assert!(!game.teleport_randomly(
                    &TeleportConsume {
                        diameter: 16.0,
                        directional_particles: true,
                    },
                    &consumed
                ));
                assert_eq!(
                    game.player.as_ref().unwrap().position,
                    Vec3::new(3.0, 70.0, 3.0),
                    "a listener's direct move invalidates the consumption target"
                );
                assert_eq!(game.world.random.next_long(), expected_random.next_long());
            })
        });
    }
}

#[cfg(test)]
mod fox_consumption_hook_tests {
    use super::*;
    use crate::game::api_values::test_snapshot;
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{Flow, Hand, Hooks, ItemConsumptionTeleportEvent, ModEntry, ServerHost};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    };
    use sunrise_client::mobs::{Mob, fox};

    struct Probe {
        mode: Arc<AtomicU8>,
        heard: Arc<Mutex<Vec<(u32, mod_api::ItemStack)>>>,
        spawned: Arc<Mutex<Vec<u32>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn item_consumption_teleport(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ItemConsumptionTeleportEvent<'_>,
        ) -> Flow {
            self.heard
                .lock()
                .unwrap()
                .push((event.consumer.entity().id(), event.consumed.clone()));
            assert_eq!(event.consumer.entity().position(host), Some(event.from));
            match self.mode.load(Ordering::SeqCst) {
                0 => Flow::Cancel,
                1 => {
                    *event.target = [4.5, 70.0, 4.5];
                    Flow::Continue
                }
                2 => {
                    assert!(
                        event
                            .consumer
                            .set_hand_item(
                                host,
                                Hand::Main,
                                Some(test_snapshot(&ItemStack::new("minecraft:stone", 1)))
                            )
                            .unwrap()
                    );
                    Flow::Continue
                }
                3 => {
                    assert!(
                        event
                            .consumer
                            .entity()
                            .teleport_to(host, [3.5, 64.0, 3.5], None)
                    );
                    Flow::Continue
                }
                4 => {
                    assert!(event.consumer.entity().discard(host));
                    Flow::Continue
                }
                5 => {
                    match event.consumer.entity().id() {
                        200 => {
                            assert_eq!(
                                event.from,
                                [8.5, 64.0, 8.5],
                                "the hook precedes this fox's movement"
                            );
                            // Full owning-host access, before this fox moves and
                            // before any later mob's AI/consumption step.
                            let next = host.entity(201).unwrap();
                            assert_eq!(next.position(host), Some([11.5, 64.0, 8.5]));
                            let next_living = host.living(next).unwrap();
                            assert_eq!(
                                next_living.with_hand_item(host, Hand::Main, |stack| stack.count()),
                                Some(7)
                            );
                            assert!(
                                next_living
                                    .set_hand_item(
                                        host,
                                        Hand::Main,
                                        Some(test_snapshot(&ItemStack::new("minecraft:stone", 1)))
                                    )
                                    .unwrap()
                            );
                            assert!(next.teleport_to(host, [12.5, 64.0, 8.5], None));
                            assert!(host.entity(202).unwrap().discard(host));
                            let spawned = host
                                .summon(
                                    "minecraft:overworld",
                                    "minecraft:cow",
                                    [2.5, 64.0, 2.5],
                                    Default::default(),
                                )
                                .unwrap();
                            self.spawned.lock().unwrap().push(spawned);
                            *event.target = [4.5, 70.0, 4.5];
                            Flow::Continue
                        }
                        204 => {
                            // The earlier fox resumed before this one, and the
                            // listener's unrelated actor writes survived it.
                            let first = host.living(host.entity(200).unwrap()).unwrap();
                            assert_eq!(
                                first.with_hand_item(host, Hand::Main, |stack| stack.count()),
                                Some(6)
                            );
                            let next = host.living(host.entity(201).unwrap()).unwrap();
                            assert_eq!(
                                next.with_hand_item(host, Hand::Main, |stack| stack.project())
                                    .flatten(),
                                Some(test_snapshot(&ItemStack::new("minecraft:stone", 1)))
                            );
                            assert!(host.entity(202).is_none());
                            Flow::Cancel
                        }
                        other => panic!("unexpected consumption by {other}"),
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn fox_consumption_adapter_uses_full_host_and_preserves_listener_writes() {
        let mode = Arc::new(AtomicU8::new(0));
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "fox-consumption-proof",
            Box::new(Probe {
                mode: Arc::clone(&mode),
                heard: Arc::clone(&heard),
                spawned: Arc::new(Mutex::new(Vec::new())),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 63, z, stone).unwrap();
                    }
                }
                let ty = game.res.mob_types().get("minecraft:fox").unwrap().clone();
                for action in 0..5 {
                    mode.store(action, Ordering::SeqCst);
                    let id = 60 + u32::from(action);
                    let from = Vec3::new(8.5, 64.0, 8.5);
                    let mut actor = Mob::new(id, ty.clone(), from, 0.0);
                    actor.ground.on_ground = true;
                    actor.fall_distance = 9.0;
                    actor.extra.ints[1] = 600;
                    let mut stack = ItemStack::new("minecraft:chorus_fruit", 7);
                    stack
                        .data_mut()
                        .set_loot_name("minecraft:custom_name", &serde_json::json!("Fox fruit"));
                    actor.extra.set_main_hand_stack(Some(stack.clone()));
                    let mut events = Vec::new();
                    let decision = fox::ai_step_until_consumption(
                        &mut actor,
                        &game.mob_world_base(&[]),
                        &mut events,
                    )
                    .expect("pause before effect draws and mouth replacement");
                    assert!(events.is_empty());
                    let mut expected_random = actor.random.clone();
                    // Independent Java Random state-advance budget before the
                    // teleport: emitParticlesAndSounds spends 1 boolean plus
                    // 3 floats for sound, 16 crumbs spend 4 floats each, and
                    // FoodProperties' sound triangle spends 2 floats: 70
                    // advances total. Discarded nextFloat advances the same
                    // 48-bit LCG state once, without copying emission logic.
                    for _ in 0..70 {
                        expected_random.next_float();
                    }
                    // The first teleport attempt consumes three doubles
                    // before any listener can cancel or replace the actor.
                    for _ in 0..3 {
                        expected_random.next_double();
                    }
                    game.mobs.push(actor);
                    let applied = game.apply_fox_consumption(&decision);
                    assert_eq!(applied, action <= 1);
                    assert_eq!(
                        heard.lock().unwrap().last(),
                        Some(&(id, test_snapshot(&stack)))
                    );
                    let actor = game.mobs.iter_mut().find(|m| m.id == id && !m.removed);
                    match action {
                        0 => {
                            let m = actor.unwrap();
                            assert_eq!(m.position, from);
                            assert_eq!(m.extra.main_hand_stack().unwrap().count, 6);
                            assert_eq!(m.extra.ints[1], 0);
                            assert_eq!(m.random.next_long(), expected_random.next_long());
                        }
                        1 => {
                            let m = actor.unwrap();
                            assert_eq!(m.position, Vec3::new(4.5, 64.0, 4.5));
                            assert_eq!(m.extra.main_hand_stack().unwrap().count, 6);
                            assert_eq!(m.fall_distance, 0.0);
                            assert_eq!(m.random.next_long(), expected_random.next_long());
                        }
                        2 => {
                            let m = actor.unwrap();
                            assert_eq!(m.position, from);
                            assert_eq!(
                                &*m.extra.main_hand_stack().unwrap().item,
                                "minecraft:stone"
                            );
                            assert_eq!(m.random.next_long(), expected_random.next_long());
                        }
                        3 => {
                            let m = actor.unwrap();
                            assert_eq!(m.position, Vec3::new(3.5, 64.0, 3.5));
                            assert_eq!(m.extra.main_hand_stack(), Some(&stack));
                            assert_eq!(m.random.next_long(), expected_random.next_long());
                        }
                        4 => assert!(actor.is_none()),
                        _ => unreachable!(),
                    }
                }
                assert_eq!(
                    heard.lock().unwrap().len(),
                    5,
                    "one callback per first attempt"
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn fox_tick_pauses_before_movement_and_next_actor_and_preserves_full_host_writes() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let spawned = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "fox-tick-consumption-proof",
            Box::new(Probe {
                mode: Arc::new(AtomicU8::new(5)),
                heard: Arc::clone(&heard),
                spawned: Arc::clone(&spawned),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                // Legacy summon checks the actual server owner as well as
                // the opened native lifecycle, like the production server does.
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 63, z, stone).unwrap();
                    }
                }
                game.next_entity = 300;
                let ty = game.res.mob_types().get("minecraft:fox").unwrap().clone();
                for (id, x) in [(200, 8.5), (201, 11.5), (202, 13.5), (204, 6.5)] {
                    let mut actor = Mob::new(id, ty.clone(), Vec3::new(x, 64.0, 8.5), 0.0);
                    actor.persistent = true;
                    actor.ground.on_ground = true;
                    actor.extra.ints[1] = 600;
                    actor
                        .extra
                        .set_main_hand_stack(Some(ItemStack::new("minecraft:chorus_fruit", 7)));
                    if id == 200 {
                        actor.velocity = Vec3::new(0.2, 0.0, 0.0);
                    }
                    game.mobs.push(actor);
                }
                game.tick_creature_entities().unwrap();
                let heard = heard.lock().unwrap();
                assert_eq!(
                    heard.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                    [200, 204]
                );
                assert!(heard.iter().all(|(_, stack)| stack.count() == 7));
                let first = game.mobs.iter().find(|m| m.id == 200).unwrap();
                assert_eq!(first.age, 1, "base tick must not repeat after the callback");
                assert_eq!(
                    first.previous,
                    Vec3::new(4.5, 64.0, 4.5),
                    "the normal teleport resets interpolation before resumed movement"
                );
                assert_eq!(first.extra.main_hand_stack().unwrap().count, 6);
                assert!(
                    first.position.x < 6.0,
                    "movement resumes at the redirected landing"
                );
                let next = game.mobs.iter().find(|m| m.id == 201).unwrap();
                assert_eq!(next.age, 1);
                assert_eq!(
                    &*next.extra.main_hand_stack().unwrap().item,
                    "minecraft:stone"
                );
                assert_eq!(next.previous, Vec3::new(12.5, 64.0, 8.5));
                assert!(!game.mobs.iter().any(|m| m.id == 202));
                let last = game.mobs.iter().find(|m| m.id == 204).unwrap();
                assert_eq!(last.age, 1);
                assert_eq!(last.extra.main_hand_stack().unwrap().count, 6);
                let spawned = spawned.lock().unwrap();
                assert_eq!(spawned.len(), 1);
                let cow = game.mobs.iter().find(|m| m.id == spawned[0]).unwrap();
                assert_eq!(
                    cow.age, 0,
                    "a listener's new entity waits until the next batch"
                );
            })
        });
    }
}
