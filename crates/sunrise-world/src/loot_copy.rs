//! Shared loot source snapshots and copy-function evaluation.
#[path = "loot_copy/custom_data.rs"]
mod custom_data;
pub use custom_data::{NbtSource, copy_custom_data};

/// Command-storage namespaces and their named compounds, retained by copy-on-write snapshots.
pub type StorageData = std::collections::BTreeMap<String, std::collections::BTreeMap<String, Tag>>;
use super::LootStack;
use serde_json::Value;
use std::borrow::Cow;
use std::{cell::RefCell, sync::Arc};
use sunrise_core::nbt::{Compound, Tag};

/// Where a loot roll happens, as a mod's own loot kinds see it:
/// `LootContextParams.ORIGIN`, `LootContextParams.BLOCK_ENTITY`, and what
/// vanilla's own conditions read of `LootContext.getLevel` (`TimeCheck`:
/// the time; `WeatherCheck`: the rain and the thunder). The game sets it
/// around a roll with [`with_native_loot_place`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LootPlace {
    /// `ORIGIN`: a broken block's centre, a dead mob's position.
    pub origin: Option<[f64; 3]>,
    /// `BLOCK_ENTITY`: the broken block's entity as it was before the
    /// break, saved with its id and position (`saveWithFullMetadata`); a
    /// mod's block entity carries its own data and its `Items`.
    pub block_entity: Option<Tag>,
    /// The block entity's collected components, separate from its saved NBT.
    pub block_components: Option<Compound>,
    /// Server command storage, shared without cloning its compounds on a roll.
    pub storage: Option<Arc<StorageData>>,
    /// The level the roll is in.
    pub level: Option<LootLevel>,
}

/// The level a loot roll is in, as a mod's own loot kinds see it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LootLevel {
    /// The dimension's id (`minecraft:overworld`).
    pub dimension: String,
    /// `Level.getGameTime`.
    pub game_time: i64,
    /// `Level.getDayTime`: it counts on from day to day, so the time of
    /// day is `day_time % 24000` (night from 13000 to 23000).
    pub day_time: i64,
    /// `Level.isRaining`.
    pub raining: bool,
    /// `Level.isThundering`.
    pub thundering: bool,
    /// `ClockManager.getInstance(clock).totalTicks` of each world clock the level has, by id.
    pub clocks: std::collections::BTreeMap<String, i64>,
}

thread_local! {
    static NATIVE_LOOT_PLACE: RefCell<Option<Arc<LootPlace>>> = const { RefCell::new(None) };
}

/// Runs `roll` with `place` as where its loot rolls happen: every
/// [`crate::loot::LootContext`] made inside it, by either engine, carries it. The
/// place before is back when `roll` returns, so these nest.
pub fn with_native_loot_place<T>(place: LootPlace, roll: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<LootPlace>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let before = self.0.take();
            NATIVE_LOOT_PLACE.with(|p| *p.borrow_mut() = before);
        }
    }
    let _restore = Restore(NATIVE_LOOT_PLACE.with(|p| p.replace(Some(Arc::new(place)))));
    roll()
}

/// The place [`with_native_loot_place`] set for the rolls now running, if any.
pub fn current_native_loot_place() -> Option<Arc<LootPlace>> {
    NATIVE_LOOT_PLACE.with(|p| p.borrow().clone())
}

/// Source captured before a block entity is removed. Copy policy belongs to the table.
#[derive(Clone, Debug)]
pub struct BlockSource {
    pub nbt: Tag,
    pub components: Compound,
}

/// Lend a removed block's retained source without losing an outer roll's origin or level.
pub fn with_block_source<T>(source: Option<&BlockSource>, roll: impl FnOnce() -> T) -> T {
    let Some(source) = source else { return roll() };
    let mut place = current_native_loot_place()
        .as_deref()
        .cloned()
        .unwrap_or_default();
    place.block_entity = Some(source.nbt.clone());
    place.block_components = Some(source.components.clone());
    with_native_loot_place(place, roll)
}

/// Lend a stable view of server command storage to this roll.
pub fn with_storage<T>(storage: Arc<StorageData>, roll: impl FnOnce() -> T) -> T {
    let mut place = current_native_loot_place()
        .as_deref()
        .cloned()
        .unwrap_or_default();
    place.storage = Some(storage);
    with_native_loot_place(place, roll)
}

