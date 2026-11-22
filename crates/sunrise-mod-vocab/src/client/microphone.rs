//! A client mod's microphone capture handle: [`MicRef`] reads whole PCM frames from a bounded
//! client-local ring filled off the OS audio input callback.
use std::sync::Arc;

/// The lowest sample rate microphone capture accepts, in Hz.
pub const MIC_RATE_MIN: u32 = 8_000;
/// The highest sample rate microphone capture accepts, in Hz.
pub const MIC_RATE_MAX: u32 = 48_000;
/// The capacity of a microphone capture ring, in milliseconds of audio.
pub const MIC_RING_MAX_MS: u32 = 500;

/// One audio input device discovered on this client.
///
/// Pass [`id`](Self::id) to [`MicSpec::with_device`] to open this specific endpoint, and display
/// [`label`](Self::label) to the player. Two physically distinct devices may share a display
/// label while carrying distinct selectors.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::MicDevice>();
/// ```
// no wire: client-local audio input device descriptor, never crosses the mod wire
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MicDevice {
    /// Opaque, distinct input selector to pass to [`MicSpec::with_device`]. Refresh the list
    /// if the OS changes the device inventory; do not construct or parse this string.
    pub id: String,
    /// Human-readable display label for this input device.
    pub label: String,
}

impl MicDevice {
    /// Builds a device descriptor on the engine side.
    // hidden: a game-side constructor, not part of the mod surface
    #[doc(hidden)]
    pub fn new(id: String, label: String) -> Self {
        Self { id, label }
    }
}

/// How to select a non-default microphone input. Endpoint IDs and display labels are distinct.
// no wire: selects a local microphone endpoint
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the device selector is either a stable endpoint ID or a display label
pub enum MicTarget {
    /// An opaque [`MicDevice::id`] returned by `microphone_devices()`. Never matched as a label.
    DeviceId(String),
    /// A human-readable [`MicDevice::label`]. Never matched as an endpoint ID or against an
    /// output-only device. The client must be able to establish that the input match is unique.
    Label(String),
}

/// The input device and PCM format requested when opening microphone capture on this client.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::MicSpec>();
/// ```
// no wire: client-local microphone capture specification, never crosses the mod wire
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MicSpec {
    /// An explicit endpoint ID or display label, or `None` for the system default input device.
    pub device: Option<MicTarget>,
    /// Channel count: `1` for mono or `2` for stereo.
    pub channels: u8,
    /// Sample rate in Hz, from [`MIC_RATE_MIN`] to [`MIC_RATE_MAX`].
    pub rate: u32,
}

impl MicSpec {
    /// A specification for the system default input device with `channels` (`1` or `2`) at `rate`
    /// Hz.
    pub fn new(channels: u8, rate: u32) -> Self {
        Self {
            device: None,
            channels,
            rate,
        }
    }

    /// The same specification targeting an input [`MicDevice::id`] from `microphone_devices()`.
    /// A disappeared endpoint is not replaced by a device with a matching display label. If OS
    /// permissions prevent checking the targeted endpoint, opening can return [`MicRefusal::OsDenied`].
    pub fn with_device(self, device: impl Into<String>) -> Self {
        Self {
            device: Some(MicTarget::DeviceId(device.into())),
            ..self
        }
    }

    /// The same specification targeting a unique input by its display label. Discovery can
    /// return [`MicRefusal::OsDenied`] if OS permissions prevent checking a previously listed
    /// match, or [`MicRefusal::DeviceFailed`] if another discovery error prevents checking
    /// whether the label is unique. Select by [`MicDevice::id`] to target an independent input.
    pub fn with_label(self, label: impl Into<String>) -> Self {
        Self {
            device: Some(MicTarget::Label(label.into())),
            ..self
        }
    }
}

/// What the client found wrong with a [`MicSpec`]. This is a local validation result.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::MicSpecFault>();
/// ```
// no wire: microphone spec validation is local to the client
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MicSpecFault {
    /// `channels` is not 1 or 2.
    Channels,
    /// `rate` is outside [`MIC_RATE_MIN`] to [`MIC_RATE_MAX`].
    Rate,
}

impl std::fmt::Display for MicSpecFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Channels => "channels is not 1 or 2",
            Self::Rate => "rate is outside 8000 to 48000 Hz",
        })
    }
}

/// Why a request to list or open microphone capture was refused. This is a local result, not a
/// network error.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::MicRefusal>();
/// ```
// no wire: microphone refusals report local capture or host conditions
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MicRefusal {
    /// This is a server: it has no microphone.
    OnServer,
    /// Called outside `ClientSetup` or later, or the host has no client audio.
    Unavailable,
    /// The capture specification has an invalid `channels` or `rate`.
    BadSpec(MicSpecFault),
    /// No default input device exists, the selected input was not found or vanished, or client
    /// audio capture is disabled. A label that matches only output devices does not select a mic.
    NoDevice,
    /// More than one input device matches the requested selection; select a specific
    /// [`MicDevice::id`] from `microphone_devices()`.
    AmbiguousDevice,
    /// The operating system denied microphone access or prevented checking the requested input,
    /// for example by blocking access to the ALSA card of a previously listed input.
    OsDenied,
    /// The input device is busy or held in exclusive use.
    Busy,
    /// Audio input discovery failed, or the input device or capture worker failed to open or start.
    DeviceFailed,
}

