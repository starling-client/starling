//! JSON-backed codecs validate nesting before entering serde's recursive code.
use super::{MAX_DEPTH, Reader, WireError};
use serde_json::Value;

pub(super) fn value_depth<'a>(roots: impl IntoIterator<Item = &'a Value>, depth: u16) -> bool {
    let mut pending: Vec<_> = roots.into_iter().map(|value| (value, depth)).collect();
    while let Some((value, depth)) = pending.pop() {
        if matches!(value, Value::Array(_) | Value::Object(_)) && depth >= MAX_DEPTH {
            return false;
        }
        match value {
            Value::Array(values) => pending.extend(values.iter().map(|child| (child, depth + 1))),
            Value::Object(values) => {
                pending.extend(values.values().map(|child| (child, depth + 1)))
            }
            _ => {}
        }
    }
    true
}

// Lexical preflight is not a JSON parser: serde still owns syntax validation.
// Quoted brackets and escaped quotes must not count toward container depth.
pub(super) fn text_depth(text: &str) -> Result<(), WireError> {
    let (mut depth, mut quoted, mut escaped) = (0u16, false, false);
    for byte in text.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    if depth >= MAX_DEPTH {
                        return Err(WireError::Depth);
                    }
                    depth += 1;
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}

pub(super) fn take<T: serde::de::DeserializeOwned>(input: &mut Reader<'_>) -> Result<T, WireError> {
    // Serde's parsed values and intermediate allocations cannot charge the shared budget.
    input.ordinary_only()?;
    let text: String = input.take()?;
    text_depth(&text)?;
    serde_json::from_str(&text).map_err(|_| WireError::Invalid("not valid JSON for its type"))
}
