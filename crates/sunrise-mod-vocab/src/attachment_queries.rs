//! Typed host attachment snapshot queries.
#[cfg(test)]
#[path = "attachment_queries/tests.rs"]
mod tests;
use crate::*;
/// Engine-only proof of a live typed key for the requested capture bucket.
// hidden: the engine builds it from a checked key inside `AttachmentQueries`; mods never name it.
// exhaustive: its fields are private and `new` is crate-private.
#[doc(hidden)]
// no wire: a borrowed proof created only from a validated live key.
pub struct AttachmentCaptureKey<'a> {
    namespace: &'a str,
    name: &'a str,
}

impl<'a> AttachmentCaptureKey<'a> {
    pub(crate) fn new<T>(
        checked: &'a sunrise_mod_runtime::attachments::CheckedAttachmentType<T>,
    ) -> Self
    where
        T: Send + Sync + 'static,
    {
        Self {
            namespace: checked.namespace(),
            name: checked.name(),
        }
    }

    pub fn id(&self) -> (&str, &str) {
        (self.namespace, self.name)
    }
}

/// Read queries available on either host side.
pub trait AttachmentQueries: Host {
    /// Capture an unbound batch with a live key. The batch can later be bound
    /// to that attachment's currently issued key; the captured membership is unchanged.
    fn capture<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentKey<T>,
    ) -> Result<AttachmentBatch, AttachmentError> {
        self.attachment_query(&AttachmentCaptureKey::new(key.live()?))
    }

    /// Capture a SNAPSHOT of membership and value bindings in the current dimension.
    /// Later changes appear only in fresh queries. This is a residency view: an
    /// entity removed but not yet swept may still appear; stale writes fail.
    /// Construction never clones or decodes values; iteration decodes lazily.
    /// Use this snapshot in process; it is not an attachment save or sync record.
    /// A capture inside a transaction is usable only while that frame is current.
    /// Refuses keys from an earlier installation with [`AttachmentError::StaleKey`].
    fn query<'key, T: Send + Sync + 'static>(
        &self,
        key: &'key AttachmentKey<T>,
    ) -> Result<AttachmentQuery<'key, T>, AttachmentError> {
        let (checked, epoch) = key.live_with_epoch()?;
        self.attachment_query(&AttachmentCaptureKey::new(checked))?
            .query_checked(checked, epoch)
    }
}
impl<H: Host + ?Sized> AttachmentQueries for H {}

/// Explicit write-through queries, available only on server hosts.
pub trait AttachmentQueriesMut: ServerHost {
    /// Capture a SNAPSHOT, not mutable references into the live store.
    /// Set/remove join the caller's journal; no implicit transaction or Drop commit
    /// occurs. Later changes require a fresh query, and stale writes fail.
    /// A capture inside a transaction is usable only while that frame is current.
    /// Refuses keys from an earlier installation with [`AttachmentError::StaleKey`].
    fn query_mut<'key, T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &'key AttachmentKey<T>,
    ) -> Result<AttachmentQueryMut<'key, T>, AttachmentError> {
        Ok(AttachmentQueryMut {
            query: self.query(key)?,
            key,
        })
    }
}
impl<H: ServerHost + ?Sized> AttachmentQueriesMut for H {}

// no wire: retains a native snapshot and a borrowed attachment key.
/// A server write-through snapshot. Values and membership remain captured.
#[non_exhaustive]
pub struct AttachmentQueryMut<'key, T: 'static> {
    query: AttachmentQuery<'key, T>,
    key: &'key AttachmentKey<T>,
}
impl<T: Clone + Send + Sync + 'static> AttachmentQueryMut<'_, T> {
    /// Create a fallible cursor over writable rows, decoding saved entries only when visited.
    pub fn iter(&self) -> AttachmentIterMut<'_, T> {
        AttachmentIterMut {
            rows: self.query.iter(),
            key: self.key,
        }
    }
}

