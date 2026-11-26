//! The totem of undying (`LivingEntity.checkTotemDeathProtection`: a hand's
//! item carrying the `death_protection` component, whatever item it is, and
//! entity event 35 on the client).
use super::Game;
use super::render_tick::player_tracking_body;
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{
    block_entity_render::vertex_light,
    health::HealthEvent,
    items::ItemStack,
    mesher::Vertex,
    motion::MotionEvent,
    particles::{ParticleOptions, ParticleType, TrackingBody, TrackingTarget},
};

const TOTEM: &str = "minecraft:totem_of_undying";

/// `EntityEvent.TOTEM_OF_UNDYING`.
pub(super) const TOTEM_EVENT: i8 = 35;

impl Game<'_> {
    /// Whether the stack has `DataComponents.DEATH_PROTECTION`, whatever
    /// item it is: the totem of undying is only one item that carries it.
    fn has_death_protection(&self, s: &ItemStack) -> bool {
        death_protection_of(&self.pack.item_renderer.models, &s.item, Some(s)).is_some()
    }
    /// The `death_effects` of the stack's `minecraft:death_protection`.
    fn death_effects(&self, s: &ItemStack) -> Vec<sunrise_client::items::ConsumeEffect> {
        s.death_protection(self.items().get(&s.item))
            .unwrap_or_default()
            .to_vec()
    }
    /// Finish the accepted protection on the live player before damage Post.
    fn apply_totem(
        &mut self,
        entity: i32,
        protection: sunrise_client::health::PlayerProtection,
    ) -> Option<Body> {
        let slot = protection.hand.map(|hand| match hand {
            mod_api::Hand::Main => self.selected,
            mod_api::Hand::Off => super::shield::OFFHAND_SLOT,
        });
        let protector = protection.stack;
        self.player.as_ref().filter(|p| p.entity_id == entity)?;
        // A mod's protector from outside the hands is its own to take.
        if let Some(slot) = slot {
            self.consume_player_slot(slot, 1);
        }
        self.award_stat("used", &protector.item, 1);
        self.trigger_item("minecraft:used_totem", &protector);
        let p = self.player.as_ref().filter(|p| p.entity_id == entity)?;
        let body = Body {
            bounds: player_tracking_body(p),
            protector: Some(protector.clone()),
        };
        // ItemStack.causeUseVibration: the totem's use effects make one.
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:item_interact_finish", body.bounds.feet, ctx);
        let p = self.player.as_mut().filter(|p| p.entity_id == entity)?;
        let _ = p.set_health(1.0, false);
        // `DeathProtection.applyEffects` reads the component from the accepted copy.
        let user = self.consuming_player_entity();
        for effect in self.death_effects(&protector) {
            self.apply_consume_effect(&protector, &effect, None, user);
        }
        if self.lan_acting.is_some()
            && let Some(host) = &self.lan
        {
            host.hub.entity_event(entity, TOTEM_EVENT);
        }
        Some(body)
    }
    /// Finish a hit before its caller can hurt the acting player again. Protection and
    /// `ServerPlayer.die` both run on the current body, not its between-turn projection.
    pub(in crate::game) fn finish_guest_damage(&mut self) {
        let _ = self.finish_guest_damage_before_resume();
    }

    /// Settle the hit, returning the remaining events instead of resuming if the acting body is gone.
    pub(in crate::game) fn finish_guest_damage_before_resume(
        &mut self,
    ) -> Result<(), Vec<MotionEvent>> {
        let (Some(conn), Some(player)) = (self.lan_acting, self.player.as_mut()) else {
            return Err(Vec::new());
        };
        let Some(hit) = player.health.damage_completion.take() else {
            return Ok(());
        };
        let entity = player.entity_id;
        // Keep queued motion and packet effects with this body across the owning callbacks.
        let mut events = std::mem::take(&mut player.events);
        if let Some(protection) = hit.protection {
            if let Some(body) = self.apply_totem(entity, protection)
                && self.serving.is_none()
                && !self.is_dedicated_server()
            {
                self.totem_event(TrackingTarget::Entity(entity), body);
            }
        } else if let Some(index) = self.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .connection_index(conn)
                .filter(|&i| !host.hub.guests[i].dead)
        }) {
            // The killing blow and its weapon must reach the combat tracker before die reads it.
            // Packet and motion events still belong to the normal turn drain.
            let combat: Vec<_> = events
                .extract_if(.., |event| {
                    matches!(
                        event,
                        MotionEvent::Health(
                            HealthEvent::ArmorHurt { .. }
                                | HealthEvent::Criterion { .. }
                                | HealthEvent::MobHurt
                                | HealthEvent::RecordedDamage { .. }
                                | HealthEvent::Statistic { .. }
                        )
                    )
                })
                .collect();
            if !combat.is_empty() {
                self.record_combat(&combat);
                self.wear_armor(&combat);
            }
            self.guest_die(index, conn);
        }
        // The death or protection callback may change or remove the body. Post is still for
        // this hit, not for whatever state the callback left in its place.
        if let Some(post) = hit.post {
            self.finish_damage_post(entity, "minecraft:player", post);
        }
        if self.lan_acting == Some(conn)
            && let Some(player) = self.player.as_mut().filter(|p| p.entity_id == entity)
        {
            events.append(&mut player.events);
            player.events = events;
            Ok(())
        } else {
            Err(events)
        }
    }

    /// Entity event 35 from a server, for this client's own player or an
    /// entity it draws.
    pub(super) fn remote_totem(&mut self, id: i32) {
        if let Some((target, bounds)) = self.remote_tracking_body(id) {
            self.totem_event(
                target,
                Body {
                    bounds,
                    protector: None,
                },
            );
        }
    }
    /// `ClientPacketListener.handleEntityEvent` 35: the totem particles (a
    /// 30-tick emitter) and its sound, heard here only (`playLocalSound`),
    /// and the player's own rumble.
    pub(super) fn totem_event(&mut self, target: TrackingTarget, body: Body) {
        self.particles.tracking_emitter_for(
            ParticleOptions::simple(ParticleType::TotemOfUndying),
            target,
            body.bounds,
            30,
            None,
        );
        self.play_audio_unshared("minecraft:item.totem.use", Some(body.bounds.feet), 1.0, 1.0);
        if target == TrackingTarget::LocalPlayer {
            self.input_feedback(sunrise_client::input_source::Feedback::Totem);
            // findTotem, then LocalPlayer.displayItemActivation. The offsets
            // come from the player's own random in vanilla, not the level's:
            // the client-only particle random stands in for it. The server
            // half hands over the stack it used up; only an event from a
            // remote server, whose item is still in the hand, looks one up.
            let totem = body.protector.unwrap_or_else(|| self.find_totem());
            self.item_activation
                .activate(totem, self.particles.random());
        }
    }
    /// `ClientPacketListener.findTotem`: the first hand's item with
    /// `death_protection`, else a new totem (a used single totem is gone by
    /// the time the event arrives).
    fn find_totem(&self) -> ItemStack {
        [self.held(), self.offhand.as_ref()]
            .into_iter()
            .flatten()
            .find(|s| self.has_death_protection(s))
            .cloned()
            .unwrap_or_else(|| ItemStack::new(TOTEM, 1))
    }
    /// `GameRenderer.tick`: the activated item counts down.
    pub(super) fn tick_item_activation(&mut self) {
        if self.player.is_some() {
            self.item_activation.tick();
        }
    }
    /// `ScreenEffectRenderer.renderItemActivationAnimation`, for the hand
    /// pass: the item in view space with the hand's projection but none of
    /// the camera's turn or bob (a fresh `PoseStack`), so those are undone
    /// here. `FIXED` display, no owner, seed 0 (`LevelExtractor`), full
    /// brightness and `Lighting.Entry.ITEMS_3D`. Only F1 hides it.
    pub(super) fn item_activation_geometry(
        &self,
        partial: f32,
        view: glam::Mat4,
        aspect: f32,
        out: &mut Vec<Vertex>,
    ) {
        if self.ui.hide_gui {
            return;
        }
        let (Some(stack), Some(pose)) = (
            self.item_activation.active(),
            self.item_activation.pose(partial, aspect),
        ) else {
            return;
        };
        let state = self.resolve_item(stack, DisplayContext::Fixed, None, 0);
        let to_world = view.inverse();
        let (a, b) = sunrise_client::item_render::gui::items_3d_lights();
        let lights = (
            to_world.transform_vector3(a).normalize(),
            to_world.transform_vector3(b).normalize(),
        );
        // PvP & HUD's totem animation size (100% is vanilla's), about the item's centre.
        let size = self.pvp_totem_scale();
        let pose = pose * glam::Mat4::from_scale(glam::Vec3::splat(size));
        self.submit_item_lit(&state, to_world * pose, vertex_light(0xFF), lights, out);
    }
}

