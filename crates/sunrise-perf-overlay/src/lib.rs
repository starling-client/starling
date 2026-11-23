//! The performance overlay (Sunrise's Overlay setting): frames per second,
//! frame, CPU and GPU times, and a graph of recent frame times.
//!
//! A Sunrise crate (docs/layers.md), which `sunrise-official` registers: it hears each rendered
//! frame's times through the platform's hooks ([`HOOKS`]: `frame_timed`), draws over everything
//! as a HUD layer ([`LAYER`]), keeps its setting as the `overlay` line of `sunrise-graphics.txt`
//! ([`SETTING`]) and shows it on Sunrise Settings' Extras page ([`SECTION`]). Each `Game` has its
//! own setting and frames, by its seat (`hooks::seat`).
#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::sync::Mutex;
use sunrise_platform::hooks::{self, Hooks, perf::FrameTimes};
use sunrise_platform::hud::{Canvas, Layer, Stage};
use sunrise_platform::pages::{Act, Label, Page, Place, Row};
use sunrise_platform::settings::Store;

const HISTORY: usize = 160;

#[derive(Default)]
pub struct FrameGraph {
    /// Frame, CPU and GPU milliseconds of recent frames, oldest first.
    frames: VecDeque<(f32, f32, f32)>,
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
    }
    /// Starling: over a menu (`Canvas::over_menu`), on a phone, the frames per second alone in a
    /// small box at the top right, where the full overlay covered the screen's own buttons. The
    /// times and the graph are for the world.
    fn draw_compact(&self, canvas: &mut dyn Canvas) {
        let n = self.frames.len() as f32;
        let frame = self.frames.iter().map(|f| f.0).sum::<f32>() / n;
        let line = format!("{:.0} fps", 1000.0 / frame.max(0.001));
        let width = canvas.text_width(&line) + 6.0;
        let x = canvas.size().0 - width - 2.0;
        canvas.fill(x, 2.0, width, 14.0, 0x9000_0000);
        canvas.text(x + 3.0, 4.0, &line, 0xFFFF_FFFF, true);
    }
    pub fn draw(&self, mode: Overlay, canvas: &mut dyn Canvas) {
        if mode == Overlay::Off || self.frames.is_empty() {
            return;
        }
        if cfg!(target_os = "android") && canvas.over_menu() {
            self.draw_compact(canvas);
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
                .map(|l| canvas.text_width(l))
                .fold(0.0, f32::max)
                + 6.0,
        );
        let x = canvas.size().0 - width - 2.0;
        let graph = if mode == Overlay::Graph { 42.0 } else { 0.0 };
        canvas.fill(
            x,
            2.0,
            width,
            4.0 + lines.len() as f32 * 10.0 + graph,
            0x9000_0000,
        );
        for (i, line) in lines.iter().enumerate() {
            let color = if i == 0 { 0xFFFF_FFFF } else { 0xFFC8_C8C8 };
            canvas.text(x + 3.0, 4.0 + i as f32 * 10.0, line, color, true);
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
                0xFFF0_5A50
            } else if ms > 16.7 {
                0xFFF0_C85A
            } else {
                0xFF6E_D278
            };
            canvas.fill(bx, gy + gh - height, bar.max(1.0), height, color);
            let cpu_height = (cpu * scale).min(height);
            canvas.fill(
                bx,
                gy + gh - cpu_height,
                bar.max(1.0),
                cpu_height,
                0xC850_8CE6,
            );
        }
        for target in [16.7, 33.3] {
            let ly = gy + gh - target * scale;
            canvas.fill(x + 3.0, ly, width - 6.0, 1.0, 0x5AFF_FFFF);
        }
    }
}

