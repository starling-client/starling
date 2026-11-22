//! Starling's performance test (the 0.8.1 test builds): which way of drawing runs fastest on the
//! player's phone, found while they play, and the average frame rate of each setup they play with.
//!
//! The guided test needs nothing from the player: it starts on its own in a test build (or from
//! the Diagnostics page) and, every 30 seconds of play, switches how terrain is drawn and where
//! the sky goes, twice round; then it says which was fastest, offers it, and sends the results.
//!
//! A frame counts only in a world with no screen, menu or container open, the game focused and
//! not paused, and some input in the last 30 seconds: menus and an idle phone don't pull the
//! averages around. Each setup (the draw choices, the graphics settings, the window's size) adds
//! up its own frames, so going back to a setup adds to it. `performance/perf-sessions.json`
//! keeps them across starts, the diagnostic ZIP takes it, and a test build sends them to the
//! build gateway (`/v1/perf`): the GPU, the settings and the frame times, nothing about the player.
//!
//! Calibrate... (the graphics screen's Performance page, any build) finds the same thing in a
//! minute or two, on its own: from the title screen it opens the benchmark's world
//! (`sunrise_client::benchmark`, seed 1234), holds the camera over it, and times each way of
//! drawing that shows the very same picture (terrain draws, the sky's side of terrain, occlusion
//! culling, hidden faces) a few seconds at a time, interleaved. It keeps the fastest where it
//! beats the device's own by [`WORTH_IT`], deletes the world and opens its page again with what
//! it found. Nothing is sent: what it measured stays in `perf-lab.json`.
use super::{Game, Screen};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sunrise_client::benchmark::{self, Control};
use sunrise_client::{lang, options_ui::Page, text::Text};

const SESSIONS: &str = "perf-sessions.json";
const CHOICES: &str = "perf-lab.json";
const REPORTS: &str = "https://191-234-193-205.sslip.io/v1/perf";
/// No input for this long: the player is away, and frames stop counting.
const IDLE: Duration = Duration::from_secs(30);
/// After a menu or a pause, frames count again from this long on (the world settles).
const SETTLE: Duration = Duration::from_secs(2);
const CHECK_EVERY: Duration = Duration::from_secs(1);
const THERMAL_EVERY: Duration = Duration::from_secs(5);
const SAVE_EVERY: Duration = Duration::from_secs(60);
/// A frame this long or longer is a hitch from outside play (a resume, a dialog): left out.
const HITCH_MS: f32 = 1000.0;
/// Setups played for less than this are left off the page (the file keeps them).
const SHOWN_AFTER: f64 = 20.0;
const KEEP: usize = 40;
/// The frame-time histogram: half-millisecond buckets up to 100 ms, then one for the rest.
const BUCKET_MS: f32 = 0.5;
const BUCKETS: usize = 201;
/// The guided test: seconds of play for each way of drawing, and how many times round.
const STEP_SECONDS: f64 = 30.0;
const ROUNDS: usize = 2;
/// A test build starts the guided test on its own after this much play.
const AUTO_START_SECONDS: f64 = 20.0;
/// A test build sends its setups again after this much more play.
const SEND_EVERY_SECONDS: f64 = 600.0;
/// Below this gain the default stays the one to keep.
const WORTH_IT: f64 = 1.03;

/// How terrain sections are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Draws {
    #[default]
    Auto,
    Direct,
    Indirect,
    Flat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Switch {
    #[default]
    Auto,
    On,
    Off,
}

impl Switch {
    fn pick(self, auto: bool) -> bool {
        match self {
            Switch::Auto => auto,
            Switch::On => true,
            Switch::Off => false,
        }
    }
    fn next(self) -> Self {
        match self {
            Switch::Auto => Switch::On,
            Switch::On => Switch::Off,
            Switch::Off => Switch::Auto,
        }
    }
}

/// The player's choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Choices {
    draws: Draws,
    late_sky: Switch,
    occlusion: Switch,
    /// Solid quads facing away from the eye left out (`Renderer::section_sides`).
    hidden_faces: Switch,
    /// Improved Transparency, the order-independent passes the Fabulous preset turns on
    /// (`Renderer::improved_transparency`). Half the GPU time of a frame on some phones.
    #[serde(default)]
    transparency: Switch,
}

/// `performance/perf-lab.json`: the choices, the test build whose guided test ran, and what the
/// last calibration found.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    #[serde(flatten)]
    choices: Choices,
    guided_done: Option<String>,
    calibrated: Option<Calibrated>,
}

/// What a calibration found: the fastest way of drawing, which it put in use, its frame rate
/// and the device's own (Auto) in the same run, and how long the run took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Calibrated {
    choices: Choices,
    fps: f64,
    auto_fps: f64,
    seconds: f64,
}

/// The renderer's own choices before the test touched them, and what this device takes.
#[derive(Clone, Copy, Debug)]
struct Device {
    flat: bool,
    indirect: bool,
    late_sky: bool,
    occlusion: bool,
    hidden_faces: bool,
    /// Improved Transparency as the player's settings left it.
    transparency: bool,
    /// `Renderer::draw_choices`: flat, indirect, late sky, occlusion culling.
    offers: [bool; 4],
}

/// One setup's frames, added up.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Setup {
    setup: Value,
    /// Unix seconds.
    first: u64,
    last: u64,
    seconds: f64,
    frames: u64,
    cpu_ms: f64,
    gpu_ms: f64,
    gpu_frames: u64,
    worst_ms: f32,
    histogram: Vec<u32>,
    #[serde(default)]
    thermal_max: Option<i32>,
    #[serde(default)]
    battery_max_c: Option<f32>,
    #[serde(default)]
    cpu_max_c: Option<f32>,
}

impl Setup {
    fn new(setup: Value) -> Self {
        let now = unix_now();
        Setup {
            setup,
            first: now,
            last: now,
            seconds: 0.0,
            frames: 0,
            cpu_ms: 0.0,
            gpu_ms: 0.0,
            gpu_frames: 0,
            worst_ms: 0.0,
            histogram: vec![0; BUCKETS],
            thermal_max: None,
            battery_max_c: None,
            cpu_max_c: None,
        }
    }
    fn add(&mut self, frame: f32, cpu: f32, gpu: Option<f32>) {
        self.seconds += f64::from(frame) / 1000.0;
        self.frames += 1;
        self.cpu_ms += f64::from(cpu);
        if let Some(gpu) = gpu {
            self.gpu_ms += f64::from(gpu);
            self.gpu_frames += 1;
        }
        self.worst_ms = self.worst_ms.max(frame);
        if self.histogram.len() != BUCKETS {
            self.histogram = vec![0; BUCKETS];
        }
        let bucket = ((frame / BUCKET_MS) as usize).min(BUCKETS - 1);
        self.histogram[bucket] += 1;
    }
    fn fps(&self) -> f64 {
        if self.seconds > 0.0 {
            self.frames as f64 / self.seconds
        } else {
            0.0
        }
    }
    /// The frame time below which `share` of the frames fall, in ms (each bucket's top).
    fn percentile_ms(&self, share: f64) -> f32 {
        let total: u64 = self.histogram.iter().map(|&n| u64::from(n)).sum();
        if total == 0 {
            return 0.0;
        }
        let want = (total as f64 * share).ceil() as u64;
        let mut seen = 0;
        for (i, &n) in self.histogram.iter().enumerate() {
            seen += u64::from(n);
            if seen >= want {
                return if i + 1 == BUCKETS {
                    self.worst_ms
                } else {
                    (i + 1) as f32 * BUCKET_MS
                };
            }
        }
        self.worst_ms
    }
    /// The frame rate of the slowest 1% of frames.
    fn low_fps(&self) -> f64 {
        let ms = self.percentile_ms(0.99);
        if ms > 0.0 {
            1000.0 / f64::from(ms)
        } else {
            0.0
        }
    }
    fn summary(&self) -> Value {
        let per = |sum: f64, n: u64| (n > 0).then(|| round(sum / n as f64));
        json!({
            "minutes": round(self.seconds / 60.0),
            "fps": round(self.fps()),
            "low_1pct_fps": round(self.low_fps()),
            "frame_ms_p50": self.percentile_ms(0.5),
            "frame_ms_p90": self.percentile_ms(0.9),
            "frame_ms_p99": self.percentile_ms(0.99),
            "cpu_ms": per(self.cpu_ms, self.frames),
            "gpu_ms": per(self.gpu_ms, self.gpu_frames),
        })
    }
}

