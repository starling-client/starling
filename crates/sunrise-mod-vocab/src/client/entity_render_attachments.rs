//! Frozen, per-entity attachment data for the submission pass.
use crate::{AttachmentError, AttachmentKey};
use sunrise_mod_runtime::attachments::{Attachments, snapshot::SyncedAttachmentSnapshot};

/// A general entity's last published generation of synced fields at render-state extraction.
/// A refused generation does not replace the published one. Clones share the captured
/// generation and do not see later sync updates.
/// Snapshots compare by attachment key and canonical NBT value, regardless of generation
/// identity within the same installation, compound iteration order, or which typed values
/// have been read.
// no wire: native render input; attachments keep their separate disk and sync codecs.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityRenderAttachments {
    epoch: Option<u64>,
    data: SyncedAttachmentSnapshot,
}

impl EntityRenderAttachments {
    /// Borrows the synced value of `key`, or returns `Ok(None)` if the key is absent from this
    /// generation. Returns [`AttachmentError::StaleKey`] unless the key and capture belong to
    /// the current installation. Never inserts a default or reads the live world. The first
    /// typed read may allocate when it decodes the value using the key's sync codec.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentKey<T>,
    ) -> Result<Option<&T>, AttachmentError> {
        let parts = crate::parts::installed()
            .filter(|parts| Some(parts.epoch) == self.epoch)
            .ok_or(AttachmentError::StaleKey)?;
        let checked = key.live_in(parts)?;
        if !checked.is_synced() {
            return Err(AttachmentError::InvalidSyncPolicy);
        }
        self.data.get_checked(checked, parts.epoch)
    }
}

/// Capture the synced bindings at the engine's publication boundary.
pub fn capture_entity_render_attachments(attachments: &Attachments) -> EntityRenderAttachments {
    EntityRenderAttachments {
        epoch: crate::parts::installed().map(|parts| parts.epoch),
        data: SyncedAttachmentSnapshot::capture(attachments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttachmentCodec, AttachmentDescriptor, Tag};
    use std::collections::HashMap;
    use sunrise_mod_runtime::attachments::AttachmentSync;

    fn i32_codec() -> AttachmentCodec<i32> {
        AttachmentCodec {
            encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |tag| match tag.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }
    }

    #[test]
    fn sync_only_key_reads_from_snapshot_while_unsynced_and_stale_keys_refuse() {
        let (glow_rec, glow) = crate::parts::registered_attachment_fixture(
            "render_test",
            AttachmentDescriptor::new("render_test:glow", || 0i32).sync(i32_codec()),
        );
        let (local_rec, local) = crate::parts::registered_attachment_fixture(
            "render_test",
            AttachmentDescriptor::new("render_test:local", || 0i32),
        );
        crate::parts::install_input(crate::parts::installation::InstalledInput {
            attachment_types: HashMap::from([(
                "render_test",
                HashMap::from([("glow", glow_rec), ("local", local_rec)]),
            )]),
            ..Default::default()
        });

        let mut store = Attachments::default();
        store.apply_sync(AttachmentSync {
            namespace: "render_test".into(),
            name: "glow".into(),
            value: Some([("n".into(), Tag::Int(19))].into()),
        });
        let snapshot = capture_entity_render_attachments(&store);
        assert_eq!(snapshot.get(&glow), Ok(Some(&19)));
        assert_eq!(
            snapshot.get(&local),
            Err(AttachmentError::InvalidSyncPolicy)
        );

        let empty = capture_entity_render_attachments(&Attachments::default());
        assert_eq!(empty.get(&glow), Ok(None));

        let fresh = crate::parts::install_attachment_fixture(
            "render_test",
            AttachmentDescriptor::new("render_test:glow", || 0i32).sync(AttachmentCodec {
                decode: |tag| (i32_codec().decode)(tag).map(|n| n + 100),
                ..i32_codec()
            }),
        );
        let recaptured = capture_entity_render_attachments(&store);
        assert_eq!(snapshot.get(&glow), Err(AttachmentError::StaleKey));
        assert_eq!(snapshot.get(&fresh), Err(AttachmentError::StaleKey));
        assert_eq!(recaptured.get(&glow), Err(AttachmentError::StaleKey));
        assert_eq!(recaptured.get(&fresh), Ok(Some(&119)));
    }
}
