//! The official game executable (docs/layers.md, "How a Sunrise crate plugs in").
#![forbid(unsafe_code)]

/// The game identity marker for launcher imports, retained by `main`.
static GAME_IDENTITY: [u8; sunrise_game::GAME_IDENTITY.len()] = sunrise_game::GAME_IDENTITY;

fn main() {
    std::hint::black_box(&GAME_IDENTITY);
    sunrise_official::register();
    sunrise_game::main();
}
