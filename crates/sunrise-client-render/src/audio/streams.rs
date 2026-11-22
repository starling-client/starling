//! Mods' audio streams have their own pool (never vanilla's streaming slots). Each fixed PCM
//! ring is filled by the pool's pull worker and drained by the mixer.
//!
//! - A mod thread sets volume, sets placement, reads state, or stops without taking the mixer lock.
//! - The callback reads PCM and the latest placement (a seqlock) and requests more pull PCM
//!   for the next buffer. It does not allocate or call the source.
//! - The main thread opens, caps, follows entities, stops and reaps streams. It allocates
//!   rings before locking the mixer.
//! - The worker owns, calls and drops admitted native sources without taking either lock.
mod pull;

use super::{Basis, Mixer, distance_gain};
use mod_api::{
    AUDIO_STREAMS_PER_MOD, AUDIO_STREAMS_TOTAL, AudioRefusal, AudioStream, AudioStreamSource,
    AudioStreamState, EntityRef, SpecFault, StreamPlacement, StreamSpec,
};
use pull::{Demand, OwnedSource, PullJob, Worker};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::*, fence},
};

/// Internal capacity of the worker-to-callback PCM ring, in milliseconds at the source rate.
const RING_MAX_MS: u64 = 1_000;
/// Minimum ring capacity in frames so the resampler has room for a frame and lookahead.
const MIN_RING_FRAMES: u64 = 2;
/// Maximum ring capacity in frames so one extreme sample rate cannot blow the ring budget alone.
const MAX_RING_FRAMES: u64 = 384_000;
/// Aggregate PCM buffer memory cap across all live voices and pending worker jobs (64 MiB).
const MAX_POOL_PCM_BYTES: usize = 64 * 1024 * 1024;
/// One mod cannot exhaust the shared PCM budget, even with jobs awaiting retirement.
const MAX_MOD_PCM_BYTES: usize = MAX_POOL_PCM_BYTES / 2;

fn try_reserve_pcm_bytes(counter: &AtomicUsize, limit: usize, bytes: usize) -> bool {
    let mut current = counter.load(Acquire);
    loop {
        let Some(next) = current.checked_add(bytes) else {
            return false;
        };
        if next > limit {
            return false;
        }
        match counter.compare_exchange_weak(current, next, AcqRel, Acquire) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
}

fn pcm(sample: i16) -> f32 {
    f32::from(sample) / 32768.0
}

/// A stream's samples: written by one producer, read by the output callback. Use f32 bits
/// so interpolation on the worker doesn't quantize a downsampled stream back to i16.
pub(super) struct Ring {
    samples: Vec<AtomicU32>,
    /// Samples written and consumed since the stream opened; the index is the count modulo the
    /// length. Only the producer moves `tail`, only the callback moves `head`.
    head: AtomicU64,
    tail: AtomicU64,
    channels: usize,
    pool_bytes: Arc<AtomicUsize>,
    bytes: usize,
}

impl Drop for Ring {
    fn drop(&mut self) {
        drop(std::mem::take(&mut self.samples));
        self.pool_bytes.fetch_sub(self.bytes, Release);
    }
}

impl Ring {
    fn try_new(
        frames: usize,
        channels: usize,
        pool_bytes: Arc<AtomicUsize>,
        limit: usize,
    ) -> Option<Self> {
        let sample_count = frames.checked_mul(channels)?;
        let bytes = sample_count.checked_mul(std::mem::size_of::<AtomicU32>())?;
        if !try_reserve_pcm_bytes(&pool_bytes, limit, bytes) {
            return None;
        }
        let mut samples = Vec::new();
        if samples.try_reserve_exact(sample_count).is_err() {
            pool_bytes.fetch_sub(bytes, Release);
            return None;
        }
        samples.resize_with(sample_count, || AtomicU32::new(0));
        Some(Self {
            samples,
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            channels,
            pool_bytes,
            bytes,
        })
    }

    /// The producer's tail (in samples), queued frames and free frames, in that order.
    fn producer_window(&self) -> (u64, usize, usize) {
        let tail = self.tail.load(Relaxed);
        let queued = tail - self.head.load(Acquire);
        let free = self.samples.len() as u64 - queued;
        let channels = self.channels as u64;
        (
            tail,
            (queued / channels) as usize,
            (free / channels) as usize,
        )
    }

    /// Write whole frames that the producer has checked fit in its window. No other producer
    /// may write between taking the window and publishing its samples.
    fn publish_frames(&self, mut tail: u64, samples: impl IntoIterator<Item = f32>) {
        let len = self.samples.len() as u64;
        for sample in samples {
            self.samples[(tail % len) as usize].store(sample.to_bits(), Relaxed);
            tail += 1;
        }
        self.tail.store(tail, Release);
    }
}

/// Where a stream was last placed, as writers see it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Held {
    Flat,
    At,
    Entity(EntityRef),
}

/// What the callback needs of a stream's placement, one consistent snapshot.
#[derive(Clone, Copy, Default)]
struct View {
    positional: bool,
    at: [f64; 3],
}

/// A stream's state shared by its handle, the pool and its voice.
pub(super) struct Control {
    rate: u32,
    /// The stream's `SoundSource` index.
    category: usize,
    range: f32,
    owner: &'static str,
    /// Charges this ring's bytes to its owner until the voice and job have both retired.
    pcm_bytes: usize,
    volume: AtomicU32,
    closed: AtomicBool,
    /// Set by `PullJob`'s retirement guard once the worker has dropped the source and ring.
    /// Quota accounting retains this control until both `closed` and `job_retired` hold.
    job_retired: AtomicBool,
    /// Set by `StreamPool::reap` once this stopped control's voice has been scheduled for removal
    /// from `mixer.streams`.
    voice_reaped: AtomicBool,
    pull: Arc<Demand>,
    /// The placement as set. Writers (a mod's `set_placement`, the frame's entity follow)
    /// take it for the whole of a change, so the newest always wins; the callback never does.
    held: Mutex<Held>,
    /// The placement the callback reads, published under `held` as a seqlock: odd while a
    /// writer is in the middle.
    sequence: AtomicU64,
    positional: AtomicBool,
    position: [AtomicU64; 3],
}

impl Control {
    fn stop(&self) {
        if !self.closed.swap(true, AcqRel) {
            self.pull.wake.signal();
        }
    }

    fn publish(&self, _held: &MutexGuard<'_, Held>, view: View) {
        let start = self.sequence.load(Relaxed);
        self.sequence.store(start + 1, Relaxed);
        fence(Release);
        for (cell, v) in self.position.iter().zip(view.at) {
            cell.store(v.to_bits(), Relaxed);
        }
        self.positional.store(view.positional, Relaxed);
        self.sequence.store(start + 2, Release);
    }
    /// The placement as the last writer left it, or none when a writer was in the middle of a
    /// change all four tries (the callback then keeps what it last had).
    fn view(&self) -> Option<View> {
        for _ in 0..4 {
            let before = self.sequence.load(Acquire);
            if before % 2 == 1 {
                continue;
            }
            let at = [0, 1, 2].map(|i| f64::from_bits(self.position[i].load(Relaxed)));
            let positional = self.positional.load(Relaxed);
            fence(Acquire);
            if self.sequence.load(Relaxed) == before {
                return Some(View { positional, at });
            }
        }
        None
    }
    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn place(&self, mut held: MutexGuard<'_, Held>, placement: Held, view: View) -> bool {
        if self.closed.load(Acquire) {
            return false;
        }
        *held = placement;
        self.publish(&held, view);
        true
    }
    /// Places an entity-following stream at its entity's position as `resolve` finds it now,
    /// under the lock a `set_placement` takes: an older resolution can never overwrite a newer
    /// placement. Stops the stream when the entity is no longer tracked or has a non-finite
    /// position (matching vanilla `EntityBoundSoundInstance.tick`).
    fn follow(&self, resolve: &mut impl FnMut(EntityRef) -> Option<[f64; 3]>) {
        if self.closed.load(Acquire) {
            return;
        }
        let held = self.lock();
        let Held::Entity(entity) = *held else {
            return;
        };
        match resolve(entity).filter(|at| at.iter().all(|c| c.is_finite())) {
            Some(at) => self.publish(
                &held,
                View {
                    positional: true,
                    at,
                },
            ),
            None => self.stop(),
        }
    }
}

