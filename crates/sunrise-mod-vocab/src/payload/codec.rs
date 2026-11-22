//! Typed payload bodies are bounded uses of the ordinary Wire codec.
use crate::wire::{Reader, Wire, WireError, WireSchema, Writer};

pub use crate::wire::limits::{
    PAYLOAD_DECODE_BUDGET, PAYLOAD_MAX_DEPTH, PAYLOAD_MAX_LIST_ENTRIES, PAYLOAD_MAX_STRING_BYTES,
};

/// Marker in a typed declaration's negotiated version, followed by the schema fingerprint
/// as 16 lowercase hex digits. A raw declaration's version cannot contain this marker.
pub const PAYLOAD_SCHEMA_MARKER: &str = "+payload-schema.";

/// The structural fingerprint of `T`'s Wire encoding, independently of Rust names.
pub fn payload_schema<T: Wire + 'static>() -> u64 {
    T::describe(&mut WireSchema::default())
}

/// The negotiated version of a typed declaration: the mod's `version`, the marker and the
/// schema fingerprint. Values whose format changes carry a different fingerprint.
pub fn typed_payload_version(version: &str, schema: u64) -> String {
    format!("{version}{PAYLOAD_SCHEMA_MARKER}{schema:016x}")
}

/// Encodes a whole typed payload body as Wire, up to `limit` bytes and the shared payload limits.
pub fn encode_payload<T: Wire>(value: &T, limit: usize) -> Result<Vec<u8>, WireError> {
    let mut out = Writer::for_payload(limit.min(PAYLOAD_DECODE_BUDGET));
    out.put(value);
    out.finish_payload()
}

/// Decodes exactly one Wire value within the shared payload limits; trailing bytes are refused.
pub fn decode_payload<T: Wire>(body: &[u8]) -> Result<T, WireError> {
    if body.len() > PAYLOAD_DECODE_BUDGET {
        return Err(WireError::Length);
    }
    let mut input = Reader::for_payload(body);
    let value = input.take()?;
    if input.rest().is_empty() {
        Ok(value)
    } else {
        Err(WireError::Trailing)
    }
}

#[cfg(test)]
#[path = "codec_tests.rs"]
mod tests;
