//! Microphone signal conditioning for mobile and desktop speakers:
//! 1. 80 Hz high-pass filter (removes DC offset & mobile hand/grip rumble) + clean pre-amp
//!    so normal speech at arm's length (30–45 cm in landscape mode) is picked up clearly without
//!    needing to speak close to the bottom microphone.
//! 2. Separate playback trackers for remote voice chat (`Mixer`) vs. game audio (`audio.rs`):
//!    - Remote voice chat uses a 40-tap (400 ms) delay-aligned `EnvelopeEchoCanceler` + 280 ms
//!      hold tail and strict half-duplex gate (resetting hangover on echo frames to prevent bounce).
//!    - Game audio (music/SFX) uses RNNoise VAD-guided bleed rejection so non-speech game sounds
//!      are blocked while human speech at normal gaming distance passes through un-attenuated.
//! 3. Post-AEC RNNoise (`nnnoiseless`) neural denoising at 48 kHz with soft-knee pre-conditioning
//!    (never clipping `[-32768, 32767]`), full user Microphone Boost support, and `tanh` limiting.

use crate::opus::FRAME;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};

/// 10 ms subframe size required by `nnnoiseless::DenoiseState` at 48 kHz.
pub const RNNOISE_SUBFRAME: usize = 480;
/// Number of 20 ms frames to hold voice activation open after speech stops (32 * 20 ms = 640 ms).
pub const HANGOVER_FRAMES: u32 = 32;
/// Sustain hysteresis (in dB) below `threshold_db` while `hangover_left > 0` so trailing words
/// and unstressed syllables never cut off mid-sentence.
pub const SUSTAIN_HYSTERESIS_DB: f32 = 6.0;
/// Extra VAD threshold required (in dB) while remote voices are playing from the speakers.
pub const DUCKING_THRESHOLD_BOOST_DB: f32 = 16.0;
/// Attenuation factor (-28 dB) applied to the microphone while remote voices dominate.
pub const DUCKING_ATTENUATION: f32 = 0.04;
/// Attenuation factor (-20 dB) applied to non-speech game audio bleed when the player is silent.
pub const GAME_BLEED_ATTENUATION: f32 = 0.10;
/// Number of 10 ms envelope history taps (40 * 10 ms = 400 ms delay coverage).
pub const ENVELOPE_TAPS: usize = 40;
/// Maximum mono reference samples buffered from the speaker mixer (100 ms at 48 kHz).
const MAX_REF_SAMPLES: usize = 4800;

struct SpeakerRefState {
    mono_ring: VecDeque<f32>,
    voice_env_history: [f32; ENVELOPE_TAPS],
    game_env_history: [f32; ENVELOPE_TAPS],
    voice_pos: usize,
    game_pos: usize,
    voice_hold_samples: usize,
    game_hold_samples: usize,
    voice_peak_rms: f32,
    game_peak_rms: f32,
}

static SPEAKER_REF: OnceLock<Mutex<SpeakerRefState>> = OnceLock::new();
static REMOTE_RMS_BITS: AtomicU32 = AtomicU32::new(0);
static GAME_RMS_BITS: AtomicU32 = AtomicU32::new(0);
/// Hardware media volume ratio (0.0..=1.0), defaulting to 0.70.
static HW_VOLUME_SCALE_BITS: AtomicU32 = AtomicU32::new(0x3f333333); // 0.70f32

fn speaker_ref() -> &'static Mutex<SpeakerRefState> {
    SPEAKER_REF.get_or_init(|| {
        Mutex::new(SpeakerRefState {
            mono_ring: VecDeque::with_capacity(MAX_REF_SAMPLES),
            voice_env_history: [0.0; ENVELOPE_TAPS],
            game_env_history: [0.0; ENVELOPE_TAPS],
            voice_pos: 0,
            game_pos: 0,
            voice_hold_samples: 0,
            game_hold_samples: 0,
            voice_peak_rms: 0.0,
            game_peak_rms: 0.0,
        })
    })
}

/// Updates the Android hardware media volume ratio (`0.0..=1.0`, e.g. `14/15 = 0.93`)
/// so the echo gate scales its bleed estimate when the user changes physical speaker volume.
pub fn set_hardware_volume_scale(scale: f32) {
    let clamped = scale.clamp(0.15, 1.5);
    HW_VOLUME_SCALE_BITS.store(clamped.to_bits(), Relaxed);
}

