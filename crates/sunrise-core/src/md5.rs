//! MD5 (RFC 1321) for Java-compatible deterministic seeds and version-3 UUIDs.
//! This is not an authentication primitive.
pub(crate) use ::md5::{Digest, Md5};

/// Compute the raw RFC 1321 digest without heap allocation.
pub fn md5(input: &[u8]) -> [u8; 16] {
    let mut digest = Md5::new();
    digest.update(input);
    digest.finalize().into()
}

/// Java `UUID.nameUUIDFromBytes`: MD5 with the version 3 and IETF variant bits set.
pub fn name_uuid(bytes: &[u8]) -> u128 {
    uuid_from_digest(md5(bytes))
}

/// The offline player UUID hashes the UTF-8 bytes of `OfflinePlayer:` followed by `name`.
pub fn offline_uuid(name: &str) -> u128 {
    let mut digest = Md5::new();
    digest.update(b"OfflinePlayer:");
    digest.update(name.as_bytes());
    uuid_from_digest(digest.finalize().into())
}

fn uuid_from_digest(mut digest: [u8; 16]) -> u128 {
    digest[6] = (digest[6] & 0x0f) | 0x30;
    digest[8] = (digest[8] & 0x3f) | 0x80;
    u128::from_be_bytes(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc_1321_vectors() {
        for (input, expected) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "57edf4a22be3c955ac49da2e2107b67a",
            ),
        ] {
            let actual: String = md5(input.as_bytes())
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn name_uuid_and_offline_uuid_use_utf8_and_set_version_and_variant() {
        // Captured from java.util.UUID.nameUUIDFromBytes on Temurin 25.0.5.
        let name = "A😺漢字";
        assert_eq!(
            name_uuid(name.as_bytes()),
            0x09bc612d_a823_3e73_b85e_63ffabe1b5b4
        );
        assert_eq!(offline_uuid(name), 0xfccd9eec_78f8_3d27_8028_e7e0cae955e3);
    }
}
