pub use client::entity_render_attachments::EntityRenderAttachments;
mod __private {
    mod game {
        pub use crate::client::entity_render_attachments::capture_entity_render_attachments;
    }
}
pub use client::entity_renderers::{
    EntityRenderState, EntityRenderer, EntityRendererRegistry, EntityRenderers,
    ItemFrameRenderState, LivingRenderState, PaintingRenderState,
};
