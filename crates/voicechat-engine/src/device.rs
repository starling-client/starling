//! The speakers and the microphone (cpal, the default devices), on a thread
//! of their own that holds the streams until told to stop.
use crate::mixer::Mixer;
use crate::opus::FRAME;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};

/// 48 kHz frames rendered from the mixer at a time.
const CHUNK: usize = 480;

#[derive(Clone, Copy, Default)]
struct PlatformHooks {
    ensure_permission: Option<fn() -> bool>,
    on_mic_open: Option<fn()>,
    on_mic_close: Option<fn()>,
}

static HOOKS: Mutex<PlatformHooks> = Mutex::new(PlatformHooks {
    ensure_permission: None,
    on_mic_open: None,
    on_mic_close: None,
});

/// Registers platform-specific microphone permission and lifecycle diagnostic hooks.
pub fn set_platform_hooks(ensure_permission: fn() -> bool, on_mic_open: fn(), on_mic_close: fn()) {
    if let Ok(mut h) = HOOKS.lock() {
        h.ensure_permission = Some(ensure_permission);
        h.on_mic_open = Some(on_mic_open);
        h.on_mic_close = Some(on_mic_close);
    }
}

/// Linear-interpolating rate change from 48 kHz stereo to the device.
pub struct OutResampler {
    step: f64,
    pos: f64,
    buf: VecDeque<[f32; 2]>,
    scratch: Vec<f32>,
}
impl OutResampler {
    pub fn new(device_rate: u32) -> Self {
        Self {
            step: 48_000.0 / device_rate.max(1) as f64,
            pos: 0.0,
            buf: VecDeque::new(),
            scratch: vec![0.0; CHUNK * 2],
        }
    }
    pub fn fill(&mut self, mixer: &Mutex<Mixer>, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        for frame in out.chunks_mut(channels) {
            while self.buf.len() < self.pos as usize + 2 {
                self.scratch.fill(0.0);
                // Non-blocking try_lock so real-time audio callbacks never stall on UDP/UI threads.
                if let Ok(mut m) = mixer.try_lock() {
                    m.render(&mut self.scratch);
                }
                self.buf
                    .extend(self.scratch.chunks(2).map(|c| [c[0], c[1]]));
            }
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            let (a, b) = (self.buf[i], self.buf[i + 1]);
            let v = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            match channels {
                1 => frame[0] = (v[0] + v[1]) * 0.5,
                _ => {
                    frame[0] = v[0];
                    frame[1] = v[1];
                    for s in &mut frame[2..] {
                        *s = 0.0;
                    }
                }
            }
            self.pos += self.step;
            let used = self.pos as usize;
            self.buf.drain(..used.min(self.buf.len()));
            self.pos -= used as f64;
        }
    }
}

/// Mono samples at the device rate in, 960-sample 48 kHz frames out.
pub struct InResampler {
    step: f64,
    pos: f64,
    buf: VecDeque<f32>,
    frame: Vec<f32>,
}
impl InResampler {
    pub fn new(device_rate: u32) -> Self {
        Self {
            step: device_rate.max(1) as f64 / 48_000.0,
            pos: 0.0,
            buf: VecDeque::new(),
            frame: Vec::with_capacity(FRAME),
        }
    }
    pub fn push(&mut self, mono: impl Iterator<Item = f32>, mut emit: impl FnMut(Vec<f32>)) {
        self.buf.extend(mono);
        while self.buf.len() >= self.pos as usize + 2 {
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            self.frame
                .push(self.buf[i] + (self.buf[i + 1] - self.buf[i]) * t);
            if self.frame.len() == FRAME {
                emit(std::mem::replace(
                    &mut self.frame,
                    Vec::with_capacity(FRAME),
                ));
            }
            self.pos += self.step;
            let used = self.pos as usize;
            self.buf.drain(..used.min(self.buf.len()));
            self.pos -= used as f64;
        }
    }
}