impl std::fmt::Display for MicRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OnServer => f.write_str("a server has no microphone"),
            Self::Unavailable => f.write_str("microphone capture is unavailable in this phase"),
            Self::BadSpec(fault) => write!(f, "the microphone spec is refused: {fault}"),
            Self::NoDevice => f.write_str("no matching microphone device is available"),
            Self::AmbiguousDevice => {
                f.write_str("multiple microphone devices match the requested selection")
            }
            Self::OsDenied => f.write_str("the operating system denied microphone permission"),
            Self::Busy => f.write_str("the microphone device is busy"),
            Self::DeviceFailed => f.write_str("the microphone device failed to open or start"),
        }
    }
}

impl std::error::Error for MicRefusal {}

/// The engine's side of one open microphone capture stream. A mod reaches it through [`MicRef`],
/// not directly. Reads work from any thread without blocking on a tick or OS callback.
// no wire: engine-side capture source reached in-process through MicRef
pub trait MicSource: Send + Sync + 'static {
    /// Reads whole interleaved signed 16-bit PCM frames into `pcm` without blocking or waiting on
    /// a tick. Only validated whole frames are written: any suffix beyond `returned * channels`
    /// (including any trailing incomplete frame) is left untouched. Returns `0` immediately without
    /// blocking or modifying `pcm` when no frames are ready, after [`close`](Self::close), or if
    /// another thread is concurrently inside `read` on the same stream.
    fn read(&self, pcm: &mut [i16]) -> u32;
    /// Channel count of the captured PCM (`1` for mono or `2` for stereo).
    fn channels(&self) -> u8;
    /// Sample rate of the captured PCM, in Hz.
    fn rate(&self) -> u32;
    /// Cumulative captured frames dropped from the bounded [`MIC_RING_MAX_MS`] ms ring because
    /// the reader fell behind.
    fn dropped_frames(&self) -> u64;
    /// Whether capture is still active; `false` after [`close`](Self::close) or if the OS input
    /// device disconnects or errors (in which case the underlying OS stream is released promptly
    /// on its own teardown thread without waiting for `close`, drop, or a tick).
    fn is_open(&self) -> bool;
    /// Stops capture and waits for the stream's teardown thread to release the OS input device
    /// before returning. Idempotent; never waits behind unrelated device probes or opens.
    fn close(&self);
}

/// An owning handle to a live microphone capture stream on this client.
///
/// Read captured PCM frames from any thread with [`read`](Self::read) without waiting for a tick
/// or running on the OS audio callback. Calling [`close`](Self::close) or dropping the handle
/// stops capture and releases the OS input device before returning. If the OS device disconnects
/// or errors, [`is_open`](Self::is_open) flips to `false` and the failed OS stream is released
/// promptly without waiting for `close`, drop, or a tick.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::MicRef>();
/// ```
// no wire: the handle reads live PCM from a microphone on this client
// exhaustive: its field is private, so mods cannot build or destructure it
pub struct MicRef {
    source: Arc<dyn MicSource>,
}

impl MicRef {
    /// A handle on the engine's `source`.
    // hidden: a game-side constructor, not part of the mod surface
    #[doc(hidden)]
    pub fn new(source: Arc<dyn MicSource>) -> Self {
        Self { source }
    }

    /// Reads whole interleaved signed 16-bit PCM frames into `pcm` without blocking or waiting on
    /// a tick. Only validated whole frames are written: any suffix beyond `returned * channels`
    /// (including any trailing incomplete frame) is left untouched. Returns `0` immediately without
    /// blocking or modifying `pcm` when no frames are ready, after [`close`](Self::close), or if
    /// another thread is concurrently inside `read` on the same handle.
    pub fn read(&self, pcm: &mut [i16]) -> u32 {
        self.source.read(pcm)
    }

    /// Channel count of the captured PCM (`1` for mono or `2` for stereo).
    pub fn channels(&self) -> u8 {
        self.source.channels()
    }

    /// Sample rate of the captured PCM, in Hz.
    pub fn rate(&self) -> u32 {
        self.source.rate()
    }

    /// Cumulative captured frames dropped from the bounded [`MIC_RING_MAX_MS`] ms ring because
    /// the reader fell behind.
    pub fn dropped_frames(&self) -> u64 {
        self.source.dropped_frames()
    }

    /// Whether capture is still active; `false` after [`close`](Self::close) or if the OS input
    /// device disconnects or errors (the failed OS stream is released promptly without waiting for
    /// `close`, drop, or a tick).
    pub fn is_open(&self) -> bool {
        self.source.is_open()
    }

    /// Stops capture and waits for the stream's teardown thread to release the OS input device
    /// before returning. Idempotent, and also called automatically on drop.
    pub fn close(&self) {
        self.source.close();
    }
}

impl Drop for MicRef {
    fn drop(&mut self) {
        self.source.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    };

