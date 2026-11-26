//! Borrow the item engine's authoritative merged tags during custom loot evaluation.
use crate::items::{Items, TagRegistry};
use sunrise_world::loot::ItemTagNames;

impl sunrise_world::loot::LootTags for Items {
    fn members(&self, registry: &str, tag: &str) -> Option<Vec<String>> {
        let registry = match registry {
            "item" => TagRegistry::Item,
            "block" => TagRegistry::Block,
            "fluid" => TagRegistry::Fluid,
            "entity_type" => TagRegistry::EntityType,
            "worldgen/biome" => TagRegistry::Biome,
            "enchantment" => TagRegistry::Enchantment,
            "worldgen/structure" => TagRegistry::Structure,
            "potion" => TagRegistry::Potion,
            "damage_type" => TagRegistry::DamageType,
            "world_clock" => TagRegistry::WorldClock,
            _ => return None,
        };
        self.tag_members(registry, tag).map(<[_]>::to_vec)
    }
    fn item_tag_names(&self) -> Option<&ItemTagNames> {
        self.item_tag_names.as_ref()
    }
    fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        match registry {
            "item" => self.item_tags.get(tag).is_some_and(|m| m.contains(id)),
            "block" => self.block_tags.get(tag).is_some_and(|m| m.contains(id)),
            "fluid" => self.fluid_tags.get(tag).is_some_and(|m| m.contains(id)),
            "entity_type" => self.entity_type_tags.contains(tag, id),
            _ => self
                .members(registry, tag)
                .is_some_and(|members| members.iter().any(|member| member == id)),
        }
    }
    /// The damage type registry and tags bound at the last reload
    /// ([`Items::bind_component_damage_catalog`], [`Items::bind_damage_registry`]).
    fn damage_type_in_tag(&self, damage_type: &str, tag: &str) -> Option<bool> {
        let types = &self
            .component_registry
            .by_registry
            .get("minecraft:damage_type")?
            .ids;
        if !types.iter().any(|id| id == damage_type) {
            return None;
        }
        Some(
            self.enchantments
                .damage_tags
                .get(tag)
                .is_some_and(|members| members.contains(damage_type)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use sunrise_world::loot::LootTags;

    #[test]
    fn loot_reads_use_current_registry_and_member_order() {
        let mut items = Items::for_tests(HashMap::new());
        for (kind, registry, member) in [
            (TagRegistry::Item, "item", "minecraft:stick"),
            (TagRegistry::Block, "block", "minecraft:stone"),
            (TagRegistry::Fluid, "fluid", "minecraft:water"),
        ] {
            items.set_tags(
                kind,
                HashMap::from([("test:tag".into(), vec![member.into()])]),
            );
            assert_eq!(items.members(registry, "test:tag").unwrap(), [member]);
            assert!(items.contains(registry, "test:tag", member));
            assert!(!items.contains(registry, "test:missing", member));
        }
        items.set_tags(
            TagRegistry::Item,
            HashMap::from([(
                "test:tag".into(),
                vec!["minecraft:potion".into(), "minecraft:stick".into()],
            )]),
        );
        assert_eq!(
            items.members("item", "test:tag").unwrap(),
            ["minecraft:potion", "minecraft:stick"]
        );
        items.set_tags(TagRegistry::Item, HashMap::new());
        assert!(!items.contains("item", "test:tag", "minecraft:stick"));
        assert_eq!(
            items.members("block", "test:tag").unwrap(),
            ["minecraft:stone"]
        );
    }
}
