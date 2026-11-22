//! Region codecs, including lz4-java's block framing (not the LZ4 frame format).
use flate2::{Decompress, FlushDecompress, Status};
use std::{
    io,
    sync::atomic::{AtomicU8, Ordering},
};
use sunrise_deflate::Deflater;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Compression {
    Gzip = 1,
    Zlib = 2,
    None = 3,
    Lz4 = 4,
}

/// The region file versions `RegionFileVersion.register` puts in its table, in
/// registration order: the names server.properties' `region-file-compression`
/// takes, and the ones an invalid value is told to use.
pub const REGION_FILE_NAMES: [&str; 4] = ["gzip", "deflate", "none", "lz4"];

/// `RegionFileVersion.selected`, which starts at `DEFAULT` (VERSION_DEFLATE).
static SELECTED: AtomicU8 = AtomicU8::new(Compression::Zlib as u8);

impl Compression {
    /// `RegionFileVersion.configure`: what `region-file-compression` names
    /// becomes the version every region file written from here on uses,
    /// whichever world it belongs to. An unknown name leaves the selection as
    /// it was, as vanilla logs the value and keeps going. False for a name
    /// that is not one of [`REGION_FILE_NAMES`].
    pub fn configure(name: &str) -> bool {
        let Some(compression) = REGION_FILE_NAMES
            .iter()
            .position(|known| *known == name)
            .map(|i| [Self::Gzip, Self::Zlib, Self::None, Self::Lz4][i])
        else {
            return false;
        };
        SELECTED.store(compression as u8, Ordering::Relaxed);
        true
    }
    /// `RegionFileVersion.getSelected`: the version a chunk or a poi region
    /// is written with.
    pub fn selected() -> Self {
        Self::try_from(SELECTED.load(Ordering::Relaxed)).unwrap_or(Self::Zlib)
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

impl TryFrom<u8> for Compression {
    type Error = io::Error;
    fn try_from(value: u8) -> io::Result<Self> {
        match value {
            1 => Ok(Self::Gzip),
            2 => Ok(Self::Zlib),
            3 => Ok(Self::None),
            4 => Ok(Self::Lz4),
            _ => Err(invalid(format!(
                "unsupported region compression ID {value}"
            ))),
        }
    }
}

impl Compression {
    pub fn decode(self, bytes: &[u8], max_output: usize) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        self.decode_into(bytes, max_output, &mut output)?;
        Ok(output)
    }
    /// [`decode`](Self::decode) into `output`, which keeps what was decoded
    /// before an error: what a stream reader would already have handed out.
    pub fn decode_into(
        self,
        bytes: &[u8],
        max_output: usize,
        output: &mut Vec<u8>,
    ) -> io::Result<()> {
        match self {
            Self::None => {
                if bytes.len() > max_output {
                    return Err(invalid("decompression limit exceeded"));
                }
                output.extend_from_slice(bytes);
                Ok(())
            }
            Self::Gzip => gzip_into(bytes, max_output, output, GzipStream::Java),
            Self::Zlib => zlib_into(bytes, max_output, output),
            Self::Lz4 => {
                LZ4_BUFFER.with_borrow_mut(|buffer| decode_lz4(bytes, max_output, output, buffer))
            }
        }
    }

    /// Gzip and zlib write the bytes `GZIPOutputStream` and
    /// `DeflaterOutputStream` write at their defaults: zlib 1.3.2's, as the
    /// JDK bundles it.
    pub fn encode(self, bytes: &[u8]) -> io::Result<Vec<u8>> {
        match self {
            Self::None => Ok(bytes.to_vec()),
            Self::Gzip | Self::Zlib => DEFLATER.with_borrow_mut(|(deflater, output)| {
                output.clear();
                if self == Self::Gzip {
                    deflater.gzip(bytes, output);
                } else {
                    deflater.zlib(bytes, output);
                }
                let encoded = output.as_slice().to_vec();
                if output.capacity() > SCRATCH_KEPT {
                    *output = Vec::new();
                }
                Ok(encoded)
            }),
            Self::Lz4 => Ok(encode_lz4(bytes)),
        }
    }
}

/// The most deflate output capacity a thread keeps between writes.
const SCRATCH_KEPT: usize = 1 << 20;

thread_local! {
    /// This thread's zlib inflater, reset for each chunk instead of a new
    /// one (and its window) allocated every time.
    static INFLATER: std::cell::RefCell<Option<Decompress>> = const { std::cell::RefCell::new(None) };
    /// This thread's deflate encoder and its output, kept between writes: the
    /// encoder's 250 KiB of state is allocated once, and the output keeps its
    /// capacity up to [`SCRATCH_KEPT`]. The encoder reserves room for the
    /// worst case, which is about the input's size, so a larger write frees it
    /// after copying its stream out.
    static DEFLATER: std::cell::RefCell<(Deflater, Vec<u8>)> =
        std::cell::RefCell::new((Deflater::new(), Vec::new()));
    /// This thread's LZ4 compressed-block scratch, retained between chunk reads.
    static LZ4_BUFFER: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Decode one complete gzip member, refusing any compressed suffix, including a second member.
/// For internal file formats that require an exact envelope; Java-format reads use [`Compression::Gzip`].
pub fn decode_single_gzip_member(bytes: &[u8], max_output: usize) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    gzip_into(bytes, max_output, &mut output, GzipStream::SingleMember)?;
    Ok(output)
}

#[derive(Clone, Copy)]
enum GzipStream {
    Java,
    SingleMember,
}

/// `GZIPInputStream` recognizes optional fields and verifies FHCRC, but does
/// not reject reserved FLG bits. Return the raw DEFLATE input, if recognized.
fn gzip_body(bytes: &[u8]) -> Option<&[u8]> {
    let (header, mut body) = bytes.split_at_checked(10)?;
    if header[..3] != [0x1f, 0x8b, 8] {
        return None;
    }
    let flags = header[3];
    if flags & 4 != 0 {
        let (len, rest) = body.split_first_chunk::<2>()?;
        body = rest.get(usize::from(u16::from_le_bytes(*len))..)?;
    }
    for flag in [8, 16] {
        if flags & flag != 0 {
            let end = body.iter().position(|&b| b == 0)?;
            body = &body[end + 1..];
        }
    }
    if flags & 2 != 0 {
        let (expected, rest) = body.split_first_chunk::<2>()?;
        let actual = crc32fast::hash(&bytes[..bytes.len() - body.len()]);
        if actual as u16 != u16::from_le_bytes(*expected) {
            return None;
        }
        body = rest;
    }
    Some(body)
}

fn gzip_into(
    bytes: &[u8],
    max_output: usize,
    output: &mut Vec<u8>,
    stream: GzipStream,
) -> io::Result<()> {
    let mut body = gzip_body(bytes).ok_or_else(|| invalid("invalid gzip header"))?;
    if output.len() > max_output {
        return Err(invalid("decompression limit exceeded"));
    }
    let limit = max_output.saturating_add(1);
    INFLATER.with_borrow_mut(|inflater| {
        let decoder = inflater.get_or_insert_with(|| Decompress::new(false));
        loop {
            decoder.reset(false);
            let start = output.len();
            loop {
                let before_in = decoder.total_in() as usize;
                let before_out = output.len();
                let input = &body[before_in..];
                let room = limit - before_out;
                if output.capacity() == before_out {
                    let want = (before_out - start).max(input.len().min(1024)).max(64);
                    output.reserve_exact(want.min(room));
                }
                let status = if output.capacity() - before_out <= room {
                    decoder.decompress_vec(input, output, FlushDecompress::None)
                } else {
                    // A reused buffer can already have more spare space than
                    // the limit allows. Don't inflate past the one-byte probe.
                    output.resize(before_out + room.min(8192), 0);
                    let status =
                        decoder.decompress(input, &mut output[before_out..], FlushDecompress::None);
                    output.truncate(start + decoder.total_out() as usize);
                    status
                };
                let status = match status {
                    Ok(status) => status,
                    Err(e) => {
                        // A stream reader does not deliver the failed call's bytes.
                        output.truncate(before_out);
                        return Err(invalid(e.to_string()));
                    }
                };
                if output.len() > max_output {
                    return Err(invalid("decompression limit exceeded"));
                }
                if status == Status::StreamEnd {
                    break;
                }
                if decoder.total_in() as usize == before_in && output.len() == before_out {
                    return Err(invalid("truncated gzip body"));
                }
            }
            let (trailer, remaining) = body[decoder.total_in() as usize..]
                .split_first_chunk::<8>()
                .ok_or_else(|| invalid("truncated gzip trailer"))?;
            let crc = u32::from_le_bytes(trailer[..4].try_into().unwrap());
            let size = u32::from_le_bytes(trailer[4..].try_into().unwrap());
            if crc != crc32fast::hash(&output[start..]) || size != (output.len() - start) as u32 {
                return Err(invalid("invalid gzip trailer"));
            }
            if matches!(stream, GzipStream::SingleMember) {
                return if remaining.is_empty() {
                    Ok(())
                } else {
                    Err(invalid("bytes follow the gzip member"))
                };
            }
            // A bad next header ends the Java stream. Once a header is recognized,
            // a damaged or missing block/trailer remains an error.
            let Some(next) = gzip_body(remaining) else {
                return Ok(());
            };
            body = next;
        }
    })
}

/// The zlib stream in `bytes` decoded onto `output`, at most `max_output`
/// bytes in all. A whole stream that fits is inflated straight into
/// `output`, which grows as it fills (from about eight times the stream, a
/// chunk's usual ratio, never past one byte over the limit); any other
/// stream (damaged, cut short, too long) is decoded again by
/// [`zlib_blocks`], so what an error leaves in `output` stays as it was.
fn zlib_into(bytes: &[u8], max_output: usize, output: &mut Vec<u8>) -> io::Result<()> {
    INFLATER.with_borrow_mut(|inflater| {
        let decoder = inflater.get_or_insert_with(|| Decompress::new(true));
        decoder.reset(true);
        let start = output.len();
        let room = max_output.saturating_sub(start);
        let mut first = Some(bytes.len().saturating_mul(8));
        loop {
            if output.capacity() == output.len() {
                let want = first.take().unwrap_or(output.len() - start).max(8192);
                output.reserve_exact(want.min((start + room).saturating_add(1) - output.len()));
            }
            let before_in = decoder.total_in();
            let before_out = output.len();
            let status =
                decoder.decompress_vec(&bytes[before_in as usize..], output, FlushDecompress::None);
            let fits = output.len() - start <= room;
            match status {
                Ok(Status::StreamEnd) if fits => return Ok(()),
                Ok(_)
                    if fits && (decoder.total_in() != before_in || output.len() != before_out) => {}
                _ => break,
            }
        }
        output.truncate(start);
        decoder.reset(true);
        zlib_blocks(decoder, bytes, max_output, output)
    })
}

/// The zlib stream in `bytes` inflated 8 KiB at a time, each block copied
/// onto `output` while the limit allows: on an error `output` keeps the
/// blocks before it, as a stream reader would have handed them out.
fn zlib_blocks(
    decoder: &mut Decompress,
    bytes: &[u8],
    max_output: usize,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    let mut block = [0; 8192];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = match decoder.decompress(
            &bytes[before_in as usize..],
            &mut block,
            FlushDecompress::None,
        ) {
            Ok(status) => status,
            // InflaterInputStream ends the stream if a dictionary is required
            // but none was supplied.
            Err(e) if e.needs_dictionary().is_some() => return Ok(()),
            Err(e) => return Err(invalid(e.to_string())),
        };
        let produced = (decoder.total_out() - before_out) as usize;
        if produced > max_output.saturating_sub(output.len()) {
            return Err(invalid("decompression limit exceeded"));
        }
        output.extend_from_slice(&block[..produced]);
        if status == Status::StreamEnd {
            return Ok(());
        }
        if decoder.total_in() == before_in && produced == 0 {
            return Err(invalid("truncated zlib stream"));
        }
    }
}