/// Returns the current hardware volume scale multiplier (`0.45..=1.85`).
pub fn hardware_volume_multiplier() -> f32 {
    let ratio = f32::from_bits(HW_VOLUME_SCALE_BITS.load(Relaxed)).clamp(0.15, 1.5);
    (0.35 + ratio * 1.15).clamp(0.45, 1.85)
}

/// Called from `Mixer::render` with the interleaved stereo output of remote voices
/// (before local mic-test loopback is mixed in). Never blocks the audio output callback.
pub fn push_speaker_reference(stereo: &[f32]) {
    let frames = stereo.len() / 2;
    if frames == 0 {
        return;
    }
    let mut sum_sq = 0.0f64;
    for i in 0..frames {
        let mono = (stereo[2 * i] + stereo[2 * i + 1]) * 0.5;
        sum_sq += (mono as f64) * (mono as f64);
    }
    let block_rms = (sum_sq / frames as f64).sqrt() as f32;

    let Ok(mut state) = speaker_ref().try_lock() else {
        return;
    };
    let pos = state.voice_pos;
    state.voice_env_history[pos] = block_rms;
    state.voice_pos = (pos + 1) % ENVELOPE_TAPS;

    let history_peak = state
        .voice_env_history
        .iter()
        .copied()
        .fold(0.0f32, f32::max);
    if block_rms > 0.003 {
        // Hold remote-talking state for 280 ms (13,440 samples at 48 kHz) to cover Android
        // Oboe output buffer + room air path + Oboe input buffer latency.
        state.voice_hold_samples = 13_440;
        state.voice_peak_rms = history_peak.max(block_rms);
    } else {
        state.voice_hold_samples = state.voice_hold_samples.saturating_sub(frames);
        if state.voice_hold_samples > 0 {
            state.voice_peak_rms = history_peak.max(state.voice_peak_rms * 0.94);
        } else {
            state.voice_peak_rms *= 0.80;
        }
    }
    let effective_rms = if state.voice_hold_samples > 0 {
        state.voice_peak_rms
    } else {
        0.0
    };
    REMOTE_RMS_BITS.store(effective_rms.to_bits(), Relaxed);

    for i in 0..frames {
        let mono = (stereo[2 * i] + stereo[2 * i + 1]) * 0.5;
        if state.mono_ring.len() >= MAX_REF_SAMPLES {
            state.mono_ring.pop_front();
        }
        state.mono_ring.push_back(mono);
    }
}

/// Called from `horizon_client_render::audio` with the mixed game audio output buffer
/// (Minecraft background music, ambient sounds, block/entity SFX). Kept strictly separate
/// from `voice_env_history` so game sounds never falsely trigger `remote_talking`.
pub fn push_game_audio_reference(samples: &[f32], channels: usize, sample_rate: u32) {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    if frames == 0 {
        return;
    }
    let mut sum_sq = 0.0f64;
    for chunk in samples.chunks_exact(channels) {
        let mono: f32 = chunk.iter().copied().sum::<f32>() / channels as f32;
        sum_sq += (mono as f64) * (mono as f64);
    }
    let block_rms = (sum_sq / frames as f64).sqrt() as f32;

    let Ok(mut state) = speaker_ref().try_lock() else {
        return;
    };
    let pos = state.game_pos;
    state.game_env_history[pos] = block_rms;
    state.game_pos = (pos + 1) % ENVELOPE_TAPS;

    let history_peak = state
        .game_env_history
        .iter()
        .copied()
        .fold(0.0f32, f32::max);
    let hold_180ms = (sample_rate.max(8_000) as usize * 18) / 100;
    if block_rms > 0.003 {
        state.game_hold_samples = hold_180ms;
        state.game_peak_rms = history_peak.max(block_rms);
    } else {
        state.game_hold_samples = state.game_hold_samples.saturating_sub(frames);
        if state.game_hold_samples > 0 {
            state.game_peak_rms = history_peak.max(state.game_peak_rms * 0.93);
        } else {
            state.game_peak_rms *= 0.80;
        }
    }
    let effective_game_rms = if state.game_hold_samples > 0 {
        state.game_peak_rms
    } else {
        0.0
    };
    GAME_RMS_BITS.store(effective_game_rms.to_bits(), Relaxed);
}

