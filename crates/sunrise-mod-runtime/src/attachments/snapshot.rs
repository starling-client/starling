//! Read-only generations of a holder's synced bindings, detached from the holder itself.
use super::{
    AttachmentEntry, AttachmentError, AttachmentKeyView, AttachmentType, Attachments,
    CheckedAttachmentType, index::AttachmentKey,
};
use std::{fmt, sync::Arc};

type Binding = (AttachmentKey, Arc<AttachmentEntry>);

/// A whole generation in attachment key order. Clones share the same backing slice.
// no wire: in-process shared bindings; attachment sync owns their network codec.
#[derive(Clone, Default)]
pub struct SyncedAttachmentSnapshot(Arc<[Binding]>);

impl SyncedAttachmentSnapshot {
    /// Capture every synced binding on `holder` in attachment key order, sharing each
    /// immutable [`AttachmentEntry`] directly without re-encoding decoded values.
    pub fn capture(holder: &Attachments) -> Self {
        let entries: Vec<Binding> = holder
            .entries
            .iter()
            .filter(|(_, binding)| binding.synced_value().is_some())
            .map(|(key, binding)| (key.clone(), Arc::clone(binding)))
            .collect();
        Self(entries.into())
    }

    /// Read a captured binding, without inserting a default or consulting its old holder.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentType<T>,
    ) -> Result<Option<&T>, AttachmentError> {
        let lookup = (key.namespace, key.name);
        self.0
            .binary_search_by(|(key, _)| {
                (key as &dyn AttachmentKeyView).cmp(&lookup as &dyn AttachmentKeyView)
            })
            .ok()
            .map(|i| self.0[i].1.read(key))
            .transpose()
    }

    /// Read a captured binding with an issued type, scoping cached decodes to its installation.
    pub fn get_checked<T: Send + Sync + 'static>(
        &self,
        key: &CheckedAttachmentType<T>,
        epoch: u64,
    ) -> Result<Option<&T>, AttachmentError> {
        let lookup = (key.namespace(), key.name());
        self.0
            .binary_search_by(|(key, _)| {
                (key as &dyn AttachmentKeyView).cmp(&lookup as &dyn AttachmentKeyView)
            })
            .ok()
            .map(|i| self.0[i].1.read_in_installation(key.as_type(), Some(epoch)))
            .transpose()
    }
}

impl fmt::Debug for SyncedAttachmentSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncedAttachmentSnapshot")
            .field("fields", &self.0.len())
            .finish()
    }
}

impl PartialEq for SyncedAttachmentSnapshot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.0.len() == other.0.len()
                && self.0.iter().zip(other.0.iter()).all(
                    |((key, value), (other_key, other_value))| {
                        key == other_key
                            && (Arc::ptr_eq(value, other_value)
                                || value.synced_value() == other_value.synced_value())
                    },
                ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{AttachmentCodec, AttachmentSync, Holder};
    use sunrise_core::nbt::{Compound, Tag};

    static FUEL: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "fuel",
        default: || 99,
        codec: None,
        copy_on_death: false,
        sync_codec: Some(AttachmentCodec {
            encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |tag| match tag.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
    };

    fn sync(holder: &mut Attachments, key: &str, value: Option<Compound>) {
        holder.apply_sync(AttachmentSync {
            namespace: "test".into(),
            name: key.into(),
            value,
        });
    }

    #[test]
    fn a_retained_generation_reads_the_old_binding_after_replacement_and_removal() {
        let mut holder = Attachments::default();
        let value = |n| Some([("n".into(), Tag::Int(n))].into());
        sync(&mut holder, "fuel", value(7));
        let first = SyncedAttachmentSnapshot::capture(&holder);
        assert!(Arc::ptr_eq(
            &first.0[0].1,
            holder.entry("test", "fuel").unwrap().unwrap()
        ));
        let frame = first.clone();
        assert!(Arc::ptr_eq(&frame.0, &first.0));

        sync(&mut holder, "fuel", value(11));
        let next = SyncedAttachmentSnapshot::capture(&holder);
        sync(&mut holder, "fuel", None);
        assert_eq!(
            SyncedAttachmentSnapshot::capture(&holder).get(&FUEL),
            Ok(None)
        );
        // Read cold, after the live holder has lost both values.
        assert_eq!(frame.get(&FUEL), Ok(Some(&7)));
        assert_eq!(next.get(&FUEL), Ok(Some(&11)));
    }

    #[test]
    fn value_equality_changes_only_with_keys_or_canonical_nbt() {
        let mut a = Attachments::default();
        let mut b = Attachments::default();
        let value = |reverse: bool, bits| {
            // Aa and BB collide. The compounds need not traverse them in the same order.
            let mut tags = [
                ("Aa", Tag::Float(bits)),
                ("BB", Tag::Int(2)),
                ("n", Tag::Int(7)),
            ];
            if reverse {
                tags.swap(0, 1);
            }
            Some(tags.map(|(k, v)| (k.into(), v)).into())
        };
        sync(&mut a, "last", Some(Compound::new()));
        sync(&mut a, "fuel", value(false, 0x7fc0_0042));
        sync(&mut b, "fuel", value(true, 0x7fc0_0042));
        sync(&mut b, "last", Some(Compound::new()));
        let a = SyncedAttachmentSnapshot::capture(&a);
        assert_eq!(a.get(&FUEL), Ok(Some(&7)));
        assert_eq!(
            a,
            SyncedAttachmentSnapshot::capture(&b),
            "neither iteration order nor warm reads count"
        );
        sync(&mut b, "fuel", value(true, 0x7fc0_0043));
        assert_ne!(
            a,
            SyncedAttachmentSnapshot::capture(&b),
            "even a changed floating-point bit counts"
        );
        sync(&mut b, "fuel", value(true, 0x7fc0_0042));
        sync(&mut b, "last", None);
        sync(&mut b, "next", Some(Compound::new()));
        assert_ne!(
            a,
            SyncedAttachmentSnapshot::capture(&b),
            "equal values under different keys do not count"
        );
    }

    #[test]
    fn snapshots_keep_replica_bytes_after_promotion_but_skip_local_values() {
        static CODEC: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "fuel",
            default: || 0,
            codec: Some(AttachmentCodec {
                encode: |n| Ok([("disk".into(), Tag::Int(*n))].into()),
                decode: |_| panic!("render snapshot must use sync_codec, not persistence codec"),
            }),
            copy_on_death: false,
            sync_codec: Some(AttachmentCodec {
                encode: |n| Ok([("n".into(), Tag::Int(n / 10))].into()),
                decode: |c| match c.get(&"n".into()) {
                    Some(Tag::Int(n)) => Ok(*n * 10),
                    _ => Err(AttachmentError::Decode),
                },
            }),
        };
        let mut holder = Attachments::default();
        CODEC.set(&mut holder, Holder::Level, 27).unwrap();
        let capture = |holder: &Attachments| SyncedAttachmentSnapshot::capture(holder);
        assert_eq!(capture(&holder).get(&CODEC), Ok(None));

        sync(
            &mut holder,
            "fuel",
            Some([("n".into(), Tag::Int(2))].into()),
        );
        assert_eq!(CODEC.get_existing(&mut holder, Holder::Level), Ok(Some(20)));
        let frame = capture(&holder);
        assert_eq!(frame.get(&CODEC), Ok(Some(&20)));
        let checked = CheckedAttachmentType::new(AttachmentType {
            namespace: "test",
            name: "fuel",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync_codec: CODEC.sync_codec,
        })
        .unwrap();
        assert_eq!(frame.get_checked(&checked, 1), Ok(Some(&20)));
    }
}