fn check_placement(
    placement: &StreamPlacement,
    entity_pos: Option<[f64; 3]>,
) -> Result<(Held, View), SpecFault> {
    let (held, at) = match *placement {
        StreamPlacement::Flat => return Ok((Held::Flat, View::default())),
        StreamPlacement::At(at) => (Held::At, at),
        StreamPlacement::Entity(entity) => {
            (Held::Entity(entity), entity_pos.ok_or(SpecFault::Position)?)
        }
    };
    if !at.iter().all(|v| v.is_finite()) {
        return Err(SpecFault::Position);
    }
    Ok((
        held,
        View {
            positional: true,
            at,
        },
    ))
}

/// A mod's end of a stream: what `AudioStreamRef` calls. Dropping it stops the stream.
pub(super) struct StreamHandle {
    control: Arc<Control>,
}

impl AudioStreamSource for StreamHandle {
    fn set_placement(&self, placement: &StreamPlacement, entity_pos: Option<[f64; 3]>) -> bool {
        let c = &*self.control;
        let Ok((placement, view)) = check_placement(placement, entity_pos) else {
            return false;
        };
        c.place(c.lock(), placement, view)
    }
    fn set_volume(&self, volume: f32) -> bool {
        if self.control.closed.load(Acquire) || volume.is_nan() {
            return false;
        }
        self.control
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Relaxed);
        true
    }
    fn state(&self) -> AudioStreamState {
        if self.control.closed.load(Acquire) {
            AudioStreamState::Stopped
        } else {
            AudioStreamState::Playing
        }
    }
    fn stop(&self) {
        self.control.stop();
    }
}
impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The mixer's end of a stream: its ring, and where the resampler is.
pub(super) struct StreamVoice {
    control: Arc<Control>,
    ring: Arc<Ring>,
    /// True once there is a current source frame; cleared when the next required frame is missing.
    primed: bool,
    /// The last frame taken from the ring and the output phase relative to it, in source frames.
    current: [f32; 2],
    fraction: f64,
    /// The placement the callback last read whole.
    view: View,
}

/// The main thread's list of open streams, with the caps. Shared by every `Audio` on one output.
pub(super) struct StreamPool {
    state: Mutex<PoolState>,
    /// How many controls there are (live or awaiting worker retirement), so that with no stream
    /// open or pending the main thread's calls and the tick take no lock at all.
    open: AtomicUsize,
    /// Aggregate PCM bytes held across all live voices and unretired worker jobs.
    pcm_bytes: Arc<AtomicUsize>,
    /// Aggregate PCM byte cap across the pool.
    max_pcm_bytes: usize,
}

impl Default for StreamPool {
    fn default() -> Self {
        Self::with_pcm_budget(MAX_POOL_PCM_BYTES)
    }
}

#[derive(Default)]
struct PoolState {
    controls: Vec<Arc<Control>>,
    worker: Option<Worker>,
    /// The output went away (a failed reopen): no stream opens until it is back.
    lost: bool,
}

impl PoolState {
    fn close_all(&self) {
        for control in &self.controls {
            control.closed.store(true, Release);
        }
    }
}

impl Drop for StreamPool {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        state.close_all();
        if let Some(worker) = state.worker.take() {
            worker.shutdown();
        }
    }
}

impl StreamPool {
    fn with_pcm_budget(max_pcm_bytes: usize) -> Self {
        Self {
            state: Mutex::new(PoolState::default()),
            open: AtomicUsize::new(0),
            pcm_bytes: Arc::new(AtomicUsize::new(0)),
            max_pcm_bytes,
        }
    }

