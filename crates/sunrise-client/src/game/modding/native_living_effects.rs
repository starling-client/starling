//! A living entity's active effects for native server mods, through a
//! `LivingRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles"): vanilla's
//! `activeEffects` visited in place, and `addEffect`, `removeEffect`,
//! `removeAllEffects` and `hasEffect` by the game's own paths, so an effect
//! a mod gives lands as a potion's or `/effect`'s does, through
//! `canBeAffected` and the added-effect hooks. Reads return `None`, writes
//! false, for a stale handle or on a client; armor stands and the dragon
//! keep no effects, as `/effect` skips them. `effect` and `visit_effects` also read
//! on a client, for the local player, from the effects the server sent it, and
//! `effect_blend_factor` reads only there, from the fades the client ticks.
use super::Game;
use super::commands::Target;
use mod_api::{ActiveEffect, EffectKey, EntityReadError, LivingRef};
use std::ops::ControlFlow;
use sunrise_client::effects::{EffectInstance, PlayerEffects};
use sunrise_client::mobs::Mob;
use sunrise_items::items::ItemStack;

/// A player's effect as the API's load-owned key and instance data.
pub(super) fn active_effect(e: &EffectInstance) -> ActiveEffect {
    ActiveEffect::from(mod_api::__private::built::ActiveEffect {
        id: mod_api::__private::effect_key_by_index(
            e.registry_index.expect("active effect registry index"),
        )
        .expect("effects resolved at load"),
        duration: e.duration,
        amplifier: e.amplifier,
        ambient: e.ambient,
        visible: e.visible,
        show_icon: e.show_icon,
    })
}

/// A mob's stored effect as the API's: ambient and particle flags remain in their own lists.
pub(super) fn mob_effect(m: &Mob, e: &sunrise_client::mobs::StoredEffect) -> ActiveEffect {
    ActiveEffect::from(mod_api::__private::built::ActiveEffect {
        id: mod_api::__private::effect_key_by_index(
            e.registry_index.expect("active effect registry index"),
        )
        .expect("effects resolved at load"),
        duration: e.duration,
        amplifier: e.amplifier,
        ambient: m.extra.ambient_effects.contains(&e.id),
        visible: !m.extra.hidden_effects.contains(&e.id),
        show_icon: e.show_icon,
    })
}

/// Project mob effects in store order, stopping before projecting the next on a break.
pub(super) fn visit_mob_effects(
    mob: &Mob,
    visit: &mut dyn FnMut(ActiveEffect) -> ControlFlow<()>,
) -> ControlFlow<()> {
    mob.extra
        .effects
        .iter()
        .map(|e| mob_effect(mob, e))
        .try_for_each(visit)
}

