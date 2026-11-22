//! MD5-derived seeds in Java’s big-endian word order.
use crate::md5::{Digest, Md5, md5};

fn seed(digest: [u8; 16]) -> [u64; 2] {
    [
        u64::from_be_bytes(digest[..8].try_into().unwrap()),
        u64::from_be_bytes(digest[8..].try_into().unwrap()),
    ]
}
pub fn seed_from_hash(name: &str) -> [u64; 2] {
    seed(md5(name.as_bytes()))
}
pub fn seed_from_hash_utf16(name: &[u16]) -> [u64; 2] {
    let mut digest = Md5::new();
    for point in char::decode_utf16(name.iter().copied()) {
        let mut bytes = [0; 4];
        // Java UTF-8 encoding replaces each malformed surrogate with '?'.
        digest.update(point.unwrap_or('?').encode_utf8(&mut bytes).as_bytes());
    }
    seed(digest.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_hash_replaces_each_unpaired_surrogate_with_question_mark() {
        assert_eq!(
            seed_from_hash_utf16(&[0xd800, 0x61, 0xdc00]),
            seed_from_hash("?a?")
        );
        assert_eq!(
            seed_from_hash_utf16(&[0xd83d, 0xde00]),
            seed_from_hash("😀")
        );
    }
}