/// The guided test under way.
struct Guided {
    /// The ways of drawing it tries, the first being the device's own (all Auto).
    variants: Vec<Choices>,
    /// Each variant's frames.
    tallies: Vec<Setup>,
    /// Which step of `variants.len() * ROUNDS`.
    step: usize,
    step_seconds: f64,
    /// The player's choices before it started, given back when it ends.
    before: Choices,
    told: Option<Instant>,
}

impl Guided {
    fn steps(&self) -> usize {
        self.variants.len() * ROUNDS
    }
    fn variant(&self) -> usize {
        self.step % self.variants.len()
    }
}

#[derive(Default)]
pub(super) struct PerfLab {
    loaded: bool,
    choices: Choices,
    guided_done: Option<String>,
    device: Option<Device>,
    setups: Vec<Setup>,
    /// `setups`' index of the setup in play.
    current: Option<usize>,
    /// Since when frames have counted without a break (menus, a pause, idle).
    counting_since: Option<Instant>,
    checked: Option<Instant>,
    thermal: Option<Instant>,
    saved: Option<Instant>,
    dirty: bool,
    guided: Option<Guided>,
    /// The guided test's fastest way of drawing, when it beat the device's own.
    fastest: Option<(Choices, f64, f64)>,
    /// This APK's version when it is a test build (Starling Preview), looked up once.
    test_build: Option<Option<String>>,
    /// Seconds of counted play, and when the setups were last sent.
    played: f64,
    sent_at: f64,
    /// Calibrate... under way.
    calibration: Option<Calibration>,
    /// What the last calibration found (kept in `perf-lab.json`).
    calibrated: Option<Calibrated>,
    /// Why the last calibration stopped, until the next one.
    calibrate_stopped: Option<String>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn round(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn folder() -> PathBuf {
    sunrise_client::logging::private_home().join("performance")
}

/// This APK's version where it is a test build: a Starling Preview, never a release, started with
/// `STARLING_PERF_TEST=1` (a line of `sunrise-args.txt`). Only then does the guided test start on
/// its own and reports go out. Every Preview started it, once per version, so again with each new
/// development APK, and `REPORTS` is the Azure gateway, gone since 2026-11-14. A Calibrate button
/// that sends nothing is to take its place.
fn test_build() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        if std::env::var_os("STARLING_PERF_TEST").is_none_or(|on| on != "1") {
            return None;
        }
        crate::android::app_version()
            .filter(|(name, release)| !release && name.contains("preview"))
            .map(|(name, _)| name)
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// Sends a report to the build gateway, on a thread of its own.
fn send(report: Value) {
    let spawned = std::thread::Builder::new()
        .name("Performance report".into())
        .spawn(move || {
            let body = serde_json::to_vec(&report).unwrap_or_default();
            let result = ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(15))
                .timeout_read(Duration::from_secs(30))
                .build()
                .post(REPORTS)
                .set("Content-Type", "application/json")
                .send_bytes(&body);
            match result {
                Ok(_) => log::info!("Starling: performance report sent ({} bytes)", body.len()),
                Err(e) => log::warn!("Starling: performance report not sent: {e}"),
            }
        });
    if let Err(e) = spawned {
        log::warn!("Starling: performance report: {e}");
    }
}

impl PerfLab {
    fn load(&mut self) {
        self.loaded = true;
        let dir = folder();
        if let Ok(text) = std::fs::read_to_string(dir.join(CHOICES)) {
            let saved: Saved = serde_json::from_str(&text).unwrap_or_default();
            self.choices = saved.choices;
            self.guided_done = saved.guided_done;
            self.calibrated = saved.calibrated;
        }
        if let Ok(text) = std::fs::read_to_string(dir.join(SESSIONS))
            && let Ok(file) = serde_json::from_str::<Value>(&text)
            && let Some(setups) = file.get("setups")
        {
            self.setups = serde_json::from_value(setups.clone()).unwrap_or_default();
        }
    }

    fn save_choices(&self) {
        let saved = Saved {
            choices: self.choices,
            guided_done: self.guided_done.clone(),
            calibrated: self.calibrated,
        };
        let text = serde_json::to_vec_pretty(&saved).unwrap_or_default();
        let dir = folder();
        if let Err(e) =
            std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(CHOICES), text))
        {
            log::warn!("Starling: performance test: {e}");
        }
    }

    /// The choices change: the frames from here on are another setup's.
    fn choose(&mut self, choices: Choices) {
        self.choices = choices;
        self.checked = None;
        self.current = None;
    }

    /// Sets the renderer's switches to the player's choices; where a choice is Auto, to the
    /// renderer's own, read before the test first touched it.
    fn apply(&mut self, r: &mut sunrise_client::gfx::Renderer) {
        let d = *self.device.get_or_insert_with(|| Device {
            flat: r.flat_draws(),
            indirect: r.indirect_draws(),
            late_sky: r.late_sky_on(),
            occlusion: r.section_cull(),
            hidden_faces: r.section_sides(),
            transparency: r.improved_transparency,
            offers: r.draw_choices(),
        });
        let c = self.choices;
        let (flat, indirect) = match c.draws {
            Draws::Auto => (d.flat, d.indirect),
            Draws::Direct => (false, false),
            Draws::Indirect => (false, d.offers[1]),
            Draws::Flat => (d.offers[0], false),
        };
        if r.flat_draws() != flat {
            let _ = r.set_flat_draws(flat);
        }
        if r.indirect_draws() != indirect {
            r.set_indirect_draws(indirect);
        }
        let late = c.late_sky.pick(d.late_sky) && d.offers[2];
        if r.late_sky_on() != late {
            let _ = r.set_late_sky(late);
        }
        let occlusion = c.occlusion.pick(d.occlusion) && d.offers[3];
        if r.section_cull() != occlusion {
            let _ = r.set_section_cull(occlusion);
        }
        let faces = c.hidden_faces.pick(d.hidden_faces);
        if r.section_sides() != faces {
            r.set_section_sides(faces);
        }
        // Only ever turned off from what the settings asked for: the game picks the breaking
        // overlay's sprites from the setting, not from here.
        r.improved_transparency = c.transparency.pick(d.transparency) && d.transparency;
    }

    /// The ways of drawing the guided test tries on this device: its own first, then each
    /// other way of drawing terrain it takes, then the sky on the other side of terrain.
    fn variants(&self) -> Vec<Choices> {
        let Some(d) = self.device else {
            return vec![Choices::default()];
        };
        let mut variants = vec![Choices::default()];
        let draws = |draws| Choices {
            draws,
            ..Choices::default()
        };
        if d.flat || d.indirect {
            variants.push(draws(Draws::Direct));
        }
        if d.offers[1] && !d.indirect {
            variants.push(draws(Draws::Indirect));
        }
        if d.offers[0] && !d.flat {
            variants.push(draws(Draws::Flat));
        }
        if d.offers[2] {
            variants.push(Choices {
                late_sky: if d.late_sky { Switch::Off } else { Switch::On },
                ..Choices::default()
            });
        }
        // Improved Transparency is only ever turned off, and only where it is on.
        if d.transparency {
            variants.push(Choices {
                transparency: Switch::Off,
                ..Choices::default()
            });
        }
        variants
    }

    /// The name of a way of drawing, for the player.
    fn describe(&self, c: Choices) -> String {
        let word = |key: &str| lang::get(&format!("starling.perf.{key}"));
        let mut parts = Vec::new();
        match c.draws {
            Draws::Auto => {}
            Draws::Direct => parts.push(word("direct")),
            Draws::Indirect => parts.push(word("indirect")),
            Draws::Flat => parts.push(word("flat")),
        }
        match c.late_sky {
            Switch::Auto => {}
            Switch::On => parts.push(word("sky_late_short")),
            Switch::Off => parts.push(word("sky_early_short")),
        }
        match c.occlusion {
            Switch::Auto => {}
            Switch::On => parts.push(word("occlusion_on_short")),
            Switch::Off => parts.push(word("occlusion_off_short")),
        }
        match c.hidden_faces {
            Switch::Auto => {}
            Switch::On => parts.push(word("faces_on_short")),
            Switch::Off => parts.push(word("faces_off_short")),
        }
        if c.transparency == Switch::Off {
            parts.push(word("transparency_off_short"));
        }
        if parts.is_empty() {
            word("auto_short")
        } else {
            parts.join(", ")
        }
    }

    fn file(&self, device: Value) -> Value {
        let setups: Vec<Value> = self
            .setups
            .iter()
            .map(|s| {
                let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
                v["summary"] = s.summary();
                v
            })
            .collect();
        json!({
            "schema": 1,
            "version": crate::crash::version(),
            "written": unix_now(),
            "device": device,
            "choices": self.choices,
            "note": "Frames count only in a world with no menu open, not paused, with input in \
                the last 30 s. Each entry adds up one setup's frames.",
            "setups": setups,
        })
    }

    /// A report for the gateway: the setups without their histograms.
    fn report(&self, kind: &str, device: Value, app: &str) -> Value {
        let setups: Vec<Value> = self
            .setups
            .iter()
            .filter(|s| s.seconds >= 10.0)
            .map(|s| {
                json!({
                    "setup": s.setup,
                    "summary": s.summary(),
                    "worst_ms": s.worst_ms,
                    "thermal_max": s.thermal_max,
                    "battery_max_c": s.battery_max_c,
                    "cpu_max_c": s.cpu_max_c,
                })
            })
            .collect();
        json!({
            "schema": 1,
            "kind": kind,
            "app": app,
            "version": crate::crash::version(),
            "device": device,
            "setups": setups,
        })
    }
}

