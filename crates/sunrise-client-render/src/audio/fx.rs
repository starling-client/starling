//! A sound's own effects in the mixer (`SoundPlay::effects`): a gain, a low-pass and a
//! high-pass on the direct path, and sends to the shared reverbs ([`super::reverb`]).
//!
//! Every parameter glides from where it was to where it is asked to be across one output
//! buffer, so a change never clicks. A sound whose effects are none, and have settled there,
//! takes the mixer's plain path untouched: [`VoiceFx::bypassed`].
use super::reverb::BUSES;
use mod_api::SoundEffects;

/// The cutoff, in hertz, at and above which the low-passes are open (no filtering).
pub(super) const OPEN_CUTOFF: f32 = 20_000.0;
/// The filters' damping: `1 / Q` of a Butterworth response, flat with no peak.
const DAMPING: f32 = std::f32::consts::SQRT_2;
/// Recursive state below this (-300 dB) is flushed to zero once a buffer: a filter or a loop
/// ringing down in silence would otherwise reach subnormal numbers, which some processors
/// handle many times slower, inside the output callback.
const TINY: f32 = 1e-15;

/// `state` flushed to zero if it has rung down below [`TINY`].
#[inline]
pub(super) fn flush_tiny(state: &mut f32) {
    if state.abs() < TINY {
        *state = 0.0;
    }
}

/// What one buffer's frames glide between: the voice's effects as the mixer applies them.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Params {
    /// The direct path's level: the sound's effect gain times its direct level.
    direct: f32,
    /// How much of the low-pass is heard, 0 (dry) to 1: it fades in and out with the filter.
    lowpass_mix: f32,
    /// The low-pass's warped cutoff, `tan(pi * cutoff / rate)`.
    lowpass_g: f32,
    /// As `lowpass_mix`, for the high-pass.
    highpass_mix: f32,
    highpass_g: f32,
    /// The send level to each reverb bus.
    send: [f32; BUSES],
    /// The send's one-pole low-pass coefficient (1 lets everything through).
    send_a: f32,
}

impl Params {
    /// Nothing changed: the dry sound at its own level, with no sends.
    const NONE: Self = Self {
        direct: 1.0,
        lowpass_mix: 0.0,
        lowpass_g: 0.0,
        highpass_mix: 0.0,
        highpass_g: 0.0,
        send: [0.0; BUSES],
        send_a: 1.0,
    };

    /// `effects` at output rate `rate`. A filter that is off keeps `held`'s cutoff, so it
    /// fades out at the cutoff it had instead of sweeping open as it goes. A direct-sound cutoff
    /// above 0.49 of the rate (just under its Nyquist frequency) works as that limit: the value
    /// was valid ([`SoundEffects::valid`] is in hertz, whatever the rate), and the limit is the
    /// device's, not a refusal.
    fn of(effects: &SoundEffects, rate: f32, held: &Self) -> Self {
        let warp = |hz: f32| (std::f32::consts::PI * hz.min(rate * 0.49) / rate).tan();
        let (lowpass_mix, lowpass_g) = if effects.lowpass < OPEN_CUTOFF {
            (1.0, warp(effects.lowpass))
        } else {
            (0.0, held.lowpass_g)
        };
        let (highpass_mix, highpass_g) = if effects.highpass > 0.0 {
            (1.0, warp(effects.highpass))
        } else {
            (0.0, held.highpass_g)
        };
        let send_a = if effects.reverb_lowpass < OPEN_CUTOFF {
            1.0 - (-std::f32::consts::TAU * effects.reverb_lowpass / rate).exp()
        } else {
            1.0
        };
        Self {
            direct: effects.gain * effects.direct,
            lowpass_mix,
            lowpass_g,
            highpass_mix,
            highpass_g,
            send: [effects.room, effects.hall, effects.cavern].map(|level| effects.gain * level),
            send_a,
        }
    }

    fn sends(&self) -> bool {
        self.send.iter().any(|s| *s != 0.0)
    }

    /// It changes nothing: full level, both filters faded out and no sends. The cutoffs a
    /// faded filter holds don't matter.
    fn is_none(&self) -> bool {
        self.direct == 1.0 && self.lowpass_mix == 0.0 && self.highpass_mix == 0.0 && !self.sends()
    }
}

