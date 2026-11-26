//! Typed native-mod data attached to server-owned holders.
#[doc(hidden)]
pub mod index;
#[doc(hidden)]
mod index_key;
mod lifetime;
#[doc(hidden)]
pub mod observer;
mod query;
mod resident;
#[doc(hidden)]
pub mod snapshot;

pub use query::{
    AttachmentBatch, AttachmentIter, AttachmentQuery, AttachmentRowError, AttachmentRowRef,
};
#[doc(hidden)]
pub use query::{AttachmentQueryScope, AttachmentQueryScopes};

#[doc(hidden)]
pub use lifetime::{AttachmentLifetime, HasAttachmentLifetime};
#[doc(hidden)]
pub use resident::ResidentAttachments;

use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    fmt,
    hash::Hash,
    sync::{Arc, LazyLock, Mutex, OnceLock, Weak},
};

use sunrise_core::nbt::{Compound, JavaString, Tag};

// A borrowed comparison view preserves Java's UTF-16 ordering without building keys.
trait AttachmentKeyView {
    fn namespace(&self) -> Utf16Units<'_>;
    fn name(&self) -> Utf16Units<'_>;
}
enum Utf16Units<'a> {
    Owned(std::iter::Copied<std::slice::Iter<'a, u16>>),
    Borrowed(std::str::EncodeUtf16<'a>),
}
impl Utf16Units<'_> {
    fn cmp(self, other: Self) -> std::cmp::Ordering {
        // Select the concrete iterators once, not for every UTF-16 unit.
        match (self, other) {
            (Self::Owned(left), Self::Owned(right)) => left.cmp(right),
            (Self::Owned(left), Self::Borrowed(right)) => left.cmp(right),
            (Self::Borrowed(left), Self::Owned(right)) => left.cmp(right),
            (Self::Borrowed(left), Self::Borrowed(right)) => left.cmp(right),
        }
    }
}
impl AttachmentKeyView for (JavaString, JavaString) {
    fn namespace(&self) -> Utf16Units<'_> {
        Utf16Units::Owned(self.0.0.iter().copied())
    }
    fn name(&self) -> Utf16Units<'_> {
        Utf16Units::Owned(self.1.0.iter().copied())
    }
}
impl AttachmentKeyView for (&str, &str) {
    fn namespace(&self) -> Utf16Units<'_> {
        Utf16Units::Borrowed(self.0.encode_utf16())
    }
    fn name(&self) -> Utf16Units<'_> {
        Utf16Units::Borrowed(self.1.encode_utf16())
    }
}
impl PartialEq for dyn AttachmentKeyView + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for dyn AttachmentKeyView + '_ {}
impl PartialOrd for dyn AttachmentKeyView + '_ {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for dyn AttachmentKeyView + '_ {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.namespace()
            .cmp(other.namespace())
            .then_with(|| self.name().cmp(other.name()))
    }
}
impl<'a> std::borrow::Borrow<dyn AttachmentKeyView + 'a> for (JavaString, JavaString) {
    fn borrow(&self) -> &(dyn AttachmentKeyView + 'a) {
        self
    }
}

// no wire: reports a local attachment operation's failure, not stored attachment data.
/// An attachment operation could not be completed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttachmentError {
    /// The holder does not exist or is unavailable on this side.
    MissingHolder,
    /// The key was used with a different Rust type.
    TypeMismatch,
    /// The registrar returned a registration for a different attachment ID.
    RegistrationMismatch,
    /// The saved value could not be decoded.
    Decode,
    /// The value could not be encoded.
    Encode,
    /// A stock stack codec could not encode this occupied index or its stack.
    StackEncoding {
        /// The index the codec could not encode.
        index: usize,
    },
    /// A namespace in the saved compound is not a compound.
    InvalidNamespace,
    /// Copy-on-death requires an already configured serializer.
    InvalidCopyPolicy,
    /// A persistence serializer has already been configured.
    DuplicateSerializer,
    /// Attachment sync policy does not match its load-time declaration.
    InvalidSyncPolicy,
    /// The holder's physical storage is temporarily extracted or lifted.
    /// Retry after the current pass; the refused store writes no value.
    HolderBusy,
    /// The holder is only lent for reading during this hook (a hit's victim or causing
    /// entity): `has` and `get_existing` answer, while a `get` that would store its default,
    /// `set`, `remove` of a held value and `sync_data` of a synced type refuse before any of
    /// the type's callbacks run and write nothing. A `remove` of an absent value and a
    /// `sync_data` of an unsynced type stay the no-ops they are on any holder.
    ReadOnlyHolder,
    /// This physical holder family has no attachment lifetime storage; retrying cannot help.
    /// Stable lowercase family names: mob, item, orb, projectile, armorstand,
    /// itemframe, painting, boat, minecart, other, fangs, crystal, dragon, tnt,
    /// falling, rocket, trident, spawner, cloud, cushion, knot, player, hook,
    /// dragonpart.
    UnsupportedHolder(&'static str),
    /// No loaded package declared this attachment id.
    Undeclared {
        package_id: String,
        attachment_id: String,
    },
    /// This attachment key belongs to a previous installation and is no longer live.
    StaleKey,
}

/// Encoding a persistent attachment for disk failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentEncodeError {
    pub namespace: String,
    pub name: String,
    pub error: AttachmentError,
}

impl fmt::Display for AttachmentEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "attachment '{}:{}' encode failed ({:?})",
            self.namespace, self.name, self.error
        )
    }
}

impl std::error::Error for AttachmentEncodeError {}

/// NBT key reserved for server-owned attachment maps.
pub const ATTACHMENTS_KEY: &str = "sunrise:attachments";

/// Read the reserved compound without replacing an invalid saved value.
pub fn attachment_compound(tag: Option<&Tag>) -> Result<Option<&Compound>, AttachmentError> {
    match tag {
        None => Ok(None),
        Some(Tag::Compound(compound)) => Ok(Some(compound)),
        Some(_) => Err(AttachmentError::InvalidNamespace),
    }
}

/// A server holder in the host's current dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: addresses storage relative to the current host.
pub enum Holder {
    /// A non-player entity ID in the host's current dimension.
    ///
    /// An entity keeps its UUID through saves so saved attachments reload with it. A saved
    /// experience orb with `count > 1` loads as one entity with its merged count, and its saved
    /// attachments bind to that entity.
    Entity(u32),
    Player(u128),
    BlockEntity([i32; 3]),
    Chunk([i32; 2]),
    Level,
}

/// Whether borrowing a map may create publication authority for a new value.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: host-local storage access intent, never a cross-target payload.
pub enum AttachmentAccess {
    /// Inspection, removal or sync: never create a registration.
    Existing,
    /// A validated value is about to be stored, including an inserted default.
    Store,
}

/// The game's access to each holder's map: every [`AttachmentStore`] answers through it.
pub trait AttachmentHost {
    /// Validate the load-owned declaration before accessing storage.
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), AttachmentError>;
    /// Validate the descriptor that issued an erased store request.
    fn validate_attachment_key(&self, key: &AttachmentStoreKey<'_>) -> Result<(), AttachmentError> {
        let (namespace, name) = key.id();
        self.validate_attachment(namespace, name, key.policy.sync)
    }
    /// Inspect an existing holder without materializing attachment storage.
    fn read_attachments(
        &mut self,
        holder: Holder,
    ) -> Result<Option<&Attachments>, AttachmentError> {
        self.attachments(holder, AttachmentAccess::Existing)
            .map(|map| Some(&*map))
    }
    /// Record side effects of an accepted write or sync, after the map loan ends.
    fn attachment_changed(&mut self, _holder: Holder, _persist: bool, _sync: bool) {}
    fn attachments(
        &mut self,
        holder: Holder,
        access: AttachmentAccess,
    ) -> Result<&mut Attachments, AttachmentError>;
    /// `holder`'s map when this host only lends it for reading: `Some(Ok(None))` for such a
    /// holder that keeps no map. Reads answer from it without decoding into it, and every
    /// write refuses with [`AttachmentError::ReadOnlyHolder`]. None, the default, for a holder
    /// [`Self::attachments`] answers.
    fn read_only_attachments(
        &self,
        _holder: Holder,
    ) -> Option<Result<Option<&Attachments>, AttachmentError>> {
        None
    }
}

/// A value of a mod's own type, as the holder keeps it: shared, so a copy of the holder's map
/// copies no value.
// no wire: host-local erased Arc value; attachment values cross through their registered codec.
type AttachmentValue = Arc<dyn Any + Send + Sync>;

type PersistenceEncoder =
    Arc<dyn Fn(&(dyn Any + Send + Sync)) -> Result<Compound, AttachmentError> + Send + Sync>;

fn persistence_encoder<T: 'static>(
    codec: AttachmentCodec<T>,
    invoke: impl Fn(&dyn Fn() -> Result<Compound, AttachmentError>) -> Result<Compound, AttachmentError>
    + Send
    + Sync
    + 'static,
) -> PersistenceEncoder {
    Arc::new(move |value| {
        let value = value
            .downcast_ref::<T>()
            .ok_or(AttachmentError::TypeMismatch)?;
        invoke(&|| (codec.encode)(value))
    })
}

/// How a policy-checked type declared its value: what saving, respawning and syncing do with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// no wire: local store metadata; disk and sync records carry their own flags.
pub struct AttachmentPolicy {
    pub copy_on_death: bool,
    pub sync: bool,
}

/// The installed clone policy; an absent registration is distinct from a memory-only one.
#[derive(Clone, Copy)]
// no wire: a runtime lookup result, not a field of the saved attachment envelope.
pub enum CloneAttachmentPolicy {
    MemoryOnly,
    Persistent { copy_on_death: bool },
}

/// What comes with a stored value: its optional persistence encoding, any original sync
/// bytes retained when a client replica is decoded, its type's policy, and why it is stored.
#[derive(Clone, Debug, PartialEq)]
// no wire: describes a local store operation; the encoded compound is the saved value.
struct AttachmentForm {
    pub encoded: Option<Compound>,
    pub synced: Option<Compound>,
    pub policy: AttachmentPolicy,
    pub write: AttachmentWrite,
}

/// Why a value is stored, which decides what else changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: tells the local store how the value arrived.
enum AttachmentWrite {
    /// A saved value just decoded: nothing else changes.
    Decoded,
    /// The type's default, inserted by a read: clients hear of it if the type syncs.
    Default,
    /// A value a mod set: clients hear of it if the type syncs, and the saved form it replaces
    /// is dropped.
    Set,
}

/// An unread disk or replica binding retained by a custom store. Pass the holder's actual
/// binding through unchanged: the destination checks its identity before promoting it.
// no wire: identity of an in-process binding, not a serialized attachment.
pub struct AttachmentBinding(Arc<AttachmentEntry>);

impl PartialEq for AttachmentBinding {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for AttachmentBinding {}

/// A borrowed proof of the descriptor that prepared an erased store operation.
#[doc(hidden)]
// no wire: borrows the in-process descriptor behind a typed attachment operation.
pub struct AttachmentStoreKey<'a> {
    id: (&'static str, &'static str),
    descriptor: &'a (dyn Any + Send + Sync),
    engine_owned: bool,
    policy: AttachmentPolicy,
    sync_release: Option<&'a SyncSnapshotRelease>,
}

type SyncSnapshotRelease = Arc<dyn Fn(AttachmentEntry) + Send + Sync>;

impl AttachmentStoreKey<'_> {
    pub fn id(&self) -> (&'static str, &'static str) {
        self.id
    }

    pub fn engine_owned(&self) -> bool {
        self.engine_owned
    }

    pub fn matches_descriptor<T: Send + Sync + 'static>(
        &self,
        descriptor: &AttachmentType<T>,
    ) -> bool {
        self.descriptor
            .downcast_ref::<AttachmentType<T>>()
            .is_some_and(|actual| std::ptr::eq(actual, descriptor))
    }
}

/// A write already prepared by a typed descriptor. Only the store consumes its value.
#[doc(hidden)]
// no wire: retains a process-local write proof and value.
pub struct AttachmentStoreWrite<'descriptor> {
    key: AttachmentStoreKey<'descriptor>,
    value: AttachmentValue,
    persistence: Option<PersistenceEncoder>,
    form: AttachmentForm,
    promotion: Option<AttachmentBinding>,
}

impl AttachmentStoreWrite<'_> {
    pub fn key(&self) -> &AttachmentStoreKey<'_> {
        &self.key
    }
}

pub type AttachmentLoan<'a> = &'a mut dyn FnMut(Option<&(dyn Any + Send + Sync)>);

