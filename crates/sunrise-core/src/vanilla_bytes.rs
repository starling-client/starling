//! Primitive encodings for vanilla Minecraft packet bodies.

use crate::nbt::{self, Tag};

/// An outgoing packet body.
#[derive(Default)]
pub struct Writer {
    pub data: Vec<u8>,
}
impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, v: u8) {
        self.data.push(v);
    }
    pub fn i8(&mut self, v: i8) {
        self.data.push(v as u8);
    }
    pub fn bool(&mut self, v: bool) {
        self.data.push(u8::from(v));
    }
    pub fn i16(&mut self, v: i16) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u16(&mut self, v: u16) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn varint(&mut self, v: i32) {
        write_varint(&mut self.data, v);
    }
    pub fn varlong(&mut self, v: i64) {
        let mut v = v as u64;
        loop {
            if v & !0x7F == 0 {
                self.data.push(v as u8);
                return;
            }
            self.data.push((v as u8 & 0x7F) | 0x80);
            v >>= 7;
        }
    }
    pub fn string(&mut self, s: &str) {
        self.varint(s.len() as i32);
        self.data.extend_from_slice(s.as_bytes());
    }
    pub fn byte_array(&mut self, b: &[u8]) {
        self.varint(b.len() as i32);
        self.data.extend_from_slice(b);
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.data.extend_from_slice(b);
    }
    pub fn uuid(&mut self, v: u128) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn nbt(&mut self, tag: &Tag) {
        match nbt::write_network(tag, nbt::Limits::default()) {
            Ok(bytes) => self.data.extend_from_slice(&bytes),
            // Unencodable tags (too deep) are sent as "no tag".
            Err(_) => self.data.push(0),
        }
    }
    pub fn long_array(&mut self, v: &[i64]) {
        self.varint(v.len() as i32);
        for &l in v {
            self.i64(l);
        }
    }
}
pub fn write_varint(out: &mut Vec<u8>, v: i32) {
    let mut v = v as u32;
    loop {
        if v & !0x7F == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
}
pub fn varint_size(v: i32) -> usize {
    let v = v as u32;
    (1..5).find(|i| v >> (7 * i) == 0).unwrap_or(5)
}
