//! The performance overlay (Horizon's Overlay setting): frames per second,
//! frame, CPU and GPU times, and a graph of recent frame times.
use crate::{
    graphics::Overlay,
    hud::{HudAtlas, HudBatch, rgba},
    text::Text,
};
use std::collections::VecDeque;

const HISTORY: usize = 160;
/// Frames the 1% low is taken over (about 20 s at 60 fps).
const LOW_WINDOW: usize = 1200;

/// Recent frames in numbers (the Stats overlay, `horizon-ctl perf`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSummary {
    pub fps: f32,
    pub frame_ms: f32,
    pub worst_ms: f32,
    /// The average frame time of the slowest 1% of the last `LOW_WINDOW` frames, as frames
    /// per second (FrameStats' `low_1pct_fps`).
    pub low_1pct_fps: f32,
    pub cpu_ms: f32,
    pub gpu_ms: f32,
}

#[derive(Default)]
pub struct FrameGraph {
    /// Frame, CPU and GPU milliseconds of recent frames, oldest first.
    frames: VecDeque<(f32, f32, f32)>,
    /// Frame milliseconds over a longer window, for the 1% low.
    window: VecDeque<f32>,
    /// The resolution scale Dynamic Resolution has the world at, in percent, while it is on:
    /// the numbers above were measured at a resolution that moves.
    pub dynamic_scale: Option<u32>,
}
impl FrameGraph {
    pub fn record(&mut self, frame_ms: f32, cpu_ms: f32, gpu_ms: Option<f32>) {
        let gpu = gpu_ms.unwrap_or_else(|| self.frames.back().map_or(0.0, |f| f.2));
        if self.frames.len() == HISTORY {
            self.frames.pop_front();
        }
        self.frames.push_back((frame_ms, cpu_ms, gpu));
        if self.window.len() == LOW_WINDOW {
            self.window.pop_front();
        }
        self.window.push_back(frame_ms);
    }
    pub fn summary(&self) -> Option<FrameSummary> {
        if self.frames.is_empty() {
            return None;
        }
        let n = self.frames.len() as f32;
        let avg = |f: fn(&(f32, f32, f32)) -> f32| self.frames.iter().map(f).sum::<f32>() / n;
        let frame_ms = avg(|f| f.0);
        let mut sorted: Vec<f32> = self.window.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        let slowest = sorted.len().div_ceil(100);
        let slow = sorted[sorted.len() - slowest..].iter().sum::<f32>() / slowest as f32;
        Some(FrameSummary {
            fps: 1000.0 / frame_ms.max(0.001),
            frame_ms,
            worst_ms: self.frames.iter().map(|f| f.0).fold(0.0f32, f32::max),
            low_1pct_fps: 1000.0 / slow.max(0.001),
            cpu_ms: avg(|f| f.1),
            gpu_ms: avg(|f| f.2),
        })
    }
    /// Starling: over a menu (the title, the options, a screen over the world), the frames per
    /// second alone in a small box at the top right, where the full overlay covered the
    /// screen's own buttons. The graph and the times are for the world. `top` is where it starts:
    /// under the touch controls' Back at the top right, which it covered at 2.
    pub fn draw_compact(&self, mode: Overlay, batch: &mut HudBatch, atlas: &HudAtlas, top: f32) {
        if matches!(mode, Overlay::Off | Overlay::Stats) || self.frames.is_empty() {
            return;
        }
        let frame = self.frames.iter().map(|f| f.0).sum::<f32>() / self.frames.len() as f32;
        let line = format!("{:.0} fps", 1000.0 / frame.max(0.001));
        let width = atlas.font.width(&line) + 6.0;
        let x = batch.width - width - 2.0;
        batch.fill(atlas, x, top, width, 14.0, rgba(0, 0, 0, 144));
        batch.text_styled(
            atlas,
            &Text::literal(line),
            x + 3.0,
            top + 2.0,
            rgba(255, 255, 255, 255),
            true,
        );
    }
    pub fn draw(&self, mode: Overlay, batch: &mut HudBatch, atlas: &HudAtlas) {
        // The Stats overlay draws its own panel (stats_overlay.rs).
        if matches!(mode, Overlay::Off | Overlay::Stats) || self.frames.is_empty() {
            return;
        }
        let n = self.frames.len() as f32;
        let avg = |f: fn(&(f32, f32, f32)) -> f32| self.frames.iter().map(f).sum::<f32>() / n;
        let (frame, cpu, gpu) = (avg(|f| f.0), avg(|f| f.1), avg(|f| f.2));
        let worst = self.frames.iter().map(|f| f.0).fold(0.0f32, f32::max);
        let lines = [
            match self.dynamic_scale {
                Some(scale) => format!("{:.0} fps  res {scale}%", 1000.0 / frame.max(0.001)),
                None => format!("{:.0} fps", 1000.0 / frame.max(0.001)),
            },
            format!("frame {frame:.1} ms  max {worst:.1}"),
            format!("cpu {cpu:.1}  gpu {gpu:.1} ms"),
        ];
        let width = 120.0f32.max(
            lines
                .iter()
                .map(|l| atlas.font.width(l))
                .fold(0.0, f32::max)
                + 6.0,
        );
        let x = batch.width - width - 2.0;
        let graph = if mode == Overlay::Graph { 42.0 } else { 0.0 };
        batch.fill(
            atlas,
            x,
            2.0,
            width,
            4.0 + lines.len() as f32 * 10.0 + graph,
            rgba(0, 0, 0, 144),
        );
        for (i, line) in lines.iter().enumerate() {
            let color = if i == 0 {
                rgba(255, 255, 255, 255)
            } else {
                rgba(200, 200, 200, 255)
            };
            batch.text_styled(
                atlas,
                &Text::literal(line.clone()),
                x + 3.0,
                4.0 + i as f32 * 10.0,
                color,
                true,
            );
        }
        if mode != Overlay::Graph {
            return;
        }
        // Bars of frame time on a 0..33 ms scale; lines at 60 and 30 fps.
        let (gy, gh) = (4.0 + lines.len() as f32 * 10.0 + 2.0, 36.0);
        let scale = gh / 33.3;
        let bar = (width - 6.0) / HISTORY as f32;
        for (i, &(ms, cpu, _)) in self.frames.iter().enumerate() {
            let height = (ms * scale).min(gh);
            let bx = x + 3.0 + i as f32 * bar;
            let color = if ms > 33.3 {
                rgba(240, 90, 80, 255)
            } else if ms > 16.7 {
                rgba(240, 200, 90, 255)
            } else {
                rgba(110, 210, 120, 255)
            };
            batch.fill(atlas, bx, gy + gh - height, bar.max(1.0), height, color);
            let cpu_height = (cpu * scale).min(height);
            batch.fill(
                atlas,
                bx,
                gy + gh - cpu_height,
                bar.max(1.0),
                cpu_height,
                rgba(80, 140, 230, 200),
            );
        }
        for target in [16.7, 33.3] {
            let ly = gy + gh - target * scale;
            batch.fill(
                atlas,
                x + 3.0,
                ly,
                width - 6.0,
                1.0,
                rgba(255, 255, 255, 90),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starling: over a menu the overlay is one short line, not the times and the graph.
    #[test]
    fn over_a_menu_the_overlay_is_the_frames_per_second_alone() {
        let atlas = HudAtlas::boot();
        let mut graph = FrameGraph::default();
        for _ in 0..30 {
            graph.record(16.7, 8.0, Some(10.0));
        }
        let drawn = |compact: bool| {
            let mut batch = HudBatch::new(
                [2400, 1080],
                crate::hud::GuiScale {
                    requested: 0,
                    unicode: false,
                },
            );
            if compact {
                graph.draw_compact(Overlay::Graph, &mut batch, &atlas, 2.0);
            } else {
                graph.draw(Overlay::Graph, &mut batch, &atlas);
            }
            let (left, bottom) = batch
                .vertices
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, b), v| {
                    (l.min(v.pos[0]), b.max(v.pos[1]))
                });
            (batch.width - left, bottom)
        };
        let (full_w, full_h) = drawn(false);
        let (w, h) = drawn(true);
        assert!(w < full_w, "{w} wide over a menu, {full_w} in the world");
        assert!(
            h < full_h / 3.0,
            "{h} tall over a menu, {full_h} in the world"
        );
    }
}