/// Engine storage for typed attachment key methods. Raw calls require an opaque
/// descriptor proof; decoded values are only accessible inside a loan.
#[doc(hidden)]
pub trait AttachmentStore {
    /// Whether `holder` is lent for reading only (a hit's victim or causing entity): its values
    /// read, and every write refuses with [`AttachmentError::ReadOnlyHolder`] before any of the
    /// type's callbacks run. False, the default, for a store that lends none.
    fn read_only(&mut self, _holder: Holder) -> Result<bool, AttachmentError> {
        Ok(false)
    }
    fn holds(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<bool, AttachmentError>;
    fn lend_decoded(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
        read: AttachmentLoan<'_>,
    ) -> Result<(), AttachmentError>;
    fn binding(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<Option<AttachmentBinding>, AttachmentError>;
    fn store(
        &mut self,
        holder: Holder,
        write: AttachmentStoreWrite<'_>,
    ) -> Result<(), AttachmentError>;
    fn discard(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<(), AttachmentError>;
    /// Check `key`'s declaration, then queue a sync of its current value when its type syncs;
    /// an unsynced type queues nothing.
    fn queue_sync(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<(), AttachmentError>;
}

impl<H: AttachmentHost + ?Sized> AttachmentStore for H {
    fn read_only(&mut self, holder: Holder) -> Result<bool, AttachmentError> {
        match self.read_only_attachments(holder) {
            Some(map) => map.map(|_| true),
            None => Ok(false),
        }
    }
    fn holds(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<bool, AttachmentError> {
        self.validate_attachment_key(key)?;
        let (namespace, name) = key.id();
        if let Some(map) = self.read_only_attachments(holder) {
            return Ok(map?.is_some_and(|map| map.holds(namespace, name)));
        }
        Ok(self
            .read_attachments(holder)?
            .is_some_and(|map| Attachments::holds(map, namespace, name)))
    }
    fn lend_decoded(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
        read: AttachmentLoan<'_>,
    ) -> Result<(), AttachmentError> {
        self.validate_attachment_key(key)?;
        let (namespace, name) = key.id();
        let map = match self.read_only_attachments(holder) {
            Some(map) => map?,
            None => self.read_attachments(holder)?,
        };
        let value = match map {
            Some(map) => Attachments::decoded(map, namespace, name)?.map(|value| value.as_ref()),
            None => None,
        };
        read(value);
        Ok(())
    }
    fn binding(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<Option<AttachmentBinding>, AttachmentError> {
        self.validate_attachment_key(key)?;
        let (namespace, name) = key.id();
        let map = match self.read_only_attachments(holder) {
            Some(map) => map?,
            None => self.read_attachments(holder)?,
        };
        Ok(match map {
            Some(map) => map.entry(namespace, name)?.cloned().map(AttachmentBinding),
            None => None,
        })
    }
    fn store(
        &mut self,
        holder: Holder,
        prepared: AttachmentStoreWrite<'_>,
    ) -> Result<(), AttachmentError> {
        self.validate_attachment_key(&prepared.key)?;
        if let Some(map) = self.read_only_attachments(holder) {
            map?;
            return Err(AttachmentError::ReadOnlyHolder);
        }
        let (namespace, name) = prepared.key.id();
        let map = self.attachments(holder, AttachmentAccess::Store)?;
        let entry = map.entry(namespace, name)?;
        if let Some(raw) = &prepared.promotion
            && !entry.is_some_and(|entry| Arc::ptr_eq(entry, &raw.0))
        {
            return Err(AttachmentError::Decode);
        }
        let AttachmentStoreWrite {
            key,
            value,
            persistence,
            form,
            ..
        } = prepared;
        Attachments::store(
            map,
            holder,
            &key,
            Decoded {
                value,
                persistence,
                encoded: form.encoded,
                synced: form.synced,
                policy: form.policy,
            },
            form.write,
        );
        if form.write != AttachmentWrite::Decoded {
            self.attachment_changed(holder, true, form.policy.sync);
        }
        Ok(())
    }
    fn discard(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<(), AttachmentError> {
        self.validate_attachment_key(key)?;
        let (namespace, name) = key.id();
        if let Some(map) = self.read_only_attachments(holder) {
            map?;
            return Err(AttachmentError::ReadOnlyHolder);
        }
        let Some(map) = self.read_attachments(holder)? else {
            return Ok(());
        };
        let has_value = map.entry(namespace, name)?.is_some();
        if has_value {
            Attachments::discard(
                self.attachments(holder, AttachmentAccess::Existing)?,
                holder,
                key,
            );
            self.attachment_changed(holder, true, key.policy.sync);
        } else if map.allocated && key.policy.sync {
            self.attachments(holder, AttachmentAccess::Existing)?
                .record_sync(holder, (namespace.into(), name.into()), key.sync_release);
            self.attachment_changed(holder, false, true);
        }
        Ok(())
    }
    fn queue_sync(
        &mut self,
        holder: Holder,
        key: &AttachmentStoreKey<'_>,
    ) -> Result<(), AttachmentError> {
        self.validate_attachment_key(key)?;
        if !key.policy.sync {
            return Ok(());
        }
        let (namespace, name) = key.id();
        if let Some(map) = self.read_only_attachments(holder) {
            map?;
            return Err(AttachmentError::ReadOnlyHolder);
        }
        self.attachments(holder, AttachmentAccess::Existing)?
            .record_sync(holder, (namespace.into(), name.into()), key.sync_release);
        self.attachment_changed(holder, false, true);
        Ok(())
    }
}

/// The optional on-disk or network representation of an attachment.
// no wire: host-local encoder/decoder function pointers, not cross-target values.
pub struct AttachmentCodec<T> {
    pub encode: fn(&T) -> Result<Compound, AttachmentError>,
    pub decode: fn(&Compound) -> Result<T, AttachmentError>,
}

impl<T> Clone for AttachmentCodec<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for AttachmentCodec<T> {}

/// A mod-defined attachment key and its value's behavior.
// no wire: host-local key with callback pointers; attachment calls cross by namespace/name and codec.
pub struct AttachmentType<T: 'static> {
    pub namespace: &'static str,
    pub name: &'static str,
    pub default: fn() -> T,
    /// Optional persistence codec for saving and loading on disk.
    pub codec: Option<AttachmentCodec<T>>,
    pub copy_on_death: bool,
    /// Optional sync codec for sending the value to clients that see the holder;
    /// independent of persistence `codec`.
    ///
    /// Values travel on `sunrise:attachment_sync` (play, clientbound, version 1, optional) only to
    /// connections that negotiated it; vanilla clients receive none. Queued changes (from `get`
    /// inserting a default, `set`, `remove`, or [`AttachmentType::sync_data`]) are sent at the end
    /// of the tick: an entity, player, chunk or level sends each accepted change in order, and a
    /// block entity sends each changed key once at its final value. A client that starts tracking
    /// a holder receives an initial pairing bundle of all synced values for that holder.
    ///
    /// Each message is prepared against the player's current connection epoch: inside a
    /// transaction it is published at root commit (a rolled-back edit sends nothing); outside a
    /// transaction it is published immediately, in order with the player's other packets. A
    /// message prepared before a connection reconfiguration never reaches the new connection.
    ///
    /// Payload size is bounded by the connection's frame limits (there is no separate cap). A
    /// namespace or name exceeding 256 UTF-16 code units, a value that cannot be encoded, or a
    /// message past the frame limits is not sent; the refusal is logged with the holder and error
    /// (not necessarily each attachment name) while the player stays joined. A full outgoing queue
    /// closes that player's connection.
    pub sync_codec: Option<AttachmentCodec<T>>,
}

impl<T> AttachmentType<T> {
    fn policy(&self) -> AttachmentPolicy {
        AttachmentPolicy {
            copy_on_death: self.copy_on_death,
            sync: self.sync_codec.is_some(),
        }
    }
    fn decode_saved(&self, saved: &Compound) -> Result<T, AttachmentError> {
        (self.codec.as_ref().ok_or(AttachmentError::Decode)?.decode)(saved)
    }
    fn decode_synced(&self, synced: &Compound) -> Result<T, AttachmentError> {
        (self
            .sync_codec
            .as_ref()
            .ok_or(AttachmentError::Decode)?
            .decode)(synced)
    }
    fn validate_policy(&self) -> Result<(), AttachmentError> {
        if self.copy_on_death && self.codec.is_none() {
            return Err(AttachmentError::InvalidCopyPolicy);
        }
        Ok(())
    }
}

/// An attachment type whose namespace, name, and copy policy have already been validated.
///
/// Syncing (`sync_codec`) is independent of persistence (`codec`), so a sync-only attachment
/// with no persistence codec is valid.
// no wire: host-local checked descriptor with callback pointers; attachment calls cross by id and codec.
pub struct CheckedAttachmentType<T: 'static> {
    raw: AttachmentType<T>,
    persistence: Option<PersistenceEncoder>,
    sync_release: Option<SyncSnapshotRelease>,
}

impl<T: 'static> CheckedAttachmentType<T> {
    /// Validate `raw`'s copy policy once.
    pub fn new(raw: AttachmentType<T>) -> Result<Self, AttachmentError> {
        Self::with_persistence_boundary(raw, |encode| encode())
    }

    /// Install the owner's fence for save-time serializer calls on canonical values.
    #[doc(hidden)]
    pub fn with_persistence_boundary(
        raw: AttachmentType<T>,
        invoke: impl Fn(
            &dyn Fn() -> Result<Compound, AttachmentError>,
        ) -> Result<Compound, AttachmentError>
        + Send
        + Sync
        + 'static,
    ) -> Result<Self, AttachmentError> {
        raw.validate_policy()?;
        let persistence = raw.codec.map(|codec| persistence_encoder(codec, invoke));
        Ok(Self {
            raw,
            persistence,
            sync_release: None,
        })
    }
    /// Install the owning mod's callback fence for values retired after sync.
    #[doc(hidden)]
    pub fn with_sync_release(
        mut self,
        release: impl Fn(AttachmentEntry) + Send + Sync + 'static,
    ) -> Self {
        if self.is_synced() {
            self.sync_release = Some(Arc::new(release));
        }
        self
    }
    /// Borrow the underlying unchecked descriptor.
    pub fn as_type(&self) -> &AttachmentType<T> {
        &self.raw
    }
    pub fn namespace(&self) -> &'static str {
        self.raw.namespace
    }
    pub fn name(&self) -> &'static str {
        self.raw.name
    }
    pub fn default_value(&self) -> T {
        (self.raw.default)()
    }
    pub fn has_codec(&self) -> bool {
        self.raw.codec.is_some()
    }
    pub fn is_persistent(&self) -> bool {
        self.raw.codec.is_some()
    }
    pub fn is_synced(&self) -> bool {
        self.raw.sync_codec.is_some()
    }
    pub fn policy(&self) -> AttachmentPolicy {
        self.raw.policy()
    }
    /// A persistence-only view of this descriptor that never invokes `sync_codec`.
    #[doc(hidden)]
    pub fn disk_only(&self) -> Self {
        Self {
            raw: AttachmentType {
                namespace: self.raw.namespace,
                name: self.raw.name,
                default: self.raw.default,
                codec: self.raw.codec,
                copy_on_death: self.raw.copy_on_death,
                sync_codec: None,
            },
            persistence: self.persistence.clone(),
            sync_release: None,
        }
    }
    /// Borrow a value from an immutable attachment entry without re-running policy checks.
    pub fn read<'a>(&self, entry: &'a AttachmentEntry) -> Result<&'a T, AttachmentError>
    where
        T: Send + Sync,
    {
        entry.read_validated(&self.raw)
    }
}

impl<T: Send + Sync + 'static> CheckedAttachmentType<T> {
    /// Lend a value, inserting its default if absent.
    pub fn get<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentError> {
        AttachmentOps::Checked(self).get(store, holder, read)
    }
    /// Whether a holder carries a value; never inserts the default.
    pub fn has<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<bool, AttachmentError> {
        AttachmentOps::Checked(self).has(store, holder)
    }
    /// Lend an existing value; a saved or synced value decodes once. Inserts no default.
    pub fn get_existing<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        AttachmentOps::Checked(self).get_existing(store, holder, read)
    }
    /// Compare storage revisions without exposing the erased value.
    #[doc(hidden)]
    pub fn binding_revision<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<Option<AttachmentBinding>, AttachmentError> {
        store.binding(holder, &AttachmentOps::Checked(self).key())
    }
    /// Read the decoded value through `read` without cloning it.
    pub fn with_decoded<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        AttachmentOps::Checked(self).with_decoded(store, holder, read)
    }
    /// Replace a value on an existing holder.
    pub fn set<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
        value: T,
    ) -> Result<(), AttachmentError> {
        AttachmentOps::Checked(self).set(store, holder, value)
    }
    /// Remove a value from an existing holder.
    pub fn remove<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError>
    where
        T: Clone,
    {
        AttachmentOps::Checked(self).remove(store, holder)
    }
    /// Send the current value to the clients that see the holder.
    pub fn sync_data<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<(), AttachmentError> {
        AttachmentOps::Checked(self).sync_data(store, holder)
    }
}

enum AttachmentOps<'a, T: 'static> {
    Raw(&'a AttachmentType<T>),
    Checked(&'a CheckedAttachmentType<T>),
}

impl<T: 'static> AttachmentOps<'_, T> {
    fn raw(&self) -> &AttachmentType<T> {
        match self {
            Self::Raw(raw) => raw,
            Self::Checked(checked) => &checked.raw,
        }
    }
    fn persistence(&self) -> Option<PersistenceEncoder> {
        match self {
            Self::Raw(raw) => raw
                .codec
                .map(|codec| persistence_encoder(codec, |encode| encode())),
            Self::Checked(checked) => checked.persistence.clone(),
        }
    }
    fn validate(&self) -> Result<(), AttachmentError> {
        match self {
            Self::Raw(raw) => raw.validate_policy(),
            Self::Checked(_) => Ok(()),
        }
    }
}

impl<T: Send + Sync + 'static> AttachmentOps<'_, T> {
    fn key(&self) -> AttachmentStoreKey<'_> {
        let raw = self.raw();
        AttachmentStoreKey {
            id: (raw.namespace, raw.name),
            descriptor: raw,
            engine_owned: matches!(self, Self::Raw(_)),
            policy: raw.policy(),
            sync_release: match self {
                Self::Raw(_) => None,
                Self::Checked(checked) => checked.sync_release.as_ref(),
            },
        }
    }

    fn lend_decoded<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(Option<&(dyn Any + Send + Sync)>) -> R,
    ) -> Result<R, AttachmentError> {
        let mut read = Some(read);
        let mut result = None;
        store.lend_decoded(holder, &self.key(), &mut |value| {
            if let Some(read) = read.take() {
                result = Some(read(value));
            }
        })?;
        result.ok_or(AttachmentError::Decode)
    }

    fn get<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentError> {
        let mut read = Some(read);
        if let Some(result) = self.existing(store, holder, |value| read.take().unwrap()(value))? {
            return Ok(result);
        }
        // A read-only holder refuses to store the default before the default is made.
        if store.read_only(holder)? {
            return Err(AttachmentError::ReadOnlyHolder);
        }
        let value = Arc::new((self.raw().default)());
        self.keep(store, holder, Arc::clone(&value), AttachmentWrite::Default)?;
        Ok(read.unwrap()(&value))
    }
    fn has<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<bool, AttachmentError> {
        self.validate()?;
        store.holds(holder, &self.key())
    }
    fn existing<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(&T) -> R,
    ) -> Result<Option<R>, AttachmentError> {
        self.validate()?;
        let mut read = Some(read);
        if let Some(result) =
            self.lend_decoded(store, holder, |value| -> Result<_, AttachmentError> {
                value
                    .map(|value| {
                        let value = value
                            .downcast_ref::<T>()
                            .ok_or(AttachmentError::TypeMismatch)?;
                        Ok(read.take().unwrap()(value))
                    })
                    .transpose()
            })??
        {
            return Ok(Some(result));
        }
        let Some(binding) = store.binding(holder, &self.key())? else {
            return Ok(None);
        };
        let Entry::Raw { saved, kind, .. } = &binding.0.state else {
            return Err(AttachmentError::Decode);
        };
        let raw = self.raw();
        let payload = raw_value(saved).ok_or(AttachmentError::Decode)?;
        let (value, encoded, synced) = match kind {
            AttachmentRawKind::Saved => (raw.decode_saved(payload)?, Some(payload.clone()), None),
            AttachmentRawKind::Synced => (raw.decode_synced(payload)?, None, Some(payload.clone())),
        };
        let key = self.key();
        let form = AttachmentForm {
            encoded,
            synced,
            policy: key.policy,
            write: AttachmentWrite::Decoded,
        };
        let value = Arc::new(value);
        // A read-only holder's saved value decodes for this read alone, never stored.
        if !store.read_only(holder)? {
            store.store(
                holder,
                AttachmentStoreWrite {
                    key,
                    value: value.clone(),
                    persistence: if *kind == AttachmentRawKind::Saved {
                        self.persistence()
                    } else {
                        None
                    },
                    form,
                    promotion: Some(binding),
                },
            )?;
        }
        Ok(Some(read.unwrap()(&value)))
    }
    fn get_existing<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        let mut read = Some(read);
        match self.existing(store, holder, |value| read.take().unwrap()(Some(value)))? {
            Some(result) => Ok(result),
            None => Ok(read.unwrap()(None)),
        }
    }
    fn with_decoded<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        self.validate()?;
        self.lend_decoded(store, holder, |value| {
            read(value.and_then(|v| v.downcast_ref::<T>()))
        })
    }
    fn set<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
        value: T,
    ) -> Result<(), AttachmentError> {
        self.validate()?;
        if store.read_only(holder)? {
            return Err(AttachmentError::ReadOnlyHolder);
        }
        if self.lend_decoded(store, holder, |old| old.is_some_and(|old| !old.is::<T>()))? {
            return Err(AttachmentError::TypeMismatch);
        }
        self.keep(store, holder, Arc::new(value), AttachmentWrite::Set)
    }
    fn remove<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError>
    where
        T: Clone,
    {
        let held = self.has(store, holder)?;
        if store.read_only(holder)? {
            return if held {
                Err(AttachmentError::ReadOnlyHolder)
            } else {
                Ok(None)
            };
        }
        let value = if held {
            self.existing(store, holder, T::clone)
        } else {
            Ok(None)
        };
        store.discard(holder, &self.key())?;
        value
    }
    fn sync_data<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<(), AttachmentError> {
        self.validate()?;
        store.queue_sync(holder, &self.key())
    }
    fn keep<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
        value: Arc<T>,
        write: AttachmentWrite,
    ) -> Result<(), AttachmentError> {
        let raw = self.raw();
        let encoded = raw
            .codec
            .as_ref()
            .map(|codec| (codec.encode)(&value))
            .transpose()?;
        let form = AttachmentForm {
            encoded,
            synced: None,
            policy: raw.policy(),
            write,
        };
        store.store(
            holder,
            AttachmentStoreWrite {
                key: self.key(),
                value,
                persistence: self.persistence(),
                form,
                promotion: None,
            },
        )
    }
}