/// The performance overlay drawn over the HUD.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    Off,
    /// Frames per second and frame time.
    Fps,
    /// The numbers and a graph of recent frame times.
    Graph,
}
impl Overlay {
    const ALL: [Overlay; 3] = [Overlay::Off, Overlay::Fps, Overlay::Graph];
    /// Its value in `sunrise-graphics.txt`.
    fn name(self) -> &'static str {
        match self {
            Overlay::Off => "off",
            Overlay::Fps => "fps",
            Overlay::Graph => "graph",
        }
    }
    fn parse(value: &str) -> Self {
        match value {
            "fps" => Overlay::Fps,
            "graph" => Overlay::Graph,
            _ => Overlay::Off,
        }
    }
    /// What its row says: a translation key.
    fn label(self) -> &'static str {
        match self {
            Overlay::Off => "Off",
            Overlay::Fps => "FPS",
            Overlay::Graph => "FPS and Graph",
        }
    }
    /// The next value, or the one before with `back`.
    fn step(self, back: bool) -> Self {
        let all = Self::ALL;
        let i = all.iter().position(|x| *x == self).unwrap_or(0);
        let n = all.len();
        all[if back { (i + n - 1) % n } else { (i + 1) % n }]
    }
}

/// One `Game`'s setting and frames.
#[derive(Default)]
struct Seat {
    overlay: Overlay,
    graph: FrameGraph,
}

/// Each `Game`'s, by seat (`hooks::seat`): 0 for the process's first.
static SEATS: Mutex<Vec<Seat>> = Mutex::new(Vec::new());

/// `f` on the player whose turn it is.
fn with_seat<T>(f: impl FnOnce(&mut Seat) -> T) -> T {
    let mut seats = SEATS.lock().unwrap_or_else(|e| e.into_inner());
    let seat = hooks::seat();
    if seats.len() <= seat {
        seats.resize_with(seat + 1, Seat::default);
    }
    f(&mut seats[seat])
}

/// Starling: the setting's label (a translation key), for the row the graphics settings' Advanced
/// page shows.
pub fn label() -> &'static str {
    with_seat(|s| s.overlay.label())
}

/// Starling: the setting's next value, or the one before with `back`.
pub fn step(back: bool) {
    with_seat(|s| s.overlay = s.overlay.step(back));
}

/// The frames' hook, for `sunrise_platform::hooks::register`, and the overlay's layer, for
/// `sunrise_platform::hud::register`.
pub static HOOKS: PerfOverlay = PerfOverlay;
pub static LAYER: PerfOverlay = PerfOverlay;

pub struct PerfOverlay;

impl Hooks for PerfOverlay {
    fn frame_timed(&self, times: &FrameTimes) {
        with_seat(|s| {
            s.graph.dynamic_scale = times.dynamic_scale;
            s.graph.record(times.frame, times.cpu, times.gpu);
        });
    }
}

impl Layer for PerfOverlay {
    fn draw(&self, canvas: &mut dyn Canvas) {
        with_seat(|s| s.graph.draw(s.overlay, canvas));
    }
    fn stage(&self) -> Stage {
        Stage::Top
    }
}

/// The setting's line of `sunrise-graphics.txt`, where the video settings kept it, for
/// `sunrise_platform::settings::register`.
pub static SETTING: Setting = Setting;

pub struct Setting;

const KEY: &str = "overlay";

impl Store for Setting {
    fn file(&self) -> &'static str {
        "sunrise-graphics.txt"
    }
    fn owns(&self, key: &str) -> bool {
        key == KEY
    }
    fn load(&self, _key: &str, value: &str) {
        with_seat(|s| s.overlay = Overlay::parse(value));
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        line(KEY, with_seat(|s| s.overlay).name());
    }
    fn is_default(&self) -> bool {
        with_seat(|s| s.overlay) == Overlay::Off
    }
}

/// The setting's row, in a group of its own on Sunrise Settings' Extras page, for
/// `sunrise_platform::pages::register`.
pub static SECTION: Section = Section;

pub struct Section;

impl Page for Section {
    fn id(&self) -> &'static str {
        "sunrise-perf-overlay.extras"
    }
    fn title(&self) -> &'static str {
        "Overlay"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Cycle {
            label: "Performance Overlay",
            value: Label::Key(with_seat(|s| s.overlay).label()),
        });
    }
    fn act(&self, row: usize, act: Act) {
        let back = match act {
            Act::Press => false,
            Act::PressBack => true,
            _ => return,
        };
        if row == 0 {
            with_seat(|s| s.overlay = s.overlay.step(back));
        }
    }
    fn tooltip(&self, _row: usize) -> Option<Label> {
        Some(Label::Key(
            "Frame rate and frame times over the game, with a graph.",
        ))
    }
}
