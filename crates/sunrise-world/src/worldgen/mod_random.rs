//! Random-source adapters shared between engine worldgen code and mod-registered
//! placement modifiers, rule tests and position rule tests.
use std::cell::RefCell;
use std::marker::PhantomData;
use sunrise_core::position::BlockPos;
use sunrise_core::random::{
    InvalidBound, LegacyPositionalRandom, LegacyRandom, PositionalRandom, RandomSource,
};

/// Mutable access to an engine `RandomSource` — directly (`&mut R`) or through
/// a shared single-call `RefCell` (`&RefCell<&mut R>` when an output closure
/// needs the same random between calls).
pub(crate) trait RandomAccess {
    type Source: RandomSource;
    fn with_random<T>(&mut self, f: impl FnOnce(&mut Self::Source) -> T) -> T;
}
impl<R: RandomSource> RandomAccess for &mut R {
    type Source = R;
    fn with_random<T>(&mut self, f: impl FnOnce(&mut R) -> T) -> T {
        f(self)
    }
}
impl<R: RandomSource> RandomAccess for &RefCell<&mut R> {
    type Source = R;
    fn with_random<T>(&mut self, f: impl FnOnce(&mut R) -> T) -> T {
        f(&mut **self.borrow_mut())
    }
}

/// Adapts an engine `R: RandomSource` (direct `&mut R` or `&RefCell<&mut R>`)
/// into `mod_api::RandomSource`.
pub(crate) struct EngineRandom<A>(pub(crate) A);
impl<A: RandomAccess> mod_api::RandomSource for EngineRandom<A> {
    fn next_int(&mut self) -> i32 {
        self.0.with_random(RandomSource::next_int)
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            0
        } else {
            self.0.with_random(|r| r.next_int_bound(bound)).unwrap_or(0)
        }
    }
    fn next_long(&mut self) -> i64 {
        self.0.with_random(RandomSource::next_long)
    }
    fn next_boolean(&mut self) -> bool {
        self.0.with_random(RandomSource::next_boolean)
    }
    fn next_float(&mut self) -> f32 {
        self.0.with_random(RandomSource::next_float)
    }
    fn next_double(&mut self) -> f64 {
        self.0.with_random(RandomSource::next_double)
    }
    fn next_gaussian(&mut self) -> f64 {
        self.0.with_random(RandomSource::next_gaussian)
    }
}

/// Adapts a `&mut dyn mod_api::RandomSource` back into
/// `sunrise_core::random::RandomSource` for nested engine evaluations.
pub(crate) enum ApiRandom<'a> {
    Borrowed(&'a mut dyn mod_api::RandomSource),
    Forked(LegacyRandom),
}
pub(crate) struct ApiPositional<'a>(LegacyPositionalRandom, PhantomData<&'a ()>);
impl<'a> PositionalRandom for ApiPositional<'a> {
    type Source = ApiRandom<'a>;
    fn at(&self, position: BlockPos) -> Self::Source {
        ApiRandom::Forked(self.0.at(position))
    }
    fn from_seed(&self, seed: i64) -> Self::Source {
        ApiRandom::Forked(self.0.from_seed(seed))
    }
    fn from_hash_of(&self, name: &str) -> Self::Source {
        ApiRandom::Forked(self.0.from_hash_of(name))
    }
    fn from_hash_of_utf16(&self, name: &[u16]) -> Self::Source {
        ApiRandom::Forked(self.0.from_hash_of_utf16(name))
    }
}
impl<'a> RandomSource for ApiRandom<'a> {
    type Positional = ApiPositional<'a>;
    fn set_seed(&mut self, seed: i64) {
        *self = Self::Forked(LegacyRandom::new(seed));
    }
    fn next_int(&mut self) -> i32 {
        match self {
            Self::Borrowed(r) => r.next_int(),
            Self::Forked(r) => r.next_int(),
        }
    }
    fn next_int_bound(&mut self, bound: i32) -> Result<i32, InvalidBound> {
        if bound <= 0 {
            return Err(InvalidBound);
        }
        match self {
            // A mod's own `RandomSource` may answer outside `0..bound`; the
            // engine indexes with the answer, so refuse it here.
            Self::Borrowed(r) => {
                let value = r.next_int_bound(bound);
                if (0..bound).contains(&value) {
                    Ok(value)
                } else {
                    Err(InvalidBound)
                }
            }
            Self::Forked(r) => r.next_int_bound(bound),
        }
    }
    fn next_long(&mut self) -> i64 {
        match self {
            Self::Borrowed(r) => r.next_long(),
            Self::Forked(r) => r.next_long(),
        }
    }
    fn next_boolean(&mut self) -> bool {
        match self {
            Self::Borrowed(r) => r.next_boolean(),
            Self::Forked(r) => r.next_boolean(),
        }
    }
    fn next_float(&mut self) -> f32 {
        match self {
            Self::Borrowed(r) => r.next_float(),
            Self::Forked(r) => r.next_float(),
        }
    }
    fn next_double(&mut self) -> f64 {
        match self {
            Self::Borrowed(r) => r.next_double(),
            Self::Forked(r) => r.next_double(),
        }
    }
    fn next_gaussian(&mut self) -> f64 {
        match self {
            Self::Borrowed(r) => r.next_gaussian(),
            Self::Forked(r) => r.next_gaussian(),
        }
    }
    fn fork_positional(&mut self) -> Self::Positional {
        ApiPositional(
            LegacyRandom::new(self.next_long()).fork_positional(),
            PhantomData,
        )
    }
    fn fork(&mut self) -> Self {
        Self::Forked(LegacyRandom::new(self.next_long()))
    }
}
