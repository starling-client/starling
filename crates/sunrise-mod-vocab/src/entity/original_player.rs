//! Direct hook-scoped loan of the retained original player during `player_clone`.

use crate::{
    ActiveEffect, AttachmentError, AttachmentKey, AttachmentStore, Compound, Holder, Identifier,
    InventoryView, LevelRef, PlayerExperience,
};

/// Direct callback-scoped loan of the retained original player during
/// [`Hooks::player_clone`](crate::Hooks::player_clone) (NeoForge's
/// `PlayerEvent.Clone.getOriginal`).
///
/// Reads expose the old player's identity, position, dimension, level, health,
/// experience, active effects, 43-slot inventory and persistent data. Attachment
/// methods operate on the retained original holder with normal attachment-holder
/// semantics (including default insertion, decode-once caching, writes and
/// removals) without touching the replacement player, and participate in the
/// enclosing `host.transaction` so an aborted transaction rolls back both holders.
///
/// The loan is bound to the callback lifetime and cannot escape into a `'static`
/// slot:
///
/// ```compile_fail
/// use mod_api::{OriginalPlayer, PlayerCloneEvent};
///
/// fn escape(event: PlayerCloneEvent<'_>, slot: &mut Option<OriginalPlayer<'static>>) {
///     *slot = Some(event.original);
/// }
/// ```
// exhaustive: only the engine lends the retained original player.
// no wire: hook-scoped loan of the retained original player, not a payload.
pub struct OriginalPlayer<'a> {
    data: &'a mut (dyn OriginalPlayerAccess + 'static),
}

impl<'a> OriginalPlayer<'a> {
    // hidden: the engine lends the retained original player for a single player_clone dispatch.
    #[doc(hidden)]
    pub fn new(data: &'a mut (dyn OriginalPlayerAccess + 'static)) -> Self {
        Self { data }
    }

    /// The original player's UUID.
    pub fn uuid(&self) -> u128 {
        self.data.uuid()
    }

    /// The original player's position before the data copy.
    pub fn position(&self) -> [f64; 3] {
        self.data.position()
    }

    /// The original player's dimension identifier before the data copy.
    pub fn dimension(&self) -> Option<Identifier> {
        Identifier::parse(self.data.dimension()).ok()
    }

    /// The original player's level before the data copy, if available.
    pub fn level(&self) -> Option<LevelRef> {
        self.data.level()
    }

    /// The original player's health at death or End return.
    pub fn health(&self) -> f32 {
        self.data.health()
    }

    /// The original player's experience at death or End return.
    pub fn experience(&self) -> PlayerExperience {
        self.data.experience()
    }

    /// The original player's active effects at death or End return.
    pub fn effects(&self) -> &[ActiveEffect] {
        self.data.effects()
    }

    /// Read the original player's inventory by vanilla `Inventory.getItem` numbers:
    /// 0 to 35 main (0 to 8 hotbar), 36 feet, 37 legs, 38 chest, 39 head, 40 offhand,
    /// 41 body, 42 saddle. The view is read-only and valid only in `read`. Use
    /// [`InventoryView::get`] for one slot or [`InventoryView::iter`] for nonempty slots
    /// in ascending order. Read scalars in `read`; use
    /// [`ItemStackView::project`](crate::ItemStackView::project) to get components or
    /// retain an owned stack.
    pub fn with_inventory<R>(&self, read: impl for<'b> FnOnce(InventoryView<'b>) -> R) -> R {
        let mut read = Some(read);
        let mut result = None;
        self.data.with_inventory(&mut |view| {
            if let Some(read) = read.take() {
                result = Some(read(view));
            }
        });
        result.expect("original player inventory view must invoke callback")
    }

    /// The original player's persistent data compound (`NeoForgeData` / `ForgeData`),
    /// if present.
    pub fn persistent_data(&self) -> Option<&Compound> {
        self.data.persistent_data()
    }

    /// Lend an attachment on the original player, using its default without inserting it
    /// if absent. A saved entry decodes and caches once; that cache write rolls back if an
    /// enclosing `host.transaction` aborts. Never touches the replacement player.
    pub fn attachment<T: Send + Sync + 'static, R>(
        &mut self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentError> {
        let key = key.live()?;
        key.get_existing(
            self.data.attachments(),
            Holder::Level,
            |value| match value {
                Some(value) => read(value),
                None => read(&key.default_value()),
            },
        )
    }

    /// Whether the original player carries an entry for `key`, saved or decoded,
    /// without inserting a default.
    pub fn has_attachment<T: Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
    ) -> Result<bool, AttachmentError> {
        key.live()?.has(self.data.attachments(), Holder::Level)
    }

    /// Read an attachment on the original player only if present; a saved entry decodes
    /// and caches once without inserting a default, rolling back if an enclosing
    /// `host.transaction` aborts.
    pub fn existing_attachment<T: Send + Sync + 'static, R>(
        &mut self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        key.live()?
            .get_existing(self.data.attachments(), Holder::Level, read)
    }

    /// Borrow an already-decoded attachment on the original player without cloning it.
    /// A raw saved entry is not decoded and no default is inserted.
    pub fn with_decoded_attachment<T: Send + Sync + 'static, R>(
        &mut self,
        key: &AttachmentKey<T>,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        key.live()?
            .with_decoded(self.data.attachments(), Holder::Level, read)
    }

    /// Set an attachment value on the retained original player. Never touches the
    /// replacement player, and rolls back if an enclosing `host.transaction` aborts.
    pub fn set_attachment<T: Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
        value: T,
    ) -> Result<(), AttachmentError> {
        key.live()?
            .set(self.data.attachments(), Holder::Level, value)
    }

    /// Remove an attachment from the retained original player, returning its previous
    /// value if present. Never touches the replacement player, and rolls back if an
    /// enclosing `host.transaction` aborts.
    pub fn remove_attachment<T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
    ) -> Result<Option<T>, AttachmentError> {
        key.live()?.remove(self.data.attachments(), Holder::Level)
    }
}

