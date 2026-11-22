//! Sunrise's mod files (`docs/modding/src/how-to/content-mods.md`): content mods (`.smod`: blocks, items, data and
//! assets, no code), modpacks and, with `index`, the mod registry's index. Code mods are
//! native (docs/modding/src/how-to/native-mods.md).
pub mod content;
pub mod dependencies;
pub mod durable;
pub mod enabled;
pub mod extension_tags;
pub mod game_identity;
#[cfg(all(
    feature = "runtime-process",
    any(target_os = "linux", target_os = "macos", windows)
))]
pub mod holder;
// Starling: Horizon's packages, made Sunrise's as they are installed.
pub mod horizon;
pub mod icon;
#[cfg(feature = "index")]
pub mod index;
pub mod install_lock;
pub mod internals;
#[cfg(all(
    feature = "runtime-process",
    any(target_os = "linux", target_os = "macos", windows)
))]
pub mod local_control;
#[cfg(feature = "runtime-process")]
pub mod locks;
pub mod manifest;
pub mod mixins_tool;
pub mod mod_list;
pub mod modpack;
pub mod native_route;
#[cfg(feature = "runtime-process")]
pub mod output;
pub mod packages;
pub mod runtime;
#[cfg(feature = "runtime-process")]
pub mod runtime_leases;
pub mod sdk;
pub mod selection;

pub use content::Content;
pub use manifest::{Capability, Manifest, Package, find};
/// A build's surface (`surface.json`) and a mod's uses table: the SDK's records the fit test
/// compares.
#[cfg(feature = "prebuilt")]
pub use mod_surface;
/// The SDK's build-pinned data files and their common header.
pub use mod_surface::build_data;
