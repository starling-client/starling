//! The random a mod draws from: one trait for every stream the game lends, and the owned
//! generator a mod seeds itself.
use sunrise_core::random::{LegacyRandom, RandomSource as EngineRandom};

/// A stream of random draws, with the names and values of vanilla's `RandomSource`.
///
/// Every stream the game lends a mod is a `&mut dyn RandomSource`, and a draw moves the game's own
/// stream in vanilla's order. A mod seeds its own with [`LegacyRandomSource`].
pub trait RandomSource {
    /// `nextInt()`: any `i32`.
    fn next_int(&mut self) -> i32;
    /// `nextInt(bound)`: an `i32` in `0..bound`. Returns 0, without drawing, for a bound below 1,
    /// where vanilla throws.
    fn next_int_bound(&mut self, bound: i32) -> i32;
    /// `nextLong()`.
    fn next_long(&mut self) -> i64;
    /// `nextBoolean()`.
    fn next_boolean(&mut self) -> bool;
    /// `nextFloat()`: in `[0, 1)`.
    fn next_float(&mut self) -> f32;
    /// `nextDouble()`: in `[0, 1)`.
    fn next_double(&mut self) -> f64;
    /// `nextGaussian()`: normally distributed, mean 0 and deviation 1. The stream keeps the
    /// second value of each pair for its next call, as vanilla's does.
    fn next_gaussian(&mut self) -> f64;
    /// `Mth.randomBetweenInclusive(random, min, max)`: an `i32` in `min..=max`, drawn as
    /// `next_int_bound(max - min + 1) + min`. For `max < min` it returns `min` without drawing,
    /// where vanilla throws. The arithmetic wraps rather than panics.
    ///
    /// Computed in the mod: drawn through `next_int_bound`, never asked of the game by itself.
    #[sunrise_mod_api_macros::computed_in_mod]
    fn next_int_between_inclusive(&mut self, min: i32, max: i32) -> i32 {
        if max < min {
            return min;
        }
        self.next_int_bound(max.wrapping_sub(min).wrapping_add(1))
            .wrapping_add(min)
    }
    /// `nextInt(origin, bound)`: an `i32` in `origin..bound`, drawn as
    /// `next_int_bound(bound - origin) + origin`. For `bound <= origin` it returns `origin`
    /// without drawing, where vanilla throws. The arithmetic wraps rather than panics.
    ///
    /// Computed in the mod: drawn through `next_int_bound`, never asked of the game by itself.
    #[sunrise_mod_api_macros::computed_in_mod]
    fn next_int_range(&mut self, origin: i32, bound: i32) -> i32 {
        if origin >= bound {
            return origin;
        }
        self.next_int_bound(bound.wrapping_sub(origin))
            .wrapping_add(origin)
    }
}

/// A one-shot request from `with_random`, scoped to the call on the caller's stack.
/// A level host can run it with the live stream or decline by returning without running it.
// no wire: the callback is borrowed from the caller's stack.
// exhaustive: only the scoped adapter can construct a request.
pub struct RandomDraw<'a>(&'a mut dyn FnMut(&mut dyn RandomSource));

impl RandomDraw<'_> {
    /// Consume this request, lending `random` directly for the closure's duration. Draws
    /// advance the live stream as vanilla `Level.getRandom()` does and are not rolled back
    /// on panic/unwind or transaction abort.
    pub fn run(self, random: &mut dyn RandomSource) {
        (self.0)(random);
    }
}

