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
use super::{Game, Screen};
use horizon_client::{lang, text::Text};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// `performance/perf-lab.json`: the choices, and the test build whose guided test ran.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    #[serde(flatten)]
    choices: Choices,
    guided_done: Option<String>,
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
    horizon_client::logging::private_home().join("performance")
}

/// This APK's version where it is a test build: a Starling Preview, never a release.
fn test_build() -> Option<String> {
    #[cfg(target_os = "android")]
    {
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
    fn apply(&mut self, r: &mut horizon_client::gfx::Renderer) {
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
    }

    /// What decides the frame rate, as the player set it and the renderer took it.
    fn perf_lab_setup(&self) -> Value {
        let s = &self.settings;
        let h = &s.horizon;
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