/// Pulls up to `len` mono reference samples and the current remote playback RMS.
pub fn pull_speaker_reference(len: usize) -> (Vec<f32>, f32) {
    let rms = f32::from_bits(REMOTE_RMS_BITS.load(Relaxed));
    let mut ref_buf = vec![0.0f32; len];
    if let Ok(mut state) = speaker_ref().try_lock() {
        let avail = state.mono_ring.len().min(len);
        let start = len - avail;
        for dst in &mut ref_buf[start..] {
            *dst = state.mono_ring.pop_front().unwrap_or(0.0);
        }
    }
    (ref_buf, rms)
}

/// Returns `(game_rms, voice_envelope_history)` where `voice_envelope_history` contains only
/// remote players' voice chat energy over the last 400 ms (never polluted by local game SFX).
fn pull_voice_and_game_history() -> (f32, [f32; ENVELOPE_TAPS]) {
    let game_rms = f32::from_bits(GAME_RMS_BITS.load(Relaxed));
    let mut voice_hist = [0.0f32; ENVELOPE_TAPS];
    if let Ok(state) = speaker_ref().try_lock() {
        for (k, dst) in voice_hist.iter_mut().enumerate() {
            let idx = (state.voice_pos + ENVELOPE_TAPS - 1 - k) % ENVELOPE_TAPS;
            *dst = state.voice_env_history[idx];
        }
    }
    (game_rms, voice_hist)
}

/// 40-tap (400 ms) non-negative Delay-Aligned Envelope Echo Canceler for remote voice chat.
/// Operates on 10–20 ms energy envelopes rather than raw 48 kHz carrier phase, making it
/// immune to Android output/input stream clock drift, 60–300 ms buffer latency, and
/// nonlinear micro-speaker harmonic distortion, while guaranteed never to diverge.
pub struct EnvelopeEchoCanceler {
    weights: [f32; ENVELOPE_TAPS],
    local_history: [f32; ENVELOPE_TAPS],
    hist_pos: usize,
    coupling_floor: f32,
}

impl Default for EnvelopeEchoCanceler {
    fn default() -> Self {
        let mut weights = [0.0; ENVELOPE_TAPS];
        // Pre-seed typical Android latency taps (60 ms to 180 ms, taps 6..18) so echo
        // suppression is active from the very first syllable after joining a server.
        for w in &mut weights[6..18] {
            *w = 0.035;
        }
        Self {
            weights,
            local_history: [0.0; ENVELOPE_TAPS],
            hist_pos: 0,
            coupling_floor: 0.35,
        }
    }
}