/// Engine backing for the retained original player lent to `player_clone`.
// hidden: the engine owns the retained original player; listeners use the checked OriginalPlayer methods.
#[doc(hidden)]
pub trait OriginalPlayerAccess {
    fn uuid(&self) -> u128;
    fn position(&self) -> [f64; 3];
    fn dimension(&self) -> &str;
    fn level(&self) -> Option<LevelRef>;
    fn health(&self) -> f32;
    fn experience(&self) -> PlayerExperience;
    fn effects(&self) -> &[ActiveEffect];
    fn with_inventory(&self, read: &mut dyn for<'a> FnMut(InventoryView<'a>));
    fn persistent_data(&self) -> Option<&Compound>;
    fn attachments(&mut self) -> &mut dyn AttachmentStore;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttachmentCodec, Attachments, ItemKey, ItemStack, Tag};
    use std::num::NonZeroU32;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StubOriginal {
        uuid: u128,
        position: [f64; 3],
        dimension: String,
        level: Option<LevelRef>,
        health: f32,
        experience: PlayerExperience,
        effects: Vec<ActiveEffect>,
        persistent_data: Option<Compound>,
        attachments: Attachments,
    }

    impl OriginalPlayerAccess for StubOriginal {
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
            let metadata = |slot| match slot {
                0 => Some((ItemKey(4), NonZeroU32::new(3).unwrap(), None)),
                40 => Some((ItemKey(9), NonZeroU32::MIN, Some(2))),
                _ => None,
            };
            let project =
                |_slot, (item, count, damage)| Some(ItemStack::from_scalars(item, count, damage));
            read(InventoryView::new(0..43, &metadata, &project));
        }
        fn persistent_data(&self) -> Option<&Compound> {
            self.persistent_data.as_ref()
        }
        fn attachments(&mut self) -> &mut dyn AttachmentStore {
            &mut self.attachments
        }
    }

    static DECODES: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn original_player_lends_state_inventory_and_normal_attachments() {
        let (counter_record, counter) = crate::parts::registered_attachment_fixture(
            "demo",
            crate::AttachmentDescriptor::new("demo:counter", || 41i32),
        );
        let (saved_record, saved) = crate::parts::registered_attachment_fixture(
            "demo",
            crate::AttachmentDescriptor::new("demo:saved", || 0i32).serialize(AttachmentCodec {
                encode: |value| {
                    let mut tag = Compound::new();
                    tag.insert("n".into(), Tag::Int(*value));
                    Ok(tag)
                },
                decode: |tag| {
                    DECODES.fetch_add(1, Ordering::SeqCst);
                    match tag.get(&"n".into()) {
                        Some(Tag::Int(n)) => Ok(*n),
                        _ => Err(AttachmentError::Decode),
                    }
                },
            }),
        );
        crate::parts::install_input(crate::parts::installation::InstalledInput {
            attachment_types: std::collections::HashMap::from([(
                "demo",
                std::collections::HashMap::from([
                    ("counter", counter_record),
                    ("saved", saved_record),
                ]),
            )]),
            ..Default::default()
        });
        DECODES.store(0, Ordering::SeqCst);
        let mut seed = Attachments::default();
        saved
            .live()
            .unwrap()
            .set(&mut seed, Holder::Level, 77)
            .unwrap();
        let attachments = Attachments::from_nbt(seed.try_to_nbt().unwrap());

        let mut persistent = Compound::new();
        persistent.insert("tag".into(), Tag::Int(12));

        let mut stub = StubOriginal {
            uuid: 99,
            position: [1.0, 64.0, -2.5],
            dimension: "minecraft:overworld".into(),
            level: None,
            health: 14.5,
            experience: PlayerExperience {
                level: 5,
                progress: 0.25,
                total: 55,
            },
            effects: Vec::new(),
            persistent_data: Some(persistent),
            attachments,
        };

        let mut original = OriginalPlayer::new(&mut stub);

        assert_eq!(original.uuid(), 99);
        assert_eq!(original.position(), [1.0, 64.0, -2.5]);
        assert_eq!(
            original.dimension(),
            Some(Identifier::parse("minecraft:overworld").unwrap())
        );
        assert_eq!(original.level(), None);
        assert_eq!(original.health(), 14.5);
        assert_eq!(
            original.experience(),
            PlayerExperience {
                level: 5,
                progress: 0.25,
                total: 55,
            }
        );
        assert!(original.effects().is_empty());
        assert_eq!(
            original
                .persistent_data()
                .and_then(|data| data.get(&"tag".into())),
            Some(&Tag::Int(12))
        );

        let (hotbar_count, offhand_damage) = original.with_inventory(|view| {
            let hotbar = view.get(0).expect("slot 0 present");
            let offhand = view.get(40).expect("offhand present");
            assert!(view.get(1).is_none());
            (hotbar.count(), offhand.damage_component())
        });
        assert_eq!(hotbar_count, 3);
        assert_eq!(offhand_damage, Some(2));

        // Reading a default leaves the original attachment absent.
        assert_eq!(original.attachment(&counter, Clone::clone), Ok(41));
        assert_eq!(original.has_attachment(&counter), Ok(false));
        assert_eq!(
            original.existing_attachment(&counter, |value| value.cloned()),
            Ok(None)
        );
        assert_eq!(
            original.with_decoded_attachment(&counter, |value| value.copied()),
            Ok(None)
        );

        // Saved entry decodes once on existing_attachment and caches for subsequent reads.
        assert_eq!(original.has_attachment(&saved), Ok(true));
        assert_eq!(
            original.with_decoded_attachment(&saved, |value| value.copied()),
            Ok(None)
        );
        assert_eq!(DECODES.load(Ordering::SeqCst), 0);
        assert_eq!(
            original.existing_attachment(&saved, |value| value.cloned()),
            Ok(Some(77))
        );
        assert_eq!(DECODES.load(Ordering::SeqCst), 1);
        assert_eq!(
            original.existing_attachment(&saved, |value| value.cloned()),
            Ok(Some(77))
        );
        assert_eq!(DECODES.load(Ordering::SeqCst), 1);
        assert_eq!(
            original.with_decoded_attachment(&saved, |value| value.copied()),
            Ok(Some(77))
        );

        // Writes and removals work normally on the retained original holder.
        assert_eq!(original.set_attachment(&counter, 99), Ok(()));
        assert_eq!(
            original.existing_attachment(&counter, |value| value.cloned()),
            Ok(Some(99))
        );
        assert_eq!(original.remove_attachment(&counter), Ok(Some(99)));
        assert_eq!(original.has_attachment(&counter), Ok(false));
        assert_eq!(original.remove_attachment(&counter), Ok(None));

        // Covariance check: &'1 str and OriginalPlayer<'2> unify into one lifetime 'a.
        fn unify<'a>(_name: &'a str, _original: OriginalPlayer<'a>) {}
        let name = String::from("alex");
        unify(&name, original);
    }
}
