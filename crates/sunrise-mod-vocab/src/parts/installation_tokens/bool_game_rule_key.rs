//! Opaque registration tokens; callers validate membership in their owning request table.
/// Cold authoring access to the opaque token representation.
pub fn bool_game_rule_key(slot: u32) -> crate::BoolGameRuleKey {
    crate::BoolGameRuleKey(slot)
}
/// Cold authoring access to the opaque token representation.
pub fn bool_game_rule_key_slot(key: crate::BoolGameRuleKey) -> u32 {
    key.0
}

/// Validate a token using the current registration or installed lookup epoch.
pub fn validate_bool_game_rule_key(slot: u32) -> Result<crate::BoolGameRuleKey, &'static str> {
    crate::BoolGameRuleKey::from_slot(slot)
}