const MAGIC: &[u8; 8] = b"LZ4Block";
fn checksum(bytes: &[u8]) -> u32 {
    // The Checksum adapter used by the pinned Java library masks to 28 bits.
    xxhash_rust::xxh32::xxh32(bytes, 0x9747_b28c) & 0x0fff_ffff
}

fn decode_lz4(
    mut bytes: &[u8],
    max_output: usize,
    output: &mut Vec<u8>,
    buffer: &mut Vec<u8>,
) -> io::Result<()> {
    // `LZ4BlockInputStream.compressedBuffer`: each header is read into it,
    // and a block's bytes over its start; it grows by half when too small.
    // The decompressor may read up to its end, past the block.
    // Each stream starts from 21 zero bytes, as a new buffer would: the
    // decompressor may read past a short block into them, so the last
    // stream's bytes must not be left there. The allocation is kept.
    buffer.clear();
    buffer.resize(21, 0);
    loop {
        let header = bytes
            .get(..21)
            .ok_or_else(|| invalid("truncated LZ4 header"))?;
        if &header[..8] != MAGIC {
            return Err(invalid("invalid LZ4 block magic"));
        }
        let method = header[8] & 0xf0;
        if !matches!(method, 0x10 | 0x20) {
            return Err(invalid("invalid LZ4 block method"));
        }
        let compressed = u32::from_le_bytes(header[9..13].try_into().unwrap()) as usize;
        let raw = u32::from_le_bytes(header[13..17].try_into().unwrap()) as usize;
        let expected = u32::from_le_bytes(header[17..21].try_into().unwrap());
        if raw > (1 << (10 + (header[8] & 15)))
            || (raw == 0) != (compressed == 0)
            || (method == 0x10 && raw != compressed)
        {
            return Err(invalid("invalid LZ4 block lengths"));
        }
        buffer[..21].copy_from_slice(header);
        bytes = &bytes[21..];
        if raw == 0 {
            if expected != 0 {
                return Err(invalid("invalid LZ4 terminator checksum"));
            }
            return Ok(());
        }
        if raw > max_output.saturating_sub(output.len()) {
            return Err(invalid("decompression limit exceeded"));
        }
        let block = bytes
            .get(..compressed)
            .ok_or_else(|| invalid("truncated LZ4 payload"))?;
        let start = output.len();
        let decoded = if method == 0x10 {
            output.extend_from_slice(block);
            Ok(())
        } else {
            if buffer.len() < compressed {
                buffer.resize(compressed.max(buffer.len() * 3 / 2), 0);
            }
            buffer[..compressed].copy_from_slice(block);
            output.resize(start + raw, 0);
            // `refill`: the block must be read to its last byte, no further.
            if decompress_lz4_java(buffer, &mut output[start..]) == Some(compressed) {
                Ok(())
            } else {
                Err(invalid("LZ4 stream is corrupted"))
            }
        }
        .and_then(|()| {
            if checksum(&output[start..]) == expected {
                Ok(())
            } else {
                Err(invalid("LZ4 checksum mismatch"))
            }
        });
        // lz4-java hands out a block only once it has decoded and checked it.
        if let Err(e) = decoded {
            output.truncate(start);
            return Err(e);
        }
        bytes = &bytes[compressed..];
    }
}

