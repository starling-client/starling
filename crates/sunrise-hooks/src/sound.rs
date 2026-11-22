//! The volume of mods' audio streams, which a crate above the engine sets:
//! 1 plays it as the mods ask, 0 plays none of it. The engine owns the
//! value and reads it where it mixes; a crate pushes it when its setting changes, never per frame.
//! Nothing set, the mods' audio plays as asked.
use std::sync::atomic::{AtomicU32, Ordering};

/// A gain from 0 to 1.
struct Gain(AtomicU32);

impl Gain {
    const fn full() -> Self {
        Self(AtomicU32::new(0x3f80_0000))
    }

    fn set(&self, gain: f32) {
        let gain = if gain.is_nan() {
            0.0
        } else {
            gain.clamp(0.0, 1.0)
        };
        self.0.store(gain.to_bits(), Ordering::Relaxed);
    }

    fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// The gain on mod audio: 1.0 until a crate sets it.
static MOD_STREAM_GAIN: Gain = Gain::full();

/// Sets the gain on every mod audio stream, from 0 to 1 (anything else is clamped, NaN is 0). At 0
/// the engine mixes none of it: the streams stay open and their audio is still taken in at its own
/// pace, so a mod needs no special case and no delay builds up.
pub fn set_mod_stream_gain(gain: f32) {
    MOD_STREAM_GAIN.set(gain);
}

/// The engine's side.
pub mod engine {
    /// [`super::set_mod_stream_gain`]'s value.
    pub fn mod_stream_gain() -> f32 {
        super::MOD_STREAM_GAIN.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gain_is_one_until_set_and_stays_within_zero_and_one() {
        let gain = Gain::full();
        assert_eq!(gain.get(), 1.0);
        gain.set(0.25);
        assert_eq!(gain.get(), 0.25);
        gain.set(7.0);
        assert_eq!(gain.get(), 1.0);
        gain.set(f32::NAN);
        assert_eq!(gain.get(), 0.0);
    }
}