    struct StubMicSource {
        channels: u8,
        rate: u32,
        dropped: AtomicU64,
        open: AtomicBool,
        closes: AtomicU32,
        samples: Mutex<Vec<i16>>,
    }

    impl StubMicSource {
        fn new(channels: u8, rate: u32, dropped: u64, samples: Vec<i16>) -> Self {
            Self {
                channels,
                rate,
                dropped: AtomicU64::new(dropped),
                open: AtomicBool::new(true),
                closes: AtomicU32::new(0),
                samples: Mutex::new(samples),
            }
        }
    }

    impl MicSource for StubMicSource {
        fn read(&self, pcm: &mut [i16]) -> u32 {
            if !self.open.load(Ordering::Acquire) {
                return 0;
            }
            let channels = usize::from(self.channels);
            if channels == 0 {
                return 0;
            }
            let mut queued = self.samples.lock().unwrap();
            let frames = (pcm.len() / channels).min(queued.len() / channels);
            let sample_count = frames * channels;
            for (dst, src) in pcm[..sample_count]
                .iter_mut()
                .zip(queued.drain(..sample_count))
            {
                *dst = src;
            }
            frames as u32
        }

        fn channels(&self) -> u8 {
            self.channels
        }

        fn rate(&self) -> u32 {
            self.rate
        }

        fn dropped_frames(&self) -> u64 {
            self.dropped.load(Ordering::Relaxed)
        }

        fn is_open(&self) -> bool {
            self.open.load(Ordering::Acquire)
        }

        fn close(&self) {
            self.open.store(false, Ordering::Release);
            self.closes.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn mic_device_and_spec_new_and_with_device() {
        let dev = MicDevice::new("usb:1".into(), "USB Microphone".into());
        assert_eq!(dev.id, "usb:1");
        assert_eq!(dev.label, "USB Microphone");

        let default_spec = MicSpec::new(1, 48_000);
        assert_eq!(default_spec.device, None);
        assert_eq!(default_spec.channels, 1);
        assert_eq!(default_spec.rate, 48_000);

        let named_spec = default_spec.with_device(&dev.id);
        assert_eq!(named_spec.device, Some(MicTarget::DeviceId("usb:1".into())));
        assert_eq!(named_spec.channels, 1);
        assert_eq!(named_spec.rate, 48_000);
    }

    #[test]
    fn mic_refusal_display() {
        assert_eq!(MicSpecFault::Channels.to_string(), "channels is not 1 or 2");
        assert_eq!(
            MicSpecFault::Rate.to_string(),
            "rate is outside 8000 to 48000 Hz"
        );
        assert_eq!(
            MicRefusal::OnServer.to_string(),
            "a server has no microphone"
        );
        assert_eq!(
            MicRefusal::Unavailable.to_string(),
            "microphone capture is unavailable in this phase"
        );
        assert_eq!(
            MicRefusal::BadSpec(MicSpecFault::Channels).to_string(),
            "the microphone spec is refused: channels is not 1 or 2"
        );
        assert_eq!(
            MicRefusal::BadSpec(MicSpecFault::Rate).to_string(),
            "the microphone spec is refused: rate is outside 8000 to 48000 Hz"
        );
        assert_eq!(
            MicRefusal::NoDevice.to_string(),
            "no matching microphone device is available"
        );
        assert_eq!(
            MicRefusal::AmbiguousDevice.to_string(),
            "multiple microphone devices match the requested selection"
        );
        assert_eq!(
            MicRefusal::OsDenied.to_string(),
            "the operating system denied microphone permission"
        );
        assert_eq!(
            MicRefusal::Busy.to_string(),
            "the microphone device is busy"
        );
        assert_eq!(
            MicRefusal::DeviceFailed.to_string(),
            "the microphone device failed to open or start"
        );
    }

    #[test]
    fn mic_ref_forwards_and_closes_on_drop() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<MicRef>();

        let source = Arc::new(StubMicSource::new(2, 16_000, 12, vec![10, -10, 20, -20]));
        let handle = Arc::<StubMicSource>::clone(&source);
        let mic = MicRef::new(handle);

        assert_eq!(mic.channels(), 2);
        assert_eq!(mic.rate(), 16_000);
        assert_eq!(mic.dropped_frames(), 12);
        assert!(mic.is_open());

        let mut pcm = [99_i16; 5];
        assert_eq!(mic.read(&mut pcm), 2);
        assert_eq!(pcm, [10, -10, 20, -20, 99]);
        assert_eq!(mic.read(&mut pcm), 0);

        mic.close();
        assert!(!mic.is_open());
        assert_eq!(source.closes.load(Ordering::Relaxed), 1);

        drop(mic);
        assert_eq!(source.closes.load(Ordering::Relaxed), 2);

        let drop_source = Arc::new(StubMicSource::new(1, 48_000, 0, vec![5]));
        let drop_handle = Arc::<StubMicSource>::clone(&drop_source);
        let drop_mic = MicRef::new(drop_handle);
        assert!(drop_mic.is_open());
        drop(drop_mic);
        assert!(!drop_source.is_open());
        assert_eq!(drop_source.closes.load(Ordering::Relaxed), 1);
    }
}
