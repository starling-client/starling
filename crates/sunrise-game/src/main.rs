//! Sunrise: native Rust client for Minecraft Java Edition 26.3 gameplay.
#![forbid(unsafe_code)]

/// Identifies this executable as the game for launcher imports. `main` reads it so the
/// linker keeps it; it carries neither an API version nor a per-build id.
static GAME_IDENTITY: [u8; sunrise_mod_package::game_identity::MARK.len()] =
    sunrise_mod_package::game_identity::MARK;

fn main() {
    std::hint::black_box(&GAME_IDENTITY);
    sunrise_game::main();
}
