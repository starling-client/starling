//! One lazy PCM producer for all native pull streams on a shared output.
use super::{Control, Ring, pcm};
use mod_api::{__private::game::CallbackOwner, AUDIO_STREAMS_TOTAL, AudioRefusal, AudioStream};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering::*},
        mpsc::{self, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, Thread},
};

const CAPACITY: usize = AUDIO_STREAMS_TOTAL as usize;
const ADMIT_CAPACITY: usize = CAPACITY;
const SCRATCH_SAMPLES: usize = 96_000;

/// Pair the thread's park token with a flag: source code runs on this thread and may use
/// standard synchronization that consumes the token before we get back to the worker loop.
#[derive(Clone)]
pub(super) struct Wake {
    pending: Arc<AtomicBool>,
    thread: Thread,
}

impl Wake {
    pub(super) fn signal(&self) {
        if !self.pending.swap(true, Release) {
            self.thread.unpark();
        }
    }
}

pub(super) struct Demand {
    pub(super) wake: Wake,
    /// The rate of queued PCM, set by the callback before it publishes its first demand.
    pub(super) ring_rate: AtomicU32,
    /// The desired number of queued ring frames, or zero when no request is pending. The latest
    /// output buffer replaces an older request rather than accumulating a backlog.
    pub(super) frames: AtomicUsize,
    pub(super) eof: AtomicBool,
}

/// The only sender. Disconnecting it and waking the worker makes teardown independent of the
/// next device callback. Live voices and handles have only the wake token, not a sender.
pub(super) struct Worker {
    sender: SyncSender<PullJob>,
    wake: Wake,
}

