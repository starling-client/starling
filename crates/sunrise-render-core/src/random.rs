//! Java's `Random` (the LCG of `java.util.Random`), the stream vanilla's sky draws from.

/// Java's `Random`: a 48-bit seed advanced by the LCG `state * 0x5deece66d + 11`.
#[derive(Clone, Debug)]
pub struct LegacyRandom {
    state: u64,
    gaussian: Option<f64>,
}

/// The whole generator: its 48-bit state and the cached second gaussian, so a copy draws on
/// exactly as the original would.
impl From<&LegacyRandom> for (u64, Option<f64>) {
    fn from(random: &LegacyRandom) -> Self {
        (random.state, random.gaussian)
    }
}
/// A generator from its state (masked to 48 bits) and cached gaussian.
impl From<(u64, Option<f64>)> for LegacyRandom {
    fn from((state, gaussian): (u64, Option<f64>)) -> Self {
        Self {
            state: state & Self::MASK,
            gaussian,
        }
    }
}

impl LegacyRandom {
    const MULTIPLIER: u64 = 0x5_deec_e66d;
    const MASK: u64 = (1 << 48) - 1;

    pub fn new(seed: i64) -> Self {
        Self {
            state: (seed as u64 ^ Self::MULTIPLIER) & Self::MASK,
            gaussian: None,
        }
    }

    /// Construct from an already-masked 48-bit state.
    pub const fn from_state(state: u64) -> Self {
        Self {
            state,
            gaussian: None,
        }
    }

    /// The 48-bit generator state (differential tests compare it).
    pub fn state(&self) -> u64 {
        self.state
    }

    /// The cached second `nextGaussian` value, if one is pending.
    pub fn cached_gaussian(&self) -> Option<f64> {
        self.gaussian
    }

    fn bits(&mut self, count: u32) -> i32 {
        self.state = self.state.wrapping_mul(Self::MULTIPLIER).wrapping_add(11) & Self::MASK;
        (self.state >> (48 - count)) as i32
    }
}

/// The draws vanilla's code makes.
pub trait RandomSource {
    fn next_int(&mut self) -> i32;
    fn next_long(&mut self) -> i64;
    fn next_boolean(&mut self) -> bool;
    fn next_float(&mut self) -> f32;
    fn next_double(&mut self) -> f64;
}

impl RandomSource for LegacyRandom {
    fn next_int(&mut self) -> i32 {
        self.bits(32)
    }

    fn next_long(&mut self) -> i64 {
        let high = i64::from(self.bits(32)) << 32;
        // Java sign-extends the low int before addition; concatenation differs.
        high.wrapping_add(i64::from(self.bits(32)))
    }

    fn next_boolean(&mut self) -> bool {
        self.bits(1) != 0
    }

    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 * (1.0 / 16_777_216.0)
    }

    fn next_double(&mut self) -> f64 {
        let value = (i64::from(self.bits(26)) << 27) + i64::from(self.bits(27));
        value as f64 * (1.0 / 9_007_199_254_740_992.0)
    }
}
