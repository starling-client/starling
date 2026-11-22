//! Borrow the item engine's authoritative merged tags during custom loot evaluation.
use crate::items::Items;

impl sunrise_world::loot::LootTags for Items {
    fn members(&self, registry: &str, tag: &str) -> Option<Vec<String>> {
        let registry = match registry {
            "item" => crate::items::TagRegistry::Item,
            "block" => crate::items::TagRegistry::Block,
            "fluid" => crate::items::TagRegistry::Fluid,
            _ => return None,
        };
        self.tag_members(registry, tag).map(<[_]>::to_vec)
    }
    fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        match registry {
            "item" => &self.item_tags,
            "block" => &self.block_tags,
            "fluid" => &self.fluid_tags,
            _ => return false,
        }
        .get(tag)
        .is_some_and(|members| members.contains(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::TagRegistry;
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