/// One attachment's new state for a client; `None` means removed.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentSync {
    pub namespace: JavaString,
    pub name: JavaString,
    pub value: Option<Compound>,
}

/// What an entry holds when a sync send route resolves it against the active registered
/// attachment descriptor.
// no wire: in-process projection input borrowed during sync snapshot/change encoding.
#[doc(hidden)]
pub enum AttachmentSyncInput<'a> {
    /// An unread saved binding; `None` means its envelope is malformed.
    Saved(Option<&'a Compound>),
    /// A client replica with its original sync payload; `None` means its envelope is malformed.
    Synced(Option<&'a Compound>),
    /// A canonical live value. Its sync codec runs only when actually projected for a send.
    Decoded(&'a (dyn Any + Send + Sync)),
}

impl<T: 'static> AttachmentType<T> {
    /// Project an entry's current state into its sync payload using this type's codecs.
    /// Returns `Ok(None)` if this type is not configured to sync. Keep `decoded_owner` outside
    /// the encoder’s panic fence; temporary disk-decoded values must be dropped separately.
    #[doc(hidden)]
    pub fn project_sync(
        &self,
        input: AttachmentSyncInput<'_>,
        decoded_owner: &mut Option<T>,
    ) -> Result<Option<Compound>, AttachmentError> {
        let Some(sync_codec) = &self.sync_codec else {
            return Ok(None);
        };
        match input {
            AttachmentSyncInput::Saved(saved) => {
                let value =
                    decoded_owner.insert(self.decode_saved(saved.ok_or(AttachmentError::Decode)?)?);
                (sync_codec.encode)(value).map(Some)
            }
            AttachmentSyncInput::Synced(synced) => {
                synced.cloned().map(Some).ok_or(AttachmentError::Decode)
            }
            AttachmentSyncInput::Decoded(value) => {
                let typed = value
                    .downcast_ref::<T>()
                    .ok_or(AttachmentError::TypeMismatch)?;
                (sync_codec.encode)(typed).map(Some)
            }
        }
    }
}

impl<T: Clone + Send + Sync + 'static> AttachmentType<T> {
    /// Read a value, inserting its default if absent; see `get_existing`.
    pub fn get<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<T, AttachmentError> {
        AttachmentOps::Raw(self).get(store, holder, T::clone)
    }
    /// Whether a holder carries a value, like NeoForge's `hasData`; never inserts the default.
    pub fn has<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<bool, AttachmentError> {
        AttachmentOps::Raw(self).has(store, holder)
    }
    /// Read a value only if present, like `getExistingData`; a saved or synced value decodes once.
    /// A read-only holder decodes a saved value on each read and stores nothing.
    pub fn get_existing<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError> {
        AttachmentOps::Raw(self).existing(store, holder, T::clone)
    }
    /// Read the decoded value through `read` without cloning it: what a read of one part of a
    /// large value costs. A saved value that has not decoded yet is not decoded, so `read` sees
    /// none where `get_existing` would decode one.
    pub fn with_decoded<S: AttachmentStore + ?Sized, R>(
        &self,
        store: &mut S,
        holder: Holder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        AttachmentOps::Raw(self).with_decoded(store, holder, read)
    }
    /// Replace a value on an existing holder. A key already decoded as a different type is
    /// rejected.
    pub fn set<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
        value: T,
    ) -> Result<(), AttachmentError> {
        AttachmentOps::Raw(self).set(store, holder, value)
    }
    /// Remove a value from an existing holder. A saved value that does not decode is removed
    /// too, and its error returned.
    pub fn remove<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError> {
        AttachmentOps::Raw(self).remove(store, holder)
    }
    /// Send the current value to the clients that see the holder.
    /// Queue a sync of this attachment for `holder` at the end of the tick when `sync_codec` is
    /// set. Without one, validates the declaration and policy and returns `Ok(())` without
    /// queuing a sync.
    ///
    /// Queued updates follow the same send limits, failure outcomes, and connection-epoch rules
    /// documented on [`AttachmentType::sync_codec`]: overlong names (over 256 UTF-16 code units),
    /// unencodable values, and oversized frames are logged with the holder and error and not sent
    /// without disconnecting the player; a full outgoing queue closes the player's connection; and
    /// updates prepared before a reconfiguration never cross into the new connection.
    pub fn sync_data<S: AttachmentStore + ?Sized>(
        &self,
        store: &mut S,
        holder: Holder,
    ) -> Result<(), AttachmentError> {
        AttachmentOps::Raw(self).sync_data(store, holder)
    }
}

/// A canonical value and its registered persistence encoder, separate from the
/// prepared encoding for a disk proposal and from original client-replica bytes.
struct Decoded {
    value: AttachmentValue,
    persistence: Option<PersistenceEncoder>,
    encoded: Option<Compound>,
    synced: Option<Compound>,
    policy: AttachmentPolicy,
}

impl Decoded {
    fn encode_current(&self) -> Result<Option<Compound>, AttachmentError> {
        self.persistence
            .as_ref()
            .map(|encode| encode(self.value.as_ref()))
            .transpose()
    }
}

fn envelope(encoded: Compound, copy_on_death: bool) -> Compound {
    let mut envelope = Compound::new();
    envelope.insert("value".into(), Tag::Compound(encoded));
    if copy_on_death {
        envelope.insert("copy_on_death".into(), Tag::Byte(1));
    }
    envelope
}

fn raw_value(raw: &Compound) -> Option<&Compound> {
    static VALUE: LazyLock<JavaString> = LazyLock::new(|| "value".into());
    match raw.get(&VALUE) {
        Some(Tag::Compound(value)) => Some(value),
        _ => None,
    }
}

/// Whether a raw attachment binding holds persistence-encoded or sync-encoded NBT.
// no wire: internal tag paired with network NBT in AttachmentBatch's Wire impl.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentRawKind {
    Saved,
    Synced,
}

enum Entry {
    Raw {
        saved: Compound,
        kind: AttachmentRawKind,
        reads: ReadCache,
    },
    Decoded(Decoded),
}

/// An immutable attachment binding shared by a holder and its snapshots.
/// Query reads may cache decoded values, but never change the canonical raw/decoded state.
#[doc(hidden)]
#[non_exhaustive]
// no wire: host-local retained binding with erased decoded state, not a wire payload.
pub struct AttachmentEntry {
    state: Entry,
}

// Readers can share the raw binding across Rust types and render installations without
// confusing their codecs. Each successful decode gets a stable borrow; failures aren't cached.
#[derive(Default)]
struct ReadCache {
    value: OnceLock<(Option<u64>, AttachmentValue)>,
    next: OnceLock<Box<ReadCache>>,
}

impl AttachmentEntry {
    fn raw(saved: Compound) -> Self {
        Self {
            state: Entry::Raw {
                saved,
                kind: AttachmentRawKind::Saved,
                reads: ReadCache::default(),
            },
        }
    }

    fn raw_synced(value: Compound) -> Self {
        Self {
            state: Entry::Raw {
                saved: [("value".into(), Tag::Compound(value))].into(),
                kind: AttachmentRawKind::Synced,
                reads: ReadCache::default(),
            },
        }
    }
    /// Borrow a value for a query without inserting a default, storing a decoded value,
    /// or changing saved policies or sync state. Decoded bindings never fall back to NBT.
    pub fn read<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentType<T>,
    ) -> Result<&T, AttachmentError> {
        key.validate_policy()?;
        self.read_validated(key)
    }

    /// Borrow a value for a query using a pre-validated attachment type.
    pub fn read_checked<T: Send + Sync + 'static>(
        &self,
        key: &CheckedAttachmentType<T>,
    ) -> Result<&T, AttachmentError> {
        self.read_validated(key.as_type())
    }

    fn read_validated<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentType<T>,
    ) -> Result<&T, AttachmentError> {
        self.read_in_installation(key, None)
    }

    fn read_in_installation<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentType<T>,
        epoch: Option<u64>,
    ) -> Result<&T, AttachmentError> {
        match &self.state {
            Entry::Decoded(value) => value
                .value
                .downcast_ref()
                .ok_or(AttachmentError::TypeMismatch),
            Entry::Raw { saved, kind, reads } => {
                let mut cache = reads;
                loop {
                    let value = match cache.value.get() {
                        Some(value) => value,
                        None => {
                            let payload = raw_value(saved).ok_or(AttachmentError::Decode)?;
                            let decoded = match kind {
                                AttachmentRawKind::Saved => key.decode_saved(payload)?,
                                AttachmentRawKind::Synced => key.decode_synced(payload)?,
                            };
                            let value = Arc::new(decoded);
                            // Another reader may have initialized this slot with its type.
                            let _ = cache.value.set((epoch, value));
                            cache.value.get().expect("cache initialized")
                        }
                    };
                    if value.0 == epoch
                        && let Some(value) = value.1.downcast_ref()
                    {
                        return Ok(value);
                    }
                    cache = cache.next.get_or_init(Default::default);
                }
            }
        }
    }
    /// The client replica's original sync payload, even after a typed read.
    fn synced_value(&self) -> Option<&Compound> {
        match &self.state {
            Entry::Raw {
                saved,
                kind: AttachmentRawKind::Synced,
                ..
            } => raw_value(saved),
            Entry::Raw {
                kind: AttachmentRawKind::Saved,
                ..
            } => None,
            Entry::Decoded(value) => value.synced.as_ref(),
        }
    }
    fn sync_input(&self) -> AttachmentSyncInput<'_> {
        match &self.state {
            Entry::Raw {
                saved,
                kind: AttachmentRawKind::Saved,
                ..
            } => AttachmentSyncInput::Saved(raw_value(saved)),
            Entry::Raw {
                saved,
                kind: AttachmentRawKind::Synced,
                ..
            } => AttachmentSyncInput::Synced(raw_value(saved)),
            Entry::Decoded(decoded) => match &decoded.synced {
                Some(synced) => AttachmentSyncInput::Synced(Some(synced)),
                None => AttachmentSyncInput::Decoded(decoded.value.as_ref()),
            },
        }
    }
}

struct AttachmentChange<'a> {
    key: &'a index::AttachmentKey,
    entry: Option<&'a Arc<AttachmentEntry>>,
}

