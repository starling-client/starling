//! `Mth.floor` and `Mth.ceil`: `(int) Math.floor(v)` and `(int) Math.ceil(v)`, a float widened to a double
//! first. Java's `int` cast saturates (NaN is 0), so values past the `int` range give `i32::MIN` or `i32::MAX`.
//! A truncating conversion and a compare, where `f64::floor` is a libm call on baseline x86-64.

/// `Mth.floor(double)`.
#[inline]
pub fn floor(v: f64) -> i32 {
    let truncated = v as i32;
    if v < f64::from(truncated) {
        truncated.saturating_sub(1)
    } else {
        truncated
    }
}

/// `Mth.floor(float)`. Below 2^24 the truncation converts back exactly; above, every float is whole.
#[inline]
pub fn floor_f32(v: f32) -> i32 {
    let truncated = v as i32;
    if v < truncated as f32 {
        truncated.saturating_sub(1)
    } else {
        truncated
    }
}

/// `Mth.ceil(double)`.
#[inline]
pub fn ceil(v: f64) -> i32 {
    let truncated = v as i32;
    if v > f64::from(truncated) {
        truncated.saturating_add(1)
    } else {
        truncated
    }
}

/// `Mth.ceil(float)`.
#[inline]
pub fn ceil_f32(v: f32) -> i32 {
    let truncated = v as i32;
    if v > truncated as f32 {
        truncated.saturating_add(1)
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn past_the_int_range_saturates_and_nan_is_zero() {
        assert_eq!(
            (floor(-0.5), ceil(-0.5), floor(2.0), ceil(2.0)),
            (-1, 0, 2, 2)
        );
        assert_eq!((floor_f32(-0.5), ceil_f32(0.25)), (-1, 1));
        for (value, expected) in [(0.0_f32, 0), (-0.0_f32, 0), (2.0_f32, 2)] {
            assert_eq!((floor_f32(value), ceil_f32(value)), (expected, expected));
        }
        assert_eq!((floor(-3e9), ceil(-3e9)), (i32::MIN, i32::MIN));
        assert_eq!((floor(3e9), ceil(3e9)), (i32::MAX, i32::MAX));
        assert_eq!(floor(f64::from(i32::MIN) - 0.5), i32::MIN);
        assert_eq!(ceil(f64::from(i32::MAX) + 0.5), i32::MAX);
        assert_eq!((floor_f32(-3e9), ceil_f32(3e9)), (i32::MIN, i32::MAX));
        assert_eq!(
            (floor_f32(f32::NEG_INFINITY), ceil_f32(f32::INFINITY)),
            (i32::MIN, i32::MAX)
        );
        assert_eq!((floor(f64::NAN), ceil_f32(f32::NAN)), (0, 0));
    }
}
