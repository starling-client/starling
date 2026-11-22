//! Client-side microphone capture: bounded drop-oldest lock-free PCM ring, pure-Rust
//! channel/sample-rate converter, distinct endpoint resolution, and a worker that owns and
//! releases the platform stream.

#[cfg(all(
    feature = "audio",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    )
))]
mod alsa_capture;
#[cfg(all(
    feature = "audio",
    not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    ))
))]
mod cpal_capture;
#[cfg(all(
    feature = "audio",
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    )
))]
use alsa_capture::AlsaBackend as NativeBackend;
#[cfg(all(
    feature = "audio",
    not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd"
    ))
))]
use cpal_capture::CpalBackend as NativeBackend;
#[cfg(any(feature = "audio", test))]
use std::collections::HashMap as ActiveMap;
use std::sync::Arc;
#[cfg(any(feature = "audio", test))]
use std::sync::{
    Mutex, TryLockError,
    atomic::{AtomicBool, AtomicI16, AtomicU64, Ordering, fence},
    mpsc,
};
#[cfg(any(feature = "audio", test))]
use std::thread;

#[cfg(any(feature = "audio", test))]
mod stop;

/// Validates a requested microphone capture specification.
pub(super) fn validate_spec(spec: &mod_api::MicSpec) -> Result<(), mod_api::MicRefusal> {
    if spec.channels != 1 && spec.channels != 2 {
        return Err(mod_api::MicRefusal::BadSpec(
            mod_api::MicSpecFault::Channels,
        ));
    }
    if !(mod_api::MIC_RATE_MIN..=mod_api::MIC_RATE_MAX).contains(&spec.rate) {
        return Err(mod_api::MicRefusal::BadSpec(mod_api::MicSpecFault::Rate));
    }
    Ok(())
}

/// Bounded drop-oldest PCM capture ring.
///
/// Single producer (`push_frames`, called from the OS capture callback: wait-free, zero
/// allocation, never locks) and any consumer thread (`read_frames`, nonblocking via
/// `scratch.try_lock()` and seqlock validation against concurrent overwrites so unreturned
/// samples never touch the caller's slice).
#[cfg(any(feature = "audio", test))]
struct CaptureRing {
    slots: Box<[AtomicI16]>,
    channels: usize,
    cap_frames: usize,
    head: AtomicU64,
    writing_tail: AtomicU64,
    committed_tail: AtomicU64,
    dropped: AtomicU64,
    scratch: Mutex<Box<[i16]>>,
}

#[cfg(any(feature = "audio", test))]
impl CaptureRing {
    fn new(rate: u32, channels: u8) -> Self {
        let cap_frames = ((rate as usize) * (mod_api::MIC_RING_MAX_MS as usize))
            .div_ceil(1_000)
            .max(1);
        Self::with_capacity_frames(cap_frames, usize::from(channels))
    }

    fn with_capacity_frames(cap_frames: usize, channels: usize) -> Self {
        let cap_frames = cap_frames.max(1);
        let channels = channels.max(1);
        let total_samples = cap_frames * channels;
        let slots = (0..total_samples)
            .map(|_| AtomicI16::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let scratch = vec![0_i16; total_samples].into_boxed_slice();
        Self {
            slots,
            channels,
            cap_frames,
            head: AtomicU64::new(0),
            writing_tail: AtomicU64::new(0),
            committed_tail: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            scratch: Mutex::new(scratch),
        }
    }

    /// Wait-free single-producer push of whole interleaved PCM frames.
    /// Drops the oldest frames whenever the ring overflows.
    fn push_frames(&self, pcm: &[i16]) {
        let ch = self.channels;
        let frames_in = pcm.len() / ch;
        if frames_in == 0 {
            return;
        }
        let (slice, n) = if frames_in > self.cap_frames {
            let skip = frames_in - self.cap_frames;
            self.dropped.fetch_add(skip as u64, Ordering::Relaxed);
            (&pcm[skip * ch..frames_in * ch], self.cap_frames)
        } else {
            (&pcm[..frames_in * ch], frames_in)
        };

        let t = self.committed_tail.load(Ordering::Relaxed);
        let new_tail = t + n as u64;
        let min_head = new_tail.saturating_sub(self.cap_frames as u64);

        // Announce write window before touching slots so concurrent readers detect overwrites.
        self.writing_tail.store(new_tail, Ordering::SeqCst);
        fence(Ordering::SeqCst);

        // Advance head past overwritten frames and account for dropped frames.
        let old_head = self.head.fetch_max(min_head, Ordering::SeqCst);
        if min_head > old_head {
            self.dropped
                .fetch_add(min_head - old_head, Ordering::Relaxed);
        }

        let cap = self.cap_frames as u64;
        for i in 0..n {
            let slot_base = ((t + i as u64) % cap) as usize * ch;
            let src_base = i * ch;
            for c in 0..ch {
                self.slots[slot_base + c].store(slice[src_base + c], Ordering::Relaxed);
            }
        }

        self.committed_tail.store(new_tail, Ordering::SeqCst);
    }

    /// Reads whole interleaved PCM frames into `out` from any thread without blocking on the
    /// producer or on another concurrent reader. Only validated whole frames are copied into
    /// `out`; any suffix beyond `returned * channels` (and the entire slice on contention or
    /// empty read) is left untouched.
    fn read_frames(&self, out: &mut [i16]) -> u32 {
        self.read_frames_inner(out, |_| {})
    }

    fn read_frames_inner(&self, out: &mut [i16], mut before_validate: impl FnMut(usize)) -> u32 {
        let ch = self.channels;
        let max_frames = out.len() / ch;
        if max_frames == 0 {
            return 0;
        }
        let mut scratch = match self.scratch.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return 0,
            Err(TryLockError::Poisoned(err)) => err.into_inner(),
        };
        let cap = self.cap_frames as u64;

        for attempt in 0..8 {
            let h = self.head.load(Ordering::SeqCst);
            let t = self.committed_tail.load(Ordering::SeqCst);
            if t <= h {
                return 0;
            }
            if t - h > cap {
                continue;
            }
            let avail = (t - h).min(max_frames as u64) as usize;
            let sample_count = avail * ch;
            for i in 0..avail {
                let slot_base = ((h + i as u64) % cap) as usize * ch;
                let dst_base = i * ch;
                for c in 0..ch {
                    scratch[dst_base + c] = self.slots[slot_base + c].load(Ordering::Relaxed);
                }
            }
            before_validate(attempt);
            fence(Ordering::SeqCst);
            let wt = self.writing_tail.load(Ordering::SeqCst);
            if wt.saturating_sub(cap) <= h
                && self
                    .head
                    .compare_exchange(h, h + avail as u64, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                out[..sample_count].copy_from_slice(&scratch[..sample_count]);
                return avail as u32;
            }
        }
        0
    }

    fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(any(feature = "audio", test))]
#[inline]
fn float_to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[cfg(any(feature = "audio", test))]
#[inline]
fn i16_to_f32(s: i16) -> f32 {
    f32::from(s) / 32768.0
}

/// Pure-Rust channel mapper and linear resampler for incoming OS capture callbacks.
/// Uses a stack scratch buffer and performs zero heap allocations per callback.
#[cfg(any(feature = "audio", test))]
struct InputConverter {
    in_channels: usize,
    out_channels: usize,
    in_rate: u32,
    out_rate: u32,
    prev: [f32; 2],
    has_prev: bool,
    pos: f64,
    step: f64,
}

#[cfg(any(feature = "audio", test))]
impl InputConverter {
    fn new(in_channels: u16, in_rate: u32, out_channels: u8, out_rate: u32) -> Self {
        Self {
            in_channels: usize::from(in_channels),
            out_channels: usize::from(out_channels),
            in_rate,
            out_rate,
            prev: [0.0; 2],
            has_prev: false,
            pos: 0.0,
            step: if out_rate > 0 {
                f64::from(in_rate) / f64::from(out_rate)
            } else {
                1.0
            },
        }
    }

