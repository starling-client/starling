//! Data-driven enchantment providers (`data/<ns>/enchantment_provider/*.json`):
//! `EnchantmentProvider`, `EnchantmentProviderTypes`, `EnchantmentsByCost`,
//! `EnchantmentsByCostWithDifficulty`, and `SingleEnchantment`.
use super::Enchantments;
use crate::item_data::Levels;
use crate::items::{ItemStack, Items};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, sync::Arc};
use sunrise_core::{identifier::Identifier, random::RandomSource};
use sunrise_world::{loot::PackOverlay, worldgen::value_provider::IntProvider};

/// `EnchantmentsByCostWithDifficulty.MAX_ALLOWED_VALUE_PART`.
const MAX_ALLOWED_VALUE_PART: i32 = 10_000;

/// `RegistryCodecs.holderSet(Registries.ENCHANTMENT)` decoded from JSON.
#[derive(Clone, Debug)]
pub enum EnchantmentSet {
    /// A `#namespace:path` tag reference, resolved against the live enchantment tags.
    Tag(Arc<str>),
    /// Direct holder ids (one string or a JSON array of strings).
    Ids(Vec<Arc<str>>),
}

/// One decoded `EnchantmentProvider` (`EnchantmentProviderTypes`).
#[derive(Clone, Debug)]
pub enum EnchantmentProvider {
    /// `minecraft:by_cost` (`EnchantmentsByCost`).
    ByCost {
        enchantments: EnchantmentSet,
        cost: IntProvider,
    },
    /// `minecraft:by_cost_with_difficulty` (`EnchantmentsByCostWithDifficulty`).
    ByCostWithDifficulty {
        enchantments: EnchantmentSet,
        min_cost: i32,
        max_cost_span: i32,
    },
    /// `minecraft:single` (`SingleEnchantment`).
    Single {
        enchantment: Arc<str>,
        level: IntProvider,
    },
}

impl EnchantmentProvider {
    /// `EnchantmentProvider.DIRECT_CODEC`: decode `by_cost`,
    /// `by_cost_with_difficulty`, or `single`.
    pub fn decode(value: &Value) -> Result<Self, String> {
        let fields = value.as_object().ok_or("not a map")?;
        let raw_type = fields
            .get("type")
            .and_then(Value::as_str)
            .ok_or("type: a string is required")?;
        let kind = Identifier::parse(raw_type)
            .map_err(|error| format!("type: invalid identifier {raw_type}: {error}"))?
            .to_string();
        let required = |name: &str| {
            fields
                .get(name)
                .ok_or_else(|| format!("{name} is required"))
        };
        match kind.as_str() {
            "minecraft:by_cost" => Ok(Self::ByCost {
                enchantments: decode_holder_set(required("enchantments")?)?,
                cost: decode_int_provider("cost", required("cost")?)?,
            }),
            "minecraft:by_cost_with_difficulty" => Ok(Self::ByCostWithDifficulty {
                enchantments: decode_holder_set(required("enchantments")?)?,
                min_cost: decode_int_range(
                    "min_cost",
                    required("min_cost")?,
                    1,
                    MAX_ALLOWED_VALUE_PART,
                )?,
                max_cost_span: decode_int_range(
                    "max_cost_span",
                    required("max_cost_span")?,
                    0,
                    MAX_ALLOWED_VALUE_PART,
                )?,
            }),
            "minecraft:single" => {
                let raw = required("enchantment")?
                    .as_str()
                    .ok_or("enchantment: a string is required")?;
                let id = Identifier::parse(raw)
                    .map_err(|error| format!("enchantment: invalid identifier {raw}: {error}"))?;
                Ok(Self::Single {
                    enchantment: Arc::from(id.to_string()),
                    level: decode_int_provider("level", required("level")?)?,
                })
            }
            _ => Err(format!("type: unknown enchantment provider type {kind}")),
        }
    }

    /// Every enchantment holder id directly referenced by this provider.
    pub fn referenced_enchantments(&self) -> &[Arc<str>] {
        match self {
            Self::Single { enchantment, .. } => std::slice::from_ref(enchantment),
            Self::ByCost {
                enchantments: EnchantmentSet::Ids(ids),
                ..
            }
            | Self::ByCostWithDifficulty {
                enchantments: EnchantmentSet::Ids(ids),
                ..
            } => ids,
            Self::ByCost {
                enchantments: EnchantmentSet::Tag(_),
                ..
            }
            | Self::ByCostWithDifficulty {
                enchantments: EnchantmentSet::Tag(_),
                ..
            } => &[],
        }
    }

    /// The enchantment tag referenced by this provider, if any.
    pub fn referenced_tag(&self) -> Option<&str> {
        match self {
            Self::ByCost {
                enchantments: EnchantmentSet::Tag(tag),
                ..
            }
            | Self::ByCostWithDifficulty {
                enchantments: EnchantmentSet::Tag(tag),
                ..
            } => Some(tag),
            _ => None,
        }
    }

