use sha2::{Digest, Sha512};

/// Checks a SHA-512 written as 128 lowercase hex digits, the form `[[jar]]` and `[runtime]` use.
pub(super) fn sha512_hex(value: &str) -> Result<(), &'static str> {
    if value.len() == 128
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err("must be 128 lowercase hex digits")
    }
}

/// The SHA-512 of `bytes`, as `[runtime]` writes it.
pub fn sha512_of(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