impl Game<'_> {
    /// Whether this frame counts: in a world, nothing open over it, focused, not paused, and
    /// the player not away.
    fn perf_lab_counts(&self) -> bool {
        self.player.is_some()
            && self.title.is_none()
            && self.screen == Screen::Playing
            && self.container.is_none()
            && self.focused
            && !self.pause
            && self.last_input.elapsed() < IDLE
            // A calibration's frames are its own: the test world, the camera held.
            && self.perf_lab.calibration.is_none()
    }

    /// What decides the frame rate, as the player set it and the renderer took it.
    fn perf_lab_setup(&self) -> Value {
        let s = &self.settings;
        let h = &s.sunrise;
        let d = |v: &dyn std::fmt::Debug| format!("{v:?}");
        let mut setup = json!({
            "test": self.perf_lab.choices,
            "graphics_api": d(&h.api),
            "preset": d(&h.preset),
            "render_distance": self.streaming.radius,
            "simulation_distance": s.simulation_distance,
            "lod_distance": self.far_terrain_choice().0,
            "far_detail": d(&h.far_detail),
            "leaf_culling": d(&h.leaf_culling),
            "cave_culling": h.cave_culling,
            "gpu_culling": d(&h.gpu_culling),
            "entity_culling": d(&h.entity_culling),
            "particle_culling": d(&h.particle_culling),
            "resolution_scale": h.resolution_scale,
            "upscaling": d(&h.upscaling),
            "upscaling_sharpness": h.upscaling_sharpness,
            "fxaa": h.fxaa,
            "taa": h.taa,
            "frame_generation": d(&h.frame_generation),
            "translucent_batching": h.translucent_batching,
            "shader_pack": h.shader_pack,
            "shader_profile": h.shader_profile,
            "shadow_cache": h.shadow_cache,
            "dither_cutout_mips": h.dither_cutout_mips,
            "animated_textures": h.animated_textures,
            "sky": h.sky,
            "fog": h.fog,
            "weather": h.weather,
            "present_mode": d(&h.present_mode),
            "pacing": d(&h.pacing),
            "workers": h.workers,
            "battery": h.battery,
            "vsync": s.vsync,
            "max_fps": s.max_fps,
            "fps_limit": self.framerate_limit(),
            "smooth_lighting": s.smooth_lighting,
            "clouds": s.clouds,
            "particles": s.particles,
            "entity_distance": s.entity_distance,
            "biome_blend": s.biome_blend,
            "mipmap_levels": s.mipmap_levels,
        });
        if let Some(r) = &self.renderer {
            setup["renderer"] = json!({
                "flat_draws": r.flat_draws(),
                "indirect_draws": r.indirect_draws(),
                "late_sky": r.late_sky_on(),
                "occlusion_culling": r.section_cull(),
                "hidden_faces_skipped": r.section_sides(),
                "improved_transparency": r.improved_transparency,
                "present": d(&r.config.present_mode),
                "size": [r.config.width, r.config.height],
            });
        }
        setup
    }

    fn perf_lab_device(&self) -> Value {
        match &self.renderer {
            Some(r) => json!({
                "gpu": r.info.name,
                "driver": r.info.driver,
                "driver_info": r.info.driver_info,
                "backend": format!("{:?}", r.info.backend),
            }),
            None => Value::Null,
        }
    }

    /// This APK's version where it is a test build, looked up once.
    fn perf_lab_test_build(&mut self) -> Option<String> {
        self.perf_lab
            .test_build
            .get_or_insert_with(test_build)
            .clone()
    }

    /// Each frame, after it was drawn (`sample`: its time, CPU and GPU ms) or not.
    pub(super) fn perf_lab_frame(&mut self, sample: Option<(f32, f32, Option<f32>)>) {
        if !self.perf_lab.loaded {
            self.perf_lab.load();
        }
        self.perf_calibrate_frame();
        if let Some(r) = self.renderer.as_mut() {
            self.perf_lab.apply(r);
        }
        let now = Instant::now();
        if !self.perf_lab_counts() {
            self.perf_lab.counting_since = None;
        } else {
            let since = *self.perf_lab.counting_since.get_or_insert(now);
            if self.perf_lab.current.is_none()
                || self
                    .perf_lab
                    .checked
                    .is_none_or(|t| now.duration_since(t) >= CHECK_EVERY)
            {
                self.perf_lab.checked = Some(now);
                let setup = self.perf_lab_setup();
                self.perf_lab.enter(setup);
            }
            if self
                .perf_lab
                .thermal
                .is_none_or(|t| now.duration_since(t) >= THERMAL_EVERY)
            {
                self.perf_lab.thermal = Some(now);
                let system = self.system_sample();
                if let Some(s) = self
                    .perf_lab
                    .current
                    .and_then(|i| self.perf_lab.setups.get_mut(i))
                {
                    let max_f = |a: Option<f32>, b: Option<f32>| match (a, b) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (a, b) => a.or(b),
                    };
                    s.thermal_max = s.thermal_max.max(system.thermal_status);
                    s.battery_max_c = max_f(s.battery_max_c, system.battery_celsius);
                    s.cpu_max_c = max_f(s.cpu_max_c, system.cpu_celsius);
                }
            }
            if now.duration_since(since) >= SETTLE
                && let Some((frame, cpu, gpu)) = sample
                && frame < HITCH_MS
            {
                if let Some(s) = self
                    .perf_lab
                    .current
                    .and_then(|i| self.perf_lab.setups.get_mut(i))
                {
                    s.add(frame, cpu, gpu);
                    s.last = unix_now();
                    self.perf_lab.dirty = true;
                }
                self.perf_lab.played += f64::from(frame) / 1000.0;
                self.perf_lab_guided_frame(frame, cpu, gpu);
            }
            self.perf_lab_test_build_frame();
        }
        if self
            .perf_lab
            .saved
            .is_none_or(|t| now.duration_since(t) >= SAVE_EVERY)
        {
            self.perf_lab_save(false);
        }
    }

    /// A test build starts the guided test on its own once per version, and sends its setups
    /// now and then.
    fn perf_lab_test_build_frame(&mut self) {
        if self.perf_lab.played < AUTO_START_SECONDS {
            return;
        }
        let Some(app) = self.perf_lab_test_build() else {
            return;
        };
        if self.perf_lab.guided.is_none()
            && self.perf_lab.device.is_some()
            && self.perf_lab.guided_done.as_deref() != Some(app.as_str())
        {
            self.perf_lab.guided_done = Some(app.clone());
            self.perf_lab.save_choices();
            self.system_message(Text::literal(lang::get("starling.perf.guided.intro")));
            self.perf_lab_guided_start();
        }
        if self.perf_lab.played - self.perf_lab.sent_at >= SEND_EVERY_SECONDS {
            self.perf_lab.sent_at = self.perf_lab.played;
            send(self.perf_lab.report("setups", self.perf_lab_device(), &app));
        }
    }

    /// Starts the guided test from the device's own way of drawing.
    fn perf_lab_guided_start(&mut self) {
        let variants = self.perf_lab.variants();
        let tallies = variants.iter().map(|_| Setup::new(Value::Null)).collect();
        let before = self.perf_lab.choices;
        self.perf_lab.fastest = None;
        self.perf_lab.guided = Some(Guided {
            variants,
            tallies,
            step: 0,
            step_seconds: 0.0,
            before,
            told: None,
        });
        self.perf_lab.choose(Choices::default());
        log::info!("Starling: guided performance test started");
    }

    /// One counted frame of the guided test: it adds to the way of drawing in play, and tells
    /// the player where the test is, once a second, above the hotbar.
    fn perf_lab_guided_frame(&mut self, frame: f32, cpu: f32, gpu: Option<f32>) {
        let Some(g) = self.perf_lab.guided.as_mut() else {
            return;
        };
        let variant = g.variant();
        g.tallies[variant].add(frame, cpu, gpu);
        g.step_seconds += f64::from(frame) / 1000.0;
        let mut next = None;
        if g.step_seconds >= STEP_SECONDS {
            g.step += 1;
            g.step_seconds = 0.0;
            if g.step < g.steps() {
                next = Some(g.variants[g.variant()]);
            }
        }
        let finished = g.step >= g.steps();
        let tell = !finished && g.told.is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
        let line = tell.then(|| {
            g.told = Some(Instant::now());
            let left = (STEP_SECONDS - g.step_seconds).max(0.0) as u32
                + ((g.steps() - g.step - 1) as u32) * STEP_SECONDS as u32;
            lang::tr(
                "starling.perf.guided.running",
                &[
                    &(g.step + 1),
                    &g.steps(),
                    &format!("{}:{:02}", left / 60, left % 60),
                ],
            )
        });
        if let Some(choices) = next {
            self.perf_lab.choose(choices);
        }
        if let Some(line) = line {
            self.titles.set_overlay(Text::literal(line), false);
        }
        if finished {
            self.perf_lab_guided_finish();
        }
    }

    /// The guided test ends: the player's choices come back, the fastest way of drawing is
    /// offered where it beat the device's own, and a test build sends the results.
    fn perf_lab_guided_finish(&mut self) {
        let Some(g) = self.perf_lab.guided.take() else {
            return;
        };
        self.perf_lab.choose(g.before);
        let own = g.tallies[0].fps();
        let best = g
            .tallies
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.fps().total_cmp(&b.1.fps()))
            .map_or(0, |(i, _)| i);
        let best_fps = g.tallies[best].fps();
        let message = if best != 0 && best_fps > own * WORTH_IT {
            self.perf_lab.fastest = Some((g.variants[best], best_fps, own));
            lang::tr(
                "starling.perf.guided.done",
                &[
                    &self.perf_lab.describe(g.variants[best]),
                    &format!("{best_fps:.0}"),
                    &format!("{own:.0}"),
                ],
            )
        } else {
            lang::tr("starling.perf.guided.done_auto", &[&format!("{own:.0}")])
        };
        self.system_message(Text::literal(message));
        log::info!("Starling: guided performance test done");
        if let Some(app) = self.perf_lab_test_build() {
            let mut report = self.perf_lab.report("guided", self.perf_lab_device(), &app);
            report["guided"] = Value::Array(
                g.variants
                    .iter()
                    .zip(&g.tallies)
                    .map(|(c, t)| json!({"choices": c, "summary": t.summary()}))
                    .collect(),
            );
            report["setup"] = self.perf_lab_setup();
            self.perf_lab.sent_at = self.perf_lab.played;
            send(report);
            self.system_message(Text::literal(lang::get("starling.perf.guided.sent")));
        }
    }

    /// Writes `perf-sessions.json` where something changed (`now`: and waits for it, as the
    /// diagnostic export reads it next).
    pub(super) fn perf_lab_save(&mut self, now: bool) {
        self.perf_lab.saved = Some(Instant::now());
        if !self.perf_lab.dirty {
            return;
        }
        self.perf_lab.dirty = false;
        let text = match serde_json::to_vec_pretty(&self.perf_lab.file(self.perf_lab_device())) {
            Ok(text) => text,
            Err(e) => {
                log::warn!("Starling: performance test: {e}");
                return;
            }
        };
        let write = move || {
            let dir = folder();
            let path = dir.join(SESSIONS);
            let stage = path.with_extension("json.partial");
            let result = std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(&stage, &text))
                .and_then(|()| std::fs::rename(&stage, &path));
            if let Err(e) = result {
                log::warn!("Starling: performance test: {}: {e}", path.display());
            }
        };
        if now {
            write();
        } else if let Err(e) = std::thread::Builder::new()
            .name("Performance test".into())
            .spawn(write)
        {
            log::warn!("Starling: performance test: {e}");
        }
    }

    /// What each of the Diagnostics page's buttons does, in order: the guided test, the
    /// fastest way of drawing when there is one, then the four switches.
    fn perf_lab_buttons(&self) -> Vec<u8> {
        let mut ids = vec![10];
        if self.perf_lab.fastest.is_some() && self.perf_lab.guided.is_none() {
            ids.push(11);
        }
        ids.extend(0..5);
        ids
    }

    /// The Diagnostics page's button `index` was tapped.
    pub(super) fn perf_lab_next(&mut self, index: u8) {
        if !self.perf_lab.loaded {
            self.perf_lab.load();
        }
        let Some(&id) = self.perf_lab_buttons().get(usize::from(index)) else {
            return;
        };
        match id {
            10 => {
                if let Some(g) = self.perf_lab.guided.take() {
                    self.perf_lab.choose(g.before);
                    log::info!("Starling: guided performance test stopped");
                } else {
                    self.perf_lab_guided_start();
                }
                return;
            }
            11 => {
                if let Some((choices, ..)) = self.perf_lab.fastest.take() {
                    self.perf_lab.choose(choices);
                    self.perf_lab.save_choices();
                }
                return;
            }
            _ => {}
        }
        // A switch by hand ends the guided test, keeping what was set before it.
        if let Some(g) = self.perf_lab.guided.take() {
            self.perf_lab.choose(g.before);
        }
        let offers = self.perf_lab.device.map_or([false; 4], |d| d.offers);
        let mut c = self.perf_lab.choices;
        match id {
            0 => {
                let order = [Draws::Auto, Draws::Direct, Draws::Indirect, Draws::Flat];
                let at = order.iter().position(|&d| d == c.draws).unwrap_or(0);
                c.draws = (1..=order.len())
                    .map(|k| order[(at + k) % order.len()])
                    .find(|d| match d {
                        Draws::Indirect => offers[1],
                        Draws::Flat => offers[0],
                        _ => true,
                    })
                    .unwrap_or_default();
            }
            1 if offers[2] => c.late_sky = c.late_sky.next(),
            2 if offers[3] => c.occlusion = c.occlusion.next(),
            3 => c.hidden_faces = c.hidden_faces.next(),
            4 if self.perf_lab.device.is_some_and(|d| d.transparency) => {
                c.transparency = if c.transparency == Switch::Off {
                    Switch::Auto
                } else {
                    Switch::Off
                };
            }
            _ => return,
        }
        log::info!("Starling: performance test choices {c:?}");
        self.perf_lab.choose(c);
        self.perf_lab.save_choices();
    }

    /// The Diagnostics page's button labels and the lines after them.
    pub(super) fn perf_lab_page(&self) -> (Vec<String>, Vec<String>) {
        let lab = &self.perf_lab;
        let Some(device) = lab.device else {
            return (Vec::new(), Vec::new());
        };
        let word = |key: &str| lang::get(&format!("starling.perf.{key}"));
        let on_off = |on: bool| word(if on { "on" } else { "off" });
        let switch = |choice: Switch, auto: bool| match choice {
            Switch::Auto => lang::tr("starling.perf.auto", &[&on_off(auto)]),
            Switch::On => word("on"),
            Switch::Off => word("off"),
        };
        let draws_word = |flat: bool, indirect: bool| {
            word(if flat {
                "flat"
            } else if indirect {
                "indirect"
            } else {
                "direct"
            })
        };
        let mut buttons = Vec::new();
        for id in self.perf_lab_buttons() {
            buttons.push(match id {
                10 => match &lab.guided {
                    Some(g) => lang::tr("starling.perf.guided.stop", &[&(g.step + 1), &g.steps()]),
                    None => lang::tr(
                        "starling.perf.guided.start",
                        &[&((lab.variants().len() * ROUNDS) as f64 * STEP_SECONDS / 60.0).ceil()],
                    ),
                },
                11 => {
                    let (choices, fps, _) = lab.fastest.unwrap_or_default();
                    lang::tr(
                        "starling.perf.guided.use",
                        &[&lab.describe(choices), &format!("{fps:.0}")],
                    )
                }
                0 => {
                    let draws = match lab.choices.draws {
                        Draws::Auto => lang::tr(
                            "starling.perf.auto",
                            &[&draws_word(device.flat, device.indirect)],
                        ),
                        Draws::Direct => word("direct"),
                        Draws::Indirect => word("indirect"),
                        Draws::Flat => word("flat"),
                    };
                    lang::tr("starling.perf.draws", &[&draws])
                }
                1 => {
                    let sky = if device.offers[2] {
                        match lab.choices.late_sky {
                            Switch::Auto => lang::tr(
                                "starling.perf.auto",
                                &[&word(if device.late_sky { "late" } else { "early" })],
                            ),
                            Switch::On => word("late"),
                            Switch::Off => word("early"),
                        }
                    } else {
                        word("unavailable")
                    };
                    lang::tr("starling.perf.sky", &[&sky])
                }
                2 => {
                    let occlusion = if device.offers[3] {
                        switch(lab.choices.occlusion, device.occlusion)
                    } else {
                        word("unavailable")
                    };
                    lang::tr("starling.perf.occlusion", &[&occlusion])
                }
                3 => lang::tr(
                    "starling.perf.faces",
                    &[&switch(lab.choices.hidden_faces, device.hidden_faces)],
                ),
                _ => {
                    let value = if !device.transparency {
                        word("unavailable")
                    } else if lab.choices.transparency == Switch::Off {
                        word("off")
                    } else {
                        lang::tr("starling.perf.auto", &[&on_off(true)])
                    };
                    lang::tr("starling.perf.transparency", &[&value])
                }
            });
        }
        let line = |s: &Setup| {
            lang::tr(
                "starling.perf.result",
                &[
                    &format!("{:.0}", s.fps()),
                    &format!("{:.0}", s.low_fps()),
                    &format!("{:.0}", s.seconds / 60.0),
                ],
            )
        };
        let mut notes = Vec::new();
        match lab.current.and_then(|i| lab.setups.get(i)) {
            Some(s) if s.seconds >= SHOWN_AFTER => {
                notes.push(lang::tr("starling.perf.now", &[&line(s)]));
            }
            _ => notes.push(word("waiting")),
        }
        let mut best: Vec<&Setup> = lab
            .setups
            .iter()
            .filter(|s| s.seconds >= SHOWN_AFTER)
            .collect();
        best.sort_by(|a, b| b.fps().total_cmp(&a.fps()));
        for (rank, s) in best.into_iter().take(3).enumerate() {
            let renderer = &s.setup["renderer"];
            let what = format!(
                "{} · {} · {}{} · rd {} · LOD {} · {}%",
                s.setup["graphics_api"].as_str().unwrap_or("?"),
                renderer["flat_draws"]
                    .as_bool()
                    .zip(renderer["indirect_draws"].as_bool())
                    .map_or_else(|| "?".to_owned(), |(f, i)| draws_word(f, i)),
                word(if renderer["late_sky"].as_bool() == Some(true) {
                    "sky_late_short"
                } else {
                    "sky_early_short"
                }),
                if renderer["occlusion_culling"].as_bool() == Some(true) {
                    format!(" · {}", word("occlusion_short"))
                } else {
                    String::new()
                },
                s.setup["render_distance"],
                s.setup["lod_distance"],
                s.setup["resolution_scale"],
            );
            notes.push(lang::tr(
                "starling.perf.best",
                &[&(rank + 1), &line(s), &what],
            ));
        }
        notes.push(word("share"));
        (buttons, notes)
    }
}

