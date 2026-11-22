//! Installed mod code loading above the engine (docs/layers.md, #2534).
//! The platform prepares the engine, loads mods here, then makes one engine start call.
#![deny(unsafe_code)]

#[allow(unsafe_code)]
<<<<<<< sync-sunrise-1
mod native_libraries;
pub use native_libraries::{GAME_BUILD_ID, Opened, RUSTC_VERSION, main, open, packages_beside};
=======
pub mod native_libraries;
pub use native_libraries::{GAME_BUILD_ID, RUSTC_VERSION, main};
>>>>>>> sunrise/main

mod loading;
pub use loading::{code_runtime, load_code, main_with};

<<<<<<< sync-sunrise-1
/// Establish main-thread stack capacity before any game startup work.
pub use sunrise_guest_stack::prepare_main as prepare_main_stack;

/// Prepares the main thread's stack before any game startup work. It also ran the WebAssembly
/// compiler's child process here, before Starling stopped running wasm mods; the name stays for
/// the starts that call it.
pub fn exit_if_compile_child() {
    sunrise_guest_stack::prepare_main();
}

/// A typed API-point listener supplied by generated native startup.
pub use mod_points::__private::SlotListener;
=======
mod mods_button;

#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script;
>>>>>>> sunrise/main
