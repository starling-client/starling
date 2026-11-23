// Adapted from Horizon Android 0.7.8 (tag v0.7.8, crates/sunrise-client/src/android_main.rs and
// android/), written by Vasysik and the Horizon Android contributors (MIT). Copyright (c) 2026
// Ludovic Marie and the Horizon contributors. See THIRD_PARTY_NOTICES.md and LICENSE.
//! Horizon Android's launcher, drawn by the game's renderer before the game starts: Play with its
//! instances, the mods, packs and shaders catalogs, what is installed (with the file manager), the
//! settings and the wardrobe. The files are Horizon Android's own, with their paths pointed here:
//! `crate::` is this module, `sunrise_client::` is [`client`], the game's library under the names
//! Horizon Android knew it by.
//!
//! A process runs the launcher or the game, never both: winit's event loop is made once a process
//! (`android::main`). Play keeps the chosen instance in `android-launcher.json` and starts the app
//! again into the game ([`play`]); the Launcher button of the title screen and the pause menu
//! starts it again into the launcher ([`back_to_launcher`]). An instance can open directly, which
//! skips the launcher until the player turns that off there.
#![allow(dead_code, clippy::all)]

pub(crate) mod catalog;
pub(crate) mod device_preferences;
pub(crate) mod device_runtime;
pub(crate) mod display;
pub(crate) mod document_bridge;
pub(crate) mod document_store;
pub(crate) mod documents;
pub(crate) mod gallery;
pub(crate) mod launcher;
pub(crate) mod native_ime;
pub(crate) mod platform;
pub(crate) mod search_queue;
pub(crate) mod shader_catalog;
// Sunrise's desktop launcher's catalog search, downloads and skins, as Horizon Android shared them
// (launcher/src at v0.7.8).
pub(crate) mod browser;
pub(crate) mod port_net;
pub(crate) mod proxy;
pub(crate) mod skins;
// Starling's accounts under the names Sunrise's launcher gave them (account.rs).
pub(crate) mod account;
pub(crate) mod content_files;
// The wardrobe's choice for Starling's game (own_skin).
pub(crate) mod appearance;
// The account's head on the header and the account pages (Horizon Android's game had it).
pub(crate) mod skin_head;
/// Horizon Android's documents provider told the launcher when the Files app changed a file;
/// Starling's (GameFolderProvider) is the system's, which tells it nothing.
pub(crate) mod document_provider {
    pub fn take_changes() -> bool {
        false
    }
}
// The game's own, which Horizon Android's had beside these.

/// Starling's diagnostics, the ZIP players attach to their reports (`crate::diagnostics`, as the
/// game's Diagnostics exports it): kept in the app's private files, where Horizon Android named
/// the launcher's folder.
pub(crate) mod diagnostics {
    pub use crate::diagnostics::{heartbeat, record_error, redact};
    use std::path::{Path, PathBuf};
    fn private() -> PathBuf {
        sunrise_client::logging::private_home()
    }
    pub fn previous_unclean(_home: &Path) -> bool {
        crate::diagnostics::previous_unclean(&private())
    }
    pub fn summary(_home: &Path) -> String {
        crate::diagnostics::summary(&private())
    }
    /// The chosen instance's logs and crash reports with the app's diagnostics, in one ZIP.
    pub fn export(home: &Path) -> Result<PathBuf, String> {
        let player = super::catalog::Config::load(home).game_home(home);
        crate::diagnostics::export_from(&private(), &player)
    }
}
/// The game's performance recording, which the game starts (Play turns it on for its run).
pub(crate) mod performance {
    pub use crate::performance::stop;
    pub fn export(_home: &std::path::Path) -> Result<(), String> {
        crate::performance::export(&sunrise_client::logging::private_home())
    }
}
// What its launcher drew with from Horizon Android's game, over Starling's boot renderer.
pub(crate) mod game;

