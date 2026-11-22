//! JSON integer conversion shared by loot codecs and their evaluators.

/// Decode the signed 32-bit value used by `Codec.INT` and integer ranges.
///
/// The result is widened for JSON-facing integer/range callers. Integer tokens
/// retain their i64/u64 precision; floating representations are truncated and
/// narrowed modulo 2^32, rather than using Rust's saturating float cast. Decimal
/// information already lost by serde_json parsing is tracked in #2105.
pub fn int_value(number: &serde_json::Number) -> i64 {
    if let Some(value) = number.as_i64() {
        return i64::from(value as i32);
    }
    if let Some(value) = number.as_u64() {
        return i64::from(value as i32);
    }
    let bits = number.as_f64().unwrap_or(0.0).to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if !(0..84).contains(&exponent) {
        return 0;
    }
    let significand = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
    let mut result = if exponent >= 52 {
        (significand as u32).wrapping_shl((exponent - 52) as u32)
    } else {
        (significand >> (52 - exponent)) as u32
    };
    if bits >> 63 != 0 {
        result = result.wrapping_neg();
    }
    i64::from(result as i32)
}

#[cfg(test)]
mod tests {
    use super::int_value;

    #[test]
    fn json_int_conversion_pins_controls_and_issue_2105_precision_gaps() {
        // Independent values from the #2105 probe table. The known-gap rows
        // deliberately pin today's representation, not an unimplemented fix. A row whose float
        // parse rounds differently with serde_json's `float_roundtrip` (on in the workspace build,
        // off in this crate's own) is left out: "9007199254740993.0" reads 2 or 0 by build.
        for (token, expected) in [
            ("2.0", 2),
            ("1e1", 10),
            ("1.9", 1),
            ("-1.9", -1),
            ("4294967297", 1),
            ("9007199254740993", 1),
            ("9007199254740995.0", 4), // #2105: Java gives 3.
            ("1e20", 1661992960),
            ("-1e20", -1661992960),
            ("18446744073709551616", 0),
            ("18446744073709551617", 0),   // #2105: Java gives 1.
            ("18446744073709551615.0", 0), // #2105: Java gives -1.
            ("1.9999999999999999999", 2),  // #2105: Java gives 1.
        ] {
            let number: serde_json::Number = serde_json::from_str(token).unwrap();
            assert_eq!(int_value(&number), expected, "{token}; see #2105");
        }
    }
}
