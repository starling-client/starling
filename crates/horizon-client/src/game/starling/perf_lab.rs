//! Starling's performance test (the 0.8.1 test builds): the renderer's draw choices on the
//! Diagnostics page, and the average frame rate of each setup the player plays with.
//!
//! A frame counts only in a world with no screen, menu or container open, the game focused and
//! not paused, and some input in the last 30 seconds: menus and an idle phone don't pull the
//! averages around. Each setup (the draw choices, the graphics settings, the window's size) adds
//! up its own frames, so going back to a setup adds to it. `performance/perf-sessions.json`
//! keeps them across starts, the diagnostic ZIP takes it, and the Diagnostics page shows the
//! setup in play and the best ones.
use super::{Game, Screen};
use horizon_client::lang;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SESSIONS: &str = "perf-sessions.json";
const CHOICES: &str = "perf-lab.json";
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

/// The player's choices, saved in `performance/perf-lab.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Choices {
    draws: Draws,
    late_sky: Switch,
    occlusion: Switch,
    /// Solid quads facing away from the eye left out (`Renderer::section_sides`).
    hidden_faces: Switch,
}

/// The renderer's own choices before the test touched them, and what this device takes.
#[derive(Clone, Copy, Debug)]
struct Device {
    flat: bool,
    indirect: bool,
    late_sky: bool,
    occlusion: bool,
    hidden_faces: bool,
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
        if ms > 0.0 { 1000.0 / f64::from(ms) } else { 0.0 }
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

#[derive(Default)]
pub(super) struct PerfLab {
    loaded: bool,
    choices: Choices,
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

impl PerfLab {
    fn load(&mut self) {
        self.loaded = true;
        let dir = folder();
        if let Ok(text) = std::fs::read_to_string(dir.join(CHOICES)) {
            self.choices = serde_json::from_str(&text).unwrap_or_default();
        }
        if let Ok(text) = std::fs::read_to_string(dir.join(SESSIONS))
            && let Ok(file) = serde_json::from_str::<Value>(&text)
            && let Some(setups) = file.get("setups")
        {
            self.setups = serde_json::from_value(setups.clone()).unwrap_or_default();
        }
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
    }

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
                let system = self.stats_view().system;
                if let Some(s) = self.perf_lab.current.and_then(|i| self.perf_lab.setups.get_mut(i))
                {
                    let max_i = |a: Option<i32>, b: Option<i32>| a.max(b);
                    let max_f = |a: Option<f32>, b: Option<f32>| match (a, b) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (a, b) => a.or(b),
                    };
                    s.thermal_max = max_i(s.thermal_max, system.thermal_status);
                    s.battery_max_c = max_f(s.battery_max_c, system.battery_celsius);
                    s.cpu_max_c = max_f(s.cpu_max_c, system.cpu_celsius);
                }
            }
            if now.duration_since(since) >= SETTLE
                && let Some((frame, cpu, gpu)) = sample
                && frame < HITCH_MS
                && let Some(s) = self.perf_lab.current.and_then(|i| self.perf_lab.setups.get_mut(i))
            {
                s.add(frame, cpu, gpu);
                s.last = unix_now();
                self.perf_lab.dirty = true;
            }
        }
        if self
            .perf_lab
            .saved
            .is_none_or(|t| now.duration_since(t) >= SAVE_EVERY)
        {
            self.perf_lab_save(false);
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

    /// The Diagnostics page's switch `index` goes to its next choice, skipping the ones this
    /// device doesn't take.
    pub(super) fn perf_lab_next(&mut self, index: u8) {
        if !self.perf_lab.loaded {
            self.perf_lab.load();
        }
        let offers = self.perf_lab.device.map_or([false; 4], |d| d.offers);
        let c = &mut self.perf_lab.choices;
        match index {
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
            _ => return,
        }
        let choices = self.perf_lab.choices;
        log::info!("Starling: performance test choices {choices:?}");
        // The frames from here on are another setup's.
        self.perf_lab.checked = None;
        self.perf_lab.current = None;
        let text = serde_json::to_vec_pretty(&choices).unwrap_or_default();
        let dir = folder();
        if let Err(e) =
            std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(CHOICES), text))
        {
            log::warn!("Starling: performance test: {e}");
        }
    }

    /// The Diagnostics page's switch labels and the lines after them.
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
        let draws = match lab.choices.draws {
            Draws::Auto => lang::tr(
                "starling.perf.auto",
                &[&draws_word(device.flat, device.indirect)],
            ),
            Draws::Direct => word("direct"),
            Draws::Indirect => word("indirect"),
            Draws::Flat => word("flat"),
        };
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
        let occlusion = if device.offers[3] {
            switch(lab.choices.occlusion, device.occlusion)
        } else {
            word("unavailable")
        };
        let buttons = vec![
            lang::tr("starling.perf.draws", &[&draws]),
            lang::tr("starling.perf.sky", &[&sky]),
            lang::tr("starling.perf.occlusion", &[&occlusion]),
            lang::tr(
                "starling.perf.faces",
                &[&switch(lab.choices.hidden_faces, device.hidden_faces)],
            ),
        ];
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
