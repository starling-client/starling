//! Typed attachment descriptors, registration records, issued keys, and declaration validation.
pub use crate::parts::{
    AttachmentDescriptor, AttachmentKey, ErasedAttachmentDescriptor, RegisteredAttachment,
};
mod __private {
    pub use crate::parts::{
        AttachmentSyncProjectionError, attachment_sync_projection, check_attachment_descriptor,
        registered_clone_attachment_policy, validate_attachment_key, validate_attachment_use,
        validate_declared_attachment,
    };
}