/// The `minecraft:death_protection` component of `item` (of `stack`, when
/// there is one): the stack's own patch when it sets or removes it, else the
/// item's default. It runs for both of the player's hands and for every
/// armed mob each tick, so the usual stack, which leaves the component alone,
/// costs one map lookup instead of a component map built from its patch.
fn death_protection_of<'m>(
    models: &'m sunrise_assets::item_model::ItemModels,
    item: &str,
    stack: Option<&ItemStack>,
) -> Option<std::borrow::Cow<'m, serde_json::Value>> {
    const KEY: &str = "minecraft:death_protection";
    if let Some(s) = stack
        && let Some(data) = s.data()
        && (data.other(KEY).is_some() || data.other("!minecraft:death_protection").is_some())
    {
        return models
            .components(&sunrise_client::item_render::stack_view(s))
            .get(KEY)
            .cloned()
            .map(std::borrow::Cow::Owned);
    }
    models
        .prototypes
        .get(item)?
        .get(KEY)
        .map(std::borrow::Cow::Borrowed)
}

#[cfg(test)]
mod death_protection_tests {
    use super::death_protection_of;
    use serde_json::json;
    use sunrise_assets::item_model::ItemModels;
    use sunrise_client::items::ItemStack;
    use sunrise_core::nbt::Tag;

