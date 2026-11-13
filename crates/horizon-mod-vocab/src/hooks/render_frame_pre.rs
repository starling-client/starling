//! The `render_frame_pre` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Once a frame, in a world or not, after Horizon's extras and just before
    /// the frame is rendered (NeoForge's `RenderFrameEvent.Pre`). `elapsed` is
    /// the frame's time. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderFrameEvent.Pre"
    )]
    fn render_frame_pre();
}