    fn state(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether no stream is open: the lock-free test every call that has nothing to do starts with.
    fn idle(&self) -> bool {
        self.open.load(Relaxed) == 0
    }

    /// Opens a stream for `owner`, a mod. The ring is allocated here, before the mixer is
    /// locked, once.
    pub(super) fn open(
        &self,
        mixer: &Mutex<Mixer>,
        owner: &'static str,
        spec: &StreamSpec,
        entity_pos: Option<[f64; 3]>,
        source: Box<dyn AudioStream>,
    ) -> Result<Arc<dyn AudioStreamSource>, AudioRefusal> {
        let source = OwnedSource::new(owner, source);
        let fault = |fault| Err(AudioRefusal::BadSpec(fault));
        if !matches!(spec.channels, 1 | 2) {
            return fault(SpecFault::Channels);
        }
        if spec.rate == 0 {
            return fault(SpecFault::Rate);
        }
        if spec.volume.is_nan() {
            return fault(SpecFault::Volume);
        }
        if !(spec.range.is_finite() && spec.range > 0.0) {
            return fault(SpecFault::Range);
        }
        let (placement, view) =
            check_placement(&spec.placement, entity_pos).map_err(AudioRefusal::BadSpec)?;
        self.reap(mixer);
        let mut state = self.state();
        if state.lost {
            return Err(AudioRefusal::NoOutput);
        }
        if state.controls.len() >= AUDIO_STREAMS_TOTAL as usize {
            drop(state);
            log::warn!("mod `{owner}` hit total audio stream limit ({AUDIO_STREAMS_TOTAL})");
            return Err(AudioRefusal::TooManyStreams);
        }
        let (owned, owned_bytes) = state
            .controls
            .iter()
            .filter(|c| c.owner == owner)
            .fold((0, 0), |(count, bytes), c| (count + 1, bytes + c.pcm_bytes));
        if owned >= AUDIO_STREAMS_PER_MOD as usize {
            drop(state);
            log::warn!("mod `{owner}` hit per-mod audio stream limit ({AUDIO_STREAMS_PER_MOD})");
            return Err(AudioRefusal::ModStreamLimit);
        }
        let channels = spec.channels as usize;
        let ring_frames = (u64::from(spec.rate) * RING_MAX_MS / 1000)
            .clamp(MIN_RING_FRAMES, MAX_RING_FRAMES) as usize;
        let requested = ring_frames * channels * std::mem::size_of::<AtomicU32>();
        if owned_bytes + requested > MAX_MOD_PCM_BYTES {
            drop(state);
            log::warn!(
                "mod `{owner}` hit its audio stream PCM memory limit \
                 (requested {requested} bytes, used {owned_bytes}/{MAX_MOD_PCM_BYTES} bytes)"
            );
            return Err(AudioRefusal::MemoryLimit);
        }
        let Some(ring) = Ring::try_new(
            ring_frames,
            channels,
            self.pcm_bytes.clone(),
            self.max_pcm_bytes,
        ) else {
            let used = self.pcm_bytes.load(Relaxed);
            let max_pcm_bytes = self.max_pcm_bytes;
            drop(state);
            log::warn!(
                "mod `{owner}` could not open audio stream: PCM buffer memory limit or allocation \
                 failure (requested {requested} bytes, used {used}/{max_pcm_bytes} bytes)"
            );
            return Err(AudioRefusal::MemoryLimit);
        };
        let ring = Arc::new(ring);
        let worker = match &mut state.worker {
            Some(worker) => worker,
            slot @ None => match Worker::start() {
                Ok(worker) => slot.insert(worker),
                Err(refusal) => {
                    drop(state);
                    log::warn!(
                        "mod `{owner}` could not open audio stream: pull worker failed to start"
                    );
                    return Err(refusal);
                }
            },
        };
        let demand = Arc::new(worker.demand());
        let control = Arc::new(Control {
            rate: spec.rate,
            category: spec.category.index(),
            range: spec.range,
            owner,
            pcm_bytes: ring.bytes,
            volume: AtomicU32::new(spec.volume.clamp(0.0, 1.0).to_bits()),
            closed: AtomicBool::new(false),
            job_retired: AtomicBool::new(false),
            voice_reaped: AtomicBool::new(false),
            pull: demand.clone(),
            held: Mutex::new(placement),
            sequence: AtomicU64::new(0),
            positional: AtomicBool::new(view.positional),
            position: view.at.map(|at| AtomicU64::new(at.to_bits())),
        });
        if let Some(job) = worker.admit(PullJob::new(control.clone(), demand, ring.clone(), source))
        {
            drop(state);
            drop(job);
            log::warn!(
                "mod `{owner}` could not open audio stream: pull worker admission queue unavailable"
            );
            return Err(AudioRefusal::WorkerUnavailable);
        }
        // If the mixer cannot accept the voice, this handle closes the job on its worker.
        let handle = StreamHandle {
            control: control.clone(),
        };
        let voice = StreamVoice {
            control: control.clone(),
            ring,
            primed: false,
            current: [0.0; 2],
            fraction: 0.0,
            view,
        };
        match mixer.lock() {
            Ok(mut mixer) => mixer.streams.push(voice),
            Err(_) => return Err(AudioRefusal::NoOutput),
        }
        state.controls.push(control);
        self.open.store(state.controls.len(), Relaxed);
        Ok(Arc::new(handle))
    }

    /// Lets go of the streams a mod stopped or dropped: their voices leave the mixer right away,
    /// and their controls release their owner and total quota slots once the worker retires their
    /// `PullJob` and drops the last reference to their rings.
    pub(super) fn reap(&self, mixer: &Mutex<Mixer>) {
        if self.idle() {
            return;
        }
        let mut state = self.state();
        let before = state.controls.len();
        let mut need_mixer_reap = false;
        state.controls.retain(|c| {
            if !c.closed.load(Acquire) {
                return true;
            }
            if !c.voice_reaped.swap(true, AcqRel) {
                need_mixer_reap = true;
            }
            !c.job_retired.load(Acquire)
        });
        self.open.store(state.controls.len(), Relaxed);
        if need_mixer_reap {
            // The voices leave into a list made before the lock, with room for every voice there
            // was (a mod may stop another stream meanwhile): nothing allocates under the lock,
            // and the rings drop after it is released, never in the callback.
            let mut gone = Vec::with_capacity(before);
            if let Ok(mut mixer) = mixer.lock() {
                let mut i = 0;
                while i < mixer.streams.len() {
                    if mixer.streams[i].control.closed.load(Acquire) {
                        gone.push(mixer.streams.swap_remove(i));
                    } else {
                        i += 1;
                    }
                }
            }
            drop(gone);
        }
    }

    /// Follows each entity-placed stream's entity: where `resolve` finds it the stream is placed
    /// there, where it doesn't (untracked, unloaded, removed) the stream stops. Takes no
    /// lock the output callback takes.
    pub(super) fn follow_entities(&self, mut resolve: impl FnMut(EntityRef) -> Option<[f64; 3]>) {
        if self.idle() {
            return;
        }
        for control in &self.state().controls {
            control.follow(&mut resolve);
        }
    }

    /// Stops every open stream (level leave, dimension change, `SoundManager.stop`).
    pub(super) fn stop_all(&self) {
        if self.idle() {
            return;
        }
        let state = self.state();
        state.close_all();
        if let Some(worker) = &state.worker {
            worker.wake();
        }
    }

    /// The output went away (or came back): its streams stop, and none opens until it is back.
    #[cfg(any(feature = "audio", test))]
    pub(super) fn output_lost(&self, lost: bool) {
        let mut state = self.state();
        state.lost = lost;
        if lost {
            state.close_all();
            if let Some(worker) = &state.worker {
                worker.wake();
            }
        }
    }
}

/// After consuming a whole output buffer, ask for enough ring frames for the next one,
/// including interpolation's lookahead. One wake suffices for the pool's shared worker.
pub(super) fn request_pull(streams: &[StreamVoice], output_frames: usize, rate: u32) {
    if output_frames == 0 {
        return;
    }
    let mut worker = None;
    for voice in streams {
        let control = &voice.control;
        if !control.closed.load(Acquire) && !control.pull.eof.load(Acquire) {
            let mut ring_rate = control.pull.ring_rate.load(Relaxed);
            if ring_rate == 0 {
                // Pin the rate at first demand. If the output device's rate changes later,
                // the mixer resamples these same queued frames at the new ratio.
                ring_rate = control.rate.min(rate);
                control.pull.ring_rate.store(ring_rate, Relaxed);
            }
            let frames = (output_frames as u64)
                .saturating_mul(u64::from(ring_rate))
                .div_ceil(u64::from(rate))
                .saturating_add(1);
            let capacity = voice.ring.samples.len() / voice.ring.channels;
            control
                .pull
                .frames
                .store(frames.min(capacity as u64) as usize, Release);
            worker = Some(&control.pull.wake);
        }
    }
    if let Some(worker) = worker {
        worker.signal();
    }
}

/// Mixes every stream into `out` (the muffle's bus or the output), `channels` wide at `rate`.
/// `category_gain` is master times group by `SoundSource` index. Allocates nothing: the rings,
/// the voices and the cameras were all made before.
pub(super) fn mix(
    streams: &mut [StreamVoice],
    cameras: &[([f64; 3], Basis, u32)],
    category_gain: &[f32; 11],
    out: &mut [f32],
    channels: usize,
    rate: u32,
) {
    let output_frames = out.len() / channels;
    if output_frames == 0 {
        return;
    }
    let shared_listeners = cameras.len() > 1;
    for voice in streams.iter_mut() {
        let c = &*voice.control;
        if c.closed.load(Acquire) {
            continue;
        }
        let ring = &*voice.ring;
        let ch = ring.channels;
        let len = ring.samples.len() as u64;
        let head = ring.head.load(Relaxed);
        // EOF orders every prior publication. Read it before the tail so an EOF snapshot
        // cannot miss the producer's last frames.
        let eof = c.pull.eof.load(Acquire);
        let tail = ring.tail.load(Acquire);
        let mut avail = (tail - head) / ch as u64;
        let mut taken = 0u64;
        // The frame at this offset from the ring head at the start of the buffer.
        let frame_at = |frame: u64| -> [f32; 2] {
            let base = head + frame * ch as u64;
            let sample =
                |i: u64| f32::from_bits(ring.samples[((base + i) % len) as usize].load(Relaxed));
            let left = sample(0);
            [left, if ch > 1 { sample(1) } else { left }]
        };
        if !voice.primed {
            if avail == 0 {
                if eof {
                    c.stop();
                }
                continue;
            }
            voice.current = frame_at(0);
            voice.fraction = 0.0;
            taken = 1;
            avail -= 1;
            voice.primed = true;
        }
        let volume = f32::from_bits(c.volume.load(Relaxed)) * category_gain[c.category];
        // Gain per output channel, and the one a mono output takes (no panning); NaN anywhere
        // is silence. Skip reading placement if the stream has no volume.
        let [left, right, mono] = if volume == 0.0 {
            [0.0; 3]
        } else {
            // What the callback last read whole stands if a writer is in the middle of a change.
            if let Some(view) = c.view() {
                voice.view = view;
            }
            if !voice.view.positional || ch > 1 {
                [volume; 3]
            } else {
                let at = voice.view.at;
                let away = |camera: &([f64; 3], Basis, u32)| {
                    [
                        at[0] - camera.0[0],
                        at[1] - camera.0[1],
                        at[2] - camera.0[2],
                    ]
                };
                let sqr = |d: [f64; 3]| d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                match cameras
                    .iter()
                    .min_by(|a, b| sqr(away(a)).total_cmp(&sqr(away(b))))
                {
                    None => [0.0; 3],
                    Some(camera) => {
                        let d = away(camera);
                        let (forward, up, right) = camera.1;
                        let dot = |a: [f64; 3], b: [f64; 3]| {
                            (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) as f32
                        };
                        let distance = sqr(d).sqrt() as f32;
                        let gain = volume * distance_gain(Some(c.range), distance);
                        let [mut l, mut r] = super::stereo_gains_with_length(
                            dot(d, right),
                            dot(d, up),
                            dot(d, forward),
                            distance,
                        );
                        if shared_listeners {
                            let middle = (l + r) * 0.5;
                            l = middle + (l - middle) * 0.5;
                            r = middle + (r - middle) * 0.5;
                        }
                        [l * gain, r * gain, gain]
                    }
                }
            }
        };
        let [left, right, mono] = [left, right, mono].map(|g| if g.is_finite() { g } else { 0.0 });
        let ring_rate = c.pull.ring_rate.load(Relaxed);
        let step = f64::from(ring_rate) / f64::from(rate);
        if left == 0.0 && right == 0.0 && mono == 0.0 {
            // Inaudible (muted, no volume, out of range): no interpolation or mixing.
            // Advance the resampler and ring head as far as audible output would have taken them.
            let wanted = (voice.fraction + step * (output_frames - 1) as f64) as u64;
            let advance = wanted.min(avail);
            if advance > 0 {
                voice.current = frame_at(taken + advance - 1);
            }
            taken += advance;
            if wanted > advance {
                voice.primed = false;
            } else {
                voice.fraction += step * output_frames as f64 - wanted as f64;
            }
        } else {
            'output: for frame in out.chunks_exact_mut(channels) {
                // Look for a source frame only when the output needs it. If none is queued, leave
                // the rest silent and start again with the next frame that arrives.
                while voice.fraction >= 1.0 {
                    if avail == 0 {
                        voice.primed = false;
                        break 'output;
                    }
                    voice.current = frame_at(taken);
                    taken += 1;
                    avail -= 1;
                    voice.fraction -= 1.0;
                }
                // Interpolate between source frames. With no next frame yet, hold the last one
                // for the rest of its duration.
                let now = if voice.fraction > 0.0 && avail > 0 {
                    let next = frame_at(taken);
                    let t = voice.fraction as f32;
                    [
                        voice.current[0] + (next[0] - voice.current[0]) * t,
                        voice.current[1] + (next[1] - voice.current[1]) * t,
                    ]
                } else {
                    voice.current
                };
                if channels >= 2 {
                    frame[0] += now[0] * left;
                    frame[1] += now[1] * right;
                } else {
                    frame[0] += if ch > 1 {
                        (now[0] + now[1]) * 0.5
                    } else {
                        now[0]
                    } * mono;
                }
                voice.fraction += step;
            }
        }
        let head = head + taken * ch as u64;
        ring.head.store(head, Release);
        if eof && head == tail && (!voice.primed || voice.fraction >= 1.0) {
            c.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Audio;
    use super::super::{Mixer, mix as mix_output};
    use super::*;
    use mod_api::SoundSource;

    struct Silent;
    impl AudioStream for Silent {
        fn read(&mut self, _pcm: &mut [i16]) -> usize {
            1
        }
    }

    /// A mixer and the pool that feeds it, every group at full volume and the listener at the
    /// origin, facing +z.
    struct Rig {
        mixer: Arc<Mutex<Mixer>>,
        pool: StreamPool,
        receiver: Mutex<std::sync::mpsc::Receiver<PullJob>>,
        jobs: Mutex<Vec<PullJob>>,
    }
    impl Rig {
        fn new() -> Self {
            let mixer = Mixer {
                stream_gain: [1.0; 11],
                ..Mixer::default()
            };
            let pool = StreamPool::default();
            let (worker, receiver) = Worker::test_pair();
            pool.state().worker = Some(worker);
            Self {
                mixer: Arc::new(Mutex::new(mixer)),
                pool,
                receiver: Mutex::new(receiver),
                jobs: Mutex::new(Vec::new()),
            }
        }
        fn drain_jobs(&self) {
            let mut jobs = self.jobs.lock().unwrap();
            let receiver = self.receiver.lock().unwrap();
            while let Ok(job) = receiver.try_recv() {
                jobs.push(job);
            }
            jobs.retain(|job| !job.control.closed.load(Acquire));
        }
        fn open(
            &self,
            owner: &'static str,
            spec: &StreamSpec,
        ) -> Result<Arc<dyn AudioStreamSource>, AudioRefusal> {
            self.open_at_entity(owner, spec, None)
        }
        fn open_at_entity(
            &self,
            owner: &'static str,
            spec: &StreamSpec,
            entity_pos: Option<[f64; 3]>,
        ) -> Result<Arc<dyn AudioStreamSource>, AudioRefusal> {
            self.drain_jobs();
            let stream = self
                .pool
                .open(&self.mixer, owner, spec, entity_pos, Box::new(Silent))?;
            self.drain_jobs();
            Ok(stream)
        }
        fn reap(&self) {
            self.drain_jobs();
            self.pool.reap(&self.mixer);
        }
        /// Publishes whole interleaved frames straight into the last opened voice's ring.
        fn feed_last(&self, samples: &[i16]) -> usize {
            let mixer = self.mixer.lock().unwrap();
            let voice = mixer.streams.last().unwrap();
            // Bypass the worker: these samples are already at the ring rate used in this rig.
            let c = &voice.control;
            if c.pull.ring_rate.load(Relaxed) == 0 {
                c.pull.ring_rate.store(c.rate, Relaxed);
            }
            let ring = &voice.ring;
            let channels = ring.channels;
            let (tail, _, free) = ring.producer_window();
            let frames = (samples.len() / channels).min(free);
            ring.publish_frames(tail, samples[..frames * channels].iter().copied().map(pcm));
            frames
        }
        /// The next `frames` of stereo output at 48 kHz.
        fn play(&self, frames: usize) -> Vec<f32> {
            let mut data = vec![0.0; frames * 2];
            mix_output(&mut self.mixer.lock().unwrap(), &mut data, 2, 48_000);
            data
        }
        fn voices(&self) -> usize {
            self.mixer.lock().unwrap().streams.len()
        }
    }
    fn spec() -> StreamSpec {
        StreamSpec::new(SoundSource::Voice, 1, 48_000)
    }
    /// Half-scale samples: 0.5 once mixed at gain 1.
    fn half(frames: usize) -> Vec<i16> {
        vec![16_384; frames]
    }
    /// 1, 2, 3...: each frame its own value, so which ones played shows.
    fn ramp(frames: usize) -> Vec<i16> {
        (1..=frames as i16).map(|i| i * 16).collect()
    }

    #[test]
    fn a_stream_starts_and_resumes_with_one_frame() {
        let rig = Rig::new();
        let stream = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 1, 36_000))
            .unwrap();
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(rig.play(2), [0.0; 4]);
        assert_eq!(rig.feed_last(&[16_384]), 1);
        // At 36 kHz a single frame is still current at output phases 0 and 0.75, in
        // separate buffers here. At 1.5 the source goes dry, without ending the stream.
        assert_eq!(rig.play(1), [0.5; 2]);
        assert_eq!(rig.play(3), [0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(rig.feed_last(&[-8192]), 1);
        // Resuming starts at phase zero, not partway through the new frame.
        assert_eq!(rig.play(3), [-0.25, -0.25, -0.25, -0.25, 0.0, 0.0]);
    }

    #[test]
    fn a_slower_stream_is_interpolated_and_keeps_its_last_frame_for_its_whole_length() {
        let rig = Rig::new();
        let _stream = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 1, 24_000))
            .unwrap();
        let samples = ramp(480);
        rig.feed_last(&samples);
        // Across two callbacks, so the resampler's place carries over the boundary.
        let mut out = rig.play(333);
        out.extend(rig.play(1_000));
        let value = |k: usize| f32::from(samples[k]) / 32768.0;
        for k in 0..479 {
            assert_eq!(out[2 * 2 * k], value(k), "source frame {k}");
            let middle = value(k) + (value(k + 1) - value(k)) * 0.5;
            assert_eq!(out[2 * (2 * k + 1)], middle, "between {k} and {}", k + 1);
        }
        // 480 source frames are 960 output frames; the last one holds for its second half.
        assert_eq!(out[2 * 958], value(479));
        assert_eq!(out[2 * 959], value(479));
        assert!(out[2 * 960..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_ring_keeps_frame_order_when_a_burst_wraps_a_full_ring() {
        let rig = Rig::new();
        let _stream = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 2, 48_000))
            .unwrap();
        // One second of stereo PCM; distinguish both ends and both channels.
        let mut queued = vec![0; 96_000];
        queued[..4].copy_from_slice(&[8192, -8192, 4096, -4096]);
        queued[95_998..].copy_from_slice(&[2048, -2048]);
        assert_eq!(rig.feed_last(&queued), 48_000);
        let burst = [16_384, -16_384, -8192, 8192, 4096, -4096];
        assert!(rig.play(0).is_empty());
        assert_eq!(rig.feed_last(&burst), 0);
        assert_eq!(rig.play(2), [0.25, -0.25, 0.125, -0.125]);

        // The next write can reuse the two slots just played, but not overwrite the rest.
        assert_eq!(rig.feed_last(&burst), 2);
        let remaining = rig.play(47_998);
        let (silence, last) = remaining.split_last_chunk::<2>().unwrap();
        assert!(silence.iter().all(|&sample| sample == 0.0));
        assert_eq!(*last, [0.0625, -0.0625]);
        assert_eq!(rig.play(3), [0.5, -0.5, -0.25, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn a_mono_stream_at_half_its_range_has_half_the_gain() {
        let rig = Rig::new();
        let range = 16.0;
        let heard = |at: [f64; 3]| {
            let stream = rig
                .open(
                    "mod",
                    &spec()
                        .with_placement(StreamPlacement::At(at))
                        .with_range(range),
                )
                .unwrap();
            rig.feed_last(&half(2_000));
            let out = rig.play(16);
            stream.stop();
            rig.reap();
            out[8]
        };
        let here = heard([0.0, 0.0, 0.0]);
        let half_way = heard([0.0, 0.0, f64::from(range) / 2.0]);
        let edge = heard([0.0, 0.0, f64::from(range)]);
        assert!(here > 0.2);
        assert!((half_way / here - 0.5).abs() < 1e-3, "{half_way} {here}");
        assert_eq!(edge, 0.0);
    }

    #[test]
    fn a_mono_output_hears_the_distance_gain_whichever_side_the_stream_is_on() {
        let rig = Rig::new();
        let mono_output = |at: [f64; 3]| {
            let stream = rig
                .open(
                    "mod",
                    &spec()
                        .with_placement(StreamPlacement::At(at))
                        .with_range(16.0),
                )
                .unwrap();
            rig.feed_last(&half(2_000));
            let mut data = vec![0.0; 8];
            mix_output(&mut rig.mixer.lock().unwrap(), &mut data, 1, 48_000);
            stream.stop();
            rig.reap();
            data[4]
        };
        // 8 blocks away to the left, to the right and ahead: half of 0.5 whichever.
        for at in [[-8.0, 0.0, 0.0], [8.0, 0.0, 0.0], [0.0, 0.0, 8.0]] {
            assert!((mono_output(at) - 0.25).abs() < 1e-6, "{at:?}");
        }
    }

    #[test]
    fn a_stereo_stream_with_a_placement_plays_unspatialized() {
        let rig = Rig::new();
        let stereo = StreamSpec::new(SoundSource::Voice, 2, 48_000)
            .with_placement(StreamPlacement::At([100.0, 0.0, 0.0]))
            .with_range(16.0)
            .with_volume(0.5);
        let stream = rig.open("mod", &stereo).unwrap();
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(
            rig.feed_last(&[16_384, -8_192, 16_384, -8_192, 16_384, -8_192]),
            3
        );
        // Far beyond range = 16.0, yet stereo plays un-spatialized at full [volume; 3] = 0.5.
        assert_eq!(rig.play(2), [0.25, -0.125, 0.25, -0.125]);
        let mut mono = [0.0; 1];
        mix_output(&mut rig.mixer.lock().unwrap(), &mut mono, 1, 48_000);
        assert_eq!(mono, [0.0625]);
        assert!(stream.set_placement(&StreamPlacement::At([1.0, 2.0, 3.0]), None));
        assert!(stream.set_placement(&StreamPlacement::Flat, None));
        assert!(!stream.set_placement(&StreamPlacement::At([f64::NAN, 0.0, 0.0]), None));
    }

    #[test]
    fn streams_are_capped_per_mod_and_overall() {
        let rig = Rig::new();
        let low_rate = StreamSpec::new(SoundSource::Voice, 1, 1_000);
        let mut open = Vec::new();
        for _ in 0..256 {
            open.push(rig.open("m0", &low_rate).unwrap());
        }
        assert!(matches!(
            rig.open("m0", &low_rate),
            Err(AudioRefusal::ModStreamLimit)
        ));
        for owner in ["m1", "m2", "m3"] {
            for _ in 0..256 {
                open.push(rig.open(owner, &low_rate).unwrap());
            }
        }
        assert!(matches!(
            rig.open("extra", &low_rate),
            Err(AudioRefusal::TooManyStreams)
        ));
        // A stopped stream frees its place.
        open.pop().unwrap().stop();
        assert!(rig.open("extra", &low_rate).is_ok());
    }

    #[test]
    fn bad_numbers_are_refused_and_never_reach_the_mixer() {
        let rig = Rig::new();
        let bad = |s: StreamSpec| rig.open("mod", &s).err();
        assert_eq!(
            bad(StreamSpec::new(SoundSource::Voice, 3, 48_000)),
            Some(AudioRefusal::BadSpec(SpecFault::Channels))
        );
        assert_eq!(
            bad(StreamSpec::new(SoundSource::Voice, 1, 0)),
            Some(AudioRefusal::BadSpec(SpecFault::Rate))
        );
        assert_eq!(
            bad(spec().with_volume(f32::NAN)),
            Some(AudioRefusal::BadSpec(SpecFault::Volume))
        );
        assert_eq!(
            bad(spec().with_range(f32::INFINITY)),
            Some(AudioRefusal::BadSpec(SpecFault::Range))
        );
        assert_eq!(
            bad(spec().with_placement(StreamPlacement::At([f64::NAN, 0.0, 0.0]))),
            Some(AudioRefusal::BadSpec(SpecFault::Position))
        );
        assert_eq!(
            bad(spec().with_placement(StreamPlacement::At([0.0, f64::INFINITY, 0.0]))),
            Some(AudioRefusal::BadSpec(SpecFault::Position))
        );
        assert_eq!(rig.voices(), 0);

        // Positive rates outside 8-48 kHz (including extreme u32::MAX) open without overflow/OOM.
        let low = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 1, 4_000))
            .unwrap();
        low.stop();
        let extreme = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 1, u32::MAX))
            .unwrap();
        extreme.stop();
        rig.reap();

        let high = rig
            .open("mod", &StreamSpec::new(SoundSource::Voice, 1, 96_000))
            .unwrap();
        assert_eq!(rig.feed_last(&[16_384, 16_384]), 2);
        assert_eq!(rig.play(1), [0.5, 0.5]);
        high.stop();
        rig.reap();

        // Finite positions past 30M blocks are accepted.
        let far = rig
            .open(
                "mod",
                &spec().with_placement(StreamPlacement::At([50_000_000.0, 0.0, 0.0])),
            )
            .unwrap();
        far.stop();
        rig.reap();

        // Finite out-of-range volumes clamp to 0.0..=1.0 on open.
        let loud = rig.open("mod", &spec().with_volume(2.5)).unwrap();
        assert_eq!(rig.feed_last(&[16_384]), 1);
        assert_eq!(rig.play(1), [0.5, 0.5]);
        loud.stop();
        rig.reap();

        let negative = rig.open("mod", &spec().with_volume(-1.0)).unwrap();
        assert_eq!(rig.feed_last(&[16_384]), 1);
        assert_eq!(rig.play(1), [0.0, 0.0]);
        negative.stop();
        rig.reap();

        // Runtime controls refuse NaN/non-finite and clamp finite out-of-range volumes.
        let stream = rig.open("mod", &spec()).unwrap();
        assert!(!stream.set_volume(f32::NAN));
        assert!(!stream.set_placement(&StreamPlacement::At([0.0, f64::INFINITY, 0.0]), None));
        assert!(stream.set_volume(2.0));
        assert_eq!(rig.feed_last(&[16_384]), 1);
        assert_eq!(rig.play(1), [0.5, 0.5]);
        assert!(stream.set_volume(-0.5));
        assert_eq!(rig.feed_last(&[16_384]), 1);
        assert_eq!(rig.play(1), [0.0, 0.0]);
        assert_eq!(rig.voices(), 1);
    }

