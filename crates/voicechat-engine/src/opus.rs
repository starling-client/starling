//! The bundled libopus (compiled by build.rs): 48 kHz mono, 20 ms frames.
use std::os::raw::{c_int, c_uchar};

pub const SAMPLE_RATE: i32 = 48_000;
pub const FRAME: usize = 960;
/// The longest packet a decoder may return: 120 ms.
const MAX_FRAME: usize = 5760;
const OPUS_RESET_STATE: c_int = 4028;
const OPUS_SET_BITRATE: c_int = 4002;

#[repr(C)]
struct RawEncoder {
    _p: [u8; 0],
}
#[repr(C)]
struct RawDecoder {
    _p: [u8; 0],
}

unsafe extern "C" {
    fn opus_encoder_create(
        fs: i32,
        channels: c_int,
        application: c_int,
        error: *mut c_int,
    ) -> *mut RawEncoder;
    fn opus_encoder_destroy(st: *mut RawEncoder);
    fn opus_encode(
        st: *mut RawEncoder,
        pcm: *const i16,
        frame_size: c_int,
        data: *mut c_uchar,
        max: i32,
    ) -> i32;
    fn opus_encoder_ctl(st: *mut RawEncoder, request: c_int, ...) -> c_int;
    fn opus_decoder_create(fs: i32, channels: c_int, error: *mut c_int) -> *mut RawDecoder;
    fn opus_decoder_destroy(st: *mut RawDecoder);
    fn opus_decode(
        st: *mut RawDecoder,
        data: *const c_uchar,
        len: i32,
        pcm: *mut i16,
        frame_size: c_int,
        fec: c_int,
    ) -> c_int;
    fn opus_decoder_ctl(st: *mut RawDecoder, request: c_int, ...) -> c_int;
}

pub struct Encoder(*mut RawEncoder);
// The state is plain memory owned by this value; it is used from one thread at a time.
unsafe impl Send for Encoder {}

impl Encoder {
    /// `application`: 2048 VoIP, 2049 audio, 2051 restricted low delay.
    pub fn new(application: i32) -> Option<Self> {
        let mut err = 0;
        let st = unsafe { opus_encoder_create(SAMPLE_RATE, 1, application, &mut err) };
        if st.is_null() || err != 0 {
            return None;
        }
        // 24 kbit/s: clear speech, well under a typical MTU.
        unsafe { opus_encoder_ctl(st, OPUS_SET_BITRATE, 24_000 as c_int) };
        Some(Self(st))
    }
    /// One 960-sample frame.
    pub fn encode(&mut self, pcm: &[i16]) -> Option<Vec<u8>> {
        if pcm.len() != FRAME {
            return None;
        }
        let mut out = vec![0u8; crate::proto::MAX_OPUS];
        let n = unsafe {
            opus_encode(
                self.0,
                pcm.as_ptr(),
                FRAME as c_int,
                out.as_mut_ptr(),
                out.len() as i32,
            )
        };
        if n < 0 {
            return None;
        }
        out.truncate(n as usize);
        Some(out)
    }
    pub fn reset(&mut self) {
        unsafe { opus_encoder_ctl(self.0, OPUS_RESET_STATE) };
    }
}
impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { opus_encoder_destroy(self.0) }
    }
}

pub struct Decoder(*mut RawDecoder);
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn new() -> Option<Self> {
        let mut err = 0;
        let st = unsafe { opus_decoder_create(SAMPLE_RATE, 1, &mut err) };
        if st.is_null() || err != 0 {
            return None;
        }
        Some(Self(st))
    }
    /// A packet's samples; `None` conceals one lost 20 ms frame.
    pub fn decode(&mut self, packet: Option<&[u8]>) -> Vec<i16> {
        let mut pcm = vec![0i16; MAX_FRAME];
        let (ptr, len, frames) = match packet {
            Some(p) => (p.as_ptr(), p.len() as i32, MAX_FRAME),
            None => (std::ptr::null(), 0, FRAME),
        };
        let n = unsafe { opus_decode(self.0, ptr, len, pcm.as_mut_ptr(), frames as c_int, 0) };
        pcm.truncate(n.max(0) as usize);
        pcm
    }
    pub fn reset(&mut self) {
        unsafe { opus_decoder_ctl(self.0, OPUS_RESET_STATE) };
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { opus_decoder_destroy(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_survives_the_round_trip() {
        let mut enc = Encoder::new(2048).unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut energy_in = 0.0;
        let mut energy_out = 0.0;
        for f in 0..20 {
            let pcm: Vec<i16> = (0..FRAME)
                .map(|i| {
                    (((f * FRAME + i) as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48000.0).sin()
                        * 8000.0) as i16
                })
                .collect();
            let packet = enc.encode(&pcm).unwrap();
            assert!(!packet.is_empty() && packet.len() < 400);
            let out = dec.decode(Some(&packet));
            assert_eq!(out.len(), FRAME);
            if f >= 5 {
                energy_in += pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>();
                energy_out += out.iter().map(|&s| (s as f64).powi(2)).sum::<f64>();
            }
        }
        let ratio = energy_out / energy_in;
        assert!((0.5..2.0).contains(&ratio), "energy ratio {ratio}");
        assert_eq!(dec.decode(None).len(), FRAME, "concealment");
    }
}
