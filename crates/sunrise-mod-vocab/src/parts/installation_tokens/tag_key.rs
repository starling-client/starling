/// Placeholder for a registration already refused by the host: the `u32::MAX` slot is never valid
/// (`TagKey::from_slot` rejects it).
pub fn invalid_tag_key() -> crate::TagKey {
    crate::TagKey(u32::MAX)
}
