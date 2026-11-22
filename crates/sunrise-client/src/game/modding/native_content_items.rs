//! Code item identities available while reading a package's content files.
use std::collections::BTreeMap;
use sunrise_client::mod_content::DeclaredContent;
use sunrise_mod_package::Manifest;

fn batch_item_ids(batch: &DeclaredContent) -> impl Iterator<Item = &str> {
    batch.items.iter().map(|item| item.get().id.as_str()).chain(
        batch
            .blocks
            .iter()
            .filter(|block| block.get().item)
            .map(|block| block.get().id.as_str()),
    )
}

pub(super) fn owned_item_ids(
    manifest: &Manifest,
    declarations: &BTreeMap<&'static str, DeclaredContent>,
) -> Vec<String> {
    let own = declarations
        .get(manifest.id.as_str())
        .into_iter()
        .flat_map(batch_item_ids);
    let from_host = manifest
        .served_by
        .as_deref()
        .and_then(|host| declarations.get(host))
        .into_iter()
        .flat_map(batch_item_ids)
        .filter(|id| {
            id.split_once(':')
                .is_some_and(|(ns, _)| manifest.owned_namespaces().any(|owned| owned == ns))
        });
    own.chain(from_host).map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_mod_package::{Content, content::Item};

    #[test]
    fn file_remainders_can_name_code_items_in_any_owned_namespace() {
        let manifest = Manifest::parse(
            r#"
            id = "porter"
            name = "Porter"
            version = "1.0.0"
            api = "0.2"
            side = "both"
            capabilities = ["registries"]
            namespaces = ["legacy"]
        "#,
        )
        .unwrap();
        let item = |id: &str| {
            Item {
                id: id.into(),
                template: "minecraft:stick".into(),
                ..Item::default()
            }
            .validate_local()
            .unwrap()
        };
        let batches = BTreeMap::from([
            (
                "porter",
                DeclaredContent {
                    items: vec![item("porter:spoon"), item("legacy:wrench")],
                    ..Default::default()
                },
            ),
            (
                "foreign",
                DeclaredContent {
                    items: vec![item("foreign:tool")],
                    ..Default::default()
                },
            ),
        ]);
        let declared = owned_item_ids(&manifest, &batches);
        assert_eq!(declared, ["porter:spoon", "legacy:wrench"]);
        let root = std::env::temp_dir().join(format!(
            "sunrise-owned-code-items-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let folder = root.join("data/legacy/sunrise/item");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("soup.json");
        std::fs::write(
            &file,
            br#"{"template":"minecraft:mushroom_stew","crafting_remainder":"legacy:wrench"}"#,
        )
        .unwrap();
        let content = Content::read_declaring(&root, &manifest, &declared).unwrap();
        assert_eq!(content.items[0].id, "legacy:soup");
        assert_eq!(
            content.items[0].crafting_remainder.as_deref(),
            Some("legacy:wrench")
        );
        std::fs::write(
            &file,
            br#"{"template":"minecraft:mushroom_stew","crafting_remainder":"foreign:tool"}"#,
        )
        .unwrap();
        let error = Content::read_declaring(&root, &manifest, &declared).unwrap_err();
        assert!(
            error.contains("foreign:tool") && error.contains("crafting_remainder"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn served_content_remainders_see_only_host_items_in_served_namespaces() {
        use sunrise_mod_package::content::Block;
        let manifest = Manifest::parse(
            r#"
            id = "runes"
            name = "Runes"
            version = "1.0.0"
            api = "0.2"
            side = "both"
            capabilities = ["registries"]
            namespaces = ["runes_extra"]
            served_by = "horizon_java"
        "#,
        )
        .unwrap();
        let item = |id: &str| {
            Item {
                id: id.into(),
                template: "minecraft:stick".into(),
                ..Item::default()
            }
            .validate_local()
            .unwrap()
        };
        let block = |id: &str, with_item: bool| {
            Block {
                id: id.into(),
                template: "minecraft:stone".into(),
                item: with_item,
                ..Block::default()
            }
            .validate_local()
            .unwrap()
        };
        let batches = BTreeMap::from([
            (
                "horizon_java",
                DeclaredContent {
                    blocks: vec![
                        block("runes:altar", true),
                        block("runes_extra:pedestal", true),
                        block("runes:itemless", false),
                        block("horizon_java:host_block", true),
                        block("other_served:block", true),
                    ],
                    items: vec![
                        item("runes:bowl"),
                        item("runes_extra:jar"),
                        item("horizon_java:wrench"),
                        item("other_served:wand"),
                    ],
                    ..Default::default()
                },
            ),
            (
                "unrelated",
                DeclaredContent {
                    blocks: vec![block("runes:spoofed_block", true)],
                    items: vec![item("runes:spoofed")],
                    ..Default::default()
                },
            ),
        ]);
        let declared = owned_item_ids(&manifest, &batches);
        assert_eq!(
            declared,
            [
                "runes:bowl",
                "runes_extra:jar",
                "runes:altar",
                "runes_extra:pedestal",
            ]
        );
        let root = std::env::temp_dir().join(format!(
            "sunrise-served-code-items-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let folder = root.join("data/runes/sunrise/item");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("ritual.json");
        std::fs::write(
            &file,
            br#"{"template":"minecraft:stick","crafting_remainder":"runes:altar"}"#,
        )
        .unwrap();
        let content = Content::read_declaring(&root, &manifest, &declared).unwrap();
        assert_eq!(
            content.items[0].crafting_remainder.as_deref(),
            Some("runes:altar")
        );
        std::fs::write(
            &file,
            br#"{"template":"minecraft:stick","crafting_remainder":"runes:itemless"}"#,
        )
        .unwrap();
        let err = Content::read_declaring(&root, &manifest, &declared).unwrap_err();
        assert!(
            err.contains("runes:itemless") && err.contains("crafting_remainder"),
            "{err}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
