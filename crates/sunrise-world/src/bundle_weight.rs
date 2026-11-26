//! Bundle weight arithmetic and lazy caching. The checked integer operations
//! retain the pinned Fraction behavior, including intermediate overflow errors.
//!
//! [`Fraction`] is commons-lang3's `org.apache.commons.lang3.math.Fraction` (3.20.0, the
//! version 26.3 ships): every `ArithmeticException` it throws is an [`ExcessiveWeight`], the
//! error `BundleContents.computeContentWeight` turns into
//! `DataResult.error("Excessive total bundle weight")`.
use std::{cell::OnceCell, rc::Rc};

/// commons-lang3's `Fraction`: an `int` numerator over a positive `int` denominator. Equality
/// compares the parts as stored, like `Fraction.equals`; [`Fraction::compare_to`] compares values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fraction {
    numerator: i32,
    denominator: i32,
}
/// `DataResult.error("Excessive total bundle weight")`: a [`Fraction`] operation overflowed
/// an `int` (commons-lang3 throws `ArithmeticException`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExcessiveWeight;

impl std::fmt::Display for ExcessiveWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Excessive total bundle weight")
    }
}

impl std::error::Error for ExcessiveWeight {}
fn gcd(a: i32, b: i32) -> i32 {
    let (mut a, mut b) = (i64::from(a).abs(), i64::from(b).abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    // Every call has a positive, representable denominator as one operand.
    a as i32
}
impl Fraction {
    pub fn numerator(self) -> i32 {
        self.numerator
    }
    pub fn denominator(self) -> i32 {
        self.denominator
    }
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };
    /// `Fraction.getFraction(int, int)`: a negative denominator moves its sign to the
    /// numerator; the parts are not reduced.
    pub fn new(mut numerator: i32, mut denominator: i32) -> Result<Self, ExcessiveWeight> {
        if denominator == 0 {
            return Err(ExcessiveWeight);
        }
        if denominator < 0 {
            numerator = numerator.checked_neg().ok_or(ExcessiveWeight)?;
            denominator = denominator.checked_neg().ok_or(ExcessiveWeight)?;
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }
    /// `Fraction.add`.
    pub fn checked_add(self, other: Self) -> Result<Self, ExcessiveWeight> {
        self.add_sub(other, true)
    }
    /// `Fraction.subtract`: the same `addSub` as [`Self::checked_add`], so it overflows
    /// where commons-lang3 throws, including `negate` of an `i32::MIN` numerator when this
    /// fraction is zero.
    pub fn checked_subtract(self, other: Self) -> Result<Self, ExcessiveWeight> {
        self.add_sub(other, false)
    }
    /// `Fraction.addSub` (commons-lang3 3.20.0): a zero operand short-circuits, coprime
    /// denominators take checked `int` products, and the rest is Knuth's reduced sum.
    fn add_sub(self, other: Self, is_add: bool) -> Result<Self, ExcessiveWeight> {
        if self.numerator == 0 {
            return if is_add {
                Ok(other)
            } else {
                other.checked_negate()
            };
        }
        if other.numerator == 0 {
            return Ok(self);
        }
        let d1 = gcd(self.denominator, other.denominator);
        if d1 == 1 {
            // `mulAndCheck`, `addAndCheck` / `subAndCheck` and `mulPosAndCheck`.
            let a = self
                .numerator
                .checked_mul(other.denominator)
                .ok_or(ExcessiveWeight)?;
            let b = other
                .numerator
                .checked_mul(self.denominator)
                .ok_or(ExcessiveWeight)?;
            let numerator = if is_add {
                a.checked_add(b)
            } else {
                a.checked_sub(b)
            };
            Ok(Self {
                numerator: numerator.ok_or(ExcessiveWeight)?,
                denominator: self
                    .denominator
                    .checked_mul(other.denominator)
                    .ok_or(ExcessiveWeight)?,
            })
        } else {
            // Native uses BigInteger here. Each product is below 2^62 in magnitude for i32
            // numerators and positive i32 denominators, so their sum or difference fits i64.
            let a = i64::from(self.numerator) * i64::from(other.denominator / d1);
            let b = i64::from(other.numerator) * i64::from(self.denominator / d1);
            let t = if is_add { a + b } else { a - b };
            let residue = t.rem_euclid(i64::from(d1)) as i32;
            let d2 = if residue == 0 { d1 } else { gcd(residue, d1) };
            Ok(Self {
                // `bitLength() > 31` throws: the quotient must fit an int.
                numerator: i32::try_from(t / i64::from(d2)).map_err(|_| ExcessiveWeight)?,
                denominator: (self.denominator / d1)
                    .checked_mul(other.denominator / d2)
                    .ok_or(ExcessiveWeight)?,
            })
        }
    }
    /// `Fraction.negate`: an `i32::MIN` numerator has no negation.
    fn checked_negate(self) -> Result<Self, ExcessiveWeight> {
        Ok(Self {
            numerator: self.numerator.checked_neg().ok_or(ExcessiveWeight)?,
            denominator: self.denominator,
        })
    }
    /// `Fraction.invert`: zero and an `i32::MIN` numerator have no inverse; a negative
    /// fraction moves its sign to the new numerator. Unreduced, as commons-lang3 leaves it.
    fn checked_invert(self) -> Result<Self, ExcessiveWeight> {
        if self.numerator == 0 || self.numerator == i32::MIN {
            return Err(ExcessiveWeight);
        }
        Ok(if self.numerator < 0 {
            Self {
                numerator: -self.denominator,
                denominator: -self.numerator,
            }
        } else {
            Self {
                numerator: self.denominator,
                denominator: self.numerator,
            }
        })
    }
    /// `Fraction.divideBy`: `multiplyBy(other.invert())`, refusing a zero divisor.
    pub fn checked_divide_by(self, other: Self) -> Result<Self, ExcessiveWeight> {
        if other.numerator == 0 {
            return Err(ExcessiveWeight);
        }
        self.checked_multiply(other.checked_invert()?)
    }
    /// `Fraction.intValue`: the quotient truncated toward zero (`idiv`).
    pub fn int_value(self) -> i32 {
        self.numerator / self.denominator
    }
    /// `Fraction.floatValue`: both parts converted to `float` (`i2f`), then `fdiv`.
    pub fn float_value(self) -> f32 {
        self.numerator as f32 / self.denominator as f32
    }
    /// `Fraction.compareTo`: equal parts compare equal, otherwise the cross products are
    /// compared as `long`s, which cannot overflow for `int` parts.
    pub fn compare_to(self, other: Self) -> std::cmp::Ordering {
        if self == other {
            return std::cmp::Ordering::Equal;
        }
        let first = i64::from(self.numerator) * i64::from(other.denominator);
        let second = i64::from(other.numerator) * i64::from(self.denominator);
        first.cmp(&second)
    }
    /// `Fraction.multiplyBy`: cross-reduced checked products, then `getReducedFraction`.
    pub fn checked_multiply(self, other: Self) -> Result<Self, ExcessiveWeight> {
        if self.numerator == 0 || other.numerator == 0 {
            return Ok(Self::ZERO);
        }
        let d1 = gcd(self.numerator, other.denominator);
        let d2 = gcd(other.numerator, self.denominator);
        let numerator = (self.numerator / d1)
            .checked_mul(other.numerator / d2)
            .ok_or(ExcessiveWeight)?;
        let denominator = (self.denominator / d2)
            .checked_mul(other.denominator / d1)
            .ok_or(ExcessiveWeight)?;
        let d = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / d,
            denominator: denominator / d,
        })
    }
}
#[derive(Debug)]
struct Data<T> {
    items: Vec<T>,
    weight: OnceCell<Result<Fraction, ExcessiveWeight>>,
}
#[derive(Debug)]
pub struct BundleWeight<T>(Rc<Data<T>>);
impl<T> Clone for BundleWeight<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
pub trait ItemWeights<T> {
    fn count(&self, item: &T) -> i32;
    fn max_stack_size(&self, item: &T) -> i32;
    fn has_bees(&self, item: &T) -> bool;
    fn bundle(&self, item: &T) -> Option<BundleWeight<T>>;
}
impl<T> BundleWeight<T> {
    pub fn new(items: Vec<T>) -> Self {
        Self(Rc::new(Data {
            items,
            weight: OnceCell::new(),
        }))
    }
    pub fn items(&self) -> &[T] {
        &self.0.items
    }
    pub fn weight(&self, view: &impl ItemWeights<T>) -> Result<Fraction, ExcessiveWeight> {
        *self.0.weight.get_or_init(|| self.compute(view))
    }
    fn compute(&self, view: &impl ItemWeights<T>) -> Result<Fraction, ExcessiveWeight> {
        let mut total = Fraction::ZERO;
        for item in self.items() {
            let weight = if let Some(bundle) = view.bundle(item) {
                bundle.weight(view)?.checked_add(Fraction {
                    numerator: 1,
                    denominator: 16,
                })?
            } else if view.has_bees(item) {
                Fraction::ONE
            } else {
                Fraction::new(1, view.max_stack_size(item))?
            };
            let count = Fraction::new(view.count(item), 1)?;
            total = total.checked_add(weight.checked_multiply(count)?)?;
        }
        Ok(total)
    }
}