fn output_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels() as usize;
    let mut resampler = OutResampler::new(config.sample_rate().0);
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            &config.config(),
            move |data: &mut [T], _| {
                scratch.resize(data.len(), 0.0);
                resampler.fill(&mixer, &mut scratch, channels);
                for (o, s) in data.iter_mut().zip(&scratch) {
                    *o = T::from_sample(*s);
                }
            },
            |e| log::warn!("Voice chat: speaker error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

fn input_stream<T: cpal::SizedSample>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    frames: SyncSender<Vec<f32>>,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let channels = (config.channels() as usize).max(1);
    let mut resampler = InResampler::new(config.sample_rate().0);
    device
        .build_input_stream(
            &config.config(),
            move |data: &[T], _| {
                let mono = data
                    .chunks(channels)
                    .map(|c| c.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32);
                resampler.push(mono, |frame| {
                    let _ = frames.try_send(frame);
                });
            },
            |e| log::warn!("Voice chat: microphone error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Returns whether `name` is a valid microphone (input) device name, filtering out
/// speaker/earpiece/telephony routes that Android's Oboe/AAudio backend may enumerate.
pub fn is_valid_input_device(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !lower.contains("builtinspeaker")
        && !lower.contains("earpiece")
        && !lower.contains(":speaker")
        && !lower.contains("telephony")
        && !lower.contains("fm_tuner")
        && !lower.contains("hdmi")
}

/// Returns whether `name` is a valid speaker/headphone (output) device name, filtering out
/// phone earpiece and microphone routes (`BuiltinEarpiece`, `BuiltinMic`, `Telephony`) that
/// Android's Oboe/AAudio backend enumerates under `output_devices()`.
pub fn is_valid_output_device(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !lower.contains("earpiece")
        && !lower.contains("builtinmic")
        && !lower.contains("mic:")
        && !lower.contains("_mic")
        && !lower.contains("telephony")
        && !lower.contains("fm_tuner")
}

/// The names of the microphones (input devices).
pub fn input_names() -> Vec<String> {
    let mut names: Vec<String> = cpal::default_host()
        .input_devices()
        .map(|d| {
            d.filter_map(|d| d.name().ok())
                .filter(|n| is_valid_input_device(n))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

/// The names of the speakers and headphones (output devices).
pub fn output_names() -> Vec<String> {
    let mut names: Vec<String> = cpal::default_host()
        .output_devices()
        .map(|d| {
            d.filter_map(|d| d.name().ok())
                .filter(|n| is_valid_output_device(n))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

/// The device called `name`, else the default one (preferring `BuiltinSpeaker` over earpiece on Android).
fn pick(name: Option<&str>, input: bool) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let valid = if input {
            is_valid_input_device(name)
        } else {
            is_valid_output_device(name)
        };
        if valid {
            let found = if input {
                host.input_devices().ok()
            } else {
                host.output_devices().ok()
            }
            .and_then(|mut all| all.find(|d| d.name().is_ok_and(|n| n == name)));
            if found.is_some() {
                return found;
            }
            log::warn!("Voice chat: {name} is gone; using the default device");
        } else {
            log::warn!(
                "Voice chat: ignoring invalid {} device '{name}'; using the default device",
                if input { "microphone" } else { "speaker" }
            );
        }
    }
    if input {
        host.default_input_device()
    } else {
        #[cfg(target_os = "android")]
        if let Ok(devices) = host.output_devices() {
            let list: Vec<cpal::Device> = devices.collect();
            let has_external = list.iter().any(|d| {
                d.name().is_ok_and(|n| {
                    let l = n.to_ascii_lowercase();
                    l.contains("bluetooth")
                        || l.contains("a2dp")
                        || l.contains("ble")
                        || l.contains("wired")
                        || l.contains("headset")
                        || l.contains("headphone")
                        || l.contains("usb")
                })
            });
            if !has_external
                && let Some(speaker) = list.into_iter().find(|d| {
                    d.name()
                        .is_ok_and(|n| n.to_ascii_lowercase().contains("builtinspeaker"))
                })
            {
                return Some(speaker);
            }
        }
        host.default_output_device()
    }
}

fn open_output_device(
    device: &cpal::Device,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<cpal::Stream, String> {
    use cpal::SampleFormat as F;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let stream = match config.sample_format() {
        F::F32 => output_stream::<f32>(device, &config, mixer),
        F::I16 => output_stream::<i16>(device, &config, mixer),
        F::I32 => output_stream::<i32>(device, &config, mixer),
        F::U16 => output_stream::<u16>(device, &config, mixer),
        F::F64 => output_stream::<f64>(device, &config, mixer),
        other => Err(format!("unsupported speaker format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    log::info!(
        "Voice chat: speakers {} ({} Hz, {} channels)",
        device.name().unwrap_or_default(),
        config.sample_rate().0,
        config.channels()
    );
    Ok(stream)
}

fn open_output(mixer: Arc<Mutex<Mixer>>, name: Option<&str>) -> Result<cpal::Stream, String> {
    let device = pick(name, false).ok_or("no speakers")?;
    match open_output_device(&device, mixer.clone()) {
        Ok(stream) => Ok(stream),
        Err(first_err) => {
            if let Some(fallback) = cpal::default_host().default_output_device()
                && fallback.name().ok() != device.name().ok()
            {
                log::warn!(
                    "Voice chat: preferred speaker failed ({first_err}); falling back to default"
                );
                return open_output_device(&fallback, mixer);
            }
            Err(first_err)
        }
    }
}

fn open_input(frames: SyncSender<Vec<f32>>, name: Option<&str>) -> Result<cpal::Stream, String> {
    use cpal::SampleFormat as F;
    let check = HOOKS.lock().ok().and_then(|h| h.ensure_permission);
    if let Some(ensure_perm) = check
        && !ensure_perm()
    {
        return Err("microphone permission denied".into());
    }
    let device = pick(name, true).ok_or("no microphone")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let stream = match config.sample_format() {
        F::F32 => input_stream::<f32>(&device, &config, frames),
        F::I16 => input_stream::<i16>(&device, &config, frames),
        F::I32 => input_stream::<i32>(&device, &config, frames),
        F::U16 => input_stream::<u16>(&device, &config, frames),
        F::F64 => input_stream::<f64>(&device, &config, frames),
        other => Err(format!("unsupported microphone format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    log::info!(
        "Voice chat: microphone {} ({} Hz, {} channels)",
        device.name().unwrap_or_default(),
        config.sample_rate().0,
        config.channels()
    );
    Ok(stream)
}

/// What opening the devices left: errors to show.
#[derive(Default, Clone)]
pub struct DeviceErrors {
    pub speakers: Option<String>,
    pub microphone: Option<String>,
}

/// Opens both devices on a thread that keeps them until `stop` receives (or
/// its sender drops). Returns the errors once they're known.
pub fn run(
    mixer: Arc<Mutex<Mixer>>,
    frames: SyncSender<Vec<f32>>,
    stop: Receiver<()>,
    input_name: Option<String>,
    output_name: Option<String>,
) -> DeviceErrors {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("voicechat-devices".into())
        .spawn(move || {
            let output = open_output(mixer, output_name.as_deref());
            let input = open_input(frames, input_name.as_deref());
            let mic_opened = input.is_ok();
            if mic_opened && let Some(on_open) = HOOKS.lock().ok().and_then(|h| h.on_mic_open) {
                on_open();
            }
            let errors = DeviceErrors {
                speakers: output.as_ref().err().cloned(),
                microphone: input.as_ref().err().cloned(),
            };
            let _ = ready_tx.send(errors);
            let _ = stop.recv();
            drop(input);
            if mic_opened && let Some(on_close) = HOOKS.lock().ok().and_then(|h| h.on_mic_close) {
                on_close();
            }
            drop(output);
        })
        .ok();
    ready_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_resampler_keeps_the_rate() {
        let mixer = Mutex::new(Mixer::default());
        for rate in [44_100u32, 48_000, 96_000] {
            let mut r = OutResampler::new(rate);
            let mut out = vec![1.0; rate as usize / 10 * 2];
            r.fill(&mixer, &mut out, 2);
            assert!(out.iter().all(|&s| s == 0.0), "silence renders as silence");
        }
    }

    #[test]
    fn input_resampler_makes_48k_frames() {
        for rate in [16_000u32, 44_100, 48_000, 96_000] {
            let mut r = InResampler::new(rate);
            let mut frames = 0;
            // One second of input.
            r.push((0..rate).map(|i| (i as f32 * 0.01).sin()), |f| {
                assert_eq!(f.len(), FRAME);
                frames += 1;
            });
            assert!((49..=50).contains(&frames), "{rate}: {frames}");
        }
    }

    #[test]
    fn android_device_filter_rejects_earpiece_and_wrong_direction() {
        assert!(!is_valid_output_device("SM-N975F:BuiltinEarpiece"));
        assert!(!is_valid_output_device("SM-N975F:BuiltinMic:bottom"));
        assert!(!is_valid_output_device("SM-N975F:Telephony"));
        assert!(is_valid_output_device("SM-N975F:BuiltinSpeaker"));
        assert!(is_valid_output_device("default"));

        assert!(!is_valid_input_device("SM-N975F:BuiltinSpeaker"));
        assert!(!is_valid_input_device("SM-N975F:BuiltinEarpiece"));
        assert!(is_valid_input_device("SM-N975F:BuiltinMic:bottom"));
        assert!(is_valid_input_device("default"));
    }
}
