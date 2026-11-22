//! Adapt the current loot evaluator's tag reads to the existing API resource.
use mod_api::{BlockKey, FluidKey, Identifier, ItemKey, TagKey, TagMembers, Tags};

pub(crate) struct LootTags<'a>(pub &'a dyn sunrise_world::loot::LootTags);

impl Tags for LootTags<'_> {
    fn key(&self, id: &Identifier) -> Option<TagKey> {
        mod_api::__private::tag_key(id)
    }
    fn is_in_tag(&self, item: ItemKey, tag: TagKey) -> Option<bool> {
        let item = mod_api::__private::item_key_id(item)?;
        mod_api::__private::with_tag_name(tag, |tag| Some(self.0.contains("item", tag?, item)))
    }
    fn items(&self, tag: TagKey) -> TagMembers<ItemKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("item", name),
            mod_api::__private::item_key,
        )
    }
    fn blocks(&self, tag: TagKey) -> TagMembers<BlockKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("block", name),
            mod_api::__private::block_key,
        )
    }
    fn fluids(&self, tag: TagKey) -> TagMembers<FluidKey> {
        super::native_tags::members(
            tag,
            |name| self.0.members("fluid", name),
            mod_api::__private::fluid_key,
        )
    }
}