    #[test]
    fn an_entity_stream_follows_its_entity_and_stops_when_untracked() {
        let rig = Rig::new();
        let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7);
        let entity_spec = spec().with_placement(StreamPlacement::Entity(entity));
        assert_eq!(
            rig.open_at_entity("mod", &entity_spec, None).err(),
            Some(AudioRefusal::BadSpec(SpecFault::Position))
        );
        assert_eq!(
            rig.open_at_entity("mod", &entity_spec, Some([f64::NAN, 0.0, 0.0]))
                .err(),
            Some(AudioRefusal::BadSpec(SpecFault::Position))
        );

        let stream = rig
            .open_at_entity("mod", &entity_spec, Some([0.0, 0.0, 8.0]))
            .unwrap();
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(rig.feed_last(&[16_384, 16_384, 16_384]), 3);
        let mut out = [0.0; 1];
        mix_output(&mut rig.mixer.lock().unwrap(), &mut out, 1, 48_000);
        // Half of range 16 blocks -> half of 0.5.
        assert!((out[0] - 0.25).abs() < 1e-6);

        // Following updates the position to the listener -> unit distance gain.
        rig.pool
            .follow_entities(|e| (e == entity).then_some([0.0; 3]));
        assert_eq!(stream.state(), AudioStreamState::Playing);
        mix_output(&mut rig.mixer.lock().unwrap(), &mut out, 1, 48_000);
        assert_eq!(out, [0.5]);

