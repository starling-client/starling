//! Load-owned typed attachment descriptors, issued keys, and registration traits.
use super::*;
use crate::{
    AttachmentCodec, AttachmentError, AttachmentPolicy, AttachmentStoreKey, Compound, Holder,
    Identifier, JavaString, LevelRandomHost,
};
use std::{fmt, sync::atomic::AtomicBool};
use sunrise_mod_runtime::attachments::{
    AttachmentSyncInput, AttachmentType, CheckedAttachmentType, CloneAttachmentPolicy,
};

/// Builder for a typed attachment registered during [`Hooks::register`](crate::Hooks::register).
///
/// Mirrors NeoForge's `AttachmentType.Builder`: an attachment is declared once with its
/// namespaced ID (`"namespace:name"`) and default value constructor, then optionally configured
/// with a persistence codec ([`serialize`](Self::serialize)), a client-sync codec
/// ([`sync`](Self::sync)), and [`copy_on_death`](Self::copy_on_death). Persistence and sync
/// codecs are independent.
// no wire: host-local registration builder with callback pointers; never crosses the wire.
// exhaustive: its fields are private; constructed through AttachmentDescriptor::new
pub struct AttachmentDescriptor<T: 'static> {
    id: &'static str,
    default: fn() -> T,
    codec: Option<AttachmentCodec<T>>,
    copy_on_death: bool,
    sync_codec: Option<AttachmentCodec<T>>,
    refusal: Option<AttachmentError>,
}

impl<T: 'static> AttachmentDescriptor<T> {
    /// Start a descriptor for the namespaced attachment `id` (`"namespace:name"`) with its
    /// default value constructor.
    ///
    /// Use a unique lowercase `namespace:name` in a namespace the registering package owns or
    /// serves, not `minecraft`. [`Registrar::register_attachment_type`] records the registering
    /// package as the owner. A dependent package obtains the issued key via
    /// [`Registrar::attachment_key`], without declaring that ID again.
    pub const fn new(id: &'static str, default: fn() -> T) -> Self {
        Self {
            id,
            default,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
            refusal: None,
        }
    }

    /// Persist this attachment to disk with `codec`.
    /// A second call leaves the first codec in place and makes registration fail.
    pub fn serialize(mut self, codec: AttachmentCodec<T>) -> Self {
        if self.codec.is_some() {
            self.refusal
                .get_or_insert(AttachmentError::DuplicateSerializer);
        } else {
            self.codec = Some(codec);
        }
        self
    }

    /// Synchronize this attachment to clients using `codec`, independently of disk persistence.
    pub fn sync(mut self, codec: AttachmentCodec<T>) -> Self {
        self.sync_codec = Some(codec);
        self
    }

    /// Copy this attachment when a player respawns from death.
    /// Calling this before [`serialize`](Self::serialize) makes registration fail.
    pub fn copy_on_death(mut self) -> Self {
        if self.codec.is_none() {
            self.refusal
                .get_or_insert(AttachmentError::InvalidCopyPolicy);
        } else {
            self.copy_on_death = true;
        }
        self
    }

    /// The full namespaced attachment ID (`"namespace:name"`).
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// Whether this descriptor has a persistence codec.
    pub const fn has_codec(&self) -> bool {
        self.codec.is_some()
    }

    /// Whether this descriptor synchronizes to clients.
    pub const fn is_synced(&self) -> bool {
        self.sync_codec.is_some()
    }

    /// Whether this descriptor copies on player death.
    pub const fn copy_on_death_enabled(&self) -> bool {
        self.copy_on_death
    }
}

impl<T: 'static> fmt::Debug for AttachmentDescriptor<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AttachmentDescriptor")
            .field("id", &self.id)
            .field("type", &type_name::<T>())
            .field("has_codec", &self.has_codec())
            .field("is_synced", &self.is_synced())
            .field("copy_on_death", &self.copy_on_death)
            .finish()
    }
}

type BuildAttachmentRecord = Box<
    dyn FnOnce(
            &'static str,
            &'static str,
            &'static str,
        ) -> Result<Arc<dyn Any + Send + Sync>, AttachmentError>
        + Send,
>;

#[derive(Debug, PartialEq, Eq)]
enum ProjectionFailure {
    Refused(AttachmentError),
    Panicked,
}

/// Engine diagnostic for an attachment that could not produce a sync value.
// hidden: only the engine's sync projection reports it, to its own log.
// exhaustive: its fields are private and only the projection builds it.
#[doc(hidden)]
// no wire: diagnostic for a local codec call.
#[derive(Debug, PartialEq, Eq)]
pub struct AttachmentSyncProjectionError {
    owner: &'static str,
    failure: ProjectionFailure,
}

impl fmt::Display for AttachmentSyncProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.failure {
            ProjectionFailure::Refused(error) => {
                write!(f, "mod {}: attachment_sync refused: {error:?}", self.owner)
            }
            ProjectionFailure::Panicked => {
                write!(f, "mod {}: attachment_sync panicked", self.owner)
            }
        }
    }
}

type ProjectAttachmentSync = fn(
    &(dyn Any + Send + Sync),
    AttachmentSyncInput<'_>,
    &AtomicBool,
) -> Result<Option<Compound>, AttachmentSyncProjectionError>;