    #[inline]
    fn convert_frame<T: Copy>(&self, frame: &[T], to_f32: &impl Fn(T) -> f32) -> [f32; 2] {
        if self.out_channels == 1 {
            if self.in_channels == 1 {
                [to_f32(frame[0]), 0.0]
            } else {
                let mut sum = 0.0_f32;
                for &s in frame {
                    sum += to_f32(s);
                }
                [sum / self.in_channels as f32, 0.0]
            }
        } else if self.in_channels == 1 {
            let s = to_f32(frame[0]);
            [s, s]
        } else {
            [to_f32(frame[0]), to_f32(frame[1])]
        }
    }

    /// Passes native signed 16-bit PCM straight through unchanged whenever channels and sample
    /// rate already match, avoiding lossy float round-trips on full-scale samples.
    fn push_i16_samples(&mut self, samples: &[i16], ring: &CaptureRing) {
        if self.in_channels == 0
            || self.out_channels == 0
            || self.in_rate == 0
            || self.out_rate == 0
        {
            return;
        }
        let in_frames = samples.len() / self.in_channels;
        if in_frames == 0 {
            return;
        }
        if self.in_rate == self.out_rate && self.in_channels == self.out_channels {
            ring.push_frames(&samples[..in_frames * self.in_channels]);
            return;
        }
        self.push_samples(samples, ring, i16_to_f32);
    }

    fn push_samples<T: Copy>(
        &mut self,
        samples: &[T],
        ring: &CaptureRing,
        to_f32: impl Fn(T) -> f32,
    ) {
        if self.in_channels == 0
            || self.out_channels == 0
            || self.in_rate == 0
            || self.out_rate == 0
        {
            return;
        }
        let in_frames = samples.len() / self.in_channels;
        if in_frames == 0 {
            return;
        }

        let mut scratch = [0_i16; 1024];
        let max_scratch = (scratch.len() / self.out_channels) * self.out_channels;
        let mut len = 0_usize;

        if self.in_rate == self.out_rate {
            if self.in_channels == self.out_channels {
                let total = in_frames * self.in_channels;
                for &s in &samples[..total] {
                    scratch[len] = float_to_i16(to_f32(s));
                    len += 1;
                    if len == max_scratch {
                        ring.push_frames(&scratch[..len]);
                        len = 0;
                    }
                }
            } else {
                for f in 0..in_frames {
                    let frame = &samples[f * self.in_channels..(f + 1) * self.in_channels];
                    let mixed = self.convert_frame(frame, &to_f32);
                    for c in 0..self.out_channels {
                        scratch[len] = float_to_i16(mixed[c]);
                        len += 1;
                    }
                    if len == max_scratch {
                        ring.push_frames(&scratch[..len]);
                        len = 0;
                    }
                }
            }
            if len > 0 {
                ring.push_frames(&scratch[..len]);
            }
            return;
        }

        let n = in_frames as isize;
        loop {
            let i0 = self.pos.floor() as isize;
            if i0 >= n {
                break;
            }
            let frac = (self.pos - i0 as f64) as f32;
            let sample = if i0 < 0 {
                let first = self.convert_frame(&samples[..self.in_channels], &to_f32);
                let a = if self.has_prev { self.prev } else { first };
                [
                    a[0] + (first[0] - a[0]) * frac,
                    a[1] + (first[1] - a[1]) * frac,
                ]
            } else if i0 == n - 1 {
                if self.pos > i0 as f64 {
                    break;
                }
                let idx = i0 as usize;
                self.convert_frame(
                    &samples[idx * self.in_channels..(idx + 1) * self.in_channels],
                    &to_f32,
                )
            } else {
                let idx0 = i0 as usize;
                let idx1 = idx0 + 1;
                let a = self.convert_frame(
                    &samples[idx0 * self.in_channels..(idx0 + 1) * self.in_channels],
                    &to_f32,
                );
                let b = self.convert_frame(
                    &samples[idx1 * self.in_channels..(idx1 + 1) * self.in_channels],
                    &to_f32,
                );
                [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]
            };

            for c in 0..self.out_channels {
                scratch[len] = float_to_i16(sample[c]);
                len += 1;
            }
            if len == max_scratch {
                ring.push_frames(&scratch[..len]);
                len = 0;
            }
            self.pos += self.step;
        }

        if len > 0 {
            ring.push_frames(&scratch[..len]);
        }

        let last = in_frames - 1;
        self.prev = self.convert_frame(
            &samples[last * self.in_channels..(last + 1) * self.in_channels],
            &to_f32,
        );
        self.has_prev = true;
        self.pos -= in_frames as f64;
    }

    #[cfg(test)]
    fn push_f32_samples(&mut self, samples: &[f32], ring: &CaptureRing) {
        self.push_samples(samples, ring, std::convert::identity);
    }

    #[cfg(all(
        feature = "audio",
        not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ))
    ))]
    fn push_cpal_samples<T: cpal::SizedSample>(&mut self, samples: &[T], ring: &CaptureRing)
    where
        f32: cpal::FromSample<T>,
    {
        self.push_samples(samples, ring, <f32 as cpal::Sample>::from_sample);
    }
}

#[cfg(any(
    test,
    all(
        feature = "audio",
        not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ))
    )
))]
mod cpal_config;
#[cfg(any(
    test,
    all(
        feature = "audio",
        not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ))
    )
))]
use cpal_config::*;

#[cfg(any(feature = "audio", test))]
trait BackendDevice: Clone + Send + 'static {
    type InputConfigs;

    fn label(&self) -> Option<String>;
    fn stable_selector(&self) -> Option<String>;
    fn same_endpoint(&self, other: &Self) -> bool;

    /// Excludes output-only endpoints using metadata alone, before OS authorization.
    /// Backends that cannot ask for OS authorization may leave this to `input_configs`.
    fn has_input_metadata(&self) -> Result<bool, mod_api::MicRefusal> {
        Ok(true)
    }

    fn input_configs(&self) -> Result<Option<Self::InputConfigs>, mod_api::MicRefusal>;
}

#[cfg(any(feature = "audio", test))]
struct DeviceInventory<D> {
    devices: Vec<D>,
    // Partial discovery can still return independently usable endpoints. When asked for an
    // explicit target, this error concerns only the part of discovery needed for that target.
    discovery_error: Option<mod_api::MicRefusal>,
}

#[cfg(any(feature = "audio", test))]
fn record_input_error(error: &mut Option<mod_api::MicRefusal>, refusal: mod_api::MicRefusal) {
    use mod_api::MicRefusal::{NoDevice, OsDenied};

    if refusal == OsDenied || matches!(*error, None | Some(NoDevice)) {
        *error = Some(refusal);
    }
}

#[cfg(any(feature = "audio", test))]
trait CaptureBackend: Send + Sync + 'static {
    type Device: BackendDevice;
    type Stream;

    /// Enumerates backend devices without an error-erasing `supports_input()` filter.
    /// During label lookup, `listed_label_ids` identifies previously listed matches.
    fn devices(
        &self,
        target: Option<&mod_api::MicTarget>,
        listed_label_ids: &[String],
    ) -> Result<DeviceInventory<Self::Device>, mod_api::MicRefusal>;

    fn default_input_device(&self) -> Result<Option<Self::Device>, mod_api::MicRefusal>;

    /// Runs capture startup, including input probing, after any OS authorization required
    /// by this backend. Resolving device metadata alone must not request access.
    fn with_input_authorization(
        &self,
        open: impl FnOnce() -> Result<Self::Stream, mod_api::MicRefusal>,
    ) -> Result<Self::Stream, mod_api::MicRefusal> {
        open()
    }

    fn open_stream(
        &self,
        device: &Self::Device,
        probed: Option<<Self::Device as BackendDevice>::InputConfigs>,
        spec: &mod_api::MicSpec,
        ring: Arc<CaptureRing>,
        events: Arc<StreamEvents>,
    ) -> Result<Self::Stream, mod_api::MicRefusal>;

    fn run_stream(&self, stream: Self::Stream, stop: stop::Receiver) {
        stop.wait();
        drop(stream);
    }
}

#[cfg(any(feature = "audio", test))]
struct DeviceRegistry<D> {
    next_id: u64,
    // Remember the last listed label for an endpoint, not its availability or probe result.
    // Only generated selectors need to retain the backend device for endpoint equality.
    entries: Vec<(mod_api::MicDevice, Option<D>)>,
}

