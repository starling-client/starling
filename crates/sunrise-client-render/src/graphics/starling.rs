//! Starling's part of graphics.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Written beside it on Android before the renderer opens its device, and removed once it has
/// presented a frame (`gfx::Renderer::forget_start_marker`). Found at the next start, the
/// graphics driver took the game down while starting (a native crash the game can't catch,
/// like the PowerVR GE8320's in `vkQueueSubmit`), so Auto and Vulkan become OpenGL ES.
pub const RENDERER_MARKER: &str = "sunrise-renderer-pending";

/// The `RENDERER_MARKER`s this process wrote ([`note_start_marker`]).
pub(super) static START_MARKERS: std::sync::Mutex<Vec<std::path::PathBuf>> =
    std::sync::Mutex::new(Vec::new());

/// A `RENDERER_MARKER` written for a renderer that is starting, for the panic hook to find.
pub fn note_start_marker(marker: &Path) {
    START_MARKERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(marker.to_owned());
}

/// For the panic hook (`crash::install`): the markers go, as a panic is the game's, which the
/// hook just caught, not the driver's. A start that panicked before its first frame (a mod's hook
/// asking for what had not loaded yet, 2026-11-16) sent every start after it to OpenGL ES.
pub fn forget_start_markers_after_panic() {
    let markers = std::mem::take(
        &mut *START_MARKERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for marker in markers {
        let _ = fs::remove_file(marker);
    }
}

pub(super) static MALI_FAST_TERRAIN_WANTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The Mali Fast Terrain switch (experimental) as `sunrise-graphics.txt` had it when it was read:
/// the renderer a start opens takes Adreno's terrain paths on Mali with it (`gfx::adreno`).
pub fn mali_fast_terrain_wanted() -> bool {
    MALI_FAST_TERRAIN_WANTED.load(std::sync::atomic::Ordering::Relaxed)
}

pub(super) static MALI_FAST_TERRAIN_OFFERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the last renderer opened Mali behind Android's Vulkan, where the switch does something.
pub fn offer_mali_fast_terrain(on: bool) {
    MALI_FAST_TERRAIN_OFFERED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn mali_fast_terrain_offered() -> bool {
    MALI_FAST_TERRAIN_OFFERED.load(std::sync::atomic::Ordering::Relaxed)
}

pub(super) static RAY_TRACING_WANTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The Ray Tracing switch (experimental, #470) as `sunrise-graphics.txt` had it when it was
/// read (`Graphics::load`): the device a renderer opens asks for ray queries with it, so a
/// change applies at the next start.
pub fn ray_tracing_wanted() -> bool {
    RAY_TRACING_WANTED.load(std::sync::atomic::Ordering::Relaxed)
}

pub(super) static RAY_TRACING_OFFERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the adapter offers hardware ray tracing (`Features::EXPERIMENTAL_RAY_QUERY`):
/// set when the renderer opens its device. The Ray Tracing switch is greyed out where it
/// is false, whatever `sunrise-graphics.txt` says.
pub fn offer_ray_tracing(on: bool) {
    RAY_TRACING_OFFERED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn ray_tracing_offered() -> bool {
    RAY_TRACING_OFFERED.load(std::sync::atomic::Ordering::Relaxed)
}

pub(super) static RAY_TRACING_MISSING: std::sync::Mutex<Option<String>> =
    std::sync::Mutex::new(None);

/// Why the adapter offers no ray queries, for the settings screen: the missing device
/// extensions `open_backend` enumerated, if it could (`None`: not enumerated, e.g. the
/// graphics API is not Vulkan). Overwritten on every device open.
pub fn set_ray_tracing_missing(missing: Option<&[&str]>) {
    *RAY_TRACING_MISSING
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = missing.map(|m| m.join(", "));
}

/// [`set_ray_tracing_missing`]'s message, if one was recorded.
pub fn ray_tracing_missing() -> Option<String> {
    RAY_TRACING_MISSING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// The profile that fits the pack to the device (`game/shader_packs.rs`): a phone's until the
/// player picks another.
pub const AUTO_PROFILE: &str = "auto";

/// The most of the screen's resolution Auto draws the world at on this device.
pub fn auto_shader_scale() -> f32 {
    use crate::system_stats::Tier;
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| match Tier::here().unwrap_or(Tier::High) {
        Tier::High => 0.5,
        Tier::Mid => 0.4,
        Tier::Low => 0.3,
    })
}

/// A pack made to be cheap, which keeps the player's resolution and draws no depth first:
/// Starling Visuals and MakeUp Ultra Fast.
pub fn light_shader_pack(name: &str) -> bool {
    let name = name.replace(['-', '_', ' '], "").to_ascii_lowercase();
    name.starts_with("starlingvisuals") || name.starts_with("makeup")
}

/// The graphics API a start uses after the last one died in the driver before its first frame
/// (`RENDERER_MARKER`): Vulkan's, chosen or by Auto, gives way to OpenGL ES; a player who chose
/// OpenGL keeps it, so the setting never swings back.
pub(super) fn after_driver_crash(api: GraphicsApi) -> GraphicsApi {
    match api {
        GraphicsApi::Auto | GraphicsApi::Vulkan => GraphicsApi::OpenGl,
        other => other,
    }
}

/// A phone's first start: settings for its tier instead of the desktop's (measured on a
/// Snapdragon 8 Gen 2 at 3200x1440; slower tiers scaled down from it).
///
/// The frame cap is 60: a steady 60 runs cooler than 70 to 90 that dips, and the display
/// follows the cap. Mobs think on the render thread, so the simulation distance stays short:
/// at 12 the ticks of a few hundred mobs made every third frame late. Clouds cost the GPU
/// 1.7 ms of a frame at any distance, fancy leaves are overdraw on tiled GPUs, and animated
/// textures are mixed on the CPU each frame.
pub fn apply_mobile_defaults(tier: crate::system_stats::Tier, s: &mut crate::settings::Settings) {
    use crate::system_stats::Tier;
    let high = tier == Tier::High;
    s.max_fps = 60;
    s.render_distance = match tier {
        Tier::High => 10,
        Tier::Mid => 8,
        Tier::Low => 6,
    };
    s.simulation_distance = if high { 8 } else { 6 };
    s.clouds = if high { "fast" } else { "false" }.into();
    s.particles = if high { 1 } else { 2 };
    s.entity_shadows = high;
    s.biome_blend = if high { 1 } else { 0 };
    s.mipmap_levels = if high { 4 } else { 2 };
    s.entity_distance = match tier {
        Tier::High => 1.0,
        Tier::Mid => 0.75,
        Tier::Low => 0.5,
    };
    s.cutout_leaves = high;
    s.weather_radius = if high { 5 } else { 3 };
    // Bedrock's touch default: the stick walks up a block without the jump button.
    s.auto_jump = true;
    s.graphics_preset = "custom".into();
    let g = &mut s.sunrise;
    g.preset = Preset::Custom;
    // Every tier at the window's own size, the world drawn straight to the screen. Below it the
    // world goes to a target of its own and a full-screen pass scales it up, which costs a phone
    // more than the pixels save: its GPU is held back by geometry (below), and on a Moto G51
    // (Adreno 619, Low tier) the 70% this gave the Low tier ran far slower than 100% (a player's
    // report, 2026-11-19). The Resolution Scale slider still lowers it.
    g.resolution_scale = 100;
    g.animated_textures = tier != Tier::Low;
    // A phone's GPU is held back by geometry, which far terrain is most of: on the Snapdragon 8
    // Gen 2 at render distance 8 and LOD distance 128, Full drew the frame in 29 to 31 ms of GPU
    // and Mobile in 19 (render distance 8 alone, 15.5).
    g.far_detail = FarDetail::Mobile;
    // Starling: the leaves inside a crown draw solid, the outer ones keep their cutout, so a crown
    // looks full from outside. Hidden (faces behind two leaf layers dropped) was the default and
    // took 18.7 to 15.5 ms of GPU on the Snapdragon 8 Gen 2, but its crowns showed the sky through
    // the outer leaves' gaps where the faces behind were gone, which players found ugly.
    g.leaf_culling = LeafCullingMode::Solid;
}

/// A phone that started before every tier began at the window's own size: the 70% (Low tier) or
/// 85% (Mid tier) its first start saved goes to 100. Run once per installation (game.rs keeps the
/// marker); a player who had picked one of them on the slider picks it again. Whether it changed.
pub fn lift_old_mobile_resolution(s: &mut crate::settings::Settings) -> bool {
    let g = &mut s.sunrise;
    let old_default = matches!(g.resolution_scale, 70 | 85);
    if old_default {
        g.resolution_scale = 100;
    }
    old_default
}

impl Graphics {
    /// The shader pack and profile chosen: `SUNRISE_SHADER_PACK`'s, for development, else the
    /// Shader Packs page's.
    pub fn shader_choice(&self) -> Option<(String, String)> {
        let env = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
        let pack = env("SUNRISE_SHADER_PACK")
            .or_else(|| Some(self.shader_pack.clone()).filter(|p| !p.is_empty()))?;
        let profile = match self.shader_profile.as_str() {
            "" => env("SUNRISE_SHADER_PROFILE").unwrap_or_else(|| {
                if cfg!(target_os = "android") {
                    AUTO_PROFILE.to_owned()
                } else {
                    String::new()
                }
            }),
            "default" => String::new(),
            profile => profile.to_owned(),
        };
        Some((pack, profile))
    }
}

/// `INTEGRATED_LOD_DISTANCE` at Mobile far detail, whose far terrain costs a fraction of Full's
/// (on a Snapdragon 8 Gen 2 at LOD distance 128: 3 ms of GPU a frame against 14).
pub const INTEGRATED_MOBILE_LOD_DISTANCE: u32 = 256;

impl Graphics {}