type MatchAttachmentKey = fn(&(dyn Any + Send + Sync), &AttachmentStoreKey<'_>) -> bool;

/// Type-erased attachment descriptor accepted by [`AttachmentTypesRegistrar::submit_attachment`].
///
/// Built from a typed [`AttachmentDescriptor<T>`] via [`From`] / [`Into`].
// no wire: host-local registration descriptor carrying erased typed callbacks; never crosses the wire.
// exhaustive: its fields are private; built via From<AttachmentDescriptor<T>>
pub struct ErasedAttachmentDescriptor {
    id: &'static str,
    has_codec: bool,
    policy: AttachmentPolicy,
    build: BuildAttachmentRecord,
    project_sync: ProjectAttachmentSync,
    match_key: MatchAttachmentKey,
}

impl<T: Send + Sync + 'static> From<AttachmentDescriptor<T>> for ErasedAttachmentDescriptor {
    fn from(descriptor: AttachmentDescriptor<T>) -> Self {
        fn project_sync<T: Send + Sync + 'static>(
            record: &(dyn Any + Send + Sync),
            input: AttachmentSyncInput<'_>,
            poisoned: &AtomicBool,
        ) -> Result<Option<Compound>, AttachmentSyncProjectionError> {
            let registration = record
                .downcast_ref::<AttachmentRegistration<T>>()
                .expect("sync projection supplied by this descriptor");
            let owner = registration.owner;
            let panicked = |_| AttachmentSyncProjectionError {
                owner,
                failure: ProjectionFailure::Panicked,
            };
            let mut decoded_owner = None;
            let result =
                crate::runtime::invoke_registered_codec(owner, "attachment_sync", poisoned, || {
                    Ok(registration
                        .checked
                        .as_type()
                        .project_sync(input, &mut decoded_owner))
                });
            // A saved value is owned outside the encoder's unwind. Release it even when
            // encoding already disabled the codec, without a second unwind crossing that fence.
            if let Some(value) = decoded_owner {
                crate::runtime::finish_registered_codec(owner, "attachment_sync", poisoned, || {
                    drop(value);
                    Ok(())
                })
                .map_err(panicked)?;
            }
            result
                .map_err(panicked)?
                .map_err(|error| AttachmentSyncProjectionError {
                    owner,
                    failure: ProjectionFailure::Refused(error),
                })
        }

        fn match_key<T: Send + Sync + 'static>(
            record: &(dyn Any + Send + Sync),
            write: &AttachmentStoreKey<'_>,
        ) -> bool {
            record
                .downcast_ref::<AttachmentRegistration<T>>()
                .is_some_and(|registration| {
                    write.matches_descriptor(registration.checked.as_type())
                })
        }

        let id = descriptor.id;
        let has_codec = descriptor.codec.is_some();
        let policy = AttachmentPolicy {
            copy_on_death: descriptor.copy_on_death,
            sync: descriptor.sync_codec.is_some(),
        };
        let build: BuildAttachmentRecord = Box::new(move |namespace, name, owner| {
            if let Some(error) = descriptor.refusal {
                return Err(error);
            }
            let poisoned = AtomicBool::new(false);
            let checked = CheckedAttachmentType::with_persistence_boundary(
                AttachmentType {
                    namespace,
                    name,
                    default: descriptor.default,
                    codec: descriptor.codec,
                    copy_on_death: descriptor.copy_on_death,
                    sync_codec: descriptor.sync_codec,
                },
                move |encode| {
                    crate::runtime::invoke_registered_codec(
                        owner,
                        "attachment_save",
                        &poisoned,
                        || encode().map_err(|error| format!("{id}: {error:?}")),
                    )
                    .map_err(|_| AttachmentError::Encode)
                },
            )?
            .with_sync_release(move |entry| {
                let operation = "attachment_sync_drop";
                let _mod = crate::runtime::mark_mod(owner, operation);
                crate::runtime::contain_callback(
                    || drop(entry),
                    |payload| {
                        crate::runtime::log_and_queue_panic(operation, owner, payload);
                    },
                );
            });
            Ok(Arc::new(AttachmentRegistration { id, owner, checked })
                as Arc<dyn Any + Send + Sync>)
        });
        Self {
            id,
            has_codec,
            policy,
            build,
            project_sync: project_sync::<T>,
            match_key: match_key::<T>,
        }
    }
}

impl ErasedAttachmentDescriptor {
    /// The full namespaced attachment ID (`"namespace:name"`).
    pub fn id(&self) -> &'static str {
        self.id
    }

    /// Whether this descriptor has a persistence codec.
    pub fn has_codec(&self) -> bool {
        self.has_codec
    }

    /// Whether this descriptor synchronizes to clients.
    pub fn is_synced(&self) -> bool {
        self.policy.sync
    }

    /// Whether this descriptor copies on player death.
    pub fn copy_on_death_enabled(&self) -> bool {
        self.policy.copy_on_death
    }
}

impl fmt::Debug for ErasedAttachmentDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErasedAttachmentDescriptor")
            .field("id", &self.id)
            .field("has_codec", &self.has_codec)
            .field("policy", &self.policy)
            .finish()
    }
}

pub(crate) struct AttachmentRegistration<T: 'static> {
    pub(crate) id: &'static str,
    pub(crate) owner: &'static str,
    pub(crate) checked: CheckedAttachmentType<T>,
}

/// Process-local identity of a single attachment registration, compared by pointer equality.
#[derive(Clone)]
pub(crate) struct AttachmentIdentity(Arc<dyn Any + Send + Sync>);

impl PartialEq for AttachmentIdentity {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

impl Eq for AttachmentIdentity {}

impl fmt::Debug for AttachmentIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AttachmentIdentity")
            .field(&Arc::as_ptr(&self.0))
            .finish()
    }
}

/// An opaque handle to an attachment registered during startup.
// no wire: host-local registration record holding an erased typed descriptor; never crosses the wire.
// exhaustive: its fields are private; opaque registration handle
#[derive(Clone)]
pub struct RegisteredAttachment {
    id: &'static str,
    namespace: &'static str,
    name: &'static str,
    owner: &'static str,
    has_codec: bool,
    policy: AttachmentPolicy,
    record: Arc<dyn Any + Send + Sync>,
    project_sync: ProjectAttachmentSync,
    match_key: MatchAttachmentKey,
    sync_panicked: Arc<AtomicBool>,
}

