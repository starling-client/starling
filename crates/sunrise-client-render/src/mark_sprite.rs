//! Sunrise's mark (`docs/media/brand/icon.svg`) on the text grid, for an inline text sprite.

/// The mark's name in the HUD atlas.
pub const SPRITE: &str = "sunrise:gui/mark";
const PNG: &[u8] = include_bytes!("../../sunrise-client/assets/sunrise/textures/gui/mark.png");

/// The mark as the HUD atlas takes an image.
pub fn image() -> std::iter::Once<(String, u32, u32, Vec<u8>)> {
    let (width, height, rgba) = crate::hud::decode_bytes(PNG).expect("Sunrise's mark is PNG");
    std::iter::once((SPRITE.to_owned(), width, height, rgba))
}