        // Losing track of the entity stops the stream.
        rig.pool.follow_entities(|_| None);
        assert_eq!(stream.state(), AudioStreamState::Stopped);
        mix_output(&mut rig.mixer.lock().unwrap(), &mut out, 1, 48_000);
        assert_eq!(out, [0.0]);

        // A non-finite coordinate during follow also stops the stream.
        let stream2 = rig
            .open_at_entity("mod", &entity_spec, Some([0.0; 3]))
            .unwrap();
        assert_eq!(stream2.state(), AudioStreamState::Playing);
        rig.pool.follow_entities(|_| Some([f64::NAN, 0.0, 0.0]));
        assert_eq!(stream2.state(), AudioStreamState::Stopped);
    }

    #[test]
    fn a_newer_placement_is_never_overwritten_by_an_older_entity_resolution() {
        let rig = Rig::new();
        let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7);
        let heard = |follow_after: bool| {
            let stream = rig
                .open_at_entity(
                    "mod",
                    &spec().with_placement(StreamPlacement::Entity(entity)),
                    Some([0.0, 0.0, 1.0]),
                )
                .unwrap();
            rig.feed_last(&half(2_000));
            // The mod places the stream at a point; the frame's resolution of the entity it
            // followed before then arrives, at another place.
            assert!(stream.set_placement(&StreamPlacement::At([0.0, 0.0, 8.0]), None));
            if follow_after {
                rig.pool.follow_entities(|_| Some([0.0, 0.0, 1.0]));
            }
            let out = rig.play(16);
            stream.stop();
            rig.reap();
            (out[8], stream.state())
        };
        let (alone, _) = heard(false);
        let (after, _) = heard(true);
        assert!(alone > 0.0);
        assert_eq!(after, alone);
    }

    #[test]
    fn moving_to_flat_loses_if_entity_removal_won_the_placement_lock() {
        use std::{sync::mpsc::sync_channel, thread};

        let rig = Rig::new();
        let entity = EntityRef::new(mod_api::WorldEpoch::new(1), 7);
        let stream = rig
            .open_at_entity(
                "mod",
                &spec().with_placement(StreamPlacement::Entity(entity)),
                Some([0.0; 3]),
            )
            .unwrap();
        let control = rig.pool.state().controls[0].clone();
        let (resolving, resolver_entered) = sync_channel(0);
        let (moving, move_entered) = sync_channel(0);
        thread::scope(|s| {
            let control = &control;
            s.spawn(move || {
                control.follow(&mut |old| {
                    assert_eq!(old, entity);
                    resolving.send(()).unwrap();
                    move_entered.recv().unwrap();
                    None
                })
            });
            resolver_entered.recv().unwrap();
            let (placement, view) = check_placement(&StreamPlacement::Flat, None).unwrap();
            // The move has validated its placement, but can't enter its critical section
            // until follow has resolved the old entity and stopped the stream.
            moving.send(()).unwrap();
            assert!(!control.place(control.lock(), placement, view));
        });
        assert_eq!(stream.state(), AudioStreamState::Stopped);
    }

    #[test]
    fn a_dimension_change_stops_all_streams() {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            std::path::PathBuf::from("/nonexistent/sunrise-test-audio"),
        ));
        audio.backend_live_for_bench();
        audio.set_world("minecraft:overworld");
        let positional = audio
            .open_stream(
                "mod",
                &spec().with_placement(StreamPlacement::At([0.0, 0.0, 1.0])),
                None,
                Box::new(Silent),
            )
            .unwrap();
        let flat = audio
            .open_stream("mod", &spec(), None, Box::new(Silent))
            .unwrap();
        audio.set_world("minecraft:overworld");
        assert_eq!(positional.state(), AudioStreamState::Playing);
        assert_eq!(flat.state(), AudioStreamState::Playing);
        audio.set_world("minecraft:the_nether");
        assert_eq!(positional.state(), AudioStreamState::Stopped);
        assert_eq!(flat.state(), AudioStreamState::Stopped);
    }

    #[test]
    fn a_stopped_stream_frees_its_ring_even_while_the_mod_keeps_the_handle() {
        let rig = Rig::new();
        let mut kept = Vec::new();
        let mut rings = Vec::new();
        for _ in 0..4 {
            let stream = rig.open("mod", &spec()).unwrap();
            rig.feed_last(&half(100));
            rings.push(Arc::downgrade(
                &rig.mixer.lock().unwrap().streams.last().unwrap().ring,
            ));
            stream.stop();
            kept.push(stream);
        }
        // Each open reaps the stopped streams before it: only the last one's ring is still held.
        assert_eq!(rig.voices(), 1);
        assert!(rings[..3].iter().all(|ring| ring.upgrade().is_none()));
        // The next open or the tick lets go of that one too.
        rig.reap();
        assert_eq!(rig.voices(), 0);
        assert!(rings.iter().all(|ring| ring.upgrade().is_none()));
        for stream in &kept {
            assert_eq!(stream.state(), AudioStreamState::Stopped);
        }
    }

    #[test]
    fn dropping_the_handle_stops_the_stream_and_the_pool_lets_it_go() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        drop(mod_api::AudioStreamRef::new(stream));
        rig.reap();
        assert_eq!(rig.voices(), 0);
    }

    #[test]
    fn leaving_a_level_stops_all_streams() {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            std::path::PathBuf::from("/nonexistent/sunrise-test-audio"),
        ));
        audio.backend_live_for_bench();
        let positional = audio
            .open_stream(
                "mod",
                &spec().with_placement(StreamPlacement::At([0.0, 0.0, 1.0])),
                None,
                Box::new(Silent),
            )
            .unwrap();
        let flat = audio
            .open_stream("mod", &spec(), None, Box::new(Silent))
            .unwrap();
        mix_output(
            &mut audio.backend.mixer.lock().unwrap(),
            &mut [0.0; 16],
            2,
            48_000,
        );
        assert_eq!(positional.state(), AudioStreamState::Playing);
        assert_eq!(flat.state(), AudioStreamState::Playing);
        audio.leave_level();
        assert_eq!(positional.state(), AudioStreamState::Stopped);
        assert_eq!(flat.state(), AudioStreamState::Stopped);
    }

    #[test]
    fn losing_a_tracked_entity_while_muted_stops_the_stream_without_unmuting() {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            std::path::PathBuf::from("/nonexistent/sunrise-test-audio"),
        ));
        audio.backend_live_for_bench();
        let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7);
        let stream = audio
            .open_stream(
                "mod",
                &spec().with_placement(StreamPlacement::Entity(entity)),
                Some([0.0, 0.0, 1.0]),
                Box::new(Silent),
            )
            .unwrap();
        let heard = |audio: &Audio| {
            audio.backend.mixer.lock().unwrap().stream_gain[SoundSource::Voice.index()]
        };
        audio.sync_stream_gain(0.0);
        assert_eq!(heard(&audio), 0.0);
        assert_eq!(stream.state(), AudioStreamState::Playing);

        // While muted, entity movement is still tracked and keeps the stream Playing in silence.
        let mut asked = 0;
        audio.follow_stream_entities(|e| {
            asked += 1;
            (e == entity).then_some([0.0, 0.0, 4.0])
        });
        assert_eq!(asked, 1);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(heard(&audio), 0.0);

        // Losing the entity while still muted stops the stream immediately without unmuting.
        audio.follow_stream_entities(|_| {
            asked += 1;
            None
        });
        assert_eq!(asked, 2);
        assert_eq!(stream.state(), AudioStreamState::Stopped);
        assert_eq!(heard(&audio), 0.0);
    }

    #[test]
    fn a_lost_output_stops_every_stream_and_refuses_new_ones_until_it_is_back() {
        let rig = Rig::new();
        let stream = rig.open("mod", &spec()).unwrap();
        rig.feed_last(&half(2_000));
        rig.play(8);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        rig.pool.output_lost(true);
        assert_eq!(stream.state(), AudioStreamState::Stopped);
        assert!(matches!(
            rig.open("mod", &spec()),
            Err(AudioRefusal::NoOutput)
        ));
        rig.pool.output_lost(false);
        assert!(rig.open("mod", &spec()).is_ok());
    }

    #[test]
    fn the_muffle_turns_streams_down_with_everything_else() {
        let rig = Rig::new();
        let _stream = rig.open("mod", &spec()).unwrap();
        rig.feed_last(&half(2_000));
        {
            let mut mixer = rig.mixer.lock().unwrap();
            mixer.muffle.target = [1.0, 0.0];
            mixer.muffle.current = [1.0, 0.0];
        }
        assert!(rig.play(64).iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_muted_stream_advances_and_resumes_after_running_dry() {
        let rig = Rig::new();
        let stream = rig
            .open("mod", &StreamSpec::new(SoundSource::Music, 1, 36_000))
            .unwrap();
        assert_eq!(rig.feed_last(&[4096, 8192, 16_384, -16_384, -8192]), 5);
        let set_gain = |gain| {
            rig.mixer.lock().unwrap().stream_gain[SoundSource::Music.index()] = gain;
        };
        set_gain(0.0);
        assert_eq!(rig.play(1), [0.0; 2]);
        assert_eq!(rig.play(2), [0.0; 4]);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        set_gain(1.0);
        // Three muted frames advance 2.25 source frames. Unmuting is a quarter of the way
        // between 0.5 and -0.5, then exactly at -0.5.
        assert_eq!(rig.play(2), [0.25, 0.25, -0.5, -0.5]);
        set_gain(0.0);
        // These output phases are 3.75, 4.5 and 5.25. The last needs PCM that is not there.
        assert_eq!(rig.play(3), [0.0; 6]);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        assert_eq!(rig.feed_last(&[16_384]), 1);
        set_gain(1.0);
        assert_eq!(rig.play(2), [0.5; 4]);
    }

    #[test]
    fn a_stream_is_never_heard_flat_before_its_placement_is_in() {
        let rig = Rig::new();
        let _stream = rig
            .open(
                "mod",
                &spec().with_placement(StreamPlacement::At([0.0, 0.0, 10_000.0])),
            )
            .unwrap();
        rig.feed_last(&half(2_000));
        // A writer is in the middle of a change when the callback first looks.
        rig.pool.state().controls[0].sequence.store(1, Relaxed);
        assert!(rig.play(64).iter().all(|s| *s == 0.0));
        let voice = &rig.mixer.lock().unwrap().streams[0];
        assert!(voice.view.positional);
    }

    #[test]
    fn with_no_stream_open_the_pool_takes_no_lock_of_the_mixers() {
        let rig = Rig::new();
        assert!(rig.pool.idle());
        // The callback holds the mixer: with nothing open, nothing here waits for it.
        let _callback = rig.mixer.lock().unwrap();
        rig.pool.reap(&rig.mixer);
        rig.pool.stop_all();
        rig.pool.follow_entities(|_| unreachable!());
        drop(_callback);
        let stream = rig.open("mod", &spec()).unwrap();
        assert!(!rig.pool.idle());
        stream.stop();
        rig.reap();
        assert!(rig.pool.idle());
        assert_eq!(rig.voices(), 0);
    }

    #[test]
    fn controls_from_another_thread_never_wait_on_the_mixer() {
        let rig = Rig::new();
        let stream = mod_api::AudioStreamRef::new(rig.open("mod", &spec()).unwrap());
        // The mixer is locked, as the output callback holds it, for the whole of the calls.
        let _callback = rig.mixer.lock().unwrap();
        let caller = std::thread::spawn(move || {
            assert!(stream.set_volume(0.5));
            assert!(stream.set_position([0.0, 0.0, 8.0]));
            assert!(stream.set_flat());
            assert_eq!(stream.state(), AudioStreamState::Playing);
            stream.stop();
            stream.state()
        });
        assert_eq!(caller.join().unwrap(), AudioStreamState::Stopped);
    }

    #[test]
    fn moving_to_a_fixed_point_from_a_thread_without_a_host() {
        let rig = Rig::new();
        let stream =
            mod_api::AudioStreamRef::new(rig.open("mod", &spec().with_range(16.0)).unwrap());
        rig.feed_last(&half(2_000));
        let moved = std::thread::spawn(move || {
            assert!(!stream.set_position([f64::NAN, 0.0, 0.0]));
            assert!(stream.set_position([0.0, 0.0, 8.0]));
            stream
        })
        .join()
        .unwrap();
        let mut mono = [0.0; 1];
        mix_output(&mut rig.mixer.lock().unwrap(), &mut mono, 1, 48_000);
        // At half of range = 16 blocks, half-scale PCM (0.5) attenuates to 0.25.
        assert!((mono[0] - 0.25).abs() < 1e-6, "got {}", mono[0]);

        let flattened = std::thread::spawn(move || {
            assert!(moved.set_flat());
            moved
        })
        .join()
        .unwrap();
        mix_output(&mut rig.mixer.lock().unwrap(), &mut mono, 1, 48_000);
        assert_eq!(mono, [0.5]);
        flattened.stop();
    }

    #[test]
    fn churning_one_owners_streams_while_worker_is_held_never_starves_another_owner() {
        let mixer = Mutex::new(Mixer {
            stream_gain: [1.0; 11],
            ..Mixer::default()
        });
        let pool = StreamPool::default();
        let (worker, receiver) = Worker::test_pair();
        pool.state().worker = Some(worker);
        let low_rate = StreamSpec::new(SoundSource::Voice, 1, 1_000);

        // Hold worker progress (never drain `receiver`) while one owner repeatedly opens, stops,
        // and reaps streams. Pending unretired jobs must stay charged to that owner's quota so
        // it hits ModStreamLimit instead of filling the shared admission queue.
        for _ in 0..AUDIO_STREAMS_PER_MOD {
            let stream = pool
                .open(&mixer, "churner", &low_rate, None, Box::new(Silent))
                .unwrap();
            stream.stop();
            pool.reap(&mixer);
        }
        assert_eq!(mixer.lock().unwrap().streams.len(), 0);
        assert_eq!(
            pool.open(&mixer, "churner", &low_rate, None, Box::new(Silent))
                .err(),
            Some(AudioRefusal::ModStreamLimit)
        );

        // An unrelated owner can still open a stream while the worker is held.
        let other = pool
            .open(&mixer, "other", &low_rate, None, Box::new(Silent))
            .unwrap();
        assert_eq!(other.state(), AudioStreamState::Playing);
        assert_eq!(mixer.lock().unwrap().streams.len(), 1);

        // Once the worker catches up and retires the canceled jobs, the churner's quota frees.
        let mut live_jobs = Vec::new();
        while let Ok(job) = receiver.try_recv() {
            if !job.control.closed.load(Acquire) {
                live_jobs.push(job);
            }
        }
        assert_eq!(live_jobs.len(), 1);
        let recovered = pool
            .open(&mixer, "churner", &low_rate, None, Box::new(Silent))
            .unwrap();
        assert_eq!(recovered.state(), AudioStreamState::Playing);
        other.stop();
        recovered.stop();
    }

    #[test]
    fn an_owners_high_rate_pcm_budget_includes_unretired_jobs_and_leaves_room_for_others() {
        let pool = StreamPool::default();
        let mixer = Mutex::new(Mixer::default());
        let (worker, receiver) = Worker::test_pair();
        pool.state().worker = Some(worker);
        let extreme = StreamSpec::new(SoundSource::Voice, 2, u32::MAX);
        // Ten maximum stereo rings take 30,720,000 bytes; an eleventh exceeds 32 MiB.
        // Leave the worker's queue alone: even though their voices are reaped, this owner's
        // stopped streams still occupy PCM memory until the jobs release their rings.
        for _ in 0..10 {
            let stream = pool
                .open(&mixer, "large", &extreme, None, Box::new(Silent))
                .unwrap();
            stream.stop();
        }
        pool.reap(&mixer);
        assert!(mixer.lock().unwrap().streams.is_empty());
        assert!(matches!(
            pool.open(&mixer, "large", &extreme, None, Box::new(Silent)),
            Err(AudioRefusal::MemoryLimit)
        ));
        let other = pool
            .open(&mixer, "other", &spec(), None, Box::new(Silent))
            .unwrap();
        assert_eq!(other.state(), AudioStreamState::Playing);

        drop(receiver.try_recv().unwrap());
        let recovered = pool
            .open(&mixer, "large", &extreme, None, Box::new(Silent))
            .unwrap();
        assert_eq!(recovered.state(), AudioStreamState::Playing);
    }

    #[test]
    fn aggregate_pcm_budget_bounds_live_and_pending_jobs_and_ring_allocation_is_fallible() {
        let max_stereo_ring_bytes = MAX_RING_FRAMES as usize * 2 * std::mem::size_of::<AtomicU32>();
        // Enough for two maximum-size stereo rings plus a small ring, but not a third maximum.
        let budget = 8 * 1024 * 1024;
        let mixer = Mutex::new(Mixer {
            stream_gain: [1.0; 11],
            ..Mixer::default()
        });
        let pool = StreamPool::with_pcm_budget(budget);
        let (worker, receiver) = Worker::test_pair();
        pool.state().worker = Some(worker);

        let extreme_stereo = StreamSpec::new(SoundSource::Voice, 2, u32::MAX);
        let s1 = pool
            .open(&mixer, "mod_a", &extreme_stereo, None, Box::new(Silent))
            .unwrap();
        let s2 = pool
            .open(&mixer, "mod_a", &extreme_stereo, None, Box::new(Silent))
            .unwrap();
        assert_eq!(pool.pcm_bytes.load(Acquire), 2 * max_stereo_ring_bytes);
        assert_eq!(
            pool.open(&mixer, "mod_b", &extreme_stereo, None, Box::new(Silent))
                .err(),
            Some(AudioRefusal::MemoryLimit)
        );

        // Stopping and reaping s1 while its PullJob is still queued does not release its ring
        // bytes until the worker retires the job.
        s1.stop();
        pool.reap(&mixer);
        assert_eq!(pool.pcm_bytes.load(Acquire), 2 * max_stereo_ring_bytes);
        assert_eq!(
            pool.open(&mixer, "mod_b", &extreme_stereo, None, Box::new(Silent))
                .err(),
            Some(AudioRefusal::MemoryLimit)
        );

        // Smaller streams that fit inside the remaining budget still open.
        let small = pool
            .open(
                &mixer,
                "mod_b",
                &StreamSpec::new(SoundSource::Voice, 2, 48_000),
                None,
                Box::new(Silent),
            )
            .unwrap();
        small.stop();
        pool.reap(&mixer);

        // Retiring canceled jobs on the worker drops their Arc<Ring> and frees the budget.
        let mut live_jobs = Vec::new();
        while let Ok(job) = receiver.try_recv() {
            if !job.control.closed.load(Acquire) {
                live_jobs.push(job);
            }
        }
        assert_eq!(live_jobs.len(), 1);
        assert_eq!(pool.pcm_bytes.load(Acquire), max_stereo_ring_bytes);

        let s3 = pool
            .open(&mixer, "mod_b", &extreme_stereo, None, Box::new(Silent))
            .unwrap();
        assert_eq!(pool.pcm_bytes.load(Acquire), 2 * max_stereo_ring_bytes);
        s2.stop();
        s3.stop();

        // Fallible allocation returns None and rolls back the byte counter on impossible sizes.
        let counter = Arc::new(AtomicUsize::new(0));
        assert!(Ring::try_new(usize::MAX / 8, 1, counter.clone(), usize::MAX).is_none());
        assert_eq!(counter.load(Acquire), 0);
        assert!(Ring::try_new(usize::MAX, 2, counter.clone(), usize::MAX).is_none());
        assert_eq!(counter.load(Acquire), 0);
    }
}
