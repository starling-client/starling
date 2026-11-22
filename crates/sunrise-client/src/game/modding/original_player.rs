//! The old `ServerPlayer` retained across `restoreFrom`, independently of its replacement.
use super::{Dim, Game};
use mod_api::{
    ActiveEffect, AttachmentError, AttachmentForm, AttachmentStore, AttachmentValue, Attachments,
    Holder, InventoryView, LevelRef, OriginalPlayerAccess, PlayerExperience,
};
use std::sync::{Arc, Mutex};
use sunrise_core::nbt::Compound;
use sunrise_items::equipment::Equipment;
use sunrise_items::items::ItemStack;

/// Per-player attachment state and active replacement player UUIDs during clone hooks.
#[derive(Default)]
pub(super) struct PlayerState {
    pub(super) attachments: sunrise_mod_runtime::attachments::ResidentAttachments,
    pub(super) cloning_players: Vec<(u128, u32, u32)>,
}

/// Retained original-player attachment storage tied to `Game::transactions` so mutations,
/// default insertions, removals and decode caching roll back with an aborted transaction.
pub(in crate::game) struct OriginalAttachmentSlot {
    map: Attachments,
    active_frames: Vec<u64>,
    journal: Vec<(u64, Attachments)>,
}

impl OriginalAttachmentSlot {
    fn new(map: Attachments) -> Self {
        Self {
            map,
            active_frames: Vec::new(),
            journal: Vec::new(),
        }
    }

    pub(in crate::game) fn open_frame(&mut self, token: u64) {
        self.active_frames.push(token);
    }

    pub(in crate::game) fn close_frame(&mut self, token: u64, commit: bool) {
        if let Some(pos) = self.active_frames.iter().rposition(|&t| t == token) {
            self.active_frames.remove(pos);
        }
        let kept = self
            .journal
            .iter()
            .rposition(|(t, _)| *t == token)
            .map(|pos| self.journal.remove(pos).1);
        let Some(kept) = kept else {
            return;
        };
        if commit {
            if let Some(&outer) = self.active_frames.last()
                && !self.journal.iter().any(|(t, _)| *t == outer)
            {
                self.journal.push((outer, kept));
            }
        } else {
            self.map = kept;
        }
    }

    fn journal_current(&mut self) {
        if let Some(&token) = self.active_frames.last()
            && !self.journal.iter().any(|(t, _)| *t == token)
        {
            self.journal.push((token, self.map.clone()));
        }
    }
}

struct RetainedOriginalAttachments {
    slot: Arc<Mutex<OriginalAttachmentSlot>>,
}

impl RetainedOriginalAttachments {
    fn new(map: Attachments) -> Self {
        Self {
            slot: Arc::new(Mutex::new(OriginalAttachmentSlot::new(map))),
        }
    }
}

impl AttachmentStore for RetainedOriginalAttachments {
    fn validate_attachment(
        &mut self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)
    }

    fn holds(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
    ) -> Result<bool, AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, false)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        AttachmentStore::holds(&mut slot.map, holder, namespace, name)
    }

    fn decoded(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
    ) -> Result<Option<AttachmentValue>, AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, false)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        AttachmentStore::decoded(&mut slot.map, holder, namespace, name)
    }

    fn saved(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
    ) -> Result<Option<Compound>, AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, false)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        AttachmentStore::saved(&mut slot.map, holder, namespace, name)
    }

    fn store(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
        value: AttachmentValue,
        form: AttachmentForm,
    ) -> Result<(), AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, form.policy.sync)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.journal_current();
        AttachmentStore::store(&mut slot.map, holder, namespace, name, value, form)
    }

    fn discard(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, sync)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.journal_current();
        AttachmentStore::discard(&mut slot.map, holder, namespace, name, sync)
    }

    fn queue_sync(
        &mut self,
        holder: Holder,
        namespace: &str,
        name: &str,
    ) -> Result<(), AttachmentError> {
        mod_api::__private::validate_attachment_use(namespace, name, true)?;
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.journal_current();
        AttachmentStore::queue_sync(&mut slot.map, holder, namespace, name)
    }
}

/// Pre-copy state of the original `ServerPlayer`, lent directly to `player_clone`.
pub(in crate::game) struct RetainedOriginalPlayer {
    uuid: u128,
    position: [f64; 3],
    dimension: String,
    level: Option<LevelRef>,
    health: f32,
    experience: PlayerExperience,
    effects: Vec<ActiveEffect>,
    inventory: Vec<Option<ItemStack>>,
    armor: [Option<ItemStack>; 4],
    offhand: Option<ItemStack>,
    equipment: Equipment,
    persistent_data: Option<Compound>,
    attachments: RetainedOriginalAttachments,
}

impl OriginalPlayerAccess for RetainedOriginalPlayer {
    fn uuid(&self) -> u128 {
        self.uuid
    }

    fn position(&self) -> [f64; 3] {
        self.position
    }

    fn dimension(&self) -> &str {
        &self.dimension
    }

    fn level(&self) -> Option<LevelRef> {
        self.level
    }

    fn health(&self) -> f32 {
        self.health
    }

    fn experience(&self) -> PlayerExperience {
        self.experience
    }