#[cfg(any(feature = "audio", test))]
impl<D: BackendDevice> DeviceRegistry<D> {
    fn new() -> Self {
        Self {
            next_id: 1,
            entries: Vec::new(),
        }
    }

    fn register(&mut self, dev: D, label: String) -> mod_api::MicDevice {
        let stable = dev.stable_selector();
        if let Some((listed, _)) =
            self.entries
                .iter_mut()
                .find(|(listed, remembered)| match &stable {
                    Some(id) => remembered.is_none() && listed.id == *id,
                    None => remembered
                        .as_ref()
                        .is_some_and(|old| old.same_endpoint(&dev)),
                })
        {
            listed.label = label;
            return listed.clone();
        }
        let (id, remembered) = match stable {
            Some(id) => (id, None),
            None => {
                let id = format!("input:{}", self.next_id);
                self.next_id += 1;
                (id, Some(dev))
            }
        };
        let listed = mod_api::MicDevice::new(id, label);
        self.entries.push((listed.clone(), remembered));
        listed
    }

    fn lookup(&self, selector: &str) -> Option<D> {
        self.entries
            .iter()
            .find(|(listed, _)| listed.id == selector)
            .and_then(|(_, dev)| dev.clone())
    }

    fn listed_ids_for_label(&self, label: &str) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(listed, _)| listed.label == label)
            .map(|(listed, _)| listed.id.clone())
            .collect()
    }
}

#[cfg(any(feature = "audio", test))]
fn list_devices_with<B: CaptureBackend>(
    backend: &B,
    registry: &Mutex<DeviceRegistry<B::Device>>,
) -> Result<Vec<mod_api::MicDevice>, mod_api::MicRefusal> {
    let DeviceInventory {
        devices,
        mut discovery_error,
    } = backend.devices(None, &[])?;
    let mut found = Vec::new();
    let mut reg = registry.lock().unwrap_or_else(|e| e.into_inner());

    for dev in devices {
        match dev.input_configs() {
            Ok(Some(_)) => {
                if let Some(label) = dev.label()
                    && !label.is_empty()
                {
                    found.push(reg.register(dev, label));
                }
            }
            Ok(None) | Err(mod_api::MicRefusal::NoDevice) => {}
            Err(err) => record_input_error(&mut discovery_error, err),
        }
    }
    if found.is_empty()
        && let Some(err) = discovery_error
    {
        return Err(err);
    }
    Ok(found)
}

#[cfg(any(feature = "audio", test))]
fn resolve_and_open_stream<B: CaptureBackend>(
    backend: &B,
    registry: &Mutex<DeviceRegistry<B::Device>>,
    spec: &mod_api::MicSpec,
    ring: Arc<CaptureRing>,
    events: Arc<StreamEvents>,
) -> Result<B::Stream, mod_api::MicRefusal> {
    use mod_api::{MicRefusal, MicTarget};

    let device = match &spec.device {
        None => backend
            .default_input_device()?
            .ok_or(MicRefusal::NoDevice)?,
        Some(target @ MicTarget::DeviceId(id)) => {
            let remembered = registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .lookup(id);
            let DeviceInventory {
                devices,
                discovery_error,
            } = backend.devices(Some(target), &[])?;
            let mut matching = devices.into_iter().filter(|dev| {
                if let Some(remembered) = &remembered {
                    dev.same_endpoint(remembered)
                } else {
                    dev.stable_selector().as_deref() == Some(id.as_str())
                }
            });
            let device = matching
                .next()
                .ok_or(discovery_error.unwrap_or(MicRefusal::NoDevice))?;
            if matching.next().is_some() {
                return Err(MicRefusal::AmbiguousDevice);
            }
            device
        }
        Some(target @ MicTarget::Label(label)) => {
            let listed_ids = registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .listed_ids_for_label(label);
            let DeviceInventory {
                devices: mut matching,
                mut discovery_error,
            } = backend.devices(Some(target), &listed_ids)?;
            // A name alone can identify a CoreAudio output. Check input direction without
            // querying configurations, which would create an AudioUnit before authorization.
            matching.retain(|dev| {
                if dev.label().as_deref() != Some(label.as_str()) {
                    return false;
                }
                match dev.has_input_metadata() {
                    Ok(supports_input) => supports_input,
                    Err(err) => {
                        record_input_error(&mut discovery_error, err);
                        false
                    }
                }
            });
            if matching.is_empty() {
                return Err(discovery_error.unwrap_or(MicRefusal::NoDevice));
            }
            return backend.with_input_authorization(|| {
                let mut candidate = None;
                let mut probe_error = None;
                for dev in matching {
                    match dev.input_configs() {
                        Ok(Some(configs)) => {
                            if candidate.is_some() {
                                return Err(MicRefusal::AmbiguousDevice);
                            }
                            candidate = Some((dev, configs));
                        }
                        Ok(None) | Err(MicRefusal::NoDevice) => {}
                        Err(err) => record_input_error(&mut probe_error, err),
                    }
                }
                if probe_error == Some(MicRefusal::OsDenied) {
                    return Err(MicRefusal::OsDenied);
                }
                // One usable input doesn't prove the label is unique if walking the
                // relevant part of the inventory failed.
                if let Some(err) = discovery_error {
                    return Err(err);
                }
                let (device, configs) =
                    candidate.ok_or(probe_error.unwrap_or(MicRefusal::NoDevice))?;
                backend.open_stream(&device, Some(configs), spec, ring, events)
            });
        }
    };

    backend.with_input_authorization(|| backend.open_stream(&device, None, spec, ring, events))
}

#[cfg(any(feature = "audio", test))]
enum StreamStartup {
    Opening(Option<mod_api::MicRefusal>),
    Published,
}

#[cfg(any(feature = "audio", test))]
struct StreamEvents {
    closed: AtomicBool,
    device_open: AtomicBool,
    // Shared only by the startup worker and the error callback, never the PCM callback.
    startup: Mutex<StreamStartup>,
    stop: stop::Sender,
}

#[cfg(any(feature = "audio", test))]
impl StreamEvents {
    fn publish_ready(
        &self,
        ready: &mpsc::SyncSender<Result<(), mod_api::MicRefusal>>,
        result: Result<(), mod_api::MicRefusal>,
    ) -> bool {
        let mut startup = self.startup.lock().unwrap_or_else(|err| err.into_inner());
        let result = match *startup {
            StreamStartup::Opening(Some(refusal)) => Err(refusal),
            _ => result,
        };
        *startup = StreamStartup::Published;
        ready.send(result).is_ok() && result.is_ok()
    }

    fn report_error(&self, refusal: mod_api::MicRefusal) {
        let mut startup = self.startup.lock().unwrap_or_else(|err| err.into_inner());
        if let StreamStartup::Opening(first_error) = &mut *startup {
            first_error.get_or_insert(refusal);
        }
        self.stop();
    }

    fn stop(&self) {
        self.device_open.store(false, Ordering::Release);
        self.stop.signal();
    }
}

/// Engine-side handle backing a mod's [`mod_api::MicRef`].
///
/// Never passed to the OS audio callback so calling `close()` or dropping `MicRef` signals the
/// stream's dedicated teardown thread and waits for the backend stream to be released before
/// returning.
#[cfg(any(feature = "audio", test))]
struct MicHandle {
    ring: Arc<CaptureRing>,
    channels: u8,
    rate: u32,
    events: Arc<StreamEvents>,
    teardown: Mutex<Option<thread::JoinHandle<()>>>,
}

#[cfg(any(feature = "audio", test))]
impl mod_api::MicSource for MicHandle {
    fn read(&self, pcm: &mut [i16]) -> u32 {
        if self.events.closed.load(Ordering::Acquire) {
            return 0;
        }
        self.ring.read_frames(pcm)
    }

    fn channels(&self) -> u8 {
        self.channels
    }

    fn rate(&self) -> u32 {
        self.rate
    }

    fn dropped_frames(&self) -> u64 {
        self.ring.dropped_frames()
    }

    fn is_open(&self) -> bool {
        !self.events.closed.load(Ordering::Acquire)
            && self.events.device_open.load(Ordering::Acquire)
    }