    /// The lookup gives what the stack's whole component map gives, for
    /// stacks that keep, set or remove the component.
    #[test]
    fn hand_lookup_matches_the_component_map() {
        let totem = json!({"death_effects": [{"type": "minecraft:clear_all_effects"}]});
        let mut models = ItemModels::default();
        let prototypes = std::sync::Arc::make_mut(&mut models.prototypes);
        prototypes.insert(
            "minecraft:totem_of_undying".into(),
            [("minecraft:death_protection".to_owned(), totem.clone())].into(),
        );
        prototypes.insert("minecraft:diamond_sword".into(), Default::default());
        let mut patched_sword = ItemStack::new("minecraft:diamond_sword", 1);
        patched_sword.data_mut().set_other(
            "minecraft:death_protection",
            Some(Tag::Compound(Default::default())),
        );
        let mut stripped_totem = ItemStack::new("minecraft:totem_of_undying", 1);
        stripped_totem.data_mut().set_other(
            "!minecraft:death_protection",
            Some(Tag::Compound(Default::default())),
        );
        for (stack, expected) in [
            (ItemStack::new("minecraft:totem_of_undying", 1), Some(totem)),
            (ItemStack::new("minecraft:diamond_sword", 1), None),
            (patched_sword, Some(json!({}))),
            (stripped_totem, None),
            (ItemStack::new("minecraft:unknown_item", 1), None),
        ] {
            let full = models
                .components(&sunrise_client::item_render::stack_view(&stack))
                .get("minecraft:death_protection")
                .cloned();
            let fast =
                death_protection_of(&models, &stack.item, Some(&stack)).map(|v| v.into_owned());
            assert_eq!(fast, expected, "{}", stack.item);
            assert_eq!(full, expected, "{}", stack.item);
            // A mob's off hand is an item id alone.
            if stack.data().is_none() {
                let by_id = death_protection_of(&models, &stack.item, None).map(|v| v.into_owned());
                assert_eq!(by_id, expected, "{}", stack.item);
            }
        }
    }