    /// `EnchantmentProvider.enchant`: compute the `(enchantment, level)`
    /// upgrades this provider applies to `stack`.
    pub fn enchant<R: RandomSource>(
        &self,
        enchantments: &Enchantments,
        items: &Items,
        random: &mut R,
        stack: &ItemStack,
        special_multiplier: f32,
    ) -> Levels {
        match self {
            Self::Single { enchantment, level } => {
                // `Holder<Enchantment>`: an enchantment that is not registered draws nothing.
                let Some(enchantment_def) = enchantments.get(enchantment) else {
                    return Vec::new();
                };
                // vanilla's `IntProvider.sample` throws on an invalid bound; no upgrade is made.
                let Ok(sampled) = level.sample(random) else {
                    return Vec::new();
                };
                // `Mth.clamp(sample, getMinLevel(), getMaxLevel())`: `Math.min(Math.max(..))`.
                let clamped = sampled.max(1).min(enchantment_def.max_level as i32) as u32;
                vec![(Arc::clone(enchantment), clamped)]
            }
            Self::ByCost {
                enchantments: set,
                cost,
            } => {
                let Ok(cost) = cost.sample(random) else {
                    return Vec::new();
                };
                let pool = match set {
                    EnchantmentSet::Tag(tag) => enchantments.tag(tag),
                    EnchantmentSet::Ids(ids) => ids.as_slice(),
                };
                enchantments.select(items, random, stack, cost, pool)
            }
            Self::ByCostWithDifficulty {
                enchantments: set,
                min_cost,
                max_cost_span,
            } => {
                let top = *min_cost + (special_multiplier * *max_cost_span as f32) as i32;
                let Ok(cost) = random.next_int_between_inclusive(*min_cost, top) else {
                    return Vec::new();
                };
                let pool = match set {
                    EnchantmentSet::Tag(tag) => enchantments.tag(tag),
                    EnchantmentSet::Ids(ids) => ids.as_slice(),
                };
                enchantments.select(items, random, stack, cost, pool)
            }
        }
    }
}

fn decode_holder_set(value: &Value) -> Result<EnchantmentSet, String> {
    if let Some(text) = value.as_str() {
        if let Some(tag) = text.strip_prefix('#') {
            let id = Identifier::parse(tag)
                .map_err(|error| format!("enchantments: invalid tag {text}: {error}"))?;
            return Ok(EnchantmentSet::Tag(Arc::from(id.to_string())));
        }
        let id = Identifier::parse(text)
            .map_err(|error| format!("enchantments: invalid identifier {text}: {error}"))?;
        return Ok(EnchantmentSet::Ids(vec![Arc::from(id.to_string())]));
    }
    if let Some(list) = value.as_array() {
        let mut ids = Vec::with_capacity(list.len());
        for element in list {
            let text = element
                .as_str()
                .ok_or("enchantments: list elements must be strings")?;
            let id = Identifier::parse(text)
                .map_err(|error| format!("enchantments: invalid identifier {text}: {error}"))?;
            ids.push(Arc::from(id.to_string()));
        }
        return Ok(EnchantmentSet::Ids(ids));
    }
    Err("enchantments: expected a tag, enchantment id, or list of enchantment ids".into())
}

fn decode_int_provider(field: &str, value: &Value) -> Result<IntProvider, String> {
    IntProvider::from_json(&value.to_string()).map_err(|error| format!("{field}: {error}"))
}

fn decode_int_range(field: &str, value: &Value, min: i32, max: i32) -> Result<i32, String> {
    let Some(n) = value.as_i64() else {
        return Err(format!("{field}: an integer is required"));
    };
    let Ok(n) = i32::try_from(n) else {
        return Err(format!("{field}: {n} out of range [{min}, {max}]"));
    };
    if n < min || n > max {
        return Err(format!("{field}: {n} out of range [{min}, {max}]"));
    }
    Ok(n)
}

