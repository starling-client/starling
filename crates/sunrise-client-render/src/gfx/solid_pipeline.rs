/// A solid terrain pipeline without HDR output's lighting (terrain.wgsl's `HDR_LIGHTING`),
/// for the frames HDR is off, and with it where the display offers HDR output: the
/// lighting's code, even never run, slows the shader on Apple GPUs (~15% of the solid
/// pass at Retina sizes).
pub struct SolidPipeline {
    pub(super) plain: wgpu::RenderPipeline,
    pub(super) hdr: Option<wgpu::RenderPipeline>,
}

impl SolidPipeline {
    /// The pipeline for a frame with HDR output on (`hdr`) or off.
    pub fn get(&self, hdr: bool) -> &wgpu::RenderPipeline {
        match &self.hdr {
            Some(lit) if hdr => lit,
            _ => &self.plain,
        }
    }
}
