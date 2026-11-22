//! The shared reverbs sounds send to (`SoundEffects`' `room`, `hall` and `cavern`): one
//! feedback delay network each, eight delay lines mixed through an orthogonal (Hadamard)
//! matrix, each line turned down so the whole tail falls 60 dB in the bus's decay time and
//! low-passed in the loop so high frequencies die first, as in a real room.
//!
//! The delay lines are allocated once, when the mixer is created ([`Reverb::new`]), long enough
//! for the highest output rate (192 kHz), so the output callback never allocates. Each line holds
//! `ceil(ms * size * 192) + 64` samples: room 36,596, hall 78,690 and cavern 120,785 samples for
//! their eight lines, 236,071 `f32` in all (944,284 bytes, about 922 KiB). A slower output uses
//! the start of each line. A bus with nothing sent to it, whose tail has died away, costs one
//! check per buffer.
//!
//! Each bus's `decay` is its 60 dB decay time at low frequencies. The one-pole low-pass in the
//! loop (unity at 0 Hz, about -3 dB at `damping` on each pass) makes higher frequencies die sooner.

/// How many shared reverbs there are: room, hall, cavern.
pub(super) const BUSES: usize = 3;
/// Delay lines per reverb.
const LINES: usize = 8;
/// The highest output rate the delay lines are sized for; a faster output gets shorter lines.
const MAX_RATE: f32 = 192_000.0;
/// The lines' lengths at size 1, in milliseconds: spread so that no two share a short common
/// period, before rounding to a prime number of frames.
const LINE_MS: [f32; LINES] = [23.1, 27.7, 31.9, 36.3, 41.3, 45.7, 50.9, 56.3];
/// The tail counts as gone below this level (-120 dB), with no input.
const SILENT: f32 = 1e-6;
/// The level each send enters the network at, and the level of its output: the network's
/// eight lines add up, and the output takes four of them on each side with each sign.
const INPUT: f32 = 0.25;
const OUTPUT: f32 = 0.35;

/// A shared reverb's room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Room {
    /// Seconds for the tail to fall 60 dB.
    pub(super) decay: f32,
    /// The lines' length, relative to [`LINE_MS`]: larger rooms reflect less often.
    pub(super) size: f32,
    /// Where the walls start absorbing high frequencies, in hertz.
    pub(super) damping: f32,
}

/// The three buses, in `SoundEffects`' order.
pub(super) const ROOMS: [Room; BUSES] = [
    // A small room: a house, a corridor.
    Room {
        decay: 0.6,
        size: 0.6,
        damping: 7_000.0,
    },
    // A hall: a large build, a ravine.
    Room {
        decay: 1.8,
        size: 1.3,
        damping: 4_500.0,
    },
    // A cavern: a deep cave, a stronghold.
    Room {
        decay: 4.0,
        size: 2.0,
        damping: 3_000.0,
    },
];

/// One shared reverb.
pub(super) struct Reverb {
    room: Room,
    /// Every line's samples, one after another.
    memory: Vec<f32>,
    start: [usize; LINES],
    len: [usize; LINES],
    at: [usize; LINES],
    /// Each line's gain per pass, from the decay time.
    gain: [f32; LINES],
    /// The loop's one-pole low-pass coefficient and each line's state.
    damp: f32,
    lp: [f32; LINES],
    /// The output rate the lines are set for; 0 before the first buffer.
    rate: u32,
    /// Consecutive frames with no input or appreciable signal in the tail.
    quiet_frames: usize,
    /// The longest delay: shorter gaps can still hide a pending reflection.
    longest_delay: usize,
    /// Its tail has died away and nothing is sent to it: it is skipped.
    idle: bool,
}

/// The first prime at or above `n`.
fn prime_from(n: usize) -> usize {
    let prime = |k: usize| {
        k >= 2
            && (2..)
                .take_while(|d| d * d <= k)
                .all(|d| !k.is_multiple_of(d))
    };
    (n..).find(|&k| prime(k)).unwrap_or(n)
}

impl Reverb {
    pub(super) fn new(room: Room) -> Self {
        let frames = |ms: f32| (ms * room.size * MAX_RATE / 1000.0).ceil() as usize + 64;
        let capacity = LINE_MS.iter().map(|&ms| frames(ms)).sum();
        Self {
            room,
            memory: vec![0.0; capacity],
            start: [0; LINES],
            len: [1; LINES],
            at: [0; LINES],
            gain: [0.0; LINES],
            damp: 1.0,
            lp: [0.0; LINES],
            rate: 0,
            quiet_frames: 0,
            longest_delay: 1,
            idle: true,
        }
    }

