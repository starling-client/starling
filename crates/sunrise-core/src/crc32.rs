//! CRC-32/ISO-HDLC, the checksum used by Java's `CRC32` and ZIP.

/// A streaming ISO-HDLC checksum. Pick the SIMD backend only on CPUs that
/// meet its runtime requirements; other CPUs keep the portable backend.
pub struct Crc32(Backend);

enum Backend {
    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    Fast(crc_fast::Digest),
    Portable(crc32fast::Hasher),
}

impl Crc32 {
    pub fn new() -> Self {
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        if accelerated() {
            return Self(Backend::Fast(crc_fast::Digest::new(
                crc_fast::CrcAlgorithm::Crc32IsoHdlc,
            )));
        }
        Self(Backend::Portable(crc32fast::Hasher::new()))
    }

    pub fn checksum(bytes: &[u8]) -> u32 {
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        if accelerated() {
            return crc_fast::crc32_iso_hdlc(bytes);
        }
        crc32fast::hash(bytes)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        match &mut self.0 {
            #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
            Backend::Fast(digest) => digest.update(bytes),
            Backend::Portable(digest) => digest.update(bytes),
        }
    }

    pub fn finalize(self) -> u32 {
        match self.0 {
            #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
            Backend::Fast(digest) => digest.finalize() as u32,
            Backend::Portable(digest) => digest.finalize(),
        }
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
fn accelerated() -> bool {
    static USE_FAST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *USE_FAST.get_or_init(|| {
        // crc-fast 1.10.0 selects its AES tiers without testing CRC.
        #[cfg(target_arch = "aarch64")]
        {
            std::arch::is_aarch64_feature_detected!("aes")
                && std::arch::is_aarch64_feature_detected!("crc")
        }
        // The VPCLMULQDQ tier additionally needs AVX-512BW; crc-fast does
        // not test that feature. Check the prerequisite AVX-512F too.
        #[cfg(target_arch = "x86_64")]
        {
            !std::arch::is_x86_feature_detected!("avx512vl")
                || (std::arch::is_x86_feature_detected!("avx512f")
                    && (!std::arch::is_x86_feature_detected!("vpclmulqdq")
                        || std::arch::is_x86_feature_detected!("avx512bw")))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bit-at-a-time definition of CRC-32/ISO-HDLC, independent of every backend.
    fn bitwise(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    }

    fn pseudo_random(len: usize, seed: u32) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 8) as u8
            })
            .collect()
    }

    fn backends() -> [Crc32; 2] {
        [
            Crc32::new(),
            Crc32(Backend::Portable(crc32fast::Hasher::new())),
        ]
    }

    #[test]
    fn streamed_crc32_matches_the_iso_hdlc_check_value() {
        for mut checksum in backends() {
            checksum.update(b"1234");
            checksum.update(b"");
            checksum.update(b"56789");
            assert_eq!(checksum.finalize(), 0xcbf4_3926);
        }
    }

    #[test]
    fn known_vectors() {
        let cases: [(&[u8], u32); 4] = [
            (b"", 0),
            (b"a", 0xe8b7_be43),
            (b"abc", 0x3524_41c2),
            (b"The quick brown fox jumps over the lazy dog", 0x414f_a339),
        ];
        for (bytes, expected) in cases {
            assert_eq!(Crc32::checksum(bytes), expected);
            for mut checksum in backends() {
                checksum.update(bytes);
                assert_eq!(checksum.finalize(), expected);
            }
        }
        assert_eq!(Crc32::checksum(&[0; 32]), 0x190a_55ad);
    }

    #[test]
    fn every_length_to_1024_matches_the_definition() {
        let bytes = pseudo_random(1024, 0x9e37_79b9);
        for len in 0..=1024 {
            let expected = bitwise(&bytes[..len]);
            assert_eq!(Crc32::checksum(&bytes[..len]), expected, "length {len}");
            let mut streamed = Crc32::new();
            streamed.update(&bytes[..len]);
            assert_eq!(streamed.finalize(), expected, "streamed length {len}");
        }
    }

    #[test]
    fn unaligned_starts_match_crc32fast() {
        let bytes = pseudo_random(4096 + 64, 7);
        for start in 0..64 {
            for len in [0, 1, 15, 16, 17, 63, 64, 255, 256, 257, 1000, 4096] {
                let slice = &bytes[start..start + len];
                assert_eq!(
                    Crc32::checksum(slice),
                    crc32fast::hash(slice),
                    "start {start} length {len}"
                );
            }
        }
    }

    #[test]
    fn incremental_updates_in_random_splits_match_one_shot() {
        let bytes = pseudo_random(70_000, 0x1234_5678);
        let expected = crc32fast::hash(&bytes);
        let mut state = 99u32;
        for _ in 0..50 {
            for mut checksum in backends() {
                let mut rest = &bytes[..];
                while !rest.is_empty() {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    let take = ((state >> 4) as usize % 5000).min(rest.len());
                    let (head, tail) = rest.split_at(take);
                    checksum.update(head);
                    rest = tail;
                }
                assert_eq!(checksum.finalize(), expected);
            }
        }
    }
}
