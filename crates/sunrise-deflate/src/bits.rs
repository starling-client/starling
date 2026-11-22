//! The deflate bit stream, least significant bit first.
//!
//! zlib keeps 16 bits in `bi_buf` and moves them to its pending buffer; this
//! keeps up to 63 bits and appends whole 32-bit words to the caller's vector.
//! The bit stream, and so every output byte, is the same.

pub(crate) struct BitWriter<'a> {
    out: &'a mut Vec<u8>,
    buf: u64,
    valid: u32,
}

impl<'a> BitWriter<'a> {
    pub(crate) fn new(out: &'a mut Vec<u8>) -> Self {
        Self {
            out,
            buf: 0,
            valid: 0,
        }
    }

    /// Appends the low `len` bits of `value` (`len <= 32`, no higher bits set).
    #[inline(always)]
    pub(crate) fn send(&mut self, value: u64, len: u32) {
        self.buf |= value << self.valid;
        self.valid += len;
        if self.valid >= 32 {
            self.out.extend_from_slice(&(self.buf as u32).to_le_bytes());
            self.buf >>= 32;
            self.valid -= 32;
        }
    }

    /// `bi_windup`: writes the remaining bits, padded with zeros to a byte boundary.
    pub(crate) fn windup(&mut self) {
        let bytes = self.valid.div_ceil(8) as usize;
        self.out.extend_from_slice(&self.buf.to_le_bytes()[..bytes]);
        self.buf = 0;
        self.valid = 0;
    }

    /// Appends whole bytes; the stream must be at a byte boundary.
    pub(crate) fn put_bytes(&mut self, bytes: &[u8]) {
        debug_assert_eq!(self.valid, 0);
        self.out.extend_from_slice(bytes);
    }
}