/// lz4-java's `LZ4JavaSafeFastDecompressor.decompress`, which
/// `LZ4BlockInputStream` uses: `dest` filled from `src`, returning how many
/// bytes of `src` it read, or `None` where it throws `LZ4Exception`. Its end
/// conditions are its own (a sequence whose literals leave less than 8 bytes of
/// output, or of input past them, must be the last and end the output), and
/// an offset of 0 fills the match with zeros.
fn decompress_lz4_java(src: &[u8], dest: &mut [u8]) -> Option<usize> {
    let (src_end, dest_end) = (src.len(), dest.len());
    if dest_end == 0 {
        return (src.first() == Some(&0)).then_some(1);
    }
    // The run-length bytes after a 15 in the token; a run cut off by the end of
    // the input adds a last 255, as the Java loop does.
    let length = |s: &mut usize, mut len: usize| {
        let mut last = 0xff;
        while *s < src_end {
            last = src[*s];
            *s += 1;
            if last != 0xff {
                break;
            }
            len += 255;
        }
        len + usize::from(last)
    };
    let (mut s, mut d) = (0, 0);
    loop {
        if s >= src_end {
            return None;
        }
        let token = src[s];
        s += 1;
        let mut literals = usize::from(token >> 4);
        if literals == 15 {
            literals = length(&mut s, literals);
        }
        let literal_end = d + literals;
        if dest_end < literal_end + 8 || src_end - s < 8 + literals {
            if literal_end != dest_end || src_end - s < literals {
                return None;
            }
            dest[d..literal_end].copy_from_slice(&src[s..s + literals]);
            return Some(s + literals);
        }
        dest[d..literal_end].copy_from_slice(&src[s..s + literals]);
        s += literals;
        d = literal_end;
        let offset = usize::from(u16::from_le_bytes([src[s], src[s + 1]]));
        s += 2;
        if offset > d {
            return None;
        }
        let mut matched = usize::from(token & 15);
        if matched == 15 {
            matched = length(&mut s, matched);
        }
        let match_end = d + matched + 4;
        if match_end > dest_end {
            return None;
        }
        if offset == 0 {
            dest[d..match_end].fill(0);
        } else {
            for i in d..match_end {
                dest[i] = dest[i - offset];
            }
        }
        d = match_end;
    }
}