/// A fallible cursor borrowing the snapshot of an [`AttachmentQueryMut`].
// no wire: borrows an in-process snapshot and its attachment key.
#[non_exhaustive]
pub struct AttachmentIterMut<'q, T: 'static> {
    rows: AttachmentIter<'q, T>,
    key: &'q AttachmentKey<T>,
}

impl<'q, T: Clone + Send + Sync + 'static> AttachmentIterMut<'q, T> {
    /// Return the next writable row, or `Ok(None)` at the end while in scope.
    /// [`AttachmentRowError::OutsideTransaction`] leaves the cursor unchanged.
    /// A value error consumes only its own row.
    pub fn try_next(&mut self) -> Result<Option<AttachmentRowMut<'q, T>>, AttachmentRowError> {
        Ok(self
            .rows
            .try_next()?
            .map(|row| AttachmentRowMut { row, key: self.key }))
    }
}

// no wire: borrowed runtime attachment rows retain storage lifetimes and write authority.
/// A borrowed snapshot row with explicit journaled replacement/removal.
#[non_exhaustive]
pub struct AttachmentRowMut<'q, T: 'static> {
    row: AttachmentRow<'q, T>,
    key: &'q AttachmentKey<T>,
}
fn write_error(error: AttachmentError) -> AttachmentRowError {
    match error {
        AttachmentError::MissingHolder => AttachmentRowError::StaleHolder,
        AttachmentError::HolderBusy => AttachmentRowError::HolderBusy,
        AttachmentError::UnsupportedHolder(kind) => AttachmentRowError::UnsupportedHolder(kind),
        error => AttachmentRowError::Value(error),
    }
}
impl<T: Clone + Send + Sync + 'static> AttachmentRowMut<'_, T> {
    /// The captured scoped identity.
    pub fn holder(&self) -> AttachmentHolder {
        self.row.holder
    }
    /// Lend the captured value; interior mutability in `T` is not deep-frozen.
    /// Pass `host.attachment_read_scope()?` for captures made in a transaction. The loan
    /// borrows the original host and refuses unless the capturing frame is current.
    pub fn with_value<R>(
        &self,
        scope: AttachmentReadScope<'_>,
        f: impl FnOnce(&T) -> R,
    ) -> Result<R, AttachmentRowError> {
        self.row.with_value(scope, f)
    }
    fn holder_for_write(&self, host: &dyn ServerHost) -> Result<Holder, AttachmentRowError> {
        host.attachment_validate_write(self.row.write_token())?;
        Ok(match self.row.holder {
            AttachmentHolder::Entity(entity) => Holder::Entity(entity.id()),
            AttachmentHolder::Player(player) => Holder::Player(player.uuid()),
            AttachmentHolder::Chunk(chunk) => Holder::Chunk(chunk.pos()),
            AttachmentHolder::BlockEntity(entity) => Holder::BlockEntity(entity.position),
            AttachmentHolder::Level(_) => Holder::Level,
        })
    }
    /// Replace through the ordinary write path. Refuses with `Unavailable` outside
    /// the world phase, `OutsideTransaction` when the captured frame is not current,
    /// or `StaleHolder` when the holder's residency no longer matches.
    pub fn set(&self, host: &mut dyn ServerHost, value: T) -> Result<(), AttachmentRowError> {
        let holder = self.holder_for_write(host)?;
        self.key
            .live()
            .and_then(|checked| checked.set(host.attachments(), holder, value))
            .map_err(write_error)
    }
    /// Remove through the ordinary write path, with the same scope, phase and
    /// residency refusals as [`Self::set`].
    pub fn remove(&self, host: &mut dyn ServerHost) -> Result<Option<T>, AttachmentRowError> {
        let holder = self.holder_for_write(host)?;
        self.key
            .live()
            .and_then(|checked| checked.remove(host.attachments(), holder))
            .map_err(write_error)
    }
}
