//! The callback's typed store: only encoded values cross the worker boundary.
use super::*;
use sunrise_core::nbt::{self, JavaString, Limits};
use sunrise_mod_runtime::attachments::ATTACHMENTS_KEY;

/// A job-local attachment transaction. No decoded value is shared with live state.
/// Use [`Self::get`] and [`Self::set`] with the ordinary typed attachment key.
/// The uncompressed disk NBT budget is 65,536 bytes per named `value` tag,
/// 64 container levels in that value, and 1,048,576 bytes for the entire
/// named attachment compound, including unknown data. Anything that would
/// exceed a limit or require truncation is refused. A codec or budget failure
/// refuses publication even if the listener ignores its error. Dispatch discards
/// that listener's edits so later listeners can still work on the accepted envelopes.
/// Each operation still runs independently and reports its own refusal.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ChunkDiskData {
    entries: Compound,
    failure: Option<ChunkDiskFailure>,
}

impl ChunkDiskData {
    /// Check the captured encoded envelopes, including unknown keys, before
    /// opening the transaction. This does not run any attachment codec.
    pub fn from_nbt(entries: Compound) -> Result<Self, ChunkDiskFailure> {
        validate(&entries)?;
        Ok(Self {
            entries,
            failure: None,
        })
    }

    /// Read the job's typed value, inserting its default if absent.
    pub fn get<T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &AttachmentType<T>,
    ) -> Result<T, ChunkDiskFailure> {
        self.edit(key, |store| key.get(store, Holder::Level))
    }

    /// Replace only the snapshot's value. Failed encoding leaves the original
    /// envelopes intact and refuses publication of the entire job.
    pub fn set<T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &AttachmentType<T>,
        value: T,
    ) -> Result<(), ChunkDiskFailure> {
        self.edit(key, |store| key.set(store, Holder::Level, value))
    }

    /// Remove a snapshot binding through its typed codec.
    pub fn remove<T: Clone + Send + Sync + 'static>(
        &mut self,
        key: &AttachmentType<T>,
    ) -> Result<Option<T>, ChunkDiskFailure> {
        self.edit(key, |store| key.remove(store, Holder::Level))
    }

    fn edit<T, R>(
        &mut self,
        key: &AttachmentType<T>,
        apply: impl FnOnce(&mut Attachments) -> Result<R, AttachmentError>,
    ) -> Result<R, ChunkDiskFailure> {
        let key_name = format!("{}:{}", key.namespace, key.name);
        let result = (|| {
            if key.codec.is_none() {
                return Err(failure(&key_name, "missing persistence codec"));
            }
            let mut store = Attachments::from_nbt(self.entries.clone());
            let value =
                apply(&mut store).map_err(|error| failure(&key_name, &format!("{error:?}")))?;
            let encoded = store.to_nbt();
            validate(&encoded)?;
            self.entries = encoded;
            Ok(value)
        })();
        result.map_err(|mut error: ChunkDiskFailure| {
            if let Some(call) = crate::content::crash::current_native_call() {
                error.mod_id = call.id.into();
                log::warn!(
                    "mod {} hook {} attachment {key_name} refused: {} (at {})",
                    call.id,
                    call.hook,
                    error.reason,
                    error.key
                );
            }
            self.failure.get_or_insert_with(|| error.clone());
            error
        })
    }

    /// Run a nested local savepoint. An explicit abort restores the prior
    /// envelopes; a codec or budget refusal still prevents the job from publishing.
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

    /// The first refusal of the active callback, or the job's first failure after dispatch.
    /// A refusal does not block later operations in this callback or in other callbacks.
    /// Use each operation's returned error to diagnose that operation.
    pub fn failure(&self) -> Option<&ChunkDiskFailure> {
        self.failure.as_ref()
    }

    pub(crate) fn attribute_failure(&mut self, error: ChunkDiskFailure) {
        self.failure = Some(error);
    }

    /// Publish a fully successful snapshot, or return its structured refusal.
    pub fn finish(self) -> Result<Compound, ChunkDiskFailure> {
        match self.failure {
            Some(error) => Err(error),
            None => Ok(self.entries),
        }
    }
}

fn failure(key: &str, reason: &str) -> ChunkDiskFailure {
    ChunkDiskFailure {
        mod_id: String::new(),
        key: key.into(),
        reason: reason.into(),
    }
}

/// Bytes of a disk NBT name, excluding its tag ID. Unlike vanilla's disk
/// fallback, reject names that would be replaced with an empty string.
fn name_bytes(name: &JavaString) -> Option<usize> {
    u16::try_from(name.modified_utf8_len())
        .ok()
        .map(|len| 2 + usize::from(len))
}

fn validate(entries: &Compound) -> Result<(), ChunkDiskFailure> {
    const TOTAL_EXCEEDED: &str = "chunk attachments exceed 1048576 bytes";
    const LONG_NAME: &str = "modified UTF-8 exceeds 65535 bytes";
    // The named `sunrise:attachments` compound: tag ID, u16-prefixed name,
    // and the closing TAG_End. Each namespace under it has the same framing.
    let mut remaining = 1_048_576 - (1 + 2 + ATTACHMENTS_KEY.len() + 1);
    let value_name = "value".into();
    for (namespace, names) in entries {
        let Tag::Compound(names) = names else {
            return Err(failure(
                &namespace.to_string_lossy(),
                "invalid attachment namespace",
            ));
        };
        let framing = name_bytes(namespace)
            .ok_or_else(|| failure(&namespace.to_string_lossy(), LONG_NAME))?;
        remaining = remaining
            .checked_sub(1 + framing + 1)
            .ok_or_else(|| failure(&namespace.to_string_lossy(), TOTAL_EXCEEDED))?;
        for (name, tag) in names {
            let refuse = |reason| {
                let key = format!("{}:{}", namespace.to_string_lossy(), name.to_string_lossy());
                failure(&key, reason)
            };
            let Tag::Compound(envelope) = tag else {
                return Err(refuse("invalid persistence envelope"));
            };
            let value = envelope
                .get(&value_name)
                .ok_or_else(|| refuse("missing encoded value"))?;

            // Both NBT writers use the same tag IDs, payload framing and mixed
            // list wrappers. NaN canonicalization doesn't change the length.
            // Use the strict writer so too-long strings cannot silently shrink;
            // add the disk-only root name to its tag ID and payload.
            nbt::write_network(
                value,
                Limits {
                    max_bytes: 65_536 - (2 + "value".len()),
                    max_nodes: usize::MAX,
                    max_depth: 64,
                },
            )
            .map_err(|error| {
                refuse(match error.reason {
                    "byte limit exceeded" => "encoded attachment exceeds 65536 bytes",
                    "depth limit exceeded" => "attachment depth exceeds 64",
                    reason => reason,
                })
            })?;

            let framing = name_bytes(name).ok_or_else(|| refuse(LONG_NAME))?;
            remaining = remaining
                .checked_sub(framing)
                .ok_or_else(|| refuse(TOTAL_EXCEEDED))?;
            let envelope_len = nbt::write_network(
                tag,
                Limits {
                    max_bytes: remaining,
                    max_nodes: usize::MAX,
                    // Root, attachment compound and namespace are outside this envelope.
                    max_depth: Limits::saved().max_depth - 3,
                },
            )
            .map_err(|error| {
                refuse(match error.reason {
                    "byte limit exceeded" => TOTAL_EXCEEDED,
                    reason => reason,
                })
            })?
            .len();
            remaining -= envelope_len;
        }
    }
    Ok(())
}
