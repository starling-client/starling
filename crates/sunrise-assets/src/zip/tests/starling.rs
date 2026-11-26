//! Starling's tests of zip.rs, out of Sunrise's file (tools/starling/move_out.py).
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// A protected pack's directory records a wrong uncompressed size (here 1 byte for a
/// 40 KiB file): Java's `ZipFile` inflates the whole entry anyway.
#[test]
fn an_entry_inflates_whatever_size_the_directory_records() {
    let body = "a texture's bytes ".repeat(2300);
    let data = miniz_oxide::deflate::compress_to_vec(body.as_bytes(), 6);
    let name = "pack.png";
    let mut out = Vec::new();
    out.extend(0x0403_4b50u32.to_le_bytes());
    out.extend([20, 0, 0, 0, 8, 0]);
    out.extend([0; 8]);
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend((name.len() as u16).to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out.extend(name.as_bytes());
    out.extend(&data);
    let start = out.len() as u32;
    let mut central = Vec::new();
    central.extend(0x0201_4b50u32.to_le_bytes());
    central.extend([20, 0, 20, 0, 0, 0, 8, 0]);
    central.extend([0; 8]);
    central.extend((data.len() as u32).to_le_bytes());
    central.extend(1u32.to_le_bytes());
    central.extend((name.len() as u16).to_le_bytes());
    central.extend([0; 12]);
    central.extend(0u32.to_le_bytes());
    central.extend(name.as_bytes());
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0; 4]);
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend([0; 2]);
    let zip = Zip::from_bytes(out).unwrap();
    assert_eq!(zip.read(name).unwrap(), body.as_bytes());
}