    fn close(&self) {
        self.events.closed.store(true, Ordering::Release);
        let mut slot = self.teardown.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(join) = slot.take() {
            self.events.stop();
            let _ = join.join();
        }
    }
}

#[cfg(any(feature = "audio", test))]
impl Drop for MicHandle {
    fn drop(&mut self) {
        mod_api::MicSource::close(self);
    }
}

#[cfg(any(feature = "audio", test))]
type ActiveStreams = ActiveMap<u64, Arc<StreamEvents>>;

#[cfg(any(feature = "audio", test))]
struct MicPoolInner<B: CaptureBackend> {
    enabled: bool,
    backend: Arc<B>,
    registry: Arc<Mutex<DeviceRegistry<B::Device>>>,
    active: Arc<Mutex<ActiveStreams>>,
    shutdown: Arc<AtomicBool>,
    next_id: AtomicU64,
}

#[cfg(any(feature = "audio", test))]
impl<B: CaptureBackend> MicPoolInner<B> {
    fn new(enabled: bool, backend: Arc<B>) -> Self {
        Self {
            enabled,
            backend,
            registry: Arc::new(Mutex::new(DeviceRegistry::new())),
            active: Arc::new(Mutex::new(ActiveStreams::new())),
            shutdown: Arc::new(AtomicBool::new(false)),
            next_id: AtomicU64::new(1),
        }
    }

    fn devices(&self) -> Result<Vec<mod_api::MicDevice>, mod_api::MicRefusal> {
        if !self.enabled || self.shutdown.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        list_devices_with(self.backend.as_ref(), &self.registry)
    }

    fn open(
        &self,
        spec: &mod_api::MicSpec,
    ) -> Result<Arc<dyn mod_api::MicSource>, mod_api::MicRefusal> {
        validate_spec(spec)?;
        if !self.enabled || self.shutdown.load(Ordering::Acquire) {
            return Err(mod_api::MicRefusal::NoDevice);
        }
        let ring = Arc::new(CaptureRing::new(spec.rate, spec.channels));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), mod_api::MicRefusal>>(1);
        let (stop_tx, stop_rx) = stop::channel().map_err(|err| {
            log::warn!("Sunrise: failed to create microphone stop signal: {err}");
            mod_api::MicRefusal::DeviceFailed
        })?;
        let events = Arc::new(StreamEvents {
            closed: AtomicBool::new(false),
            device_open: AtomicBool::new(true),
            startup: Mutex::new(StreamStartup::Opening(None)),
            stop: stop_tx,
        });

        let backend = Arc::clone(&self.backend);
        let registry = Arc::clone(&self.registry);
        let active = Arc::clone(&self.active);
        let shutdown = Arc::clone(&self.shutdown);
        let spec_owned = spec.clone();
        let ring_t = Arc::clone(&ring);
        let worker_events = Arc::clone(&events);

        let join = thread::Builder::new()
            .name("mic-stream".into())
            .spawn(move || {
                let opened = resolve_and_open_stream(
                    backend.as_ref(),
                    &registry,
                    &spec_owned,
                    ring_t,
                    Arc::clone(&worker_events),
                );
                match opened {
                    Ok(stream) => {
                        let registered = match active.lock() {
                            Ok(mut map) if !shutdown.load(Ordering::Acquire) => {
                                map.insert(id, Arc::clone(&worker_events));
                                true
                            }
                            _ => false,
                        };
                        let result = if registered {
                            Ok(())
                        } else {
                            Err(mod_api::MicRefusal::NoDevice)
                        };
                        if worker_events.publish_ready(&ready_tx, result) {
                            backend.run_stream(stream, stop_rx);
                        }
                    }
                    Err(refusal) => {
                        worker_events.publish_ready(&ready_tx, Err(refusal));
                    }
                }
                worker_events.device_open.store(false, Ordering::Release);
                if let Ok(mut map) = active.lock() {
                    map.remove(&id);
                }
            })
            .map_err(|e| {
                log::warn!("Sunrise: failed to start mic-stream thread: {e}");
                mod_api::MicRefusal::DeviceFailed
            })?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Arc::new(MicHandle {
                ring,
                channels: spec.channels,
                rate: spec.rate,
                events,
                teardown: Mutex::new(Some(join)),
            })),
            Ok(Err(refusal)) => {
                let _ = join.join();
                Err(refusal)
            }
            Err(_) => {
                let _ = join.join();
                Err(mod_api::MicRefusal::DeviceFailed)
            }
        }
    }
}

#[cfg(any(feature = "audio", test))]
impl<B: CaptureBackend> Drop for MicPoolInner<B> {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Ok(mut map) = self.active.lock() {
            for (_, events) in map.drain() {
                events.stop();
            }
        }
    }
}

/// Client microphone capture pool. Spawns no threads until a stream is opened.
pub(super) struct MicPool {
    #[cfg(feature = "audio")]
    inner: MicPoolInner<NativeBackend>,
}

impl MicPool {
    /// Creates a new capture pool without spawning any thread.
    pub(super) fn new(enabled: bool) -> Self {
        #[cfg(not(feature = "audio"))]
        let _ = enabled;
        Self {
            #[cfg(feature = "audio")]
            inner: MicPoolInner::new(enabled, Arc::new(NativeBackend)),
        }
    }

    /// Lists available audio input devices with distinct selectors and display labels.
    pub(super) fn devices(&self) -> Result<Vec<mod_api::MicDevice>, mod_api::MicRefusal> {
        #[cfg(not(feature = "audio"))]
        {
            Ok(Vec::new())
        }
        #[cfg(feature = "audio")]
        {
            self.inner.devices()
        }
    }

    /// Validates `spec` and opens a live microphone capture stream.
    pub(super) fn open(
        &self,
        spec: &mod_api::MicSpec,
    ) -> Result<Arc<dyn mod_api::MicSource>, mod_api::MicRefusal> {
        #[cfg(not(feature = "audio"))]
        {
            validate_spec(spec)?;
            Err(mod_api::MicRefusal::NoDevice)
        }
        #[cfg(feature = "audio")]
        {
            self.inner.open(spec)
        }
    }
}

#[cfg(any(all(feature = "audio", target_os = "macos"), test))]
fn parse_coreaudio_device_id(debug: &str) -> Option<u32> {
    let rest = debug.split_once("audio_device_id:")?.1.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(any(all(feature = "audio", target_os = "macos"), test))]