impl RegisteredAttachment {
    /// The full namespaced attachment ID (`"namespace:name"`).
    pub fn id(&self) -> &'static str {
        self.id
    }

    /// The attachment ID's namespace part.
    pub fn namespace(&self) -> &'static str {
        self.namespace
    }

    /// The attachment ID's path/name part.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The package ID that registered this attachment.
    pub fn owner(&self) -> &'static str {
        self.owner
    }

    /// Whether this attachment has a persistence codec.
    pub fn has_codec(&self) -> bool {
        self.has_codec
    }

    /// Whether this attachment synchronizes to clients.
    pub fn is_synced(&self) -> bool {
        self.policy.sync
    }

    /// The attachment's persistence and sync policy.
    pub fn policy(&self) -> AttachmentPolicy {
        self.policy
    }

    pub(crate) fn matches_identity(&self, identity: &AttachmentIdentity) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.record), Arc::as_ptr(&identity.0))
    }

    pub(crate) fn downcast_key<T: Send + Sync + 'static>(
        &self,
    ) -> Result<AttachmentKey<T>, AttachmentError> {
        Arc::clone(&self.record)
            .downcast::<AttachmentRegistration<T>>()
            .map(AttachmentKey)
            .map_err(|_| AttachmentError::TypeMismatch)
    }

    fn key_for<T: Send + Sync + 'static>(
        &self,
        id: &str,
    ) -> Result<AttachmentKey<T>, AttachmentError> {
        if self.id != id {
            return Err(AttachmentError::RegistrationMismatch);
        }
        self.downcast_key::<T>()
    }

    pub(crate) fn project_sync(
        &self,
        input: AttachmentSyncInput<'_>,
    ) -> Result<Option<Compound>, AttachmentSyncProjectionError> {
        (self.project_sync)(self.record.as_ref(), input, &self.sync_panicked)
    }
}

impl PartialEq for RegisteredAttachment {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.record), Arc::as_ptr(&other.record))
    }
}

impl Eq for RegisteredAttachment {}

impl fmt::Debug for RegisteredAttachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredAttachment")
            .field("id", &self.id)
            .field("owner", &self.owner)
            .field("has_codec", &self.has_codec)
            .field("policy", &self.policy)
            .finish()
    }
}

/// Validate an erased attachment descriptor and build its [`RegisteredAttachment`] record.
// hidden: shared between vocabulary fixtures and `sunrise-mod-api`'s registrar; not an authoring method.
#[doc(hidden)]
pub fn check_attachment_descriptor(
    descriptor: ErasedAttachmentDescriptor,
    owner: &'static str,
) -> Result<RegisteredAttachment, String> {
    if Identifier::parse_namespaced(descriptor.id).is_none() {
        return Err("explicit lowercase namespaced id required".to_owned());
    }
    let (namespace, name) = descriptor
        .id
        .split_once(':')
        .expect("parsed namespaced id contains ':'");
    let record = (descriptor.build)(namespace, name, owner).map_err(|err| match err {
        AttachmentError::InvalidCopyPolicy => "copy_on_death requires a serializer".to_owned(),
        other => format!("{other:?}"),
    })?;
    Ok(RegisteredAttachment {
        id: descriptor.id,
        namespace,
        name,
        owner,
        has_codec: descriptor.has_codec,
        policy: descriptor.policy,
        record,
        project_sync: descriptor.project_sync,
        match_key: descriptor.match_key,
        sync_panicked: Default::default(),
    })
}

/// Typed handle to an attachment registered during [`Hooks::register`](crate::Hooks::register).
///
/// Cloning an `AttachmentKey` is cheap (it shares the underlying registration) and preserves its
/// identity. A key obtained from a previous installation is refused with
/// [`AttachmentError::StaleKey`] even when a later installation registers the same ID and Rust
/// type.
// no wire: host-local typed handle with callback pointers and registration identity; never crosses the wire.
// exhaustive: its field is private; issued only by attachment registration
pub struct AttachmentKey<T: 'static>(Arc<AttachmentRegistration<T>>);

impl<T: 'static> Clone for AttachmentKey<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: 'static> PartialEq for AttachmentKey<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: 'static> Eq for AttachmentKey<T> {}

impl<T: 'static> fmt::Debug for AttachmentKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AttachmentKey")
            .field("id", &self.0.id)
            .field("owner", &self.0.owner)
            .field("type", &type_name::<T>())
            .finish()
    }
}

impl<T: 'static> AttachmentKey<T> {
    /// The full namespaced attachment ID (`"namespace:name"`).
    pub fn id(&self) -> &'static str {
        self.0.id
    }

    /// The attachment ID's namespace part.
    pub fn namespace(&self) -> &'static str {
        self.0.checked.namespace()
    }

    /// The attachment ID's path/name part.
    pub fn name(&self) -> &'static str {
        self.0.checked.name()
    }

    /// The package ID that registered this attachment.
    pub fn owner(&self) -> &'static str {
        self.0.owner
    }

    /// Whether this attachment has a persistence codec.
    pub fn has_codec(&self) -> bool {
        self.0.checked.has_codec()
    }

    /// Whether this attachment synchronizes to clients.
    pub fn is_synced(&self) -> bool {
        self.0.checked.is_synced()
    }

    /// The attachment's persistence and sync policy.
    pub fn policy(&self) -> AttachmentPolicy {
        self.0.checked.policy()
    }

    /// Whether this key belongs to the currently installed registration epoch.
    pub fn is_live(&self) -> bool {
        self.live().is_ok()
    }

    pub(crate) fn live(&self) -> Result<&CheckedAttachmentType<T>, AttachmentError> {
        self.live_in(installed().ok_or(AttachmentError::StaleKey)?)
    }

    pub(crate) fn live_with_epoch(
        &self,
    ) -> Result<(&CheckedAttachmentType<T>, u64), AttachmentError> {
        let parts = installed().ok_or(AttachmentError::StaleKey)?;
        Ok((self.live_in(parts)?, parts.epoch))
    }

    pub(crate) fn live_in(
        &self,
        parts: &crate::parts::Installed,
    ) -> Result<&CheckedAttachmentType<T>, AttachmentError> {
        let current = parts
            .attachment_types
            .get(self.namespace())
            .and_then(|types| types.get(self.name()))
            .ok_or(AttachmentError::StaleKey)?;
        if !std::ptr::addr_eq(Arc::as_ptr(&current.record), Arc::as_ptr(&self.0)) {
            return Err(AttachmentError::StaleKey);
        }
        Ok(&self.0.checked)
    }

    pub(crate) fn parts(&self) -> (&'static str, &'static str) {
        (self.namespace(), self.name())
    }

    pub(crate) fn descriptor(&self) -> &CheckedAttachmentType<T> {
        &self.0.checked
    }
}

