// hidden: proof for the game-side capture adapter.
#[doc(hidden)]
pub use crate::attachment_queries::AttachmentCaptureKey;
pub use entity::attachment_query::{
    AttachmentBatch, AttachmentIter, AttachmentQuery, AttachmentReadScope, AttachmentRow,
    AttachmentWriteToken,
};

pub use sunrise_mod_runtime::attachments::AttachmentRowError;

mod __private {
    mod game {
        pub use crate::entity::attachment_query::validate_attachment_write;
    }
}
