//! Context-set membership is warning-only. Unknown or malformed sets mean generic.
pub(super) fn allowed(set: &serde_json::Value) -> Option<&'static [&'static str]> {
    let name = set
        .as_str()?
        .strip_prefix("minecraft:")
        .unwrap_or(set.as_str()?);
    Some(match name {
        "empty" => &[],
        "chest" | "fishing" => {
            if name == "fishing" {
                &["origin", "tool", "this_entity", "attacking_entity"]
            } else {
                &["origin", "this_entity", "attacking_entity"]
            }
        }
        "command" | "command_compute_default" | "command_compute_position" => {
            if name == "command_compute_position" {
                &["origin", "block_state", "this_entity", "block_entity"]
            } else {
                &["origin", "this_entity"]
            }
        }
        "command_slot_source" => &["origin", "container", "this_entity"],
        "command_compute_entity" => &["origin", "target_entity", "this_entity"],
        "selector" | "equipment" | "gift" => &["origin", "this_entity"],
        "villager_trade" => &["origin", "this_entity", "additional_cost_component_allowed"],
        "entity" => &[
            "this_entity",
            "origin",
            "damage_source",
            "attacking_entity",
            "direct_attacking_entity",
            "last_damage_player",
        ],
        "archaeology" | "vault" | "shearing" | "advancement_location" => {
            if name == "advancement_location" {
                &[
                    "this_entity",
                    "origin",
                    "tool",
                    "block_state",
                    "block_entity",
                ]
            } else {
                &["origin", "this_entity", "tool"]
            }
        }
        "barter" => &["this_entity"],
        "advancement_reward" | "advancement_entity" => &["this_entity", "origin"],
        "block_use" => &["this_entity", "origin", "block_state"],
        "block" => &[
            "block_state",
            "origin",
            "tool",
            "this_entity",
            "block_entity",
            "explosion_radius",
        ],
        "entity_interact" => &["target_entity", "tool", "interacting_entity"],
        "block_interact" => &[
            "block_state",
            "origin",
            "block_entity",
            "interacting_entity",
            "tool",
        ],
        "container_process" => &[
            "block_entity",
            "block_state",
            "container",
            "origin",
            "queried_stack",
        ],
        "enchanted_damage" => &[
            "this_entity",
            "enchantment_level",
            "origin",
            "damage_source",
            "direct_attacking_entity",
            "attacking_entity",
        ],
        "enchanted_item" => &["tool", "enchantment_level"],
        "enchanted_location" => &[
            "this_entity",
            "enchantment_level",
            "origin",
            "enchantment_active",
        ],
        "enchanted_entity" => &["this_entity", "enchantment_level", "origin"],
        "hit_block" => &["this_entity", "enchantment_level", "origin", "block_state"],
        _ => return None,
    })
}