/// A two-pole state-variable filter's state (the trapezoidal, zero-delay-feedback form):
/// stable while its cutoff moves, so a cutoff can glide every frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

/// A state-variable filter's coefficients for one warped cutoff `g`.
#[derive(Clone, Copy, Debug)]
struct SvfCoefficients {
    a1: f32,
    a2: f32,
    a3: f32,
}

impl SvfCoefficients {
    #[inline]
    fn of(g: f32) -> Self {
        let a1 = 1.0 / (1.0 + g * (g + DAMPING));
        let a2 = g * a1;
        Self { a1, a2, a3: g * a2 }
    }
}

impl Svf {
    /// Its state flushed to zero where it has rung down below [`TINY`].
    fn flush(&mut self) {
        flush_tiny(&mut self.ic1);
        flush_tiny(&mut self.ic2);
    }

    /// One sample through it: its `(low-pass, high-pass)` outputs.
    #[inline]
    fn run(&mut self, x: f32, c: SvfCoefficients) -> (f32, f32) {
        let SvfCoefficients { a1, a2, a3 } = c;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, x - DAMPING * v1 - v2)
    }
}

/// One sound's effects in the mixer: what a mod asked for, where the glide stands, and the
/// filters' state for each of the sound's channels (left and right; a mono sound uses the left).
#[derive(Clone, Debug)]
pub(super) struct VoiceFx {
    /// The effects asked for (`Backend::set_effects`).
    pub(super) target: SoundEffects,
    /// Where the last buffer ended.
    current: Params,
    /// The output rate `current` was worked out at; 0 before the voice's first buffer.
    rate: u32,
    lowpass: [Svf; 2],
    highpass: [Svf; 2],
    /// The send's one-pole low-pass state.
    send_lp: f32,
}

impl Default for VoiceFx {
    fn default() -> Self {
        Self {
            target: SoundEffects::NONE,
            current: Params::NONE,
            rate: 0,
            lowpass: [Svf::default(); 2],
            highpass: [Svf::default(); 2],
            send_lp: 0.0,
        }
    }
}

/// One buffer's glide for a voice, from [`VoiceFx::begin`]: where each parameter starts and
/// how far it moves per frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct Glide {
    at: Params,
    step: Params,
    /// The low-pass or high-pass is heard at some point of the buffer.
    lowpass: bool,
    highpass: bool,
    /// The voice sends to a reverb at some point of the buffer.
    pub(super) sends: bool,
    /// Which reverb buses it sends to at some point of the buffer.
    pub(super) sent: [bool; BUSES],
    /// The filters' coefficients, worked out again each frame only while a cutoff moves.
    lowpass_c: SvfCoefficients,
    highpass_c: SvfCoefficients,
}

impl VoiceFx {
    /// Nothing to do: no effects asked for, and none still fading out. The mixer's plain path
    /// plays the sound exactly as it did before effects existed.
    pub(super) fn bypassed(&self) -> bool {
        self.target == SoundEffects::NONE && self.current.is_none()
    }

    /// The mixer's plain path played a buffer of the voice at output rate `rate`: effects asked
    /// for later glide in from the sound as it was heard.
    pub(super) fn played(&mut self, rate: u32) {
        self.rate = rate;
    }

