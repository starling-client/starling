//! One membership implementation for full-game and in-write tag services.
use mod_api::{ItemKey, TagKey, TagMembers};
use sunrise_items::items::{Items, TagRegistry};

pub(super) fn is_in_tag(items: Option<&Items>, item: ItemKey, tag: TagKey) -> Option<bool> {
    let items = items.filter(|items| items.tags_loaded(TagRegistry::Item))?;
    let item = mod_api::__private::item_key_id(item)?;
    mod_api::__private::with_tag_name(tag, |name| {
        Some(
            items
                .item_tags
                .get(name?)
                .is_some_and(|members| members.contains(item)),
        )
    })
}
pub(super) fn snapshot_members<K>(
    items: Option<&Items>,
    registry: TagRegistry,
    tag: TagKey,
    key: impl FnMut(&str) -> Option<K>,
) -> TagMembers<K> {
    let Some(items) = items.filter(|items| items.tags_loaded(registry)) else {
        return TagMembers::Unavailable;
    };
    members(tag, |name| items.tag_members(registry, name), key)
}

/// Map complete ordered membership from either a host snapshot or a loot table.
pub(super) fn members<K, I: IntoIterator>(
    tag: TagKey,
    read: impl FnOnce(&str) -> Option<I>,
    mut key: impl FnMut(&str) -> Option<K>,
) -> TagMembers<K>
where
    I::Item: AsRef<str>,
{
    mod_api::__private::with_tag_name(tag, |name| {
        let Some(name) = name else {
            return TagMembers::Unavailable;
        };
        let Some(values) = read(name) else {
            return TagMembers::Missing;
        };
        values
            .into_iter()
            .map(|id| key(id.as_ref()))
            .collect::<Option<Vec<_>>>()
            .map_or(TagMembers::Unavailable, TagMembers::Members)
    })
}

