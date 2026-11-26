//! Private custom recipe-book synchronization capability, independent of mod payloads.
/// Configuration advertisement and play Offer/Snapshot channel.
pub const CHANNEL: &str = "sunrise:recipe_book_settings";
/// Settings message format, shared by configuration and play Hello/Ready.
pub const VERSION: u8 = 2;
/// The only supported configuration advertisement body.
pub const CAPABILITY: &[u8] = &[VERSION];