impl Worker {
    pub(super) fn start() -> Result<Self, AudioRefusal> {
        let (sender, receiver) = mpsc::sync_channel(ADMIT_CAPACITY);
        let pending = Arc::new(AtomicBool::new(false));
        let signaled = pending.clone();
        let worker = thread::Builder::new()
            .name("audio-pull".into())
            .spawn(move || {
                let mut jobs = Vec::<PullJob>::with_capacity(CAPACITY);
                let mut scratch = vec![0; SCRATCH_SAMPLES];
                loop {
                    while jobs.len() < CAPACITY {
                        match receiver.try_recv() {
                            Ok(job) => jobs.push(job),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    let was_full = jobs.len() == CAPACITY;
                    let mut more = false;
                    jobs.retain_mut(|job| {
                        let live = job.refill(&mut scratch);
                        more |= live && job.target != 0;
                        live
                    });
                    // Each job gets one bounded read per pass. Keep going if any job still
                    // owes frames, or if a full job list freed up without polling the queue.
                    // A wake sent during this pass is not lost.
                    if !more && (!was_full || jobs.len() == CAPACITY) {
                        while !signaled.swap(false, Acquire) {
                            thread::park();
                        }
                    }
                }
            })
            .map_err(|_| AudioRefusal::WorkerUnavailable)?;
        Ok(Self {
            sender,
            wake: Wake {
                pending,
                thread: worker.thread().clone(),
            },
        })
    }

    pub(super) fn demand(&self) -> Demand {
        Demand {
            wake: self.wake.clone(),
            ring_rate: AtomicU32::new(0),
            frames: AtomicUsize::new(0),
            eof: AtomicBool::new(false),
        }
    }

    /// Returns the unadmitted job, if any, so the caller can release its guard
    /// before dropping the source.
    pub(super) fn admit(&self, job: PullJob) -> Option<PullJob> {
        match self.sender.try_send(job) {
            Ok(()) => {
                self.wake();
                None
            }
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => Some(job),
        }
    }

    pub(super) fn wake(&self) {
        self.wake.signal();
    }

    pub(super) fn shutdown(self) {
        drop(self.sender);
        self.wake.signal();
    }

    #[cfg(test)]
    pub(super) fn test_pair() -> (Self, mpsc::Receiver<PullJob>) {
        let (sender, receiver) = mpsc::sync_channel(ADMIT_CAPACITY);
        (
            Self {
                sender,
                wake: Wake {
                    pending: Arc::default(),
                    thread: thread::current(),
                },
            },
            receiver,
        )
    }
}

/// Runs last when a [`PullJob`] drops, after its source and PCM ring have been released.
struct JobRetirement(Arc<Control>);

impl Drop for JobRetirement {
    fn drop(&mut self) {
        self.0.job_retired.store(true, Release);
    }
}

/// Owns native source disposal, including when opening refuses the source before admission.
pub(super) struct OwnedSource {
    owner: CallbackOwner,
    source: Option<Box<dyn AudioStream>>,
}

impl OwnedSource {
    pub(super) fn new(owner: &'static str, source: Box<dyn AudioStream>) -> Self {
        Self {
            owner: CallbackOwner::for_mod(owner),
            source: Some(source),
        }
    }

    fn finish(&mut self) -> bool {
        if let Some(source) = self.source.take() {
            self.owner
                .dispose("audio stream", "AudioStream::drop", source);
        }
        self.owner.is_enabled()
    }
}

impl Drop for OwnedSource {
    fn drop(&mut self) {
        self.finish();
    }
}

pub(super) struct PullJob {
    source: OwnedSource,
    pub(super) ring: Arc<Ring>,
    pub(super) demand: Arc<Demand>,
    pub(super) control: Arc<Control>,
    /// Last requested queue length, carried over when a single read cannot fill it.
    target: usize,
    downsample: Downsample,
    _retire: JobRetirement,
}

/// A worker-side linear resampler for sources faster than the ring. The next output frame
/// is at `index + phase / ring_rate` in source frames. Whole frames between output times
/// are discarded here instead of requiring the mixer to buffer that entire advance.
#[derive(Default)]
struct Downsample {
    index: u64,
    phase: u64,
    consumed: u64,
    previous: [f32; 2],
}

impl Downsample {
    /// How many more source frames the requested ring frames need, including a right-hand
    /// sample for each fractional position. The last position determines how far to read.
    fn needed(&self, missing: usize, input_rate: u64, ring_rate: u64) -> u64 {
        let phase = self.phase + (missing as u64 - 1) * input_rate;
        self.index + phase.div_ceil(ring_rate) + 1 - self.consumed
    }

    fn queue(
        &mut self,
        ring: &Ring,
        tail: u64,
        samples: &[i16],
        input_rate: u64,
        ring_rate: u64,
    ) -> usize {
        let ch = ring.channels;
        let start = self.consumed;
        let end = start + (samples.len() / ch) as u64;
        let previous = self.previous;
        let at = |index| {
            if index < start {
                // Downsampling advances by >1 source frame, so only the last frame of the
                // previous input chunk can be needed across a chunk boundary.
                previous
            } else {
                let i = (index - start) as usize * ch;
                let left = pcm(samples[i]);
                [left, if ch == 2 { pcm(samples[i + 1]) } else { left }]
            }
        };
        let mut written = 0;
        let frames = std::iter::from_fn(|| {
            if self.index >= end {
                return None;
            }
            let mut frame = at(self.index);
            if self.phase != 0 {
                if self.index + 1 < end {
                    let next = at(self.index + 1);
                    let t = (self.phase as f64 / ring_rate as f64) as f32;
                    for (sample, next) in frame.iter_mut().zip(next).take(ch) {
                        *sample += (next - *sample) * t;
                    }
                } else if !samples.is_empty() {
                    return None; // Wait for the next chunk's first frame (or EOF).
                }
                // At EOF, hold the final source sample for the rest of its duration.
            }
            let step = self.phase + input_rate;
            self.index += step / ring_rate;
            self.phase = step % ring_rate;
            written += 1;
            Some(frame)
        });
        ring.publish_frames(tail, frames.flat_map(|frame| frame.into_iter().take(ch)));
        if end != start {
            self.previous = at(end - 1);
            self.consumed = end;
        }
        written
    }
}

impl Drop for PullJob {
    fn drop(&mut self) {
        // Silence non-EOF exits before dropping mod code. Only EOF drains queued samples.
        if !self.demand.eof.load(Relaxed) {
            self.control.stop();
        }
    }
}

impl PullJob {
    pub(super) fn new(
        control: Arc<Control>,
        demand: Arc<Demand>,
        ring: Arc<Ring>,
        source: OwnedSource,
    ) -> Self {
        let retire = JobRetirement(control.clone());
        Self {
            source,
            ring,
            demand,
            control,
            target: 0,
            downsample: Downsample::default(),
            _retire: retire,
        }
    }

    /// At most one read per pass, using only the missing whole frames. False retires the job
    /// and its source on the worker; only the mixer decides when an EOF voice has drained.
    fn refill(&mut self, scratch: &mut [i16]) -> bool {
        if self.control.closed.load(Acquire) || !self.source.owner.is_enabled() {
            return false;
        }
        let target = self.demand.frames.swap(0, AcqRel);
        if target != 0 {
            self.target = target;
        }
        if self.target == 0 {
            return true;
        }
        let (tail, queued, free) = self.ring.producer_window();
        let missing = self.target.saturating_sub(queued).min(free);
        if missing == 0 {
            self.target = 0;
            return true;
        }
        let channels = self.ring.channels;
        let input_rate = u64::from(self.control.rate);
        let ring_rate = u64::from(self.demand.ring_rate.load(Relaxed));
        let downsample = input_rate > ring_rate;
        let needed = if downsample {
            self.downsample.needed(missing, input_rate, ring_rate)
        } else {
            missing as u64
        };
        let offered = needed.min((scratch.len() / channels) as u64) as usize;
        let buffer = &mut scratch[..offered * channels];
        buffer.fill(0);
        let source = self.source.source.as_mut().expect("live pull source");
        let Ok(frames) = self
            .source
            .owner
            .call("audio stream", "AudioStream::read", || source.read(buffer))
        else {
            return false;
        };
        if !self.source.owner.is_enabled() || self.control.closed.load(Acquire) {
            return false;
        }
        if frames > offered {
            log::warn!(
                "mod `{}` audio stream returned invalid frame count {frames} (offered {offered}); stopping stream",
                self.control.owner
            );
            return false;
        }
        let samples = &buffer[..frames * channels];
        let filled = if downsample {
            self.downsample
                .queue(&self.ring, tail, samples, input_rate, ring_rate)
                == missing
        } else {
            self.ring
                .publish_frames(tail, samples.iter().copied().map(pcm));
            // As before, an ordinary short read yields until the next device request.
            // A full read limited by scratch continues the outstanding demand.
            frames < offered || frames == missing
        };
        if frames == 0 {
            if self.source.finish() {
                self.demand.eof.store(true, Release);
            }
            return false;
        }
        if filled {
            self.target = 0;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{Mixer, mix_pieces};
    use super::super::StreamPool;
    use super::*;
    use mod_api::{AudioStreamSource, AudioStreamState, SoundSource, StreamSpec};
    use std::{sync::Mutex, thread::ThreadId, time::Duration};

    struct Source<F>(F);

    impl<F: FnMut(&mut [i16]) -> usize + Send + 'static> AudioStream for Source<F> {
        fn read(&mut self, pcm: &mut [i16]) -> usize {
            (self.0)(pcm)
        }
    }

    /// Takes the admitted job instead of starting a thread, so each refill happens at a chosen
    /// output buffer boundary. Uses the same queue, refill and mixer as a live output.
    struct Rig {
        _pool: StreamPool,
        mixer: Mutex<Mixer>,
        stream: Arc<dyn AudioStreamSource>,
        job: Option<PullJob>,
        scratch: [i16; 16],
    }

    impl Rig {
        fn new(read: impl FnMut(&mut [i16]) -> usize + Send + 'static) -> Self {
            let pool = StreamPool::default();
            let (worker, receiver) = Worker::test_pair();
            pool.state().worker = Some(worker);
            let mixer = Mutex::new(Mixer {
                stream_gain: [1.0; 11],
                ..Mixer::default()
            });
            let spec = StreamSpec::new(SoundSource::Voice, 2, 24_000);
            let stream = pool
                .open(&mixer, "mod", &spec, None, Box::new(Source(read)))
                .unwrap();
            Self {
                _pool: pool,
                mixer,
                stream,
                job: Some(receiver.try_recv().unwrap()),
                scratch: [99; 16],
            }
        }

        fn play<const SAMPLES: usize>(&self) -> [f32; SAMPLES] {
            let mut out = [0.0; SAMPLES];
            mix_pieces(&mut self.mixer.lock().unwrap(), &mut out, 2, 48_000);
            out
        }

        fn refill(&mut self) -> bool {
            if self.job.as_mut().unwrap().refill(&mut self.scratch) {
                true
            } else {
                self.job = None;
                false
            }
        }
    }

    #[test]
    fn a_pull_worker_reads_and_drops_the_source_with_both_guards_held() {
        struct Probe {
            read: SyncSender<(ThreadId, usize)>,
            dropped: SyncSender<ThreadId>,
        }

        impl AudioStream for Probe {
            fn read(&mut self, pcm: &mut [i16]) -> usize {
                pcm[0] = 16_384;
                let _ = self.read.try_send((thread::current().id(), pcm.len()));
                1
            }
        }

        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = self.dropped.try_send(thread::current().id());
            }
        }

        let (read, reads) = mpsc::sync_channel(1);
        let (dropped, drops) = mpsc::sync_channel(1);
        let pool = StreamPool::default();
        let mixer = Mutex::new(Mixer::default());
        let spec = StreamSpec::new(SoundSource::Voice, 2, 24_000);
        let stream = pool
            .open(
                &mixer,
                "mod",
                &spec,
                None,
                Box::new(Probe { read, dropped }),
            )
            .unwrap();

        let _pool = pool.state();
        let mut mixer = mixer.lock().unwrap();
        let mut out = [0.0; 8];
        mix_pieces(&mut mixer, &mut out, 2, 48_000);
        assert_eq!(out, [0.0; 8]);

        let timeout = Duration::from_secs(1);
        let (worker, samples) = reads
            .recv_timeout(timeout)
            .expect("worker did not read PCM");
        assert_ne!(worker, thread::current().id());
        assert_eq!(samples, 6); // Four output frames need two stereo source frames plus lookahead.

        stream.stop();
        assert_eq!(stream.state(), AudioStreamState::Stopped);
        // Still holding both guards, with no further mix or reap call.
        assert_eq!(
            drops.recv_timeout(timeout).expect("source was not dropped"),
            worker
        );
    }

    #[test]
    fn pull_plays_short_reads_silence_frames_and_drains_the_last_frame_at_eof() {
        let reads: [(usize, &[i16], usize); 4] = [
            (6, &[16_384, -8_192], 1),
            (6, &[], 1),
            (6, &[8_192, 16_384, -16_384, 8_192], 2),
            (2, &[], 0),
        ];
        let mut reads = reads.into_iter();
        let mut rig = Rig::new(move |pcm| {
            let (offered, samples, result) = reads.next().unwrap();
            assert_eq!(pcm.len(), offered);
            assert!(pcm.iter().all(|&sample| sample == 0));
            pcm[..samples.len()].copy_from_slice(samples);
            result
        });
        // An empty callback asks for nothing.
        assert_eq!(rig.play::<0>(), [0.0; 0]);
        assert!(rig.refill());
        assert_eq!(rig.play::<8>(), [0.0; 8]);
        assert!(rig.refill());
        assert_eq!(
            rig.play::<8>(),
            [0.5, -0.25, 0.5, -0.25, 0.0, 0.0, 0.0, 0.0]
        );
        // Returning 1 without writing publishes one silent frame from the zeroed buffer.
        assert!(rig.refill());
        assert!(rig.refill());
        assert_eq!(rig.job.as_ref().unwrap().ring.producer_window().1, 1);
        assert_eq!(rig.stream.state(), AudioStreamState::Playing);
        assert_eq!(rig.play::<8>(), [0.0; 8]);
        assert!(rig.refill());
        assert_eq!(rig.play::<2>(), [0.25, 0.5]);
        assert!(!rig.refill()); // EOF while another frame is queued
        assert_eq!(rig.play::<4>(), [-0.125, 0.375, -0.5, 0.25]);
        assert_eq!(rig.stream.state(), AudioStreamState::Playing);
        assert_eq!(rig.play::<2>(), [-0.5, 0.25]);
        assert_eq!(rig.stream.state(), AudioStreamState::Stopped);
        assert_eq!(rig.play::<4>(), [0.0; 4]);
    }

    #[test]
    fn pull_uses_the_latest_buffer_demand_when_the_worker_has_not_read_yet() {
        let mut rig = Rig::new(|pcm| {
            // The most recent callback asked for two output frames at twice the source rate:
            // one stereo source frame plus one frame of interpolation lookahead.
            assert_eq!(pcm.len(), 4);
            pcm.copy_from_slice(&[16_384, -8192, -16_384, 8192]);
            2
        });
        // Two callbacks, no producer pass in between. The smaller request replaces the larger.
        assert_eq!(rig.play::<8>(), [0.0; 8]);
        assert_eq!(rig.play::<4>(), [0.0; 4]);
        assert!(rig.refill());
        assert_eq!(
            rig.play::<8>(),
            [0.5, -0.25, 0.0, 0.0, -0.5, 0.25, -0.5, 0.25]
        );
    }

    #[test]
    fn pull_refuses_an_oversized_count_without_publishing_samples() {
        let mut rig = Rig::new(|pcm| {
            pcm.fill(16_384);
            usize::MAX
        });
        assert_eq!(rig.play::<4>(), [0.0; 4]);
        let ring = rig.job.as_ref().unwrap().ring.clone();
        assert!(!rig.refill());
        assert_eq!(ring.producer_window().1, 0);
        assert_eq!(rig.stream.state(), AudioStreamState::Stopped);
        assert_eq!(rig.play::<4>(), [0.0; 4]);
    }

    #[test]
    fn invalid_read_silences_queued_pcm_before_source_disposal() {
        struct Invalid {
            first: bool,
            dropping: SyncSender<()>,
            finish: mpsc::Receiver<()>,
        }

        impl AudioStream for Invalid {
            fn read(&mut self, pcm: &mut [i16]) -> usize {
                if std::mem::take(&mut self.first) {
                    pcm[..2].fill(16_384);
                    1
                } else {
                    usize::MAX
                }
            }
        }

        impl Drop for Invalid {
            fn drop(&mut self) {
                let _ = self.dropping.send(());
                let _ = self.finish.recv();
            }
        }

        let (dropping, drop_started) = mpsc::sync_channel(1);
        let (finish, finish_drop) = mpsc::sync_channel(1);
        let pool = StreamPool::default();
        let (worker, queued_jobs) = Worker::test_pair();
        pool.state().worker = Some(worker);
        let mixer = Mutex::new(Mixer {
            stream_gain: [1.0; 11],
            ..Mixer::default()
        });
        let spec = StreamSpec::new(SoundSource::Voice, 2, 48_000);
        let stream = pool
            .open(
                &mixer,
                "mod",
                &spec,
                None,
                Box::new(Invalid {
                    first: true,
                    dropping,
                    finish: finish_drop,
                }),
            )
            .unwrap();
        let mut job = queued_jobs.try_recv().unwrap();
        let mut out = [0.0; 4];
        mix_pieces(&mut mixer.lock().unwrap(), &mut out, 2, 48_000);
        assert!(job.refill(&mut [0; 16]));
        assert_eq!(job.ring.producer_window().1, 1);
        job.demand.frames.store(2, Release);
        let control = job.control.clone();
        let worker = thread::spawn(move || {
            assert!(!job.refill(&mut [0; 16]));
            drop(job);
        });
        drop_started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!control.job_retired.load(Acquire));
        assert_eq!(stream.state(), AudioStreamState::Stopped);
        mix_pieces(&mut mixer.lock().unwrap(), &mut out, 2, 48_000);
        assert_eq!(out, [0.0; 4]);
        finish.send(()).unwrap();
        worker.join().unwrap();
        assert!(control.job_retired.load(Acquire));
    }

