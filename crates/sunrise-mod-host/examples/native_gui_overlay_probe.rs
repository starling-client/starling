//! A real-renderer consumer of the public GUI API. Run in a disposable game home:
//! `cargo run -p sunrise-mod-host --example native_gui_overlay_probe --
//! --resources PATH --world OverlayProbe --creative --world-frames 120`.
//! Every world frame turns the camera 90 degrees; GUI callbacks assert that
//! ClientHost::camera already sees that frame. A magenta square draws in the post-HUD pass.
use mod_api::{
    ClientHost, ComputeCameraAnglesEvent, Hooks, HudMode, ModEntry, RenderGuiPostEvent,
    ScreenRenderPostEvent,
};
use std::sync::atomic::{AtomicU32, Ordering};

struct Probe {
    frame: AtomicU32,
    yaw: AtomicU32,
}

impl Probe {
    fn observe(&self, host: &dyn ClientHost, phase: &str, flags: Option<(bool, bool)>) {
        let camera = host.camera();
        if let Some(camera) = &camera {
            assert_eq!(
                camera.yaw.to_bits(),
                self.yaw.load(Ordering::SeqCst),
                "previous frame's camera in {phase}"
            );
            assert!(
                camera
                    .view
                    .iter()
                    .chain(camera.projection.iter())
                    .all(|v| v.is_finite())
            );
        }
        eprintln!(
            "native_gui_probe {}",
            serde_json::json!({
                "frame": self.frame.load(Ordering::SeqCst), "phase": phase,
                "yaw": camera.map(|c| c.yaw), "flags": flags
            })
        );
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn compute_camera_angles(&self, _: &dyn ClientHost, event: &mut ComputeCameraAnglesEvent) {
        let frame = self.frame.fetch_add(1, Ordering::SeqCst);
        event.yaw = [0.0_f32, 90.0, 180.0, -90.0][frame as usize % 4];
        self.yaw.store(event.yaw.to_bits(), Ordering::SeqCst);
    }

    fn render_gui_post(&self, host: &dyn ClientHost, event: &mut RenderGuiPostEvent<'_>) {
        let [w, h] = event.draw.size();
        event
            .draw
            .fill([w * 0.5 - 8.0, h - 24.0, 16.0, 16.0], 0xffff00ff);
        self.observe(
            host,
            "render_gui_post",
            Some((
                host.hud_mode() != HudMode::AllVisible,
                host.window_focused(),
            )),
        );
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        self.observe(host, event.screen.id, None);
    }
}

fn main() {
    let startup = sunrise_client::entry::prepare();
    sunrise_mod_host::main_with(
        vec![ModEntry::new(
            "gui_overlay_probe",
            Box::new(Probe {
                frame: AtomicU32::new(0),
                yaw: AtomicU32::new(0),
            }),
        )],
        &[],
        startup,
    );
}