impl<T: Send + Sync + 'static> AttachmentKey<T> {
    pub(crate) fn identity(&self) -> AttachmentIdentity {
        AttachmentIdentity(Arc::clone(&self.0) as Arc<dyn Any + Send + Sync>)
    }

    /// Lend a value for `read`, inserting the owner's default if absent. The loan
    /// does not outlive the call; copy or project only what you need to keep.
    ///
    /// ```compile_fail
    /// use mod_vocab::{AttachmentKey, Holder, Host};
    /// fn escape<'a>(host: &'a mut dyn Host, key: &'a AttachmentKey<u32>) -> &'a u32 {
    ///     key.get(host, Holder::Level, |value| value).unwrap()
    /// }
    /// ```
    pub fn get<R>(
        &self,
        host: &mut dyn Host,
        holder: Holder,
        read: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentError> {
        self.live()?.get(host.attachments(), holder, read)
    }

    /// Whether `holder` currently carries a value; never inserts the default.
    pub fn has(&self, host: &mut dyn Host, holder: Holder) -> Result<bool, AttachmentError> {
        self.live()?.has(host.attachments(), holder)
    }

    /// Lend the existing value or `None`. Decodes a saved or synced value once,
    /// and never inserts a default. On error `read` is not called.
    pub fn get_existing<R>(
        &self,
        host: &mut dyn Host,
        holder: Holder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        self.live()?.get_existing(host.attachments(), holder, read)
    }

    /// Borrow an already-decoded value without inserting or decoding it.
    pub fn with_decoded<R>(
        &self,
        host: &mut dyn Host,
        holder: Holder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        self.live()?.with_decoded(host.attachments(), holder, read)
    }

    /// Replace a value on `holder`.
    pub fn set(
        &self,
        host: &mut dyn Host,
        holder: Holder,
        value: T,
    ) -> Result<(), AttachmentError> {
        self.live()?.set(host.attachments(), holder, value)
    }

    /// Remove a value from `holder`, returning the previous value if one was present.
    /// A server removal of an absent synced key from an allocated map still queues a removal
    /// marker to clear any replica value.
    pub fn remove(&self, host: &mut dyn Host, holder: Holder) -> Result<Option<T>, AttachmentError>
    where
        T: Clone,
    {
        self.live()?.remove(host.attachments(), holder)
    }

    /// Queue the current value on `holder` for synchronization to clients that see it, or a
    /// removal marker if the value is absent. A live key with no sync codec does nothing.
    ///
    /// Server-only: client hooks cannot queue attachment sync packets.
    ///
    /// ```compile_fail
    /// use mod_vocab::{AttachmentKey, ClientHost, Holder};
    /// fn client_cannot_sync(host: &mut dyn ClientHost, key: &AttachmentKey<u32>) {
    ///     let _ = key.sync_data(host, Holder::Level);
    /// }
    /// ```
    pub fn sync_data(
        &self,
        host: &mut dyn ServerHost,
        holder: Holder,
    ) -> Result<(), AttachmentError> {
        self.live()?.sync_data(host.attachments(), holder)
    }

    /// [`Self::get`] through a [`LevelRandomHost`].
    pub fn get_in<R>(
        &self,
        level: &mut dyn LevelRandomHost,
        holder: Holder,
        read: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentError> {
        self.live()?.get(level.attachments(), holder, read)
    }

    /// Whether `holder` currently carries a value in `level`; never inserts the default.
    pub fn has_in(
        &self,
        level: &mut dyn LevelRandomHost,
        holder: Holder,
    ) -> Result<bool, AttachmentError> {
        self.live()?.has(level.attachments(), holder)
    }

    /// [`Self::get_existing`] through a [`LevelRandomHost`].
    pub fn get_existing_in<R>(
        &self,
        level: &mut dyn LevelRandomHost,
        holder: Holder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentError> {
        self.live()?.get_existing(level.attachments(), holder, read)
    }

    /// Replace a value on `holder` in `level`.
    pub fn set_in(
        &self,
        level: &mut dyn LevelRandomHost,
        holder: Holder,
        value: T,
    ) -> Result<(), AttachmentError> {
        self.live()?.set(level.attachments(), holder, value)
    }

    /// [`Self::remove`] through a [`LevelRandomHost`].
    pub fn remove_in(
        &self,
        level: &mut dyn LevelRandomHost,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError>
    where
        T: Clone,
    {
        self.live()?.remove(level.attachments(), holder)
    }

    /// [`Self::sync_data`] through a [`LevelRandomHost`].
    pub fn sync_data_in(
        &self,
        level: &mut dyn LevelRandomHost,
        holder: Holder,
    ) -> Result<(), AttachmentError> {
        self.live()?.sync_data(level.attachments(), holder)
    }
}

/// Object-safe attachment registration and lookup operations on [`Registrar`].
///
/// Prefer the typed inherent methods [`Registrar::register_attachment_type`] and
/// [`Registrar::attachment_key`] when the attachment value type `T` is known.
pub trait AttachmentTypesRegistrar {
    /// Register an erased attachment descriptor owned by the current package.
    ///
    /// The ID must belong to a namespace owned or served by the registering package.
    /// A refusal fails installation
    /// even when the caller ignores this result.
    fn submit_attachment(
        &mut self,
        descriptor: ErasedAttachmentDescriptor,
    ) -> Result<RegisteredAttachment, String>;

    /// Look up an attachment already registered earlier in this registration pass by full
    /// `"namespace:name"` ID.
    fn lookup_attachment(&self, id: &str) -> Result<RegisteredAttachment, AttachmentError>;
}

impl dyn Registrar + '_ {
    /// Register a typed attachment descriptor and return its issued [`AttachmentKey<T>`].
    ///
    /// Mirrors NeoForge's `RegisterEvent.register` for `NeoForgeRegistries.Keys.ATTACHMENT_TYPES`:
    /// register the descriptor by its ID in a namespace the package owns or serves. Keep the
    /// returned key on the mod or Part; keys from prior
    /// installations are rejected with [`AttachmentError::StaleKey`]. A refusal fails
    /// installation even when ignored.
    pub fn register_attachment_type<T: Send + Sync + 'static>(
        &mut self,
        descriptor: AttachmentDescriptor<T>,
    ) -> Result<AttachmentKey<T>, String> {
        let id = descriptor.id();
        let record = self.submit_attachment(descriptor.into())?;
        record
            .key_for::<T>(id)
            .map_err(|error| format!("attachment {id}: {error:?}"))
    }

    /// Look up a typed [`AttachmentKey<T>`] registered earlier in this pass (for example by a
    /// dependency).
    ///
    /// Returns [`AttachmentError::Undeclared`] if `id` has not been registered yet and
    /// [`AttachmentError::TypeMismatch`] if `id` was registered with a different Rust type `T`.
    /// A custom registrar that returns a record for another ID is refused with
    /// [`AttachmentError::RegistrationMismatch`].
    pub fn attachment_key<T: Send + Sync + 'static>(
        &self,
        id: &str,
    ) -> Result<AttachmentKey<T>, AttachmentError> {
        self.lookup_attachment(id)?.key_for::<T>(id)
    }
}

