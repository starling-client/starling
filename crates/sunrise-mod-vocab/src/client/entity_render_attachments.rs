//! Frozen, per-entity attachment data for the submission pass.
use crate::{AttachmentError, AttachmentType};
use sunrise_mod_runtime::attachments::{Attachments, snapshot::SyncedAttachmentSnapshot};

/// A general entity's last published generation of synced fields at render-state extraction.
/// A refused generation does not replace the published one. Clones share the captured
/// generation and do not see later sync updates.
/// Snapshots compare by attachment key and canonical NBT value, regardless of generation
/// identity, compound iteration order, or which typed values have been read.
// no wire: native render input; attachments keep their separate disk and sync codecs.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityRenderAttachments {
    data: SyncedAttachmentSnapshot,
}

impl EntityRenderAttachments {
    /// Borrows the synced value of `key`, or returns `Ok(None)` if the key is absent from this
    /// generation. New snapshots can show updates or removals while the viewer is held during
    /// a proxy handoff, without waiting for placement or requesting a resync. Never inserts a
    /// default or reads the live world. The first typed read of an undecoded binding may allocate
    /// when it decodes the value using the key's codec.
    pub fn get<T: Send + Sync + 'static>(
        &self,
        key: &AttachmentType<T>,
    ) -> Result<Option<&T>, AttachmentError> {
        crate::parts::validate_attachment_use(key.namespace, key.name, key.sync)?;
        self.data.get(key)
    }
}

/// Capture the synced bindings at the engine's publication boundary.
pub fn capture_entity_render_attachments(attachments: &Attachments) -> EntityRenderAttachments {
    EntityRenderAttachments {
        data: SyncedAttachmentSnapshot::capture(attachments),
    }
}