    /// Sets the lines for output rate `rate` and empties them. Allocation-free: the lines fit
    /// in the memory sized for [`MAX_RATE`].
    fn set_rate(&mut self, rate: u32) {
        // The lines' lengths stop growing at MAX_RATE; their gains and the damping follow the
        // actual rate, so a faster output keeps the bus's decay time and damping.
        let (actual, r) = (rate.max(1) as f32, (rate as f32).min(MAX_RATE));
        let mut start = 0;
        for (i, &ms) in LINE_MS.iter().enumerate() {
            let len = prime_from(((ms * self.room.size * r / 1000.0) as usize).max(2));
            self.start[i] = start;
            self.len[i] = len;
            self.at[i] = 0;
            // 60 dB over `decay` seconds: each pass of `len` frames takes its share of it.
            self.gain[i] = 10f32.powf(-3.0 * len as f32 / (actual * self.room.decay));
            start += len;
        }
        debug_assert!(start <= self.memory.len());
        self.damp = 1.0 - (-std::f32::consts::TAU * self.room.damping / actual).exp();
        self.memory.fill(0.0);
        self.lp = [0.0; LINES];
        self.rate = rate;
        self.quiet_frames = 0;
        self.longest_delay = self.len.into_iter().max().unwrap_or(1);
        self.idle = true;
    }

    /// Its tail has died away and nothing was sent to it.
    pub(super) fn idle(&self) -> bool {
        self.idle
    }

    /// Adds the reverb of `input` (mono, one sample a frame) into `out` (`channels` interleaved:
    /// left and right, or the middle of both on a mono output). `sent` says whether anything was
    /// sent to it this buffer; a send of silence lets its tail die away and the bus go idle, as
    /// no send does.
    pub(super) fn run(
        &mut self,
        input: &[f32],
        sent: bool,
        out: &mut [f32],
        channels: usize,
        rate: u32,
    ) {
        if self.rate != rate {
            self.set_rate(rate);
        }
        let fed = sent && input.iter().any(|x| *x != 0.0);
        if self.idle && !fed {
            return;
        }
        let mut loudest = 0.0f32;
        let mut frames = 0;
        for (x, frame) in input.iter().zip(out.chunks_mut(channels)) {
            let mut v = [0.0f32; LINES];
            for (i, v) in v.iter_mut().enumerate() {
                *v = self.memory[self.start[i] + self.at[i]];
                loudest = loudest.max(v.abs());
            }
            // Two different sign patterns of the lines, so the two ears hear unrelated tails.
            let left = (v[0] - v[1] + v[2] - v[3] + v[4] - v[5] + v[6] - v[7]) * OUTPUT;
            let right = (v[0] + v[1] - v[2] - v[3] + v[4] + v[5] - v[6] - v[7]) * OUTPUT;
            loudest = loudest.max(left.abs()).max(right.abs());
            if channels >= 2 {
                frame[0] += left;
                frame[1] += right;
            } else {
                frame[0] += (left + right) * 0.5;
            }
            hadamard(&mut v);
            let x = x * INPUT;
            for i in 0..LINES {
                let fed = v[i] * self.gain[i];
                self.lp[i] += (fed - self.lp[i]) * self.damp;
                // Alternate signs keep the input from piling up in one of the matrix's modes.
                let input = if i % 2 == 0 { x } else { -x };
                let sample = self.lp[i] + input;
                self.memory[self.start[i] + self.at[i]] = sample;
                loudest = loudest.max(sample.abs());
                self.at[i] += 1;
                if self.at[i] == self.len[i] {
                    self.at[i] = 0;
                }
            }
            frames += 1;
        }
        self.lp.iter_mut().for_each(super::fx::flush_tiny);
        if fed || loudest >= SILENT {
            self.quiet_frames = 0;
        } else {
            self.quiet_frames += frames;
        }
        self.idle = self.quiet_frames >= self.longest_delay;
    }
}

/// The 8-point Walsh-Hadamard transform, scaled to keep energy (orthogonal): lossless
/// mixing between the lines, so only the lines' own gains decide the decay.
#[inline]
fn hadamard(v: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        for i in (0..LINES).step_by(h * 2) {
            for j in i..i + h {
                let (a, b) = (v[j], v[j + h]);
                v[j] = a + b;
                v[j + h] = a - b;
            }
        }
        h *= 2;
    }
    let scale = (LINES as f32).sqrt().recip();
    for x in v.iter_mut() {
        *x *= scale;
    }
}

#[cfg(test)]
#[path = "reverb_tests.rs"]
mod tests;