// A restricted host supplies the same authoritative item tables but cannot borrow Game.
// Only that choice differs; key resolution, ordered reads and membership stay above.
macro_rules! impl_tags {
    ($ty:ty, $this:ident, $items:expr) => {
        impl mod_api::Tags for $ty {
            fn key(&self, id: &mod_api::Identifier) -> Option<mod_api::TagKey> {
                mod_api::__private::tag_key(id)
            }
            fn is_in_tag(&self, item: mod_api::ItemKey, tag: mod_api::TagKey) -> Option<bool> {
                let $this = self;
                $crate::game::native_tags::is_in_tag($items, item, tag)
            }
            fn items(&self, tag: mod_api::TagKey) -> mod_api::TagMembers<mod_api::ItemKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Item,
                    tag,
                    mod_api::__private::item_key,
                )
            }
            fn fluids(&self, tag: mod_api::TagKey) -> mod_api::TagMembers<mod_api::FluidKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Fluid,
                    tag,
                    mod_api::__private::fluid_key,
                )
            }
            fn blocks(&self, tag: mod_api::TagKey) -> mod_api::TagMembers<mod_api::BlockKey> {
                let $this = self;
                $crate::game::native_tags::snapshot_members(
                    $items,
                    sunrise_items::items::TagRegistry::Block,
                    tag,
                    mod_api::__private::block_key,
                )
            }
        }
    };
}
pub(super) use impl_tags;
impl_tags!(super::Game<'_>, game, game.native_tag_items());

#[cfg(test)]
pub(in crate::game) const PACK_BINDING_CASES: [(&str, &[u8], bool); 3] = [
    (
        "optional_only",
        br#"{"values":[{"id":"binding:not_registered","required":false}]}"#,
        true,
    ),
    (
        "broken_parent",
        br##"{"values":["minecraft:stick","#binding:absent"]}"##,
        false,
    ),
    (
        "broken_nested",
        br##"{"values":["#binding:absent"]}"##,
        false,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::Tags;

    struct TagView<'a>(Option<&'a Items>);
    impl_tags!(TagView<'_>, view, view.0);

    #[test]
    fn registration_resolves_the_same_tag_key_without_loaded_membership() {
        use mod_api::{Hooks, Identifier, ModEntry, Registrar, RegistrationContext};
        use std::sync::{Arc, OnceLock};

        struct ResolveTag(Arc<OnceLock<TagKey>>);
        #[mod_api::__subscriptions]
        impl Hooks for ResolveTag {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let _ = self.0.set(r.tag_key("registrar_tag_lookup"));
            }
        }

        let found = Arc::new(OnceLock::new());
        crate::game::api_values::install_parts(&[ModEntry::new(
            "tag_reader",
            Box::new(ResolveTag(found.clone())),
        )])
        .unwrap();
        struct UnloadedTags;
        impl_tags!(UnloadedTags, _view, None);
        let name = Identifier::parse("minecraft:registrar_tag_lookup").unwrap();
        assert_eq!(*found.get().unwrap(), UnloadedTags.key(&name).unwrap());
    }

    #[test]
    fn block_members_preserve_authoritative_order_reload_and_atomic_refusal() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_block_keys(
            ["minecraft:air", "minecraft:stone", "minecraft:birch_log"],
            |_| {
                let properties = std::collections::BTreeMap::new();
                sunrise_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                    .map_err(|error| format!("{error:?}"))
            },
        )
        .unwrap();
        let stone = mod_api::__private::block_key("minecraft:stone").unwrap();
        let birch = mod_api::__private::block_key("minecraft:birch_log").unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:blocks").unwrap())
            .unwrap();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("test:empty").unwrap())
                .unwrap();
        let mut table = Items::for_tests(Default::default());
        assert_eq!(TagView(Some(&table)).blocks(tag), TagMembers::Unavailable);
        table.set_tags(
            TagRegistry::Block,
            [
                (
                    "test:blocks".into(),
                    vec!["minecraft:stone".into(), "minecraft:birch_log".into()],
                ),
                ("test:empty".into(), vec![]),
            ]
            .into(),
        );
        let view = TagView(Some(&table));
        assert_eq!(view.blocks(tag), TagMembers::Members(vec![stone, birch]));
        assert_eq!(view.blocks(missing), TagMembers::Members(vec![]));
        assert_eq!(TagView(None).blocks(tag), TagMembers::Unavailable);
        table.set_tags(
            TagRegistry::Block,
            [("test:blocks".into(), vec!["minecraft:birch_log".into()])].into(),
        );
        let view = TagView(Some(&table));
        assert_eq!(view.blocks(tag), TagMembers::Members(vec![birch]));
        assert_eq!(view.blocks(missing), TagMembers::Missing);
        table.block_tag_order.insert(
            "test:blocks".into(),
            vec!["minecraft:stone".into(), "test:unloaded".into()],
        );
        assert_eq!(
            TagView(Some(&table)).blocks(tag),
            TagMembers::Unavailable,
            "no partial member list on unresolved block mapping"
        );
    }

    #[test]
    fn fluid_members_distinguish_missing_empty_and_unavailable() {
        let tag =
            |name| mod_api::__private::tag_key(&mod_api::Identifier::parse(name).unwrap()).unwrap();
        let empty = tag("test:empty_fluid");
        let absent = tag("test:absent_fluid");
        let mut table = Items::empty();
        table.set_tags(TagRegistry::Block, Default::default());
        assert_eq!(TagView(Some(&table)).fluids(empty), TagMembers::Unavailable);
        table.set_tags(
            TagRegistry::Fluid,
            [("test:empty_fluid".into(), vec![])].into(),
        );
        let view = TagView(Some(&table));
        assert_eq!(view.fluids(empty), TagMembers::Members(vec![]));
        assert_eq!(view.fluids(absent), TagMembers::Missing);
        assert_eq!(view.items(absent), TagMembers::Unavailable);
    }

    #[test]
    fn empty_stacks_refuse_before_membership_and_air_is_an_explicit_key() {
        use crate::game::api_values::{StackReadError, TryIntoApi};
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:air", "minecraft:stone"]).unwrap();
        let air = mod_api::__private::item_key("minecraft:air").unwrap();
        let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("x").unwrap()).unwrap();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("minecraft:y").unwrap())
                .unwrap();
        let mut table = Items::for_tests(
            [(
                "minecraft:x".to_owned(),
                ["minecraft:air".to_owned()].into(),
            )]
            .into(),
        );
        let empty = sunrise_client::items::ItemStack::new("minecraft:stone", 0);
        assert_eq!(empty.try_into_api(), Err(StackReadError::Empty));
        assert_eq!(is_in_tag(Some(&table), air, tag), Some(true));
        assert_eq!(is_in_tag(Some(&table), stone, tag), Some(false));
        assert_eq!(is_in_tag(Some(&table), air, missing), Some(false));
        table.item_tag_order.insert(
            "minecraft:x".into(),
            vec!["minecraft:air".into(), "missing:unloaded".into()],
        );
        assert_eq!(
            TagView(Some(&table)).items(tag),
            TagMembers::Unavailable,
            "unresolved members must not produce a partial list"
        );
    }
    #[test]
    fn keys_survive_membership_reload_and_reads_distinguish_unavailable_from_unknown() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:stick", "minecraft:stone"]).unwrap();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:reload").unwrap())
            .unwrap();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("test:missing").unwrap())
                .unwrap();
        let mut table = Items::empty();
        assert_eq!(TagView(Some(&table)).items(tag), TagMembers::Unavailable);
        table.set_tags(
            TagRegistry::Item,
            [
                (
                    "test:reload".into(),
                    vec!["minecraft:stone".into(), "minecraft:stick".into()],
                ),
                ("test:missing".into(), Vec::new()),
            ]
            .into(),
        );
        assert_eq!(is_in_tag(Some(&table), stick, tag), Some(true));
        assert_eq!(
            TagView(Some(&table)).items(tag),
            TagMembers::Members(vec![stone, stick])
        );
        assert_eq!(
            TagView(Some(&table)).items(missing),
            TagMembers::Members(vec![])
        );
        table.set_tags(
            TagRegistry::Item,
            [("test:reload".into(), vec!["minecraft:stone".into()])].into(),
        );
        assert_eq!(is_in_tag(Some(&table), stick, tag), Some(false));
        assert_eq!(
            TagView(Some(&table)).items(tag),
            TagMembers::Members(vec![stone])
        );
        assert_eq!(is_in_tag(Some(&table), stone, missing), Some(false));
        assert_eq!(TagView(Some(&table)).items(missing), TagMembers::Missing);
        assert_eq!(is_in_tag(None, stone, tag), None);
        assert_eq!(TagView(None).items(tag), TagMembers::Unavailable);
    }
}