trait AttachmentPublisher: Send + Sync {
    fn publish(&self, changes: &[AttachmentChange<'_>]);
    fn unregister(&self) -> bool;
}

struct IndexedAttachments<H> {
    index: Weak<Mutex<index::AttachmentIndex<H>>>,
    owner: index::AttachmentOwner<H>,
}

impl<H: Copy + Eq + Hash + Send + Sync> AttachmentPublisher for IndexedAttachments<H> {
    fn publish(&self, changes: &[AttachmentChange<'_>]) {
        if let Some(index) = self.index.upgrade() {
            let retired = {
                let mut index = index.lock().unwrap();
                let mut retired = index::AttachmentChange::new(());
                for change in changes {
                    retired.merge(match change.entry {
                        Some(entry) => index.set(&self.owner, change.key, entry.clone()),
                        None => index.remove(&self.owner, change.key),
                    });
                }
                retired
            };
            retired.into_result();
        }
    }

    fn unregister(&self) -> bool {
        self.index.upgrade().is_some_and(|index| {
            let change = {
                let mut index = index.lock().unwrap();
                index.unregister(&self.owner)
            };
            change.into_result()
        })
    }
}

/// Shared authority for one indexed map registration. Cloning or dropping the handle
/// has no effect; authoritative lifetime owners must explicitly unregister it.
#[doc(hidden)]
#[derive(Clone)]
#[non_exhaustive]
pub struct AttachmentRegistration {
    publisher: Arc<dyn AttachmentPublisher>,
}

impl AttachmentRegistration {
    /// Retire only this registration. A stale handle cannot unload its replacement,
    /// and subsequent writes through the retired map cannot restore membership.
    pub fn unregister(&self) -> bool {
        self.publisher.unregister()
    }

    /// Whether two handles refer to the same registration.
    pub fn same_registration(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.publisher, &other.publisher)
    }
}

// Tokens order committed batches from different holders in the same level. An
// aborted edit drops its token with the journaled map; gaps are unobservable.
static NEXT_SYNC_ORDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Exclusive watermark for a whole snapshot: earlier events are already covered.
#[doc(hidden)]
pub fn attachment_sync_watermark() -> u64 {
    NEXT_SYNC_ORDER.load(std::sync::atomic::Ordering::Relaxed)
}

/// One accepted non-block-entity update and the binding it wrote.
#[doc(hidden)]
#[derive(Clone)]
pub struct OrderedAttachmentSync {
    order: u64,
    key: index::AttachmentKey,
    entry: Option<Arc<AttachmentEntry>>,
    release: Option<SyncSnapshotRelease>,
}

impl Drop for OrderedAttachmentSync {
    fn drop(&mut self) {
        if let Some(release) = &self.release
            && let Some(entry) = self.entry.take().and_then(Arc::into_inner)
        {
            release(entry);
        }
    }
}

impl OrderedAttachmentSync {
    pub fn order(&self) -> u64 {
        self.order
    }

    pub fn project(
        mut self,
        mut project: impl FnMut(&JavaString, &JavaString, AttachmentSyncInput<'_>) -> Option<Compound>,
    ) -> Option<AttachmentSync> {
        let (namespace, name) = std::mem::take(&mut self.key);
        let value = match &self.entry {
            None => None,
            Some(entry) => Some(project(&namespace, &name, entry.sync_input())?),
        };
        Some(AttachmentSync {
            namespace,
            name,
            value,
        })
    }
}

/// The data held by one entity, block entity, chunk or level.
#[derive(Default)]
// no wire: host-local canonical map and publication authority; only saved values cross the boundary.
pub struct Attachments {
    // An allocated holder map remains allocated even after removing its last value.
    allocated: bool,
    entries: BTreeMap<index::AttachmentKey, Arc<AttachmentEntry>>,
    // Keep malformed or otherwise unrecognized NBT exactly as it was read.
    // Empty holders need no shared allocation.
    raw: Option<Arc<Compound>>,
    // Block entities publish their final field per key. Other holders publish each
    // accepted binding, in order. Both queues move and roll back with their map.
    pending: BTreeSet<index::AttachmentKey>,
    ordered: Vec<OrderedAttachmentSync>,
    registration: Option<AttachmentRegistration>,
}

// Save/journal/COW copies are detached. Moves alone preserve publication authority.
impl Clone for Attachments {
    fn clone(&self) -> Self {
        Self {
            allocated: self.allocated,
            entries: self.entries.clone(),
            raw: self.raw.clone(),
            pending: self.pending.clone(),
            ordered: self.ordered.clone(),
            registration: None,
        }
    }
}

impl fmt::Debug for Attachments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attachments")
            .field("keys", &self.entries.keys())
            .finish()
    }
}

/// A lone map answers for any holder.
impl AttachmentHost for Attachments {
    fn validate_attachment(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), AttachmentError> {
        let _ = (namespace, name, sync);
        Ok(())
    }
    fn attachments(
        &mut self,
        _: Holder,
        _: AttachmentAccess,
    ) -> Result<&mut Attachments, AttachmentError> {
        Ok(self)
    }
}

impl Attachments {
    /// Whether stored attachment memberships exist, excluding preserved unknown NBT.
    #[doc(hidden)]
    pub fn has_index_entries(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Whether this map retains an index publisher. Detached clones do not.
    /// This is not a residency check: storage must rebind on installation or scope
    /// changes, and retire membership explicitly on unload.
    #[doc(hidden)]
    pub fn has_index_binding(&self) -> bool {
        self.registration.is_some()
    }

    /// Detach and retire a publisher when its owning boundary rolls back a binding.
    #[doc(hidden)]
    pub fn unbind_index(&mut self) {
        if let Some(registration) = self.registration.take() {
            registration.unregister();
        }
    }

    /// Detach the publisher without retiring it, for a caller that retires it later: the
    /// index keeps the map's rows as they are now, and later writes no longer reach them.
    #[doc(hidden)]
    pub fn take_index_binding(&mut self) -> Option<AttachmentRegistration> {
        self.registration.take()
    }

    /// Install this authoritative map in key order, invalidating any prior binding
    /// for the holder. Rebind a detached COW map before its first attachment write.
    /// The weak publisher does not keep the dimension alive. Map Drop never unloads;
    /// storage must retire the returned handle or use trusted index `unload`.
    #[doc(hidden)]
    pub fn bind_index<H: Copy + Eq + Hash + Send + Sync + 'static>(
        &mut self,
        index: &Arc<Mutex<index::AttachmentIndex<H>>>,
        holder: H,
    ) -> AttachmentRegistration {
        let change = {
            let mut index = index.lock().unwrap();
            index.register(
                holder,
                self.entries
                    .iter()
                    .map(|(key, entry)| (key.clone(), entry.clone())),
            )
        };
        let owner = change.into_result();
        let registration = AttachmentRegistration {
            publisher: Arc::new(IndexedAttachments {
                index: Arc::downgrade(index),
                owner,
            }),
        };
        self.registration = Some(registration.clone());
        registration
    }

    /// Restore content (including saved data and queued sync) without detaching the
    /// current live registration. Any binding on `replacement` is discarded, not
    /// retired. Unchanged entries keep their index slots and shared row backing.
    #[doc(hidden)]
    pub fn replace_contents(&mut self, mut replacement: Self) {
        replacement.registration = self.registration.take();
        let previous = std::mem::replace(self, replacement);
        if let Some(registration) = &self.registration {
            let changes: Vec<_> = previous
                .entries
                .keys()
                .filter(|key| !self.entries.contains_key(*key))
                .map(|key| AttachmentChange { key, entry: None })
                .chain(self.entries.iter().map(|(key, entry)| AttachmentChange {
                    key,
                    entry: Some(entry),
                }))
                .collect();
            registration.publisher.publish(&changes);
        }
    }

    // All canonical entry mutations publish the same Arc the map keeps. Retain the
    // replaced entry until after publication releases the index lock.
    fn write_entry(&mut self, key: &index::AttachmentKey, entry: Option<Arc<AttachmentEntry>>) {
        let previous = match entry {
            Some(entry) => {
                self.allocated = true;
                self.entries.insert(key.clone(), entry)
            }
            None => self.entries.remove(key),
        };
        if let Some(registration) = &self.registration {
            registration.publisher.publish(&[AttachmentChange {
                key,
                entry: self.entries.get(key),
            }]);
        }
        drop(previous);
    }

    /// Read the namespaced attachment compound, leaving unknown data intact.
    pub fn from_nbt(raw: Compound) -> Self {
        let mut result = Self::default();
        for (namespace, tag) in &raw {
            if let Tag::Compound(names) = tag {
                for (name, value) in names {
                    if let Tag::Compound(value) = value {
                        result.entries.insert(
                            (namespace.clone(), name.clone()),
                            Arc::new(AttachmentEntry::raw(value.clone())),
                        );
                    }
                }
            }
        }
        result.allocated = !result.entries.is_empty();
        result.raw = (!raw.is_empty()).then(|| Arc::new(raw));
        result
    }
    /// `Entity.load` on an entity whose constructing listeners already set values: each saved
    /// attachment replaces the value there and the others stay, so saved data wins. Unknown
    /// saved data is kept as `from_nbt` keeps it.
    #[doc(hidden)]
    pub fn read_saved_over(&mut self, raw: Compound) {
        let saved = Self::from_nbt(raw);
        for (key, entry) in &saved.entries {
            self.write_entry(key, Some(entry.clone()));
        }
        let Some(saved_raw) = saved.raw else {
            return;
        };
        let Some(own) = &mut self.raw else {
            self.raw = Some(saved_raw);
            return;
        };
        let own = Arc::make_mut(own);
        for (namespace, tag) in saved_raw.iter() {
            match (own.get_mut(namespace), tag) {
                (Some(Tag::Compound(names)), Tag::Compound(saved_names)) => {
                    for (name, value) in saved_names {
                        names.insert(name.clone(), value.clone());
                    }
                }
                _ => {
                    own.insert(namespace.clone(), tag.clone());
                }
            }
        }
    }
    /// Whether this map holds neither entries nor preserved unknown NBT.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.raw.is_none()
    }

    /// Whether a save must run a live codec before handing the immutable record to a worker.
    pub fn has_live_persistence(&self) -> bool {
        self.entries.values().any(
            |entry| matches!(&entry.state, Entry::Decoded(value) if value.persistence.is_some()),
        )
    }

    /// Save canonical objects with their persistence codec. Unread entries and
    /// unrecognized tags stay as received; replica-only values are not saved.
    /// Returns an error naming the failing attachment if any persistence codec fails.
    pub fn try_to_nbt(&self) -> Result<Compound, AttachmentEncodeError> {
        self.saved_nbt(Decoded::encode_current)
    }

    /// The staged disk edit already ran its codec; publish that validated proposal
    /// rather than invoking the codec a second time.
    #[doc(hidden)]
    pub fn to_staged_nbt(&self) -> Compound {
        self.saved_nbt(|value| Ok(value.encoded.clone()))
            .expect("staged attachment encoding is infallible")
    }

    fn saved_nbt(
        &self,
        encode: impl Fn(&Decoded) -> Result<Option<Compound>, AttachmentError>,
    ) -> Result<Compound, AttachmentEncodeError> {
        let mut raw = self.raw.as_deref().cloned().unwrap_or_default();
        for ((namespace, name), entry) in &self.entries {
            let value = match &entry.state {
                Entry::Raw {
                    kind: AttachmentRawKind::Saved,
                    saved,
                    ..
                } => Some(saved.clone()),
                Entry::Raw {
                    kind: AttachmentRawKind::Synced,
                    ..
                } => continue,
                Entry::Decoded(decoded) => encode(decoded)
                    .map_err(|error| AttachmentEncodeError {
                        namespace: String::from_utf16_lossy(&namespace.0),
                        name: String::from_utf16_lossy(&name.0),
                        error,
                    })?
                    .map(|value| envelope(value, decoded.policy.copy_on_death)),
            };
            if let Some(value) = value {
                let tag = raw
                    .entry(namespace.clone())
                    .or_insert_with(|| Tag::Compound(Compound::new()));
                let Tag::Compound(names) = tag else {
                    unreachable!("attachment namespace checked before mutation");
                };
                names.insert(name.clone(), Tag::Compound(value));
            } else if let Some(Tag::Compound(names)) = raw.get_mut(namespace) {
                names.remove(name);
            }
        }
        Ok(raw)
    }
    /// Copy serializable entries through their disk encoding for a player clone
    /// (NeoForge's `AttachmentInternals.copyEntityAttachments`).
    /// When `is_death` is true, only `copy_on_death` entries are copied; when false
    /// (End credits return), every serializable entry is copied. Codec-less entries
    /// are never copied, and copied entries start encoded rather than sharing decoded
    /// `Arc` bindings with the source map.
    pub fn copy_for_clone(
        &self,
        is_death: bool,
        policy: impl Fn(&str, &str) -> Option<CloneAttachmentPolicy>,
    ) -> Self {
        let mut copy = Self::default();
        copy.copy_clone_attachments_from(self, is_death, policy);
        copy
    }

