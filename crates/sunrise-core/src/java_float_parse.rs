//! Java floating-point string syntax, including hexadecimal significands.

/// JDK `Double.parseDouble` for Java's decimal and hexadecimal syntax.
#[inline]
pub fn parse_java_double(text: &str) -> Option<f64> {
    match literal(text)? {
        Literal::Decimal(text) => text.parse().ok(),
        Literal::Hex(negative, body) => Some(f64::from_bits(
            hex_bits(body, 52, 1023)? | ((negative as u64) << 63),
        )),
        Literal::Nan => Some(f64::NAN),
        Literal::Infinity(false) => Some(f64::INFINITY),
        Literal::Infinity(true) => Some(f64::NEG_INFINITY),
    }
}

/// JDK `Float.parseFloat`, rounded directly to binary32 without a double intermediate.
#[inline]
pub fn parse_java_float(text: &str) -> Option<f32> {
    match literal(text)? {
        Literal::Decimal(text) => text.parse().ok(),
        Literal::Hex(negative, body) => Some(f32::from_bits(
            hex_bits(body, 23, 127)? as u32 | ((negative as u32) << 31),
        )),
        Literal::Nan => Some(f32::NAN),
        Literal::Infinity(false) => Some(f32::INFINITY),
        Literal::Infinity(true) => Some(f32::NEG_INFINITY),
    }
}

enum Literal<'a> {
    Decimal(&'a str),
    Hex(bool, &'a str),
    Nan,
    Infinity(bool),
}

#[inline]
fn literal(text: &str) -> Option<Literal<'_>> {
    // String.trim discards only ASCII characters <= U+0020, not general whitespace.
    let text = text.trim_matches(|c| c <= ' ');
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let negative = text.starts_with('-');
    match body {
        "NaN" => return Some(Literal::Nan),
        "Infinity" => return Some(Literal::Infinity(negative)),
        _ => {}
    }
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        let hex = hex.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(hex);
        return Some(Literal::Hex(negative, hex));
    }
    // Rust's decimal grammar is the same once the Java suffix is removed and
    // non-finite spellings (which Rust accepts case-insensitively) are excluded.
    if !matches!(body.as_bytes().first(), Some(b'0'..=b'9' | b'.')) {
        return None;
    }
    Some(Literal::Decimal(
        text.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(text),
    ))
}

fn hex_bits(text: &str, fraction_bits: u32, bias: i64) -> Option<u64> {
    let (hex, exp) = text.split_once(['p', 'P'])?;
    let unsigned_exp = exp.strip_prefix(['+', '-']).unwrap_or(exp);
    if unsigned_exp.is_empty() {
        return None;
    }
    let mut exponent = 0i64;
    for digit in unsigned_exp.bytes() {
        if !digit.is_ascii_digit() {
            return None;
        }
        exponent = exponent
            .saturating_mul(10)
            .saturating_add(i64::from(digit - b'0'));
    }
    if exp.starts_with('-') {
        exponent = -exponent;
    }

    // The top 16 significant hex digits supply at least 61 bits when there is
    // more text. Both targets need at most 53 bits plus guard and sticky bits.
    let mut head = 0u64;
    let mut stored = 0;
    let mut discarded = 0i64;
    let mut sticky = false;
    let mut fractional = false;
    let mut digits = false;
    for c in hex.bytes() {
        let nibble = match c {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'F' => c - b'A' + 10,
            b'a'..=b'f' => c - b'a' + 10,
            b'.' if !fractional => {
                fractional = true;
                continue;
            }
            _ => return None,
        };
        digits = true;
        if fractional {
            exponent = exponent.saturating_sub(4);
        }
        if stored == 0 && nibble == 0 {
            continue;
        }
        if stored < 16 {
            head = (head << 4) | u64::from(nibble);
            stored += 1;
        } else {
            discarded = discarded.saturating_add(4);
            sticky |= nibble != 0;
        }
    }
    if head == 0 {
        return digits.then_some(0);
    }

    let significant = i64::from(head.ilog2()) + 1;
    let top = exponent
        .saturating_add(discarded)
        .saturating_add(significant - 1);
    if top > bias {
        return Some(((bias * 2 + 1) as u64) << fraction_bits);
    }
    let normal_min = 1 - bias;
    let quantum = top.max(normal_min) - i64::from(fraction_bits);
    if top < quantum - 1 {
        return Some(0);
    }
    let keep = top - quantum + 1;
    let shift = significant - keep;
    let rounded = if shift <= 0 {
        head << -shift
    } else {
        let kept = head.checked_shr(shift as u32).unwrap_or(0);
        let half = 1u64 << (shift - 1);
        let remainder = head & (half | (half - 1));
        kept + u64::from(remainder > half || (remainder == half && (sticky || kept & 1 != 0)))
    };
    if top < normal_min {
        // The rounded subnormal can carry into the first normal bit pattern.
        Some(rounded)
    } else {
        // The hidden leading bit contributes the exponent's last bit. A carry
        // out of the significand advances the exponent (even into infinity).
        Some((((top + bias - 1) as u64) << fraction_bits) + rounded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_java_float_literals_without_intermediate_rounding() {
        // Independent Temurin 25 parseDouble/parseFloat results, in raw bits.
        for (text, double, float) in [
            (
                "\0\t-0x1.8p1D\r",
                Some(0xc008_0000_0000_0000),
                Some(0xc040_0000),
            ),
            (
                "0x1.000001p0",
                Some(0x3ff0_0000_1000_0000),
                Some(0x3f80_0000),
            ),
            (
                "0x1.00000100000000001p0",
                Some(0x3ff0_0000_1000_0000),
                Some(0x3f80_0001),
            ),
            ("0x1.8p-1074", Some(2), Some(0)),
            ("-0x1p-1075", Some(0x8000_0000_0000_0000), Some(0x8000_0000)),
            (
                "0x1.ffffffp127",
                Some(0x47ef_ffff_f000_0000),
                Some(0x7f80_0000),
            ),
            (
                "1.5658424496650695800781250000001D",
                Some(0x3ff9_0db0_d000_0000),
                Some(0x3fc8_6d87),
            ),
            ("-NaN", Some(0x7ff8_0000_0000_0000), Some(0x7fc0_0000)),
            ("-NaNd", None, None),
            ("Infinityf", None, None),
            ("0x1", None, None),
            ("0x.p0", None, None),
        ] {
            assert_eq!(
                parse_java_double(text).map(f64::to_bits),
                double,
                "{text:?}"
            );
            assert_eq!(parse_java_float(text).map(f32::to_bits), float, "{text:?}");
        }
    }
}