pub(crate) fn with_random_draw<T>(
    draw: impl FnOnce(&mut dyn RandomSource) -> T,
    lend: impl FnOnce(RandomDraw<'_>),
) -> Option<T> {
    let mut draw = Some(draw);
    let mut result = None;
    lend(RandomDraw(&mut |random| {
        result = draw.take().map(|draw| draw(random));
    }));
    result
}

/// Vanilla's `LegacyRandomSource`: the generator Java's `Random` is, seeded by the mod.
///
/// Own one of these if you choose the seed. It draws independently of the level's live
/// stream. Two sources with the same seed draw the same values. Its Wire form carries its
/// own 48-bit state and cached Gaussian.
#[derive(Clone, Debug, sunrise_mod_api_macros::Wire)]
#[wire(via = (u64, Option<f64>))]
// exhaustive: its one field is private, so mods cannot build or destructure it
pub struct LegacyRandomSource(LegacyRandom);

impl LegacyRandomSource {
    /// A source seeded as `new LegacyRandomSource(seed)` is.
    pub fn new(seed: i64) -> Self {
        Self(LegacyRandom::new(seed))
    }
    /// `setSeed(seed)`: restart the stream from `seed`, dropping any cached gaussian.
    pub fn set_seed(&mut self, seed: i64) {
        EngineRandom::set_seed(&mut self.0, seed);
    }
}

/// The whole generator: its 48-bit state and the cached second gaussian, the pair the Wire form
/// carries.
impl From<&LegacyRandomSource> for (u64, Option<f64>) {
    fn from(source: &LegacyRandomSource) -> Self {
        Self::from(&source.0)
    }
}

impl From<(u64, Option<f64>)> for LegacyRandomSource {
    fn from(state: (u64, Option<f64>)) -> Self {
        Self(LegacyRandom::from(state))
    }
}

impl RandomSource for LegacyRandomSource {
    fn next_int(&mut self) -> i32 {
        BorrowedRandom(&mut self.0).next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        BorrowedRandom(&mut self.0).next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        BorrowedRandom(&mut self.0).next_long()
    }
    fn next_boolean(&mut self) -> bool {
        BorrowedRandom(&mut self.0).next_boolean()
    }
    fn next_float(&mut self) -> f32 {
        BorrowedRandom(&mut self.0).next_float()
    }
    fn next_double(&mut self) -> f64 {
        BorrowedRandom(&mut self.0).next_double()
    }
    fn next_gaussian(&mut self) -> f64 {
        BorrowedRandom(&mut self.0).next_gaussian()
    }
}
// no wire: borrowed RNG adapter used only when crossing into a native callback.
// hidden: the renderer lends its provider RNG through the native callback.
#[doc(hidden)]
// exhaustive: a game-only adapter around the precise engine RNG loan.
pub struct BorrowedRandom<'a>(pub &'a mut LegacyRandom);
impl RandomSource for BorrowedRandom<'_> {
    fn next_int(&mut self) -> i32 {
        EngineRandom::next_int(self.0)
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        EngineRandom::next_int_bound(self.0, bound).unwrap_or(0)
    }
    fn next_long(&mut self) -> i64 {
        EngineRandom::next_long(self.0)
    }
    fn next_boolean(&mut self) -> bool {
        EngineRandom::next_boolean(self.0)
    }
    fn next_float(&mut self) -> f32 {
        EngineRandom::next_float(self.0)
    }
    fn next_double(&mut self) -> f64 {
        EngineRandom::next_double(self.0)
    }
    fn next_gaussian(&mut self) -> f64 {
        EngineRandom::next_gaussian(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Reader, Writer};
    use crate::{BlockEntityRef, Compound, LevelHost, LevelRandomHost};

    #[test]
    fn a_host_without_a_stream_declines_without_running_the_draw() {
        struct Empty;
        impl LevelHost for Empty {
            fn block_entity_data(&self, _: BlockEntityRef) -> Option<Compound> {
                None
            }
        }
        impl LevelRandomHost for Empty {}

        let host: &mut dyn LevelRandomHost = &mut Empty;
        let mut called = false;
        assert_eq!(host.with_random(|_| called = true), None);
        assert!(!called);
    }

    #[test]
    fn a_bound_below_one_answers_zero_and_draws_nothing() {
        let mut source = LegacyRandomSource::new(7);
        let random: &mut dyn RandomSource = &mut source;
        assert_eq!(random.next_int_bound(0), 0);
        assert_eq!(random.next_int_bound(i32::MIN), 0);
        let mut fresh = LegacyRandom::new(7);
        assert_eq!(random.next_long(), EngineRandom::next_long(&mut fresh));
    }

    #[test]
    fn draws_through_the_trait_object_are_the_engine_sequence() {
        let mut source = LegacyRandomSource::new(0x5eed);
        let random: &mut dyn RandomSource = &mut source;
        let mut engine = LegacyRandom::new(0x5eed);
        assert_eq!(random.next_int(), EngineRandom::next_int(&mut engine));
        assert_eq!(
            random.next_int_bound(100),
            EngineRandom::next_int_bound(&mut engine, 100).unwrap()
        );
        assert_eq!(random.next_long(), EngineRandom::next_long(&mut engine));
        assert_eq!(
            random.next_boolean(),
            EngineRandom::next_boolean(&mut engine)
        );
        assert_eq!(random.next_float(), EngineRandom::next_float(&mut engine));
        assert_eq!(random.next_double(), EngineRandom::next_double(&mut engine));
        assert_eq!(
            random.next_gaussian(),
            EngineRandom::next_gaussian(&mut engine)
        );
        assert_eq!(
            random.next_gaussian(),
            EngineRandom::next_gaussian(&mut engine)
        );
    }

    #[test]
    fn range_helpers_follow_vanilla_and_answer_for_empty_ranges() {
        let mut a = LegacyRandomSource::new(3);
        let mut b = LegacyRandomSource::new(3);
        assert_eq!(
            a.next_int_between_inclusive(-4, 5),
            b.next_int_bound(10) - 4
        );
        assert_eq!(a.next_int_range(10, 20), b.next_int_bound(10) + 10);
        // Empty ranges answer the lower end and draw nothing.
        assert_eq!(a.next_int_between_inclusive(5, 4), 5);
        assert_eq!(a.next_int_between_inclusive(i32::MAX, i32::MIN), i32::MAX);
        assert_eq!(a.next_int_range(8, 8), 8);
        assert_eq!(a.next_int_range(9, -9), 9);
        assert_eq!(a.next_int_range(i32::MIN, i32::MAX), i32::MIN);
        assert_eq!(a.next_long(), b.next_long());
    }

    #[test]
    fn the_owned_source_crosses_wire_as_seed_and_gaussian() {
        let mut source = LegacyRandomSource::new(11);
        source.next_gaussian();
        let state = <(u64, Option<f64>)>::from(&source);
        assert!(state.1.is_some());
        let mut tuple = Writer::new();
        tuple.put(&state);
        let mut out = Writer::new();
        out.put(&source);
        assert_eq!(out.bytes(), tuple.bytes());
        let mut back: LegacyRandomSource = Reader::decode(out.bytes()).unwrap();
        assert_eq!(back.next_gaussian(), source.next_gaussian());
        assert_eq!(back.next_long(), source.next_long());
    }
}
