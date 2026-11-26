//! Client audio streams, voice payloads and microphone capture.

// ANCHOR: play_clip
use mod_api::{AudioRefusal, AudioStream, AudioStreamRef, ClientHost, SoundSource, StreamSpec};

struct Clip(std::vec::IntoIter<i16>);

impl AudioStream for Clip {
    fn read(&mut self, pcm: &mut [i16]) -> usize {
        let mut frames = 0;
        for (out, sample) in pcm.iter_mut().zip(self.0.by_ref()) {
            *out = sample;
            frames += 1; // Mono: one sample per frame.
        }
        frames // 0 signals EOF.
    }
}

pub fn play_clip(
    host: &dyn ClientHost,
    mono_pcm: Vec<i16>,
) -> Result<AudioStreamRef, AudioRefusal> {
    let spec = StreamSpec::new(SoundSource::Music, 1, 48_000);
    host.open_audio_stream(&spec, Box::new(Clip(mono_pcm.into_iter())))
}
// ANCHOR_END: play_clip

// ANCHOR: voice_frame
#[derive(mod_api::Wire)]
pub struct VoiceFrame {
    pub emitter_id: u32,
    pub encoded_audio: Vec<u8>,
}
// ANCHOR_END: voice_frame

// ANCHOR: open_mic
use mod_api::{MicDevice, MicRef, MicRefusal, MicSpec};

pub fn open_mic(
    host: &dyn mod_api::ClientHost,
    preferred: Option<&MicDevice>,
) -> Result<MicRef, MicRefusal> {
    let mut spec = MicSpec::new(1, 48_000); // 1 or 2 channels, 8000 to 48000 Hz.
    if let Some(device) = preferred {
        spec = spec.with_device(&device.id);
    }
    host.open_microphone(&spec)
}

// On any thread: read whole interleaved signed 16-bit PCM frames without waiting on a tick.
pub fn read_mono<'a>(mic: &MicRef, pcm: &'a mut [i16]) -> &'a [i16] {
    let frames = mic.read(pcm) as usize;
    &pcm[..frames]
}
// ANCHOR_END: open_mic