impl PerfLab {
    /// Plays `setup` from now on: the one already kept with the same settings, else a new one.
    fn enter(&mut self, setup: Value) {
        if self
            .current
            .and_then(|i| self.setups.get(i))
            .is_some_and(|s| s.setup == setup)
        {
            return;
        }
        let index = match self.setups.iter().position(|s| s.setup == setup) {
            Some(i) => i,
            None => {
                if self.setups.len() >= KEEP
                    && let Some((oldest, _)) =
                        self.setups.iter().enumerate().min_by_key(|(_, s)| s.last)
                {
                    self.setups.remove(oldest);
                }
                self.setups.push(Setup::new(setup));
                self.setups.len() - 1
            }
        };
        self.current = Some(index);
    }
}

/// How fast a calibration goes: the wait after a switch before its frames count, and each
/// block's seconds (`bench` takes whole seconds).
#[derive(Clone, Copy)]
struct Pace {
    warmup: Duration,
    block: u32,
}
/// About a minute of blocks for the six ways of drawing a phone can have.
const PACE: Pace = Pace {
    warmup: Duration::from_secs(1),
    block: 3,
};
/// How high over the spawn the camera holds, and where it looks: out over the land.
const CALIBRATE_LIFT: f64 = 24.0;
const CALIBRATE_YAW: f64 = 0.0;
const CALIBRATE_PITCH: f64 = 15.0;
/// How long the game waits, back on the title screen, for the test world's last save before it
/// gives up deleting the world (it stays out of the world list all the same).
const CALIBRATE_CLEANUP: Duration = Duration::from_secs(30);
/// The run's error when the game asked it to stop (`CalibrationState::stop`).
const STOPPED: &str = "stopped";

