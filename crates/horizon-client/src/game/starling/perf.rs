//! The Stats overlay's numbers from the game (horizon_client::stats_overlay), its panel over the
//! HUD, and the `perf` and `overlay` control commands.
use super::Game;
use horizon_client::{
    Result, graphics::Overlay, hud::HudBatch, stats_overlay::StatsView, system_stats::Sampler,
    ui_theme::Ui,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(super) const HELP: &[&str] = &[
    "perf                           frames (1% low), CPU/GPU time, memory, chunks, meshing, draws, temperatures, throttling",
    "overlay off|fps|graph|stats    the performance overlay (stats: everything perf reports)",
];

/// How often the GPU allocator's report is taken: it walks every allocation.
const GPU_MEMORY_EVERY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct Perf {
    system: Sampler,
    /// The GPU allocator's (allocated, reserved) bytes, and when they were taken.
    gpu_memory: Option<(Option<u64>, Option<u64>, Instant)>,
}

impl Game<'_> {
    /// The last system sample (memory, temperatures, throttling), taken on the sampler's own
    /// thread: nothing else of the Stats overlay is gathered.
    pub(super) fn system_sample(&self) -> horizon_client::system_stats::SystemSample {
        self.stats_overlay.system.latest()
    }
    pub(super) fn stats_view(&mut self) -> StatsView {
        let now = Instant::now();
        if let Some(r) = &self.renderer
            && self
                .stats_overlay
                .gpu_memory
                .is_none_or(|(.., at)| now.duration_since(at) >= GPU_MEMORY_EVERY)
        {
            let report = r.device.generate_allocator_report();
            self.stats_overlay.gpu_memory = Some((
                report.as_ref().map(|m| m.total_allocated_bytes),
                report.as_ref().map(|m| m.total_reserved_bytes),
                now,
            ));
        }
        let (gpu_allocated, gpu_reserved) = self
            .stats_overlay
            .gpu_memory
            .map_or((None, None), |(a, r, _)| (a, r));
        let stages = self.streaming.stages();
        let stage = |name: &str| stages.iter().find(|(n, _)| *n == name).map_or(0, |s| s.1);
        StatsView {
            frames: self.frame_graph.summary(),
            system: self.stats_overlay.system.latest(),
            gpu_allocated,
            gpu_reserved,
            chunks: self.world.chunks.len(),
            meshed_sections: self.streaming.meshed_total,
            streaming_queue: stage("queued"),
            meshing: stage("meshing"),
            pending_uploads: self.pending_uploads.len(),
            draw_calls: self.renderer.as_ref().map_or(0, |r| r.section_draw_calls),
            drawn_sections: self.renderer.as_ref().map_or(0, |r| r.drawn_sections),
            render_distance: self.streaming.radius,
            backend: self
                .renderer
                .as_ref()
                .map_or_else(String::new, |r| format!("{:?}", r.info.backend)),
            gpu: self
                .renderer
                .as_ref()
                .map_or_else(String::new, |r| r.info.name.clone()),
        }
    }

    /// The Stats overlay's panel, when it is the chosen overlay.
    pub(super) fn draw_stats_overlay(&mut self, batch: &mut HudBatch) {
        if self.settings.horizon.overlay != Overlay::Stats {
            return;
        }
        let view = self.stats_view();
        let mut ui = Ui::new(&self.gui.atlas, batch, (-1.0, -1.0));
        view.draw(&mut ui);
        ui.finish();
    }

    /// The three-finger tap: the Stats overlay on, or the overlay off.
    pub(super) fn toggle_stats_overlay(&mut self) {
        let overlay = &mut self.settings.horizon.overlay;
        *overlay = if *overlay == Overlay::Stats {
            Overlay::Off
        } else {
            Overlay::Stats
        };
    }

    pub(super) fn perf_command(&mut self, words: &[&str]) -> Result<Option<Value>> {
        match words.first().copied() {
            Some("perf") => Ok(Some(self.stats_view().json())),
            Some("overlay") => {
                let overlay = match words.get(1).copied() {
                    Some("off") => Overlay::Off,
                    Some("fps") => Overlay::Fps,
                    Some("graph") => Overlay::Graph,
                    Some("stats") => Overlay::Stats,
                    other => {
                        return Err(format!(
                            "overlay takes off, fps, graph or stats, not {}",
                            other.unwrap_or("nothing")
                        )
                        .into());
                    }
                };
                self.settings.horizon.overlay = overlay;
                Ok(Some(json!({ "ok": true, "overlay": words[1] })))
            }
            _ => Ok(None),
        }
    }
}
