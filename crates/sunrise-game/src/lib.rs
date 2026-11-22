//! The game as the platform runs it, for an executable to start (docs/layers.md, "How a Sunrise
//! crate plugs in"). The official executable (`sunrise-official`, Sunrise layer) registers its
//! hooks (`sunrise_platform::hooks`) and then calls [`main`]; it never names an engine crate.
//!
//! An executable built on it:
//!
//! ```ignore
//! static GAME_IDENTITY: [u8; sunrise_game::GAME_IDENTITY.len()] = sunrise_game::GAME_IDENTITY;
//!
//! fn main() {
//!     std::hint::black_box(&GAME_IDENTITY);
//!     my_crate::register();
//!     sunrise_game::main();
//! }
//! ```
//!
//! The platform's own parts register here, in [`main`], as they leave the engine (layer plan M6).
#![forbid(unsafe_code)]

/// Identifies an executable as the game for launcher imports, without an API version or build id.
pub const GAME_IDENTITY: [u8; sunrise_mod_package::game_identity::MARK.len()] =
    sunrise_mod_package::game_identity::MARK;

/// Runs the game with the hooks registered so far, and returns when it ends. Register hooks
/// (`sunrise_platform::hooks::register`) before calling it.
pub fn main() {
    let startup = sunrise_client::entry::prepare();
    std::hint::black_box(&sunrise_mod_package::holder::CAPABILITY);
    sunrise_mod_host::main_with(Vec::new(), &[], startup);
}