/// Contained stack sizes are checked without recursively validating their
/// components. Bundle weight is evaluated only after all its item sizes pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentsError {
    Size { count: i32, maximum: i32 },
    Weight(ExcessiveWeight),
}
pub fn validate_sizes<T>(items: &[T], view: &impl ItemWeights<T>) -> Result<(), ContentsError> {
    for item in items {
        let count = view.count(item);
        let maximum = view.max_stack_size(item);
        if count > maximum {
            return Err(ContentsError::Size { count, maximum });
        }
    }
    Ok(())
}
pub fn validate_contents<T>(
    container: Option<&[T]>,
    bundle: Option<&BundleWeight<T>>,
    projectiles: Option<&[T]>,
    view: &impl ItemWeights<T>,
) -> Result<(), ContentsError> {
    if let Some(items) = container {
        validate_sizes(items, view)?;
    }
    if let Some(bundle) = bundle {
        validate_sizes(bundle.items(), view)?;
        bundle.weight(view).map_err(ContentsError::Weight)?;
    }
    if let Some(items) = projectiles {
        validate_sizes(items, view)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ExcessiveWeight, Fraction};
    use std::cmp::Ordering;

    fn f(numerator: i32, denominator: i32) -> Fraction {
        Fraction::new(numerator, denominator).unwrap()
    }
    fn parts(fraction: Fraction) -> (i32, i32) {
        (fraction.numerator(), fraction.denominator())
    }

    /// `addSub(.., false)`: coprime denominators take checked products, shared ones Knuth's
    /// reduced difference, and a zero minuend negates the subtrahend.
    #[test]
    fn subtract_follows_add_sub() {
        assert_eq!(parts(f(1, 1).checked_subtract(f(3, 4)).unwrap()), (1, 4));
        assert_eq!(parts(f(3, 4).checked_subtract(f(1, 4)).unwrap()), (1, 2));
        assert_eq!(parts(f(1, 4).checked_subtract(f(0, 1)).unwrap()), (1, 4));
        assert_eq!(parts(f(0, 1).checked_subtract(f(5, 4)).unwrap()), (-5, 4));
        // `negate` has no answer for i32::MIN, `subAndCheck` none past the int range, and
        // `mulPosAndCheck` none for a denominator product past i32::MAX (a prime).
        assert_eq!(
            f(0, 1).checked_subtract(f(i32::MIN, 1)),
            Err(ExcessiveWeight)
        );
        assert_eq!(
            f(i32::MIN, 1).checked_subtract(f(1, 1)),
            Err(ExcessiveWeight)
        );
        assert_eq!(
            f(1, 3).checked_subtract(f(1, i32::MAX)),
            Err(ExcessiveWeight)
        );
        // The add path is the same `addSub`.
        assert_eq!(f(i32::MAX, 1).checked_add(f(1, 1)), Err(ExcessiveWeight));
        assert_eq!(parts(f(1, 6).checked_add(f(1, 3)).unwrap()), (1, 2));
    }

    /// `divideBy` is `multiplyBy(invert())`; zero and `i32::MIN` have no inverse.
    #[test]
    fn divide_by_multiplies_by_the_inverse() {
        assert_eq!(parts(f(1, 1).checked_divide_by(f(1, 64)).unwrap()), (64, 1));
        assert_eq!(parts(f(9, 4).checked_divide_by(f(4, 1)).unwrap()), (9, 16));
        assert_eq!(parts(f(1, 1).checked_divide_by(f(-3, 4)).unwrap()), (-4, 3));
        assert_eq!(f(1, 1).checked_divide_by(f(0, 1)), Err(ExcessiveWeight));
        assert_eq!(
            f(1, 1).checked_divide_by(f(i32::MIN, 1)),
            Err(ExcessiveWeight)
        );
        // `mulAndCheck(i32::MAX, 64)` and `mulPosAndCheck(64, 2^30)` overflow.
        assert_eq!(
            f(i32::MAX, 1).checked_divide_by(f(1, 64)),
            Err(ExcessiveWeight)
        );
        assert_eq!(
            f(1, 64).checked_divide_by(f(1 << 30, 1)),
            Err(ExcessiveWeight)
        );
    }

    #[test]
    fn int_float_and_compare_read_the_parts_as_java_does() {
        assert_eq!(f(7, 2).int_value(), 3);
        assert_eq!(f(-7, 2).int_value(), -3);
        assert_eq!(f(9, 16).float_value(), 0.5625);
        assert_eq!(f(3, 2).float_value(), 1.5);
        // Unreduced parts compare equal by value but not as stored.
        assert_eq!(f(1, 2).compare_to(f(2, 4)), Ordering::Equal);
        assert_ne!(f(1, 2), f(2, 4));
        assert_eq!(f(1, 3).compare_to(f(1, 2)), Ordering::Less);
        assert_eq!(f(i32::MAX, 1).compare_to(f(i32::MIN, 1)), Ordering::Greater);
        assert_eq!(
            f(i32::MAX, i32::MAX - 1).compare_to(f(1, 1)),
            Ordering::Greater
        );
    }
}
