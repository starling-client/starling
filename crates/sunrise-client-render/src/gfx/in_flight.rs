//! Frames in flight: the readbacks a submission starts, landing frames the submit thread
//! submitted, keeping offscreen frames at most two ahead of the GPU, and a screenshot's copy.

use super::{Renderer, ScreenshotRequest, copy_frame, submit};
use std::collections::BTreeMap;
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
    /// statistics, then a pack-drawn light texture's copy while F3 charts it, then the copies of
    /// the mods' storage buffer readbacks. GPU frames leave
    /// occlusion culling off, so a frame has one of the first two at most.
    pub(super) fn after_submit(&mut self) -> Option<submit::AfterSubmit> {
        let cull = self.cull.as_mut().and_then(|c| c.submitted());
        let gpu = self.gpu_culling.as_mut().and_then(|g| g.submitted());
        let sourced = self
            .gpu_sourced_frame
            .then(|| self.sourced.submitted())
            .flatten()
            .map(|s| Box::new(s) as submit::AfterSubmit);
        let lightmap = self.core_lightmap.as_mut().and_then(|l| l.submitted());
        let cull = cull.map(|c| Box::new(c) as submit::AfterSubmit);
        let gpu = gpu.map(|g| Box::new(g) as submit::AfterSubmit);
        // A mod's readback copies, mapped as the frame that holds them is submitted.
        let mods = self.mod_gpu.readbacks_submitted();
        chain(chain(chain(cull, gpu), chain(sourced, lightmap)), mods)
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
    /// CPU frame; in GPU frames, the age of the last sample published (`collect_stats`),
    /// while its counts come back through the rings — a frame or a few, and longer when a
    /// pass's ring held no readback free for a frame (`copy_stats`) and a complete sample
    /// had to wait for a later one's.
    pub fn stats_lag(&self) -> u64 {
        self.frame_serial - self.stats_frame
    }
    /// The two readback halves joined as they come back (`draw_frame`), one frame's
    /// statistics published, never half of one (`collect_stats`).
    pub(super) fn collect_stats(
        &mut self,
        near: Vec<(u64, [usize; 3], bool)>,
        sourced: Vec<(u64, [usize; 3])>,
    ) -> Option<(u64, [usize; 3])> {
        collect_stats(&mut self.stats_halves, near, sourced)
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
        if let Some(Some(timer)) = Self::pass_timer(&mut self.pack_post) {
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

/// The held halves of GPU frames' statistics (`GpuCulling::collect`,
/// `Sourced::collect`) waiting for each other (`collect_stats`).
#[derive(Default)]
pub(crate) struct StatsHalves {
    /// The near table's held halves by full frame serial: only frames that drew
    /// sourced geometry wait (the flag rides the readback, `copy_stats`).
    near: BTreeMap<u64, [usize; 3]>,
    /// Sourced geometry's held halves by full frame serial.
    sourced: BTreeMap<u64, [usize; 3]>,
    /// The newest frame serial each pass's ring has delivered: a held half at or
    /// below it whose partner is missing can never be joined, because that
    /// partner's copy was skipped (`copy_stats`) — a ring delivers in frame order.
    near_delivered: u64,
    sourced_delivered: u64,
}

/// One frame's statistics published from its two readback halves as they come back, never
/// half of one: the near table's (`GpuCulling::collect`) and sourced geometry's
/// (`Sourced::collect`) each wait in `halves`, by full frame serial, for the partner of
/// its serial, as a half alone undercounts the frame — the other pass's ring held no
/// readback free for it (`copy_stats`), or its counts land a poll later. Which frames
/// drew sourced geometry rides the near half itself: those frames' near halves wait for
/// a sourced half, the others' are the whole frame and publish at once. A held half is
/// dropped once its partner can no longer come: the partner's pass has delivered a newer
/// frame, so the partner's own copy was skipped.
fn collect_stats(
    halves: &mut StatsHalves,
    near: Vec<(u64, [usize; 3], bool)>,
    sourced: Vec<(u64, [usize; 3])>,
) -> Option<(u64, [usize; 3])> {
    let mut published: Option<(u64, [usize; 3])> = None;
    for (frame, counts, drew_sourced) in near {
        halves.near_delivered = halves.near_delivered.max(frame);
        if drew_sourced {
            // Its sourced half can no longer come once the sourced ring passed it.
            if frame <= halves.sourced_delivered && !halves.sourced.contains_key(&frame) {
                continue;
            }
            halves.near.insert(frame, counts);
        } else {
            // The frame drew no sourced geometry, so it has no sourced half; no held
            // half pairs with it later. Its counts are the frame's: publish and clear.
            halves.near.remove(&frame);
            published = Some((frame, counts));
        }
    }
    for (frame, counts) in sourced {
        halves.sourced_delivered = halves.sourced_delivered.max(frame);
        if frame <= halves.near_delivered && !halves.near.contains_key(&frame) {
            continue;
        }
        halves.sourced.insert(frame, counts);
    }
    // The newest frame both halves delivered publishes; its counts sum, sourced
    // geometry's sections and draws added to the near table's (the calls are the
    // near pass's). A flag-false near half that published above stays ahead of it.
    let joined = halves
        .near
        .keys()
        .filter(|frame| halves.sourced.contains_key(*frame))
        .max()
        .copied();
    if let Some(frame) = joined {
        let [a, b, c] = halves.near.remove(&frame).expect("the near half");
        let [d, e, _] = halves.sourced.remove(&frame).expect("the sourced half");
        if published.as_ref().is_none_or(|&(shown, _)| frame > shown) {
            published = Some((frame, [a + d, b + e, c]));
        }
    }
    // A held half whose frame the other pass's ring has passed without delivering
    // can never be joined: that partner's copy was skipped. Rings deliver in frame
    // order, so no half below a pass's newest delivery gets a partner any more.
    halves
        .near
        .retain(|&frame, _| frame > halves.sourced_delivered);
    halves
        .sourced
        .retain(|&frame, _| frame > halves.near_delivered);
    published
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
mod stats_tests {
    use super::{StatsHalves, collect_stats};

    /// One frame's counts publish together: a half whose partner lands a poll later is
    /// held, not published half; halves of different frames never publish alone, and a
    /// half whose partner's copy was skipped goes once the partner's pass passes it.
    #[test]
    fn halves_of_one_frame_publish_together() {
        let mut halves = StatsHalves::default();
        // Sourced geometry's half lands a poll before the near table's: held, then both.
        assert_eq!(
            collect_stats(&mut halves, vec![], vec![(5, [2, 0, 0])]),
            None
        );
        assert_eq!(
            collect_stats(&mut halves, vec![(5, [7, 3, 1], true)], vec![]),
            Some((5, [9, 3, 1]))
        );
        // The near table a frame behind (its ring held no readback free for 6): neither
        // half publishes alone. Sourced 7 proves sourced 6 was skipped, so near 6 goes
        // instead of waiting forever, and 7's halves pair once both are in.
        assert_eq!(
            collect_stats(
                &mut halves,
                vec![(6, [7, 3, 1], true)],
                vec![(7, [2, 0, 0])],
            ),
            None
        );
        assert_eq!(
            collect_stats(&mut halves, vec![(7, [7, 3, 1], true)], vec![]),
            Some((7, [9, 3, 1]))
        );
        assert_eq!(collect_stats(&mut halves, vec![], vec![]), None);
    }

    /// A frame the sourced pass did not draw has no sourced half: its near half is the
    /// whole frame and publishes at once; one that did draw it waits for its counts.
    #[test]
    fn a_frame_without_sourced_geometry_publishes_alone() {
        let mut halves = StatsHalves::default();
        assert_eq!(
            collect_stats(&mut halves, vec![(6, [7, 3, 1], false)], vec![]),
            Some((6, [7, 3, 1]))
        );
        assert_eq!(
            collect_stats(&mut halves, vec![(7, [7, 3, 1], true)], vec![]),
            None
        );
        assert_eq!(
            collect_stats(&mut halves, vec![], vec![(7, [2, 0, 0])]),
            Some((7, [9, 3, 1]))
        );
    }

    /// A poll where one ring skipped a copy carries a complete older frame beside a
    /// half of a newer one: near 20 publishes whole from the coalesced poll, sourced
    /// 21 waits for near 21 (whose copy was skipped), and goes once the near ring
    /// passes 21 without delivering it.
    #[test]
    fn a_coalesced_poll_still_publishes_the_older_complete_frame() {
        let mut halves = StatsHalves::default();
        assert_eq!(
            collect_stats(
                &mut halves,
                vec![(20, [7, 3, 1], true)],
                vec![(20, [2, 0, 0]), (21, [4, 0, 0])],
            ),
            Some((20, [9, 3, 1]))
        );
        assert_eq!(
            collect_stats(&mut halves, vec![(22, [6, 2, 1], false)], vec![]),
            Some((22, [6, 2, 1]))
        );
        assert_eq!(collect_stats(&mut halves, vec![], vec![]), None);
    }

    /// A frame that drew no sourced geometry needs no partner, so it publishes even
    /// when the sourced ring has already delivered a newer frame.
    #[test]
    fn a_whole_near_frame_publishes_behind_the_sourced_ring() {
        let mut halves = StatsHalves::default();
        assert_eq!(
            collect_stats(&mut halves, vec![], vec![(6, [2, 0, 0])]),
            None
        );
        assert_eq!(
            collect_stats(&mut halves, vec![(5, [7, 3, 1], false)], vec![]),
            Some((5, [7, 3, 1]))
        );
        assert_eq!(
            collect_stats(&mut halves, vec![(6, [7, 3, 1], true)], vec![]),
            Some((6, [9, 3, 1]))
        );
    }

    /// A newer unmatched half never displaces an older pairing: sourced 21 waits while
    /// sourced 20 completes frame 20, then joins near 21 when it comes.
    #[test]
    fn a_newer_half_does_not_displace_an_older_pairing() {
        let mut halves = StatsHalves::default();
        // Frame 20's near half waits for its partner.
        assert_eq!(
            collect_stats(&mut halves, vec![(20, [7, 3, 1], true)], vec![]),
            None
        );
        // One poll brings near 21 (its sourced counts still away) and sourced 20:
        // 20 completes from the coalesced poll, 21 waits on.
        assert_eq!(
            collect_stats(
                &mut halves,
                vec![(21, [5, 2, 1], true)],
                vec![(20, [2, 0, 0])],
            ),
            Some((20, [9, 3, 1]))
        );
        // Then sourced 21 joins near 21, not lost to the older pairing.
        assert_eq!(
            collect_stats(&mut halves, vec![], vec![(21, [4, 0, 0])]),
            Some((21, [9, 2, 1]))
        );
    }

    /// The sourced flag rides the readback, so a near half whose frame drew sourced
    /// geometry still waits for its partner — and never publishes its counts alone —
    /// after 64 and more newer frames' flags have been and gone.
    #[test]
    fn a_near_half_waits_for_its_partner_past_the_flag_cycles() {
        let mut halves = StatsHalves::default();
        // Frame 20 drew sourced geometry; its sourced counts come back much later.
        assert_eq!(
            collect_stats(&mut halves, vec![(20, [7, 3, 1], true)], vec![]),
            None
        );
        // The frames after it drew none; each publishes by itself.
        for frame in 21..=84_u64 {
            let near = [(frame % 7) as usize + 1, 0, 0];
            assert_eq!(
                collect_stats(&mut halves, vec![(frame, near, false)], vec![]),
                Some((frame, near))
            );
        }
        // Frame 20's half is still held, its flag still says sourced: it joins its
        // partner instead of publishing its near counts alone.
        assert_eq!(
            collect_stats(&mut halves, vec![], vec![(20, [2, 0, 0])]),
            Some((20, [9, 3, 1]))
        );
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
