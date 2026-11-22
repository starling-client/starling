//! UUID formatting shared by world data and game clients.

/// Format vanilla's four most-significant-first ints as a hyphenated UUID.
pub fn uuid_text(u: [i32; 4]) -> String {
    let most = (i64::from(u[0]) << 32) | (i64::from(u[1]) & 0xFFFF_FFFF);
    let least = (i64::from(u[2]) << 32) | (i64::from(u[3]) & 0xFFFF_FFFF);
    let (m, l) = (most as u64, least as u64);
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        m >> 32,
        (m >> 16) & 0xFFFF,
        m & 0xFFFF,
        l >> 48,
        l & 0xFFFF_FFFF_FFFF
    )
}