    /// Starts a buffer of `frames` frames at output rate `rate`: the parameters glide from
    /// where the last buffer ended to the target in equal steps. The voice's first buffer starts
    /// on the target: there is no earlier sound to glide from, so a sound started silent, or
    /// behind a wall, is silent or muffled from its first frame.
    pub(super) fn begin(&mut self, frames: usize, rate: u32) -> Glide {
        let target = Params::of(&self.target, rate as f32, &self.current);
        if self.rate == 0 {
            self.current = target;
            self.rate = rate;
        } else if self.rate != rate {
            // A new output rate: the filters' warped cutoffs depend on it.
            self.current.lowpass_g = target.lowpass_g;
            self.current.highpass_g = target.highpass_g;
            self.rate = rate;
        }
        // A filter fading in starts at its target cutoff: from silence there is nothing to sweep.
        if self.current.lowpass_mix == 0.0 {
            self.current.lowpass_g = target.lowpass_g;
        }
        if self.current.highpass_mix == 0.0 {
            self.current.highpass_g = target.highpass_g;
        }
        if !self.current.sends() {
            self.current.send_a = target.send_a;
        }
        let at = self.current;
        let n = frames.max(1) as f32;
        let d = |from: f32, to: f32| (to - from) / n;
        let step = Params {
            direct: d(at.direct, target.direct),
            lowpass_mix: d(at.lowpass_mix, target.lowpass_mix),
            lowpass_g: d(at.lowpass_g, target.lowpass_g),
            highpass_mix: d(at.highpass_mix, target.highpass_mix),
            highpass_g: d(at.highpass_g, target.highpass_g),
            send: std::array::from_fn(|i| d(at.send[i], target.send[i])),
            send_a: d(at.send_a, target.send_a),
        };
        let lowpass = at.lowpass_mix != 0.0 || target.lowpass_mix != 0.0;
        let highpass = at.highpass_mix != 0.0 || target.highpass_mix != 0.0;
        if lowpass {
            self.lowpass.iter_mut().for_each(Svf::flush);
        } else {
            self.lowpass = [Svf::default(); 2];
        }
        if highpass {
            self.highpass.iter_mut().for_each(Svf::flush);
        } else {
            self.highpass = [Svf::default(); 2];
        }
        let sent: [bool; BUSES] =
            std::array::from_fn(|i| at.send[i] != 0.0 || target.send[i] != 0.0);
        let sends = sent.contains(&true);
        if sends {
            flush_tiny(&mut self.send_lp);
        } else {
            self.send_lp = 0.0;
        }
        // The buffer ends exactly on the target, whatever rounding the steps carry.
        self.current = target;
        Glide {
            at,
            step,
            lowpass,
            highpass,
            sends,
            sent,
            lowpass_c: SvfCoefficients::of(at.lowpass_g),
            highpass_c: SvfCoefficients::of(at.highpass_g),
        }
    }

    /// One frame of the voice: `x` (`[left, right]`; a mono sound fills only the left, with
    /// `stereo` false) through the direct path's filters and level, written back into `x`, and
    /// its send to each reverb bus.
    #[inline]
    pub(super) fn frame(
        &mut self,
        glide: &mut Glide,
        x: &mut [f32; 2],
        stereo: bool,
    ) -> [f32; BUSES] {
        let p = glide.at;
        let mono = if stereo { (x[0] + x[1]) * 0.5 } else { x[0] };
        let mut send = [0.0; BUSES];
        if glide.sends {
            self.send_lp += (mono - self.send_lp) * p.send_a;
            for (out, level) in send.iter_mut().zip(p.send) {
                *out = self.send_lp * level;
            }
        }
        if glide.lowpass && glide.step.lowpass_g != 0.0 {
            glide.lowpass_c = SvfCoefficients::of(p.lowpass_g);
        }
        if glide.highpass && glide.step.highpass_g != 0.0 {
            glide.highpass_c = SvfCoefficients::of(p.highpass_g);
        }
        let used = if stereo { 2 } else { 1 };
        for (ch, s) in x.iter_mut().enumerate().take(used) {
            if glide.lowpass {
                let (low, _) = self.lowpass[ch].run(*s, glide.lowpass_c);
                *s += (low - *s) * p.lowpass_mix;
            }
            if glide.highpass {
                let (_, high) = self.highpass[ch].run(*s, glide.highpass_c);
                *s += (high - *s) * p.highpass_mix;
            }
            *s *= p.direct;
        }
        let a = &mut glide.at;
        let s = glide.step;
        a.direct += s.direct;
        a.lowpass_mix += s.lowpass_mix;
        a.lowpass_g += s.lowpass_g;
        a.highpass_mix += s.highpass_mix;
        a.highpass_g += s.highpass_g;
        for (v, d) in a.send.iter_mut().zip(s.send) {
            *v += d;
        }
        a.send_a += s.send_a;
        send
    }
}

#[cfg(test)]
#[path = "fx_tests.rs"]
mod tests;