/// What the calibration thread and the game tell each other.
#[derive(Default)]
struct CalibrationState {
    /// The way of drawing the run times next: the game takes it on its next frame.
    wanted: Option<Choices>,
    /// The way of drawing the game took last: it draws with it from its next frame.
    drawn: Option<Choices>,
    /// The test world is loaded and the run times it: a menu, a pause or leaving the game
    /// stops it from here on.
    measuring: bool,
    /// Blocks done, blocks in all and when the last should end, for the line over the hotbar.
    progress: Option<(usize, usize, Instant)>,
    /// The game asks the run to stop.
    stop: bool,
    /// What the run found, or why it stopped: the game leaves the test world, deletes it, and
    /// then puts it in use.
    finished: Option<Result<Calibrated, String>>,
}

type Shared = Arc<Mutex<CalibrationState>>;

fn lock(shared: &Shared) -> MutexGuard<'_, CalibrationState> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// Calibrate... under way, on the game's side.
struct Calibration {
    shared: Shared,
    /// The player's choices before it started, given back when it stops.
    before: Choices,
    told: Option<Instant>,
    /// The game has left the test world for the title screen.
    left: bool,
    /// What the run found, held until the test world is deleted, and since when the game has
    /// been back on the title screen to delete it.
    result: Option<Result<Calibrated, String>>,
    on_title: Option<Instant>,
}