    #[test]
    fn panicking_read_queues_the_shared_attributed_diagnostic() {
        mod_api::testing::with_dispatch(Default::default(), || {
            let mut rig = Rig::new(|_| panic!("test audio source panic"));
            assert_eq!(rig.play::<4>(), [0.0; 4]);
            assert!(!rig.refill());
            assert_eq!(rig.stream.state(), AudioStreamState::Stopped);
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "mod",
                    hook: "AudioStream::read",
                }]
            );
        });
    }

    #[test]
    fn pull_panic_stops_only_the_panicking_stream() {
        struct Panicker(SyncSender<()>);

        impl AudioStream for Panicker {
            fn read(&mut self, _pcm: &mut [i16]) -> usize {
                panic!("test audio source panic");
            }
        }

        impl Drop for Panicker {
            fn drop(&mut self) {
                let _ = self.0.try_send(());
            }
        }

        let (panicker_drop, panicker_dropped) = mpsc::sync_channel(1);
        let (healthy_read, healthy_reads) = mpsc::sync_channel(4);
        let pool = StreamPool::default();
        let mixer = Mutex::new(Mixer {
            stream_gain: [1.0; 11],
            ..Mixer::default()
        });
        let spec = StreamSpec::new(SoundSource::Voice, 2, 24_000);
        let panicker = pool
            .open(
                &mixer,
                "bad_mod",
                &spec,
                None,
                Box::new(Panicker(panicker_drop)),
            )
            .unwrap();
        let healthy = pool
            .open(
                &mixer,
                "good_mod",
                &spec,
                None,
                Box::new(Source(move |pcm: &mut [i16]| {
                    pcm[..2].copy_from_slice(&[16_384, -8_192]);
                    let _ = healthy_read.try_send(());
                    1
                })),
            )
            .unwrap();

        let timeout = Duration::from_secs(2);
        let mut out = [0.0; 8];
        mix_pieces(&mut mixer.lock().unwrap(), &mut out, 2, 48_000);
        panicker_dropped
            .recv_timeout(timeout)
            .expect("panicking source was not dropped");
        healthy_reads
            .recv_timeout(timeout)
            .expect("healthy source was not read after sibling panicked");
        assert_eq!(panicker.state(), AudioStreamState::Stopped);
        assert_eq!(healthy.state(), AudioStreamState::Playing);

        mix_pieces(&mut mixer.lock().unwrap(), &mut out, 2, 48_000);
        if out == [0.0; 8] {
            // If the worker had not yet published read #1 before the mix, wait for read #2
            // which runs serially on the same worker after read #1's publish.
            healthy_reads
                .recv_timeout(timeout)
                .expect("healthy source was not read on follow-up demand");
            mix_pieces(&mut mixer.lock().unwrap(), &mut out, 2, 48_000);
        }
        assert_eq!(out[..4], [0.5, -0.25, 0.5, -0.25]);
        assert_eq!(healthy.state(), AudioStreamState::Playing);
        healthy.stop();
    }

    #[test]
    fn pull_clamps_each_read_to_scratch_capacity_without_losing_demand() {
        let mut chunks = [16, 6].into_iter();
        let mut rig = Rig::new(move |pcm| {
            // 20 output stereo frames at 48 kHz demand 11 source frames at 24 kHz, or
            // 8 and 3 frames in two reads with the rig's scratch buffer.
            assert_eq!(pcm.len(), chunks.next().unwrap());
            pcm.fill(16_384);
            pcm.len() / 2
        });
        assert_eq!(rig.play::<40>(), [0.0; 40]);
        assert!(rig.refill());
        assert!(rig.refill());
        assert_eq!(rig.job.as_ref().unwrap().ring.producer_window().1, 11);
        assert_eq!(rig.play::<40>(), [0.5; 40]);
    }

    #[test]
    fn high_rate_pull_crosses_chunk_and_ring_capacity_without_losing_source_time() {
        // At this output rate, even one step at u32::MAX Hz passes more than 384,000
        // source frames (ring capacity) and several 48,000-frame stereo scratch chunks.
        const OUTPUT_RATE: u32 = 11_025;
        const END: usize = 779_133;
        fn signal(index: usize) -> i16 {
            ((index % 4_096) as i16 - 2_048) * 4
        }

        struct Source {
            next: usize,
            finished: SyncSender<usize>,
        }
        impl AudioStream for Source {
            fn read(&mut self, buffer: &mut [i16]) -> usize {
                assert!(buffer.len() <= SCRATCH_SAMPLES);
                let frames = (END - self.next).min(buffer.len() / 2);
                for frame in buffer[..frames * 2].as_chunks_mut::<2>().0 {
                    let left = signal(self.next);
                    frame.copy_from_slice(&[left, -left]);
                    self.next += 1;
                }
                frames
            }
        }
        impl Drop for Source {
            fn drop(&mut self) {
                let _ = self.finished.try_send(self.next);
            }
        }

        let pool = StreamPool::default();
        let mut out = [0.0; 16];
        let mixer = Mutex::new(Mixer {
            stream_gain: [1.0; 11],
            ..Mixer::default()
        });
        let (finished, consumed) = mpsc::sync_channel(1);
        let spec = StreamSpec::new(SoundSource::Voice, 2, u32::MAX);
        let stream = pool
            .open(
                &mixer,
                "mod",
                &spec,
                None,
                Box::new(Source { next: 0, finished }),
            )
            .unwrap();

        // One callback, one wake. The real worker must continue through all chunks to EOF
        // even if the device does not call back again before the queued PCM is ready.
        mix_pieces(&mut mixer.lock().unwrap(), &mut out, 2, OUTPUT_RATE);
        assert_eq!(out, [0.0; 16]);
        assert_eq!(consumed.recv_timeout(Duration::from_secs(1)).unwrap(), END);
        // Source::drop signals before the worker publishes EOF. Wait for publication
        // without mixing again, so all PCM remains queued for the assertions below.
        let control = pool.state().controls[0].clone();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !control.pull.eof.load(Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not publish EOF"
            );
            thread::yield_now();
        }
        assert_eq!(stream.state(), AudioStreamState::Playing);

        let mut first = [0.0; 4];
        mix_pieces(&mut mixer.lock().unwrap(), &mut first, 2, OUTPUT_RATE);
        assert_eq!(stream.state(), AudioStreamState::Playing);
        mix_pieces(&mut mixer.lock().unwrap(), &mut out, 2, OUTPUT_RATE);
        let samples: Vec<_> = first.into_iter().chain(out).collect();
        // Source positions of successive outputs. At the third position, the fractional
        // offset would need source frame 779,133, so EOF instead holds frame 779,132.
        let expected = [(0, 0), (389_566, 2_145), (779_132, 0)];
        for (frame, (index, remainder)) in samples.as_chunks::<2>().0.iter().zip(expected) {
            let a = f32::from(signal(index)) / 32768.0;
            let b = f32::from(signal(index + 1)) / 32768.0;
            let left = a + (b - a) * (remainder as f32 / OUTPUT_RATE as f32);
            assert_eq!(*frame, [left, -left], "source frame {index}");
        }
        assert_eq!(samples[6..], [0.0; 14]);
        assert_eq!(stream.state(), AudioStreamState::Stopped);
    }
}