fn enumerated_default<D: BackendDevice>(default: &D, devices: Vec<D>) -> Option<D> {
    let selector = default.stable_selector()?;
    devices
        .into_iter()
        .find(|dev| dev.stable_selector().as_ref() == Some(&selector))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{MicDevice, MicRefusal, MicSpec, MicSpecFault};
    use std::sync::Barrier;
    use std::time::{Duration, Instant};

    #[derive(Clone)]
    struct FakeDevice {
        endpoint_id: String,
        expose_selector: bool,
        label: String,
        has_input_metadata: bool,
        supported: Result<Vec<InputConfigRange>, MicRefusal>,
        probed: Option<Arc<AtomicBool>>,
        default_cfg: Result<InputConfig, MicRefusal>,
    }

    impl FakeDevice {
        fn input(endpoint_id: &str, label: &str, rate: u32, format: PcmFormat) -> Self {
            Self {
                endpoint_id: endpoint_id.to_owned(),
                expose_selector: true,
                label: label.to_owned(),
                has_input_metadata: true,
                supported: Ok(vec![InputConfigRange {
                    channels: 1,
                    min_rate: 8_000,
                    max_rate: 48_000,
                    format,
                }]),
                probed: None,
                default_cfg: Ok(InputConfig {
                    channels: 1,
                    rate,
                    format,
                }),
            }
        }
    }

    impl BackendDevice for FakeDevice {
        type InputConfigs = Vec<InputConfigRange>;

        fn label(&self) -> Option<String> {
            Some(self.label.clone())
        }

        fn stable_selector(&self) -> Option<String> {
            self.expose_selector.then(|| self.endpoint_id.clone())
        }

        fn same_endpoint(&self, other: &Self) -> bool {
            self.endpoint_id == other.endpoint_id
        }

        fn has_input_metadata(&self) -> Result<bool, MicRefusal> {
            Ok(self.has_input_metadata)
        }

        fn input_configs(&self) -> Result<Option<Self::InputConfigs>, MicRefusal> {
            if let Some(probed) = &self.probed {
                probed.store(true, Ordering::Relaxed);
            }
            match &self.supported {
                Ok(ranges) if ranges.is_empty() => Ok(None),
                Ok(ranges) => Ok(Some(ranges.clone())),
                Err(err) => Err(*err),
            }
        }
    }

    struct FakeStream {
        disposed: Arc<AtomicBool>,
    }

    impl Drop for FakeStream {
        fn drop(&mut self) {
            self.disposed.store(true, Ordering::Release);
        }
    }

    struct OpenedStreamRecord {
        endpoint_id: String,
        config: InputConfig,
        disposed: Arc<AtomicBool>,
        ring: Arc<CaptureRing>,
        events: Arc<StreamEvents>,
    }

    struct FakeBackendState {
        devices: Vec<FakeDevice>,
        default_device: Option<FakeDevice>,
        devices_error: Option<MicRefusal>,
        // The first native request and its result, if this fake needs OS authorization.
        authorization_request: Option<Result<(), MicRefusal>>,
        authorization_requests: usize,
        preserve_hardware_rate: bool,
        startup_error: Option<MicRefusal>,
        opened: Vec<OpenedStreamRecord>,
        devices_barrier: Option<Arc<Barrier>>,
    }

    struct FakeBackend {
        state: Mutex<FakeBackendState>,
    }

    impl FakeBackend {
        fn new(devices: Vec<FakeDevice>, default_device: Option<FakeDevice>) -> Self {
            Self {
                state: Mutex::new(FakeBackendState {
                    devices,
                    default_device,
                    devices_error: None,
                    authorization_request: None,
                    authorization_requests: 0,
                    preserve_hardware_rate: false,
                    startup_error: None,
                    opened: Vec::new(),
                    devices_barrier: None,
                }),
            }
        }
    }

    impl CaptureBackend for FakeBackend {
        type Device = FakeDevice;
        type Stream = FakeStream;

        fn devices(
            &self,
            _target: Option<&mod_api::MicTarget>,
            _listed_label_ids: &[String],
        ) -> Result<DeviceInventory<Self::Device>, MicRefusal> {
            let barrier = self.state.lock().unwrap().devices_barrier.clone();
            if let Some(barrier) = barrier {
                barrier.wait();
                barrier.wait();
            }
            let st = self.state.lock().unwrap();
            if let Some(err) = st.devices_error {
                return Err(err);
            }
            Ok(DeviceInventory {
                devices: st.devices.clone(),
                discovery_error: None,
            })
        }

        fn default_input_device(&self) -> Result<Option<Self::Device>, MicRefusal> {
            Ok(self.state.lock().unwrap().default_device.clone())
        }

        fn with_input_authorization(
            &self,
            open: impl FnOnce() -> Result<Self::Stream, MicRefusal>,
        ) -> Result<Self::Stream, MicRefusal> {
            {
                let mut state = self.state.lock().unwrap();
                if let Some(decision) = state.authorization_request {
                    state.authorization_requests += 1;
                    decision?;
                }
            }
            open()
        }

        fn open_stream(
            &self,
            device: &Self::Device,
            probed: Option<Vec<InputConfigRange>>,
            spec: &MicSpec,
            ring: Arc<CaptureRing>,
            events: Arc<StreamEvents>,
        ) -> Result<Self::Stream, MicRefusal> {
            let mut state = self.state.lock().unwrap();
            let config = if state.preserve_hardware_rate {
                device.default_cfg?
            } else {
                let ranges = match probed {
                    Some(ranges) => ranges,
                    None => device.input_configs()?.ok_or(MicRefusal::NoDevice)?,
                };
                choose_best_config(&ranges, spec).ok_or(MicRefusal::DeviceFailed)?
            };
            let disposed = Arc::new(AtomicBool::new(false));
            if let Some(refusal) = state.startup_error {
                events.report_error(refusal);
            }
            state.opened.push(OpenedStreamRecord {
                endpoint_id: device.endpoint_id.clone(),
                config,
                disposed: Arc::clone(&disposed),
                ring,
                events,
            });
            Ok(FakeStream { disposed })
        }
    }

    #[test]
    fn validate_spec_bounds() {
        assert_eq!(
            validate_spec(&MicSpec::new(0, 16_000)),
            Err(MicRefusal::BadSpec(MicSpecFault::Channels))
        );
        assert_eq!(
            validate_spec(&MicSpec::new(3, 16_000)),
            Err(MicRefusal::BadSpec(MicSpecFault::Channels))
        );
        assert_eq!(
            validate_spec(&MicSpec::new(1, 7_999)),
            Err(MicRefusal::BadSpec(MicSpecFault::Rate))
        );
        assert_eq!(
            validate_spec(&MicSpec::new(2, 48_001)),
            Err(MicRefusal::BadSpec(MicSpecFault::Rate))
        );
        for channels in [1_u8, 2] {
            for rate in [8_000_u32, 16_000, 44_100, 48_000] {
                assert_eq!(validate_spec(&MicSpec::new(channels, rate)), Ok(()));
            }
        }
    }

    #[test]
    fn capture_ring_fifo_and_whole_frame_truncation() {
        let sized = CaptureRing::new(16_000, 1);
        assert_eq!(sized.cap_frames, 8_000);

        let ring = CaptureRing::with_capacity_frames(8, 2);
        ring.push_frames(&[10, -10, 20, -20, 30, -30]);
        assert_eq!(ring.dropped_frames(), 0);

        let mut odd = [99_i16; 5];
        assert_eq!(ring.read_frames(&mut odd), 2);
        assert_eq!(odd, [10, -10, 20, -20, 99]);

        let mut too_small = [77_i16; 1];
        assert_eq!(ring.read_frames(&mut too_small), 0);
        assert_eq!(too_small, [77]);

        let mut rest = [0_i16; 4];
        assert_eq!(ring.read_frames(&mut rest), 1);
        assert_eq!(&rest[..2], &[30, -30]);
        assert_eq!(ring.read_frames(&mut rest), 0);
    }

    #[test]
    fn capture_ring_overflow_and_oversized_push() {
        let ring = CaptureRing::with_capacity_frames(4, 1);
        ring.push_frames(&[1, 2, 3]);
        ring.push_frames(&[4, 5, 6]);
        assert_eq!(ring.dropped_frames(), 2);

        let mut out = [0_i16; 4];
        assert_eq!(ring.read_frames(&mut out), 4);
        assert_eq!(out, [3, 4, 5, 6]);

        ring.push_frames(&[10, 20]);
        ring.push_frames(&[30, 40, 50, 60, 70, 80]);
        assert_eq!(ring.dropped_frames(), 6);
        assert_eq!(ring.read_frames(&mut out), 4);
        assert_eq!(out, [50, 60, 70, 80]);
    }

    #[test]
    fn capture_ring_reader_contention_and_overwrite_leave_suffix_untouched() {
        const SENTINEL: i16 = 0x5A5A;

        // 1. Reader contention is nonblocking and never modifies the caller's slice.
        let ring = CaptureRing::with_capacity_frames(8, 2);
        ring.push_frames(&[11, -11, 22, -22]);
        {
            let _held = ring.scratch.lock().unwrap();
            let mut out = [SENTINEL; 6];
            assert_eq!(ring.read_frames(&mut out), 0);
            assert_eq!(out, [SENTINEL; 6]);
        }

        // 2. Overwrite retry that exhausts attempts returns 0 and leaves the entire slice untouched.
        let ring = CaptureRing::with_capacity_frames(4, 1);
        ring.push_frames(&[1, 2, 3, 4]);
        let mut out = [SENTINEL; 4];
        let count = ring.read_frames_inner(&mut out, |attempt| {
            let base = (attempt as i16 + 1) * 10;
            ring.push_frames(&[base, base + 1, base + 2, base + 3]);
        });
        assert_eq!(count, 0);
        assert_eq!(out, [SENTINEL; 4]);

        // 3. Overwrite retry that succeeds on a shorter prefix leaves the suffix completely untouched.
        let ring = CaptureRing::with_capacity_frames(4, 1);
        ring.push_frames(&[1, 2, 3, 4]);
        let mut out = [SENTINEL; 4];
        let count = ring.read_frames_inner(&mut out, |attempt| {
            if attempt == 0 {
                // Force first 4-frame attempt to fail overwrite validation, then leave only 2 new
                // frames unread by advancing head through an overflow + draining 2 frames.
                ring.push_frames(&[10, 20, 30, 40]);
                let _ = ring.head.fetch_add(2, Ordering::SeqCst);
            }
        });
        assert_eq!(count, 2);
        assert_eq!(out, [30, 40, SENTINEL, SENTINEL]);
    }

    #[test]
    fn capture_ring_concurrent_monotonic_tear_free() {
        const TOTAL_FRAMES: u64 = 20_000;
        const SENTINEL: i16 = i16::MIN;
        let ring = Arc::new(CaptureRing::with_capacity_frames(32, 2));
        let done = Arc::new(AtomicBool::new(false));

        let prod_ring = Arc::clone(&ring);
        let prod_done = Arc::clone(&done);
        let producer = std::thread::spawn(move || {
            let mut seq = 1_i16;
            let mut sent = 0_u64;
            let mut batch = [0_i16; 14];
            while sent < TOTAL_FRAMES {
                let count = ((TOTAL_FRAMES - sent) as usize).min(7);
                for i in 0..count {
                    batch[i * 2] = seq;
                    batch[i * 2 + 1] = -seq;
                    seq += 1;
                }
                prod_ring.push_frames(&batch[..count * 2]);
                sent += count as u64;
            }
            prod_done.store(true, Ordering::Release);
        });

        let mut last = 0_i16;
        let mut total_read = 0_u64;
        loop {
            let mut buf = [SENTINEL; 25]; // 12 stereo frames + 1 trailing sample
            let n = ring.read_frames(&mut buf) as usize;
            assert!(
                buf[n * 2..].iter().all(|&s| s == SENTINEL),
                "unreturned suffix must remain untouched"
            );
            if n > 0 {
                for i in 0..n {
                    let l = buf[i * 2];
                    let r = buf[i * 2 + 1];
                    assert_eq!(r, -l, "stereo frame must not tear");
                    assert!(l > last, "frames must remain strictly monotonic");
                    last = l;
                }
                total_read += n as u64;
            } else if done.load(Ordering::Acquire) {
                let mut tail_buf = [SENTINEL; 24];
                let tail = ring.read_frames(&mut tail_buf) as usize;
                assert!(tail_buf[tail * 2..].iter().all(|&s| s == SENTINEL));
                for i in 0..tail {
                    let l = tail_buf[i * 2];
                    let r = tail_buf[i * 2 + 1];
                    assert_eq!(r, -l, "stereo frame must not tear");
                    assert!(l > last, "frames must remain strictly monotonic");
                    last = l;
                }
                total_read += tail as u64;
                if tail == 0 {
                    break;
                }
            } else {
                std::thread::yield_now();
            }
        }
        producer.join().unwrap();
        assert_eq!(total_read + ring.dropped_frames(), TOTAL_FRAMES);
        assert!(total_read > 0);
    }

    #[test]
    fn input_converter_native_i16_passthrough_preserves_full_scale() {
        // Matching mono i16 -> bit-exact passthrough including both full-scale endpoints.
        let ring = CaptureRing::with_capacity_frames(16, 1);
        let mut conv = InputConverter::new(1, 48_000, 1, 48_000);
        let input = [i16::MIN, -32_767, -1, 0, 1, 32_766, i16::MAX];
        conv.push_i16_samples(&input, &ring);
        let mut out = [0_i16; 8];
        assert_eq!(ring.read_frames(&mut out), 7);
        assert_eq!(&out[..7], &input);

        // Matching stereo i16 -> bit-exact passthrough including both full-scale endpoints.
        let ring = CaptureRing::with_capacity_frames(16, 2);
        let mut conv = InputConverter::new(2, 16_000, 2, 16_000);
        let stereo = [i16::MIN, i16::MAX, -1, 1];
        conv.push_i16_samples(&stereo, &ring);
        let mut out_stereo = [0_i16; 4];
        assert_eq!(ring.read_frames(&mut out_stereo), 2);
        assert_eq!(out_stereo, stereo);
    }

    #[test]
    fn input_converter_channel_mapping_and_clamping() {
        let ring = CaptureRing::with_capacity_frames(16, 1);
        let mut conv = InputConverter::new(2, 16_000, 1, 16_000);
        conv.push_f32_samples(&[1.0, 0.0, -0.5, 0.5, -1.0, -1.0], &ring);
        let mut mono = [0_i16; 4];
        assert_eq!(ring.read_frames(&mut mono), 3);
        assert_eq!(&mono[..3], &[16_384, 0, -32_767]);

        let ring = CaptureRing::with_capacity_frames(16, 2);
        let mut conv = InputConverter::new(1, 16_000, 2, 16_000);
        conv.push_f32_samples(&[0.25, -0.5], &ring);
        let mut stereo = [0_i16; 4];
        assert_eq!(ring.read_frames(&mut stereo), 2);
        assert_eq!(stereo, [8_192, 8_192, -16_384, -16_384]);

        let ring = CaptureRing::with_capacity_frames(16, 1);
        let mut conv = InputConverter::new(4, 16_000, 1, 16_000);
        conv.push_f32_samples(&[0.4, 0.4, 0.0, 0.0], &ring);
        let mut out1 = [0_i16; 2];
        assert_eq!(ring.read_frames(&mut out1), 1);
        assert_eq!(out1[0], 6_553);

        let ring = CaptureRing::with_capacity_frames(16, 2);
        let mut conv = InputConverter::new(4, 16_000, 2, 16_000);
        conv.push_f32_samples(&[0.5, -0.25, 1.0, -1.0], &ring);
        let mut out2 = [0_i16; 2];
        assert_eq!(ring.read_frames(&mut out2), 1);
        assert_eq!(out2, [16_384, -8_192]);

        let ring = CaptureRing::with_capacity_frames(16, 1);
        let mut conv = InputConverter::new(1, 16_000, 1, 16_000);
        conv.push_f32_samples(&[2.5, -3.0, 1.0, -1.0], &ring);
        let mut clamped = [0_i16; 4];
        assert_eq!(ring.read_frames(&mut clamped), 4);
        assert_eq!(clamped, [32_767, -32_767, 32_767, -32_767]);
    }

    #[test]
    fn input_converter_resampling_continuity_across_chunks() {
        let input_down: Vec<f32> = (0..30).map(|i| i as f32 / 100.0).collect();
        let ring_chunked = CaptureRing::with_capacity_frames(64, 1);
        let mut conv_chunked = InputConverter::new(1, 48_000, 1, 16_000);
        let mut offset = 0;
        for chunk_len in [7, 5, 11, 7] {
            conv_chunked.push_f32_samples(&input_down[offset..offset + chunk_len], &ring_chunked);
            offset += chunk_len;
        }
        let mut got_down = [0_i16; 16];
        let count_down = ring_chunked.read_frames(&mut got_down) as usize;
        assert_eq!(count_down, 10);
        for (k, &sample) in got_down[..count_down].iter().enumerate() {
            let expected = (((k * 3) as f32 / 100.0) * 32767.0).round() as i16;
            assert_eq!(sample, expected);
        }

        let input_up: Vec<f32> = (0..10).map(|i| i as f32 * 0.1).collect();
        let ring_single = CaptureRing::with_capacity_frames(64, 1);
        let mut conv_single = InputConverter::new(1, 16_000, 1, 48_000);
        conv_single.push_f32_samples(&input_up, &ring_single);

        let ring_multi = CaptureRing::with_capacity_frames(64, 1);
        let mut conv_multi = InputConverter::new(1, 16_000, 1, 48_000);
        for chunk in [&input_up[0..3], &input_up[3..5], &input_up[5..10]] {
            conv_multi.push_f32_samples(chunk, &ring_multi);
        }

        let mut single_out = [0_i16; 64];
        let mut multi_out = [0_i16; 64];
        let n_single = ring_single.read_frames(&mut single_out) as usize;
        let n_multi = ring_multi.read_frames(&mut multi_out) as usize;
        assert_eq!(n_single, 28);
        assert_eq!(n_multi, n_single);
        assert_eq!(&multi_out[..n_multi], &single_out[..n_single]);
        for (k, &sample) in multi_out[..n_multi].iter().enumerate() {
            let expected = (((k as f64 / 3.0) * 0.1) as f32 * 32767.0).round() as i16;
            assert!((sample - expected).abs() <= 1);
        }
    }

    #[test]
    fn pcm_format_ranking_prefers_native_i16_then_f32() {
        assert!(PcmFormat::I16.rank() > PcmFormat::F32.rank());
        assert!(PcmFormat::F32.rank() > PcmFormat::I24.rank());
        assert_eq!(PcmFormat::I24.rank(), PcmFormat::I32.rank());
        assert_eq!(PcmFormat::I32.rank(), PcmFormat::F64.rank());
        assert!(PcmFormat::F64.rank() > PcmFormat::I8.rank());
        for fmt in [
            PcmFormat::I8,
            PcmFormat::I64,
            PcmFormat::U8,
            PcmFormat::U16,
            PcmFormat::U32,
            PcmFormat::U64,
        ] {
            assert_eq!(fmt.rank(), 0);
        }
        let chosen = choose_best_config(
            &[
                InputConfigRange {
                    channels: 1,
                    min_rate: 8_000,
                    max_rate: 48_000,
                    format: PcmFormat::F32,
                },
                InputConfigRange {
                    channels: 1,
                    min_rate: 8_000,
                    max_rate: 48_000,
                    format: PcmFormat::I16,
                },
            ],
            &MicSpec::new(1, 48_000),
        );
        assert_eq!(
            chosen,
            Some(InputConfig {
                channels: 1,
                rate: 48_000,
                format: PcmFormat::I16,
            })
        );
    }

    #[test]
    fn denied_backend_preserves_os_denied_for_list_named_and_default_open() {
        let denied = FakeDevice {
            supported: Err(MicRefusal::OsDenied),
            // Even if default_input_config would succeed, it must not mask OsDenied.
            ..FakeDevice::input("alsa:usb-mic", "USB Mic", 48_000, PcmFormat::F32)
        };
        let output_only = FakeDevice {
            has_input_metadata: false,
            supported: Ok(Vec::new()),
            default_cfg: Err(MicRefusal::DeviceFailed),
            ..FakeDevice::input("alsa:hdmi", "HDMI Out", 48_000, PcmFormat::F32)
        };
        let backend = Arc::new(FakeBackend::new(
            vec![output_only, denied.clone()],
            Some(denied),
        ));
        let pool = MicPoolInner::new(true, backend);

        assert_eq!(pool.devices(), Err(MicRefusal::OsDenied));
        assert_eq!(
            pool.open(&MicSpec::new(1, 16_000)).err(),
            Some(MicRefusal::OsDenied)
        );
        assert_eq!(
            pool.open(&MicSpec::new(1, 16_000).with_label("USB Mic"))
                .err(),
            Some(MicRefusal::OsDenied)
        );
        assert_eq!(
            pool.open(&MicSpec::new(1, 16_000).with_device("alsa:usb-mic"))
                .err(),
            Some(MicRefusal::OsDenied)
        );
    }

    #[test]
    fn duplicate_labels_keep_distinct_selectors_and_refuse_vanished_or_ambiguous() {
        let mic_a = FakeDevice {
            expose_selector: false, // Exercise endpoint-equality selector registry (WASAPI-like).
            ..FakeDevice::input("usb-port-1", "USB Microphone", 48_000, PcmFormat::I16)
        };
        let mic_b = FakeDevice {
            expose_selector: false,
            ..FakeDevice::input("usb-port-2", "USB Microphone", 48_000, PcmFormat::I16)
        };
        let backend = Arc::new(FakeBackend::new(
            vec![mic_a.clone(), mic_b.clone()],
            Some(mic_a.clone()),
        ));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));

        let listed = pool.devices().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].label, "USB Microphone");
        assert_eq!(listed[1].label, "USB Microphone");
        assert_ne!(listed[0].id, listed[1].id);

        // Passing the ambiguous display label refuses instead of opening whichever is first.
        assert_eq!(
            pool.open(&MicSpec::new(1, 48_000).with_label("USB Microphone"))
                .err(),
            Some(MicRefusal::AmbiguousDevice)
        );

        // Reorder devices in the backend: selector for mic B still resolves to usb-port-2.
        backend.state.lock().unwrap().devices = vec![mic_b.clone(), mic_a.clone()];
        let handle_b = pool
            .open(&MicSpec::new(1, 48_000).with_device(&listed[1].id))
            .unwrap();
        assert_eq!(
            backend
                .state
                .lock()
                .unwrap()
                .opened
                .last()
                .unwrap()
                .endpoint_id,
            "usb-port-2"
        );
        handle_b.close();

        // Unplug mic B: opening its selector refuses as vanished rather than opening mic A.
        backend.state.lock().unwrap().devices = vec![mic_a];
        assert_eq!(
            pool.open(&MicSpec::new(1, 48_000).with_device(&listed[1].id))
                .err(),
            Some(MicRefusal::NoDevice)
        );
    }

    #[test]
    fn vanished_backend_selector_does_not_match_an_input_label() {
        let removed = FakeDevice::input("coreaudio:42", "Built-in Mic", 48_000, PcmFormat::F32);
        let remaining = FakeDevice::input("coreaudio:75", "coreaudio:42", 48_000, PcmFormat::F32);
        let backend = Arc::new(FakeBackend::new(vec![removed, remaining], None));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        let listed = pool.devices().unwrap();
        let id = &listed[0].id;
        assert_eq!(id, "coreaudio:42");

        backend.state.lock().unwrap().devices.remove(0);
        let spec = MicSpec::new(1, 48_000);
        assert_eq!(
            pool.open(&spec.clone().with_device(id)).err(),
            Some(MicRefusal::NoDevice)
        );
        assert!(backend.state.lock().unwrap().opened.is_empty());

        let mic = pool.open(&spec.with_label(id)).unwrap();
        assert_eq!(
            backend.state.lock().unwrap().opened[0].endpoint_id,
            "coreaudio:75"
        );
        mic.close();
    }

    #[test]
    fn label_open_requests_native_permission_before_a_failing_input_probe() {
        let mut device = FakeDevice::input("coreaudio:42", "Desk Mic", 48_000, PcmFormat::F32);
        device.supported = Err(MicRefusal::NoDevice);
        let probed = Arc::new(AtomicBool::new(false));
        device.probed = Some(Arc::clone(&probed));
        let backend = Arc::new(FakeBackend::new(vec![device], None));
        backend.state.lock().unwrap().authorization_request = Some(Err(MicRefusal::OsDenied));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        let spec = MicSpec::new(1, 48_000);

        assert_eq!(
            pool.open(&spec.clone().with_label("Absent Mic")).err(),
            Some(MicRefusal::NoDevice)
        );
        assert_eq!(backend.state.lock().unwrap().authorization_requests, 0);
        let desk = spec.with_label("Desk Mic");
        assert_eq!(pool.open(&desk).err(), Some(MicRefusal::OsDenied));
        assert!(!probed.load(Ordering::Relaxed));
        {
            let mut state = backend.state.lock().unwrap();
            assert_eq!(state.authorization_requests, 1);
            assert!(state.opened.is_empty());
            state.authorization_request = Some(Ok(()));
        }
        assert_eq!(pool.open(&desk).err(), Some(MicRefusal::NoDevice));
        assert!(probed.load(Ordering::Relaxed));
        let state = backend.state.lock().unwrap();
        assert_eq!(state.authorization_requests, 2);
        assert!(state.opened.is_empty());
    }

    #[test]
    fn output_only_label_neither_probes_nor_requests_microphone_access() {
        let probed = Arc::new(AtomicBool::new(false));
        let output = FakeDevice {
            has_input_metadata: false,
            supported: Ok(Vec::new()),
            probed: Some(Arc::clone(&probed)),
            ..FakeDevice::input("coreaudio:75", "HDMI Out", 48_000, PcmFormat::F32)
        };
        let backend = Arc::new(FakeBackend::new(vec![output], None));
        backend.state.lock().unwrap().authorization_request = Some(Err(MicRefusal::OsDenied));
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        assert_eq!(
            pool.open(&MicSpec::new(1, 48_000).with_label("HDMI Out"))
                .err(),
            Some(MicRefusal::NoDevice)
        );
        assert!(!probed.load(Ordering::Relaxed));
        let state = backend.state.lock().unwrap();
        assert_eq!(state.authorization_requests, 0);
        assert!(state.opened.is_empty());
    }

    #[test]
    fn coreaudio_policy_preserves_hardware_rate_across_readers() {
        let dev = FakeDevice::input("coreaudio:42", "Built-in Mic", 48_000, PcmFormat::F32);
        let backend = Arc::new(FakeBackend::new(vec![dev.clone()], Some(dev)));
        backend.state.lock().unwrap().preserve_hardware_rate = true;
        let pool = MicPoolInner::new(true, Arc::clone(&backend));

        let mic_16k = pool.open(&MicSpec::new(1, 16_000)).unwrap();
        let mic_44k = pool.open(&MicSpec::new(1, 44_100)).unwrap();
        assert_eq!(mic_16k.rate(), 16_000);
        assert_eq!(mic_44k.rate(), 44_100);

        {
            let st = backend.state.lock().unwrap();
            assert_eq!(st.opened.len(), 2);
            assert_eq!(st.opened[0].config.rate, 48_000);
            assert_eq!(st.opened[1].config.rate, 48_000);
        }

        mic_16k.close();
        mic_44k.close();
    }

    #[test]
    fn close_drop_and_backend_error_dispose_stream_promptly_without_probe_blocking() {
        let dev = FakeDevice::input("mic-1", "Desk Mic", 48_000, PcmFormat::I16);
        let backend = Arc::new(FakeBackend::new(vec![dev.clone()], Some(dev)));
        let pool = Arc::new(MicPoolInner::new(true, Arc::clone(&backend)));

        // 1. Explicit close disposes backend stream before returning, even while devices() is blocked.
        let mic1 = pool.open(&MicSpec::new(1, 48_000)).unwrap();
        let disposed1 = Arc::clone(&backend.state.lock().unwrap().opened[0].disposed);
        assert!(!disposed1.load(Ordering::Acquire));

        let barrier = Arc::new(Barrier::new(2));
        backend.state.lock().unwrap().devices_barrier = Some(Arc::clone(&barrier));
        let pool_probe = Arc::clone(&pool);
        let probe_thread = thread::spawn(move || pool_probe.devices());
        // Wait until probe_thread is actively inside backend.devices().
        barrier.wait();

        mic1.close();
        assert!(
            disposed1.load(Ordering::Acquire),
            "close() must release backend stream before returning even during a blocked probe"
        );
        assert!(!mic1.is_open());

        // Release the blocked probe.
        barrier.wait();
        let listed = probe_thread.join().unwrap().unwrap();
        assert_eq!(
            listed,
            vec![MicDevice::new("mic-1".into(), "Desk Mic".into())]
        );
        backend.state.lock().unwrap().devices_barrier = None;

        // 2. Dropping the handle disposes the backend stream before drop returns.
        let mic2 = pool.open(&MicSpec::new(1, 48_000)).unwrap();
        let disposed2 = Arc::clone(&backend.state.lock().unwrap().opened[1].disposed);
        assert!(!disposed2.load(Ordering::Acquire));
        drop(mic2);
        assert!(disposed2.load(Ordering::Acquire));

        // 3. Backend stream error callback disposes the stream promptly without close/drop/open.
        let mic3 = pool.open(&MicSpec::new(1, 48_000)).unwrap();
        let (disposed3, events3, ring3) = {
            let st = backend.state.lock().unwrap();
            let rec = &st.opened[2];
            (
                Arc::clone(&rec.disposed),
                Arc::clone(&rec.events),
                Arc::clone(&rec.ring),
            )
        };
        ring3.push_frames(&[123]);
        let mut pcm = [0_i16; 1];
        assert_eq!(mic3.read(&mut pcm), 1);
        assert_eq!(pcm, [123]);

        // Simulate OS device disconnect error callback.
        events3.report_error(MicRefusal::NoDevice);
        assert!(!mic3.is_open());

        let deadline = Instant::now() + Duration::from_secs(2);
        while !disposed3.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            disposed3.load(Ordering::Acquire),
            "backend error callback must release the failed stream promptly"
        );
        // Subsequent close/drop after error disposal is a safe no-op.
        mic3.close();
        drop(mic3);
    }

    #[test]
    fn callback_error_before_ready_returns_typed_refusal_and_disposes_stream() {
        let dev = FakeDevice::input("mic-1", "Desk Mic", 48_000, PcmFormat::I16);
        let backend = Arc::new(FakeBackend::new(vec![dev.clone()], Some(dev)));
        backend.state.lock().unwrap().startup_error = Some(MicRefusal::OsDenied);
        let pool = MicPoolInner::new(true, Arc::clone(&backend));
        assert_eq!(
            pool.open(&MicSpec::new(1, 48_000)).err(),
            Some(MicRefusal::OsDenied)
        );
        assert!(
            backend.state.lock().unwrap().opened[0]
                .disposed
                .load(Ordering::Acquire)
        );
    }

    #[test]
    fn coreaudio_default_uses_the_enumerated_endpoint_not_its_unmonitored_handle() {
        let default = FakeDevice::input("coreaudio:14", "Default token", 48_000, PcmFormat::F32);
        let concrete = FakeDevice::input("coreaudio:14", "Headset", 48_000, PcmFormat::F32);
        let other = FakeDevice::input("coreaudio:38", "Headset", 48_000, PcmFormat::F32);
        let matched = enumerated_default(&default, vec![other.clone(), concrete]).unwrap();
        assert_eq!(matched.endpoint_id, "coreaudio:14");
        assert_eq!(matched.label, "Headset");
        assert!(enumerated_default(&default, vec![other]).is_none());
    }

    #[test]
    fn parse_coreaudio_device_id_extracts_numeric_id() {
        assert_eq!(
            parse_coreaudio_device_id(
                "Device { audio_device_id: 47, name: Ok(\"USB Microphone\") }"
            ),
            Some(47)
        );
        assert_eq!(
            parse_coreaudio_device_id("Device { name: Ok(\"x\") }"),
            None
        );
    }

    #[test]
    fn classify_error_message_mappings() {
        for msg in [
            "Permission denied",
            "ACCESS IS DENIED",
            "Not authorized to capture audio",
            "EPERM",
            "EACCES",
            "E_ACCESSDENIED",
            "HRESULT 0x80070005",
        ] {
            assert_eq!(classify_error_message(msg), MicRefusal::OsDenied, "{msg}");
        }
        for msg in [
            "Device or resource busy",
            "EBUSY",
            "Device already in use",
            "exclusive mode active",
            "AUDCLNT_E_DEVICE_IN_USE",
            "HRESULT 0x8889000a",
        ] {
            assert_eq!(classify_error_message(msg), MicRefusal::Busy, "{msg}");
        }
        for msg in ["unknown backend error", "cable unplugged", ""] {
            assert_eq!(
                classify_error_message(msg),
                MicRefusal::DeviceFailed,
                "{msg}"
            );
        }
    }

    #[test]
    fn disabled_mic_pool_behavior() {
        let pool = MicPool::new(false);
        assert_eq!(pool.devices(), Ok(Vec::new()));
        assert_eq!(
            pool.open(&MicSpec::new(1, 16_000)).err(),
            Some(MicRefusal::NoDevice)
        );
        assert_eq!(
            pool.open(&MicSpec::new(0, 16_000)).err(),
            Some(MicRefusal::BadSpec(MicSpecFault::Channels))
        );
        assert_eq!(
            pool.open(&MicSpec::new(1, 4_000)).err(),
            Some(MicRefusal::BadSpec(MicSpecFault::Rate))
        );
    }
}
