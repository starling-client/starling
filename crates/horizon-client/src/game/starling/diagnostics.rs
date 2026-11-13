//! Horizon Android's diagnostic reports, integrated with Starling's game settings.
use super::Game;
use horizon_client::{lang, logging};
use serde_json::{Value, json};
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};

static EXPORTING: AtomicBool = AtomicBool::new(false);
static EXPORTED: Mutex<Option<Result<std::path::PathBuf, String>>> = Mutex::new(None);
static NOTICE: Mutex<Option<&'static str>> = Mutex::new(None);

impl Game<'_> {
    pub(super) fn diagnostic_context(&mut self) -> Value {
        let mut context = self.stats_view().json();
        context["focused"] = json!(self.focused);
        context["paused"] = json!(self.pause);
        context["resolution"] = self
            .renderer
            .as_ref()
            .map(|r| json!([r.config.width, r.config.height]))
            .unwrap_or(Value::Null);
        if let Some(r) = &self.renderer {
            context["driver"] = json!({"name":r.info.driver,"details":r.info.driver_info,"vendor":r.info.vendor,"device":r.info.device});
            context["graphics_features"] = json!(format!("{:?}", r.device.features()));
            context["graphics_limits"] = json!(format!("{:?}", r.device.limits()));
        }
        context["runtime"] = json!({"simulation_distance":self.settings.simulation_distance,
            "fps_limit":self.framerate_limit(), "in_world":self.player.is_some(),
            "playing_screen":self.screen == super::Screen::Playing,"idle_seconds":self.last_input.elapsed().as_secs(),
            "underwater":self.ambience.was_underwater(),"mobs":self.mobs.len(),"particles":self.particles.len(),
            "dirty_light":self.world.dirty_light.len(),"remote_backlog":self.remote_backlog(),
            "streaming":self.streaming.stages().into_iter().collect::<std::collections::BTreeMap<_,_>>()});
        if let Some(r) = &self.renderer {
            let [arena, vertices] = r.section_vertex_bytes();
            context["renderer"] = json!({"present":format!("{:?}",r.config.present_mode),
                "wait_ms":r.last_wait.as_secs_f32()*1000.0,"resident_sections":r.resident_sections(),
                "uploaded_bytes_total":r.uploaded_bytes,"arena_bytes":arena,"section_vertex_bytes":vertices,
                "gpu_profile_requested":crate::performance::gpu_profile(),
                "gpu_timestamp_supported":r.device.features().contains(wgpu::Features::TIMESTAMP_QUERY)});
        }
        context
    }

    #[cfg(target_os = "android")]
    pub(super) fn diagnostic_heartbeat(&self) {
        let phase = if !self.focused {
            "background"
        } else if self.rest_loader.is_some() || self.pending_entry.is_some() {
            "world_loading"
        } else if self.title.is_some() {
            "title"
        } else {
            "in_world"
        };
        crate::diagnostics::heartbeat(phase, || {
            json!({"focused":self.focused,"paused":self.pause,
            "gpu":self.renderer.as_ref().map(|r|r.info.name.clone()),
            "backend":self.renderer.as_ref().map(|r|format!("{:?}",r.info.backend))})
        });
    }

    pub(super) fn poll_diagnostics(&mut self) {
        self.ui.options.diagnostics_status = status();
        if let Some(r) = &self.renderer {
            self.ui.options.diagnostics_status.push(lang::tr(
                "starling.diagnostics.hardware",
                &[&r.info.name, &format!("{:?}", r.info.backend)],
            ));
        }
    }

    pub(super) fn toggle_diagnostic_recording(&mut self) {
        if crate::performance::active() {
            crate::performance::stop();
        } else {
            let metadata = self.diagnostic_context();
            if let Err(error) = crate::performance::start(&logging::private_home(), metadata) {
                log::warn!("Cannot start performance recording: {error}");
                *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some("starling.diagnostics.failed");
            }
        }
    }

    pub(super) fn toggle_diagnostic_gpu(&mut self) {
        crate::performance::configure_gpu_profile(
            !crate::performance::gpu_profile(),
            &logging::private_home(),
        );
    }

    pub(super) fn export_diagnostics(&mut self) {
        export_diagnostics();
    }
    pub(super) fn open_diagnostics(&mut self) {
        open_diagnostics();
    }
}
pub(super) fn export_diagnostics() {
    if EXPORTING.swap(true, Ordering::Relaxed) {
        return;
    }
    let home = logging::private_home();
    if let Err(error) = std::thread::Builder::new()
        .name("Diagnostic export".into())
        .spawn(move || {
            logging::flush();
            let result = crate::diagnostics::export(&home);
            *EXPORTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
            EXPORTING.store(false, Ordering::Relaxed);
        })
    {
        EXPORTING.store(false, Ordering::Relaxed);
        log::warn!("Cannot start diagnostic export: {error}");
    }
}

pub(super) fn open_diagnostics() {
    let dir = logging::private_home().join("performance");
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(target_os = "android")]
    let opened = std::fs::read_dir(&dir)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.starts_with("starling-diagnostics-") && n.ends_with(".zip"))
                .max()
        })
        .is_some_and(|name| crate::android::share_diagnostic(&name));
    #[cfg(not(target_os = "android"))]
    let opened = horizon_client::platform::open_folder(&dir) == Some(true);
    if !opened {
        *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) = Some("starling.diagnostics.failed");
    }
}

pub(super) fn status() -> Vec<String> {
    static UNCLEAN: OnceLock<bool> = OnceLock::new();
    let unclean =
        *UNCLEAN.get_or_init(|| crate::diagnostics::previous_unclean(&logging::private_home()));
    let mut lines = vec![
        lang::get(if unclean {
            "starling.diagnostics.unclean"
        } else {
            "starling.diagnostics.clean"
        }),
        lang::get(if crate::performance::active() {
            "starling.diagnostics.recording"
        } else {
            "starling.diagnostics.stopped"
        }),
    ];
    lines.push(lang::get(if crate::performance::gpu_profile() {
        "starling.diagnostics.gpu_on"
    } else {
        "starling.diagnostics.gpu_off"
    }));
    if let Some(result) = EXPORTED.lock().unwrap_or_else(|e| e.into_inner()).take() {
        match result {
            Ok(path) => {
                log::info!("Diagnostic report exported: {}", path.display());
                *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some("starling.diagnostics.exported");
            }
            Err(error) => {
                log::warn!("Diagnostic export failed: {error}");
                *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some("starling.diagnostics.failed");
            }
        }
    }
    if EXPORTING.load(Ordering::Relaxed) {
        lines.push(lang::get("starling.diagnostics.exporting"));
    } else if let Some(key) = *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) {
        lines.push(lang::get(key));
    }
    lines
}