fn encode_lz4(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for block in bytes.chunks(65536) {
        let compressed = lz4_flex::block::compress(block);
        let (token, payload) = if compressed.len() < block.len() {
            (0x26, compressed.as_slice())
        } else {
            (0x16, block)
        };
        output.extend_from_slice(MAGIC);
        output.push(token);
        output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        output.extend_from_slice(&(block.len() as u32).to_le_bytes());
        output.extend_from_slice(&checksum(block).to_le_bytes());
        output.extend_from_slice(payload);
    }
    output.extend_from_slice(MAGIC);
    output.push(0x16);
    output.extend_from_slice(&[0; 12]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    /// lz4-java's `LZ4BlockInputStream` decides which blocks are corrupted
    /// (#5064). Each stream below has a correct checksum; the results are
    /// lz4-java 1.10.1's.
    #[test]
    fn lz4_blocks_are_read_as_lz4_java_reads_them() {
        let lz4 = |s: &str| Compression::Lz4.decode(&hex(s), usize::MAX);
        const END: &str = "4c5a34426c6f636b16000000000000000000000000";
        // The issue's stream: its last match ends 2 bytes before the output's end.
        assert!(lz4(&format!(
            "4c5a34426c6f636b2620000000270000002817d90bb112cd89673ffd3d11ad97420600708664345bbd2cb70b00248fc80b0020a52b{END}"
        ))
        .is_err());
        // A literal with less than 8 bytes of input after it must end the output.
        assert!(
            lz4(&format!(
                "4c5a34426c6f636b260500000005000000f5a1570e100301000c{END}"
            ))
            .is_err()
        );
        // An offset of 0 fills the match with zeros.
        assert_eq!(
            lz4(&format!(
                "4c5a34426c6f636b260400000010000000eace86020c000007{END}"
            ))
            .unwrap(),
            [0; 16]
        );
    }
    #[test]
    fn gzip_ignores_junk_after_a_complete_member_but_checks_later_members() {
        // GZIPOutputStream's output for [41], captured from Temurin 25.0.5.
        let first = [
            0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 0xd3, 4, 0, 0xe1, 0x77, 0xb0, 0x90, 1, 0, 0, 0,
        ];
        assert_eq!(decode_single_gzip_member(&first, 2).unwrap(), [41]);
        for junk in [&[0, 0, 0][..], &first[..3], &[7; 20]] {
            let input = [&first[..], junk].concat();
            assert_eq!(Compression::Gzip.decode(&input, 2).unwrap(), [41]);
            assert!(decode_single_gzip_member(&input, 2).is_err());
        }
        let mut two = [first, first].concat();
        assert_eq!(Compression::Gzip.decode(&two, 2).unwrap(), [41, 41]);
        assert!(decode_single_gzip_member(&two, 2).is_err());
        two[2 * first.len() - 8] ^= 1;
        assert!(Compression::Gzip.decode(&two, 2).is_err());
        assert!(Compression::Gzip.decode(&first[..3], 2).is_err());
    }

    #[test]
    fn gzip_accepts_reserved_bits_and_checks_optional_header_crc() {
        // Temurin 25.0.5 GZIPInputStream reads [41] from this fixture: all
        // FLG bits set, 3 extra bytes, a name, a comment and a valid FHCRC.
        let member = [
            0x1f, 0x8b, 8, 0xff, 0, 0, 0, 0, 0, 0xff, 3, 0, 0xa6, 0, 0xc4, b't', b'e', b's', b't',
            0, b'n', b'a', b'm', b'e', 0, 0x14, 0x67, 0xd3, 4, 0, 0xe1, 0x77, 0xb0, 0x90, 1, 0, 0,
            0,
        ];
        assert_eq!(Compression::Gzip.decode(&member, 1).unwrap(), [41]);
        assert_eq!(
            Compression::Gzip
                .decode(&[member, member].concat(), 2)
                .unwrap(),
            [41, 41],
        );
        let mut wrong_crc = member;
        wrong_crc[26] ^= 1;
        assert!(Compression::Gzip.decode(&wrong_crc, 1).is_err());
        assert_eq!(
            Compression::Gzip
                .decode(&[member, wrong_crc].concat(), 1)
                .unwrap(),
            [41],
        );
        // A good continuation header with no body is not trailing junk.
        assert!(
            Compression::Gzip
                .decode(&[&member[..], &member[..27]].concat(), 2)
                .is_err()
        );
    }

    #[test]
    fn gzip_keeps_only_bytes_from_successful_inflate_calls() {
        // One non-final stored block, then an invalid block type (BTYPE=3).
        // The stored block fills one output window and produces another 256
        // bytes in the same call that encounters the invalid block type.
        const WINDOW: usize = 8192;
        let len = (WINDOW + 256) as u16;
        let mut gzip = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 0];
        gzip.extend_from_slice(&len.to_le_bytes());
        gzip.extend_from_slice(&(!len).to_le_bytes());
        gzip.resize(gzip.len() + WINDOW, b'a');
        gzip.resize(gzip.len() + 256, b'b');
        gzip.push(0x07);
        for capacity in [WINDOW, 4 * WINDOW] {
            // The second buffer already has more spare capacity than the
            // decode budget, so this also exercises the capped-output path.
            let mut raw = Vec::with_capacity(capacity);
            assert!(
                Compression::Gzip
                    .decode_into(&gzip, 2 * WINDOW, &mut raw)
                    .is_err()
            );
            assert_eq!(raw, [b'a'; WINDOW]);
        }
    }

    #[test]
    fn zlib_needs_dictionary_is_end_of_stream_like_inflater_input_stream() {
        // A valid zlib header with FDICT set and a dictionary identifier, no
        // dictionary supplied. Temurin 25.0.5 InflaterInputStream reads no bytes.
        let needs_dictionary = [0x78, 0xf9, 0x77, 0x0a, 0xed, 0x81];
        assert!(
            Compression::Zlib
                .decode(&needs_dictionary, 32)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn all_codecs_roundtrip_and_enforce_limits() {
        for codec in [
            Compression::Gzip,
            Compression::Zlib,
            Compression::None,
            Compression::Lz4,
        ] {
            for bytes in [
                vec![],
                vec![0; 131073],
                (0..262144).map(|i| (i * 37) as u8).collect(),
            ] {
                let encoded = codec.encode(&bytes).unwrap();
                assert_eq!(codec.decode(&encoded, bytes.len()).unwrap(), bytes);
                if !bytes.is_empty() {
                    assert!(codec.decode(&encoded, bytes.len() - 1).is_err());
                }
                if codec != Compression::None {
                    assert!(
                        codec
                            .decode(&encoded[..encoded.len() - 1], bytes.len())
                            .is_err()
                    );
                }
            }
        }
    }
    /// The zlib decoder before it inflated in place: a new inflater for
    /// each stream, and 8 KiB blocks copied out.
    fn blockwise(bytes: &[u8], max_output: usize, output: &mut Vec<u8>) -> io::Result<()> {
        zlib_blocks(&mut Decompress::new(true), bytes, max_output, output)
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Inflating in place keeps what the block decoder kept and fails as it
    /// failed: whole streams, every limit around them, streams cut short and
    /// damaged ones, decoded after bytes already in the output.
    #[test]
    fn zlib_in_place_matches_the_block_decoder() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut cases = 0;
        for n in [0usize, 1, 8191, 8192, 8193, 20_000, 100_000] {
            let raw: Vec<u8> = (0..n)
                .map(|i| {
                    if i % 700 < 400 {
                        7
                    } else {
                        random() as u8 & 31
                    }
                })
                .collect();
            let encoded = Compression::Zlib.encode(&raw).unwrap();
            let mut streams = vec![encoded.clone()];
            for cut in [1, 2, encoded.len() / 2, encoded.len() - 1] {
                streams.push(encoded[..encoded.len().saturating_sub(cut)].to_vec());
            }
            for at in [
                2,
                3,
                encoded.len() / 3,
                encoded.len() / 2,
                encoded.len() - 5,
            ] {
                let mut damaged = encoded.clone();
                if let Some(b) = damaged.get_mut(at) {
                    *b ^= 0x5a;
                }
                streams.push(damaged);
            }
            for stream in &streams {
                for limit in [
                    0,
                    1,
                    8191,
                    8192,
                    8193,
                    n / 2,
                    n.saturating_sub(1),
                    n,
                    n + 1,
                    n + 9000,
                ] {
                    for before in [0usize, 5] {
                        let mut old = vec![9; before];
                        let mut new = old.clone();
                        let a = blockwise(stream, limit + before, &mut old);
                        let b = Compression::Zlib.decode_into(stream, limit + before, &mut new);
                        assert_eq!(
                            a.map_err(|e| e.to_string()),
                            b.map_err(|e| e.to_string()),
                            "{n} bytes, stream of {}, limit {limit}",
                            stream.len()
                        );
                        assert_eq!(
                            old,
                            new,
                            "{n} bytes, stream of {}, limit {limit}",
                            stream.len()
                        );
                        cases += 1;
                    }
                }
            }
        }
        assert!(cases > 1000);
        // No limit at all.
        let encoded = Compression::Zlib.encode(b"Sunrise").unwrap();
        assert_eq!(
            Compression::Zlib.decode(&encoded, usize::MAX).unwrap(),
            b"Sunrise"
        );
    }

    /// `RegionFileVersion.configure` and `getSelected`: the names
    /// server.properties' `region-file-compression` takes, the default before
    /// any is set, and an unknown name leaving the selection alone.
    #[test]
    fn the_selected_region_version_follows_the_property() {
        assert_eq!(
            Compression::selected(),
            Compression::Zlib,
            "VERSION_DEFLATE is DEFAULT"
        );
        for (name, want) in [
            ("gzip", Compression::Gzip),
            ("deflate", Compression::Zlib),
            ("none", Compression::None),
            ("lz4", Compression::Lz4),
        ] {
            assert!(Compression::configure(name), "{name}");
            assert_eq!(Compression::selected(), want, "{name}");
        }
        // Vanilla logs the value and keeps the version it had.
        assert!(!Compression::configure("zstd"));
        assert!(!Compression::configure("LZ4"), "the names are exact");
        assert_eq!(Compression::selected(), Compression::Lz4);
        assert!(Compression::configure("deflate"));
        assert_eq!(Compression::selected(), Compression::Zlib);
    }
    /// Bytes Temurin 25.0.5 wrote for 8193 zero bytes through
    /// `DeflaterOutputStream` and `GZIPOutputStream`; zlib-rs, the encoder
    /// before, wrote a different distance code.
    #[test]
    fn deflate_writes_the_jdks_bytes() {
        let zeros = [0; 8193];
        assert_eq!(
            Compression::Zlib.encode(&zeros).unwrap(),
            hex("789cedc1010d000000c2a0f74f6d0e37a000000000000000807b0320010001")
        );
        assert_eq!(
            Compression::Gzip.encode(&zeros).unwrap(),
            hex(
                "1f8b08000000000000ffedc1010d000000c2a0f74f6d0e37a000000000000000807b03494cb82501200000"
            )
        );
    }
    #[test]
    fn checksums_reject_corruption() {
        for codec in [Compression::Gzip, Compression::Zlib, Compression::Lz4] {
            let mut encoded = codec.encode(b"Sunrise world data").unwrap();
            let i = if codec == Compression::Lz4 {
                17
            } else {
                encoded.len() - 5
            };
            encoded[i] ^= 1;
            assert!(codec.decode(&encoded, 1000).is_err());
        }
    }
}