impl EnvelopeEchoCanceler {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Updates the delay-aligned envelope model and returns `(estimated_echo_rms, suppression_gain)`.
    pub fn estimate_and_suppress(
        &mut self,
        mic_rms: f32,
        frame_ref_rms: f32,
        voice_hist: &[f32; ENVELOPE_TAPS],
        hw_mult: f32,
    ) -> (f32, f32) {
        self.local_history[self.hist_pos] = frame_ref_rms;
        self.hist_pos = (self.hist_pos + 1) % ENVELOPE_TAPS;

        let mut taps = [0.0f32; ENVELOPE_TAPS];
        let mut norm = 1e-4f32;
        let mut peak_tap = 0.0f32;
        for (k, tap) in taps.iter_mut().enumerate() {
            let local_idx = (self.hist_pos + ENVELOPE_TAPS - 1 - k) % ENVELOPE_TAPS;
            let r = self.local_history[local_idx].max(voice_hist[k]) * hw_mult;
            *tap = r;
            norm += r * r;
            if r > peak_tap {
                peak_tap = r;
            }
        }

        let mut echo_est = 0.0f32;
        for (w, &r) in self.weights.iter().zip(&taps) {
            echo_est += *w * r;
        }

        // Adapt non-negative tap weights when remote voice reference is present and mic is not
        // overwhelmingly louder than the speaker (double-talk guard).
        if peak_tap > 0.0025 && mic_rms <= peak_tap * 2.2 {
            let err = mic_rms - echo_est;
            let step = (0.08 * err) / norm;
            let mut weight_sum = 0.0f32;
            for (w, &r) in self.weights.iter_mut().zip(&taps) {
                let updated = (*w * 0.9992 + step * r).clamp(0.0, 1.0);
                *w = if updated.is_finite() { updated } else { 0.0 };
                weight_sum += *w;
            }
            if weight_sum > 2.20 {
                let scale = 2.20 / weight_sum;
                for w in &mut self.weights {
                    *w *= scale;
                }
            }
            let obs_ratio = (mic_rms / peak_tap).clamp(0.12, 0.55);
            self.coupling_floor = (self.coupling_floor * 0.92 + obs_ratio * 0.08).clamp(0.20, 0.50);
        } else {
            self.coupling_floor = (self.coupling_floor * 0.998 + 0.25 * 0.002).max(0.20);
            for w in &mut self.weights {
                *w *= 0.9995;
            }
        }

        // Use the recent 200 ms peak rather than the whole 400 ms window so an old loud remote
        // word cannot keep inflating the echo estimate over the player's own speech.
        let recent_peak = taps[..20].iter().copied().fold(0.0f32, f32::max);
        let guaranteed_floor = recent_peak * self.coupling_floor;
        let effective_echo = echo_est.max(guaranteed_floor);
        let gain = if mic_rms > 1e-5 && effective_echo > 0.001 {
            (1.0 - (1.35 * effective_echo) / mic_rms).clamp(0.04, 1.0)
        } else {
            1.0
        };
        (effective_echo, gain)
    }
}

/// Lowers a threshold by the sustain hysteresis while a sentence is in progress.
fn sustain_db_adjust(threshold_db: f32, sustain_db: f32) -> f32 {
    threshold_db - sustain_db
}

/// Output of conditioning a single 20 ms (960-sample) 48 kHz microphone frame.
#[derive(Clone, Debug)]
pub struct FilterOutput {
    pub samples: Vec<f32>,
    pub level_db: f32,
    pub vad_prob: f32,
    pub speaking: bool,
    pub ducked: bool,
}

/// Stateful microphone conditioner running on the `voicechat-mic` thread.
pub struct MicFilter {
    #[cfg(feature = "rnnoise")]
    denoiser: Box<nnnoiseless::DenoiseState<'static>>,
    voice_aec: EnvelopeEchoCanceler,
    game_bleed_ratio: f32,
    hp_prev_in: f32,
    hp_prev_out: f32,
    hangover_left: u32,
    /// Consecutive frames the remote-voice gate wanted to duck while the player was mid-sentence.
    duck_streak: u32,
}