/// Capture the installed descriptors for a client sync send pass. Invoke mod
/// codecs on the owning server thread before workers assemble pairing packets.
// hidden: engine-owned client attachment sync projection; not an authoring method.
#[doc(hidden)]
pub fn attachment_sync_projection() -> impl Fn(
    &JavaString,
    &JavaString,
    AttachmentSyncInput<'_>,
)
    -> Result<Option<Compound>, AttachmentSyncProjectionError>
+ Sync {
    let installed = installed();
    move |namespace, name, input| {
        thread_local! {
            static COMPONENTS: std::cell::RefCell<[String; 2]> = Default::default();
        }
        // Stored IDs are Java strings, the descriptor table uses Rust strings. Reuse
        // conversion buffers and release them before invoking a possibly reentrant codec.
        let record = COMPONENTS.with_borrow_mut(|[ns, nm]| {
            for (out, source) in [(&mut *ns, namespace), (&mut *nm, name)] {
                out.clear();
                for c in char::decode_utf16(source.0.iter().copied()) {
                    out.push(c.ok()?);
                }
            }
            installed?
                .attachment_types
                .get(ns.as_str())?
                .get(nm.as_str())
        });
        let Some(record) = record.filter(|record| {
            record.is_synced()
                && !record
                    .sync_panicked
                    .load(std::sync::atomic::Ordering::Acquire)
        }) else {
            return Ok(None);
        };
        record.project_sync(input)
    }
}

/// Resolve persistence and death-copy policy independently of a saved envelope.
// hidden: engine-owned respawn copy policy lookup.
#[doc(hidden)]
pub fn registered_clone_attachment_policy(
    namespace: &str,
    name: &str,
) -> Option<CloneAttachmentPolicy> {
    let record = installed()?.attachment_types.get(namespace)?.get(name)?;
    Some(if record.has_codec() {
        CloneAttachmentPolicy::Persistent {
            copy_on_death: record.policy().copy_on_death,
        }
    } else {
        CloneAttachmentPolicy::MemoryOnly
    })
}

/// Validate use against the installed owner's declaration.
// hidden: shared engine and host adapter validation, not an authoring method.
#[doc(hidden)]
pub fn validate_attachment_use(
    namespace: &str,
    name: &str,
    sync: bool,
) -> Result<(), AttachmentError> {
    // Engine-owned storage work has no package identity. Every mod dispatch
    // enters its owning package scope before reaching this boundary.
    let Some(package) = crate::content::crash::current_native_mod() else {
        return Ok(());
    };
    validate_declared_attachment(
        installed().map(|parts| &parts.attachment_types),
        package,
        namespace,
        name,
        sync,
    )
}

/// Require an operation prepared by this installation's exact typed descriptor.
// hidden: engine-owned attachment store descriptor check; not an authoring method.
#[doc(hidden)]
pub fn validate_attachment_key(write: &AttachmentStoreKey<'_>) -> Result<(), AttachmentError> {
    let (namespace, name) = write.id();
    let record = installed()
        .and_then(|parts| parts.attachment_types.get(namespace))
        .and_then(|types| types.get(name));
    match record {
        Some(record) if (record.match_key)(record.record.as_ref(), write) => Ok(()),
        // Runtime-only attachments used by the engine are not mod declarations.
        None if write.engine_owned() && crate::content::crash::current_native_mod().is_none() => {
            Ok(())
        }
        _ => Err(AttachmentError::StaleKey),
    }
}

// Owner policy: the owner's declaration suffices for all callers.
/// Check one id against collected declarations, before or after installation.
// hidden: engine-owned load validation; not an authoring operation.
#[doc(hidden)]
pub fn validate_declared_attachment(
    declarations: Option<&HashMap<&'static str, HashMap<&'static str, RegisteredAttachment>>>,
    caller: &str,
    namespace: &str,
    name: &str,
    sync: bool,
) -> Result<(), AttachmentError> {
    let Some(record) = declarations
        .and_then(|types| types.get(namespace))
        .and_then(|types| types.get(name))
    else {
        return Err(AttachmentError::Undeclared {
            package_id: caller.into(),
            attachment_id: format!("{namespace}:{name}"),
        });
    };
    if sync && !record.is_synced() {
        return Err(AttachmentError::InvalidSyncPolicy);
    }
    Ok(())
}

