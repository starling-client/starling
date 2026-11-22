/// Construct the token for the slot allocated by the registration boundary.
pub fn damage_type_key(slot: u32) -> crate::DamageTypeKey {
    crate::DamageTypeKey(slot)
}

/// The canonical registry name this token carries across level changes.
pub fn damage_type_name(key: crate::DamageTypeKey) -> Option<&'static str> {
    crate::parts::requested_keys().damage_types.id(key.0)
}
