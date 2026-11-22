//! Frames in flight: the readbacks a submission starts, landing frames the submit thread
//! submitted, keeping offscreen frames at most two ahead of the GPU, and a screenshot's copy.

use super::{Renderer, ScreenshotRequest, copy_frame, submit};
use std::sync::atomic::{AtomicBool, Ordering};

/// The error of a wait that found the device lost.
pub(super) const LOST: &str = "the graphics device was lost";

/// `Device::poll`, unless the device is lost (`lost`, set by its loss callback). wgpu has no
/// error result for a poll that finds the device lost (it panics), so that panic is contained;
/// the callback has set `lost` by then. Any other panic goes on.
pub(crate) fn poll_live(
    device: &wgpu::Device,
    lost: &AtomicBool,
    poll: wgpu::PollType,
) -> Result<(), String> {
    contain_loss(lost, || {
        device.poll(poll).map(|_| ()).map_err(|e| e.to_string())
    })
}

/// `wait`, unless `lost` is set; a panic in it while `lost` is set is the loss, a panic with
/// `lost` clear goes on.
fn contain_loss(
    lost: &AtomicBool,
    wait: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if lost.load(Ordering::Relaxed) {
        return Err(LOST.into());
    }
    match crate::crash::contain(wait) {
        Ok(waited) => waited,
        Err(_) if lost.load(Ordering::Relaxed) => Err(LOST.into()),
        Err(message) => panic!("{message}"),
    }
}

impl Renderer {
    /// The frame's readbacks to start right after its submission (on the submit thread when
    /// the frame goes there): occlusion culling's results (`cull`) and GPU culling's
    /// statistics, then a pack-drawn light texture's copy while F3 charts it. GPU frames leave
    /// occlusion culling off, so a frame has one of the first two at most.
    pub(super) fn after_submit(&mut self) -> Option<submit::AfterSubmit> {
        let cull = self.cull.as_mut().and_then(|c| c.submitted());
        let gpu = self.gpu_culling.as_mut().and_then(|g| g.submitted());
        let lightmap = self.core_lightmap.as_mut().and_then(|l| l.submitted());
        let cull = cull.map(|c| Box::new(c) as submit::AfterSubmit);
        let gpu = gpu.map(|g| Box::new(g) as submit::AfterSubmit);
        chain(chain(cull, gpu), lightmap)
    }
    /// Frame `frame`'s statistics shown, unless a newer frame's are: a CPU frame's as it is
    /// drawn, a GPU frame's when its counts come back (`GpuCulling::collect`).
    pub(super) fn show_stats(&mut self, frame: u64, [sections, draws, calls]: [usize; 3]) {
        if frame > self.stats_frame {
            self.stats_frame = frame;
            (
                self.drawn_sections,
                self.section_draws,
                self.section_draw_calls,
            ) = (sections, draws, calls);
        }
    }
    /// Frames between the last one drawn and the one whose statistics are shown: 0 after a
    /// CPU frame, one to three in GPU frames, while their counts are on their way back.
    pub fn stats_lag(&self) -> u64 {
        self.frame_serial - self.stats_frame
    }
    /// Take in the frame the submit thread submitted: start its readbacks and, offscreen,
    /// keep at most two frames ahead of the GPU, as a frame submitted in place does. Nothing
    /// more once the device is lost: that submission is often where the loss shows. The
    /// readbacks come back through the next frame's poll (`draw_frame`).
    pub(super) fn land_frame(&mut self) {
        let Some(offscreen) = self.deferred.take() else {
            return;
        };
        let submission = self.queue.landed();
        if self.device_lost() {
            return;
        }
        if let Some(timer) = &mut self.gpu_timer {
            timer.submitted(&self.queue);
        }
        if let Some(Some(timer)) = Self::pass_timer(&mut self.pack_post, &mut self.native) {
            timer.submitted(&self.queue);
        }
        if let Some(timer) = &mut self.own_timing {
            timer.submitted(&self.queue);
        }
        if offscreen && let Some(submission) = submission {
            self.in_flight.push_back(submission);
            self.wait_frames();
        }
    }
    /// No swapchain paces offscreen frames: keep two on the GPU at most.
    pub fn wait_in_flight(&mut self) {
        self.land_frame();
        self.wait_frames();
    }
    /// The frame on the submit thread submitted: a surface reconfigure waits for the
    /// device and breaks if a frame is submitted while it waits.
    pub fn wait_submissions(&mut self) {
        self.queue.wait();
    }
    pub(super) fn wait_frames(&mut self) {
        while self.in_flight.len() > 2 {
            let index = self.in_flight.pop_front();
            // A frame that doesn't finish in time, or a lost device, isn't waited for again.
            let _ = poll_live(
                &self.device,
                &self.device_lost,
                wgpu::PollType::Wait {
                    submission_index: index,
                    timeout: Some(std::time::Duration::from_secs(5)),
                },
            );
        }
    }
    /// Take a screenshot request: queue the frame's copy, or report why the
    /// frame cannot be read back.
    pub(super) fn take_readback(
        &mut self,
        request: ScreenshotRequest,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Option<(ScreenshotRequest, wgpu::Buffer)> {
        // The copy and PNG writer both assume 8-bit pixels; a float surface would make
        // the row stride too small for the copy and abort in validation.
        if self.config.format.block_copy_size(None) == Some(4) {
            let size = [self.config.width, self.config.height];
            let format = self.config.format.remove_srgb_suffix();
            // The layered float frame maps the world to SDR, then adds the HUD as drawn.
            // Other 16-bit frames keep their clipped 8-bit copy.
            let copy = self
                .hdr
                .as_mut()
                .and_then(|hdr| {
                    hdr.sdr_screenshot(&self.device, encoder, format, request.size.is_some())
                })
                .or_else(|| {
                    (texture.format() != format && texture.format() == self.output.frame.format)
                        .then(|| {
                            self.readback
                                .unorm_copy(&self.device, encoder, texture, format)
                        })
                });
            let texture = copy.as_ref().unwrap_or(texture);
            Some((request, copy_frame(&self.device, encoder, texture, size)))
        } else {
            let error = format!(
                "screenshots need a 32-bit surface format, not {:?}",
                self.config.format
            );
            self.screenshot_result = Some(Err(error));
            None
        }
    }
}

/// `first`, then `then`, as one readback: a new box only when both are there.
fn chain(
    first: Option<submit::AfterSubmit>,
    then: Option<submit::AfterSubmit>,
) -> Option<submit::AfterSubmit> {
    match (first, then) {
        (Some(a), Some(b)) => Some(Box::new(move || {
            a();
            b();
        })),
        (a, None) => a,
        (None, b) => b,
    }
}

#[cfg(test)]
mod chain_tests {
    use super::{chain, submit};
    use std::sync::{Arc, Mutex};