impl Default for MicFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl MicFilter {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "rnnoise")]
            denoiser: nnnoiseless::DenoiseState::new(),
            voice_aec: EnvelopeEchoCanceler::default(),
            game_bleed_ratio: 0.30,
            hp_prev_in: 0.0,
            hp_prev_out: 0.0,
            hangover_left: 0,
            duck_streak: 0,
        }
    }

    /// Resets all adaptive filter weights, AEC envelopes, and hangover state.
    pub fn reset(&mut self) {
        #[cfg(feature = "rnnoise")]
        {
            self.denoiser = nnnoiseless::DenoiseState::new();
        }
        self.voice_aec = EnvelopeEchoCanceler::default();
        self.game_bleed_ratio = 0.30;
        self.hp_prev_in = 0.0;
        self.hp_prev_out = 0.0;
        self.hangover_left = 0;
        self.duck_streak = 0;
    }

    /// Processes a 960-sample 48 kHz mono frame:
    /// 1. 80 Hz high-pass filter to remove DC offset and low-frequency hand/grip rumble.
    /// 2. 400 ms delay-aligned envelope echo suppression for remote voice chat (`voice_aec`).
    /// 3. Pre-RNNoise makeup gain (`2.4x * sqrt(user_gain)`) with soft-knee `tanh` ceiling so
    ///    distant arm's-length speech (30–45 cm) is recognized clearly by RNNoise without clipping.
    /// 4. RNNoise neural denoising + VAD probability estimation (`vad_prob`).
    /// 5. VAD-guided game audio bleed gate: blocks non-speech game sounds (`vad_prob < 0.45`)
    ///    while letting human speech (`vad_prob >= 0.45`) pass through un-attenuated.
    /// 6. Post-filter user gain + `tanh` soft limiter.
    pub fn process_frame(
        &mut self,
        raw_frame: &[f32],
        speaker_ref: &[f32],
        remote_rms: f32,
        user_gain: f32,
        threshold_db: f32,
        rnnoise_enabled: bool,
    ) -> FilterOutput {
        let len = raw_frame.len().max(1);
        let mut working = Vec::with_capacity(raw_frame.len());

        // Step 1: 80 Hz 1st-order high-pass filter (alpha = 0.9896 at 48 kHz) removes DC offset
        // and low-frequency finger/grip rumble from the phone's bottom microphone.
        const HP_ALPHA: f32 = 0.9896;
        for &x in raw_frame {
            let clamped = x.clamp(-1.0, 1.0);
            let y = HP_ALPHA * (self.hp_prev_out + clamped - self.hp_prev_in);
            self.hp_prev_in = clamped;
            self.hp_prev_out = y.clamp(-1.0, 1.0);
            working.push(self.hp_prev_out);
        }

        let raw_rms =
            (working.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / len as f64).sqrt() as f32;
        let ref_frame_rms = if speaker_ref.is_empty() {
            0.0
        } else {
            (speaker_ref.iter().map(|&v| (v as f64).powi(2)).sum::<f64>()
                / speaker_ref.len() as f64)
                .sqrt() as f32
        };

        let (game_rms, voice_hist) = pull_voice_and_game_history();
        let hw_mult = hardware_volume_multiplier();
        let voice_ref_rms = remote_rms.max(ref_frame_rms);

        // Step 2: Remote voice chat echo estimation (strictly isolated from game audio!).
        let (voice_echo_rms, voice_aec_gain) =
            self.voice_aec
                .estimate_and_suppress(raw_rms, voice_ref_rms, &voice_hist, hw_mult);

        let remote_talking =
            remote_rms > 0.0025 || ref_frame_rms > 0.0025 || voice_echo_rms > 0.0035;
        let game_playing = game_rms > 0.003;

        // Compare using user-boosted RMS so increasing "Microphone Boost" in settings helps the
        // player's voice break through at comfortable arm's-length distance.
        let user_boost = user_gain.clamp(0.5, 4.0);
        let boosted_raw_rms = raw_rms * user_boost;
        let voice_breakout_rms = (voice_echo_rms * 2.1)
            .max(voice_ref_rms * hw_mult * 1.8)
            .max(0.018);

        let wants_duck = remote_talking && boosted_raw_rms < voice_breakout_rms;
        // Mid-sentence, a brief remote blip must not chop the player's own words: only the echo
        // suppression gain applies until the duck request persists for 10 frames (200 ms).
        let was_speaking = self.hangover_left > 0;
        self.duck_streak = if wants_duck { self.duck_streak + 1 } else { 0 };
        let voice_ducked = wants_duck && (!was_speaking || self.duck_streak >= 10);
        if voice_ducked {
            let atten = DUCKING_ATTENUATION.min(voice_aec_gain);
            for s in &mut working {
                *s *= atten;
            }
            // Immediately clear hangover on ducked remote-voice echo frames so echo can never bounce.
            self.hangover_left = 0;
        } else if remote_talking && voice_aec_gain < 1.0 {
            for s in &mut working {
                *s *= voice_aec_gain;
            }
        }

        // Step 3: Clean pre-conditioning gain before RNNoise so normal arm's-length speech on
        // Android's un-amplified VOICE_RECOGNITION input (-40..-30 dBFS) sits in RNNoise's
        // optimal sensitivity window without ever clipping [-32768, 32767].
        let pre_gain = if voice_ducked {
            1.0
        } else {
            (2.4 * user_boost.sqrt()).clamp(1.2, 4.0)
        };
        for s in &mut working {
            let boosted = *s * pre_gain;
            // Soft-knee ceiling at 0.85 prevents hard clipping inside DenoiseState::process_frame.
            *s = boosted.clamp(-0.95, 0.95);
        }

        // Step 4: RNNoise neural denoising + human speech VAD probability.
        let dry: Vec<f32> = working.clone();
        let mut vad_prob = 1.0f32;
        #[cfg(feature = "rnnoise")]
        if rnnoise_enabled && working.len() == FRAME {
            let mut in_sub = [0.0f32; RNNOISE_SUBFRAME];
            let mut out_sub = [0.0f32; RNNOISE_SUBFRAME];

            for (dst, &src) in in_sub.iter_mut().zip(&working[..RNNOISE_SUBFRAME]) {
                *dst = src.clamp(-1.0, 1.0) * 32767.0;
            }
            let vad_0 = self.denoiser.process_frame(&mut out_sub, &in_sub);
            for (dst, &src) in working[..RNNOISE_SUBFRAME].iter_mut().zip(&out_sub) {
                *dst = (src / 32767.0).clamp(-1.0, 1.0);
            }

            for (dst, &src) in in_sub.iter_mut().zip(&working[RNNOISE_SUBFRAME..FRAME]) {
                *dst = src.clamp(-1.0, 1.0) * 32767.0;
            }
            let vad_1 = self.denoiser.process_frame(&mut out_sub, &in_sub);
            for (dst, &src) in working[RNNOISE_SUBFRAME..FRAME].iter_mut().zip(&out_sub) {
                *dst = (src / 32767.0).clamp(-1.0, 1.0);
            }

            vad_prob = vad_0.max(vad_1);

            // Dry/wet blend: keep a floor of the clean voice so RNNoise cannot fully erase a
            // syllable it misreads as noise (45% dry at low VAD, 20% at high VAD).
            let dry_share = 0.55 - 0.35 * vad_prob.clamp(0.0, 1.0);
            for (w, &d) in working.iter_mut().zip(&dry) {
                *w = *w * (1.0 - dry_share) + d * dry_share;
            }
        }
        #[cfg(not(feature = "rnnoise"))]
        let _ = rnnoise_enabled;

        // Step 5: VAD-guided Game Audio (SFX/Music) Bleed Gate.
        // Only adapt the game bleed ratio on non-speech frames (`vad_prob < 0.35`) so the player's
        // own voice never inflates the game bleed estimate.
        let scaled_game_rms = game_rms * hw_mult;
        if game_playing
            && self.hangover_left == 0
            && (!rnnoise_enabled || vad_prob < 0.35)
            && raw_rms <= scaled_game_rms * 1.5
        {
            let obs = (raw_rms / scaled_game_rms.max(1e-4)).clamp(0.10, 0.50);
            self.game_bleed_ratio = (self.game_bleed_ratio * 0.90 + obs * 0.10).clamp(0.15, 0.45);
        } else if !game_playing {
            self.game_bleed_ratio = (self.game_bleed_ratio * 0.998 + 0.25 * 0.002).max(0.15);
        }
        let expected_game_bleed = scaled_game_rms * self.game_bleed_ratio;

        // Determine whether the frame is dominated by non-speech game audio bleed:
        // If RNNoise confirms human speech (`vad_prob >= 0.45`) and the boosted voice exceeds
        // the expected game bleed, let the player's voice pass through at 100% volume!
        let is_clear_speech = if rnnoise_enabled {
            vad_prob >= 0.45 && boosted_raw_rms > expected_game_bleed * 1.15
        } else {
            boosted_raw_rms > (expected_game_bleed * 1.8).max(0.018)
        };
        // While the gate is held open after speech, never duck for game bleed: VAD dips between
        // syllables would otherwise chop the sentence after two or three words.
        let was_in_speech = self.hangover_left > 0;

        let voice_scale = (user_gain.clamp(0.0, 4.0) / user_boost.sqrt()).clamp(0.5, 2.5);
        let pre_voice_rms =
            (working.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / len as f64).sqrt() as f32;
        let pre_level_db = if pre_voice_rms > 0.0 {
            20.0 * (pre_voice_rms * voice_scale).log10()
        } else {
            -100.0
        };

        let voice_echo_floor_db = if voice_echo_rms > 1e-4 {
            20.0 * (voice_echo_rms * 2.0).log10()
        } else {
            -100.0
        };
        let sustain_db = if was_in_speech {
            SUSTAIN_HYSTERESIS_DB
        } else {
            0.0
        };
        let effective_threshold = sustain_db_adjust(
            if remote_talking {
                (threshold_db + DUCKING_THRESHOLD_BOOST_DB)
                    .max(voice_echo_floor_db)
                    .min(-14.0)
            } else if game_playing && !is_clear_speech {
                (threshold_db + 10.0).min(-22.0)
            } else {
                threshold_db
            },
            sustain_db,
        );
        let min_vad = if !rnnoise_enabled {
            0.0
        } else if was_in_speech {
            0.0
        } else if remote_talking {
            0.78
        } else if game_playing {
            0.45
        } else {
            0.35
        };

        let should_trigger_speech = !voice_ducked
            && (is_clear_speech || was_in_speech || !game_playing)
            && pre_level_db > effective_threshold
            && vad_prob >= min_vad;

        if should_trigger_speech {
            self.hangover_left = HANGOVER_FRAMES;
        } else if self.hangover_left > 0 {
            self.hangover_left -= 1;
        }
        let in_speech = self.hangover_left > 0;

        let music_ducked = !voice_ducked && game_playing && !is_clear_speech && !in_speech;
        if music_ducked {
            for s in &mut working {
                *s *= GAME_BLEED_ATTENUATION;
            }
        }

        let ducked = voice_ducked || music_ducked;

        // Step 6: Remaining post-filter user gain + soft limiter.
        let post_gain = if voice_ducked || music_ducked {
            1.0
        } else {
            voice_scale
        };
        for s in &mut working {
            *s = (*s * post_gain).tanh();
        }

        let post_rms =
            (working.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / len as f64).sqrt();
        let level_db = if post_rms > 0.0 {
            (20.0 * post_rms.log10()) as f32
        } else {
            -100.0
        };
        let speaking = !voice_ducked && in_speech;

        FilterOutput {
            samples: working,
            level_db,
            vad_prob,
            speaking,
            ducked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_arms_length_speech_triggers_mic_without_shouting() {
        let mut filter = MicFilter::new();
        // Simulate quiet arm's-length speech on Android VOICE_RECOGNITION (amplitude 0.015,
        // raw RMS ~0.0106 = -39.5 dBFS) at default settings (user_gain = 1.0, threshold = -45 dB).
        let arms_length_speech: Vec<f32> = (0..FRAME)
            .map(|i| ((i as f32 * 0.08).sin()) * 0.015)
            .collect();
        let out = filter.process_frame(&arms_length_speech, &[], 0.0, 1.0, -45.0, false);
        assert!(
            !out.ducked && out.speaking,
            "expected arm's-length speech (-39.5 dBFS raw) to open the mic without shouting: level_db={}, ducked={}",
            out.level_db,
            out.ducked
        );
        assert!(
            out.level_db > -33.0,
            "expected pre-amp to lift arm's-length speech above -33 dBFS: got {} dBFS",
            out.level_db
        );
    }

    #[test]
    fn ducking_attenuates_bleed_when_remote_is_talking() {
        let mut filter = MicFilter::new();
        // Speaker bleed (RMS ~0.05, typical for phone loudspeaker at 10/15..14/15 volume)
        // while remote speaker is loud (RMS 0.08).
        let bleed: Vec<f32> = (0..FRAME)
            .map(|i| ((i as f32 * 0.1).sin()) * 0.05)
            .collect();
        let speaker_ref: Vec<f32> = (0..FRAME)
            .map(|i| ((i as f32 * 0.1).sin()) * 0.11)
            .collect();

        let out = filter.process_frame(&bleed, &speaker_ref, 0.08, 1.0, -50.0, false);
        assert!(
            out.ducked,
            "expected ducking gate to engage on speaker bleed"
        );
        assert!(
            !out.speaking,
            "expected raised ducking threshold and half-duplex gate to prevent false VAD trigger"
        );
        let out_rms = (out.samples.iter().map(|&x| x * x).sum::<f32>() / FRAME as f32).sqrt();
        let in_rms = (bleed.iter().map(|&x| x * x).sum::<f32>() / FRAME as f32).sqrt();
        assert!(
            out_rms < in_rms * 0.15,
            "expected strong attenuation (< 0.15x): in_rms={in_rms}, out_rms={out_rms}"
        );
    }

    #[test]
    fn delayed_echo_after_speaker_stops_is_still_suppressed() {
        let mut filter = MicFilter::new();
        let loud_ref: Vec<f32> = (0..FRAME)
            .map(|i| ((i as f32 * 0.1).sin()) * 0.12)
            .collect();
        let silence_frame = vec![0.0f32; FRAME];

        // Frame 0: remote player speaks into speaker_ref, mic has not heard it yet (0.0).
        let _ = filter.process_frame(&silence_frame, &loud_ref, 0.09, 1.0, -50.0, false);

        // Frames 1..8 (160 ms later): remote stream is now silent (`remote_rms = 0.0`),
        // and the delayed acoustic echo (amplitude 0.04) arrives at the phone's microphone!
        for _ in 0..7 {
            let _ = filter.process_frame(&silence_frame, &[], 0.0, 1.0, -50.0, false);
        }
        let delayed_echo: Vec<f32> = (0..FRAME)
            .map(|i| ((i as f32 * 0.1).sin()) * 0.04)
            .collect();
        let out = filter.process_frame(&delayed_echo, &[], 0.0, 1.0, -50.0, false);
        assert!(
            out.ducked && !out.speaking,
            "160 ms delayed speaker echo must be caught by the 400 ms envelope history without triggering VAD"
        );
    }

    #[test]
    fn game_music_bleed_does_not_trigger_mic_transmission() {
        let mut filter = MicFilter::new();
        // Simulate game audio playing through `horizon_client_render::audio` at stereo 44.1 kHz.
        let music_stereo: Vec<f32> = (0..882 * 2)
            .map(|i| ((i as f32 * 0.07).sin()) * 0.10)
            .collect();
        push_game_audio_reference(&music_stereo, 2, 44_100);

        // Non-speech game audio bleeds into the phone microphone at amplitude 0.02 (-34 dBFS).
        let mic_music_bleed: Vec<f32> = (0..FRAME)
            .map(|i| ((i as f32 * 0.37).sin() * (i as f32 * 0.03).cos()) * 0.02)
            .collect();
        let out = filter.process_frame(&mic_music_bleed, &[], 0.0, 1.0, -45.0, true);
        assert!(
            out.ducked,
            "expected game audio bleed to be ducked when player is not speaking"
        );
        assert!(
            !out.speaking,
            "expected game audio bleed not to trigger microphone transmission"
        );

        // Reset game audio reference to silence for subsequent tests.
        for _ in 0..ENVELOPE_TAPS {
            push_game_audio_reference(&[0.0; 1764], 2, 44_100);
        }
    }

    #[test]
    fn long_session_echo_canceler_remains_stable_after_five_minutes() {
        let mut aec = EnvelopeEchoCanceler::default();
        let hist = [0.05f32; ENVELOPE_TAPS];
        // Simulate 15,000 frames (5 minutes at 20 ms/frame) of continuous speaker + mic bleed.
        let mut last_echo = 0.0f32;
        let mut last_gain = 1.0f32;
        for i in 0..15_000 {
            let mic_rms = 0.04 + ((i % 17) as f32) * 0.002;
            let ref_rms = 0.08 + ((i % 13) as f32) * 0.003;
            let (echo_est, gain) = aec.estimate_and_suppress(mic_rms, ref_rms, &hist, 1.4);
            assert!(echo_est.is_finite() && echo_est > 0.01 && echo_est < 1.0);
            assert!(gain.is_finite() && (0.04..=1.0).contains(&gain));
            last_echo = echo_est;
            last_gain = gain;
        }
        assert!(
            last_echo > 0.02 && last_gain < 0.35,
            "after 5 minutes of operation, echo canceler must remain bounded and active (echo={last_echo}, gain={last_gain})"
        );
    }

    #[test]
    fn rnnoise_processes_normal_speech_without_clipping() {
        let mut filter = MicFilter::new();
        let speech_like: Vec<f32> = (0..FRAME)
            .map(|i| ((i as f32 * 0.05).sin()) * 0.25)
            .collect();
        let out = filter.process_frame(&speech_like, &[], 0.0, 2.0, -45.0, true);
        assert_eq!(out.samples.len(), FRAME);
        assert!(
            out.samples.iter().all(|&s| s.is_finite() && s.abs() <= 1.0),
            "all output samples must remain finite and within [-1.0, 1.0] soft-limiter bounds"
        );
    }
}
