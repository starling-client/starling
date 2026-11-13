//! The `render_frame_post` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once a frame, in a world or not, right after the frame was rendered
    /// (NeoForge's `RenderFrameEvent.Post`). `elapsed` is the frame's time. It
    /// fires for a frame that was skipped too; `flip_frame` is the moment a
    /// frame was presented. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderFrameEvent.Post"
    )]
    fn render_frame_post();
}