/// Load `enchantment_provider` definitions from resource files and `overlay`
/// when there is no staged `PackRegistryDefinitions` snapshot. Like
/// `RegistryDataLoader`, a decode error refuses the whole load.
pub(crate) fn load_from_resources(
    root: &Path,
    overlay: &PackOverlay<'_>,
) -> crate::Result<BTreeMap<String, EnchantmentProvider>> {
    crate::items::data_files(root, "enchantment_provider", overlay)
        .into_iter()
        .map(|(id, value)| {
            EnchantmentProvider::decode(&value)
                .map(|provider| (id.clone(), provider))
                .map_err(|error| format!("enchantment provider {id}: {error}").into())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sunrise_core::random::LegacyRandom;

    #[test]
    fn decodes_all_vanilla_provider_types_and_rejects_invalid_inputs() {
        let single = EnchantmentProvider::decode(&json!({
            "type": "single",
            "enchantment": "sharpness",
            "level": 5
        }))
        .unwrap();
        assert_eq!(
            single.referenced_enchantments(),
            &[Arc::from("minecraft:sharpness")]
        );
        assert_eq!(single.referenced_tag(), None);

        let by_cost = EnchantmentProvider::decode(&json!({
            "type": "minecraft:by_cost",
            "enchantments": ["minecraft:sharpness", "smite"],
            "cost": {"type": "minecraft:uniform", "min_inclusive": 5, "max_inclusive": 15}
        }))
        .unwrap();
        assert_eq!(
            by_cost.referenced_enchantments(),
            &[
                Arc::from("minecraft:sharpness"),
                Arc::from("minecraft:smite")
            ]
        );

        let by_difficulty = EnchantmentProvider::decode(&json!({
            "type": "minecraft:by_cost_with_difficulty",
            "enchantments": "#on_mob_spawn_equipment",
            "min_cost": 5,
            "max_cost_span": 17
        }))
        .unwrap();
        assert_eq!(
            by_difficulty.referenced_tag(),
            Some("minecraft:on_mob_spawn_equipment")
        );

        // Unknown type.
        assert!(EnchantmentProvider::decode(&json!({"type": "minecraft:unknown"})).is_err());
        // min_cost must be in 1..=10000, max_cost_span in 0..=10000.
        assert!(
            EnchantmentProvider::decode(&json!({
                "type": "minecraft:by_cost_with_difficulty",
                "enchantments": "#minecraft:on_mob_spawn_equipment",
                "min_cost": 0,
                "max_cost_span": 17
            }))
            .is_err()
        );
        assert!(
            EnchantmentProvider::decode(&json!({
                "type": "minecraft:by_cost_with_difficulty",
                "enchantments": "#minecraft:on_mob_spawn_equipment",
                "min_cost": 10001,
                "max_cost_span": 0
            }))
            .is_err()
        );
        assert!(
            EnchantmentProvider::decode(&json!({
                "type": "minecraft:by_cost_with_difficulty",
                "enchantments": "#minecraft:on_mob_spawn_equipment",
                "min_cost": 1,
                "max_cost_span": -1
            }))
            .is_err()
        );
        assert!(
            EnchantmentProvider::decode(&json!({
                "type": "minecraft:by_cost_with_difficulty",
                "enchantments": "#minecraft:on_mob_spawn_equipment",
                "min_cost": 1,
                "max_cost_span": 10001
            }))
            .is_err()
        );
        // Tag inside direct list is invalid.
        assert!(
            EnchantmentProvider::decode(&json!({
                "type": "minecraft:by_cost",
                "enchantments": ["#minecraft:on_mob_spawn_equipment"],
                "cost": 5
            }))
            .is_err()
        );
        // Invalid IntProvider.
        assert!(
            EnchantmentProvider::decode(&json!({
                "type": "minecraft:single",
                "enchantment": "minecraft:sharpness",
                "level": {"type": "minecraft:uniform", "min_inclusive": 5, "max_inclusive": 1}
            }))
            .is_err()
        );
    }

    #[test]
    fn single_provider_clamps_to_enchantment_min_and_max_level() {
        let mut enchantments = crate::enchant::tests::sample();
        let items = crate::enchant::tests::sample_items();
        let mut providers = BTreeMap::new();
        providers.insert(
            "test:too_high".into(),
            EnchantmentProvider::decode(&json!({
                "type": "minecraft:single",
                "enchantment": "minecraft:sharpness",
                "level": 99
            }))
            .unwrap(),
        );
        providers.insert(
            "test:too_low".into(),
            EnchantmentProvider::decode(&json!({
                "type": "minecraft:single",
                "enchantment": "minecraft:sharpness",
                "level": 0
            }))
            .unwrap(),
        );
        enchantments.install_providers(providers);
        let stack = ItemStack::new("minecraft:iron_sword", 1);
        let mut rng = LegacyRandom::new(1);
        assert_eq!(
            enchantments.enchant_from_provider("test:too_high", &items, &mut rng, &stack, 0.0),
            vec![(Arc::from("minecraft:sharpness"), 5)]
        );
        assert_eq!(
            enchantments.enchant_from_provider("test:too_low", &items, &mut rng, &stack, 0.0),
            vec![(Arc::from("minecraft:sharpness"), 1)]
        );
        // Unregistered provider draws nothing from random.
        let mut rng_after = rng.clone();
        assert!(
            enchantments
                .enchant_from_provider("test:missing", &items, &mut rng, &stack, 0.0)
                .is_empty()
        );
        assert_eq!(rng.next_long(), rng_after.next_long());
    }

    #[test]
    fn an_invalid_provider_file_refuses_the_whole_load() {
        let root = std::env::temp_dir().join(format!(
            "sunrise-enchantment-provider-load-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("data/demo/enchantment_provider");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("good.json"),
            json!({"type": "minecraft:single", "enchantment": "minecraft:sharpness", "level": 1})
                .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("bad.json"),
            json!({"type": "minecraft:unknown"}).to_string(),
        )
        .unwrap();
        let loaded = load_from_resources(&root, &PackOverlay::default());
        std::fs::remove_dir_all(&root).unwrap();
        assert!(loaded.is_err());
    }
}
