//! A client mod's PCM streams: the mixer's worker pulls from a native [`AudioStream`] on its
//! own pool, never vanilla's streaming slots.
use super::types::SoundSource;
use crate::{ClientHost, EntityRef};
use std::sync::Arc;

/// The most streams one mod holds open at once ([`AudioRefusal::ModStreamLimit`]).
pub const AUDIO_STREAMS_PER_MOD: u32 = 256;
/// The most streams open at once across every mod ([`AudioRefusal::TooManyStreams`]).
pub const AUDIO_STREAMS_TOTAL: u32 = 1_024;

/// A native source of PCM owned by the mixer. Its worker calls [`read`](Self::read) serially
/// on demand, never in the device callback or while holding mixer or pool locks.
///
/// The source runs in-process, with no `Host` or world borrow and no reentry into the game.
/// Reads must do bounded, nonblocking work: do not wait on I/O, locks, another thread or a tick.
/// An async or network producer bridges through its own nonblocking buffer read here. Muting the
/// stream does not pause demand. The worker drops an admitted source at EOF or after stop,
/// without waiting for another device callback. A panic in [`read`](Self::read) immediately
/// stops this stream and its source callback, and the shared log and error toast name the mod.
// no wire: the mixer owns and calls this native source in-process.
pub trait AudioStream: Send + 'static {
    /// Fill at most `pcm.len() / channels` whole frames of interleaved signed 16-bit PCM in the
    /// stream's [`StreamSpec`] format. `pcm` is zeroed before each call.
    ///
    /// - Returning `0` signals EOF: already-queued frames drain, then the stream stops.
    /// - Returning `1..=pcm.len() / channels` supplies that many whole interleaved frames;
    ///   return `n > 0` without writing to `pcm` for temporary silence.
    /// - Returning `> pcm.len() / channels` stops the stream immediately without publishing.
    fn read(&mut self, pcm: &mut [i16]) -> usize;
}

/// Where a stream is heard. Positions are in the client's current level.
/// This is local playback state. Send the emitter's network entity ID in your own payload,
/// and resolve it to a local handle with `host.entity(id)` before constructing `Entity`.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::StreamPlacement>();
/// ```
// exhaustive: mods build every form and the mixer matches them without a wildcard
// no wire: placement can contain a client-local entity handle
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StreamPlacement {
    /// At the listener, wherever they move, with no panning and no fading with distance.
    Flat,
    /// At a point, in blocks: mono streams are panned and faded by `range` as a positioned sound
    /// is; stereo streams play un-spatialized without panning or distance attenuation (matching
    /// OpenAL).
    At([f64; 3]),
    /// At an entity, followed each frame. The entity must be tracked when opening the stream or
    /// calling [`AudioStreamRef::set_entity`]; if the entity becomes untracked, unloaded or
    /// removed, the stream stops (matching vanilla `EntityBoundSoundInstance.tick`), even while
    /// mod audio is muted. Stereo streams play un-spatialized.
    Entity(EntityRef),
}

/// What an audio stream reads (`AudioStreamRef::state`). The state describes this client's mixer,
/// not the sender of the audio, and is not a payload type.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::AudioStreamState>();
/// ```
// no wire: playback state is local to the mixer
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AudioStreamState {
    /// Open and playing, even if silent because it is muted or has no PCM ready.
    Playing,
    /// Over: the pull source ended and its PCM drained, it returned an invalid frame count or
    /// panicked, its followed entity became untracked or removed, the listener changed dimension
    /// or left the level, `stop` was called, the handle was dropped, or sound output went away.
    /// It never plays again.
    #[default]
    Stopped,
}

/// The format, placement and volume used to open an audio stream. Construct this on the playing
/// client rather than sending it as a payload.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::StreamSpec>();
/// ```
// no wire: stream setup includes local playback placement
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct StreamSpec {
    /// The mixer group, scaled by master times the group, as every sound is. Voice chat plays on
    /// [`SoundSource::Voice`], the "Voice/Speech" slider.
    pub category: SoundSource,
    /// 1 or 2. Both mono and stereo streams accept any [`StreamPlacement`]; mono streams are
    /// spatialized, while stereo streams play un-spatialized without panning or distance
    /// attenuation (matching OpenAL).
    pub channels: u8,
    /// The rate of the source PCM, in Hz; any positive rate is accepted. The mixer resamples to
    /// the output's.
    pub rate: u32,
    /// Where it is heard.
    pub placement: StreamPlacement,
    /// For a positioned mono stream, the distance in blocks at which it has faded to silence, in
    /// a straight line (`AL_LINEAR_DISTANCE`, as a sound's range). Positive and finite.
    pub range: f32,
    /// The stream's own volume, clamped to `0.0..=1.0` (only NaN is refused), before the group's
    /// and the master's.
    pub volume: f32,
}