/// The game's library as Horizon Android 0.7.8 named it. Starling has the same code under other
/// names: Horizon Android's touch controls, their editor, its theme and its keyboard came over as
/// `android_controls`, `android_control_ui`, `android_control_theme` and
/// `android_control_keyboard`.
pub(crate) mod client {
    pub(crate) use super::{content_files, skin_head};
    pub use sunrise_client::android_control_keyboard as osk;
    pub use sunrise_client::android_control_theme as ui_theme;
    pub use sunrise_client::android_control_ui as touch_ui;
    pub use sunrise_client::android_controls as touch;
    pub use sunrise_client::*;
    pub use sunrise_client_render::android_gamepad;

    /// Horizon Android's input runtime, over Starling's: the controllers come through the input
    /// hook (`android::pad_state`), not winit's device events.
    pub mod android_runtime {
        use sunrise_client::gamepad::PadState;
        pub use sunrise_client_render::android_runtime::*;

        /// The active controller's state; Starling's input hook keeps it.
        pub fn gamepad_state() -> PadState {
            crate::android::pad_state()
                .map(|(_, state)| state)
                .unwrap_or_default()
        }
        /// The controller in use, for its motors.
        pub fn active_gamepad_id() -> Option<i32> {
            crate::android::pad_state().map(|(id, _)| id)
        }
        /// Starling's input hook takes a controller's events before winit.
        pub fn gamepad_event(_id: i32, _event: &winit::event::DeviceEvent) -> bool {
            false
        }
        pub fn retain_gamepads(_ids: &[i32]) {}
        pub fn clear_gamepads() {}
        /// The launcher's process never runs the game: nothing to return from.
        pub fn take_launcher_return() -> bool {
            false
        }
        // Horizon Android's device knobs, kept for the launcher's own run; the game's process
        // reads the launcher's settings itself when it starts (`ha_launcher::game_started`).
        pub fn configure_worker_limit(_limit: u32) {}
        pub fn configure_cache_budget(_mib: u32) {}
        pub fn configure_gpu(_batching: bool) {}
        pub fn configure_graphics(_open_gl: bool) {}
        pub fn set_experimental_leaf_culling(_enabled: bool) {}
        /// Horizon Android's Vulkan start-up marker; Starling's game keeps its own
        /// (`graphics::RENDERER_MARKER`) and goes to OpenGL ES by itself after a failed start.
        pub mod graphics_startup {
            pub fn configure(_home: std::path::PathBuf) {}
            pub fn recover(_home: &std::path::Path) -> bool {
                false
            }
        }
    }

    /// Horizon Android installed the game's files itself. Starling's game does it on its setup
    /// screen (the official files from Mojang, then the preparing on the phone: `accounts`,
    /// `preparer`), so the launcher only asks whether that is done, and Play starts the game
    /// there when it is not.
    pub mod official_install {
        use std::path::Path;

        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub enum InstallStage {
            #[default]
            Metadata,
            Download,
            Extract,
            WorldData,
            Controls,
            Finalize,
        }
        impl InstallStage {
            pub const ALL: [Self; 6] = [
                Self::Metadata,
                Self::Download,
                Self::Extract,
                Self::WorldData,
                Self::Controls,
                Self::Finalize,
            ];
            pub fn label(self) -> &'static str {
                match self {
                    Self::Metadata => "Checking Minecraft version",
                    Self::Download => "Downloading game resources and sound",
                    Self::Extract => "Unpacking verified files",
                    Self::WorldData => "Preparing worlds and registries",
                    Self::Controls => "Downloading touch controls",
                    Self::Finalize => "Finishing installation",
                }
            }
        }
        #[derive(Clone, Copy, Debug, Default)]
        pub struct InstallProgress {
            pub stage: InstallStage,
            pub done: u64,
            pub total: u64,
        }
        /// Whether the game's prepared files are in place (their manifest, as the game checks).
        pub fn ready(_home: &Path) -> bool {
            sunrise_client::logging::private_home()
                .join("resources/sunrise-resources.json")
                .is_file()
        }
        pub fn cleanup_completed_downloads(_home: &Path) -> sunrise_client::Result<()> {
            Ok(())
        }
    }

    /// The game's mod list. Horizon Android's game carried mods of its own, which its Mods page
    /// listed beside the installed ones; Starling's has none.
    pub mod mod_list {
        pub use sunrise_client::mod_list::*;
        pub const ANDROID_BUNDLED: [(&str, &str, &str); 0] = [];
    }

    /// Horizon Android mended its own engine data's structure files; Starling's prepared files
    /// need nothing of the kind.
    pub mod android_resources {
        pub fn repair_structure_transforms(
            _engine: &std::path::Path,
            _resources: &std::path::Path,
        ) -> sunrise_client::Result<()> {
            Ok(())
        }
    }
}