/// Build a `(RegisteredAttachment, AttachmentKey<T>)` pair for unit tests without a full
/// registration pass.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn registered_attachment_fixture<T: Send + Sync + 'static>(
    owner: &'static str,
    descriptor: AttachmentDescriptor<T>,
) -> (RegisteredAttachment, AttachmentKey<T>) {
    let record = check_attachment_descriptor(descriptor.into(), owner)
        .expect("fixture attachment descriptor must be valid");
    let key = record
        .downcast_key::<T>()
        .expect("fixture descriptor carries type T");
    (record, key)
}

/// Install a single attachment descriptor in the process-wide parts table for unit tests and
/// return its live [`AttachmentKey<T>`].
#[cfg(any(test, feature = "testing"))]
pub(crate) fn install_attachment_fixture<T: Send + Sync + 'static>(
    owner: &'static str,
    descriptor: AttachmentDescriptor<T>,
) -> AttachmentKey<T> {
    let (record, key) = registered_attachment_fixture(owner, descriptor);
    let namespace = record.namespace();
    let name = record.name();
    crate::parts::install_input(crate::parts::installation::InstalledInput {
        attachment_types: HashMap::from([(namespace, HashMap::from([(name, record)]))]),
        ..Default::default()
    });
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttachmentStore, Attachments, Compound, LevelHost, Tag};

    fn i32_codec() -> AttachmentCodec<i32> {
        AttachmentCodec {
            encode: |n| Ok([("v".into(), Tag::Int(*n))].into()),
            decode: |tag| match tag.get(&"v".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }
    }

    struct FakeLevel(Attachments);
    impl LevelHost for FakeLevel {
        fn block_entity_data(&self, _: crate::BlockEntityRef) -> Option<Compound> {
            None
        }
    }
    impl LevelRandomHost for FakeLevel {
        fn attachments(&mut self) -> &mut dyn AttachmentStore {
            &mut self.0
        }
    }

    #[test]
    fn registrar_reply_must_match_both_requested_id_and_type() {
        let (record, key) =
            registered_attachment_fixture("demo", AttachmentDescriptor::new("demo:a", || 1u32));
        assert_eq!(record.key_for::<u32>("demo:a"), Ok(key));
        assert_eq!(
            record.key_for::<i64>("demo:a"),
            Err(AttachmentError::TypeMismatch)
        );
        assert_eq!(
            record.key_for::<u32>("demo:b"),
            Err(AttachmentError::RegistrationMismatch)
        );
    }

    #[test]
    fn owner_declaration_allows_b_to_use_a_mana_and_names_missing_use() {
        let (record, _) =
            registered_attachment_fixture("a", AttachmentDescriptor::new("a:mana", || 0u32));
        let declarations = HashMap::from([("a", HashMap::from([("mana", record)]))]);
        assert_eq!(
            validate_declared_attachment(Some(&declarations), "b", "a", "mana", false),
            Ok(())
        );
        assert_eq!(
            validate_declared_attachment(Some(&declarations), "b", "a", "mana", true),
            Err(AttachmentError::InvalidSyncPolicy)
        );
        assert_eq!(
            validate_declared_attachment(Some(&declarations), "b", "a", "missing", false),
            Err(AttachmentError::Undeclared {
                package_id: "b".into(),
                attachment_id: "a:missing".into(),
            })
        );
    }

    #[test]
    fn descriptor_accepts_sync_without_persistence_codec_and_rejects_copy_on_death_without_codec() {
        let sync_only = AttachmentDescriptor::new("shared:mana", || 5i32).sync(i32_codec());
        assert!(!sync_only.has_codec());
        assert!(sync_only.is_synced());
        assert!(!sync_only.copy_on_death_enabled());
        let record = check_attachment_descriptor(sync_only.into(), "owner_mod")
            .expect("sync without persistence codec must be valid");
        assert_eq!(record.id(), "shared:mana");
        assert_eq!(record.namespace(), "shared");
        assert_eq!(record.name(), "mana");
        assert_eq!(record.owner(), "owner_mod");
        assert!(!record.has_codec());
        assert!(record.is_synced());
    }

    #[test]
    fn descriptor_refuses_duplicate_serializer_and_death_copy_before_serializer() {
        let descriptor = || AttachmentDescriptor::new("owner_mod:mana", || 0i32);
        let build = |descriptor: AttachmentDescriptor<i32>| {
            let descriptor: ErasedAttachmentDescriptor = descriptor.into();
            (descriptor.build)("owner_mod", "mana", "owner_mod").map(|_| ())
        };
        assert_eq!(
            build(descriptor().serialize(i32_codec()).serialize(i32_codec())),
            Err(AttachmentError::DuplicateSerializer)
        );
        assert_eq!(
            build(descriptor().copy_on_death().serialize(i32_codec())),
            Err(AttachmentError::InvalidCopyPolicy)
        );
        let valid = check_attachment_descriptor(
            descriptor().serialize(i32_codec()).copy_on_death().into(),
            "owner_mod",
        )
        .unwrap();
        assert!(valid.has_codec());
        assert!(valid.policy().copy_on_death);
    }

    #[test]
    fn attachment_key_detects_stale_keys_across_installations_even_with_same_id_and_type() {
        let first =
            install_attachment_fixture("mod_a", AttachmentDescriptor::new("mod_a:count", || 1i32));
        assert!(first.is_live());

        let mut host = crate::testing::FakeHost::default();
        assert_eq!(first.get(&mut host, Holder::Level, Clone::clone), Ok(1));
        first.set(&mut host, Holder::Level, 9).unwrap();
        assert_eq!(
            first.get_existing(&mut host, Holder::Level, |value| value.cloned()),
            Ok(Some(9))
        );

        let mut level = FakeLevel(Attachments::default());
        assert_eq!(first.get_in(&mut level, Holder::Level, Clone::clone), Ok(1));
        first.set_in(&mut level, Holder::Level, 4).unwrap();
        assert_eq!(first.remove_in(&mut level, Holder::Level), Ok(Some(4)));
        assert_eq!(first.has_in(&mut level, Holder::Level), Ok(false));

        // Reinstall with the exact same ID, owner, and Rust type: the first key must be stale.
        let second =
            install_attachment_fixture("mod_a", AttachmentDescriptor::new("mod_a:count", || 1i32));
        assert!(!first.is_live());
        assert!(second.is_live());
        assert_ne!(first, second);
        assert_eq!(
            first.get(&mut host, Holder::Level, Clone::clone),
            Err(AttachmentError::StaleKey)
        );
        assert_eq!(
            first.has(&mut host, Holder::Level),
            Err(AttachmentError::StaleKey)
        );
        assert_eq!(
            first.set(&mut host, Holder::Level, 2),
            Err(AttachmentError::StaleKey)
        );
        assert_eq!(
            first.remove(&mut host, Holder::Level),
            Err(AttachmentError::StaleKey)
        );
        assert_eq!(
            first.sync_data(&mut host, Holder::Level),
            Err(AttachmentError::StaleKey)
        );
        assert_eq!(
            first.get_in(&mut level, Holder::Level, Clone::clone),
            Err(AttachmentError::StaleKey)
        );
        assert_eq!(
            second.get_existing(&mut host, Holder::Level, |value| value.cloned()),
            Ok(Some(9))
        );
    }

    #[test]
    fn active_descriptor_projects_unread_saved_dual_codec_for_tracking_and_resync() {
        let disk_codec = AttachmentCodec {
            encode: |n: &i32| Ok([("disk".into(), Tag::Int(*n))].into()),
            decode: |tag: &Compound| match tag.get(&"disk".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        };
        let sync_v1 = AttachmentCodec {
            encode: |n: &i32| Ok([("net".into(), Tag::Int(*n + 100))].into()),
            decode: |tag: &Compound| match tag.get(&"net".into()) {
                Some(Tag::Int(n)) => Ok(*n - 100),
                _ => Err(AttachmentError::Decode),
            },
        };
        let sync_v2 = AttachmentCodec {
            encode: |n: &i32| Ok([("net_v2".into(), Tag::Int(*n * 10))].into()),
            decode: |_: &Compound| Err(AttachmentError::Decode),
        };

        let key_v1 = install_attachment_fixture(
            "mod_a",
            AttachmentDescriptor::new("mod_a:dual", || 0i32)
                .serialize(disk_codec)
                .sync(sync_v1),
        );
        let mut level = FakeLevel(Attachments::default());
        key_v1.set_in(&mut level, Holder::Level, 7).unwrap();

        let mut saved = level.0.try_to_nbt().unwrap();
        let Some(Tag::Compound(names)) = saved.get(&"mod_a".into()) else {
            panic!("saved namespace missing");
        };
        let Some(Tag::Compound(envelope)) = names.get(&"dual".into()) else {
            panic!("saved entry missing");
        };
        assert!(!envelope.contains_key(&"sync".into()));
        assert!(!envelope.contains_key(&"sync_value".into()));

        // Preserve an unregistered entry alongside the active dual-codec binding.
        saved.insert(
            "foreign".into(),
            Tag::Compound(
                [(
                    "opaque".into(),
                    Tag::Compound([("value".into(), Tag::Compound(Compound::new()))].into()),
                )]
                .into(),
            ),
        );

        let mut reloaded = FakeLevel(Attachments::from_nbt(saved.clone()));
        let expected_v1 = vec![sunrise_mod_runtime::attachments::AttachmentSync {
            namespace: "mod_a".into(),
            name: "dual".into(),
            value: Some([("net".into(), Tag::Int(107))].into()),
        }];
        let project =
            |namespace: &JavaString, name: &JavaString, input: AttachmentSyncInput<'_>| {
                attachment_sync_projection()(namespace, name, input).unwrap()
            };
        assert_eq!(reloaded.0.synced(project), expected_v1);
        key_v1.sync_data_in(&mut reloaded, Holder::Level).unwrap();
        assert_eq!(reloaded.0.take_sync_changes(project), expected_v1);

        // Migrating the active descriptor's sync codec re-projects the unread saved entry.
        let key_v2 = install_attachment_fixture(
            "mod_a",
            AttachmentDescriptor::new("mod_a:dual", || 0i32)
                .serialize(disk_codec)
                .sync(sync_v2),
        );
        let expected_v2 = vec![sunrise_mod_runtime::attachments::AttachmentSync {
            namespace: "mod_a".into(),
            name: "dual".into(),
            value: Some([("net_v2".into(), Tag::Int(70))].into()),
        }];
        assert_eq!(reloaded.0.synced(project), expected_v2);
        key_v2.sync_data_in(&mut reloaded, Holder::Level).unwrap();
        assert_eq!(reloaded.0.take_sync_changes(project), expected_v2);

        // Unsyncing the active descriptor stops sending while keeping saved NBT intact.
        let _unsynced = install_attachment_fixture(
            "mod_a",
            AttachmentDescriptor::new("mod_a:dual", || 0i32).serialize(disk_codec),
        );
        assert!(reloaded.0.synced(project).is_empty());
        assert_eq!(reloaded.0.try_to_nbt().unwrap(), saved);
    }

    #[test]
    fn unread_projection_refusals_are_reported_on_tracking_and_resync_without_stopping_other_keys()
    {
        let descriptor = |id| AttachmentDescriptor::new(id, || 0i32);
        let (bad, bad_key) = registered_attachment_fixture(
            "disk_owner",
            descriptor("shared:bad")
                .serialize(AttachmentCodec {
                    decode: |_| Err(AttachmentError::Decode),
                    ..i32_codec()
                })
                .sync(i32_codec()),
        );
        let (good, good_key) = registered_attachment_fixture(
            "other",
            descriptor("shared:good")
                .serialize(i32_codec())
                .sync(i32_codec()),
        );
        crate::parts::install_input(crate::parts::installation::InstalledInput {
            attachment_types: HashMap::from([(
                "shared",
                HashMap::from([("bad", bad), ("good", good)]),
            )]),
            ..Default::default()
        });
        let mut level = FakeLevel(Attachments::default());
        bad_key.set_in(&mut level, Holder::Level, 19).unwrap();
        good_key.set_in(&mut level, Holder::Level, 42).unwrap();
        level.0 = Attachments::from_nbt(level.0.try_to_nbt().unwrap());
        let mut failures = Vec::new();
        let mut project = |ns: &JavaString, name: &JavaString, input: AttachmentSyncInput<'_>| {
            match attachment_sync_projection()(ns, name, input) {
                Ok(value) => value,
                Err(error) => {
                    failures.push((ns.clone(), name.clone(), error));
                    None
                }
            }
        };
        let expected = [sunrise_mod_runtime::attachments::AttachmentSync {
            namespace: "shared".into(),
            name: "good".into(),
            value: Some([("v".into(), Tag::Int(42))].into()),
        }];
        assert_eq!(level.0.synced(&mut project), expected);
        bad_key.sync_data_in(&mut level, Holder::Level).unwrap();
        good_key.sync_data_in(&mut level, Holder::Level).unwrap();
        assert_eq!(level.0.take_sync_changes(&mut project), expected);
        assert!(level.0.take_sync_changes(&mut project).is_empty());
        assert_eq!(
            failures,
            [(); 2].map(|()| (
                JavaString::from("shared"),
                JavaString::from("bad"),
                AttachmentSyncProjectionError {
                    owner: "disk_owner",
                    failure: ProjectionFailure::Refused(AttachmentError::Decode)
                },
            ))
        );
    }

    #[test]
    fn sync_codec_panic_payload_cannot_unwind_into_next_key() {
        struct PanicsOnDrop;
        impl Drop for PanicsOnDrop {
            fn drop(&mut self) {
                panic!("sync payload destructor");
            }
        }
        let (broken, broken_key) = registered_attachment_fixture(
            "demo",
            AttachmentDescriptor::new("demo:broken", || 0i32).sync(AttachmentCodec {
                encode: |_| std::panic::panic_any(PanicsOnDrop),
                ..i32_codec()
            }),
        );
        let (healthy, healthy_key) = registered_attachment_fixture(
            "demo",
            AttachmentDescriptor::new("demo:healthy", || 0i32).sync(i32_codec()),
        );
        crate::parts::install_input(crate::parts::installation::InstalledInput {
            attachment_types: HashMap::from([(
                "demo",
                HashMap::from([("broken", broken), ("healthy", healthy)]),
            )]),
            ..Default::default()
        });
        let mut level = FakeLevel(Attachments::default());
        broken_key.set_in(&mut level, Holder::Level, 1).unwrap();
        healthy_key.set_in(&mut level, Holder::Level, 2).unwrap();
        let mut failures = Vec::new();
        let mut project = |ns: &JavaString, name: &JavaString, input: AttachmentSyncInput<'_>| {
            attachment_sync_projection()(ns, name, input).unwrap_or_else(|error| {
                failures.push(error);
                None
            })
        };
        let expected = [sunrise_mod_runtime::attachments::AttachmentSync {
            namespace: "demo".into(),
            name: "healthy".into(),
            value: Some([("v".into(), Tag::Int(2))].into()),
        }];
        assert_eq!(level.0.synced(&mut project), expected);
        assert_eq!(level.0.take_sync_changes(&mut project), expected);
        assert_eq!(
            failures,
            [AttachmentSyncProjectionError {
                owner: "demo",
                failure: ProjectionFailure::Panicked
            }]
        );
    }

    #[test]
    fn retiring_an_ordered_binding_cannot_unwind_the_sync_pass() {
        struct RetiredValue(i32);
        impl Drop for RetiredValue {
            fn drop(&mut self) {
                assert_ne!(self.0, 17, "the superseded binding panics on drop");
            }
        }
        let key = install_attachment_fixture(
            "demo",
            AttachmentDescriptor::new("demo:retired", || RetiredValue(0)).sync(AttachmentCodec {
                encode: |value| Ok([("v".into(), Tag::Int(value.0))].into()),
                decode: |_| Err(AttachmentError::Decode),
            }),
        );
        let mut level = FakeLevel(Attachments::default());
        key.set_in(&mut level, Holder::Level, RetiredValue(17))
            .unwrap();
        key.set_in(&mut level, Holder::Level, RetiredValue(37))
            .unwrap();
        let project = attachment_sync_projection();
        let changes = level
            .0
            .take_sync_changes(|ns, name, input| project(ns, name, input).unwrap());
        assert_eq!(
            changes
                .into_iter()
                .map(|change| change.value)
                .collect::<Vec<_>>(),
            [
                Some([("v".into(), Tag::Int(17))].into()),
                Some([("v".into(), Tag::Int(37))].into()),
            ]
        );
    }

    #[test]
    fn respawn_uses_registered_policy_even_if_saved_metadata_says_the_opposite() {
        let descriptor =
            || AttachmentDescriptor::new("shared:respawn", || 0i32).serialize(i32_codec());
        for saved_copy in [false, true] {
            let saved = if saved_copy {
                descriptor().copy_on_death()
            } else {
                descriptor()
            };
            let old = install_attachment_fixture("owner", saved);
            let mut level = FakeLevel(Attachments::default());
            old.set_in(&mut level, Holder::Level, 17).unwrap();
            let mut level = FakeLevel(Attachments::from_nbt(level.0.try_to_nbt().unwrap()));

            let current = if saved_copy {
                descriptor()
            } else {
                descriptor().copy_on_death()
            };
            let fresh = install_attachment_fixture("owner", current);
            let expected = (!saved_copy).then_some(17);
            let read_copy = |level: &FakeLevel| {
                let mut copy = FakeLevel(level.0.copy_on_death(registered_clone_attachment_policy));
                fresh.get_existing_in(&mut copy, Holder::Level, |value| value.cloned())
            };
            assert_eq!(read_copy(&level), Ok(expected));
            assert_eq!(
                fresh.get_existing_in(&mut level, Holder::Level, |value| value.cloned()),
                Ok(Some(17))
            );
            assert_eq!(read_copy(&level), Ok(expected));
        }
    }
}
