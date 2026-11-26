//! The callback's typed store: only encoded values cross into background encoding.
use super::*;
use std::sync::Arc;
use sunrise_core::nbt::{self, Limits, NamedTag};
use sunrise_mod_runtime::attachments::{ATTACHMENTS_KEY, CheckedAttachmentType};

/// A job-local attachment transaction. No decoded value is shared with live state.
/// Use [`Self::get`], [`Self::set`] and [`Self::remove`] with an issued [`AttachmentKey`].
/// Disk operations consume only the key's persistence codec; any refusal returns locally
/// without modifying the stored envelopes or poisoning unrelated edits.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ChunkDiskData {
    entries: Arc<Compound>,
}

impl ChunkDiskData {
    /// Open a transaction over the captured envelopes. The ordinary chunk writer owns NBT encoding.
    pub(crate) fn from_nbt(entries: Compound) -> Self {
        Self {
            entries: Arc::new(entries),
        }
    }

    /// Read the job's typed value through its persistence codec, inserting its default if absent.
    pub fn get<T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
    ) -> Result<T, ChunkDiskFailure> {
        self.edit(key, |checked, store| {
            checked.get(store, Holder::Level, T::clone)
        })
    }

    /// Replace only the snapshot's value through its persistence codec. A codec or saved-NBT
    /// refusal leaves the prior envelopes intact and does not poison other edits.
    pub fn set<T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
        value: T,
    ) -> Result<(), ChunkDiskFailure> {
        self.edit(key, |checked, store| {
            checked.set(store, Holder::Level, value)
        })
    }

    /// Remove a snapshot binding through its persistence codec.
    pub fn remove<T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &AttachmentKey<T>,
    ) -> Result<Option<T>, ChunkDiskFailure> {
        self.edit(key, |checked, store| checked.remove(store, Holder::Level))
    }

    fn edit<T: 'static, R>(
        &mut self,
        key: &AttachmentKey<T>,
        apply: impl FnOnce(&CheckedAttachmentType<T>, &mut Attachments) -> Result<R, AttachmentError>,
    ) -> Result<R, ChunkDiskFailure> {
        let key_name = key.id();
        let refused = |error| failure(key_name, ChunkDiskFailureKind::Attachment(error));
        let checked = key.live().map_err(refused)?;
        if !checked.has_codec() {
            return Err(failure(
                key_name,
                ChunkDiskFailureKind::MissingPersistenceCodec,
            ));
        }
        let disk = checked.disk_only();
        let mut store = Attachments::from_nbt((*self.entries).clone());
        let value = apply(&disk, &mut store).map_err(refused)?;
        let encoded = store.to_staged_nbt();
        // Mount the envelopes at their real depth in the saved record. The region writer's
        // write guard is not the reader's depth limit (vanilla NbtIo.write/read).
        let mut root = NamedTag {
            name: "".into(),
            tag: Tag::Compound([(ATTACHMENTS_KEY.into(), Tag::Compound(encoded))].into()),
        };
        let limits = Limits::saved();
        let write = Limits {
            max_depth: nbt::DISK_WRITE_NESTING_GUARD.max(limits.max_depth),
            ..limits
        };
        if nbt::write_named(&root, write)
            .and_then(|bytes| nbt::read_named(&bytes, limits).map(|_| ()))
            .is_err()
        {
            return Err(failure(key_name, ChunkDiskFailureKind::InvalidSavedNbt));
        }
        let Tag::Compound(root) = &mut root.tag else {
            unreachable!()
        };
        let Some(Tag::Compound(encoded)) = root.remove(&ATTACHMENTS_KEY.into()) else {
            unreachable!()
        };
        self.entries = Arc::new(encoded);
        Ok(value)
    }

    /// Run a nested savepoint. An explicit abort restores the prior envelopes.
    pub fn transaction<R, E>(
        &mut self,
        apply: impl FnOnce(&mut Self) -> Result<R, E>,
    ) -> Result<R, E> {
        let previous = self.entries.clone();
        let result = apply(self);
        if result.is_err() {
            self.entries = previous;
        }
        result
    }

    /// Publish the accepted edits from this snapshot.
    pub fn finish(self) -> Compound {
        Arc::unwrap_or_clone(self.entries)
    }
}

fn failure(key: &str, reason: ChunkDiskFailureKind) -> ChunkDiskFailure {
    let mod_id = crate::content::crash::current_native_mod().unwrap_or_default();
    if !mod_id.is_empty() {
        log::warn!("mod {mod_id}: chunk attachment {key} operation refused: {reason:?}");
    }
    ChunkDiskFailure {
        mod_id: mod_id.into(),
        key: key.into(),
        reason,
    }
}
