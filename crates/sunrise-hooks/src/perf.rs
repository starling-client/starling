//! Performance: open and share a saved performance capture, with none registered the game offers
//! neither; and each rendered frame's times ([`crate::Hooks::frame_timed`]).
use std::path::Path;

pub trait Captures: Sync {
    /// The viewer page, for the Open link.
    fn viewer(&self) -> &str;
    /// Serves `file` once on 127.0.0.1 and opens the viewer on it.
    fn open(&self, file: &Path) -> Result<(), String>;
    /// Uploads `file`; the share link.
    fn share(&self, file: &Path) -> Result<String, String>;
}

/// One frame the window rendered ([`crate::Hooks::frame_timed`]), in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FrameTimes {
    /// From the previous frame to this one.
    pub frame: f32,
    /// The game thread's part: the frame without its wait for the GPU.
    pub cpu: f32,
    /// The GPU's, of the last frame it timed; `None` where the device times no frame.
    pub gpu: Option<f32>,
    /// The world's resolution scale in percent while Dynamic Resolution moves it.
    pub dynamic_scale: Option<u32>,
}

impl FrameTimes {
    pub fn new(frame: f32, cpu: f32, gpu: Option<f32>, dynamic_scale: Option<u32>) -> Self {
        Self {
            frame,
            cpu,
            gpu,
            dynamic_scale,
        }
    }
}