/// What a calibration times on this device: its own way of drawing first, then each single
/// change from it that shows the same picture. Improved Transparency stays as the player has it
/// throughout: turning it off changes the picture.
fn calibration_candidates(d: Device, keep: Choices) -> Vec<Choices> {
    let base = Choices {
        transparency: keep.transparency,
        ..Choices::default()
    };
    let flip = |on: bool| if on { Switch::Off } else { Switch::On };
    let mut candidates = vec![base];
    let mut terrain = |draws: Draws| candidates.push(Choices { draws, ..base });
    if d.flat || d.indirect {
        terrain(Draws::Direct);
    }
    if d.offers[1] && !d.indirect {
        terrain(Draws::Indirect);
    }
    if d.offers[0] && !d.flat {
        terrain(Draws::Flat);
    }
    if d.offers[2] {
        candidates.push(Choices {
            late_sky: flip(d.late_sky),
            ..base
        });
    }
    if d.offers[3] {
        candidates.push(Choices {
            occlusion: flip(d.occlusion),
            ..base
        });
    }
    candidates.push(Choices {
        hidden_faces: flip(d.hidden_faces),
        ..base
    });
    candidates
}

fn mean(rates: &[f64]) -> f64 {
    if rates.is_empty() {
        0.0
    } else {
        rates.iter().sum::<f64>() / rates.len() as f64
    }
}

/// What the first round of blocks found (`fps[i]`: candidate `i`'s frame rates, the first
/// being the device's own way): every change that beat the device's own by [`WORTH_IT`] at
/// once, the fastest terrain draws among them, and the fastest of those changes alone (none
/// beat it: `None`).
fn combine(candidates: &[Choices], fps: &[Vec<f64>]) -> (Choices, Option<usize>) {
    let base = candidates[0];
    let own = mean(&fps[0]);
    let (mut both, mut single, mut draws_fps) = (base, None::<usize>, 0.0);
    for (i, c) in candidates.iter().enumerate().skip(1) {
        let rate = mean(&fps[i]);
        let beats = rate > own * WORTH_IT;
        if !beats {
            continue;
        }
        if single.is_none_or(|s| rate > mean(&fps[s])) {
            single = Some(i);
        }
        if c.draws != base.draws && rate > draws_fps {
            (both.draws, draws_fps) = (c.draws, rate);
        }
        if c.late_sky != base.late_sky {
            both.late_sky = c.late_sky;
        }
        if c.occlusion != base.occlusion {
            both.occlusion = c.occlusion;
        }
        if c.hidden_faces != base.hidden_faces {
            both.hidden_faces = c.hidden_faces;
        }
    }
    (both, single)
}