/// Sunrise's launcher's account and catalog code asks the launcher for its version and folders.
pub(crate) mod backend {
    pub const VERSION: &str = env!("CARGO_PKG_VERSION");
    static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    pub fn initialize(home: std::path::PathBuf) {
        let _ = HOME.set(home);
    }
    pub fn game_home() -> std::path::PathBuf {
        data_dir()
    }
    pub fn game_mod_api_level() -> Option<u32> {
        None
    }
    pub fn data_dir() -> std::path::PathBuf {
        HOME.get()
            .cloned()
            .unwrap_or_else(sunrise_client::logging::private_home)
    }
}

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use winit::platform::android::activity::AndroidApp;

thread_local! {
    pub(crate) static APP: RefCell<Option<AndroidApp>> = const { RefCell::new(None) };
}

pub(crate) const PUBLIC_BUILD: bool = true;

pub(crate) fn keyboard_mouse_input() -> bool {
    client::android_runtime::active_input() == catalog::InputHost::KeyboardMouse
}
pub(crate) fn configure_input_mode(mode: catalog::InputHost) {
    client::android_runtime::configure_input(mode);
}
pub(crate) fn touch_visible() -> bool {
    client::android_runtime::active_input() == catalog::InputHost::Touch
}
pub(crate) fn touch_input(touch: bool) {
    client::android_runtime::note_input(if touch {
        catalog::InputHost::Touch
    } else {
        catalog::InputHost::KeyboardMouse
    });
}
pub(crate) fn save_input_mode() {
    let home = backend::data_dir();
    let mut config = catalog::Config::load(&home);
    let mode = client::android_runtime::input_mode();
    if config.input_host != mode {
        config.input_host = mode;
        if let Err(e) = config.save(&home) {
            log::warn!("Saving input mode: {e}");
        }
    }
}

/// Written by Play: the next start is the game's, in the instance `android-launcher.json` names.
const PLAY: &str = "launcher-play";
/// Written by the Launcher button: the next start is the launcher's, even for an instance that
/// opens directly.
const OPEN: &str = "launcher-open";

/// What this start of the app runs.
pub enum Start {
    /// The launcher, over the game folder.
    Launcher,
    /// The game, in this instance's folder.
    Game(PathBuf),
}

/// The start's choice, from the app's private files and the game folder `home`: the launcher
/// after its button, the game after Play (or a restart the game asked for), else the launcher
/// unless the chosen instance opens directly.
pub fn start(private: &Path, home: &Path) -> Start {
    let open = std::fs::remove_file(private.join(OPEN)).is_ok();
    let play = private.join(PLAY).is_file();
    if open {
        acknowledge_play(private);
    }
    let config = catalog::Config::load(home);
    display::migrate_preferences(&config, home);
    if !open
        && (play || config.open_directly || config.onboarding == catalog::OnboardingState::Chosen)
    {
        let instance = config.profile_home(home);
        let _ = std::fs::create_dir_all(&instance);
        return Start::Game(instance);
    }
    Start::Launcher
}

