use sunrise_core::{random::java_string_hash, text::parse_java_long};

/// WorldOptions.parseSeed: trim Java's space-or-control units, then parse a
/// long or hash the exact original UTF-16. An empty string requests a new seed.
pub(crate) fn parse_seed(units: &[u16]) -> Option<i64> {
    let start = units.iter().position(|&unit| unit > 0x20)?;
    let end = units.iter().rposition(|&unit| unit > 0x20).unwrap() + 1;
    let text = &units[start..end];
    Some(
        String::from_utf16(text)
            .ok()
            .and_then(|s| parse_java_long(&s))
            .unwrap_or_else(|| i64::from(java_string_hash(text.iter().copied()))),
    )
}
