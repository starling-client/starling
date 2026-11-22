//! Deflate that writes the same bytes as zlib 1.3.2, the zlib bundled in the
//! JDK vanilla runs on.
//!
//! Vanilla compresses region chunks, `level.dat`, player data and structure
//! files with `java.util.zip` at the default level (6) and strategy, and the
//! JDK hands that to its bundled zlib. A different deflate implementation
//! writes valid streams with different bytes; this crate is a port of zlib
//! 1.3.2's own algorithm (`deflate.c` and `trees.c`) for that one
//! configuration, so saves match vanilla's byte for byte. See the README for
//! what was ported and how it is checked.
//!
//! A [`Deflater`] holds zlib's encoder state (about 250 KiB) and is reused
//! from call to call; each call compresses one whole input, as one
//! `deflate(Z_FINISH)`, and appends the stream to the caller's vector. Each
//! call first reserves zlib's worst-case bound for its input (about the input's
//! size, plus the wrapper), so it allocates nothing only once the vector's
//! spare capacity covers that bound, however well the input compresses.
//!
//! "The JDK's bytes" below means a JDK that bundles zlib 1.3.2, as Temurin
//! 25.0.5 does; a JDK built against a different zlib may produce different
//! bytes.
//!
//! zlib's notice is in this crate's LICENSE; the ported files say what was
//! altered.

#![forbid(unsafe_code)]

mod bits;
mod deflate;
mod trees;

use bits::BitWriter;

/// The gzip header `java.util.zip.GZIPOutputStream` writes: deflate, no flags,
/// no modification time, no extra flags, operating system 255 ("unknown").
const GZIP_HEADER: [u8; 10] = [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];

/// zlib's encoder for windowBits 15, memLevel 8, level 6 and the default
/// strategy: what `new java.util.zip.Deflater()` uses.
pub struct Deflater {
    state: deflate::State,
}

impl Default for Deflater {
    fn default() -> Self {
        Self::new()
    }
}

impl Deflater {
    /// Bytes of encoder state a `Deflater` holds (window, hash chains, trees and symbols).
    pub const STATE_BYTES: usize = deflate::STATE_BYTES;

    pub fn new() -> Self {
        Self {
            state: deflate::State::new(),
        }
    }

    /// Appends a zlib stream of `input` (RFC 1950: header `78 9c`, deflate,
    /// Adler-32), byte-identical to `java.util.zip.Deflater` at its defaults,
    /// and to `DeflaterOutputStream` around it, on a JDK bundling zlib 1.3.2.
    pub fn zlib(&mut self, input: &[u8], out: &mut Vec<u8>) {
        out.reserve(bound(input.len()) + 6);
        out.extend_from_slice(&[0x78, 0x9c]);
        self.state.compress(input, &mut BitWriter::new(out));
        out.extend_from_slice(&simd_adler32::adler32(&input).to_be_bytes());
    }

    /// Appends a gzip member of `input` as `java.util.zip.GZIPOutputStream`
    /// writes it on a JDK bundling zlib 1.3.2: its fixed header, raw deflate, CRC-32 and the length mod 2³²,
    /// both little-endian.
    pub fn gzip(&mut self, input: &[u8], out: &mut Vec<u8>) {
        out.reserve(bound(input.len()) + 18);
        out.extend_from_slice(&GZIP_HEADER);
        self.state.compress(input, &mut BitWriter::new(out));
        out.extend_from_slice(&crc32fast::hash(input).to_le_bytes());
        out.extend_from_slice(&(input.len() as u32).to_le_bytes());
    }

    /// Appends a raw deflate stream of `input` (RFC 1951, no wrapper), as
    /// `new Deflater(Deflater.DEFAULT_COMPRESSION, true)` writes it on a JDK
    /// bundling zlib 1.3.2.
    pub fn raw(&mut self, input: &[u8], out: &mut Vec<u8>) {
        out.reserve(bound(input.len()));
        self.state.compress(input, &mut BitWriter::new(out));
    }
}

/// `deflateBound` for the default parameters, without the wrapper.
fn bound(len: usize) -> usize {
    len + (len >> 12) + (len >> 14) + (len >> 25) + 13 - 6
}

#[cfg(test)]
mod tests;