    fn effects(&self) -> &[ActiveEffect] {
        &self.effects
    }

    fn with_inventory(&self, read: &mut dyn for<'a> FnMut(InventoryView<'a>)) {
        super::native_player_inventory::lend_inventory_view(
            &self.inventory,
            &self.armor,
            &self.offhand,
            &self.equipment,
            read,
        );
    }

    fn persistent_data(&self) -> Option<&Compound> {
        self.persistent_data.as_ref()
    }

    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        &mut self.attachments
    }
}

impl Game<'_> {
    /// Make the callback's player addressable without publishing it in a level's player list
    /// or entity table. This also covers construction and the post-clone set-spawn event.
    pub(super) fn with_cloning_player<R>(
        &mut self,
        conn: u32,
        run: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let guest = self.command_guest(conn)?;
        let identity = (guest.uuid, conn, guest.entity_id as u32);
        self.player_state.cloning_players.push(identity);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(self)));
        self.player_state.cloning_players.pop();
        match result {
            Ok(result) => Some(result),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    /// Common `ServerPlayer.restoreFrom` boundary for death respawn (`was_death = true`)
    /// and End credits return (`was_death = false`).
    ///
    /// Expects old-player removal/menu-close work to have completed against the old state
    /// and the destination level to be open (`self.enter_level`). Copies eligible
    /// attachments into a fresh replacement map unconditionally, retains the old player
    /// when `player_clone` is listened, assigns the replacement's destination dimension
    /// without placing it, reinitializes the replacement `ServerPlayer`, and dispatches
    /// `player_clone` with both holders journaled to `host.transaction`.
    pub(in crate::game) fn restore_cloned_player(
        &mut self,
        conn: u32,
        destination: Dim,
        was_death: bool,
        restore_replacement: impl FnOnce(&mut sunrise_client::lan::guest::Guest),
        after_clone: impl FnOnce(&mut Self),
    ) -> bool {
        let Some((i, uuid, name)) = self.lan.as_ref().and_then(|host| {
            let i = host.hub.guests.iter().position(|g| g.conn == conn)?;
            let g = &host.hub.guests[i];
            Some((i, g.uuid, g.name.clone()))
        }) else {
            return false;
        };
        debug_assert!(
            self.transactions.is_empty(),
            "respawn must not replace an open journal's player"
        );
        let Some(map) = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.guests.attachments_mut(conn))
        else {
            return false;
        };
        let previous = std::mem::take(&mut **map);
        let replacement = previous.copy_for_clone(was_death);
        let mut original = self.retain_original_player(conn, previous);
        // NeoForge's clone keeps only the `PlayerPersisted` part of a mod's saved compound.
        self.keep_only_player_persisted(conn);
        if let Some(host) = self.lan.as_mut() {
            host.hub.guests.update(i, |g| {
                g.dimension = destination.id().to_owned();
                restore_replacement(g);
            });
            if let Some(map) = host.hub.guests.attachments_mut(conn) {
                **map = replacement;
            }
        }
        self.with_cloning_player(conn, |game| {
            game.reinitialize_server_player(uuid, conn);
            if let Some(original) = &mut original {
                game.transactions
                    .push_retained_original(Arc::clone(&original.attachments.slot));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.native_player_clone(
                        uuid,
                        &name,
                        was_death,
                        mod_api::OriginalPlayer::new(original),
                    );
                }));
                game.transactions.pop_retained_original();
                if let Err(panic) = result {
                    std::panic::resume_unwind(panic);
                }
            }
            after_clone(game);
        })
        .is_some()
    }

    /// `ServerPlayer.restoreFrom`: retain the old player's state before replacing any data.
    pub(in crate::game) fn retain_original_player(
        &self,
        conn: u32,
        detached_attachments: Attachments,
    ) -> Option<RetainedOriginalPlayer> {
        if !mod_api::__private::listened(mod_api::HookId::PlayerClone) {
            return None;
        }
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        Some(RetainedOriginalPlayer {
            uuid: guest.uuid,
            position: guest.position,
            dimension: guest.dimension.clone(),
            level: self.level(&guest.dimension),
            health: guest.health,
            experience: PlayerExperience::from(mod_api::__private::built::PlayerExperience {
                level: guest.xp_level,
                progress: guest.xp_progress,
                total: guest.xp_total,
            }),
            effects: guest
                .effects
                .list
                .iter()
                .map(super::native_living_effects::active_effect)
                .collect(),
            inventory: guest.inventory.clone(),
            armor: guest.armor.clone(),
            offhand: guest.offhand.clone(),
            equipment: guest.equipment.clone(),
            persistent_data: super::native_player_persistent_data::persistent_data_compound(
                guest.saved.as_ref(),
            )
            .cloned(),
            attachments: RetainedOriginalAttachments::new(detached_attachments),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_guest_turn_tests::with_guests;
    use mod_api::{
        AttachmentType, Holder, Hooks, HostTransaction, PlayerCloneEvent,
        PlayerContainerCloseEvent, ServerHost,
    };
    use sunrise_core::nbt::{JavaString, Tag};

    const VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "unsaved",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    const DEFAULTED: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "defaulted",
        default: || 41,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    const RETAINED: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "retained",
        default: || 0,
        codec: Some(mod_api::AttachmentCodec {
            encode: |value| Ok([("value".into(), Tag::Int(*value))].into()),
            decode: |value| match value.get(&JavaString::from("value")) {
                Some(Tag::Int(value)) => Ok(*value),
                _ => Err(mod_api::AttachmentError::Decode),
            },
        }),
        copy_on_death: true,
        sync: false,
    };
    const SAVED: AttachmentType<i32> = AttachmentType {
        name: "saved",
        copy_on_death: false,
        ..RETAINED
    };

    struct Listener {
        death: bool,
        keep: bool,
        heard: Arc<Mutex<usize>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_attachment_type("clone:unsaved", false).unwrap();
            r.register_attachment_type("clone:defaulted", false)
                .unwrap();
            r.register_attachment_type("clone:retained", false).unwrap();
            r.register_attachment_type("clone:saved", false).unwrap();
        }

        fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
            assert_eq!(event.was_death, self.death);
            assert_eq!(event.original.uuid(), 12);
            assert_eq!(event.original.position(), [7.0, 64.0, -9.0]);
            let dimension = if self.death { Dim::Nether } else { Dim::End };
            assert_eq!(
                event.original.dimension().unwrap().to_string(),
                dimension.id()
            );
            let original_level = event.original.level().unwrap();
            assert_eq!(original_level.dimension_id(), dimension.id());
            assert_eq!(event.original.health(), if self.death { 0.0 } else { 13.0 });
            assert_eq!(event.original.experience().level, 7);
            let effects = event.original.effects();
            assert_eq!(effects.len(), 1);
            assert_eq!(effects[0].duration, 200);
            assert_eq!(effects[0].amplifier, 1);

            event.original.with_inventory(|inv| {
                if self.death && !self.keep {
                    assert!(inv.get(2).is_none());
                    assert!(inv.get(36).is_none());
                    assert!(inv.get(40).is_none());
                } else {
                    assert_eq!(inv.get(2).unwrap().count(), 4);
                    assert_eq!(inv.get(36).unwrap().count(), 1);
                    assert_eq!(inv.get(40).unwrap().count(), 2);
                }
                assert!(inv.get(99).is_none());
            });

            let outside = JavaString::from("outside");
            let persisted = JavaString::from("PlayerPersisted");
            let persisted_value = Tag::compound([("inside", Tag::Int(2))]);
            let original_data = event.original.persistent_data().unwrap();
            assert_eq!(original_data.get(&outside), Some(&Tag::Int(1)));
            assert_eq!(original_data.get(&persisted), Some(&persisted_value));
            let replacement = host.player(event.player).unwrap();
            // Cross-dimension clone: replacement is already in its destination level while
            // the retained original preserves its source level.
            let replacement_level = replacement.level(host).unwrap();
            assert_eq!(replacement_level.dimension_id(), Dim::Overworld.id());
            assert_eq!(
                replacement.dimension(host).unwrap().to_string(),
                Dim::Overworld.id()
            );
            assert_ne!(original_level, replacement_level);
            assert_eq!(replacement.entity().position(host), Some([0.0; 3]));
            let _ = replacement_level
                .visit_online_players(host, &mut |found| {
                    assert_ne!(
                        found, replacement,
                        "replacement is not resident during clone"
                    );
                    core::ops::ControlFlow::Continue(())
                })
                .unwrap();
            let replacement_data = replacement.persistent_data(host).unwrap();
            assert_eq!(replacement_data.get(&outside), None);
            assert_eq!(replacement_data.get(&persisted), Some(&persisted_value));
            let replacement_effects = mod_api::LivingRef::new(replacement.entity())
                .effects(host)
                .unwrap();
            assert_eq!(replacement_effects.len(), usize::from(!self.death));

            const MISSING: AttachmentType<i32> = AttachmentType {
                name: "missing",
                ..VALUE
            };
            // Never declared, so refused by name (#1448), not read as its default.
            assert_eq!(
                event.original.attachment(&MISSING),
                Err(mod_api::AttachmentError::Undeclared {
                    package_id: "clone".into(),
                    attachment_id: "clone:missing".into(),
                })
            );

            // A declared but absent key yields its default without writing either holder.
            assert_eq!(event.original.attachment(&DEFAULTED), Ok(41));
            assert_eq!(event.original.has_attachment(&DEFAULTED), Ok(false));
            assert_eq!(event.original.existing_attachment(&DEFAULTED), Ok(None));
            assert_eq!(
                event
                    .original
                    .with_decoded_attachment(&DEFAULTED, |value| value.copied()),
                Ok(None)
            );
            assert_eq!(DEFAULTED.has(host, Holder::Player(event.player)), Ok(false));

            // Saved codec-backed entry decodes once on existing_attachment and caches it.
            assert_eq!(event.original.has_attachment(&SAVED), Ok(true));
            assert_eq!(
                event
                    .original
                    .with_decoded_attachment(&SAVED, |value| value.copied()),
                Ok(None)
            );
            assert_eq!(event.original.existing_attachment(&SAVED), Ok(Some(31)));
            assert_eq!(
                event
                    .original
                    .with_decoded_attachment(&SAVED, |value| value.copied()),
                Ok(Some(31))
            );
            assert_eq!(
                SAVED.has(host, Holder::Player(event.player)),
                Ok(!self.death)
            );
            // Copied serializable entries start undecoded (independent of original's decoded cache).
            assert_eq!(
                SAVED.with_decoded(host, Holder::Player(event.player), |value| value.copied()),
                Ok(None)
            );
            assert_eq!(
                SAVED.get(host, Holder::Player(event.player)),
                Ok(if self.death { 0 } else { 31 })
            );

            // Codec-less attachment exists only on original until explicitly written to replacement,
            // on both death and End credits return (NeoForge AttachmentInternals.copyEntityAttachments).
            assert_eq!(event.original.attachment(&VALUE), Ok(73));
            assert_eq!(VALUE.has(host, Holder::Player(event.player)), Ok(false));
            assert_eq!(event.original.attachment(&RETAINED), Ok(17));
            assert_eq!(
                RETAINED.with_decoded(host, Holder::Player(event.player), |value| value.copied()),
                Ok(None)
            );
            assert_eq!(RETAINED.get(host, Holder::Player(event.player)), Ok(17));

            // Writes and removals on original affect only original, leaving replacement untouched.
            assert_eq!(event.original.set_attachment(&VALUE, 100), Ok(()));
            assert_eq!(event.original.existing_attachment(&VALUE), Ok(Some(100)));
            assert_eq!(VALUE.has(host, Holder::Player(event.player)), Ok(false));
            assert_eq!(event.original.remove_attachment(&RETAINED), Ok(Some(17)));
            assert_eq!(event.original.has_attachment(&RETAINED), Ok(false));
            assert_eq!(RETAINED.get(host, Holder::Player(event.player)), Ok(17));

            // Mutating the replacement leaves original untouched.
            VALUE.set(host, Holder::Player(event.player), 99).unwrap();
            assert_eq!(event.original.existing_attachment(&VALUE), Ok(Some(100)));
            *self.heard.lock().unwrap() += 1;
        }
    }

    struct Observer {
        heard: Arc<Mutex<usize>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
            // The first listener read the default without storing it. Its explicit mutations
            // are visible to the second listener only on the holder it wrote to.
            assert_eq!(event.original.has_attachment(&DEFAULTED), Ok(false));
            assert_eq!(DEFAULTED.has(host, Holder::Player(event.player)), Ok(false));
            assert_eq!(event.original.existing_attachment(&VALUE), Ok(Some(100)));
            assert_eq!(VALUE.get(host, Holder::Player(event.player)), Ok(99));
            assert_eq!(event.original.has_attachment(&RETAINED), Ok(false));
            assert_eq!(RETAINED.get(host, Holder::Player(event.player)), Ok(17));
            assert_eq!(
                event
                    .original
                    .with_decoded_attachment(&SAVED, |value| value.copied()),
                Ok(Some(31))
            );
            *self.heard.lock().unwrap() += 1;
        }
    }

    fn clone_case(death: bool, keep: bool) {
        let heard = Arc::new(Mutex::new(0));
        let observer_heard = Arc::new(Mutex::new(0));
        let mods = vec![
            mod_api::ModEntry::new(
                "clone",
                Box::new(Listener {
                    death,
                    keep,
                    heard: heard.clone(),
                }),
            ),
            mod_api::ModEntry::new(
                "observer",
                Box::new(Observer {
                    heard: observer_heard.clone(),
                }),
            ),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                let dimension = if death { Dim::Nether } else { Dim::End };
                game.enter_level(dimension).unwrap();
                game.ui.rules.set(
                    "keep_inventory",
                    sunrise_client::gamerules::Value::Bool(keep),
                );
                let host = game.lan.as_mut().unwrap();
                host.hub.guests.update(0, |guest| {
                    guest.dimension = dimension.id().to_owned();
                    guest.position = [7.0, 64.0, -9.0];
                    guest.spawn = Some([3.5, 64.0, 5.5]);
                    guest.dead = death;
                    guest.health = if death { 0.0 } else { 13.0 };
                    guest.xp_level = 7;
                    guest
                        .effects
                        .list
                        .push(sunrise_client::effects::EffectInstance::from_def(
                            sunrise_client_core::potions::effect_by_index(
                                mod_api::__private::effect_index(
                                    mod_platform::__private::part_tests::effect_key(
                                        "minecraft:speed",
                                    ),
                                )
                                .unwrap(),
                            )
                            .unwrap(),
                            200,
                            1,
                        ));
                    // Respawn starts after death drops; keep-inventory leaves these intact.
                    if !death || keep {
                        guest.inventory[2] = Some(ItemStack::new("minecraft:diamond", 4));
                        guest.armor[3] = Some(ItemStack::new("minecraft:diamond_boots", 1));
                        guest.offhand = Some(ItemStack::new("minecraft:apple", 2));
                    }
                    let mut neoforge_data = Compound::new();
                    neoforge_data.insert("outside".into(), Tag::Int(1));
                    neoforge_data.insert(
                        "PlayerPersisted".into(),
                        Tag::compound([("inside", Tag::Int(2))]),
                    );
                    guest.saved = Some(Tag::compound([(
                        "NeoForgeData",
                        Tag::Compound(neoforge_data),
                    )]));
                });
                let attachments = host.hub.guests.attachments_mut(12).unwrap();
                RETAINED.set(attachments, Holder::Level, 17).unwrap();
                SAVED.set(attachments, Holder::Level, 31).unwrap();
                // Keep codec-backed entries undecoded, beside an unserialized binding.
                let saved = Attachments::from_nbt(attachments.to_nbt());
                attachments.replace_contents(saved);
                VALUE.set(attachments, Holder::Level, 73).unwrap();
                if death {
                    game.guest_respawn(12);
                } else {
                    let play = host.play.entry(12).or_default();
                    play.won_game = true;
                    game.guest_respawn(12);
                }
                assert_eq!(*heard.lock().unwrap(), 1);
                assert_eq!(*observer_heard.lock().unwrap(), 1);
                assert!(game.player_state.cloning_players.is_empty());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn death_clone_retains_original_and_expires_after_dispatch() {
        crate::game::native_entity_refs::tests::run_with_game_stack(|| {
            clone_case(true, false);
            clone_case(true, true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn end_return_clone_retains_end_state_and_expires_after_dispatch() {
        crate::game::native_entity_refs::tests::run_with_game_stack(|| {
            clone_case(false, true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn unsubscribed_clone_does_not_capture() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            with_guests(|game| {
                assert!(
                    game.retain_original_player(12, Attachments::default())
                        .is_none()
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn clone_removes_menus_and_retires_old_attachment_owner_before_copy() {
        use mod_api::{AttachmentHolder, AttachmentRowError, AttachmentWriteToken};
        use sunrise_mod_runtime::attachments::index::AttachmentIndex;
        type OldIndex = Arc<Mutex<AttachmentIndex<AttachmentHolder>>>;

        struct CloseAndClone {
            death: bool,
            closed: Arc<Mutex<usize>>,
            cloned: Arc<Mutex<usize>>,
            source: Arc<Mutex<Option<(OldIndex, AttachmentWriteToken)>>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for CloseAndClone {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_attachment_type("clone:unsaved", false).unwrap();
                r.register_attachment_type("clone:saved", false).unwrap();
                r.register_attachment_type("clone:retained", false).unwrap();
            }

            fn player_container_close(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                VALUE.set(host, Holder::Player(event.player), 55).unwrap();
                SAVED.set(host, Holder::Player(event.player), 66).unwrap();
                RETAINED
                    .set(host, Holder::Player(event.player), 77)
                    .unwrap();
                *self.closed.lock().unwrap() += 1;
            }

            fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                assert_eq!(event.was_death, self.death);
                assert_eq!(*self.closed.lock().unwrap(), 1);
                let expected = if self.death { 0 } else { 10 };
                let count = event
                    .original
                    .with_inventory(|inv| inv.iter().map(|(_, stack)| stack.count()).sum::<u32>());
                assert_eq!(count, expected);
                let replacement = host.player(event.player).unwrap();
                assert_eq!(
                    replacement.with_inventory(host, |inv| inv
                        .iter()
                        .map(|(_, s)| s.count())
                        .sum::<u32>()),
                    Some(expected)
                );

                assert_eq!(event.original.existing_attachment(&VALUE), Ok(Some(55)));
                assert_eq!(event.original.existing_attachment(&SAVED), Ok(Some(66)));
                assert_eq!(event.original.existing_attachment(&RETAINED), Ok(Some(77)));
                assert_eq!(VALUE.has(host, Holder::Player(event.player)), Ok(false));
                assert_eq!(
                    SAVED.has(host, Holder::Player(event.player)),
                    Ok(!self.death)
                );
                assert_eq!(RETAINED.get(host, Holder::Player(event.player)), Ok(77));
                VALUE.set(host, Holder::Player(event.player), 999).unwrap();
                let source = self.source.lock().unwrap();
                let (source_index, old_token) = source.as_ref().unwrap();
                assert_eq!(
                    host.attachment_validate_write(old_token.clone()),
                    Err(AttachmentRowError::StaleHolder)
                );
                assert!(
                    source_index
                        .lock()
                        .unwrap()
                        .snapshot_named("clone", "retained")
                        .is_empty()
                );
                let batch = host.attachment_query("clone", "unsaved").unwrap();
                assert!(
                    batch
                        .query(&VALUE)
                        .unwrap()
                        .iter()
                        .try_next()
                        .unwrap()
                        .is_none(),
                    "clone writes are unpublished until placement"
                );
                *self.cloned.lock().unwrap() += 1;
            }
        }

        crate::game::native_entity_refs::tests::run_with_game_stack(|| {
            for death in [true, false] {
                let closed = Arc::new(Mutex::new(0));
                let cloned = Arc::new(Mutex::new(0));
                let source = Arc::new(Mutex::new(None));
                let mods = vec![mod_api::ModEntry::new(
                    "clone",
                    Box::new(CloseAndClone {
                        death,
                        closed: closed.clone(),
                        cloned: cloned.clone(),
                        source: source.clone(),
                    }),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_guests(|game| {
                        game.options.dedicated = Some(Vec::new());
                        let dim = if death { Dim::Overworld } else { Dim::End };
                        game.enter_level(dim).unwrap();
                        game.streaming.insert_remote(
                            &mut game.world,
                            sunrise_client::chunk::Chunk::empty(0, 0),
                        );
                        if !death {
                            assert!(
                                game.set_state_named([0, 64, 0], "minecraft:end_portal", &[])
                                    .unwrap()
                            );
                        }
                        let host = game.lan.as_mut().unwrap();
                        host.hub.guests.update(0, |guest| {
                            guest.dimension = dim.id().to_owned();
                            guest.position = [0.5, 64.0, 0.5];
                            guest.spawn = Some([3.5, 64.0, 5.5]);
                            guest.craft_grid[0] = Some(ItemStack::new("minecraft:oak_planks", 5));
                            guest.dead = death;
                            guest.health = if death { 0.0 } else { 20.0 };
                        });
                        let attachments = host.hub.guests.attachments_mut(12).unwrap();
                        VALUE.set(attachments, Holder::Level, 19).unwrap();
                        RETAINED.set(attachments, Holder::Level, 17).unwrap();
                        game.bind_player_attachments(12, false);
                        let index = game.attachment_indexes[&dim].clone();
                        let rows = index.lock().unwrap().snapshot_named("clone", "retained");
                        let batch: mod_api::AttachmentBatch =
                            sunrise_mod_runtime::attachments::AttachmentBatch::new(rows).into();
                        let query = batch.query(&RETAINED).unwrap();
                        let old = query.iter().try_next().unwrap().unwrap();
                        let token = old.write_token();
                        assert_eq!(game.attachment_validate_write(token.clone()), Ok(()));
                        *source.lock().unwrap() = Some((index, token));
                        game.lan_open_crafting(12, [0, 64, 0]);
                        let host = game.lan.as_mut().unwrap();
                        host.menus
                            .get_mut(&12)
                            .unwrap()
                            .crafting
                            .table
                            .as_mut()
                            .unwrap()
                            .2
                            .load_slot(1, Some(ItemStack::new("minecraft:stick", 3)));
                        host.hub.guests.update(0, |guest| {
                            guest.carried = Some(ItemStack::new("minecraft:apple", 2));
                        });
                        if death {
                            game.guest_respawn(12);
                            let mut drops: Vec<_> = game
                                .dropped
                                .iter()
                                .map(|item| (item.stack.item.to_string(), item.stack.count))
                                .collect();
                            drops.sort();
                            assert_eq!(
                                drops,
                                [
                                    ("minecraft:apple".to_owned(), 2),
                                    ("minecraft:oak_planks".to_owned(), 5),
                                    ("minecraft:stick".to_owned(), 3)
                                ]
                            );
                            assert_eq!(
                                VALUE.get(game, Holder::Player(12)),
                                Ok(999),
                                "the old UUID checkpoint must not overwrite the clone listener's write"
                            );
                            assert_eq!(RETAINED.get(game, Holder::Player(12)), Ok(77));
                        } else {
                            game.handle_server_portal(12);
                            assert!(game.lan.as_ref().unwrap().play[&12].won_game);
                            assert_eq!(*closed.lock().unwrap(), 1, "menu closes at credits entry");
                            assert_eq!(*cloned.lock().unwrap(), 0);
                            let guest = game.command_guest(12).unwrap();
                            assert!(!guest.placed && guest.respawning);
                            let before = guest.motion.as_ref().map(|m| m.health.tick_count);
                            game.tick_guests();
                            let guest = game.command_guest(12).unwrap();
                            assert_eq!(guest.motion.as_ref().map(|m| m.health.tick_count), before);
                            assert!(!game.lan.as_ref().unwrap().hub.here(guest));
                            game.guest_respawn(12);
                        }
                        let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                        assert_eq!(
                            guest
                                .inventory
                                .iter()
                                .flatten()
                                .map(|stack| stack.count)
                                .sum::<u32>(),
                            if death { 0 } else { 10 }
                        );
                        assert_eq!(*closed.lock().unwrap(), 1);
                        assert_eq!(*cloned.lock().unwrap(), 1);
                    });
                });
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn direct_respawn_clones_in_the_resolved_level_not_the_callers_level() {
        use sunrise_client::chunk::Chunk;
        use sunrise_core::dimension::Dimension;

        struct Destination {
            missing: bool,
            heard: Arc<Mutex<Vec<String>>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for Destination {
            fn player_respawn_position(
                &self,
                _: &mut dyn ServerHost,
                event: &mut mod_api::PlayerRespawnPositionEvent<'_>,
            ) {
                assert_eq!(event.missing_respawn_block, self.missing);
                assert_eq!(event.original_level, Dimension::Overworld);
                if self.missing {
                    *event.level = Dimension::End;
                    *event.position = [2.5, 90.0, 4.5];
                }
            }

            fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                assert_eq!(
                    event.original.level().unwrap().dimension_id(),
                    Dim::Overworld.id()
                );
                self.heard.lock().unwrap().push(
                    host.player(event.player)
                        .unwrap()
                        .level(host)
                        .unwrap()
                        .dimension_id()
                        .to_owned(),
                );
            }
        }

        crate::game::native_entity_refs::tests::run_with_game_stack(|| {
            for missing in [false, true] {
                let heard = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![mod_api::ModEntry::new(
                    "clone",
                    Box::new(Destination {
                        missing,
                        heard: heard.clone(),
                    }),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    with_guests(|game| {
                        game.options.dedicated = Some(Vec::new());
                        game.level.world_spawn = Some([0, 100, 0]);
                        assert!(game.ui.rules.set_string("respawn_radius", "0"));
                        if missing {
                            for x in -2..=2 {
                                for z in -2..=2 {
                                    game.streaming
                                        .insert_remote(&mut game.world, Chunk::empty(x, z));
                                }
                            }
                        }
                        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                            g.dead = true;
                            g.health = 0.0;
                            g.spawn = (!missing).then_some([7.5, 64.0, 5.5]);
                            g.respawn_elsewhere = missing.then(|| {
                                (Dim::Nether.id().to_owned(), [0, 100, 0], 0.0, 0.0, false)
                            });
                        });
                        game.enter_level(Dim::End).unwrap();
                        let player = game.player(12).unwrap();
                        player.respawn(game).unwrap();
                        assert_eq!(game.dim, Dim::End, "the caller's tick level is restored");
                        if missing {
                            assert!(
                                heard.lock().unwrap().is_empty(),
                                "do not clone before checking the Nether point"
                            );
                            game.enter_level(Dim::Nether).unwrap();
                            for x in -2..=2 {
                                for z in -2..=2 {
                                    game.streaming
                                        .insert_remote(&mut game.world, Chunk::empty(x, z));
                                }
                            }
                            game.tick_server_travel();
                        }
                        let wanted = if missing { Dim::End } else { Dim::Overworld };
                        assert_eq!(*heard.lock().unwrap(), [wanted.id().to_owned()]);
                        let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                        assert_eq!(guest.dimension, wanted.id());
                        assert_eq!(
                            guest.position,
                            if missing {
                                [2.5, 90.0, 4.5]
                            } else {
                                [7.5, 64.0, 5.5]
                            }
                        );
                        assert!(!guest.dead);
                    })
                });
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn unsubscribed_end_return_copies_only_serializable_attachments() {
        struct DeclareOnly;

        #[mod_api::__subscriptions]
        impl Hooks for DeclareOnly {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_attachment_type("clone:unsaved", false).unwrap();
                r.register_attachment_type("clone:saved", false).unwrap();
                r.register_attachment_type("clone:retained", false).unwrap();
            }
        }

        crate::game::native_entity_refs::tests::run_with_game_stack(|| {
            let mods = vec![mod_api::ModEntry::new("clone", Box::new(DeclareOnly))].into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.enter_level(Dim::End).unwrap();
                    let host = game.lan.as_mut().unwrap();
                    host.hub.guests.update(0, |guest| {
                        guest.dimension = Dim::End.id().to_owned();
                        guest.spawn = Some([3.5, 64.0, 5.5]);
                    });
                    let attachments = host.hub.guests.attachments_mut(12).unwrap();
                    VALUE.set(attachments, Holder::Level, 73).unwrap();
                    SAVED.set(attachments, Holder::Level, 31).unwrap();
                    RETAINED.set(attachments, Holder::Level, 17).unwrap();
                    let play = host.play.entry(12).or_default();
                    play.won_game = true;
                    game.guest_respawn(12);

                    let host: &mut dyn ServerHost = game;
                    assert_eq!(VALUE.has(host, Holder::Player(12)), Ok(false));
                    assert_eq!(
                        SAVED.with_decoded(host, Holder::Player(12), |v| v.copied()),
                        Ok(None)
                    );
                    assert_eq!(SAVED.get(host, Holder::Player(12)), Ok(31));
                    assert_eq!(
                        RETAINED.with_decoded(host, Holder::Player(12), |v| v.copied()),
                        Ok(None)
                    );
                    assert_eq!(RETAINED.get(host, Holder::Player(12)), Ok(17));
                });
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn respawn_copies_only_committed_attachment_state() {
        struct Declare;
        #[mod_api::__subscriptions]
        impl Hooks for Declare {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_attachment_type("clone:retained", false).unwrap();
            }
        }
        struct Observe(Arc<Mutex<Vec<i32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observe {
            fn player_clone(&self, _: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                self.0.lock().unwrap().push(
                    event
                        .original
                        .existing_attachment(&RETAINED)
                        .unwrap()
                        .unwrap(),
                );
            }
        }
        crate::game::native_entity_refs::tests::run_with_game_stack(|| {
            for subscribe in [false, true] {
                let seen = Arc::new(Mutex::new(Vec::new()));
                let mut mods = vec![mod_api::ModEntry::new("clone", Box::new(Declare))];
                if subscribe {
                    mods.push(mod_api::ModEntry::new(
                        "observe",
                        Box::new(Observe(seen.clone())),
                    ));
                }
                mod_platform::testing::with_mods(mods.into(), || {
                    with_guests(|game| {
                        game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                            g.spawn = Some([3.5, 64.0, 5.5]);
                            g.dead = true;
                            g.health = 0.0;
                        });
                        let holder = Holder::Player(12);
                        RETAINED.set(game, holder, 17).unwrap();
                        let txn = game.transaction_open();
                        RETAINED.set(game, holder, 23).unwrap();
                        game.guest_respawn(12);
                        assert!(game.command_guest(12).unwrap().dead);
                        game.transaction_close(&txn, false);
                        assert!(game.command_guest(12).unwrap().dead);
                        assert_eq!(RETAINED.get(game, holder), Ok(17));
                        assert!(seen.lock().unwrap().is_empty());
                        let txn = game.transaction_open();
                        RETAINED.set(game, holder, 29).unwrap();
                        game.guest_respawn(12);
                        game.transaction_close(&txn, true);
                        assert!(!game.command_guest(12).unwrap().dead);
                        assert_eq!(RETAINED.get(game, holder), Ok(29));
                        assert_eq!(
                            &*seen.lock().unwrap(),
                            if subscribe { &[29][..] } else { &[] }
                        );
                    });
                });
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn failed_transaction_rolls_back_both_original_and_replacement_attachments() {
        struct AbortWriter {
            heard: Arc<Mutex<usize>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for AbortWriter {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_attachment_type("clone:unsaved", false).unwrap();
                r.register_attachment_type("clone:defaulted", false)
                    .unwrap();
                r.register_attachment_type("clone:saved", false).unwrap();
                r.register_attachment_type("clone:retained", false).unwrap();
            }

            fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                let aborted: Result<(), ()> = host.transaction(|host, _| {
                    VALUE.set(host, Holder::Player(event.player), 555).unwrap();
                    event.original.set_attachment(&VALUE, 444).unwrap();
                    assert_eq!(event.original.remove_attachment(&RETAINED), Ok(Some(17)));
                    assert_eq!(event.original.attachment(&DEFAULTED), Ok(41));
                    assert_eq!(event.original.existing_attachment(&SAVED), Ok(Some(31)));
                    // Inner committed transaction must still roll back when outer aborts.
                    host.transaction(|_, _| {
                        event.original.set_attachment(&VALUE, 445).unwrap();
                        Ok::<(), ()>(())
                    })
                    .unwrap();
                    assert_eq!(event.original.existing_attachment(&VALUE), Ok(Some(445)));
                    assert_eq!(VALUE.get(host, Holder::Player(event.player)), Ok(555));
                    Err(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(VALUE.has(host, Holder::Player(event.player)), Ok(false));
                assert_eq!(event.original.existing_attachment(&VALUE), Ok(Some(73)));
                assert_eq!(event.original.has_attachment(&RETAINED), Ok(true));
                assert_eq!(event.original.has_attachment(&DEFAULTED), Ok(false));
                assert_eq!(
                    event
                        .original
                        .with_decoded_attachment(&SAVED, |value| value.copied()),
                    Ok(None)
                );
                *self.heard.lock().unwrap() += 1;
            }
        }

        struct CleanObserver {
            heard: Arc<Mutex<usize>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for CleanObserver {
            fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
                assert_eq!(VALUE.has(host, Holder::Player(event.player)), Ok(false));
                assert_eq!(RETAINED.get(host, Holder::Player(event.player)), Ok(17));
                assert_eq!(event.original.existing_attachment(&VALUE), Ok(Some(73)));
                assert_eq!(event.original.has_attachment(&RETAINED), Ok(true));
                assert_eq!(event.original.has_attachment(&DEFAULTED), Ok(false));
                assert_eq!(
                    event
                        .original
                        .with_decoded_attachment(&SAVED, |value| value.copied()),
                    Ok(None)
                );
                *self.heard.lock().unwrap() += 1;
            }
        }

        crate::game::native_entity_refs::tests::run_with_game_stack(|| {
            let writer_heard = Arc::new(Mutex::new(0));
            let observer_heard = Arc::new(Mutex::new(0));
            let mods = vec![
                mod_api::ModEntry::new(
                    "clone",
                    Box::new(AbortWriter {
                        heard: writer_heard.clone(),
                    }),
                ),
                mod_api::ModEntry::new(
                    "observer",
                    Box::new(CleanObserver {
                        heard: observer_heard.clone(),
                    }),
                ),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let host = game.lan.as_mut().unwrap();
                    host.hub.guests.update(0, |guest| {
                        guest.dead = true;
                        guest.health = 0.0;
                        guest.spawn = Some([3.5, 64.0, 5.5]);
                    });
                    let attachments = host.hub.guests.attachments_mut(12).unwrap();
                    RETAINED.set(attachments, Holder::Level, 17).unwrap();
                    SAVED.set(attachments, Holder::Level, 31).unwrap();
                    let saved = Attachments::from_nbt(attachments.to_nbt());
                    attachments.replace_contents(saved);
                    VALUE.set(attachments, Holder::Level, 73).unwrap();
                    game.guest_respawn(12);
                    assert_eq!(*writer_heard.lock().unwrap(), 1);
                    assert_eq!(*observer_heard.lock().unwrap(), 1);
                });
            });
        });
    }
}