/// Times one block of `choices`: asks the game to draw with it, waits for it to settle where it
/// `switched`, and returns the block's frame rate.
fn calibration_block(
    control: &Control,
    shared: &Shared,
    choices: Choices,
    switched: bool,
    pace: Pace,
) -> Result<f64, String> {
    lock(shared).wanted = Some(choices);
    let asked = Instant::now();
    loop {
        {
            let s = lock(shared);
            if s.stop {
                return Err(STOPPED.into());
            }
            if s.wanted.is_none() && s.drawn == Some(choices) {
                break;
            }
        }
        if asked.elapsed() > Duration::from_secs(10) {
            return Err("the game did not switch how it draws".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    if switched {
        thread::sleep(pace.warmup);
    }
    let reply = control.call(&format!("bench {} raw", pace.block))?;
    if lock(shared).stop {
        return Err(STOPPED.into());
    }
    Ok(benchmark::frames_of(&reply, "calibrate", 0)?.avg_fps)
}

/// Times every candidate twice, forth then back so a phone warming up slows both ends alike,
/// and, where more than one change beat the device's own, those changes together against the
/// fastest alone, B A A B. Returns the fastest way of drawing, its frame rate and the device's
/// own.
fn calibration_rounds(
    control: &Control,
    shared: &Shared,
    candidates: &[Choices],
    pace: Pace,
) -> Result<(Choices, f64, f64), String> {
    let n = candidates.len();
    let order: Vec<usize> = (0..n).chain((0..n).rev()).collect();
    let steps = order.len() + if n > 2 { 4 } else { 0 };
    let per_block = pace.warmup.as_secs_f64() + f64::from(pace.block);
    let tell = |done: usize| {
        let left = Duration::from_secs_f64((steps - done) as f64 * per_block);
        lock(shared).progress = Some((done, steps, Instant::now() + left));
    };
    let mut fps = vec![Vec::new(); n];
    let mut last = None;
    for (done, &i) in order.iter().enumerate() {
        tell(done);
        let c = candidates[i];
        fps[i].push(calibration_block(
            control,
            shared,
            c,
            last != Some(c),
            pace,
        )?);
        last = Some(c);
    }
    let own = mean(&fps[0]);
    let (both, single) = combine(candidates, &fps);
    let Some(single) = single else {
        return Ok((candidates[0], own, own));
    };
    let alone = candidates[single];
    if both == alone {
        return Ok((alone, mean(&fps[single]), own));
    }
    let mut rates = [Vec::new(), Vec::new()];
    for (k, which) in [0, 1, 1, 0].into_iter().enumerate() {
        tell(order.len() + k);
        let c = if which == 0 { both } else { alone };
        rates[which].push(calibration_block(
            control,
            shared,
            c,
            last != Some(c),
            pace,
        )?);
        last = Some(c);
    }
    let (b, a) = (mean(&rates[0]), mean(&rates[1]));
    Ok(if b > a {
        (both, b, own)
    } else {
        (alone, a, own)
    })
}

/// The run, from the title screen: the test world afresh, the camera held over its spawn, then
/// the rounds.
fn calibrate(
    control: &Control,
    shared: &Shared,
    game_dir: &Path,
    candidates: &[Choices],
    pace: Pace,
) -> Result<(Choices, f64, f64), String> {
    benchmark::open_world(control, game_dir)?;
    benchmark::hold_weather(control)?;
    let at: Vec<f64> = control.status()["player"]["position"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    let &[x, y, z] = at.as_slice() else {
        return Err("no player in the test world".into());
    };
    benchmark::hold(
        control,
        [x, y + CALIBRATE_LIFT, z],
        CALIBRATE_YAW,
        CALIBRATE_PITCH,
    )?;
    lock(shared).measuring = true;
    thread::sleep(pace.warmup);
    calibration_rounds(control, shared, candidates, pace)
}

/// The calibration thread: the run; the game then leaves the test world and deletes it,
/// whatever happened in it.
fn calibration_thread(
    control: Control,
    shared: Shared,
    game_dir: PathBuf,
    candidates: Vec<Choices>,
) {
    let started = Instant::now();
    let found = calibrate(&control, &shared, &game_dir, &candidates, PACE);
    let result = found.map(|(choices, fps, auto_fps)| Calibrated {
        choices,
        fps: round(fps),
        auto_fps: round(auto_fps),
        seconds: started.elapsed().as_secs_f64().round(),
    });
    let mut s = lock(&shared);
    s.measuring = false;
    s.wanted = None;
    s.finished = Some(result);
}

/// Deletes the test world once nothing has it open: as Select World's Delete does, where the
/// session lock lets it, else the folder itself (`benchmark::replace_world`). Only ever the
/// world the benchmark marked as its own; true once it is gone.
fn delete_test_world(saves: &Path) -> bool {
    let world = saves.join(benchmark::WORLD);
    if !world.exists() {
        return true;
    }
    if !world.join(benchmark::MARKER).exists() {
        log::warn!(
            "Starling: calibration: {} is not the test world: it stays",
            world.display()
        );
        return true;
    }
    if let Err(e) = sunrise_client::title::world_management::delete(saves, benchmark::WORLD) {
        log::warn!("Starling: calibration: deleting the test world: {e}");
        let game_dir = saves.parent().unwrap_or(saves);
        if let Err(e) = benchmark::replace_world(game_dir) {
            log::warn!("Starling: calibration: deleting the test world: {e}");
        }
    }
    !world.exists()
}

/// Minutes and seconds, as the calibration's lines say them.
fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl Game<'_> {
    /// Why Calibrate... can't start now: the language key of the reason.
    fn perf_calibrate_blocked(&self) -> Option<&'static str> {
        if self.perf_lab.calibration.is_some() {
            Some("starling.calibrate.busy")
        } else if !self.res.rest_ready() || self.perf_lab.device.is_none() {
            // A world can't open before the items have loaded.
            Some("starling.calibrate.wait_loading")
        } else if self.title.is_none() {
            Some("starling.calibrate.leave_world")
        } else {
            None
        }
    }

    /// What each of the calibration page's buttons does, in order: run it (0), where it can,
    /// and back to Auto (1), where a way of drawing is chosen.
    fn perf_calibrate_buttons(&self) -> Vec<u8> {
        let mut ids = Vec::new();
        if self.perf_calibrate_blocked().is_none() {
            ids.push(0);
        }
        let c = self.perf_lab.choices;
        let auto = Choices {
            transparency: c.transparency,
            ..Choices::default()
        };
        if c != auto && self.perf_lab.calibration.is_none() {
            ids.push(1);
        }
        ids
    }

    /// The calibration page's button `index` was tapped.
    pub(super) fn perf_calibrate_button(&mut self, index: u8) {
        if !self.perf_lab.loaded {
            self.perf_lab.load();
        }
        match self.perf_calibrate_buttons().get(usize::from(index)) {
            Some(0) => self.perf_calibrate_start(),
            Some(1) => {
                let c = self.perf_lab.choices;
                self.perf_lab.choose(Choices {
                    transparency: c.transparency,
                    ..Choices::default()
                });
                self.perf_lab.calibrated = None;
                self.perf_lab.calibrate_stopped = None;
                self.perf_lab.save_choices();
                log::info!("Starling: calibration: back to Auto");
            }
            _ => {}
        }
    }

    /// Starts Calibrate...: the options close, and its thread opens the test world over the
    /// title screen through the game's own command queue.
    fn perf_calibrate_start(&mut self) {
        if self.perf_calibrate_blocked().is_some() {
            return;
        }
        let Some(device) = self.perf_lab.device else {
            return;
        };
        let game_dir = match std::env::current_dir() {
            Ok(dir) => dir,
            Err(e) => {
                log::warn!("Starling: calibration: {e}");
                return;
            }
        };
        // A guided test under way ends: the calibration answers the same question.
        let before = match self.perf_lab.guided.take() {
            Some(g) => g.before,
            None => self.perf_lab.choices,
        };
        let candidates = calibration_candidates(device, before);
        log::info!("Starling: calibration started: {candidates:?}");
        let shared = Shared::default();
        let control = Control(
            self.control
                .get_or_insert_with(sunrise_client::control::Server::local)
                .sender(),
        );
        let theirs = shared.clone();
        let spawned = thread::Builder::new()
            .name("Calibration".into())
            .spawn(move || calibration_thread(control, theirs, game_dir, candidates));
        if let Err(e) = spawned {
            log::warn!("Starling: calibration: {e}");
            return;
        }
        self.perf_lab.calibrate_stopped = None;
        self.perf_lab.calibration = Some(Calibration {
            shared,
            before,
            told: None,
            left: false,
            result: None,
            on_title: None,
        });
        if self.screen == Screen::Options
            && let Err(e) =
                self.handle_option_events(vec![sunrise_client::options_ui::Event::Close])
        {
            log::warn!("Starling: calibration: {e}");
        }
    }

    /// Whether Calibrate... runs: the touch controls hide and take no finger meanwhile.
    pub(super) fn perf_calibrating(&self) -> bool {
        self.perf_lab.calibration.is_some()
    }

    /// Each frame while Calibrate... runs: the player counts as present (no idle frame cap), the
    /// way of drawing it asks for, its stop when the player opens a menu, pauses or leaves the
    /// game, its line over the hotbar once a second, and once it is done, out of the test world,
    /// the world deleted, and its end.
    fn perf_calibrate_frame(&mut self) {
        let Some(shared) = self.perf_lab.calibration.as_ref().map(|c| c.shared.clone()) else {
            return;
        };
        let now = Instant::now();
        // The camera is held and the fingers ignored: Inactivity FPS Limit's AFK would cap the
        // frames it times, as a held key keeps it off (touch.rs).
        self.last_input = now;
        let in_world = self.player.is_some() && self.title.is_none();
        let away = self.pause || self.screen != Screen::Playing || !self.focused;
        let (progress, finished) = {
            let mut s = lock(&shared);
            if let Some(choices) = s.wanted.take() {
                self.perf_lab.choose(choices);
                s.drawn = Some(choices);
            }
            if s.measuring && in_world && away {
                s.stop = true;
            }
            (s.progress, s.finished.take())
        };
        let on_title = self.title.is_some();
        // The world is closed and saved: nothing holds it open any more.
        let closed = on_title && self.pending_save.is_none() && self.integrated.is_none();
        let Some(run) = self.perf_lab.calibration.as_mut() else {
            return;
        };
        if finished.is_some() {
            run.result = finished;
        }
        let done = run.result.is_some();
        let out = done && in_world && !run.left;
        if out {
            run.left = true;
        }
        let tell = in_world
            && !done
            && run
                .told
                .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1));
        if tell {
            run.told = Some(now);
        }
        // Back on the title screen: the test world goes once its last save is done, tried once
        // a second.
        let since = if done && on_title {
            *run.on_title.get_or_insert(now)
        } else {
            now
        };
        let clean = done
            && on_title
            && (closed || now.duration_since(since) >= CALIBRATE_CLEANUP)
            && run
                .told
                .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1));
        if clean {
            run.told = Some(now);
        }
        if out {
            self.perf_calibrate_leave_world();
        }
        if tell {
            let line = match progress {
                Some((done, steps, end)) => lang::tr(
                    "starling.calibrate.running",
                    &[
                        &(done + 1),
                        &steps,
                        &clock(end.saturating_duration_since(now).as_secs_f64().ceil() as u32),
                    ],
                ),
                None => lang::get("starling.calibrate.loading"),
            };
            self.titles.set_overlay(Text::literal(line), false);
        }
        if clean {
            let saves = Self::game_dir().join("saves");
            let gone = delete_test_world(&saves);
            let late = now.duration_since(since) >= CALIBRATE_CLEANUP;
            if gone || late {
                if !gone {
                    log::warn!("Starling: calibration: the test world stays (hidden)");
                }
                if let Some(title) = self.title.as_mut() {
                    title.worlds_changed();
                }
                let result = self
                    .perf_lab
                    .calibration
                    .as_mut()
                    .and_then(|run| run.result.take());
                if let Some(result) = result {
                    self.perf_calibrate_done(result);
                }
            }
        }
    }

    /// The run is over and the test world gone: what it found is put in use and saved (or the
    /// player's choices come back), and its page opens again over the title screen.
    fn perf_calibrate_done(&mut self, result: Result<Calibrated, String>) {
        let Some(run) = self.perf_lab.calibration.take() else {
            return;
        };
        match result {
            Ok(found) => {
                log::info!("Starling: calibration done: {found:?}");
                self.perf_lab.choose(found.choices);
                self.perf_lab.calibrated = Some(found);
                self.perf_lab.calibrate_stopped = None;
            }
            Err(e) => {
                log::warn!("Starling: calibration stopped: {e}");
                self.perf_lab.choose(run.before);
                self.perf_lab.calibrate_stopped = Some(e);
            }
        }
        self.perf_lab.save_choices();
        if self.title.is_some() {
            self.open_options_at(true, Page::PerfCalibration);
        }
    }

    /// The calibration page's lines and buttons, while it shows: before the options lay out
    /// and draw (`draw_options`), as the Diagnostics page's, so a tap lands on what shows.
    pub(super) fn perf_calibrate_page(&mut self) {
        if self.screen != Screen::Options || self.ui.options.page() != Page::PerfCalibration {
            return;
        }
        let lab = &self.perf_lab;
        let mut notes = Vec::new();
        let calibrated = |c: Choices| Choices {
            transparency: Switch::Auto,
            ..c
        };
        match (&lab.calibrate_stopped, lab.calibrated) {
            (Some(why), _) if why == STOPPED => {
                notes.push(lang::get("starling.calibrate.stopped_by_you"));
            }
            (Some(why), _) => notes.push(lang::tr("starling.calibrate.stopped", &[&why])),
            (None, Some(found)) if calibrated(found.choices) == Choices::default() => {
                notes.push(lang::tr(
                    "starling.calibrate.done_auto",
                    &[
                        &clock(found.seconds as u32),
                        &format!("{:.0}", found.auto_fps),
                    ],
                ));
            }
            (None, Some(found)) => notes.push(lang::tr(
                "starling.calibrate.done",
                &[
                    &clock(found.seconds as u32),
                    &lab.describe(calibrated(found.choices)),
                    &format!("{:.0}", found.fps),
                    &format!("{:.0}", found.auto_fps),
                ],
            )),
            (None, None) => notes.push(lang::get("starling.calibrate.never")),
        }
        if let Some(why) = self.perf_calibrate_blocked() {
            notes.push(lang::get(why));
        }
        let buttons = self
            .perf_calibrate_buttons()
            .into_iter()
            .map(|id| match id {
                0 if lab.calibrated.is_some() => lang::get("starling.calibrate.again"),
                0 => lang::get("starling.calibrate.start"),
                _ => lang::get("starling.calibrate.auto"),
            })
            .collect();
        self.ui.options.set_calibrate(notes, buttons);
    }

    /// Out of the test world for the title screen, as the pause menu's Save and Quit to Title
    /// (`execute_pause_action`): the run waits for the title, then deletes the world.
    fn perf_calibrate_leave_world(&mut self) {
        if let Err(e) = self.close_world() {
            log::warn!("Starling: calibration: leaving the test world: {e}");
        }
    }
}