    /// Apply the original's clone-eligible entries over the replacement's constructor values.
    #[doc(hidden)]
    pub fn copy_clone_attachments_from(
        &mut self,
        original: &Self,
        is_death: bool,
        policy: impl Fn(&str, &str) -> Option<CloneAttachmentPolicy>,
    ) {
        let flag = JavaString::from("copy_on_death");
        for (key, entry) in &original.entries {
            let current = key
                .0
                .to_string()
                .ok()
                .zip(key.1.to_string().ok())
                .and_then(|(namespace, name)| policy(&namespace, &name));
            let current = match current {
                Some(CloneAttachmentPolicy::MemoryOnly) => continue,
                Some(CloneAttachmentPolicy::Persistent { copy_on_death }) => Some(copy_on_death),
                None => None,
            };
            let encoded = match &entry.state {
                Entry::Raw {
                    saved,
                    kind: AttachmentRawKind::Saved,
                    ..
                } if !is_death
                    || current.unwrap_or(matches!(saved.get(&flag), Some(Tag::Byte(1)))) =>
                {
                    Some(saved.clone())
                }
                Entry::Decoded(value)
                    if !is_death || current.unwrap_or(value.policy.copy_on_death) =>
                {
                    value.encode_current().ok().flatten().map(|encoded| {
                        envelope(encoded, current.unwrap_or(value.policy.copy_on_death))
                    })
                }
                _ => None,
            };
            if let Some(encoded) = encoded {
                self.write_entry(key, Some(Arc::new(AttachmentEntry::raw(encoded))));
            }
        }
        self.allocated = !self.entries.is_empty();
    }
    /// Copy opted-in values through their disk encoding for a player respawn.
    pub fn copy_on_death(
        &self,
        policy: impl Fn(&str, &str) -> Option<CloneAttachmentPolicy>,
    ) -> Self {
        self.copy_for_clone(true, policy)
    }
    /// The shared immutable binding, without decoding or inserting a value.
    #[doc(hidden)]
    pub fn entry(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<&Arc<AttachmentEntry>>, AttachmentError> {
        self.check_namespace(namespace)?;
        Ok(self
            .entries
            .get(&(namespace, name) as &dyn AttachmentKeyView))
    }
    /// Whether a value is stored, saved or set, without decoding or inserting it.
    fn holds(&self, namespace: &str, name: &str) -> bool {
        self.entries
            .contains_key(&(namespace, name) as &dyn AttachmentKeyView)
    }
    /// Refuse a namespace whose saved form isn't a compound: nothing under it can be read or
    /// written without losing it.
    fn check_namespace(&self, namespace: &str) -> Result<(), AttachmentError> {
        thread_local! {
            static NAMESPACE: std::cell::RefCell<JavaString> = Default::default();
        }
        let invalid = self.raw.as_ref().is_some_and(|raw| {
            NAMESPACE.with_borrow_mut(|key| {
                key.0.clear();
                key.0.extend(namespace.encode_utf16());
                raw.get(key)
                    .is_some_and(|tag| !matches!(tag, Tag::Compound(_)))
            })
        });
        match invalid {
            true => Err(AttachmentError::InvalidNamespace),
            _ => Ok(()),
        }
    }
    fn decoded(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<Option<&AttachmentValue>, AttachmentError> {
        Ok(
            match self.entry(namespace, name)?.map(|entry| &entry.state) {
                Some(Entry::Decoded(value)) => Some(&value.value),
                _ => None,
            },
        )
    }
    fn store(
        &mut self,
        holder: Holder,
        descriptor: &AttachmentStoreKey<'_>,
        value: Decoded,
        write: AttachmentWrite,
    ) {
        let sync = value.policy.sync;
        let (namespace, name) = descriptor.id();
        // UTF-8 byte length is an upper bound on UTF-16 units. Reserve once
        // instead of repeatedly growing the short key buffers during encoding.
        let [owned_namespace, owned_name] = [namespace, name].map(|text| {
            let mut units = Vec::with_capacity(text.len());
            if text.is_ascii() {
                units.extend(text.bytes().map(u16::from));
            } else {
                units.extend(text.encode_utf16());
            }
            JavaString(units)
        });
        let key = (owned_namespace, owned_name);
        self.write_entry(
            &key,
            Some(Arc::new(AttachmentEntry {
                state: Entry::Decoded(value),
            })),
        );
        if sync && write != AttachmentWrite::Decoded {
            self.record_sync(holder, key, descriptor.sync_release);
        }
        if write == AttachmentWrite::Set {
            self.remove_saved(namespace, name);
        }
    }
    fn discard(&mut self, holder: Holder, descriptor: &AttachmentStoreKey<'_>) {
        let (namespace, name) = descriptor.id();
        let key: (JavaString, JavaString) = (namespace.into(), name.into());
        self.write_entry(&key, None);
        if descriptor.policy.sync {
            self.record_sync(holder, key, descriptor.sync_release);
        }
        self.remove_saved(namespace, name);
    }
    fn remove_saved(&mut self, namespace: &str, name: &str) {
        if let Some(raw) = &mut self.raw
            && let Some(Tag::Compound(names)) = Arc::make_mut(raw).get_mut(&namespace.into())
        {
            names.remove(&name.into());
        }
    }
    fn record_sync(
        &mut self,
        holder: Holder,
        key: index::AttachmentKey,
        release: Option<&SyncSnapshotRelease>,
    ) {
        if matches!(holder, Holder::BlockEntity(_)) {
            self.pending.insert(key);
        } else {
            let order = NEXT_SYNC_ORDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let entry = self.entries.get(&key).cloned();
            self.ordered.push(OrderedAttachmentSync {
                order,
                key,
                entry,
                release: release.cloned(),
            });
        }
    }

    /// Drain ordered updates for this map so the owner can merge them with other holders.
    pub fn take_ordered_sync(&mut self) -> Vec<OrderedAttachmentSync> {
        std::mem::take(&mut self.ordered)
    }
    /// Project accepted updates in order and coalesced block-entity keys at their final state.
    pub fn take_sync_changes(
        &mut self,
        mut project: impl FnMut(&JavaString, &JavaString, AttachmentSyncInput<'_>) -> Option<Compound>,
    ) -> Vec<AttachmentSync> {
        let mut changes: Vec<_> = self
            .take_ordered_sync()
            .into_iter()
            .filter_map(|change| change.project(&mut project))
            .collect();
        changes.extend(std::mem::take(&mut self.pending).into_iter().filter_map(
            |(namespace, name)| {
                let key = (namespace, name);
                let value = match self.entries.get(&key) {
                    None => None,
                    Some(entry) => Some(project(&key.0, &key.1, entry.sync_input())?),
                };
                Some(AttachmentSync {
                    namespace: key.0,
                    name: key.1,
                    value,
                })
            },
        ));
        changes
    }
    /// Whether a delivery pass has anything to project for this holder.
    pub fn has_pending_sync(&self) -> bool {
        !self.pending.is_empty() || !self.ordered.is_empty()
    }

    /// Discard queued changes when no client sees this holder. A new tracker gets a snapshot.
    pub fn discard_sync_changes(&mut self) {
        self.pending.clear();
        self.ordered.clear();
    }

    /// Every synced value resolved against the active registered descriptors, for a client
    /// that starts seeing the holder. Unknown saved entries stay intact and are never sent.
    pub fn synced(
        &self,
        mut project: impl FnMut(&JavaString, &JavaString, AttachmentSyncInput<'_>) -> Option<Compound>,
    ) -> Vec<AttachmentSync> {
        self.entries
            .iter()
            .filter_map(|((namespace, name), entry)| {
                let value = project(namespace, name, entry.sync_input())?;
                Some(AttachmentSync {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    value: Some(value),
                })
            })
            .collect()
    }
    /// Client side: keep what the server sent, decoded by the next `get`.
    pub fn apply_sync(&mut self, change: AttachmentSync) {
        let key = (change.namespace, change.name);
        let entry = change
            .value
            .map(|value| Arc::new(AttachmentEntry::raw_synced(value)));
        self.write_entry(&key, entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(n: &i32) -> Result<Compound, AttachmentError> {
        Ok([(JavaString::from("value"), Tag::Int(*n))].into())
    }
    fn decode(c: &Compound) -> Result<i32, AttachmentError> {
        match c.get(&"value".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        }
    }
    static COUNTER: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec { encode, decode }),
        copy_on_death: false,
        sync_codec: None,
    };
    static SYNCED: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "synced",
        default: || 7,
        codec: Some(AttachmentCodec { encode, decode }),
        copy_on_death: false,
        sync_codec: Some(AttachmentCodec { encode, decode }),
    };
    fn change(name: &str, value: Option<i32>) -> AttachmentSync {
        AttachmentSync {
            namespace: "test".into(),
            name: name.into(),
            value: value.map(|n| encode(&n).unwrap()),
        }
    }
    static MEMORY: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "memory",
        default: || 3,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };
    fn same_component(units: &JavaString, text: &str) -> bool {
        units.0.iter().copied().eq(text.encode_utf16())
    }
    fn project_type<'a, T: 'static>(
        ty: &'a AttachmentType<T>,
    ) -> impl FnMut(&JavaString, &JavaString, AttachmentSyncInput<'_>) -> Option<Compound> + 'a
    {
        move |namespace, name, input| {
            (same_component(namespace, ty.namespace) && same_component(name, ty.name))
                .then(|| ty.project_sync(input, &mut None).ok().flatten())
                .flatten()
        }
    }
    fn project_standard(
        namespace: &JavaString,
        name: &JavaString,
        input: AttachmentSyncInput<'_>,
    ) -> Option<Compound> {
        [&SYNCED, &COUNTER, &MEMORY]
            .into_iter()
            .find(|ty| same_component(namespace, ty.namespace) && same_component(name, ty.name))?
            .project_sync(input, &mut None)
            .ok()
            .flatten()
    }
    fn rows(
        index: &Arc<Mutex<index::AttachmentIndex<u32>>>,
        key: &AttachmentType<i32>,
    ) -> Arc<index::AttachmentSnapshot<u32>> {
        index
            .lock()
            .unwrap()
            .snapshot(&(key.namespace.into(), key.name.into()))
    }

    fn indexed_values(
        index: &Arc<Mutex<index::AttachmentIndex<u32>>>,
        key: &AttachmentType<i32>,
    ) -> Vec<(u32, i32)> {
        rows(index, key)
            .iter()
            .map(|row| (row.holder, *row.entry.read(key).unwrap()))
            .collect()
    }

    /// A host that lends entity 1's map and entity 2 (which keeps none) only for reading.
    struct ReadOnlyHost {
        map: Attachments,
    }
    impl AttachmentHost for ReadOnlyHost {
        fn validate_attachment(&self, _: &str, _: &str, _: bool) -> Result<(), AttachmentError> {
            Ok(())
        }
        fn attachments(
            &mut self,
            _: Holder,
            _: AttachmentAccess,
        ) -> Result<&mut Attachments, AttachmentError> {
            Err(AttachmentError::MissingHolder)
        }
        fn read_only_attachments(
            &self,
            holder: Holder,
        ) -> Option<Result<Option<&Attachments>, AttachmentError>> {
            match holder {
                Holder::Entity(1) => Some(Ok(Some(&self.map))),
                Holder::Entity(2) => Some(Ok(None)),
                _ => None,
            }
        }
    }

    /// How many times [`COUNTED`]'s encoder and default ran.
    static COUNTED_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static COUNTED: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "counted",
        default: || {
            COUNTED_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            -1
        },
        codec: Some(AttachmentCodec {
            encode: |n| {
                COUNTED_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                encode(n)
            },
            decode,
        }),
        copy_on_death: false,
        sync_codec: None,
    };

    #[test]
    fn a_read_only_holder_answers_reads_and_refuses_every_write() {
        use std::sync::atomic::Ordering;
        let mut source = Attachments::default();
        COUNTER.set(&mut source, Holder::Level, 5).unwrap();
        COUNTED.set(&mut source, Holder::Level, 6).unwrap();
        let mut map = Attachments::from_nbt(source.try_to_nbt().unwrap());
        MEMORY.set(&mut map, Holder::Level, 8).unwrap();
        let mut host = ReadOnlyHost { map };
        let (one, two) = (Holder::Entity(1), Holder::Entity(2));
        COUNTED_CALLS.store(0, Ordering::SeqCst);

        assert_eq!(host.read_only(one), Ok(true));
        assert_eq!(host.read_only(Holder::Level), Ok(false));
        assert_eq!(COUNTER.has(&mut host, one), Ok(true));
        assert_eq!(COUNTER.get_existing(&mut host, one), Ok(Some(5)));
        assert_eq!(MEMORY.get_existing(&mut host, one), Ok(Some(8)));
        // The saved value decoded for that read alone, without its encoder: the lent map still
        // keeps it saved.
        assert_eq!(COUNTED.get_existing(&mut host, one), Ok(Some(6)));
        assert!(matches!(host.map.decoded("test", "counted"), Ok(None)));
        assert_eq!(COUNTER.has(&mut host, two), Ok(false));
        assert_eq!(COUNTER.get_existing(&mut host, two), Ok(None));

        // Writes refuse before the type's default or encoder runs.
        assert_eq!(
            COUNTED.get(&mut host, two),
            Err(AttachmentError::ReadOnlyHolder)
        );
        assert_eq!(
            COUNTED.set(&mut host, one, 9),
            Err(AttachmentError::ReadOnlyHolder)
        );
        assert_eq!(
            COUNTED.set(&mut host, two, 9),
            Err(AttachmentError::ReadOnlyHolder)
        );
        assert_eq!(COUNTED_CALLS.load(Ordering::SeqCst), 0);
        assert_eq!(
            COUNTER.remove(&mut host, one),
            Err(AttachmentError::ReadOnlyHolder)
        );
        assert_eq!(
            SYNCED.sync_data(&mut host, one),
            Err(AttachmentError::ReadOnlyHolder)
        );
        // What is a no-op on any holder stays one.
        assert_eq!(COUNTER.remove(&mut host, two), Ok(None));
        assert_eq!(COUNTER.sync_data(&mut host, one), Ok(()));
        assert_eq!(COUNTER.has(&mut host, two), Ok(false));
        assert_eq!(COUNTER.get_existing(&mut host, one), Ok(Some(5)));
        assert_eq!(MEMORY.get_existing(&mut host, one), Ok(Some(8)));
        // Any other holder still goes through the host's writable maps.
        assert_eq!(
            COUNTER.has(&mut host, Holder::Level),
            Err(AttachmentError::MissingHolder)
        );
    }

    #[test]
    fn binding_publishes_unread_saved_entries_and_canonical_decode() {
        let mut source = Attachments::default();
        COUNTER.set(&mut source, Holder::Level, 42).unwrap();
        SYNCED.set(&mut source, Holder::Level, 8).unwrap();
        let saved = source.try_to_nbt().unwrap();
        let mut map = Attachments::from_nbt(saved.clone());
        let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
        map.bind_index(&index, 1);
        let before = rows(&index, &COUNTER);
        assert!(Arc::ptr_eq(
            &before[0].entry,
            map.entry("test", "counter").unwrap().unwrap()
        ));
        assert_eq!(indexed_values(&index, &COUNTER), [(1, 42)]);
        assert_eq!(indexed_values(&index, &SYNCED), [(1, 8)]);
        assert_eq!(map.try_to_nbt().unwrap(), saved);
        assert!(map.take_sync_changes(project_standard).is_empty());
        assert_eq!(COUNTER.get_existing(&mut map, Holder::Level), Ok(Some(42)));
        let decoded = rows(&index, &COUNTER);
        assert!(!Arc::ptr_eq(&before[0].entry, &decoded[0].entry));
        assert!(Arc::ptr_eq(
            &decoded[0].entry,
            map.entry("test", "counter").unwrap().unwrap()
        ));
        assert_eq!(before[0].entry.read(&COUNTER), Ok(&42));
        assert_eq!(map.try_to_nbt().unwrap(), saved);
        assert!(map.take_sync_changes(project_standard).is_empty());
    }

    #[test]
    fn bound_default_set_remove_and_client_sync_publish_current_shared_values() {
        let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
        let mut map = Attachments::default();
        map.bind_index(&index, 1);
        assert!(rows(&index, &SYNCED).is_empty());
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(7));
        let default = rows(&index, &SYNCED);
        assert_eq!(
            map.take_sync_changes(project_standard),
            [change("synced", Some(7))]
        );
        SYNCED.set(&mut map, Holder::Level, 9).unwrap();
        assert_eq!(indexed_values(&index, &SYNCED), [(1, 9)]);
        assert_eq!(SYNCED.remove(&mut map, Holder::Level), Ok(Some(9)));
        assert!(rows(&index, &SYNCED).is_empty());
        assert_eq!(
            map.take_sync_changes(project_standard),
            [change("synced", Some(9)), change("synced", None)]
        );
        map.apply_sync(change("synced", Some(12)));
        assert_eq!(indexed_values(&index, &SYNCED), [(1, 12)]);
        assert!(map.take_sync_changes(project_standard).is_empty());
        map.apply_sync(change("synced", None));
        assert!(rows(&index, &SYNCED).is_empty());
        assert_eq!(default[0].entry.read(&SYNCED), Ok(&7));
    }

    #[test]
    fn clone_is_detached_moves_keep_publication_and_drop_never_unloads() {
        let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
        let mut map = Attachments::default();
        COUNTER.set(&mut map, Holder::Level, 1).unwrap();
        let registration = map.bind_index(&index, 1);
        let mut detached = map.clone();
        COUNTER.set(&mut detached, Holder::Level, 2).unwrap();
        COUNTER.remove(&mut detached, Holder::Level).unwrap();
        detached.apply_sync(change("counter", Some(3)));
        drop(detached);
        drop(registration.clone());
        assert_eq!(indexed_values(&index, &COUNTER), [(1, 1)]);
        let mut moved = map;
        COUNTER.set(&mut moved, Holder::Level, 4).unwrap();
        drop(moved);
        assert_eq!(indexed_values(&index, &COUNTER), [(1, 4)]);
        assert!(registration.unregister());
        assert!(rows(&index, &COUNTER).is_empty());
    }

    #[test]
    fn cow_rebind_keeps_entries_and_rejects_stale_rows_maps_and_lifetime_handles() {
        let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
        let mut old = Attachments::default();
        COUNTER.set(&mut old, Holder::Level, 1).unwrap();
        let old_registration = old.bind_index(&index, 1);
        let mut neighbor = old.clone();
        COUNTER.set(&mut neighbor, Holder::Level, 2).unwrap();
        neighbor.bind_index(&index, 2);
        let before = rows(&index, &COUNTER);
        let mut current = old.clone();
        let registration = current.bind_index(&index, 1);
        assert!(Arc::ptr_eq(
            &before[0].entry,
            &rows(&index, &COUNTER)[0].entry
        ));
        assert_eq!(
            index.lock().unwrap().validate_row(&before[0]),
            Err(AttachmentRowError::StaleHolder)
        );
        COUNTER.set(&mut old, Holder::Level, 99).unwrap();
        COUNTER.remove(&mut old, Holder::Level).unwrap();
        old.apply_sync(change("counter", Some(100)));
        old.replace_contents(Attachments::default());
        assert!(!old_registration.unregister());
        assert_eq!(indexed_values(&index, &COUNTER), [(1, 1), (2, 2)]);
        COUNTER.set(&mut current, Holder::Level, 3).unwrap();
        assert_eq!(indexed_values(&index, &COUNTER), [(1, 3), (2, 2)]);
        assert_eq!(before[0].entry.read(&COUNTER), Ok(&1));
        assert!(registration.unregister());
        assert!(!registration.unregister());
        COUNTER.set(&mut current, Holder::Level, 4).unwrap();
        assert_eq!(indexed_values(&index, &COUNTER), [(2, 2)]);
        assert_eq!(COUNTER.get(&mut current, Holder::Level), Ok(4));
    }

    #[test]
    fn restoring_contents_restores_membership_payloads_saved_data_and_future_writes() {
        let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
        let mut map = Attachments::from_nbt([("unknown".into(), Tag::Int(5))].into());
        SYNCED.set(&mut map, Holder::Level, 10).unwrap();
        MEMORY.set(&mut map, Holder::Level, 20).unwrap();
        map.bind_index(&index, 1);
        let kept = map.clone();
        let saved = kept.try_to_nbt().unwrap();
        SYNCED.set(&mut map, Holder::Level, 11).unwrap();
        MEMORY.remove(&mut map, Holder::Level).unwrap();
        COUNTER.set(&mut map, Holder::Level, 30).unwrap();
        let aborted = rows(&index, &SYNCED);
        map.replace_contents(kept);
        assert_eq!(map.try_to_nbt().unwrap(), saved);
        assert_eq!(
            map.take_sync_changes(project_standard),
            [change("synced", Some(10))]
        );
        assert_eq!(indexed_values(&index, &SYNCED), [(1, 10)]);
        assert_eq!(indexed_values(&index, &MEMORY), [(1, 20)]);
        assert!(rows(&index, &COUNTER).is_empty());
        assert_eq!(aborted[0].entry.read(&SYNCED), Ok(&11));
        SYNCED.set(&mut map, Holder::Level, 12).unwrap();
        assert_eq!(indexed_values(&index, &SYNCED), [(1, 12)]);
        map.replace_contents(Attachments::default());
        assert!(rows(&index, &SYNCED).is_empty());
        assert!(rows(&index, &MEMORY).is_empty());
        COUNTER.get(&mut map, Holder::Level).unwrap();
        assert_eq!(indexed_values(&index, &COUNTER), [(1, 0)]);
    }

    #[test]
    fn replacing_contents_keeps_only_the_receivers_registration_and_unchanged_rows() {
        let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
        let mut map = Attachments::default();
        COUNTER.set(&mut map, Holder::Level, 1).unwrap();
        let registration = map.bind_index(&index, 1);
        let before = rows(&index, &COUNTER);
        map.replace_contents(map.clone());
        assert!(Arc::ptr_eq(&before, &rows(&index, &COUNTER)));
        let mut other = map.clone();
        COUNTER.set(&mut other, Holder::Level, 2).unwrap();
        let other_registration = other.bind_index(&index, 2);
        map.replace_contents(other);
        COUNTER.set(&mut map, Holder::Level, 3).unwrap();
        assert_eq!(indexed_values(&index, &COUNTER), [(1, 3), (2, 2)]);
        assert!(registration.unregister());
        assert_eq!(indexed_values(&index, &COUNTER), [(2, 2)]);
        assert!(other_registration.unregister());
    }

    #[test]
    fn failed_typed_removal_still_publishes_removal_of_the_unread_saved_key() {
        let raw = [(
            "test".into(),
            Tag::Compound([("counter".into(), Tag::Compound(Compound::new()))].into()),
        )]
        .into();
        let mut map = Attachments::from_nbt(raw);
        let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
        map.bind_index(&index, 1);
        let before = rows(&index, &COUNTER);
        assert_eq!(before.len(), 1);
        assert_eq!(
            COUNTER.remove(&mut map, Holder::Level),
            Err(AttachmentError::Decode)
        );
        assert!(rows(&index, &COUNTER).is_empty());
        assert_eq!(before[0].entry.read(&COUNTER), Err(AttachmentError::Decode));
    }

    #[test]
    fn expired_index_does_not_keep_maps_or_registration_from_standalone_use() {
        let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
        let weak = Arc::downgrade(&index);
        let mut map = Attachments::default();
        let registration = map.bind_index(&index, 1);
        drop(index);
        assert!(weak.upgrade().is_none());
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(7));
        SYNCED.set(&mut map, Holder::Level, 8).unwrap();
        let saved = map.clone();
        SYNCED.remove(&mut map, Holder::Level).unwrap();
        map.replace_contents(saved);
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(8));
        map.apply_sync(change("synced", Some(9)));
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(9));
        assert!(!registration.unregister());
    }

    #[test]
    fn retirement_and_rebind_drop_mod_payloads_only_after_unlock() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        #[derive(Clone)]
        struct Probe {
            index: Weak<Mutex<index::AttachmentIndex<u32>>>,
            unlocked: Arc<AtomicBool>,
            drops: Arc<AtomicUsize>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                if let Some(index) = self.index.upgrade() {
                    self.unlocked
                        .fetch_and(index.try_lock().is_ok(), Ordering::Relaxed);
                }
                self.drops.fetch_add(1, Ordering::Relaxed);
            }
        }
        let key: AttachmentType<Probe> = AttachmentType {
            namespace: "test",
            name: "drop_probe",
            default: || panic!("no default"),
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        let mut outcomes = Vec::new();
        for rebind in [false, true] {
            let index = Arc::new(Mutex::new(index::AttachmentIndex::default()));
            let unlocked = Arc::new(AtomicBool::new(true));
            let drops = Arc::new(AtomicUsize::new(0));
            let mut map = Attachments::default();
            key.set(
                &mut map,
                Holder::Level,
                Probe {
                    index: Arc::downgrade(&index),
                    unlocked: unlocked.clone(),
                    drops: drops.clone(),
                },
            )
            .unwrap();
            let registration = map.bind_index(&index, 1);
            let mut replacement = map.clone();
            key.remove(&mut replacement, Holder::Level).unwrap();
            drop(map);
            if rebind {
                replacement.bind_index(&index, 1);
            } else {
                assert!(registration.unregister());
            }
            assert_eq!(drops.load(Ordering::Relaxed), 2);
            outcomes.push(unlocked.load(Ordering::Relaxed));
        }
        assert_eq!(outcomes, [true, true]);
    }

    #[test]
    fn defaults_and_codecs_run_without_an_index_lock() {
        static INDEX: OnceLock<Arc<Mutex<index::AttachmentIndex<u32>>>> = OnceLock::new();
        fn probe() {
            drop(
                INDEX
                    .get()
                    .unwrap()
                    .try_lock()
                    .expect("mod code under index lock"),
            );
        }
        let key = AttachmentType {
            namespace: "test",
            name: "probe",
            default: || {
                probe();
                5
            },
            codec: Some(AttachmentCodec {
                encode: |n| {
                    probe();
                    encode(n)
                },
                decode: |form| {
                    probe();
                    decode(form)
                },
            }),
            copy_on_death: false,
            sync_codec: None,
        };
        let index = INDEX.get_or_init(Default::default);
        let mut map = Attachments::default();
        map.bind_index(index, 1);
        assert_eq!(key.get(&mut map, Holder::Level), Ok(5));
        key.set(&mut map, Holder::Level, 6).unwrap();
        let mut loaded = Attachments::from_nbt(map.try_to_nbt().unwrap());
        loaded.bind_index(index, 1);
        assert_eq!(indexed_values(index, &key), [(1, 6)]);
        assert_eq!(key.get_existing(&mut loaded, Holder::Level), Ok(Some(6)));
    }

    #[test]
    fn saved_values_read_over_a_map_replace_only_their_own_keys() {
        let mut source = Attachments::from_nbt([("unknown".into(), Tag::Int(5))].into());
        COUNTER.set(&mut source, Holder::Level, 42).unwrap();
        let saved = source.try_to_nbt().unwrap();
        // What the constructing listeners set before the load.
        let mut map = Attachments::default();
        COUNTER.set(&mut map, Holder::Level, 1).unwrap();
        SYNCED.set(&mut map, Holder::Level, 9).unwrap();
        map.read_saved_over(saved);
        assert_eq!(COUNTER.get(&mut map, Holder::Level), Ok(42));
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(9));
        let mut reloaded = Attachments::from_nbt(map.try_to_nbt().unwrap());
        assert_eq!(COUNTER.get(&mut reloaded, Holder::Level), Ok(42));
        assert_eq!(SYNCED.get(&mut reloaded, Holder::Level), Ok(9));
        assert_eq!(
            reloaded.try_to_nbt().unwrap().get(&"unknown".into()),
            Some(&Tag::Int(5))
        );

        // A map with saved data of its own keeps it beside what is read over it.
        let mut own = Attachments::from_nbt([("kept".into(), Tag::Int(1))].into());
        own.read_saved_over([("unknown".into(), Tag::Int(5))].into());
        let nbt = own.try_to_nbt().unwrap();
        assert_eq!(nbt.get(&"kept".into()), Some(&Tag::Int(1)));
        assert_eq!(nbt.get(&"unknown".into()), Some(&Tag::Int(5)));
    }

    #[test]
    fn empty_and_new_maps_need_no_saved_backing_allocation() {
        let mut map = Attachments::default();
        assert!(map.raw.is_none());
        COUNTER.set(&mut map, Holder::Level, 1).unwrap();
        assert!(map.raw.is_none());
        COUNTER.remove(&mut map, Holder::Level).unwrap();
        assert!(map.raw.is_none());
        assert!(Attachments::from_nbt(Compound::new()).raw.is_none());
    }
    #[test]
    fn shared_bindings_and_map_snapshots_keep_replaced_and_removed_values() {
        let mut map = Attachments::default();
        SYNCED.set(&mut map, Holder::Level, 1).unwrap();
        map.take_sync_changes(project_standard);
        let loaded = Attachments::from_nbt(map.try_to_nbt().unwrap());
        for mut map in [map, loaded] {
            let mut snapshot = map.clone();
            let entry = map.entry("test", "synced").unwrap().unwrap().clone();
            assert!(Arc::ptr_eq(
                &entry,
                snapshot.entry("test", "synced").unwrap().unwrap()
            ));
            let saved = snapshot.try_to_nbt().unwrap();

            SYNCED.set(&mut map, Holder::Level, 2).unwrap();
            assert_eq!(entry.read(&SYNCED), Ok(&1));
            assert_eq!(SYNCED.get_existing(&mut map, Holder::Level), Ok(Some(2)));
            assert_eq!(SYNCED.remove(&mut map, Holder::Level), Ok(Some(2)));
            assert!(map.entry("test", "synced").unwrap().is_none());
            assert_eq!(entry.read(&SYNCED), Ok(&1));
            assert_eq!(
                SYNCED.get_existing(&mut snapshot, Holder::Level),
                Ok(Some(1))
            );
            assert_eq!(snapshot.try_to_nbt().unwrap(), saved);
            assert!(snapshot.take_sync_changes(project_standard).is_empty());
            assert_eq!(
                map.take_sync_changes(project_standard),
                [change("synced", Some(2)), change("synced", None)]
            );
        }
    }
    #[test]
    fn raw_read_cache_borrows_without_claiming_the_canonical_type_or_policy() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static DECODES: AtomicUsize = AtomicUsize::new(0);
        struct NonClone(i32);
        let key = AttachmentType {
            namespace: "test",
            name: "counter",
            default: || panic!("a query must not create a default"),
            codec: Some(AttachmentCodec {
                encode: |_: &NonClone| panic!("a query must not encode"),
                decode: |saved| {
                    DECODES.fetch_add(1, Ordering::Relaxed);
                    decode(saved).map(NonClone)
                },
            }),
            copy_on_death: true,
            sync_codec: Some(AttachmentCodec {
                encode: |_: &NonClone| panic!("a query must not encode"),
                decode: |_: &Compound| panic!("a disk read must use persistence codec"),
            }),
        };
        let mut original = Attachments::default();
        COUNTER.set(&mut original, Holder::Level, 42).unwrap();
        let saved = original.try_to_nbt().unwrap();
        let mut map = Attachments::from_nbt(saved.clone());
        let snapshot = map.clone();
        assert!(Arc::ptr_eq(
            map.raw.as_ref().unwrap(),
            snapshot.raw.as_ref().unwrap()
        ));
        let entry = map.entry("test", "counter").unwrap().unwrap().clone();
        let value = entry.read(&key).unwrap();
        assert_eq!(value.0, 42);
        assert!(std::ptr::eq(value, entry.read(&key).unwrap()));
        assert!(std::ptr::eq(
            value,
            snapshot
                .entry("test", "counter")
                .unwrap()
                .unwrap()
                .read(&key)
                .unwrap()
        ));
        assert_eq!(DECODES.load(Ordering::Relaxed), 1);
        assert!(map.decoded("test", "counter").unwrap().is_none());
        assert_eq!(map.try_to_nbt().unwrap(), saved);
        assert!(map.synced(project_standard).is_empty());
        assert!(map.take_sync_changes(project_standard).is_empty());
        assert!(
            map.copy_on_death(|_, _| None)
                .entry("test", "counter")
                .unwrap()
                .is_none()
        );

        // Another query type gets its own borrowed cache; ordinary reads still decode raw.
        assert_eq!(entry.read(&COUNTER), Ok(&42));
        assert!(std::ptr::eq(value, entry.read(&key).unwrap()));
        assert_eq!(COUNTER.get_existing(&mut map, Holder::Level), Ok(Some(42)));
        let decoded = map.entry("test", "counter").unwrap().unwrap();
        assert!(matches!(
            decoded.read(&key),
            Err(AttachmentError::TypeMismatch)
        ));
        assert_eq!(map.try_to_nbt().unwrap(), saved);
        assert!(map.take_sync_changes(project_standard).is_empty());
        assert_eq!(entry.read(&key).unwrap().0, 42);
    }
    #[test]
    fn query_reads_reject_invalid_policy_without_populating_the_cache() {
        let mut map = Attachments::default();
        map.apply_sync(change("synced", Some(9)));
        let invalid = AttachmentType {
            codec: None,
            copy_on_death: true,
            ..SYNCED
        };
        let entry = map.entry("test", "synced").unwrap().unwrap();
        assert_eq!(
            entry.read(&invalid),
            Err(AttachmentError::InvalidCopyPolicy)
        );
        assert_eq!(entry.read(&SYNCED), Ok(&9));
        assert!(map.take_sync_changes(project_standard).is_empty());
    }
    #[test]
    fn failed_query_decodes_are_retried_without_changing_the_saved_binding() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        let key = AttachmentType {
            sync_codec: Some(AttachmentCodec {
                encode,
                decode: |saved| {
                    if ATTEMPTS.fetch_add(1, Ordering::Relaxed) == 0 {
                        Err(AttachmentError::Decode)
                    } else {
                        decode(saved)
                    }
                },
            }),
            ..SYNCED
        };
        let mut map = Attachments::default();
        map.apply_sync(change("synced", Some(9)));
        let saved = map.try_to_nbt().unwrap();
        let entry = map.entry("test", "synced").unwrap().unwrap();
        assert_eq!(entry.read(&key), Err(AttachmentError::Decode));
        assert_eq!(entry.read(&key), Ok(&9));
        assert_eq!(entry.read(&key), Ok(&9));
        assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 2);
        assert!(map.decoded("test", "synced").unwrap().is_none());
        assert_eq!(map.try_to_nbt().unwrap(), saved);
        assert!(map.take_sync_changes(project_standard).is_empty());
    }
    #[test]
    fn sync_replacement_and_removal_leave_the_shared_old_entry_readable() {
        let mut map = Attachments::default();
        map.apply_sync(change("synced", Some(9)));
        let old = map.entry("test", "synced").unwrap().unwrap().clone();
        map.apply_sync(change("synced", Some(10)));
        assert_eq!(old.read(&SYNCED), Ok(&9));
        assert_eq!(SYNCED.get_existing(&mut map, Holder::Level), Ok(Some(10)));
        map.apply_sync(change("synced", None));
        assert_eq!(old.read(&SYNCED), Ok(&9));
        assert_eq!(SYNCED.get_existing(&mut map, Holder::Level), Ok(None));
    }
    #[test]
    fn saved_values_round_trip_and_memory_values_do_not() {
        let mut map = Attachments::default();
        COUNTER.set(&mut map, Holder::Level, 42).unwrap();
        MEMORY.set(&mut map, Holder::Level, 8).unwrap();
        let mut loaded = Attachments::from_nbt(map.try_to_nbt().unwrap());
        assert_eq!(COUNTER.get(&mut loaded, Holder::Level), Ok(42));
        assert_eq!(MEMORY.get(&mut loaded, Holder::Level), Ok(3));
        assert_eq!(COUNTER.remove(&mut loaded, Holder::Level), Ok(Some(42)));
        let mut reopened = Attachments::from_nbt(loaded.try_to_nbt().unwrap());
        assert_eq!(COUNTER.get(&mut reopened, Holder::Level), Ok(0));
    }
    #[test]
    fn untouched_saved_value_copies_on_respawn() {
        let keep = AttachmentType {
            namespace: "test",
            name: "keep",
            default: || 0,
            codec: Some(AttachmentCodec { encode, decode }),
            copy_on_death: true,
            sync_codec: None,
        };
        let mut original = Attachments::default();
        keep.set(&mut original, Holder::Level, 42).unwrap();
        COUNTER.set(&mut original, Holder::Level, 5).unwrap();
        let loaded = Attachments::from_nbt(original.try_to_nbt().unwrap());
        let mut respawned = loaded.copy_on_death(|_, _| None);
        assert_eq!(keep.get(&mut respawned, Holder::Level), Ok(42));
        assert_eq!(COUNTER.get(&mut respawned, Holder::Level), Ok(0));
    }
    #[test]
    fn saved_reads_and_removal_do_not_depend_on_the_sync_encoder() {
        let key = AttachmentType {
            sync_codec: Some(AttachmentCodec {
                encode: |_| Err(AttachmentError::Encode),
                decode,
            }),
            ..SYNCED
        };
        let mut original = Attachments::default();
        key.set(&mut original, Holder::Level, 31).unwrap();
        let saved = original.try_to_nbt().unwrap();
        let mut loaded = Attachments::from_nbt(saved.clone());
        assert_eq!(key.get_existing(&mut loaded, Holder::Level), Ok(Some(31)));
        assert_eq!(loaded.try_to_nbt().unwrap(), saved);
        assert!(loaded.take_sync_changes(project_standard).is_empty());
        let mut unread = Attachments::from_nbt(saved);
        assert_eq!(key.remove(&mut unread, Holder::Level), Ok(Some(31)));
        assert_eq!(key.get_existing(&mut unread, Holder::Level), Ok(None));
    }

    #[test]
    fn rolled_back_defaults_and_sets_never_run_the_sync_encoder() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let key = AttachmentType {
            sync_codec: Some(AttachmentCodec {
                encode: |n| {
                    CALLS.fetch_add(1, Ordering::Relaxed);
                    encode(n)
                },
                decode,
            }),
            ..SYNCED
        };
        let mut holder = Attachments::default();
        let journal_before = holder.clone();
        assert_eq!(key.get(&mut holder, Holder::Level), Ok(7));
        key.set(&mut holder, Holder::Level, 42).unwrap();
        holder.replace_contents(journal_before);
        assert!(holder.take_sync_changes(project_type(&key)).is_empty());
        assert_eq!(CALLS.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn explicit_resync_projects_the_current_value_instead_of_the_previous_encoding() {
        use std::sync::atomic::{AtomicI32, Ordering};
        let key = AttachmentType {
            namespace: "test",
            name: "live",
            default: || Arc::new(AtomicI32::new(0)),
            codec: None,
            copy_on_death: false,
            sync_codec: Some(AttachmentCodec {
                encode: |value| encode(&value.load(Ordering::Relaxed)),
                decode: |bytes| decode(bytes).map(|n| Arc::new(AtomicI32::new(n))),
            }),
        };
        let value = Arc::new(AtomicI32::new(7));
        let mut holder = Attachments::default();
        key.set(&mut holder, Holder::Level, value.clone()).unwrap();
        assert_eq!(
            holder.take_sync_changes(project_type(&key)),
            [change("live", Some(7))]
        );
        value.store(42, Ordering::Relaxed);
        key.sync_data(&mut holder, Holder::Level).unwrap();
        assert_eq!(
            holder.take_sync_changes(project_type(&key)),
            [change("live", Some(42))]
        );
    }

    #[test]
    fn saved_and_copied_values_encode_interior_mutation_through_a_loan() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let key = CheckedAttachmentType::new(AttachmentType {
            namespace: "test",
            name: "atomic",
            default: || AtomicU32::new(7),
            codec: Some(AttachmentCodec {
                encode: |value| {
                    Ok([("n".into(), Tag::Int(value.load(Ordering::Relaxed) as i32))].into())
                },
                decode: |tag| match tag.get(&"n".into()) {
                    Some(Tag::Int(value)) => Ok(AtomicU32::new(*value as u32)),
                    _ => Err(AttachmentError::Decode),
                },
            }),
            copy_on_death: true,
            sync_codec: None,
        })
        .unwrap();
        let mut holder = Attachments::default();
        assert_eq!(
            key.get(&mut holder, Holder::Level, |value| value
                .swap(42, Ordering::Relaxed)),
            Ok(7)
        );
        let mut saved = Attachments::from_nbt(holder.try_to_nbt().unwrap());
        let mut copied = holder.copy_on_death(|_, _| None);
        for replacement in [&mut saved, &mut copied] {
            assert_eq!(
                key.get_existing(replacement, Holder::Level, |value| value
                    .map(|value| value.load(Ordering::Relaxed))),
                Ok(Some(42))
            );
        }
    }

    #[test]
    fn existing_reads_leave_an_absent_value_absent() {
        let mut map = Attachments::default();
        assert!(!COUNTER.has(&mut map, Holder::Level).unwrap());
        assert_eq!(COUNTER.get_existing(&mut map, Holder::Level), Ok(None));
        assert!(!COUNTER.has(&mut map, Holder::Level).unwrap());
        assert_eq!(map.try_to_nbt().unwrap(), Compound::new());
        assert_eq!(COUNTER.get(&mut map, Holder::Level), Ok(0));
        assert!(COUNTER.has(&mut map, Holder::Level).unwrap());
        assert_ne!(map.try_to_nbt().unwrap(), Compound::new());
        COUNTER.set(&mut map, Holder::Level, 5).unwrap();
        assert_eq!(COUNTER.get_existing(&mut map, Holder::Level), Ok(Some(5)));
    }
    #[test]
    fn existing_read_decodes_a_saved_value_without_changing_it() {
        let mut map = Attachments::default();
        COUNTER.set(&mut map, Holder::Level, 42).unwrap();
        MEMORY.set(&mut map, Holder::Level, 8).unwrap();
        let saved = map.try_to_nbt().unwrap();
        let mut loaded = Attachments::from_nbt(saved.clone());
        assert!(COUNTER.has(&mut loaded, Holder::Level).unwrap());
        assert_eq!(
            COUNTER.get_existing(&mut loaded, Holder::Level),
            Ok(Some(42))
        );
        assert_eq!(loaded.try_to_nbt().unwrap(), saved);
        assert!(!MEMORY.has(&mut loaded, Holder::Level).unwrap());
        assert_eq!(MEMORY.get_existing(&mut loaded, Holder::Level), Ok(None));
    }
    #[test]
    fn unknown_and_invalid_values_survive_a_failed_decode() {
        let saved: Compound = [(
            JavaString::from("test"),
            Tag::Compound(
                [
                    (
                        JavaString::from("counter"),
                        Tag::Compound(
                            [(JavaString::from("value"), Tag::String("bad".into()))].into(),
                        ),
                    ),
                    (JavaString(vec![0xd800]), Tag::Compound(Compound::new())),
                ]
                .into(),
            ),
        )]
        .into();
        let mut map = Attachments::from_nbt(saved.clone());
        let old = map.entry("test", "counter").unwrap().unwrap().clone();
        assert_eq!(old.read(&COUNTER), Err(AttachmentError::Decode));
        assert!(COUNTER.has(&mut map, Holder::Level).unwrap());
        assert_eq!(
            COUNTER.get_existing(&mut map, Holder::Level),
            Err(AttachmentError::Decode)
        );
        assert_eq!(
            COUNTER.get(&mut map, Holder::Level),
            Err(AttachmentError::Decode)
        );
        assert_eq!(map.try_to_nbt().unwrap(), saved);
        assert_eq!(
            COUNTER.remove(&mut map, Holder::Level),
            Err(AttachmentError::Decode)
        );
        assert_eq!(
            COUNTER.remove(&mut map, Holder::Level),
            Ok(None),
            "removed all the same"
        );
        assert_eq!(old.read(&COUNTER), Err(AttachmentError::Decode));
    }
    #[test]
    fn block_entity_sync_coalesces_writes_and_removal_to_one_final_state() {
        let mut map = Attachments::default();
        let holder = Holder::BlockEntity([0, 65, 0]);
        SYNCED.set(&mut map, holder, 1).unwrap();
        SYNCED.set(&mut map, holder, 2).unwrap();
        SYNCED.remove(&mut map, holder).unwrap();
        assert_eq!(
            map.take_sync_changes(project_standard),
            [change("synced", None)]
        );
        assert!(map.take_sync_changes(project_standard).is_empty());
    }
    #[test]
    fn non_block_entity_sync_keeps_each_accepted_write_in_order() {
        for holder in [
            Holder::Entity(7),
            Holder::Player(17),
            Holder::Chunk([2, 3]),
            Holder::Level,
        ] {
            let mut map = Attachments::default();
            SYNCED.set(&mut map, holder, 18).unwrap();
            SYNCED.set(&mut map, holder, 29).unwrap();
            assert_eq!(
                map.take_sync_changes(project_standard),
                [
                    AttachmentSync {
                        namespace: "test".into(),
                        name: "synced".into(),
                        value: Some([("value".into(), Tag::Int(18))].into())
                    },
                    AttachmentSync {
                        namespace: "test".into(),
                        name: "synced".into(),
                        value: Some([("value".into(), Tag::Int(29))].into())
                    },
                ]
            );
            assert!(map.take_sync_changes(project_standard).is_empty());
        }
    }
    #[test]
    fn default_creation_is_synced_and_a_later_read_is_not() {
        let mut map = Attachments::default();
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(7));
        assert_eq!(
            map.take_sync_changes(project_standard),
            [change("synced", Some(7))]
        );
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(7));
        assert!(map.take_sync_changes(project_standard).is_empty());
    }
    #[test]
    fn removing_a_missing_synced_key_clears_a_replica_default_only_from_an_allocated_map() {
        let holder = Holder::Level;
        let mut server = Attachments::default();
        let mut replica = Attachments::default();
        assert_eq!(SYNCED.get(&mut replica, holder), Ok(7));
        assert_eq!(SYNCED.remove(&mut server, holder), Ok(None));
        assert!(server.take_sync_changes(project_standard).is_empty());

        COUNTER.set(&mut server, holder, 4).unwrap();
        assert_eq!(SYNCED.remove(&mut server, holder), Ok(None));
        let changes = server.take_sync_changes(project_standard);
        assert_eq!(
            changes,
            [AttachmentSync {
                namespace: "test".into(),
                name: "synced".into(),
                value: None
            }]
        );
        for change in changes {
            replica.apply_sync(change);
        }
        assert_eq!(SYNCED.get_existing(&mut replica, holder), Ok(None));

        // Removing the last unrelated value does not deallocate the server's map.
        assert_eq!(COUNTER.remove(&mut server, holder), Ok(Some(4)));
        assert_eq!(SYNCED.remove(&mut server, holder), Ok(None));
        assert_eq!(
            server.take_sync_changes(project_standard),
            [change("synced", None)]
        );
    }
    #[test]
    fn unsynced_types_report_no_changes() {
        let mut map = Attachments::default();
        COUNTER.set(&mut map, Holder::Level, 1).unwrap();
        COUNTER.remove(&mut map, Holder::Level).unwrap();
        COUNTER.sync_data(&mut map, Holder::Level).unwrap();
        assert!(map.take_sync_changes(project_standard).is_empty());
    }
    #[test]
    fn sync_without_persistence_codec_syncs_and_does_not_persist() {
        let sync_only = AttachmentType {
            namespace: "test",
            name: "ephemeral",
            default: || 5,
            codec: None,
            copy_on_death: false,
            sync_codec: Some(AttachmentCodec { encode, decode }),
        };
        let mut server = Attachments::default();
        assert_eq!(sync_only.get(&mut server, Holder::Level), Ok(5));
        assert_eq!(
            server.take_sync_changes(project_type(&sync_only)),
            [change("ephemeral", Some(5))]
        );
        sync_only.set(&mut server, Holder::Level, 11).unwrap();
        assert_eq!(server.try_to_nbt().unwrap(), Compound::new());
        assert_eq!(
            server.synced(project_type(&sync_only)),
            [change("ephemeral", Some(11))]
        );
        let updates = server.take_sync_changes(project_type(&sync_only));
        assert_eq!(updates, [change("ephemeral", Some(11))]);

        let mut client = Attachments::default();
        for update in updates {
            client.apply_sync(update);
        }
        assert_eq!(client.try_to_nbt().unwrap(), Compound::new());
        assert_eq!(
            client
                .entry("test", "ephemeral")
                .unwrap()
                .unwrap()
                .read(&sync_only),
            Ok(&11)
        );
        assert_eq!(
            sync_only.get_existing(&mut client, Holder::Level),
            Ok(Some(11))
        );
        assert_eq!(client.try_to_nbt().unwrap(), Compound::new());

        // copy_on_death still requires a persistence codec even when sync_codec is present.
        let bad_copy = AttachmentType {
            copy_on_death: true,
            ..sync_only
        };
        assert_eq!(
            bad_copy.set(&mut server, Holder::Level, 1),
            Err(AttachmentError::InvalidCopyPolicy)
        );
    }
    #[test]
    fn distinct_persistence_and_sync_codecs_project_from_active_descriptor_on_both_routes() {
        fn encode_disk(n: &i32) -> Result<Compound, AttachmentError> {
            Ok([(JavaString::from("disk"), Tag::Int(*n))].into())
        }
        fn decode_disk(c: &Compound) -> Result<i32, AttachmentError> {
            match c.get(&"disk".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            }
        }
        fn encode_net(n: &i32) -> Result<Compound, AttachmentError> {
            Ok([(JavaString::from("net"), Tag::Int(*n + 100))].into())
        }
        fn decode_net(c: &Compound) -> Result<i32, AttachmentError> {
            match c.get(&"net".into()) {
                Some(Tag::Int(n)) => Ok(*n - 100),
                _ => Err(AttachmentError::Decode),
            }
        }
        fn encode_net_v2(n: &i32) -> Result<Compound, AttachmentError> {
            Ok([(JavaString::from("net_v2"), Tag::Int(*n * 10))].into())
        }
        let dual = AttachmentType {
            namespace: "test",
            name: "dual",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: encode_disk,
                decode: decode_disk,
            }),
            copy_on_death: false,
            sync_codec: Some(AttachmentCodec {
                encode: encode_net,
                decode: decode_net,
            }),
        };
        let mut server = Attachments::default();
        dual.set(&mut server, Holder::Level, 7).unwrap();
        let expected_sync = AttachmentSync {
            namespace: "test".into(),
            name: "dual".into(),
            value: Some(encode_net(&7).unwrap()),
        };
        assert_eq!(
            server.take_sync_changes(project_type(&dual)),
            *std::slice::from_ref(&expected_sync)
        );
        assert_eq!(
            server.synced(project_type(&dual)),
            *std::slice::from_ref(&expected_sync)
        );

        // Saved disk envelope carries only persistence data (no sync flag or sync_value).
        let saved = server.try_to_nbt().unwrap();
        let Some(Tag::Compound(names)) = saved.get(&"test".into()) else {
            panic!("namespace saved");
        };
        let Some(Tag::Compound(envelope)) = names.get(&"dual".into()) else {
            panic!("entry saved");
        };
        assert_eq!(
            envelope.get(&"value".into()),
            Some(&Tag::Compound(encode_disk(&7).unwrap()))
        );
        assert!(!envelope.contains_key(&"sync".into()));
        assert!(!envelope.contains_key(&"sync_value".into()));

        // Reloading without a prior typed read projects through the active descriptor on both
        // initial tracking (`synced`) and explicit resync (`sync_data` + `take_sync_changes`).
        let mut reloaded = Attachments::from_nbt(saved.clone());
        assert_eq!(
            reloaded.synced(project_type(&dual)),
            *std::slice::from_ref(&expected_sync)
        );
        dual.sync_data(&mut reloaded, Holder::Level).unwrap();
        assert_eq!(
            reloaded.take_sync_changes(project_type(&dual)),
            *std::slice::from_ref(&expected_sync)
        );

        // Changing the active descriptor's sync codec re-projects from disk without stale NBT.
        let dual_v2 = AttachmentType {
            sync_codec: Some(AttachmentCodec {
                encode: encode_net_v2,
                decode: |_| Err(AttachmentError::Decode),
            }),
            ..dual
        };
        let expected_v2 = AttachmentSync {
            namespace: "test".into(),
            name: "dual".into(),
            value: Some(encode_net_v2(&7).unwrap()),
        };
        assert_eq!(
            reloaded.synced(project_type(&dual_v2)),
            *std::slice::from_ref(&expected_v2)
        );
        dual_v2.sync_data(&mut reloaded, Holder::Level).unwrap();
        assert_eq!(
            reloaded.take_sync_changes(project_type(&dual_v2)),
            [expected_v2]
        );

        // Dropping sync or leaving the entry unregistered sends nothing while keeping disk NBT.
        let no_sync = AttachmentType {
            sync_codec: None,
            ..dual
        };
        assert!(reloaded.synced(project_type(&no_sync)).is_empty());
        assert!(reloaded.synced(|_, _, _| None).is_empty());
        assert_eq!(reloaded.try_to_nbt().unwrap(), saved);

        assert_eq!(
            reloaded.entry("test", "dual").unwrap().unwrap().read(&dual),
            Ok(&7)
        );
        assert_eq!(dual.get_existing(&mut reloaded, Holder::Level), Ok(Some(7)));
        assert_eq!(reloaded.try_to_nbt().unwrap(), saved);

        // Client decodes synced packets with sync_codec without touching the persistence codec.
        let client_view = AttachmentType {
            codec: Some(AttachmentCodec {
                encode: |_| panic!("client sync read must not invoke persistence encode"),
                decode: |_| panic!("client sync read must not invoke persistence decode"),
            }),
            ..dual
        };
        let mut client = Attachments::default();
        client.apply_sync(expected_sync);
        assert_eq!(
            client
                .entry("test", "dual")
                .unwrap()
                .unwrap()
                .read(&client_view),
            Ok(&7)
        );
        assert_eq!(
            client_view.get_existing(&mut client, Holder::Level),
            Ok(Some(7))
        );
        assert_eq!(client.try_to_nbt().unwrap(), Compound::new());
    }
    #[test]
    fn saved_entries_sync_only_through_active_descriptors_and_preserve_unknown_entries() {
        let mut map = Attachments::default();
        SYNCED.set(&mut map, Holder::Level, 5).unwrap();
        COUNTER.set(&mut map, Holder::Level, 6).unwrap();
        let mut saved = map.try_to_nbt().unwrap();
        let Some(Tag::Compound(names)) = saved.get(&"test".into()) else {
            panic!("namespace saved");
        };
        let Some(Tag::Compound(envelope)) = names.get(&"synced".into()) else {
            panic!("entry saved");
        };
        assert!(!envelope.contains_key(&"sync".into()));
        assert!(!envelope.contains_key(&"sync_value".into()));
        saved.insert(
            "unknown".into(),
            Tag::Compound(
                [(
                    "opaque".into(),
                    Tag::Compound([("value".into(), Tag::Compound(encode(&99).unwrap()))].into()),
                )]
                .into(),
            ),
        );

        let loaded = Attachments::from_nbt(saved.clone());
        assert_eq!(loaded.synced(project_standard), [change("synced", Some(5))]);
        // A saved attachment that gains a sync codec on load syncs without a prior typed read.
        let counter_now_synced = AttachmentType {
            sync_codec: Some(AttachmentCodec { encode, decode }),
            ..COUNTER
        };
        assert_eq!(
            loaded.synced(project_type(&counter_now_synced)),
            [change("counter", Some(6))]
        );
        assert_eq!(loaded.try_to_nbt().unwrap(), saved);
    }
    #[test]
    fn applied_changes_decode_through_the_codec_and_removals_clear() {
        let mut map = Attachments::default();
        map.apply_sync(change("synced", Some(9)));
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(9));
        map.apply_sync(change("synced", None));
        assert_eq!(SYNCED.get(&mut map, Holder::Level), Ok(7));
    }
    #[test]
    fn copy_for_clone_copies_serializable_entries_by_policy_and_skips_unserialized() {
        const UNSERIALIZED: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "unserialized",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        const KEEP_ON_DEATH: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "keep_on_death",
            default: || 0,
            codec: Some(AttachmentCodec { encode, decode }),
            copy_on_death: true,
            sync_codec: None,
        };
        let mut source = Attachments::default();
        COUNTER.set(&mut source, Holder::Level, 11).unwrap();
        KEEP_ON_DEATH.set(&mut source, Holder::Level, 22).unwrap();
        UNSERIALIZED.set(&mut source, Holder::Level, 33).unwrap();

        let mut end_copy = source.copy_for_clone(false, |_, _| None);
        assert_eq!(UNSERIALIZED.has(&mut end_copy, Holder::Level), Ok(false));
        assert!(end_copy.decoded("test", "counter").unwrap().is_none());
        assert!(end_copy.decoded("test", "keep_on_death").unwrap().is_none());
        assert_eq!(
            COUNTER.get_existing(&mut end_copy, Holder::Level),
            Ok(Some(11))
        );
        assert_eq!(
            KEEP_ON_DEATH.get_existing(&mut end_copy, Holder::Level),
            Ok(Some(22))
        );
        // Decoded Arcs are independent from the source map.
        let src_arc = source.decoded("test", "counter").unwrap().unwrap();
        let copy_arc = end_copy.decoded("test", "counter").unwrap().unwrap();
        assert!(!Arc::ptr_eq(src_arc, copy_arc));

        let mut death_copy = source.copy_for_clone(true, |_, _| None);
        assert_eq!(UNSERIALIZED.has(&mut death_copy, Holder::Level), Ok(false));
        assert_eq!(COUNTER.has(&mut death_copy, Holder::Level), Ok(false));
        assert!(
            death_copy
                .decoded("test", "keep_on_death")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            KEEP_ON_DEATH.get_existing(&mut death_copy, Holder::Level),
            Ok(Some(22))
        );
    }
    #[test]
    fn sync_data_reports_the_current_value() {
        struct One(Attachments);
        impl AttachmentHost for One {
            fn validate_attachment(
                &self,
                namespace: &str,
                name: &str,
                sync: bool,
            ) -> Result<(), AttachmentError> {
                let _ = (namespace, name, sync);
                Ok(())
            }
            fn attachments(
                &mut self,
                _: Holder,
                _: AttachmentAccess,
            ) -> Result<&mut Attachments, AttachmentError> {
                Ok(&mut self.0)
            }
        }
        let mut host = One(Attachments::default());
        SYNCED.set(&mut host, Holder::Level, 4).unwrap();
        host.0.take_sync_changes(project_standard);
        SYNCED.sync_data(&mut host, Holder::Level).unwrap();
        assert_eq!(
            host.0.take_sync_changes(project_standard),
            [change("synced", Some(4))]
        );
    }

    #[test]
    fn checked_attachment_type_validates_copy_policy_and_allows_sync_only() {
        assert_eq!(
            CheckedAttachmentType::new(AttachmentType {
                namespace: "test",
                name: "x",
                default: || 0i32,
                codec: None,
                copy_on_death: true,
                sync_codec: None,
            })
            .err(),
            Some(AttachmentError::InvalidCopyPolicy)
        );
        let sync_only = CheckedAttachmentType::new(AttachmentType {
            namespace: "test",
            name: "sync_only",
            default: || 7i32,
            codec: None,
            copy_on_death: false,
            sync_codec: Some(AttachmentCodec { encode, decode }),
        })
        .expect("sync_codec without persistence codec is valid");
        assert!(!sync_only.has_codec());
        assert!(!sync_only.is_persistent());
        assert!(sync_only.is_synced());
        assert_eq!(
            sync_only.policy(),
            AttachmentPolicy {
                copy_on_death: false,
                sync: true,
            }
        );

        let mut store = Attachments::default();
        assert_eq!(
            sync_only.get(&mut store, Holder::Level, |value| *value),
            Ok(7)
        );
        assert_eq!(sync_only.has(&mut store, Holder::Level), Ok(true));
        sync_only.set(&mut store, Holder::Level, 11).unwrap();
        assert_eq!(
            sync_only.get_existing(&mut store, Holder::Level, |value| value.copied()),
            Ok(Some(11))
        );
        assert_eq!(
            store.take_sync_changes(project_type(sync_only.as_type())),
            [change("sync_only", Some(7)), change("sync_only", Some(11))]
        );
        assert!(store.try_to_nbt().unwrap().is_empty());
    }

    #[test]
    fn encode_failure_refuses_serialization_with_attachment_id() {
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Clone)]
        struct Toggle {
            value: i32,
            fail: Arc<AtomicBool>,
        }

        fn encode_toggle(t: &Toggle) -> Result<Compound, AttachmentError> {
            if t.fail.load(Ordering::Relaxed) {
                return Err(AttachmentError::Encode);
            }
            Ok([(JavaString::from("v"), Tag::Int(t.value))].into())
        }
        fn decode_toggle(c: &Compound) -> Result<Toggle, AttachmentError> {
            match c.get(&"v".into()) {
                Some(Tag::Int(value)) => Ok(Toggle {
                    value: *value,
                    fail: Arc::new(AtomicBool::new(false)),
                }),
                _ => Err(AttachmentError::Decode),
            }
        }
        static TOGGLE: AttachmentType<Toggle> = AttachmentType {
            namespace: "example_mod",
            name: "counter",
            default: || Toggle {
                value: 0,
                fail: Arc::new(AtomicBool::new(false)),
            },
            codec: Some(AttachmentCodec {
                encode: encode_toggle,
                decode: decode_toggle,
            }),
            copy_on_death: false,
            sync_codec: None,
        };

        let fail = Arc::new(AtomicBool::new(false));
        let mut store = Attachments::default();
        TOGGLE
            .set(
                &mut store,
                Holder::Level,
                Toggle {
                    value: 17,
                    fail: fail.clone(),
                },
            )
            .unwrap();
        let saved = store.try_to_nbt().unwrap();
        fail.store(true, Ordering::Relaxed);
        let error = store.try_to_nbt().unwrap_err();
        assert_eq!(error.namespace, "example_mod");
        assert_eq!(error.name, "counter");
        assert_eq!(error.error, AttachmentError::Encode);
        // The previously saved NBT still decodes the original value.
        let mut reloaded = Attachments::from_nbt(saved);
        assert_eq!(
            TOGGLE
                .get_existing(&mut reloaded, Holder::Level)
                .unwrap()
                .map(|t| t.value),
            Some(17)
        );
    }
}

