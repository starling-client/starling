//! Authoritative list of engine-owned `sunrise:` custom-payload channels.
//!
//! Shared between mod payload registration (`sunrise-mod-api`) and generic transport access
//! (`sunrise-protocol`) so every engine service channel — whether or not it overrides the
//! discard payload size limit with a dedicated codec — is reserved consistently.

/// Engine-owned `sunrise:` custom-payload channels (transport control and engine services),
/// sorted for binary search.
pub const RESERVED_ENGINE_CHANNELS: &[&str] = &[
    "sunrise:anvil_cost",
    "sunrise:attachment_sync",
    "sunrise:block_registry",
    "sunrise:carry_age",
    "sunrise:collider_sync",
    "sunrise:datapack_offer",
    "sunrise:datapack_registries",
    "sunrise:general_entity",
    "sunrise:levels",
    "sunrise:lod",
    "sunrise:menu_data",
    "sunrise:menu_extra",
    "sunrise:mod_list",
    "sunrise:passage_cross",
    "sunrise:passage_sync",
    "sunrise:payloads",
    "sunrise:perf",
    "sunrise:quiet_commands",
    "sunrise:recipe_book_settings",
    "sunrise:recipe_check",
    "sunrise:recipe_content",
    "sunrise:recipe_transfer",
    "sunrise:selector_tokens",
];

/// Whether `channel` is an engine-owned `sunrise:` custom-payload channel.
pub fn is_reserved_engine_channel(channel: &str) -> bool {
    RESERVED_ENGINE_CHANNELS.binary_search(&channel).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_engine_channels_are_sorted_and_unique() {
        assert!(
            RESERVED_ENGINE_CHANNELS
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        for channel in RESERVED_ENGINE_CHANNELS {
            assert!(is_reserved_engine_channel(channel), "{channel}");
        }
        assert!(!is_reserved_engine_channel("sunrise:unused_mod_channel"));
    }
}
