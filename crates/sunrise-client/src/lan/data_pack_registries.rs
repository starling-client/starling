//! Winning definitions for the synchronized registries that read world packs.
use super::{PACK_REGISTRIES, condition_filter, json_to_nbt, synchronized_entries};
use crate::dialog::Reference;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use sunrise_core::{
    identifier::Identifier,
    nbt::{Compound, Tag},
};
use sunrise_items::{
    enchant::{Enchantments, HolderSetBinding},
    items::{Instrument, Items},
};
use sunrise_world::{item_defaults::ItemDefaults, loot::PackOverlay};

type Registry = BTreeMap<(String, String), Option<(Definition, bool)>>;

/// Validated once for the world session. A false condition suppresses the
/// winning entry, including any prepared or lower-priority definition.
#[derive(Default)]
pub struct PackRegistryDefinitions {
    entries: BTreeMap<&'static str, Registry>,
}

/// Keep the admitted item definitions alongside the exact wire encoding.
enum Definition {
    Encoded(Tag),
    Enchantment(Value),
    Instrument(Instrument, Tag),
}

impl PackRegistryDefinitions {
    /// RegistryDataLoader's collected errors: try every winning definition,
    /// then refuse the entire load and report each failure by registry and id.
    pub(super) fn load(
        resources: &Path,
        defaults: &ItemDefaults,
        overlay: &PackOverlay<'_>,
        read_errors: &BTreeMap<String, String>,
        text_registries: &crate::command::Registries,
        sound_exists: impl Fn(&str) -> bool,
    ) -> Result<Self, String> {
        let (definitions, admitted) = Self::stage(
            resources,
            defaults,
            overlay,
            read_errors,
            text_registries,
            sound_exists,
        );
        admitted.map(|()| definitions)
    }