    /// Two readbacks chained both run, the first first; one alone is itself; none is none.
    #[test]
    fn chained_readbacks_run_in_order_and_none_is_none() {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let read = |n: u8| -> Option<submit::AfterSubmit> {
            let ran = ran.clone();
            Some(Box::new(move || ran.lock().unwrap().push(n)))
        };
        chain(read(1), read(2)).expect("both")();
        chain(read(3), None).expect("the first")();
        chain(None, read(4)).expect("the second")();
        assert!(chain(None, None).is_none());
        assert_eq!(*ran.lock().unwrap(), [1, 2, 3, 4]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_device_is_not_waited_on() {
        let lost = AtomicBool::new(true);
        let result = contain_loss(&lost, || unreachable!("waited on a lost device"));
        assert_eq!(result, Err(LOST.into()));
    }

    #[test]
    fn a_panic_that_comes_with_the_loss_is_the_loss() {
        let lost = AtomicBool::new(false);
        let result = contain_loss(&lost, || {
            lost.store(true, Ordering::Relaxed);
            panic!("device lost")
        });
        assert_eq!(result, Err(LOST.into()));
    }

    #[test]
    #[should_panic(expected = "not a loss")]
    fn a_panic_on_a_live_device_goes_on() {
        let lost = AtomicBool::new(false);
        let _ = contain_loss(&lost, || panic!("not a loss"));
    }

    #[test]
    fn a_wait_on_a_live_device_returns_its_result() {
        let lost = AtomicBool::new(false);
        assert_eq!(contain_loss(&lost, || Ok(())), Ok(()));
        assert_eq!(
            contain_loss(&lost, || Err("timed out".into())),
            Err("timed out".into())
        );
    }
}
