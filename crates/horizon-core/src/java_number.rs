//! Java's floating-point spelling, shared by SNBT and JSON numeric conversions.

/// Java `Double.toString`.
pub fn java_double(d: f64) -> String {
    java_fp_string(d, format!("{d:e}"))
}
/// Java `Float.toString`.
pub fn java_float(f: f32) -> String {
    java_fp_string(f64::from(f), format!("{f:e}"))
}
fn java_fp_string(d: f64, sci: String) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if d == 0.0 {
        return if d.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    // `sci` is Rust's shortest round-trip form: d.ddde<exp>.
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if negative { "-" } else { "" };
    let a = d.abs();
    if (1e-3..1e7).contains(&a) {
        if exp >= 0 {
            let e = exp as usize;
            let (int, frac) = if digits.len() > e + 1 {
                (digits[..=e].to_string(), digits[e + 1..].to_string())
            } else {
                (format!("{digits:0<width$}", width = e + 1), String::new())
            };
            let frac = if frac.is_empty() { "0".into() } else { frac };
            format!("{sign}{int}.{frac}")
        } else {
            let zeros = "0".repeat((-exp - 1) as usize);
            format!("{sign}0.{zeros}{digits}")
        }
    } else {
        if digits.len() == 1 {
            // Select the nearest two-digit decimal, rather than padding the
            // one-digit round-trip form; Java can choose a different exponent.
            return format!("{d:.1e}").replace('e', "E");
        }
        let (first, rest) = digits.split_at(1);
        format!("{sign}{first}.{rest}E{exp}")
    }
}
