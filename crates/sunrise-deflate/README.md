# sunrise-deflate

This crate compresses data with deflate and writes exactly the bytes that zlib 1.3.2 writes at the default
settings.

Vanilla writes region chunks, `level.dat`, player data and structure files through `java.util.zip` with its
defaults: level 6, the default strategy, a 32 KiB window and memLevel 8. The JDK passes that work to its bundled
zlib, which is 1.3.2 in Temurin 25.0.5. Any other deflate implementation (zlib-ng, zlib-rs, miniz, libdeflate)
writes streams that decompress to the same data but have different bytes. To make Sunrise's saves byte-identical,
this crate ports zlib's own algorithm.

```rust
let mut deflater = sunrise_deflate::Deflater::new(); // keep one per thread
let mut out = Vec::new();
deflater.zlib(&nbt, &mut out); // region chunks: DeflaterOutputStream
deflater.gzip(&nbt, &mut out); // level.dat, player data, structures: GZIPOutputStream
```

Each call compresses one whole input, the same as `setInput(all)`, `finish()` and draining the output. Vanilla's
streams produce these same bytes; see the clean-room spec on compressed NBT bytes. A `Deflater` keeps zlib's
state, `Deflater::STATE_BYTES` (about 250 KiB). Each call reserves zlib's worst-case raw deflate size for its
input (about the input's size) plus the stream wrapper, if any, before it starts, so a call allocates nothing only once the vector's spare capacity covers
that bound, however small the result. The crate has no `unsafe` code.

## What was ported

The source is the official zlib 1.3.2 release, `zlib-1.3.2.tar.gz`, with SHA-256
`bb329a0a2cd0274d05519d61c667c062e06990d72e125ee2dfa8de64f0119d16`. zlib.net and the GitHub release
madler/zlib v1.3.2 serve the same file.

- **`deflate.c`**: the parts level 6 reaches: `deflate_slow` (lazy matching), `longest_match`, `fill_window`
  (including the `high_water` zeroing), `slide_hash` and the hash chains. Also the zlib header and the Adler-32
  trailer that `deflate()` writes.
- **`trees.c`**: `build_tree`, `gen_bitlen`, `gen_codes`, `scan_tree`, `send_tree`, `build_bl_tree`,
  `send_all_trees`, `compress_block`, `_tr_stored_block` and `_tr_flush_block`, which chooses between a stored,
  fixed or dynamic block. Tie-breaks follow zlib's exactly: heap order, node depth, and the order in which the
  Freq/Dad/Len/Code unions are overwritten.

Some changes are made for speed. None of them changes the bytes:

- **Match length.** `longest_match` compares eight bytes at a time. Like zlib, it skips byte 2, which the hash
  already fixes, and caps the length at 258. zlib compares one byte at a time and gets the same length.
- **Bit buffer.** It holds up to 63 bits and writes whole 32-bit words straight to the caller's vector. zlib uses
  a 16-bit buffer and a pending buffer. The bit stream is the same.
- **Symbol buffer.** Symbols go into separate distance and literal arrays, zlib's `LIT_MEM` layout. A block still
  ends after the same 16 383 symbols.
- **Checksums.** Adler-32 comes from `simd-adler32` and CRC-32 from `crc32fast`. Both are computed over the whole
  input once.

Not ported, because vanilla never reaches them:

- other levels and strategies;
- flush modes other than finishing;
- preset dictionaries;
- input fed in pieces.

Levels 4 to 9 differ from level 6 only in `configuration_table` and the header's level bits, so adding them
later is a small change.

## 1.3.1 and 1.3.2

zlib 1.3.2 compresses exactly like 1.3.1. Between the two releases, `deflate.c`, `deflate.h` and `trees.c` changed
only in these ways:

- casts;
- `deflateBound` overflow checks;
- `deflateCopy` and `deflatePending`;
- the new `deflateUsed` with its `bi_used` field, and the `slid` field;
- 16-bit and MSVC guards.

None of these affects the compressed bytes. So a faithful port of 1.3.1's deflate, such as the
`zlib-bitexact-rs` crate (level 9 only), would write the same bytes as this one at the same settings.

## How it is checked

- `cargo test` compares the output with streams written by Temurin 25.0.5's own `DeflaterOutputStream` and
  `GZIPOutputStream`, stored in `src/fixtures/`.
- jvm-parity's jdk-kit `deflate` family (variant `sunrise-deflate`) compares it with the JVM on seeded inputs:
  NBT-shaped data, random bytes, repetitive data, the empty input, and lengths around the window, block and buffer
  edges up to a few MiB. The kit also reports speed, allocations and table bytes against HotSpot's `Deflater`.

## Licence

This crate is an altered version of parts of zlib and is distributed under zlib's licence. zlib's notice is in
`LICENSE`, and each ported file says what was changed.