/// Keep the Play request until the game draws, so Android recovery retries the same startup.
pub fn acknowledge_play(private: &Path) {
    if let Err(error) = std::fs::remove_file(private.join(PLAY))
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("Launcher: could not clear the launch request: {error}");
    }
}

/// The game's next start, in the instance the launcher chose: the app starts again
/// (`StarlingActivity.restart`).
pub fn play(installed: bool) -> Result<(), String> {
    let marker = sunrise_client::logging::private_home().join(PLAY);
    std::fs::write(&marker, b"")
        .map_err(|error| format!("Could not save the launch request. {error}"))?;
    let message = if installed {
        sunrise_client::lang::get_or("starling.restart.play", "Starting Starling…")
    } else {
        sunrise_client::lang::get_or("starling.restart.setup", "Opening setup…")
    };
    if !crate::android::call_restart(&message) {
        let _ = std::fs::remove_file(marker);
        return Err("Android could not start Starling. Try again.".into());
    }
    Ok(())
}

/// Set by the Launcher button: the game ends as Quit ends it (the world saved first), then the app
/// starts again into the launcher (`android::finish_and_exit`).
static LAUNCHER_NEXT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The title screen's and the pause menu's Launcher button, before the game quits.
pub fn back_to_launcher() {
    let _ = std::fs::write(sunrise_client::logging::private_home().join(OPEN), b"");
    LAUNCHER_NEXT.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the game's end starts the launcher.
pub fn launcher_next() -> bool {
    LAUNCHER_NEXT.load(std::sync::atomic::Ordering::Relaxed)
}

/// A restart the game asks for (a build with other native mods, a file opened with Starling)
/// comes back to the game.
pub fn restart_into_game() {
    if !launcher_next() {
        let _ = std::fs::write(sunrise_client::logging::private_home().join(PLAY), b"");
    }
}

/// The game's process, in the instance `home` of the game folder `folder`: what the launcher's
/// settings ask of the game's run.
pub fn game_started(folder: &Path, home: &Path) {
    let config = catalog::Config::load(folder);
    if config.record_performance || config.device.gpu_profile {
        if let Err(error) = crate::performance::start(
            &sunrise_client::logging::private_home(),
            serde_json::json!({
                "renderer": config.renderer.label(),
                "resolution_percent": config.game_render_scale(folder).percent(),
                "instance": config.profile,
                "performance_recording": config.record_performance,
                "gpu_profiling": config.device.gpu_profile,
            }),
        ) {
            log::warn!("Launcher: cannot start the performance recording: {error}");
        }
    }
    let _ = home;
    // The theme and its motion came with Config::load.
    client::ui_theme::set_ui_scale_percent(config.ui_scale_percent);
}

/// `StarlingActivity.nativeDocumentResult`: the file the launcher's picker chose (android_lib.rs).
pub fn document_result(
    env: jni::JNIEnv<'_>,
    activity: jni::objects::JObject<'_>,
    request: i32,
    result: i32,
    uri: jni::objects::JString<'_>,
) {
    documents::document_result(env, activity, request, result, uri);
}

/// The launcher's run, from `android::main` once the process has its folders and platform hooks:
/// its window until the player leaves or presses Play.
pub fn run(app: AndroidApp, home: PathBuf) {
    use winit::event_loop::EventLoop;
    use winit::platform::android::EventLoopBuilderExtAndroid;
    backend::initialize(home.clone());
    APP.with(|slot| *slot.borrow_mut() = Some(app.clone()));
    let mut launcher = launcher::Launcher::new(home);
    let result = EventLoop::builder()
        .with_android_app(app)
        .build()
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })
        .and_then(|events| events.run_app(&mut launcher).map_err(Into::into));
    if let Err(error) = result {
        log::error!("Launcher: {error}");
    }
    drop(launcher);
    APP.with(|slot| *slot.borrow_mut() = None);
}