impl StreamSpec {
    /// A flat stream on `category`: `channels` (1 or 2) at `rate` Hz, range 16 blocks (a sound's
    /// default) and volume 1.
    pub fn new(category: SoundSource, channels: u8, rate: u32) -> Self {
        Self {
            category,
            channels,
            rate,
            placement: StreamPlacement::Flat,
            range: 16.0,
            volume: 1.0,
        }
    }
    /// The same stream at `placement`.
    pub fn with_placement(self, placement: StreamPlacement) -> Self {
        Self { placement, ..self }
    }
    /// The same stream fading out at `range` blocks.
    pub fn with_range(self, range: f32) -> Self {
        Self { range, ..self }
    }
    /// The same stream at `volume`.
    pub fn with_volume(self, volume: f32) -> Self {
        Self { volume, ..self }
    }
}

/// What the playing client found wrong with a [`StreamSpec`]. This is a local validation result.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::SpecFault>();
/// ```
// no wire: spec faults report local validation
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpecFault {
    /// `channels` is not 1 or 2.
    Channels,
    /// `rate` is zero.
    Rate,
    /// `volume` is NaN; finite values outside `0.0..=1.0` are clamped.
    Volume,
    /// `range` is not a positive, finite number of blocks.
    Range,
    /// A point that is not finite, or an entity that the client does not currently track.
    Position,
}

/// Why a request to open an audio stream opened nothing. This is a local result, not a network error.
///
/// ```compile_fail,E0277
/// fn requires_wire<T: mod_api::wire::Wire>() {}
/// requires_wire::<mod_api::AudioRefusal>();
/// ```
// no wire: stream refusals report local mixer or host conditions
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AudioRefusal {
    /// This is a server: it has no output to play on.
    OnServer,
    /// The call has no owning mod.
    Unavailable,
    /// The spec has a value out of range.
    BadSpec(SpecFault),
    /// The game has no sound output (no device, or sound off).
    NoOutput,
    /// The calling mod already holds [`AUDIO_STREAMS_PER_MOD`] streams: stop one first.
    ModStreamLimit,
    /// [`AUDIO_STREAMS_TOTAL`] streams are open across every mod: stop one first.
    TooManyStreams,
    /// The mod's or the pool's PCM buffer memory budget is exhausted, or ring allocation failed.
    /// Stopped streams still count while the worker is disposing of their source and ring.
    MemoryLimit,
    /// The pull worker could not start or admit this source.
    WorkerUnavailable,
}
impl std::fmt::Display for AudioRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OnServer => f.write_str("a server has no sound output"),
            Self::Unavailable => f.write_str("an audio stream must belong to a mod"),
            Self::BadSpec(fault) => write!(f, "the stream spec is refused: {fault}"),
            Self::NoOutput => f.write_str("the game has no sound output"),
            Self::ModStreamLimit => write!(
                f,
                "this mod already holds {AUDIO_STREAMS_PER_MOD} audio streams"
            ),
            Self::TooManyStreams => {
                write!(f, "{AUDIO_STREAMS_TOTAL} audio streams are already open")
            }
            Self::MemoryLimit => {
                f.write_str("the mod or audio stream pool cannot allocate another PCM buffer")
            }
            Self::WorkerUnavailable => f.write_str("the audio pull worker cannot accept a source"),
        }
    }
}
impl std::error::Error for AudioRefusal {}
impl std::fmt::Display for SpecFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Channels => "channels is not 1 or 2",
            Self::Rate => "rate must be greater than zero",
            Self::Volume => "volume is NaN",
            Self::Range => "range is not a positive, finite number of blocks",
            Self::Position => "the position is not finite or the entity is not tracked",
        })
    }
}

/// The game's side of one open audio stream. A mod reaches it through [`AudioStreamRef`], not
/// directly. Every method works from any thread and never waits on the mixer.
pub trait AudioStreamSource: Send + Sync + 'static {
    /// Moves the stream; false, changing nothing, for a position that is not finite, for an
    /// `Entity` placement with no finite `entity_pos`, and once the stream is stopped.
    fn set_placement(&self, placement: &StreamPlacement, entity_pos: Option<[f64; 3]>) -> bool;
    /// Sets the stream's own volume (clamped to `0.0..=1.0`); false, changing nothing, for NaN
    /// and once the stream is stopped.
    fn set_volume(&self, volume: f32) -> bool;
    /// Whether the stream is playing or stopped, regardless of queued PCM.
    fn state(&self) -> AudioStreamState;
    /// Ends the stream for good: it is silent from the next output buffer and its audio is
    /// dropped. Dropping the last handle does the same.
    fn stop(&self);
}