impl Game<'_> {
    /// Apply one loaded consumable effect without consuming the item.
    pub(super) fn living_apply_consume_effect(
        &mut self,
        living: LivingRef,
        stack: &ItemStack,
        index: usize,
    ) -> Option<bool> {
        let target = self.native_living_target(living)?;
        let consumable = self.consumable(stack)?;
        let effect = consumable.effects.get(index)?;
        match target {
            t @ (Target::Player | Target::Guest(_)) => self
                .for_player(t, |g| {
                    g.apply_consume_effect(stack, effect, None, living.entity())
                })
                .flatten(),
            Target::Mob(_) => {
                self.apply_consume_effect(stack, effect, Some(living), living.entity())
            }
            _ => None,
        }
    }

    /// Where a player's or a guest's effects are: the acting player's own
    /// are the game's (swapped in by the entry of `guest_turn`; a player handle names
    /// the acting player, and a game without a hub keeps its single
    /// player's the same way), any other guest's are their hub record's.
    /// None for another kind.
    pub(super) fn native_living_player_effects(&self, t: Target) -> Option<&PlayerEffects> {
        match t {
            Target::Player => Some(&self.potions.effects),
            Target::Guest(conn) if self.lan_acting == Some(conn) => Some(&self.potions.effects),
            Target::Guest(conn) => Some(&self.lan.as_ref()?.hub.guest(conn)?.effects),
            _ => None,
        }
    }

    /// `LivingEntity.addEffect(instance)`: gives the living entity the
    /// effect by the game's own path. A player's or a guest's goes through
    /// `add_player_effect` and a mob's goes through `add_mob_effect` (so
    /// `canBeAffected`, `MobEffectEvent.Added`, `MobEffectInstance.update`,
    /// attribute modifiers, absorption and `MobEffectPart::on_effect_added`
    /// all run in vanilla/NeoForge order).
    /// The amplifier is cut to 0..=255 and the duration is -1 (infinite) or
    /// ticks. False before effect resolution, for a duration below -1, an armor
    /// stand, the dragon, a wither, a stale handle or a client, or when the
    /// active instance did not change, as in vanilla's `addEffect`.
    pub(super) fn living_add_effect(&mut self, living: LivingRef, effect: &ActiveEffect) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect.id)
            .and_then(sunrise_client_core::potions::effect_by_index)
        else {
            return false;
        };
        if effect.duration < -1 {
            return false;
        }
        let amplifier = effect.amplifier.clamp(0, 255);
        let mut instance = EffectInstance::from_def(def, effect.duration, amplifier);
        instance.ambient = effect.ambient;
        instance.visible = effect.visible;
        instance.show_icon = effect.show_icon;
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| g.add_player_effect(instance))
                .unwrap_or(false),
            Some(Target::Mob(id)) => self.add_mob_effect(id, instance, None),
            _ => false,
        }
    }

    /// The entity's instance of `effect` (`LivingEntity.getEffect`): its id
    /// namespaced as the registry has it, the ticks left (-1 infinite), the
    /// amplifier and the flags. The effect is a registration-slot key resolved at load.
    /// None when it has the effect not, before effect resolution, or for an armor
    /// stand, the dragon, a stale handle or a client.
    pub(super) fn living_effect(
        &self,
        living: LivingRef,
        effect: EffectKey,
    ) -> Option<ActiveEffect> {
        let def = mod_api::__private::effect_index(effect)
            .and_then(sunrise_client_core::potions::effect_by_index)?;
        let target = self.native_living_read_target(living)?;
        super::native_living_body::body_effect(self.native_living_body(target)?, def.index)
    }

    /// Visit `LivingEntity.getActiveEffects` over the resident player, guest or mob store.
    pub(super) fn living_visit_effects(
        &self,
        living: LivingRef,
        visit: &mut dyn FnMut(ActiveEffect) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let result = match self.native_living_visit_target(living)? {
            t @ (Target::Player | Target::Guest(_)) => self
                .native_living_player_effects(t)
                .ok_or(EntityReadError::Unavailable)?
                .list
                .iter()
                .map(active_effect)
                .try_for_each(visit),
            Target::Mob(id) => {
                let mob = self
                    .mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)
                    .ok_or(EntityReadError::Stale)?;
                visit_mob_effects(mob, visit)
            }
            _ => return Err(EntityReadError::Unavailable),
        };
        Ok(result)
    }

    /// Whether the entity has `effect` (`LivingEntity.hasEffect`), using its resolved key.
    /// False before effect resolution, for an armor stand, the dragon or a stale handle;
    /// on a client, for any entity but the local player, whose copy of what the server
    /// sent it is read.
    pub(super) fn living_potion_affected(&self, living: LivingRef, effect: EffectKey) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect)
            .and_then(sunrise_client_core::potions::effect_by_index)
        else {
            return false;
        };
        self.native_living_read_target(living)
            .and_then(|target| self.native_living_body(target))
            .is_some_and(|body| super::native_living_body::body_potion_affected(body, def.index))
    }

    /// `LivingEntity.getEffectBlendFactor(effect, partialTick)` on a client:
    /// how far the effect has faded in, from 0 to 1, between the last tick
    /// and the next at `partial`. Darkness fades in and out over 22 ticks and
    /// nausea in over 150 and out over 20 (`MobEffect.setBlendDuration`), the
    /// fades the fog, lightmap and screen spin draw. Other effects start at
    /// 0 after a blended add and interpolate to their post-countdown target
    /// during each client tick: 1 while active, 0 once expired. An unblended
    /// add starts at its target immediately; an update preserves the old blend.
    /// `partial` is not clamped. The registration-slot key is resolved at load.
    /// 0 when the entity lacks the effect. None before effect resolution, for a stale
    /// handle, or an entity other than the local player: the client keeps other living
    /// entities' effects, but this query reads only the local player's.
    pub(super) fn living_effect_blend_factor(
        &self,
        living: LivingRef,
        effect: EffectKey,
        partial: f32,
    ) -> Option<f32> {
        let def = mod_api::__private::effect_index(effect)
            .and_then(sunrise_client_core::potions::effect_by_index)?;
        let Target::Player = self.native_living_read_target(living)? else {
            return None;
        };
        let Some(e) = self.potions.effects.get_registered(def.index) else {
            return Some(0.0);
        };
        Some(match def.id {
            "darkness" => self.darkness_blend(partial),
            "nausea" => self.nausea_blend(partial),
            _ => {
                let (previous, current) = e.client_blend;
                let previous = u8::from(previous) as f32;
                let current = u8::from(current) as f32;
                previous + partial * (current - previous)
            }
        })
    }

    /// `LivingEntity.removeEffect(effect)` the way its running out goes
    /// (`LivingEntity.tickEffects` drops it): what the effect gave the
    /// player goes with it — the attribute modifiers recompute
    /// (`apply_effect_status`) and absorption's hearts leave — and a mob's
    /// hidden and ambient lists drop it too, with absorption's hearts
    /// leaving the same way its expiry takes them. False (and nothing
    /// changes) when the entity has the effect not, before effect resolution, or for
    /// an armor stand, the dragon, a stale handle or a client.
    pub(super) fn remove_mob_effect_by_index(&mut self, id: u32, effect_index: usize) -> bool {
        let Some(def) = sunrise_client_core::potions::effect_by_index(effect_index) else {
            return false;
        };
        let Some(amplifier) = self
            .mobs
            .iter()
            .find(|m| m.id == id && !m.removed)
            .and_then(|m| {
                m.extra
                    .effects
                    .iter()
                    .find(|e| e.registry_index == Some(def.index))
                    .map(|e| e.amplifier)
            })
        else {
            return false;
        };
        // `MobEffectEvent.Remove`: refusing the removal keeps it.
        if let Some(living) = self.effect_living_ref(Some(id as i32))
            && let Some(key) = mod_api::__private::effect_key_by_index(def.index)
        {
            let mut cancel = false;
            self.native_mob_effect_remove(living, key, amplifier, &mut cancel);
            if cancel {
                return false;
            }
        }
        self.mutate_mob_effects(id, Some(def.id), |mob| {
            mob.retain_effects(|e| e.registry_index != Some(def.index))
        })
        .unwrap_or(false)
    }

    pub(super) fn living_remove_effect(&mut self, living: LivingRef, effect: EffectKey) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect)
            .and_then(sunrise_client_core::potions::effect_by_index)
        else {
            return false;
        };
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| g.remove_player_effects(Some(def.index)))
                .unwrap_or(false),
            Some(Target::Mob(id)) => self.remove_mob_effect_by_index(id, def.index),
            _ => false,
        }
    }

    /// `LivingEntity.removeAllEffects()` the way their running out goes:
    /// every effect's attribute modifiers and absorption hearts leave with
    /// them. False when the entity has none, or for an armor stand, the
    /// dragon, a stale handle or a client.
    pub(super) fn living_remove_all_effects(&mut self, living: LivingRef) -> bool {
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| g.remove_player_effects(None))
                .unwrap_or(false),
            Some(Target::Mob(id)) => {
                let held: Vec<(usize, i32)> = self
                    .mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)
                    .map(|m| {
                        m.extra
                            .effects
                            .iter()
                            .filter_map(|e| e.registry_index.map(|index| (index, e.amplifier)))
                            .collect()
                    })
                    .unwrap_or_default();
                // `MobEffectEvent.Remove` per effect: a refused removal
                // keeps that effect.
                let keep: Vec<usize> = held
                    .iter()
                    .filter(|(name, amplifier)| {
                        let mut cancel = false;
                        if let Some(key) = mod_api::__private::effect_key_by_index(*name) {
                            self.native_mob_effect_remove(living, key, *amplifier, &mut cancel);
                        }
                        cancel
                    })
                    .map(|(name, _)| *name)
                    .collect();
                // The refused removals keep their effects; the rest go.
                self.mutate_mob_effects(id, None, |mob| {
                    mob.retain_effects(|e| {
                        e.registry_index.is_some_and(|index| keep.contains(&index))
                    })
                })
                .unwrap_or(false)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{fixture_resident_column, with_game};
    use mod_api::{ActiveEffect, EntityReadError, EntityRef, Host, LivingRef, WorldEpoch};
    use std::ops::ControlFlow;
    use sunrise_client::mobs::Mob;
    use sunrise_core::geometry::Vec3;

    fn collect_effects(
        host: &dyn Host,
        living: LivingRef,
    ) -> Result<Vec<ActiveEffect>, EntityReadError> {
        let mut effects = Vec::new();
        assert_eq!(
            living.visit_effects(host, &mut |effect| {
                effects.push(effect);
                ControlFlow::Continue(())
            })?,
            ControlFlow::Continue(())
        );
        Ok(effects)
    }

    fn effect(id: &'static str, duration: i32, amplifier: i32) -> ActiveEffect {
        ActiveEffect::from(mod_api::__private::built::ActiveEffect {
            id: mod_platform::__private::part_tests::effect_key(id),
            duration,
            amplifier,
            ambient: false,
            visible: true,
            show_icon: true,
        })
    }

    #[test]
    #[ignore = "daily"]
    fn active_effect_ids_are_load_owned_slots() {
        use sunrise_client::entities::ArmorStand;

        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(21).unwrap()).unwrap();
            // The same load-owned slot is used for application, enumeration and diagnostics.
            let speed = ActiveEffect::new(
                mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                200,
                0,
                false,
                true,
                true,
            );
            let slowness = ActiveEffect::new(
                mod_platform::__private::part_tests::effect_key("minecraft:slowness"),
                90,
                2,
                true,
                false,
                true,
            );
            assert!(game.living_add_effect(zombie, &speed));
            assert!(game.living_add_effect(zombie, &slowness));
            let listed = collect_effects(game, zombie).unwrap();
            assert_eq!(listed, [speed.clone(), slowness]);
            assert_eq!(
                listed[0].id,
                mod_platform::__private::part_tests::effect_key("speed")
            );
            assert_eq!(listed[0].id.id(game), Some("minecraft:speed"));

            let mut first = Vec::new();
            assert_eq!(
                zombie.visit_effects(game, &mut |effect| {
                    first.push(effect);
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [speed]);

            game.armor_stands.push(ArmorStand::new(22, Vec3::ZERO, 0.0));
            let stand = game.living(game.entity(22).unwrap()).unwrap();
            assert_eq!(
                stand.visit_effects(game, &mut |_| panic!("unavailable effects visited")),
                Err(EntityReadError::Unavailable)
            );
        });
    }

    /// `MobEffectInstance.showIcon` crosses `add_effect` and the reads both ways, for a player and
    /// a mob, apart from the particle flag; adding it again with the other value changes it.
    #[test]
    #[ignore = "daily"]
    fn an_effects_icon_flag_is_written_and_read_for_players_and_mobs() {
        use sunrise_client::motion::PlayerMotion;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(23, kind, Vec3::ZERO, 0.0));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.player.as_mut().unwrap().entity_id = 20;
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let key = mod_platform::__private::part_tests::effect_key("minecraft:speed");
            for id in [23, 20] {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                for show_icon in [false, true] {
                    let given = ActiveEffect::new(key, 200, 0, false, true, show_icon);
                    assert!(game.living_add_effect(living, &given), "{id} {show_icon}");
                    let read = game.living_effect(living, key).unwrap();
                    assert_eq!((read.show_icon, read.visible), (show_icon, true), "{id}");
                    assert_eq!(collect_effects(game, living).unwrap(), vec![read]);
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resolved_keys_keep_the_mobs_hidden_effect_identity() {
        with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(22, kind, Vec3::ZERO, 0.0));
            let cow = game.living(game.entity(22).unwrap()).unwrap();
            let key = mod_platform::__private::part_tests::effect_key("speed");
            assert!(game.living_add_effect(cow, &effect("speed", 2, 1)));
            assert!(!game.living_add_effect(cow, &effect("speed", 5, 0)));
            let mob = &game.mobs[0];
            assert_eq!(mob.extra.effect_chains[0].id, "minecraft:speed");
            assert_eq!(mob.extra.effect_chains[0].registry_index, Some(0));
            assert_eq!(game.living_effect(cow, key).unwrap().id, key);
            assert_eq!(game.living_effect(cow, key).unwrap().amplifier, 1);
        });
    }

    /// A mob marked removed but still listed counts as absent, as the effect reads treat it: the
    /// three writes return false and change nothing.
    #[test]
    #[ignore = "daily"]
    fn effect_writes_on_a_removed_mob_change_nothing() {
        with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(24, kind, Vec3::ZERO, 0.0));
            let cow = game.living(game.entity(24).unwrap()).unwrap();
            let speed = mod_platform::__private::part_tests::effect_key("speed");
            assert!(game.living_add_effect(cow, &effect("speed", 200, 0)));
            game.mobs[0].removed = true;
            assert!(!game.living_add_effect(cow, &effect("slowness", 200, 0)));
            assert!(!game.living_remove_effect(cow, speed));
            assert!(!game.living_remove_all_effects(cow));
            let held: Vec<_> = game.mobs[0].extra.effects.iter().map(|e| e.id).collect();
            assert_eq!(held, ["minecraft:speed"]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn indexed_consumable_effects_apply_to_a_mob() {
        use sunrise_client::items::ItemStack;
        use sunrise_core::random::RandomSource;
        with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let cow = game.living(game.entity(12).unwrap()).unwrap();
            let item = |id| ItemStack::new(id, 1);
            let mut expected = game.mobs[0].random.clone();
            let mut level_random = game.world.random.clone();
            expected.next_float();
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:pufferfish"), 0),
                Some(true)
            );
            assert_eq!(game.mobs[0].random.next_float(), expected.next_float());
            assert_eq!(game.world.random.next_float(), level_random.next_float());
            assert!(game.living_potion_affected(
                cow,
                mod_platform::__private::part_tests::effect_key("minecraft:poison")
            ));
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:honey_bottle"), 0),
                Some(true)
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:honey_bottle"), 0),
                Some(false)
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:milk_bucket"), 0),
                Some(true)
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:milk_bucket"), 0),
                Some(false)
            );
            // This fixture has no server sound hub; a sound cannot be queued.
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:ominous_bottle"), 0),
                None
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:chorus_fruit"), 0),
                None
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:pufferfish"), 1),
                None
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("minecraft:stone"), 0),
                None
            );
            assert_eq!(
                game.living_apply_consume_effect(cow, &item("missing:unknown"), 0),
                None
            );
            let stale = LivingRef::new(EntityRef::new(
                WorldEpoch::new(cow.entity().epoch().get() + 1),
                12,
            ));
            assert_eq!(
                game.living_apply_consume_effect(stale, &item("minecraft:pufferfish"), 0),
                None
            );
        });
    }

    /// `LivingEntity.getEffectBlendFactor` on a client: the local player's
    /// darkness fades in a 22nd a tick and nausea a 150th
    /// (`MobEffect.setBlendDuration` in `MobEffects`), an ordinary effect
    /// interpolates during its first tick, a missing one reads 0, and no other
    /// entity reads.
    #[test]
    #[ignore = "daily"]
    fn a_client_reads_the_local_players_effect_blend_factors() {
        use super::super::execution_role::ExecutionRole;
        use sunrise_client::{effects::EffectInstance, motion::PlayerMotion};
        use sunrise_protocol::connection::Pipe;
        let close =
            |read: Option<f32>, expected: f32| read.is_some_and(|f| (f - expected).abs() < 1e-6);
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "BlendReadTest").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let me = game.living(game.entity(7).unwrap()).unwrap();
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:darkness"),
                    0.5
                ),
                Some(0.0)
            );

            // A blended darkness starts from nothing; two client ticks later
            // it is between 1/22 and 2/22.
            game.potions
                .effects
                .list
                .push(EffectInstance::new("darkness", 200, 0));
            game.sculk.darkness = (0.0, 0.0);
            // The same effect reads whole on the client: `getEffect`, `getActiveEffects`, `hasEffect`.
            let darkness = mod_platform::__private::part_tests::effect_key("minecraft:darkness");
            assert!(me.potion_affected(game, darkness));
            assert_eq!(me.effect(game, darkness).map(|e| e.duration), Some(200));
            assert_eq!(collect_effects(game, me).map(|list| list.len()), Ok(1));
            assert!(!me.potion_affected(
                game,
                mod_platform::__private::part_tests::effect_key("minecraft:nausea")
            ));
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("darkness"),
                    1.0
                ),
                Some(0.0)
            );
            game.tick_client_effects();
            game.tick_client_effects();
            assert!(close(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:darkness"),
                    0.0
                ),
                1.0 / 22.0
            ));
            assert!(close(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:darkness"),
                    0.5
                ),
                1.5 / 22.0
            ));

            // Nausea fades in over 150 ticks, as the screen spin ticks it.
            game.potions
                .effects
                .list
                .push(EffectInstance::new("nausea", 600, 0));
            game.screen_fx.tick_spinning(Some(true));
            assert!(close(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:nausea"),
                    0.5
                ),
                0.5 / 150.0
            ));

            // Speed starts at 0 and interpolates to 1 over its first tick.
            game.potions
                .effects
                .list
                .push(EffectInstance::new("speed", 100, 0));
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                    0.0
                ),
                Some(0.0)
            );
            game.tick_client_effects();
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                    0.5
                ),
                Some(0.5)
            );

            // Gone, darkness reads 0 though its fade had not reached 0.
            game.potions.effects.list.retain(|e| e.id != "darkness");
            assert_eq!(
                me.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:darkness"),
                    0.5
                ),
                Some(0.0)
            );

            // Another entity and a stale handle read nothing.

            let epoch = me.entity().epoch();
            let other = LivingRef::new(EntityRef::new(epoch, 8));
            assert_eq!(
                other.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                    0.5
                ),
                None
            );
            let stale = LivingRef::new(EntityRef::new(WorldEpoch::new(epoch.get() + 1), 7));
            assert_eq!(
                stale.effect_blend_factor(
                    game,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed"),
                    0.5
                ),
                None
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mob_takes_effects_by_the_games_own_paths() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            for (id, kind) in [
                (12, "minecraft:zombie"),
                (13, "minecraft:spider"),
                (14, "minecraft:cow"),
            ] {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs.push(Mob::new(id, kind, Vec3::ZERO, 0.0));
            }
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let spider = game.living(game.entity(13).unwrap()).unwrap();
            let cow = game.living(game.entity(14).unwrap()).unwrap();

            // Reusing the registered key reads the applied instance.
            assert!(game.living_add_effect(zombie, &effect("minecraft:speed", 200, 0)));
            assert!(game.living_potion_affected(
                zombie,
                mod_platform::__private::part_tests::effect_key("speed")
            ));
            assert_eq!(
                game.living_effect(
                    zombie,
                    mod_platform::__private::part_tests::effect_key("speed")
                ),
                Some(effect("minecraft:speed", 200, 0))
            );
            assert_eq!(
                collect_effects(game, zombie),
                Ok(vec![effect("minecraft:speed", 200, 0)])
            );

            // A re-add is `MobEffectInstance.update`: a weaker or shorter
            // one changes nothing, a stronger replaces with its own
            // duration, an equal one only extends.
            assert!(!game.living_add_effect(zombie, &effect("minecraft:speed", 100, 0)));
            assert_eq!(
                game.living_effect(
                    zombie,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed")
                ),
                Some(effect("minecraft:speed", 200, 0))
            );
            assert!(game.living_add_effect(zombie, &effect("minecraft:speed", 90, 1)));
            assert_eq!(
                game.living_effect(
                    zombie,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed")
                ),
                Some(effect("minecraft:speed", 90, 1))
            );

            // canBeAffected: a spider shrugs off poison and an undead one
            // with it, a cow takes it.
            assert!(!game.living_add_effect(spider, &effect("minecraft:poison", 100, 0)));
            assert_eq!(collect_effects(game, spider), Ok(Vec::new()));
            assert!(!game.living_add_effect(zombie, &effect("minecraft:poison", 100, 0)));
            assert!(game.living_add_effect(cow, &effect("minecraft:poison", 100, 0)));

            // A duration below -1 changes nothing.

            assert!(!game.living_add_effect(zombie, &effect("minecraft:speed", -2, 0)));

            // remove_effect takes its absorption hearts with it; a second
            // remove has nothing to do.
            assert!(game.living_add_effect(zombie, &effect("minecraft:absorption", 100, 1)));
            game.mobs[0].absorption = 8.0;
            assert!(game.living_remove_effect(
                zombie,
                mod_platform::__private::part_tests::effect_key("minecraft:absorption")
            ));
            assert_eq!(game.mobs[0].absorption, 0.0);
            assert!(!game.living_remove_effect(
                zombie,
                mod_platform::__private::part_tests::effect_key("minecraft:absorption")
            ));
            assert!(game.living_potion_affected(
                zombie,
                mod_platform::__private::part_tests::effect_key("minecraft:speed")
            ));

            // remove_all_effects takes the rest, and says when there was none.
            assert!(game.living_remove_all_effects(zombie));
            assert_eq!(collect_effects(game, zombie), Ok(Vec::new()));
            assert!(!game.living_potion_affected(
                zombie,
                mod_platform::__private::part_tests::effect_key("minecraft:speed")
            ));
            assert!(!game.living_remove_all_effects(zombie));

            // A stale handle reads nothing and writes nothing.
            let stale = EntityRef::new(WorldEpoch::new(zombie.entity().epoch().get() + 1), 12);
            let stale = LivingRef::new(stale);
            assert!(!game.living_add_effect(stale, &effect("minecraft:speed", 100, 0)));
            assert_eq!(
                game.living_effect(
                    stale,
                    mod_platform::__private::part_tests::effect_key("minecraft:speed")
                ),
                None
            );
            assert_eq!(
                stale.visit_effects(game, &mut |_| panic!("stale effects visited")),
                Err(EntityReadError::Stale)
            );
            assert!(!game.living_potion_affected(
                stale,
                mod_platform::__private::part_tests::effect_key("minecraft:speed")
            ));
            assert!(!game.living_remove_effect(
                stale,
                mod_platform::__private::part_tests::effect_key("minecraft:speed")
            ));
            assert!(!game.living_remove_all_effects(stale));
        });
    }
}