    /// A server sends the component as wire bytes, not as a saved tag: the
    /// stack's data has to carry it for the hand lookup, or an apple given
    /// `[death_protection={}]` animates as a plain totem (#12632).
    #[test]
    fn a_servers_patch_reaches_the_hand_lookup() {
        use sunrise_client::items::{ConsumeEffect, ConsumeEffectSpec};
        use sunrise_protocol::item::{Component, Stack, component};
        use sunrise_protocol::registry::{Builtin, Dynamic, Ids};

        let items = [
            "minecraft:air",
            "minecraft:apple",
            "minecraft:totem_of_undying",
        ];
        let mut names = vec![String::new(); component::DEATH_PROTECTION as usize + 1];
        names[component::DEATH_PROTECTION as usize] = "minecraft:death_protection".into();
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".into(),
                    Ids::from_names(items.iter().map(|s| s.to_string()).collect()),
                ),
                (
                    "minecraft:data_component_type".into(),
                    Ids::from_names(names),
                ),
                (
                    "minecraft:mob_effect".into(),
                    Ids::from_names(vec!["minecraft:speed".into()]),
                ),
            ]
            .into(),
        };
        let from_wire = |item, added, removed| {
            let mut wire = Stack::new(item, 1);
            wire.added = added;
            wire.removed = removed;
            crate::item_sync::from_network(&wire, &builtin, &Dynamic::default())
        };
        let models = ItemModels::default();
        // `death_protection={}`: an empty effect list and no saved tag.
        let apple = from_wire(
            1,
            vec![(component::DEATH_PROTECTION, Component::Raw(vec![0]))],
            vec![],
        );
        assert_eq!(
            death_protection_of(&models, &apple.item, Some(&apple)).map(|v| v.into_owned()),
            Some(json!({})),
            "the apple carries the server's component"
        );
        // `clear_all_effects`, then speed 1 for 100 ticks.
        let mut bytes = vec![2, 2, 0, 1, 0, 1, 100, 0, 1, 1, 0];
        bytes.extend(1.0f32.to_bits().to_be_bytes());
        let boosted = from_wire(
            1,
            vec![(component::DEATH_PROTECTION, Component::Raw(bytes))],
            vec![],
        );
        let component = death_protection_of(&models, &boosted.item, Some(&boosted))
            .expect("the apple carries the server's component")
            .into_owned();
        assert_eq!(
            sunrise_client::items::parse_death_effects(&component).unwrap(),
            [
                ConsumeEffect::ClearAllEffects,
                ConsumeEffect::ApplyEffects {
                    effects: vec![ConsumeEffectSpec {
                        id: "minecraft:speed".into(),
                        duration: 100,
                        amplifier: 1,
                        ambient: false,
                        show_particles: true,
                        show_icon: true,
                    }],
                    probability: 1.0,
                }
            ],
            "death_effects cross the wire"
        );
        // `totem_of_undying[!death_protection]` keeps the prototype's
        // component from protecting.
        let mut models = ItemModels::default();
        std::sync::Arc::make_mut(&mut models.prototypes).insert(
            "minecraft:totem_of_undying".into(),
            [("minecraft:death_protection".to_owned(), json!({}))].into(),
        );
        let stripped = from_wire(2, vec![], vec![component::DEATH_PROTECTION]);
        assert_eq!(
            death_protection_of(&models, &stripped.item, Some(&stripped)),
            None,
            "a removed component is no protection"
        );
    }
}

/// Where an entity event's entity stands and its size, and, for the event
/// this window's own player fires after the server half used a stack up, the
/// protector copy the activation animation shows.
#[derive(Clone)]
pub(super) struct Body {
    pub bounds: TrackingBody,
    protector: Option<ItemStack>,
}