#[cfg(test)]
mod calibration_tests {
    use super::*;
    use std::sync::mpsc;
    use sunrise_client::control::Request;

    /// A phone that draws flat by default and takes every other way, the sky early, occlusion
    /// culling and hidden faces skipped.
    const PHONE: Device = Device {
        flat: true,
        indirect: false,
        late_sky: false,
        occlusion: true,
        hidden_faces: true,
        transparency: true,
        offers: [true, true, true, true],
    };

    fn quick() -> Pace {
        Pace {
            warmup: Duration::ZERO,
            block: 1,
        }
    }

    #[test]
    fn calibration_tries_each_change_that_keeps_the_picture_once() {
        let keep = Choices {
            draws: Draws::Indirect,
            transparency: Switch::Off,
            ..Choices::default()
        };
        let c = calibration_candidates(PHONE, keep);
        let base = Choices {
            transparency: Switch::Off,
            ..Choices::default()
        };
        assert_eq!(
            c,
            vec![
                base,
                Choices {
                    draws: Draws::Direct,
                    ..base
                },
                Choices {
                    draws: Draws::Indirect,
                    ..base
                },
                Choices {
                    late_sky: Switch::On,
                    ..base
                },
                Choices {
                    occlusion: Switch::Off,
                    ..base
                },
                Choices {
                    hidden_faces: Switch::Off,
                    ..base
                },
            ],
            "from the device's own; the player's Improved Transparency throughout"
        );
        let bare = Device {
            offers: [false; 4],
            flat: false,
            ..PHONE
        };
        assert_eq!(
            calibration_candidates(bare, Choices::default()),
            vec![
                Choices::default(),
                Choices {
                    hidden_faces: Switch::Off,
                    ..Choices::default()
                }
            ],
            "what the device doesn't take isn't tried"
        );
    }

    #[test]
    fn calibration_keeps_each_change_that_beat_the_default_by_the_margin() {
        let c = calibration_candidates(PHONE, Choices::default());
        // Auto, direct, indirect, late sky, no occlusion, hidden faces drawn.
        let fps: Vec<Vec<f64>> = [50.0, 49.0, 56.0, 53.0, 40.0, 50.5]
            .iter()
            .map(|&f| vec![f, f])
            .collect();
        let (both, single) = combine(&c, &fps);
        assert_eq!(
            both,
            Choices {
                draws: Draws::Indirect,
                late_sky: Switch::On,
                ..Choices::default()
            },
            "1% faster is within the margin"
        );
        assert_eq!(single, Some(2), "indirect is the fastest change alone");
        // Two terrain draws that both beat it: the faster.
        let fps: Vec<Vec<f64>> = [50.0, 55.0, 60.0, 50.0, 50.0, 50.0]
            .iter()
            .map(|&f| vec![f])
            .collect();
        assert_eq!(combine(&c, &fps).0.draws, Draws::Indirect);
        // Nothing beats the default by 3%: it stays.
        let fps = vec![vec![60.0, 58.0]; c.len()];
        assert_eq!(combine(&c, &fps), (Choices::default(), None));
    }

    #[test]
    fn a_saved_calibration_comes_back_and_older_files_still_read() {
        let found = Calibrated {
            choices: Choices {
                draws: Draws::Flat,
                late_sky: Switch::On,
                ..Choices::default()
            },
            fps: 58.0,
            auto_fps: 51.0,
            seconds: 84.0,
        };
        let saved = Saved {
            choices: found.choices,
            guided_done: None,
            calibrated: Some(found),
        };
        let text = serde_json::to_string(&saved).unwrap();
        let back: Saved = serde_json::from_str(&text).unwrap();
        assert_eq!(
            (back.choices, back.calibrated),
            (found.choices, Some(found))
        );
        let old: Saved = serde_json::from_str(r#"{"draws":"direct","late_sky":"on"}"#).unwrap();
        assert_eq!(old.choices.draws, Draws::Direct);
        assert_eq!(old.calibrated, None);
    }

    /// The game for the rounds: takes the way of drawing asked for, as its frames do, and draws
    /// each one at `rate`'s frame rate.
    fn rounds_game(
        rx: mpsc::Receiver<Request>,
        shared: Shared,
        rate: fn(Choices) -> f64,
    ) -> thread::JoinHandle<Vec<Choices>> {
        thread::spawn(move || {
            let mut timed = Vec::new();
            loop {
                {
                    let mut s = lock(&shared);
                    if let Some(c) = s.wanted.take() {
                        s.drawn = Some(c);
                    }
                }
                match rx.recv_timeout(Duration::from_millis(5)) {
                    Ok(request) => {
                        let drawn = lock(&shared).drawn.unwrap_or_default();
                        assert_eq!(request.line, "bench 1 raw");
                        timed.push(drawn);
                        let frames = (rate(drawn) as usize).max(1);
                        let reply = json!({
                            "frames": frames,
                            "frame_ms": vec![1000.0 / rate(drawn); frames],
                            "cpu_ms": vec![1.0; frames],
                            "gpu_ms": vec![1.0; frames],
                        });
                        let _ = request.reply.send(reply.to_string());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return timed,
                }
            }
        })
    }

    #[test]
    fn the_rounds_time_each_way_forth_and_back_and_keep_the_fastest_together() {
        let c = calibration_candidates(PHONE, Choices::default());
        let shared = Shared::default();
        let (tx, rx) = mpsc::channel();
        // Indirect and the late sky each help; together they help most.
        let game = rounds_game(rx, shared.clone(), |c| {
            let mut fps = 50.0;
            if c.draws == Draws::Indirect {
                fps += 6.0;
            }
            if c.late_sky == Switch::On {
                fps += 3.0;
            }
            fps
        });
        let control = Control(tx);
        let found = calibration_rounds(&control, &shared, &c, quick()).unwrap();
        drop(control);
        let timed = game.join().unwrap();
        let both = Choices {
            draws: Draws::Indirect,
            late_sky: Switch::On,
            ..Choices::default()
        };
        let alone = Choices {
            draws: Draws::Indirect,
            ..Choices::default()
        };
        let mut expected: Vec<Choices> = c.iter().chain(c.iter().rev()).copied().collect();
        expected.extend([both, alone, alone, both]);
        assert_eq!(timed, expected);
        assert_eq!(found.0, both);
        assert!((found.1 - 59.0).abs() < 0.5 && (found.2 - 50.0).abs() < 0.5);
        let (done, steps, _) = lock(&shared).progress.unwrap();
        assert_eq!((done, steps), (15, 16));
    }

    #[test]
    fn a_stop_ends_the_rounds_and_nothing_is_kept() {
        let c = calibration_candidates(PHONE, Choices::default());
        let shared = Shared::default();
        lock(&shared).stop = true;
        let (tx, rx) = mpsc::channel();
        let game = rounds_game(rx, shared.clone(), |_| 60.0);
        let control = Control(tx);
        assert_eq!(
            calibration_rounds(&control, &shared, &c, quick()),
            Err(STOPPED.to_owned())
        );
        drop(control);
        assert!(game.join().unwrap().is_empty(), "nothing was timed");
    }
}