    /// Keep replaced keys even on failure, so the caller can collect errors
    /// from other registry files without checking a losing definition.
    pub(super) fn stage(
        resources: &Path,
        defaults: &ItemDefaults,
        overlay: &PackOverlay<'_>,
        read_errors: &BTreeMap<String, String>,
        text_registries: &crate::command::Registries,
        sound_exists: impl Fn(&str) -> bool,
    ) -> (Self, Result<(), String>) {
        let mut definitions = Self::default();
        let mut errors = BTreeMap::<_, BTreeMap<_, _>>::new();
        let mut dialog_references = Vec::new();
        for &registry in PACK_REGISTRIES {
            let kind = registry.trim_start_matches("minecraft:");
            let entries = definitions.entries.entry(registry).or_default();
            for (key, error) in read_errors {
                if let Some(id) = key.strip_prefix(kind).and_then(|id| id.strip_prefix('/')) {
                    if let Ok(id) = Identifier::parse(id) {
                        entries.insert((id.namespace().to_owned(), id.path().to_owned()), None);
                    }
                    errors
                        .entry(registry)
                        .or_default()
                        .insert(id.to_owned(), error.clone());
                }
            }
            for (id, files) in overlay.kind(kind) {
                if errors
                    .get(registry)
                    .is_some_and(|errors| errors.contains_key(id))
                {
                    continue;
                }
                let mut decode = || -> Result<(), String> {
                    let id = Identifier::parse(id).map_err(|error| error.to_string())?;
                    let key = (id.namespace().to_owned(), id.path().to_owned());
                    // Reserve the winner even if it fails admission or its condition is false.
                    let definition = entries.entry(key).or_insert(None);
                    let value: Value = serde_json::from_slice(PackOverlay::top(files))
                        .map_err(|error| error.to_string())?;
                    let file = format!("data/{}/{kind}/{}.json", id.namespace(), id.path());
                    if overlay.conditions.allows_file_isolated(&value, &file)? {
                        let decoded = if registry == "minecraft:dialog" {
                            let (decoded, references) =
                                decode_dialog(value, text_registries, defaults)?;
                            dialog_references.push(references);
                            decoded
                        } else {
                            decode_definition(
                                registry,
                                &id.to_string(),
                                value,
                                text_registries,
                                &sound_exists,
                            )?
                        };
                        *definition = Some((decoded, true));
                    }
                    Ok(())
                };
                if let Err(error) = decode() {
                    errors
                        .entry(registry)
                        .or_default()
                        .insert(id.to_owned(), error);
                }
            }
            if Self::includes_prepared_entries(registry) {
                let mut prepared = synchronized_entries(
                    &resources.join("data/minecraft"),
                    resources,
                    registry,
                    overlay,
                );
                // The pack winner (even if refused or skipped) hides resources below it.
                prepared.retain(|(key, _, _)| !entries.contains_key(key));
                condition_filter::filter(&mut prepared, false, overlay.conditions, kind);
                for (key, value, from_pack) in prepared {
                    let definition = if kind == "enchantment" {
                        Definition::Enchantment(value)
                    } else if from_pack {
                        let id = format!("{}:{}", key.0, key.1);
                        match decode_definition(
                            registry,
                            &id,
                            value,
                            text_registries,
                            &sound_exists,
                        ) {
                            Ok(definition) => definition,
                            Err(error) => {
                                errors.entry(registry).or_default().insert(id, error);
                                entries.insert(key, None);
                                continue;
                            }
                        }
                    } else {
                        let wire = json_to_nbt(&value);
                        match Instrument::decode(&value) {
                            Ok(instrument) => Definition::Instrument(instrument, wire),
                            Err(_) => Definition::Encoded(wire),
                        }
                    };
                    entries.insert(key, Some((definition, from_pack)));
                }
            }
        }
        if let Some(entries) = definitions.entries.get_mut("minecraft:enchantment") {
            let enchantments: Vec<_> = entries
                .iter_mut()
                .filter_map(|((namespace, path), entry)| {
                    let (Definition::Enchantment(value), from_pack) = entry.as_mut()? else {
                        return None;
                    };
                    Some((format!("{namespace}:{path}"), value, *from_pack))
                })
                .collect();
            let binding = enchantments
                .iter()
                .any(|(_, value, _)| HolderSetBinding::needed(value))
                .then(|| {
                    HolderSetBinding::load(
                        resources,
                        overlay,
                        defaults,
                        enchantments.iter().map(|(id, _, _)| id.clone()).collect(),
                    )
                });
            for (id, value, from_pack) in enchantments {
                let result = if let Some(binding) =
                    binding.as_ref().filter(|_| HolderSetBinding::needed(value))
                {
                    match binding {
                        Ok(binding) => binding.bind(&id, value).map_err(|error| error.to_string()),
                        Err(error) => Err(error.to_string()),
                    }
                } else {
                    Ok(())
                };
                // Holder binding may change the transmitted shape. Preflight
                // the exact JSON-to-NBT projection the registry packet uses.
                let result = result.and_then(|()| {
                    if from_pack {
                        check_network_nbt(&json_to_nbt(value))?;
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    errors
                        .entry("minecraft:enchantment")
                        .or_default()
                        .insert(id, error);
                }
            }
        }
        // RegistryDataLoader.validateRegistry: every dialog a winner names is bound.
        let dialogs = definitions.entries.get("minecraft:dialog");
        let root = resources.join("data/minecraft");
        let mut prepared = synchronized_entries(&root, resources, "minecraft:dialog", overlay);
        prepared.retain(|(key, ..)| !dialogs.is_some_and(|d| d.contains_key(key)));
        condition_filter::filter(&mut prepared, false, overlay.conditions, "dialog");
        let held: BTreeSet<String> = dialogs
            .into_iter()
            .flatten()
            .filter(|(_, entry)| entry.is_some())
            .map(|((namespace, path), _)| format!("{namespace}:{path}"))
            .chain(
                prepared
                    .into_iter()
                    .map(|((namespace, path), ..)| format!("{namespace}:{path}")),
            )
            .collect();
        let (tags, mut unbound) = bind_dialog_tags(&root, overlay, |id| held.contains(id));
        let mut unbound_tags = BTreeSet::new();
        for reference in dialog_references.into_iter().flatten() {
            match reference {
                Reference::Id(id) if !held.contains(&id) => unbound.insert(id),
                Reference::Tag(tag) if !tags.contains_key(&tag) => unbound_tags.insert(tag),
                _ => false,
            };
        }
        for (what, names) in [("values", unbound), ("tags", unbound_tags)] {
            if !names.is_empty() {
                errors.entry("minecraft:dialog").or_default().insert(
                    format!("(unbound {what})"),
                    format!("unbound {what} in registry: {names:?}"),
                );
            }
        }
        if errors.is_empty() {
            return (definitions, Ok(()));
        }
        let mut report = String::from("failed to load world registry definitions:");
        for (registry, entries) in errors {
            report.push_str(&format!("\n{registry}:"));
            for (id, error) in entries {
                report.push_str(&format!("\n  {id}: {error}"));
            }
        }
        (definitions, Err(report))
    }

    pub(super) fn discard_replaced(
        &self,
        registry: &str,
        entries: &mut Vec<((String, String), Value, bool)>,
    ) {
        if let Some(pack) = self.entries.get(registry) {
            entries.retain(|(key, _, _)| !pack.contains_key(key));
        }
    }

    pub(super) fn includes_prepared_entries(registry: &str) -> bool {
        matches!(registry, "minecraft:enchantment" | "minecraft:instrument")
    }

    pub(super) fn append_encoded(
        &self,
        registry: &str,
        entries: &mut Vec<((String, String), Tag, bool)>,
    ) {
        if let Some(pack) = self.entries.get(registry) {
            entries.extend(pack.iter().filter_map(|(key, value)| {
                let (definition, from_pack) = value.as_ref()?;
                let tag = match definition {
                    Definition::Encoded(tag) | Definition::Instrument(_, tag) => tag.clone(),
                    Definition::Enchantment(json) => json_to_nbt(json),
                };
                Some((key.clone(), tag, *from_pack))
            }));
        }
    }

    /// Use the same selected definitions as the synchronized registry packets.
    pub fn install_items(
        &self,
        items: &mut Items,
        resources: &Path,
        overlay: &PackOverlay<'_>,
        defaults: &ItemDefaults,
    ) -> sunrise_items::Result<()> {
        let enchantments: Vec<_> = self
            .entries
            .get("minecraft:enchantment")
            .into_iter()
            .flatten()
            .filter_map(|((namespace, path), entry)| match entry {
                Some((Definition::Enchantment(value), _)) => {
                    Some((format!("{namespace}:{path}"), value))
                }
                _ => None,
            })
            .collect();
        let enchantments = Enchantments::load_selected(
            resources,
            overlay,
            defaults,
            enchantments.iter().map(|(id, value)| (id.as_str(), *value)),
        )?;
        let instruments = self
            .entries
            .get("minecraft:instrument")
            .into_iter()
            .flatten()
            .filter_map(|((namespace, path), entry)| match entry {
                Some((Definition::Instrument(instrument, _), _)) => {
                    Some((format!("{namespace}:{path}"), instrument.clone()))
                }
                _ => None,
            })
            .collect();
        items.install_registries(enchantments, instruments);
        Ok(())
    }
}

/// Bind the dialog tag stack both at world admission and when producing tag
/// packets (including reloads over the frozen registry). TagLoader's writable
/// lookup reserves a holder for each missing required value; these names are
/// reported by the registry freeze, even if no dialog uses the tag.
pub(super) fn bind_dialog_tags(
    root: &Path,
    overlay: &PackOverlay<'_>,
    held: impl Fn(&str) -> bool,
) -> (BTreeMap<String, Vec<String>>, BTreeSet<String>) {
    let mut unbound = BTreeSet::new();
    let bound = bind_registry_tags(root, overlay, "dialog", |id, required| {
        if held(id) {
            Some(id.to_owned())
        } else {
            if required {
                unbound.insert(id.to_owned());
            }
            None
        }
    });
    (bound, unbound)
}

/// Build the stacked tag files against the registry that will publish them.
/// Only a successfully built tag can satisfy a nested reference.
pub(super) fn bind_registry_tags<T: Clone + Ord>(
    root: &Path,
    overlay: &PackOverlay<'_>,
    kind: &str,
    mut lookup: impl FnMut(&str, bool) -> Option<T>,
) -> BTreeMap<String, Vec<T>> {
    use sunrise_world::worldgen::tags::TagDefinitions;

    // ResourceManager lists whole stacks in Identifier file order (path
    // first, namespace second). Pack versions then apply low to high.
    let prepared = sunrise_world::json_folder::read(&root.join("tags").join(kind)).ok();
    let mut files = BTreeMap::<_, Vec<&[u8]>>::new();
    for file in prepared.iter().flat_map(|folder| &folder.files) {
        if let Ok(bytes) = file.bytes() {
            files
                .entry((format!("{}.json", file.name), "minecraft".to_owned()))
                .or_default()
                .push(bytes);
        }
    }
    for (id, versions) in overlay.tag_kind(kind) {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", &id));
        files
            .entry((format!("{path}.json"), namespace.to_owned()))
            .or_default()
            .extend(versions.iter().map(Vec::as_slice));
    }
    let mut definitions = TagDefinitions::default();
    for ((path, namespace), stack) in files {
        let id = format!("{namespace}:{}", path.strip_suffix(".json").unwrap());
        for bytes in stack {
            if let Ok(json) = std::str::from_utf8(bytes) {
                let _ = definitions.apply_json(&id, "pack", json);
            }
        }
    }
    let built = definitions.bind(&mut lookup);
    for (id, missing) in built.missing {
        let entries: Vec<_> = missing.iter().map(|e| e.entry.display()).collect();
        log::warn!(
            "Sunrise: {kind} tag {id} has unresolved required entries: {}",
            entries.join(", ")
        );
    }
    built.tags
}

fn check_network_nbt(tag: &Tag) -> Result<(), String> {
    sunrise_protocol::buf::check_network_nbt(tag).map_err(|error| format!("network {error}"))
}

/// A winning dialog definition: `Dialog.DIRECT_CODEC` over the file, and the
/// dialogs it names, which the registry must hold (`decode_pack`).
fn decode_dialog(
    value: Value,
    text_registries: &crate::command::Registries,
    defaults: &ItemDefaults,
) -> Result<(Definition, Vec<crate::dialog::Reference>), String> {
    // Transient components are excluded from persisted world definitions.
    sunrise_world::item_defaults::validate_dialog(&value)?;
    let (encoded, references) =
        crate::dialog::Dialog::decode_pack(&value, text_registries, &|id| {
            defaults.entries().contains_key(id)
        })
        .map_err(|field| format!("invalid dialog field {field}"))?;
    check_network_nbt(&encoded)?;
    Ok((Definition::Encoded(encoded), references))
}

fn decode_definition(
    registry: &str,
    id: &str,
    value: Value,
    text_registries: &crate::command::Registries,
    sound_exists: &dyn Fn(&str) -> bool,
) -> Result<Definition, String> {
    let decode_text = |field: &str, value: &Value| {
        crate::text_grammar::build(value, text_registries, false)
            .ok_or_else(|| format!("{field}: invalid JSON text component"))
    };
    let string = |value: &str| Tag::String(value.into());
    let asset_id = |asset: &str| {
        // The render readers store the default namespace as a path only.
        if asset.contains(':') {
            string(asset)
        } else {
            string(&format!("minecraft:{asset}"))
        }
    };
    let encoded = match registry {
        "minecraft:dimension_type" => {
            sunrise_world::dimension_type::DimensionFacts::decode_pack_definition(id, &value)?
        }
        "minecraft:enchantment" => {
            let mut value = sunrise_items::enchant::decode_pack_definition(value)?;
            value["description"] = decode_text("description", &value["description"])?;
            return Ok(Definition::Enchantment(value));
        }
        "minecraft:painting_variant" => {
            let painting = crate::entities::PaintingVariant::decode(id, &value)?;
            let mut fields = Compound::from([
                ("asset_id".into(), asset_id(&painting.asset)),
                ("width".into(), Tag::Int(painting.width)),
                ("height".into(), Tag::Int(painting.height)),
            ]);
            for (field, text) in [("title", painting.title), ("author", painting.author)] {
                if let Some(text) = text.filter(|value| !value.is_null()) {
                    fields.insert(field.into(), json_to_nbt(&decode_text(field, &text)?));
                }
            }
            Tag::Compound(fields)
        }
        "minecraft:instrument" => {
            let instrument = Instrument::decode(&value)?;
            let float = |v: f32| Tag::Float(v.to_bits());
            let sound = match instrument.inline_sound_range {
                None if !sound_exists(&instrument.sound) => {
                    return Err(format!("sound_event: unknown holder {}", instrument.sound));
                }
                None => string(&instrument.sound),
                Some(fixed) => {
                    let mut fields =
                        Compound::from([("sound_id".into(), string(&instrument.sound))]);
                    if let Some(range) = fixed {
                        fields.insert("range".into(), float(range));
                    }
                    Tag::Compound(fields)
                }
            };
            let mut fields = Compound::from([
                ("sound_event".into(), sound),
                ("use_duration".into(), float(instrument.use_duration)),
                ("range".into(), float(instrument.range)),
                (
                    "description".into(),
                    json_to_nbt(&decode_text("description", &value["description"])?),
                ),
            ]);
            if instrument.durability_damage != 0 {
                fields.insert(
                    "durability_damage".into(),
                    Tag::Int(instrument.durability_damage),
                );
            }
            let tag = Tag::Compound(fields);
            check_network_nbt(&tag)?;
            return Ok(Definition::Instrument(instrument, tag));
        }
        "minecraft:banner_pattern" => {
            let pattern = crate::banner::Pattern::decode(id, &value)?;
            Tag::compound([
                ("asset_id", asset_id(&pattern.asset)),
                ("translation_key", string(&pattern.translation_key)),
            ])
        }
        // WorldClock.DIRECT_CODEC is a unit map codec: extra keys are permitted.
        "minecraft:world_clock" => {
            if !value.is_object() {
                return Err("not a map".into());
            }
            json_to_nbt(&value)
        }
        _ => unreachable!("only selected registry families are decoded"),
    };
    check_network_nbt(&encoded)?;
    Ok(Definition::Encoded(encoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn enchantment(description: &str) -> Value {
        json!({
            "description": description, "weight": 1, "max_level": 1, "anvil_cost": 1,
            "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 5, "per_level_above_first": 0},
            "supported_items": "minecraft:stick", "slots": ["hand"]
        })
    }

    #[test]
    fn a_set_object_reaches_clients_as_ids_and_plain_forms_are_untouched() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("items.json"),
            json!({"minecraft:stick": {}, "minecraft:iron_sword": {}, "test:custom": {}})
                .to_string(),
        )
        .unwrap();
        let mut sticky = enchantment("Sticky");
        sticky["supported_items"] = json!({"type": "neoforge:or",
            "values": ["#minecraft:swords", "minecraft:stick"]});
        sticky["primary_items"] = json!({"type": "neoforge:any"});
        sticky["exclusive_set"] = json!({"type": "neoforge:any"});
        let mut plain = enchantment("Plain");
        plain["supported_items"] = json!("#minecraft:swords");
        let files = [
            (
                "tags/minecraft:item/swords".into(),
                vec![
                    json!({"values": ["test:custom", "minecraft:iron_sword"]})
                        .to_string()
                        .into_bytes(),
                ],
            ),
            (
                "enchantment/test:sticky".into(),
                vec![sticky.to_string().into_bytes()],
            ),
            (
                "enchantment/test:plain".into(),
                vec![plain.to_string().into_bytes()],
            ),
        ];
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let packed = PackRegistryDefinitions::load(
            root.path(),
            &defaults,
            &PackOverlay::new(&files),
            &Default::default(),
            &Default::default(),
            |_| true,
        )
        .unwrap();
        let mut wire = Vec::new();
        packed.append_encoded("minecraft:enchantment", &mut wire);
        assert_eq!(wire[0].0.1, "plain");
        assert_eq!(wire[1].0.1, "sticky");
        let ids =
            |ids: &[&str]| Tag::List(ids.iter().map(|id| Tag::String((*id).into())).collect());
        for (field, expected) in [
            (
                "supported_items",
                ids(&["test:custom", "minecraft:iron_sword", "minecraft:stick"]),
            ),
            (
                "primary_items",
                ids(&["minecraft:iron_sword", "minecraft:stick", "test:custom"]),
            ),
            ("exclusive_set", ids(&["test:plain", "test:sticky"])),
        ] {
            assert_eq!(wire[1].1.get(field), Some(&expected));
        }
        assert_eq!(
            wire[0].1.get("supported_items"),
            Some(&Tag::String("#minecraft:swords".into()))
        );
    }

    #[test]
    fn bound_tag_conditions_select_the_same_item_definitions_for_gameplay_and_wire() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("items.json"),
            br#"{"minecraft:stick": {}}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let mut enchantment = enchantment("Conditional");
        let lower_instrument = root.path().join("data/test/instrument");
        let lower_enchantment = root.path().join("data/test/enchantment");
        std::fs::create_dir_all(&lower_instrument).unwrap();
        std::fs::create_dir_all(&lower_enchantment).unwrap();
        std::fs::write(lower_instrument.join("conditional.json"),
            br#"{"sound_event":"test:lower","use_duration":3,"range":32,"description":"Lower horn"}"#).unwrap();
        std::fs::write(
            lower_enchantment.join("conditional.json"),
            enchantment.to_string(),
        )
        .unwrap();
        let guard = json!([{"type": "neoforge:tag_empty", "tag": "test:gate"}]);
        enchantment["neoforge:conditions"] = guard.clone();
        let files = [
            (
                "instrument/test:conditional".into(),
                vec![
                json!({"sound_event": {"sound_id": "test:horn"}, "use_duration": 5, "range": 64,
                    "description": "Conditional horn", "neoforge:conditions": guard})
                    .to_string().into_bytes()
            ],
            ),
            (
                "enchantment/test:conditional".into(),
                vec![enchantment.to_string().into_bytes()],
            ),
        ];
        for (tag, admitted) in [("test:unrelated", true), ("test:gate", false)] {
            let mut items = Items::for_tests(std::collections::HashMap::from([(
                tag.to_owned(),
                std::collections::HashSet::from(["minecraft:stick".to_owned()]),
            )]));
            let conditions =
                sunrise_world::conditions::LoadConditions::new([]).with_tags(items.bound_tags());
            let overlay = PackOverlay {
                conditions: &conditions,
                files: &files,
                ..PackOverlay::default()
            };
            let packed = super::super::SyncData::stage_registry_definitions(
                root.path(),
                &defaults,
                &overlay,
                &Default::default(),
                &Default::default(),
                |_| true,
            )
            .unwrap();
            // Gameplay must not evaluate the definition conditions with this other context.
            packed
                .install_items(
                    &mut items,
                    root.path(),
                    &PackOverlay::new(&files),
                    &defaults,
                )
                .unwrap();
            assert_eq!(
                items
                    .instruments
                    .get("test:conditional")
                    .map(|i| i.sound.as_str()),
                admitted.then_some("test:horn")
            );
            assert_eq!(
                items.enchantments.get("test:conditional").is_some(),
                admitted
            );
            assert_eq!(
                items
                    .component_registry
                    .enchantments
                    .contains(&"test:conditional".to_owned()),
                admitted
            );
            for registry in ["minecraft:instrument", "minecraft:enchantment"] {
                let mut wire = Vec::new();
                packed.append_encoded(registry, &mut wire);
                assert_eq!(
                    wire.first().map(|((ns, id), _, from_pack)| (
                        ns.as_str(),
                        id.as_str(),
                        *from_pack
                    )),
                    admitted.then_some(("test", "conditional", true)),
                    "{registry}"
                );
            }
        }
    }

    /// What world open makes of `files` over a resource root holding the three
    /// vanilla dialogs and their tags (`stage` is the load the world open runs).
    fn load_dialogs(
        files: &[(String, Vec<Vec<u8>>)],
    ) -> Result<Vec<((String, String), Tag, bool)>, String> {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("items.json"),
            json!({"minecraft:stone": {}, "minecraft:air": {}}).to_string(),
        )
        .unwrap();
        let dialogs = root.path().join("data/minecraft/dialog");
        let tags = root.path().join("data/minecraft/tags/dialog");
        std::fs::create_dir_all(&dialogs).unwrap();
        std::fs::create_dir_all(&tags).unwrap();
        for name in ["custom_options", "quick_actions", "server_links"] {
            std::fs::write(
                dialogs.join(format!("{name}.json")),
                br#"{"type":"notice","title":"Vanilla"}"#,
            )
            .unwrap();
        }
        for name in ["pause_screen_additions", "quick_actions"] {
            std::fs::write(
                tags.join(format!("{name}.json")),
                br#"{"values":["minecraft:quick_actions"]}"#,
            )
            .unwrap();
        }
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let reg = crate::command::Registries {
            items: defaults.entries().keys().cloned().collect(),
            ..Default::default()
        };
        let (registry, result) = PackRegistryDefinitions::stage(
            root.path(),
            &defaults,
            &PackOverlay::new(files),
            &BTreeMap::new(),
            &reg,
            |_| true,
        );
        result?;
        let mut entries = Vec::new();
        registry.append_encoded("minecraft:dialog", &mut entries);
        Ok(entries)
    }

    /// Cases the dialog fixture leaves to the typed component codec: a persistent
    /// component patch's keys, values, removal payloads and duplicates.
    const COMPONENT_PATCH_CASES: &[&str] = &[
        "body/component/unknown_key",
        "body/component/unknown_removal",
        "body/component/wrong_type",
        "body/component/wrong_type_negative",
        "body/component/removal_number",
        "body/component/removal_string",
        "body/component/removal_list",
        "body/component/duplicate_after_namespace",
        "body/component/remove_and_removal_namespace",
        "body/component/invalid_key_chars",
        "body/component/not_transient_only",
        "body/component/bad_nested_text",
        "text_show_item/unknown_key",
        "text_show_item/wrong_type",
        "text_show_item/removal_number",
        "text_show_item/duplicate_after_namespace",
        "text_show_item/in_button_label",
        "text_show_item/in_body",
    ];

    const DIALOG_FIXTURE: &str = include_str!("../../tests/fixtures/dialog_codec_cases.json");

    #[test]
    fn dialog_packets_refuse_bad_components_and_unbound_values() {
        use crate::dialog::Dialog;
        let fixture: Value = serde_json::from_str(DIALOG_FIXTURE).unwrap();
        let reg = crate::command::Registries {
            items: vec!["minecraft:stone".into()],
            ..Default::default()
        };
        for case in fixture["packets"].as_array().unwrap() {
            let (tag, _) = sunrise_core::nbt::parse_snbt(case["snbt"].as_str().unwrap()).unwrap();
            let parse = |tag| Dialog::parse(tag, &reg, &|id| id == "minecraft:stone", &|_| false);
            let decoded = parse(&tag);
            assert_eq!(
                decoded.is_some(),
                case["accept"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if case["observe"] == "inline_custom_activation" {
                use crate::{dialog::Click, text::ClickEvent};
                let dialog = decoded.unwrap();
                let Some(ClickEvent::ShowDialog(holder)) = &dialog.title.spans[0].style.click
                else {
                    panic!("title must open an inline dialog");
                };
                let opened = parse(holder).unwrap();
                let buttons = opened.buttons();
                let button = buttons[0];
                let Some(Click::Custom { id, payload }) =
                    button.action.as_ref().unwrap().click(&[], &[])
                else {
                    panic!("button must activate a custom click");
                };
                let expected = &case["round_trip"];
                let (expected_payload, _) =
                    sunrise_core::nbt::parse_snbt(expected["payload"].as_str().unwrap()).unwrap();
                assert_eq!(
                    json!({"id": id, "width": button.width}),
                    json!({"id": expected["id"], "width": expected["width"]})
                );
                assert_eq!(payload, Some(expected_payload));
            }
        }
    }

    #[test]
    fn admitted_dialogs_emit_the_decoded_values_over_nbt() {
        use crate::dialog::{Body, Control, Dialog, Reference};
        use crate::text::ClickEvent;
        use sunrise_core::nbt::{read_network, write_network};
        let fixture: Value = serde_json::from_str(DIALOG_FIXTURE).unwrap();
        let reg = crate::command::Registries {
            items: vec!["minecraft:stone".into()],
            ..Default::default()
        };
        let parse = |tag: &Tag| {
            Dialog::parse(
                tag,
                &reg,
                &|id| id == "minecraft:stone",
                &|reference| matches!(reference, Reference::Id(id) if id == "minecraft:quick_actions"),
            )
        };
        let mut stacks = Vec::new();
        for case in fixture["cases"].as_array().unwrap() {
            let Some(observe) = case["observe"].as_str() else {
                continue;
            };
            let name = case["name"].as_str().unwrap();
            let files = [(
                "dialog/test:case".into(),
                vec![serde_json::to_vec(&case["dialog"]).unwrap()],
            )];
            let entries = load_dialogs(&files).unwrap_or_else(|error| panic!("{name}: {error}"));
            let (_, emitted, from_pack) = &entries[0];
            assert!(from_pack);
            let bytes = write_network(emitted, Default::default()).unwrap();
            let (tag, used) = read_network(&bytes, Default::default()).unwrap();
            assert_eq!(used, bytes.len());
            let mut dialog =
                parse(&tag).unwrap_or_else(|| panic!("{name}: refused emitted dialog"));
            if observe == "inline_button" {
                let Some(ClickEvent::ShowDialog(inner)) = dialog
                    .title
                    .spans
                    .iter()
                    .find_map(|span| span.click_event())
                else {
                    panic!("{name}: inline holder was lost");
                };
                dialog =
                    parse(inner).unwrap_or_else(|| panic!("{name}: refused emitted inline dialog"));
            }
            let actual = match observe {
                "button" | "inline_button" => {
                    let button = dialog.buttons()[0];
                    let label = button
                        .label
                        .source_component()
                        .and_then(|component| component.get("translate"))
                        .and_then(Value::as_str)
                        .map_or_else(|| button.label.plain(), str::to_owned);
                    json!({"label": label, "width": button.width, "has_action": button.action.is_some()})
                }
                "named_holder" => {
                    let Some(Tag::String(id)) = tag
                        .get("title")
                        .and_then(|title| title.get("click_event"))
                        .and_then(|click| click.get("dialog"))
                    else {
                        panic!("{name}: missing emitted holder")
                    };
                    json!({"id": id.to_string_lossy()})
                }
                "item" => {
                    let Body::Item { item, .. } = &dialog.body[0] else {
                        panic!("{name}: expected item body")
                    };
                    // The exact stack reader the renderer uses. Status reads the same id.
                    let stack = sunrise_items::item_nbt::item_from_nbt(item).unwrap();
                    let id = stack.item.as_ref().to_owned();
                    let count = stack.count;
                    assert_eq!(
                        emitted
                            .get("body")
                            .and_then(|body| match body {
                                Tag::List(items) => items.first(),
                                _ => None,
                            })
                            .and_then(|body| body.get("item"))
                            .and_then(|stack| stack.get("count")),
                        Some(&Tag::Int(count as i32))
                    );
                    assert_eq!(dialog.json(&[])["body"][0], id);
                    stacks.push((id.clone(), count));
                    json!({"id": id, "count": count})
                }
                "multiline" => {
                    let Control::Text {
                        multiline: Some((max_lines, height)),
                        ..
                    } = &dialog.inputs[0].control
                    else {
                        panic!("{name}: expected multiline control")
                    };
                    json!({"max_lines": max_lines, "height": crate::dialog_screen::text_height(Some((*max_lines, *height))) as i32})
                }
                _ => panic!("{name}: unknown observation {observe}"),
            };
            assert_eq!(actual, case["round_trip"], "{name}");
        }
        assert_eq!(stacks.len(), 2);
        assert_eq!(stacks[0], stacks[1]);
    }

    /// The dialog codec fixture holds what real 26.3 answered
    /// (`Dialog.DIRECT_CODEC` for each case, `RegistryDataLoader` for each pack),
    /// recorded by `tools/test_dialog_codec_parity.py`.
    #[test]
    fn dialog_admission_answers_as_the_real_codecs_do() {
        let fixture: Value = serde_json::from_str(DIALOG_FIXTURE).unwrap();
        let mut wrong = Vec::new();
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let accept = case["accept"].as_bool().unwrap();
            // Cross-entry references are the packs' cases below.
            if COMPONENT_PATCH_CASES.contains(&name)
                || [
                    "action/show_dialog_unknown_id",
                    "list/dialog_list_unknown_id",
                    "list/dialog_list_unknown_tag",
                    "list/dialog_list_second_unknown",
                ]
                .contains(&name)
            {
                continue;
            }
            let files = [(
                "dialog/test:case".to_owned(),
                vec![serde_json::to_vec(&case["dialog"]).unwrap()],
            )];
            let result = load_dialogs(&files);
            if result.is_ok() != accept {
                wrong.push(format!(
                    "{name}: vanilla {}, Sunrise {}",
                    if accept {
                        "accepts".to_owned()
                    } else {
                        format!("rejects ({})", case["vanilla_error"].as_str().unwrap())
                    },
                    match result {
                        Ok(_) => "accepts".to_owned(),
                        Err(error) => format!("rejects ({error})"),
                    },
                ));
            }
        }
        for case in fixture["packs"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let mut files: Vec<(String, Vec<Vec<u8>>)> = case["files"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(path, value)| {
                    let rest = path.strip_prefix("data/").unwrap();
                    let (namespace, rest) = rest.split_once('/').unwrap();
                    let key = match rest.strip_prefix("tags/dialog/") {
                        Some(tag) => {
                            format!("tags/{namespace}:dialog/{}", tag.trim_end_matches(".json"))
                        }
                        None => format!(
                            "dialog/{namespace}:{}",
                            rest.strip_prefix("dialog/")
                                .unwrap()
                                .trim_end_matches(".json")
                        ),
                    };
                    (key, vec![serde_json::to_vec(value).unwrap()])
                })
                .collect();
            if let Some(length) = case["tag_chain"].as_u64() {
                files.extend((0..length).map(|i| {
                    let value = if i + 1 == length {
                        "test:leaf".into()
                    } else {
                        format!("#test:chain/{:05}", i + 1)
                    };
                    (
                        format!("tags/test:dialog/chain/{i:05}"),
                        vec![serde_json::to_vec(&json!({"values": [value]})).unwrap()],
                    )
                }));
            }
            let result = load_dialogs(&files);
            let accept = case["accept"].as_bool().unwrap();
            if result.is_ok() != accept {
                wrong.push(format!(
                    "{name}: vanilla accept={accept}, Sunrise {result:?}"
                ));
            }
            if let (Ok(entries), Some(expected)) = (result, case["round_trip"].as_object()) {
                let root = tempfile::tempdir().unwrap();
                let held: BTreeSet<_> = entries
                    .iter()
                    .map(|((namespace, path), _, _)| format!("{namespace}:{path}"))
                    .collect();
                let (bound, _) = bind_dialog_tags(root.path(), &PackOverlay::new(&files), |id| {
                    held.contains(id)
                });
                for (tag, members) in expected {
                    assert_eq!(json!(bound.get(tag)), *members, "{name}: {tag}");
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} cases differ:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    #[test]
    fn every_bad_winning_definition_is_reported_in_one_error() {
        let mut oversized = enchantment("Oversized");
        // Each string fits modified UTF-8, but together they exceed the 2 MiB quota.
        oversized["description"] = json!(vec!["x".repeat(64_000); 33]);
        assert!(
            sunrise_core::nbt::write_network(&json_to_nbt(&oversized), Default::default()).is_ok()
        );
        let files = [
            (
                "dialog/test:broken".into(),
                vec![
                    br#"{"type":"notice","title":"Lower"}"#.to_vec(),
                    b"{".to_vec(),
                ],
            ),
            (
                "dialog/test:negative_columns".into(),
                vec![br#"{"type":"multi_action","title":"Buttons","actions":[{"label":"OK"}],"columns":-1}"#.to_vec()],
            ),
            (
                "dialog/test:wrong_columns".into(),
                vec![br#"{"type":"dialog_list","title":"Buttons","dialogs":[],"columns":true}"#.to_vec()],
            ),
            (
                "dialog/test:wide_button".into(),
                vec![br#"{"type":"server_links","title":"Links","button_width":1025}"#.to_vec()],
            ),
            (
                "dialog/test:wrong_pause".into(),
                vec![br#"{"type":"notice","title":"Pause","pause":1}"#.to_vec()],
            ),
            (
                "dialog/test:wrong_action".into(),
                vec![br#"{"type":"notice","title":"Action","after_action":"stay"}"#.to_vec()],
            ),
            (
                "dialog/test:condition_false".into(),
                vec![br#"{"neoforge:conditions":[{"type":"neoforge:never"}]}"#.to_vec()],
            ),
            (
                "dimension_type/test:wrong_scale".into(),
                vec![br#"{"min_y":0,"height":64,"logical_height":64,"has_skylight":true,"has_ceiling":false,"has_ender_dragon_fight":false,"infiniburn":["minecraft:netherrack"],"coordinate_scale":"one","ambient_light":0,"monster_spawn_light_level":0,"monster_spawn_block_light_limit":0}"#.to_vec()],
            ),
            (
                "dimension_type/test:unknown_attribute".into(),
                vec![br#"{"min_y":0,"height":64,"logical_height":64,"has_skylight":true,"has_ceiling":false,"has_ender_dragon_fight":false,"infiniburn":["minecraft:netherrack"],"coordinate_scale":1,"ambient_light":0,"monster_spawn_light_level":0,"monster_spawn_block_light_limit":0,"attributes":{"minecraft:visual/not_an_attribute":1}}"#.to_vec()],
            ),
            (
                "dimension_type/test:empty_bed_rule".into(),
                vec![br#"{"min_y":0,"height":64,"logical_height":64,"has_skylight":true,"has_ceiling":false,"has_ender_dragon_fight":false,"infiniburn":["minecraft:netherrack"],"coordinate_scale":1,"ambient_light":0,"monster_spawn_light_level":0,"monster_spawn_block_light_limit":0,"attributes":{"minecraft:gameplay/bed_rule":{}}}"#.to_vec()],
            ),
            ("enchantment/test:not_read".into(), vec![b"{}".to_vec()]),
            ("enchantment/test:wrong_level".into(), vec![br#"{"max_level":"seven"}"#.to_vec()]),
            (
                "enchantment/test:unencodable".into(),
                vec![enchantment(&"\0".repeat(32_768)).to_string().into_bytes()],
            ),
            (
                "enchantment/test:oversized".into(),
                vec![oversized.to_string().into_bytes()],
            ),
            ("painting_variant/test:numeric_style".into(), vec![br#"{"asset_id":"test:mural","width":1,"height":1,"title":{"text":"No","bold":1}}"#.to_vec()]),
            ("painting_variant/test:numeric_object".into(), vec![br#"{"asset_id":"test:mural","width":1,"height":1,"author":{"object":7,"sprite":"test:icon"}}"#.to_vec()]),
            ("painting_variant/test:wrong_atlas".into(), vec![br#"{"asset_id":"test:mural","width":1,"height":1,"author":{"object":"atlas","atlas":7,"sprite":"test:icon","player":"Steve"}}"#.to_vec()]),
            ("painting_variant/test:legacy_click".into(), vec![br#"{"asset_id":"test:mural","width":1,"height":1,"title":{"text":"Label","hover_event":{"action":"show_text","value":{"text":"Click","click_event":{"action":"run_command","value":"/help"}}}}}"#.to_vec()]),
            (
                "painting_variant/test:bad".into(),
                vec![br#"{"asset_id":"test:mural","width":17,"height":1}"#.to_vec()],
            ),
            (
                "instrument/test:bad".into(),
                vec![br#"{"sound_event":"minecraft:item.goat_horn.sound.0","use_duration":-1,"range":128,"description":"Broken"}"#.to_vec()],
            ),
            (
                "instrument/test:wrong_sound_range".into(),
                vec![br#"{"sound_event":{"sound_id":"test:sound","range":"near"},"use_duration":4,"range":128,"description":"Horn"}"#.to_vec()],
            ),
            (
                "banner_pattern/test:bad".into(),
                vec![br#"{"asset_id":"test:banner","translation_key":7}"#.to_vec()],
            ),
            (
                "banner_pattern/test:unencodable".into(),
                vec![serde_json::json!({"asset_id": "test:banner", "translation_key": "\0".repeat(32_768)})
                    .to_string()
                    .into_bytes()],
            ),
        ];
        let io = BTreeMap::from([("enchantment/test:not_read".into(), "read failed".into())]);
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("items.json"), b"{}").unwrap();
        let instruments = root.path().join("data/test/instrument");
        std::fs::create_dir_all(&instruments).unwrap();
        std::fs::write(
            instruments.join("resource_duration.json"),
            br#"{"sound_event":"test:horn","use_duration":"four","range":128,"description":"Broken resource horn"}"#,
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
        let Err(error) = super::super::SyncData::stage_registry_definitions(
            root.path(),
            &defaults,
            &PackOverlay::new(&files),
            &io,
            &Default::default(),
            |_| true,
        ) else {
            panic!("world registry load accepted invalid entries");
        };
        for (registry, defs) in [
            (
                "dialog",
                &[
                    ("broken", ""),
                    ("negative_columns", "columns"),
                    ("wrong_columns", "columns"),
                    ("wide_button", "button_width"),
                    ("wrong_pause", "pause"),
                    ("wrong_action", "after_action"),
                ][..],
            ),
            (
                "dimension_type",
                &[
                    ("wrong_scale", "coordinate_scale"),
                    ("empty_bed_rule", "bed_rule"),
                    ("unknown_attribute", "minecraft:visual/not_an_attribute"),
                ],
            ),
            (
                "enchantment",
                &[
                    ("not_read", "read failed"),
                    ("oversized", "byte limit exceeded"),
                    ("wrong_level", "max_level"),
                    ("unencodable", "modified UTF-8"),
                ],
            ),
            (
                "painting_variant",
                &[
                    ("numeric_style", "title"),
                    ("numeric_object", "author"),
                    ("wrong_atlas", "author"),
                    ("legacy_click", "title"),
                    ("bad", "width"),
                ],
            ),
            (
                "instrument",
                &[
                    ("bad", "use_duration"),
                    ("resource_duration", "use_duration"),
                    ("wrong_sound_range", "sound_event.range"),
                ],
            ),
            (
                "banner_pattern",
                &[
                    ("bad", "translation_key"),
                    ("unencodable", "modified UTF-8"),
                ],
            ),
        ] {
            let header = format!("\nminecraft:{registry}:\n");
            let group = error
                .split_once(&header)
                .unwrap()
                .1
                .split("\nminecraft:")
                .next()
                .unwrap();
            for (id, field) in defs {
                let entry = format!("  test:{id}:");
                assert!(
                    group
                        .lines()
                        .any(|line| line.starts_with(&entry) && line.contains(field)),
                    "{error}"
                );
            }
        }
        assert!(!error.contains("condition_false"));
    }
}