/// Resolve providers identically for both evaluators; entity selection is adapter-owned.
pub fn nbt_source<'a>(
    source: NbtSource<'_>,
    place: Option<&'a LootPlace>,
    entity: impl FnOnce(&str) -> Option<&'a super::LootEntity>,
) -> Option<Cow<'a, Tag>> {
    match source {
        NbtSource::Context("block_entity") => place?.block_entity.as_ref().map(Cow::Borrowed),
        NbtSource::Context(target) => entity(target)?.nbt.as_deref().map(Cow::Borrowed),
        NbtSource::Storage(id) => {
            let storage = place?.storage.as_ref()?;
            Some(
                match storage
                    .get(id.namespace())
                    .and_then(|entries| entries.get(id.path()))
                {
                    Some(tag) => Cow::Borrowed(tag),
                    None => Cow::Owned(Tag::Compound(Compound::new())),
                },
            )
        }
    }
}

impl super::LootData {
    pub(in crate::loot) fn refresh_copy_requirements(&mut self) {
        self.copies_entity_nbt = self
            .tables
            .ids()
            .map(|id| &self.tables[id])
            .chain(self.modifiers.values())
            .any(may_read_entity_nbt);
    }
}

/// Cold, conservative metadata: ordinary tables need no entity serialization.
fn may_read_entity_nbt(value: &Value) -> bool {
    let kind = value["type"]
        .as_str()
        .map(|kind| kind.strip_prefix("minecraft:").unwrap_or(kind));
    let reads_entity = match kind {
        Some("copy_components") => {
            !matches!(value["source"].as_str(), Some("block_entity" | "tool"))
        }
        Some("copy_custom_data") => {
            let source = &value["source"];
            source.as_str() != Some("block_entity")
                && !matches!(
                    source["type"].as_str(),
                    Some("storage" | "minecraft:storage")
                )
                && !(matches!(
                    source["type"].as_str(),
                    Some("context" | "minecraft:context")
                ) && source["target"].as_str() == Some("block_entity"))
        }
        _ => false,
    };
    reads_entity
        || match value {
            Value::Object(object) => object.values().any(may_read_entity_nbt),
            Value::Array(array) => array.iter().any(may_read_entity_nbt),
            _ => false,
        }
}

/// Entity getters include intrinsic empty custom data even when it was not saved.
pub fn entity_components(entity: &super::LootEntity) -> Cow<'_, Compound> {
    let mut components = entity.components.clone();
    components
        .entry("minecraft:custom_data".into())
        .or_insert_with(|| {
            entity
                .nbt
                .as_deref()
                .and_then(|nbt| nbt.get("data"))
                .and_then(|value| crate::custom_data::borrow(value).ok())
                .map(|data| Tag::Compound(data.into_owned()))
                .unwrap_or_else(|| Tag::Compound(Compound::new()))
        });
    if let Some(name) = entity.nbt.as_deref().and_then(|nbt| nbt.get("CustomName")) {
        components
            .entry("minecraft:custom_name".into())
            .or_insert_with(|| name.clone());
    }
    Cow::Owned(components)
}

/// Copy only values the selected source actually has. Include and exclude intersect;
/// exclusion wins, and an absent source leaves the candidate untouched.
pub fn copy_components<'a>(
    function: &Value,
    source: impl FnOnce(&str) -> Option<Cow<'a, Compound>>,
    mut apply: impl FnMut(String, &Tag),
) {
    let filter =
        |field| -> Option<Option<std::collections::HashSet<sunrise_core::nbt::JavaString>>> {
            let Some(value) = function.get(field) else {
                return Some(None);
            };
            value
                .as_array()?
                .iter()
                .map(|value| {
                    let id = sunrise_core::identifier::Identifier::parse(value.as_str()?).ok()?;
                    Some(id.to_string().as_str().into())
                })
                .collect::<Option<_>>()
                .map(Some)
        };
    let (Some(includes), Some(excludes)) = (filter("include"), filter("exclude")) else {
        return;
    };
    let Some(source) = function["source"].as_str().and_then(source) else {
        return;
    };
    for (id, value) in source.iter() {
        if includes.as_ref().is_some_and(|list| !list.contains(id))
            || excludes.as_ref().is_some_and(|list| list.contains(id))
        {
            continue;
        }
        apply(id.to_string_lossy(), value);
    }
}

/// Preserve the exact loaded prototype, including native default-component changes.
pub fn component_defaults(
    defaults: &crate::item_defaults::ItemDefaults,
) -> Result<Arc<std::collections::HashMap<String, Compound>>, String> {
    defaults
        .entries()
        .iter()
        .map(|(id, value)| {
            let object = value
                .as_object()
                .ok_or_else(|| format!("{id}: component prototype is not an object"))?;
            Ok((
                id.clone(),
                sunrise_core::json_nbt::json_to_compound(object)
                    .map_err(|error| error.to_string())?,
            ))
        })
        .collect::<Result<_, String>>()
        .map(Arc::new)
}