#[cfg(test)]
mod borrowed_key_tests {
    use super::*;
    #[test]
    fn borrowed_order_matches_java_units() {
        let samples = [
            "", "a", "aa", "z", "é", "\u{d7ff}", "\u{e000}", "\u{ffff}", "😀", "𐀀", "a😀",
        ];
        for a in samples {
            for b in samples {
                for c in samples {
                    for d in samples {
                        let owned: (JavaString, JavaString) = (a.into(), b.into());
                        let other: (JavaString, JavaString) = (c.into(), d.into());
                        let borrowed = (a, b);
                        let right = (c, d);
                        assert_eq!(
                            (&borrowed as &dyn AttachmentKeyView).cmp(&right),
                            owned.cmp(&other)
                        );
                        assert_eq!(
                            (&owned as &dyn AttachmentKeyView).cmp(&right),
                            owned.cmp(&other)
                        );
                    }
                }
            }
        }
        // Saved Java names can contain unpaired surrogates as well.
        let malformed = (JavaString(vec![0xd800]), JavaString(vec![0xdc00]));
        let valid = ("𐀀", "");
        assert_eq!(
            (&malformed as &dyn AttachmentKeyView).cmp(&valid),
            malformed.cmp(&(valid.0.into(), valid.1.into()))
        );
    }
}
