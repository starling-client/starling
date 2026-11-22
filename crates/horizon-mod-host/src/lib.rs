//! Installed mod code loading above the engine (docs/layers.md, #2534).
//! The platform prepares the engine, loads mods here, then makes one engine start call.
#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod native_libraries;
pub use native_libraries::{GAME_BUILD_ID, Opened, RUSTC_VERSION, main, open, packages_beside};

mod loading;
pub use loading::{code_runtime, load_code, main_with};

/// Establish main-thread stack capacity before any game startup work.
pub use horizon_guest_stack::prepare_main as prepare_main_stack;

/// Prepares the main thread's stack before any game startup work. It also ran the WebAssembly
/// compiler's child process here, before Starling stopped running wasm mods; the name stays for
/// the starts that call it.
pub fn exit_if_compile_child() {
    horizon_guest_stack::prepare_main();
}

/// A typed API-point listener supplied by generated native startup.
pub use mod_points::__private::SlotListener;