/// The controls for an open mod audio stream.
///
/// Keep the handle while the stream should play. Dropping it stops the stream, as [`stop`]
/// does. [`set_flat`], [`set_position`], [`set_volume`], [`state`] and [`stop`] work from any
/// thread, including a `Network`-thread payload handler, without a [`ClientHost`] and without
/// waiting on the mixer or a client tick. [`set_entity`] takes a [`ClientHost`] on the client
/// thread to resolve entity placement eagerly.
///
/// [`set_flat`]: AudioStreamRef::set_flat
/// [`set_position`]: AudioStreamRef::set_position
/// [`set_entity`]: AudioStreamRef::set_entity
/// [`set_volume`]: AudioStreamRef::set_volume
/// [`state`]: AudioStreamRef::state
/// [`stop`]: AudioStreamRef::stop
// no wire: the handle controls a stream on this client's mixer.
// exhaustive: its field is private, so mods cannot build or destructure it
pub struct AudioStreamRef {
    source: Arc<dyn AudioStreamSource>,
}

impl AudioStreamRef {
    /// A handle on the game's `source`.
    // hidden: a game-side constructor, not part of the mod surface
    #[doc(hidden)]
    pub fn new(source: Arc<dyn AudioStreamSource>) -> Self {
        Self { source }
    }
    /// Moves the stream to [`StreamPlacement::Flat`] (at the listener, with no panning or
    /// distance fade). Callable from any thread without a [`ClientHost`] and never waits on the
    /// mixer. Publishes the placement immediately; false once the stream has stopped.
    pub fn set_flat(&self) -> bool {
        self.source.set_placement(&StreamPlacement::Flat, None)
    }
    /// Moves the stream to [`StreamPlacement::At(at)`]. Callable from any thread without a
    /// [`ClientHost`] and never waits on the mixer. False, changing nothing, when any coordinate
    /// is not finite or once the stream has stopped. Stereo streams accept any finite position
    /// and play un-spatialized. A successful call publishes the placement immediately.
    pub fn set_position(&self, at: [f64; 3]) -> bool {
        self.source.set_placement(&StreamPlacement::At(at), None)
    }
    /// Moves the stream to follow `entity` on the client thread, resolving its position eagerly
    /// through `host`. False, changing nothing, when the client does not currently track
    /// `entity`, its position is not finite, or once the stream has stopped. Stereo streams
    /// accept any tracked entity and play un-spatialized. A successful call publishes the
    /// placement immediately, including in a client executor task.
    ///
    /// In `client_setup`, use `if let Some(executor) = host.client_executor()` to obtain a
    /// [`ClientExecutor`](crate::ClientExecutor), then clone it into a network-thread
    /// producer holding an `EntityRef`.
    /// The task runs after the packet phase, so even a removal or level change published
    /// during packet handling can make the handle stale. Retarget without borrowing a host
    /// or a live payload context:
    ///
    /// ```
    /// # use mod_api::{AudioStreamRef, ClientExecutor, EntityRef};
    /// # use std::sync::Arc;
    /// # fn retarget(executor: &ClientExecutor, stream: Arc<AudioStreamRef>, entity: EntityRef) {
    /// let executor = executor.clone();
    /// executor.execute(Box::new(move |host| {
    ///     if !stream.set_entity(host, entity) {
    ///         eprintln!("Audio retarget refused: stream stopped or entity unavailable");
    ///     }
    /// }));
    /// # }
    /// ```
    pub fn set_entity(&self, host: &dyn ClientHost, entity: EntityRef) -> bool {
        let Some(at) = host
            .backend()
            .and_then(|b| b.open().entity_position(entity))
        else {
            return false;
        };
        self.source
            .set_placement(&StreamPlacement::Entity(entity), Some(at))
    }
    /// Sets the stream's own volume (clamped to `0.0..=1.0`), from the next output buffer. False,
    /// changing nothing, for NaN and once the stream has stopped.
    pub fn set_volume(&self, volume: f32) -> bool {
        self.source.set_volume(volume)
    }
    /// Whether the stream is playing or stopped. A dry or muted stream is still
    /// [`AudioStreamState::Playing`] while open.
    pub fn state(&self) -> AudioStreamState {
        self.source.state()
    }
    /// Ends the stream for good. It happens on drop too.
    pub fn stop(&self) {
        self.source.stop()
    }
}
