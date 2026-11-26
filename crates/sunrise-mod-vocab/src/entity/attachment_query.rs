//! Owned snapshot data of attachment queries.
use crate::*;
use std::sync::Mutex;
use sunrise_mod_runtime::attachments::{self as runtime, index};

/// An immutable borrow of a host for reading attachments captured in its transaction.
/// Get it from `host.attachment_read_scope()?` or `level.attachment_read_scope()`.
/// While it is in use the host cannot open or close a transaction. The default
/// value can be used for captures made outside a transaction.
// no wire: borrows the capturing host's frame tracker.
#[derive(Clone, Copy, Default)]
#[non_exhaustive]
pub struct AttachmentReadScope<'host>(pub(crate) Option<&'host runtime::AttachmentQueryScopes>);

/// A SNAPSHOT of membership and value bindings in the host's current dimension.
/// Later changes require a fresh query. This is a residency view: removed but
/// unswept entities may appear, and stale writes fail. Acquisition neither clones values nor
/// decodes saved payloads; iteration decodes lazily. A batch captured in a transaction
/// can only be used while that frame is current, on its owning thread. Keep the batch in
/// process; use the attachment type's codec for saving or syncing individual values.
///
/// ```compile_fail
/// use mod_api::{AttachmentBatch, wire::Writer};
/// fn encode_batch(batch: &AttachmentBatch) {
///     Writer::new().put(batch); // snapshots have no Wire encoding
/// }
/// ```
// no wire: retains all captured bindings, including values that have no persistence codec.
#[derive(Clone)]
#[non_exhaustive]
pub struct AttachmentBatch(runtime::AttachmentBatch<AttachmentHolder>);
impl From<runtime::AttachmentBatch<AttachmentHolder>> for AttachmentBatch {
    fn from(batch: runtime::AttachmentBatch<AttachmentHolder>) -> Self {
        Self(batch)
    }
}
impl AttachmentBatch {
    /// Bind an issued attachment key without eagerly decoding or cloning any value.
    /// This view borrows only the key, never the host. An unbound batch can be bound
    /// after reinstallation with the current key for its captured bucket; a query that
    /// was already bound remains tied to the installation that issued its key.
    /// Refuses keys from an earlier installation with [`AttachmentError::StaleKey`], or a key
    /// for a different captured bucket with [`AttachmentError::TypeMismatch`].
    pub fn query<'key, T: Send + Sync + 'static>(
        self,
        key: &'key AttachmentKey<T>,
    ) -> Result<AttachmentQuery<'key, T>, AttachmentError> {
        let (checked, epoch) = key.live_with_epoch()?;
        self.query_checked(checked, epoch)
    }

    pub(crate) fn query_checked<'key, T: Send + Sync + 'static>(
        self,
        checked: &'key runtime::CheckedAttachmentType<T>,
        epoch: u64,
    ) -> Result<AttachmentQuery<'key, T>, AttachmentError> {
        Ok(AttachmentQuery(
            self.0.query_checked(checked, epoch)?,
            epoch,
        ))
    }
}

/// A typed snapshot whose rows lend values from their captured bindings.
/// [`Self::get`] uses an expected O(1) holder probe before decode.
/// Missing or not yet synced holders lend `None`; no default is created.
/// `None` does not mean the holder is alive: the probe looks at captured membership
/// without a host liveness check.
/// Only scoped residency probes report `StaleHolder`. `get` and iteration report
/// [`AttachmentRowError::OutsideTransaction`] if the capture's frame is not current, or
/// [`AttachmentError::StaleKey`] as a value error after the installation changes.
// no wire: binds a borrowed key to an in-process snapshot.
#[non_exhaustive]
pub struct AttachmentQuery<'key, T: 'static>(
    runtime::AttachmentQuery<'key, AttachmentHolder, T>,
    u64,
);

fn check_epoch(epoch: u64) -> Result<(), AttachmentRowError> {
    if crate::parts::installed().is_some_and(|installed| installed.epoch == epoch) {
        Ok(())
    } else {
        Err(AttachmentRowError::Value(AttachmentError::StaleKey))
    }
}

impl<T: Send + Sync + 'static> AttachmentQuery<'_, T> {
    /// Create a fallible cursor in captured order, decoding values only as rows are visited.
    pub fn iter(&self) -> AttachmentIter<'_, T> {
        AttachmentIter(self.0.iter(), self.1)
    }
}

/// A fallible cursor borrowing the snapshot of an [`AttachmentQuery`].
// no wire: borrows a live snapshot and tracks a local cursor position.
#[non_exhaustive]
pub struct AttachmentIter<'q, T: 'static>(runtime::AttachmentIter<'q, AttachmentHolder, T>, u64);

impl<'q, T: Send + Sync + 'static> AttachmentIter<'q, T> {
    /// Return the next row, or `Ok(None)` at the end while in scope.
    /// [`AttachmentRowError::OutsideTransaction`] does not advance the cursor;
    /// the next call can resume once the capturing frame is current again.
    /// A value error consumes only its own row.
    pub fn try_next(&mut self) -> Result<Option<AttachmentRow<'q, T>>, AttachmentRowError> {
        check_epoch(self.1)?;
        Ok(self.0.try_next()?.map(|binding| AttachmentRow {
            holder: binding.holder,
            binding,
            epoch: self.1,
        }))
    }
}