impl super::LootData {
    pub fn component_defaults(&self) -> &Arc<std::collections::HashMap<String, Compound>> {
        &self.component_defaults
    }
}

impl<R: sunrise_core::random::RandomSource> super::Context<'_, R> {
    pub(super) fn effective_custom_data<'a>(
        &'a self,
        stack: &'a LootStack,
    ) -> Result<Option<Cow<'a, Compound>>, String> {
        let id = "minecraft:custom_data";
        let default = self
            .data
            .component_defaults
            .get(&stack.item)
            .and_then(|prototype| prototype.get(&id.into()));
        let raw = match stack
            .components
            .iter()
            .rev()
            .find(|(key, _)| key == id || key == "!minecraft:custom_data")
        {
            Some((key, value)) if key == id => value.as_ref(),
            None => default,
            _ => None,
        };
        raw.map(crate::custom_data::borrow).transpose()
    }

    pub(super) fn copy_source_custom_data(&self, function: &Value, stack: &mut LootStack) {
        let id = "minecraft:custom_data";
        let Ok(original) = self.effective_custom_data(stack) else {
            return;
        };
        let place = current_native_loot_place();
        if let Some(data) = copy_custom_data(function, original.as_deref(), |source| {
            nbt_source(source, place.as_deref(), |target| self.target(target))
        }) {
            stack
                .components
                .retain(|(key, _)| key != id && key != "!minecraft:custom_data");
            stack
                .components
                .push((id.to_owned(), Some(Tag::Compound(data))));
        }
    }

    pub(super) fn copy_source_components(&self, function: &Value, stack: &mut LootStack) {
        let place = current_native_loot_place();
        copy_components(
            function,
            |source| match source {
                "block_entity" => place.as_ref()?.block_components.as_ref().map(Cow::Borrowed),
                "tool" => self.query.tool.as_ref().map(|tool| {
                    Cow::Owned(stack_components(
                        tool,
                        self.data.component_defaults.get(&tool.item),
                    ))
                }),
                target => self.target(target).map(entity_components),
            },
            |id, value| {
                match (id.as_str(), value) {
                    ("minecraft:damage", Tag::Int(damage)) => stack.damage = *damage,
                    ("minecraft:enchantments", Tag::Compound(levels)) => {
                        stack.enchantments = levels
                            .iter()
                            .filter_map(|(key, value)| {
                                if let Tag::Int(level) = value {
                                    Some((key.to_string_lossy(), *level))
                                } else {
                                    None
                                }
                            })
                            .collect();
                    }
                    _ => {}
                }
                stack.components.push((id, Some(value.clone())));
            },
        );
    }
}

/// Apply a saved component patch to its loaded prototype, including removals.
pub fn components_with_patch(prototype: Option<&Compound>, patch: Option<&Tag>) -> Compound {
    let mut components = prototype.cloned().unwrap_or_default();
    if let Some(Tag::Compound(patch)) = patch {
        for (id, value) in patch {
            let name = id.to_string_lossy();
            if let Some(base) = name.strip_prefix('!') {
                components.remove(&base.into());
            } else {
                components.insert(id.clone(), value.clone());
            }
        }
    }
    components
}

/// A tool's effective components: prototype plus its ordered patch and live scalars.
pub fn stack_components(stack: &LootStack, prototype: Option<&Compound>) -> Compound {
    let mut components = prototype.cloned().unwrap_or_default();
    if stack.damage != 0 || components.contains_key(&"minecraft:damage".into()) {
        components.insert("minecraft:damage".into(), Tag::Int(stack.damage));
    }
    if !stack.enchantments.is_empty() {
        components.insert(
            "minecraft:enchantments".into(),
            Tag::Compound(
                stack
                    .enchantments
                    .iter()
                    .map(|(id, level)| (id.as_str().into(), Tag::Int(*level)))
                    .collect(),
            ),
        );
    }
    for (id, value) in &stack.names {
        if let Some(value) = super::json_tag(value) {
            components.insert(id.as_str().into(), value);
        }
    }
    for (id, value) in &stack.components {
        if let Some(value) = value {
            components.insert(id.as_str().into(), value.clone());
        } else {
            components.remove(&id.as_str().into());
        }
    }
    components
}