impl<T: Send + Sync + 'static> AttachmentQuery<'_, T> {
    /// Lend a captured value or `None` without inserting a default. A capture made
    /// in a transaction requires the current frame and a read scope from its host.
    /// Stale installation, invalid scope and decode errors refuse before calling `read`.
    pub fn get<R>(
        &self,
        scope: AttachmentReadScope<'_>,
        holder: AttachmentHolder,
        read: impl FnOnce(Option<&T>) -> R,
    ) -> Result<R, AttachmentRowError> {
        check_epoch(self.1)?;
        self.0.get(scope.0, holder, read)
    }
}

/// One captured binding. The holder is copyable; the value must be used through a loan.
// no wire: borrows an in-process binding and its transaction scope.
#[non_exhaustive]
pub struct AttachmentRow<'q, T: 'static> {
    binding: runtime::AttachmentRowRef<'q, AttachmentHolder, T>,
    epoch: u64,
    /// The captured holder's identity.
    pub holder: AttachmentHolder,
}

impl<T: 'static> AttachmentRow<'_, T> {
    /// Lend the value for this call. To keep data beyond the loan, copy what you need.
    /// A transactional capture requires a read scope from the same host and the current
    /// frame. The scope borrows the host so the frame cannot change during the loan.
    /// The default scope is only for captures made outside a transaction.
    ///
    /// ```compile_fail,E0500
    /// use mod_api::{AttachmentRow, HostTransaction, ServerHost};
    /// fn transact_during_read(row: &AttachmentRow<'_, u32>, host: &mut dyn ServerHost) {
    ///     let _ = row.with_value(host.attachment_read_scope().unwrap(), |value| {
    ///         let _ = host.transaction(|_, _| Ok::<_, ()>(()));
    ///         *value
    ///     });
    /// }
    /// ```
    pub fn with_value<R>(
        &self,
        scope: AttachmentReadScope<'_>,
        f: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentRowError> {
        check_epoch(self.epoch)?;
        self.binding.with_value(scope.0, f)
    }

    /// The row's residency token. A token captured in a transaction is scoped to that frame.
    pub fn write_token(&self) -> AttachmentWriteToken {
        AttachmentWriteToken {
            owner: self.binding.write_token(),
            scope: self.binding.scope().cloned(),
        }
    }
}

/// A scoped residency identity issued from a captured row, not an access authorization
/// token. It cannot be reconstructed from bytes. Mods typically use the row's
/// `set` or `remove` method rather than passing the token to the host themselves.
///
/// ```compile_fail
/// use mod_api::{AttachmentWriteToken, wire::Reader};
/// let _ = Reader::decode::<AttachmentWriteToken>(&[]);
/// ```
// no wire: names a residency issued by the live attachment index.
#[derive(Clone)]
#[non_exhaustive]
pub struct AttachmentWriteToken {
    owner: index::AttachmentOwner<AttachmentHolder>,
    scope: Option<runtime::AttachmentQueryScope>,
}
/// The engine's write boundary: check the frame before checking dimension residency.
pub fn validate_attachment_write(
    token: &AttachmentWriteToken,
    index: Option<&Mutex<index::AttachmentIndex<AttachmentHolder>>>,
) -> Result<(), AttachmentRowError> {
    if let Some(scope) = &token.scope {
        scope.check()?;
    }
    index
        .ok_or(AttachmentRowError::StaleHolder)?
        .lock()
        .unwrap()
        .validate_owner(&token.owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn write_tokens_go_stale_with_their_residency() {
        let key = crate::parts::install_attachment_fixture(
            "test",
            AttachmentDescriptor::new("test:value", || 0u32),
        );
        let holder = AttachmentHolder::Entity(EntityRef::new(WorldEpoch::new(1), 42));
        let index = Arc::new(std::sync::Mutex::new(index::AttachmentIndex::default()));
        let mut values = runtime::Attachments::default();
        key.descriptor()
            .set(&mut values, runtime::Holder::Level, 7)
            .unwrap();
        values.bind_index(&index, holder);
        let batch = AttachmentBatch::from(runtime::AttachmentBatch::new(
            index.lock().unwrap().snapshot_named("test", "value"),
        ));
        let query = batch.query(&key).unwrap();
        let row = query.iter().try_next().unwrap().unwrap();
        assert_eq!(row.holder, holder);
        assert_eq!(row.with_value(Default::default(), |value| *value), Ok(7));
        let token = row.write_token();
        assert_eq!(validate_attachment_write(&token, Some(&index)), Ok(()));
        assert_eq!(
            validate_attachment_write(&token, Some(&Mutex::new(index::AttachmentIndex::default()))),
            Err(AttachmentRowError::StaleHolder)
        );
        index.lock().unwrap().unload(holder).into_result();
        assert_eq!(
            validate_attachment_write(&token, Some(&index)),
            Err(AttachmentRowError::StaleHolder)
        );
        let mut again = runtime::Attachments::default();
        key.descriptor()
            .set(&mut again, runtime::Holder::Level, 7)
            .unwrap();
        again.bind_index(&index, holder);
        assert_eq!(
            validate_attachment_write(&token, Some(&index)),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(row.with_value(Default::default(), |value| *value), Ok(7));
    }
}
