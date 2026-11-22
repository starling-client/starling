//! ClientboundUpdateRecipesPacket: what RecipeManager.finalizeRecipeLoading
//! synchronizes, the recipe property sets (the items each furnace, smithing
//! table slot and brewing stand slot accepts) and the stonecutter's recipes,
//! read from the data pack's recipe files.
use super::data::SyncData;
use crate::{
    crafting::{DecodedRecipe, Ingredient as ParsedIngredient, Recipe, ResolvedRecipe, Special},
    recipe_book::{KnownEntry, RecipeIndex},
    stations::{ParsedStationRecipe, Smithing},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::packets::recipes::{HolderSet, SlotDisplay};

/// RecipeManager.RECIPE_PROPERTY_SETS: each set and the ingredients it takes
/// from a recipe of the given type.
pub(crate) const PROPERTY_SETS: &[(&str, &str, &[&str])] = &[
    ("smithing_addition", "smithing", &["addition"]),
    ("smithing_base", "smithing", &["base"]),
    ("smithing_template", "smithing", &["template"]),
    ("furnace_input", "minecraft:smelting", &["ingredient"]),
    ("blast_furnace_input", "minecraft:blasting", &["ingredient"]),
    ("smoker_input", "minecraft:smoking", &["ingredient"]),
    (
        "campfire_input",
        "minecraft:campfire_cooking",
        &["ingredient"],
    ),
    ("brewing_input", "minecraft:brewing", &["input"]),
    ("brewing_reagent", "minecraft:brewing", &["reagent"]),
];

/// One ingredient projected for recipe-book wire encoding (`HolderSet` / `TagSlotDisplay`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookIngredient {
    /// Original `#tag` reference (qualified, without `#`).
    Tag(String),
    /// Resolved item ids in display/match order.
    Items(Vec<String>),
}

impl BookIngredient {
    fn from_parsed(parsed: &ParsedIngredient) -> Self {
        match &parsed.tag {
            Some(tag) => Self::Tag(sunrise_client::crafting::qualified(tag)),
            None => Self::Items(
                parsed
                    .content_candidates()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            ),
        }
    }
}

/// Typed display shape for a recipe-book entry, projected directly from a
/// decoded `ResolvedRecipe` with no intermediate JSON serialization.
#[derive(Clone, Debug, PartialEq)]
pub enum BookRecipeKind {
    Shaped {
        width: usize,
        height: usize,
        cells: Vec<Option<BookIngredient>>,
        result_item: Option<String>,
        result_count: i32,
        category: String,
    },
    Shapeless {
        ingredients: Vec<BookIngredient>,
        result_item: Option<String>,
        result_count: i32,
        category: String,
    },
    Cooking {
        kind: String,
        ingredient: BookIngredient,
        result_item: String,
        result_count: i32,
        time: i32,
        experience: f32,
        category: String,
    },
    Dye {
        target: BookIngredient,
        dye: BookIngredient,
        category: String,
    },
    Imbue {
        source: BookIngredient,
        material: BookIngredient,
        result_item: Option<String>,
        result_count: i32,
        category: String,
    },
    Transmute {
        input: BookIngredient,
        material: BookIngredient,
        min: usize,
        max: usize,
        add_material_count: bool,
        result_item: Option<String>,
        result_count: i32,
        has_result_components: bool,
        category: String,
    },
    /// Special, station, brewing, or mod recipe with no standard recipe-book display.
    NonBook,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BookRecipe {
    pub group: Option<String>,
    pub show_notification: bool,
    pub kind: BookRecipeKind,
}

impl BookRecipe {
    /// A minimal shapeless crafting recipe (`minecraft:stone` -> `minecraft:diamond`)
    /// for advancement-reward tests.
    pub fn shapeless_stub() -> Self {
        Self {
            group: None,
            show_notification: true,
            kind: BookRecipeKind::Shapeless {
                ingredients: vec![BookIngredient::Items(vec!["minecraft:stone".into()])],
                result_item: Some("minecraft:diamond".into()),
                result_count: 1,
                category: String::new(),
            },
        }
    }
}

/// Typed recipe-book projections built once from `ResolvedSources`:
/// the book entries (`recipes`), the shared display index (`displays`),
/// and `RecipeManager.unpackRecipeInfo`'s group numbers (`groups`).
#[derive(Clone, Default)]
pub struct BookProjections {
    pub recipes: BTreeMap<String, BookRecipe>,
    pub displays: RecipeIndex,
    pub groups: HashMap<String, i32>,
}

impl BookProjections {
    /// Minimal projections containing stub shapeless recipes for `ids` (for tests).
    pub fn from_shapeless(ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut recipes = BTreeMap::new();
        let mut entries = Vec::new();
        for id in ids {
            let id = id.into();
            entries.push((
                id.clone(),
                KnownEntry {
                    special: false,
                    displays: 1,
                    station: "minecraft:crafting_table",
                    notify: true,
                    result: Some("minecraft:diamond".into()),
                },
            ));
            recipes.insert(id, BookRecipe::shapeless_stub());
        }
        Self {
            recipes,
            displays: RecipeIndex::from_known_entries(entries),
            groups: HashMap::new(),
        }
    }

    /// Minimal projections containing one stub shapeless recipe (for tests).
    pub fn single_shapeless(id: &str) -> Self {
        Self::from_shapeless([id])
    }
}

/// Typed recipe-book projections built directly from the already-resolved load
/// cohort. Only decoded recipes with enumerable placement candidates
/// participate in display numbering and group numbering, keeping client
/// display ids aligned with the server's recipe index.
pub(super) fn book_projections(
    sources: &sunrise_client::crafting::ResolvedSources,
) -> crate::Result<BookProjections> {
    sources.check()?;
    let mut recipes = BTreeMap::new();
    let mut ordered_groups = Vec::new();
    for recipe in &sources.recipes {
        if !recipe.has_placement_candidates() {
            continue;
        }
        let id = sunrise_client::crafting::qualified(&recipe.name);
        if let Some(group) = recipe.group() {
            ordered_groups.push((sunrise_client::crafting::registration_key(&id), group));
        }
        recipes.insert(id, project_book_recipe(recipe));
    }
    // RecipeManager.unpackRecipeInfo numbers groups in registration order
    // (path, then namespace), not in the order of the ids' text.
    ordered_groups.sort_by(|a, b| a.0.cmp(&b.0));
    let mut groups = HashMap::new();
    for (_, g) in ordered_groups {
        let next = groups.len() as i32;
        groups.entry(g.to_owned()).or_insert(next);
    }
    let displays = RecipeIndex::from_resolved(sources);
    Ok(BookProjections {
        recipes,
        displays,
        groups,
    })
}

fn project_book_recipe(recipe: &ResolvedRecipe) -> BookRecipe {
    let group = recipe.group().map(str::to_owned);
    match recipe.decoded() {
        DecodedRecipe::Crafting {
            recipe: crafted,
            category,
            show_notification,
            ..
        } => {
            let kind = match crafted {
                Recipe::Shaped {
                    width,
                    height,
                    cells,
                    result,
                } => BookRecipeKind::Shaped {
                    width: *width,
                    height: *height,
                    cells: cells
                        .iter()
                        .map(|c| c.as_ref().map(BookIngredient::from_parsed))
                        .collect(),
                    result_item: result.item.as_deref().map(str::to_owned),
                    result_count: result.count as i32,
                    category: category.clone(),
                },
                Recipe::Shapeless {
                    ingredients,
                    result,
                } => BookRecipeKind::Shapeless {
                    ingredients: ingredients.iter().map(BookIngredient::from_parsed).collect(),
                    result_item: result.item.as_deref().map(str::to_owned),
                    result_count: result.count as i32,
                    category: category.clone(),
                },
                Recipe::Special(Special::Dye { target, dye, .. }) => BookRecipeKind::Dye {
                    target: BookIngredient::from_parsed(target),
                    dye: BookIngredient::from_parsed(dye),
                    category: category.clone(),
                },
                Recipe::Special(Special::Imbue {
                    source,
                    material,
                    result,
                }) => BookRecipeKind::Imbue {
                    source: BookIngredient::from_parsed(source),
                    material: BookIngredient::from_parsed(material),
                    result_item: result.item.as_deref().map(str::to_owned),
                    result_count: result.count as i32,
                    category: category.clone(),
                },
                Recipe::Special(Special::Transmute {
                    input,
                    material,
                    min,
                    max,
                    add_count,
                    result,
                }) => BookRecipeKind::Transmute {
                    input: BookIngredient::from_parsed(input),
                    material: BookIngredient::from_parsed(material),
                    min: *min,
                    max: *max,
                    add_material_count: *add_count,
                    result_item: result.item.as_deref().map(str::to_owned),
                    result_count: result.count as i32,
                    has_result_components: result.components.as_ref().is_some_and(|tag| {
                        !matches!(tag, sunrise_core::nbt::Tag::Compound(map) if map.is_empty())
                    }),
                    category: category.clone(),
                },
                Recipe::Special(_) | Recipe::Mod(_) => BookRecipeKind::NonBook,
            };
            BookRecipe {
                group,
                show_notification: *show_notification,
                kind,
            }
        }
        DecodedRecipe::Cooking(cook) => BookRecipe {
            group,
            show_notification: cook.show_notification,
            kind: BookRecipeKind::Cooking {
                kind: cook.kind.clone(),
                ingredient: BookIngredient::from_parsed(&cook.ingredient),
                result_item: cook.result.item.to_string(),
                result_count: cook.result.count as i32,
                time: cook.time as i32,
                experience: cook.experience,
                category: cook.category.clone(),
            },
        },
        DecodedRecipe::Station {
            show_notification, ..
        } => BookRecipe {
            group,
            show_notification: *show_notification,
            kind: BookRecipeKind::NonBook,
        },
        DecodedRecipe::Brewing { .. } | DecodedRecipe::Mod(_) | DecodedRecipe::Ignored => {
            BookRecipe {
                group,
                show_notification: true,
                kind: BookRecipeKind::NonBook,
            }
        }
    }
}

/// For each item, the packet's matching holder-set entries in button order. Values are
/// actual server recipe IDs, not a promise that the item's full stack passes the predicate.
#[derive(Debug, Default, PartialEq)]
pub struct StonecutterButtons(HashMap<Arc<str>, Vec<Arc<str>>>);

impl StonecutterButtons {
    pub fn recipe(&self, input: &str, index: usize) -> Option<&str> {
        self.0.get(input)?.get(index).map(AsRef::as_ref)
    }
}

/// The packet's payload and its stonecutter button map, built directly from
/// the decoded recipes in `sources`.
pub fn update_recipes(
    sync: &SyncData,
    sources: &sunrise_client::crafting::ResolvedSources,
) -> (Vec<u8>, StonecutterButtons) {
    let additions = &sources.property_additions;
    let tags: BTreeMap<&str, &[i32]> = sync
        .tags
        .registries
        .iter()
        .filter(|r| r.registry == "minecraft:item")
        .flat_map(|r| r.tags.iter())
        .map(|t| (t.name.trim_start_matches("minecraft:"), t.ids.as_slice()))
        .collect();
    // An item the registries do not know is left out, not sent as id 0.
    let item = |name: &str| sync.id("minecraft:item", name);
    let ingredient_ids = |parsed: &ParsedIngredient| -> Vec<i32> {
        if let Some(tag) = &parsed.tag {
            tags.get(tag.trim_start_matches("minecraft:"))
                .map(|ids| ids.to_vec())
                .unwrap_or_default()
        } else {
            parsed.order.iter().filter_map(|n| item(n)).collect()
        }
    };
    let mut w = Writer::new();
    let custom: BTreeMap<_, _> = additions.custom().collect();
    w.varint((PROPERTY_SETS.len() + custom.len()) as i32);
    for (key, kind, fields) in PROPERTY_SETS {
        let mut set = BTreeSet::new();
        for recipe in &sources.recipes {
            if !recipe.has_placement_candidates() {
                continue;
            }
            match (recipe.decoded(), *kind) {
                (
                    DecodedRecipe::Station {
                        recipe: ParsedStationRecipe::Smithing(smithing),
                        ..
                    },
                    "smithing",
                ) => {
                    let (template, base, addition) = match smithing {
                        Smithing::Transform {
                            template,
                            base,
                            addition,
                            ..
                        } => (template.as_ref(), base, addition.as_ref()),
                        Smithing::Trim {
                            template,
                            base,
                            addition,
                            ..
                        } => (Some(template), base, Some(addition)),
                    };
                    let base_ids = ingredient_ids(base);
                    let template_ids = template.map(ingredient_ids);
                    let addition_ids = addition.map(ingredient_ids);
                    // A recipe with any required empty ingredient contributes to no property set.
                    if base_ids.is_empty()
                        || template_ids.as_ref().is_some_and(Vec::is_empty)
                        || addition_ids.as_ref().is_some_and(Vec::is_empty)
                    {
                        continue;
                    }
                    for field in *fields {
                        match *field {
                            "base" => set.extend(base_ids.iter().copied()),
                            "template" => {
                                if let Some(ids) = &template_ids {
                                    set.extend(ids.iter().copied());
                                }
                            }
                            "addition" => {
                                if let Some(ids) = &addition_ids {
                                    set.extend(ids.iter().copied());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                (DecodedRecipe::Cooking(cook), kind) if cook.kind == kind => {
                    let ids = ingredient_ids(&cook.ingredient);
                    if !ids.is_empty() {
                        set.extend(ids);
                    }
                }
                (DecodedRecipe::Brewing { input, reagent, .. }, "minecraft:brewing") => {
                    let input_ids = ingredient_ids(input);
                    let reagent_ids = ingredient_ids(reagent);
                    if input_ids.is_empty() || reagent_ids.is_empty() {
                        continue;
                    }
                    for field in *fields {
                        match *field {
                            "input" => set.extend(input_ids.iter().copied()),
                            "reagent" => set.extend(reagent_ids.iter().copied()),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        // An item the registries do not know is left out (`item`).
        set.extend(
            additions
                .get(key)
                .into_iter()
                .flatten()
                .filter_map(|n| item(n)),
        );
        w.string(&format!("minecraft:{key}"));
        w.varint(set.len() as i32);
        for id in set {
            w.varint(id);
        }
    }
    for (key, names) in custom {
        let ids: BTreeSet<_> = names.iter().filter_map(|name| item(name)).collect();
        w.string(key);
        w.varint(ids.len() as i32);
        for id in ids {
            w.varint(id);
        }
    }
    // SelectableRecipe.SingleInputSet preserves the resolved source order. The server
    // numbers the same candidate items the packet shows; it still tests the real recipe.
    let mut buttons = StonecutterButtons::default();
    let mut stonecutting = Vec::new();
    let item_names = sync.builtin.get("minecraft:item");
    for recipe in &sources.recipes {
        if !recipe.has_placement_candidates() {
            continue;
        }
        let DecodedRecipe::Station {
            recipe: ParsedStationRecipe::Stonecutting(cut),
            ..
        } = recipe.decoded()
        else {
            continue;
        };
        let candidates = ingredient_ids(&cut.input);
        if candidates.is_empty() || item(&cut.result.item).is_none() {
            continue;
        }
        let id: Arc<str> = cut.id.as_str().into();
        // HolderSets represent membership; an ID only gets this button once.
        for item_id in candidates.iter().copied().collect::<BTreeSet<_>>() {
            if let Some(name) = item_names.and_then(|names| names.name(item_id)) {
                buttons.0.entry(name.into()).or_default().push(id.clone());
            }
        }
        let holders = match &cut.input.tag {
            Some(tag) => HolderSet::Tag(sunrise_client::crafting::qualified(tag)),
            None => HolderSet::Items(candidates),
        };
        let stack = super::guest::network_stack(
            &|name| item(name).unwrap_or(0),
            &|registry, name| sync.id(registry, name),
            &cut.result,
        );
        stonecutting.push((holders, SlotDisplay::ItemStack(stack)));
    }
    w.varint(stonecutting.len() as i32);
    for (input, result) in stonecutting {
        input.write(&mut w);
        result.write(&mut w);
    }
    (w.data, buttons)
}

/// The Sunrise channel a server sends its clients the recipes its native mods
/// asked for (`native_datapack_sync`), NeoForge's
/// `OnDatapackSyncEvent.sendRecipes`. Sunrise clients only.
pub const CONTENT_CHANNEL: &str = "sunrise:recipe_content";
/// `ClientboundCustomPayloadPacket`'s cap on a payload, less room for the
/// envelope.
const CONTENT_MAX: usize = 1_048_576 - 1024;

/// One payload of the recipe content: some recipes as `(id, json)`, and on the
/// last one the recipe types they replace on the client (a type the server has
/// no recipe of still empties the client's).
#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ContentPart {
    pub recipes: Vec<(String, Value)>,
    pub last: Option<Vec<String>>,
    /// Why a recipe of this part has no decodable serializer. Set by
    /// `decode`, never sent: the receiver refuses the whole update when its
    /// last part comes, not only this part.
    #[serde(skip)]
    pub refused: Option<String>,
}
impl ContentPart {
    /// Decode a content part only within the same bound used by the sender.
    /// Each recipe's `type` is decoded here, as on disk, so that the
    /// replacement and the book's numbering read the canonical id even from a
    /// server that sends a bare one. A malformed envelope is an error of this
    /// part; a malformed serializer is `refused`, which keeps `last`.
    pub fn decode(data: &[u8]) -> Result<Self, String> {
        if data.len() > CONTENT_MAX {
            return Err(format!("recipe content exceeds {CONTENT_MAX} bytes"));
        }
        let mut part: Self = serde_json::from_slice(data).map_err(|error| error.to_string())?;
        for (id, recipe) in &mut part.recipes {
            if sunrise_core::identifier::Identifier::parse_namespaced(id).is_none() {
                part.refused = Some(format!("invalid recipe id {id:?}"));
                break;
            }
            if let Err(error) = sunrise_items::recipe_sources::decode_type(recipe) {
                part.refused = Some(format!("recipe {id}: {error}"));
                break;
            }
        }
        Ok(part)
    }
}

/// Frame the already-selected recipes, with ids made whole (`minecraft:<name>`).
/// With no type
/// asked for: nothing, or with `reset` (this server sent recipes before) one
/// last part with no types, which puts its clients back on their own. A
/// recipe too big for a payload of its own is left out.
pub fn encode_content(types: &[String], sources: &[(String, Value)], reset: bool) -> Vec<Vec<u8>> {
    // The last part carries the types: they count against its cap too.
    let types_len = serde_json::to_string(types).map_or(0, |t| t.len()) + 32;
    if types_len > CONTENT_MAX {
        log::warn!("Sunrise: too many recipe types to send to clients");
        return Vec::new();
    }
    if types.is_empty() && !reset {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut part: Vec<(String, Value)> = Vec::new();
    let mut size = 0;
    for (name, v) in sources.iter().filter(|_| !types.is_empty()) {
        let id = if name.contains(':') {
            name.clone()
        } else {
            format!("minecraft:{name}")
        };
        let len = id.len() + v.to_string().len() + 8;
        if len > CONTENT_MAX {
            log::warn!("Sunrise: recipe {id} is too big to send to clients");
            continue;
        }
        if size + len > CONTENT_MAX {
            let recipes = std::mem::take(&mut part);
            out.push(
                serde_json::to_vec(&ContentPart {
                    recipes,
                    last: None,
                    refused: None,
                })
                .unwrap_or_default(),
            );
            size = 0;
        }
        size += len;
        part.push((id, v.clone()));
    }
    if size + types_len > CONTENT_MAX {
        let recipes = std::mem::take(&mut part);
        out.push(
            serde_json::to_vec(&ContentPart {
                recipes,
                last: None,
                refused: None,
            })
            .unwrap_or_default(),
        );
    }
    let last = ContentPart {
        recipes: part,
        last: Some(types.to_vec()),
        refused: None,
    };
    out.push(serde_json::to_vec(&last).unwrap_or_default());
    out
}

/// One connection's selected recipe types and encoded response. Dropped on disconnect.
#[derive(Default)]
pub struct PlayerRecipeSync {
    pub types: Vec<String>,
    pub payloads: Vec<Vec<u8>>,
    sent: bool,
}

impl PlayerRecipeSync {
    pub fn replace(&mut self, sunrise: bool, mut types: Vec<String>, sources: &[(String, Value)]) {
        types.sort();
        types.dedup();
        self.payloads = if sunrise {
            encode_content(&types, sources, self.sent)
        } else {
            Vec::new()
        };
        self.sent |= !self.payloads.is_empty();
        self.types = types;
    }
}

/// Sunrise recipe viewer transfer channel.
pub const TRANSFER_CHANNEL: &str = "sunrise:recipe_transfer";

/// Decode a transfer request: container, bounded recipe id, optional all flag.
pub fn decode_transfer_request(data: &[u8]) -> sunrise_protocol::Result<(i32, String, bool)> {
    let mut reader = sunrise_protocol::buf::Reader::new(data);
    let container = reader.varint()?;
    let id = reader.string(512)?;
    let all = match reader.remaining() {
        0 => false,
        1 => reader.u8()? == 1,
        _ => {
            return Err(sunrise_protocol::Error::new(
                "recipe transfer request: trailing bytes",
            ));
        }
    };
    Ok((container, id, all))
}

/// Decode a fixed-shape transfer result, or an empty feature advertisement.
pub fn decode_transfer_reply(data: &[u8]) -> sunrise_protocol::Result<Option<(i32, u8)>> {
    if data.is_empty() {
        return Ok(None);
    }
    let mut reader = sunrise_protocol::buf::Reader::new(data);
    let reply = (reader.varint()?, reader.u8()?);
    if reader.remaining() != 0 {
        return Err(sunrise_protocol::Error::new(
            "recipe transfer reply: trailing bytes",
        ));
    }
    Ok(Some(reply))
}

/// Sunrise's check of a viewer transfer, worked out without moving anything
/// (`Menu::check_transfer`); like `TRANSFER_CHANNEL`, an empty clientbound
/// payload advertises it. A request is the menu id, a tag the client picks and
/// the recipe's id; the reply is the menu id, the tag, the result as
/// `TRANSFER_CHANNEL` sends it (`result_byte`) and, for `Missing`, the count
/// of the recipe's cells nothing fills then each cell as a byte.
pub const CHECK_CHANNEL: &str = "sunrise:recipe_check";

/// Whether `channel` is one of the recipe viewer's (`TRANSFER_CHANNEL`,
/// `CHECK_CHANNEL`).
pub fn viewer_channel(channel: &str) -> bool {
    channel == TRANSFER_CHANNEL || channel == CHECK_CHANNEL
}

/// Tells a Sunrise client the server answers both viewer channels: an empty
/// payload on each.
pub fn advertise_viewer_channels(g: &super::guest::Guest) {
    for channel in [TRANSFER_CHANNEL, CHECK_CHANNEL] {
        g.send(
            &sunrise_protocol::packets::play::Clientbound::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: channel.into(),
                    data: Vec::new(),
                },
            ),
        );
    }
}

/// A viewer transfer's result as the engine's menu gives it.
pub fn transfer_result(
    result: Result<(), crate::menu::place::TransferError>,
) -> mod_api::RecipeTransferResult {
    use crate::menu::place::TransferError;
    use mod_api::RecipeTransferResult as R;
    match result {
        Ok(()) => R::Moves,
        Err(TransferError::Missing) => R::Missing,
        Err(TransferError::NoRoom) => R::NoRoom,
        Err(TransferError::Unavailable) => R::NotIntoMenu,
    }
}

/// A `menu_transfer` part's answer as the engine's transfer result (`transfer_result` back).
pub fn transfer_outcome(
    result: mod_api::RecipeTransferResult,
) -> Result<(), crate::menu::place::TransferError> {
    use crate::menu::place::TransferError;
    use mod_api::RecipeTransferResult as R;
    match result {
        R::Moves => Ok(()),
        R::Missing => Err(TransferError::Missing),
        R::NoRoom => Err(TransferError::NoRoom),
        R::NotIntoMenu => Err(TransferError::Unavailable),
    }
}

/// The byte both transfer channels carry a result as.
pub fn result_byte(result: mod_api::RecipeTransferResult) -> u8 {
    use mod_api::RecipeTransferResult as R;
    match result {
        R::Moves => 0,
        R::Missing => 1,
        R::NoRoom => 2,
        R::NotIntoMenu => 3,
    }
}

/// The result a transfer channel's byte names; None for any other byte.
pub fn byte_result(byte: u8) -> Option<mod_api::RecipeTransferResult> {
    use mod_api::RecipeTransferResult as R;
    Some(match byte {
        0 => R::Moves,
        1 => R::Missing,
        2 => R::NoRoom,
        3 => R::NotIntoMenu,
        _ => return None,
    })
}

/// The recipe content the native mods asked for (stored on this guest),
/// to one Sunrise client: none when no mod asked.
pub fn send_content(g: &super::guest::Guest) {
    if g.brand != "sunrise" {
        return;
    }
    for data in &g.recipe_sync.payloads {
        g.send(
            &sunrise_protocol::packets::play::Clientbound::CustomPayload(
                sunrise_protocol::packets::configuration::CustomPayload {
                    channel: CONTENT_CHANNEL.into(),
                    data: data.clone(),
                },
            ),
        );
    }
}

#[cfg(test)]
mod content_tests {
    use super::*;
    use serde_json::json;
    use sunrise_client::crafting::ResolvedSources;

    use mod_api::DataLoadOrigin;
    use sunrise_world::native_load_context::{
        DataLoadOrigin as EngineDataLoadOrigin, LoadContext, LoadInputs,
    };

    #[test]
    fn built_in_ingredient_network_lists_use_membership_not_display() {
        let items = crate::items::Items::empty();
        let resolved = ResolvedSources::resolve(
            vec![(
                "fixture:display".into(),
                json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [
                        {"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}},
                        {"type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:item","item":"minecraft:diamond"}}
                    ],
                    "result": {"id":"minecraft:emerald"}
                }),
            )],
            &items,
            &load(EngineDataLoadOrigin::ServerDataLoad),
        )
        .unwrap();
        let projections = book_projections(&resolved).unwrap();
        let BookRecipeKind::Shapeless { ingredients, .. } =
            &projections.recipes["fixture:display"].kind
        else {
            panic!("expected a shapeless display");
        };
        assert_eq!(
            ingredients,
            &[
                BookIngredient::Items(vec!["minecraft:diamond_sword".into()]),
                BookIngredient::Items(vec!["minecraft:apple".into()]),
            ]
        );
    }

    #[derive(Default)]
    struct AspectCalls {
        load: std::sync::Mutex<Vec<(DataLoadOrigin, String)>>,
        matched: std::sync::Mutex<Vec<(String, &'static str, &'static str)>>,
    }
    struct AspectListener(std::sync::Arc<AspectCalls>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for AspectListener {
        fn ingredient(
            &self,
            context: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            let kind = event.kind;
            let data = event.data;
            if kind != "mymod:aspect" {
                return None;
            }
            let sunrise_core::nbt::Tag::String(aspect) = data.get(&"aspect".into()).unwrap() else {
                panic!("expected aspect string");
            };
            let aspect = aspect.to_string().unwrap();
            let count = match aspect.as_str() {
                "nested" => 2,
                "plain" => 3,
                "banner" => 4,
                _ => 1,
            };
            self.0
                .load
                .lock()
                .unwrap()
                .push((context.origin(), aspect.clone()));
            let calls = self.0.clone();
            Some(mod_api::IngredientDefinition::new(
                vec![
                    "minecraft:emerald".into(),
                    "minecraft:lapis_lazuli".into(),
                    "minecraft:emerald".into(),
                ],
                move |_, stack| {
                    let call = mod_api::content::crash::current_native_call().unwrap();
                    calls
                        .matched
                        .lock()
                        .unwrap()
                        .push((aspect.clone(), call.id, call.hook));
                    stack.count() == count
                },
            ))
        }
    }

    struct ShadowedAspect;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ShadowedAspect {
        fn ingredient(
            &self,
            _: &dyn mod_api::DataLoadContext,
            _: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            panic!("later mod was asked after the first ingredient answer");
        }
    }

    fn load(origin: EngineDataLoadOrigin) -> LoadContext {
        LoadContext::new(
            origin,
            std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
            [],
        )
    }

    /// The `ingredient` hook hears each custom-ingredient occurrence once per
    /// load, in the load's single decode pass (`ResolvedSources::resolve`),
    /// whatever reads the recipes afterwards: repeated shaped pattern symbols,
    /// the crafting/station tables, and the guests' copy all read the decoded
    /// ingredients from that pass. Source JSON remains intact but cannot supply
    /// an answer.
    #[test]
    fn ingredient_hook_hears_each_occurrence_once_per_load_with_its_origin() {
        let calls = std::sync::Arc::new(AspectCalls::default());
        let mods = vec![
            mod_api::ModEntry::new("aspects", Box::new(AspectListener(calls.clone()))),
            mod_api::ModEntry::new("shadow", Box::new(ShadowedAspect)),
        ];
        let items = sunrise_client::items::Items::for_tests(Default::default());
        let shapeless = json!({
            "type": "minecraft:crafting_shapeless",
            "ingredients": [
                {"type": "neoforge:compound", "children": [
                    {"type": "mymod:aspect", "aspect": "nested"},
                    {"type": "mymod:aspect", "aspect": "plain"}, "minecraft:stone"]}
            ],
            "result": {"id": "minecraft:stick", "count": 1}
        });
        let banner_recipe = json!({
            "type": "minecraft:crafting_special_bannerduplicate",
            "banner": {"type": "mymod:aspect", "aspect": "banner",
                "sunrise:resolved_items": ["minecraft:diamond"]},
            "result": {"id": "minecraft:white_banner"}
        });
        let shaped_recipe = json!({
            "type": "minecraft:crafting_shaped",
            "pattern": ["AA", "A "],
            "key": {"A": {"type": "mymod:aspect", "aspect": "shaped"}},
            "result": {"id": "minecraft:stick", "count": 1}
        });
        let sources = || {
            vec![
                ("aspects_banner".to_owned(), banner_recipe.clone()),
                ("aspects_shaped".to_owned(), shaped_recipe.clone()),
                ("aspects_test".to_owned(), shapeless.clone()),
            ]
        };
        mod_platform::testing::with_mods(mods.into(), || {
            let server_origin = DataLoadOrigin::ServerDataLoad;
            let resolved = ResolvedSources::resolve(
                sources(),
                &items,
                &load(EngineDataLoadOrigin::ServerDataLoad),
            )
            .unwrap();
            let once = calls.load.lock().unwrap().clone();
            assert_eq!(
                once,
                [
                    (server_origin, "banner".to_owned()),
                    (server_origin, "shaped".to_owned()),
                    (server_origin, "nested".to_owned()),
                    (server_origin, "plain".to_owned()),
                ]
            );

            let loaded_banner = &resolved.recipes[0];
            let loaded_shaped = &resolved.recipes[1];
            let loaded_shapeless = &resolved.recipes[2];
            assert_eq!(loaded_banner.file(), &banner_recipe);
            assert_eq!(loaded_shaped.file(), &shaped_recipe);
            assert_eq!(loaded_shapeless.file(), &shapeless);
            let DecodedRecipe::Crafting {
                recipe: Recipe::Special(Special::BannerDuplicate { banner, .. }),
                ..
            } = loaded_banner.decoded()
            else {
                panic!("expected decoded banner recipe");
            };
            assert_eq!(
                banner.order,
                ["minecraft:emerald", "minecraft:lapis_lazuli"]
            );
            let context = load(EngineDataLoadOrigin::ServerDataLoad)
                .loop_context(sunrise_world::native_load_context::LoopSide::Server, []);
            let DecodedRecipe::Crafting {
                recipe: Recipe::Shapeless { ingredients, .. },
                ..
            } = loaded_shapeless.decoded()
            else {
                panic!("expected decoded shapeless recipe");
            };
            let nested = &ingredients[0];
            use sunrise_items::items::ItemStack;
            assert!(nested.test_in(&context, &ItemStack::new("minecraft:emerald", 2)));
            assert!(nested.test_in(&context, &ItemStack::new("minecraft:emerald", 3)));
            assert!(!nested.test_in(&context, &ItemStack::new("minecraft:diamond", 3)));
            {
                let mut matched = calls.matched.lock().unwrap();
                assert_eq!(
                    *matched,
                    [
                        ("nested".to_owned(), "aspects", "ingredient_predicate"),
                        ("nested".to_owned(), "aspects", "ingredient_predicate"),
                        ("plain".to_owned(), "aspects", "ingredient_predicate"),
                    ]
                );
                matched.clear();
            }
            assert_eq!(
                nested.test_in_guarded(&context, &ItemStack::new("minecraft:emerald", 3), &|| {
                    calls.matched.lock().unwrap().is_empty()
                },),
                None
            );
            assert_eq!(
                *calls.matched.lock().unwrap(),
                [("nested".to_owned(), "aspects", "ingredient_predicate"),]
            );
            // Standalone parsing without the load hook cannot supply the forged or the real answer.
            assert!(
                crate::crafting::Ingredient::parse(&loaded_banner.file()["banner"], &items)
                    .is_none()
            );

            let root = tempfile::tempdir().unwrap();
            let tables = crate::crafting::Recipes::from_resolved(
                root.path(),
                &items,
                &sunrise_world::loot::PackOverlay::NONE,
                &resolved,
            )
            .unwrap();
            assert_eq!(&*tables.by_type["minecraft:crafting"][2].json, &shapeless);

            let projections = book_projections(&resolved).unwrap();
            let BookRecipeKind::Shapeless { ingredients, .. } =
                &projections.recipes[&crate::crafting::qualified(&loaded_shapeless.name)].kind
            else {
                panic!("expected a shapeless display");
            };
            assert_eq!(
                ingredients,
                &[BookIngredient::Items(vec![
                    "minecraft:emerald".into(),
                    "minecraft:lapis_lazuli".into(),
                    "minecraft:stone".into(),
                ])]
            );
            let BookRecipeKind::Shaped { cells, .. } =
                &projections.recipes[&crate::crafting::qualified(&loaded_shaped.name)].kind
            else {
                panic!("expected a shaped display");
            };
            let candidates = BookIngredient::Items(vec![
                "minecraft:emerald".into(),
                "minecraft:lapis_lazuli".into(),
            ]);
            assert_eq!(
                cells,
                &[
                    Some(candidates.clone()),
                    Some(candidates.clone()),
                    Some(candidates),
                    None
                ]
            );
            assert_eq!(loaded_shapeless.file(), &shapeless);
            assert_eq!(*calls.load.lock().unwrap(), once);

            calls.load.lock().unwrap().clear();
            let client = ResolvedSources::resolve(
                sources(),
                &items,
                &load(EngineDataLoadOrigin::ClientPreload),
            )
            .unwrap();
            assert!(
                calls
                    .load
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|(origin, _)| *origin == DataLoadOrigin::ClientPreload)
            );
            assert_eq!(calls.load.lock().unwrap().len(), 4);
            drop(client);
        });
    }

    struct PropertyListener(std::sync::Arc<std::sync::Mutex<Vec<DataLoadOrigin>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PropertyListener {
        fn recipe_property_sets(
            &self,
            context: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::RecipePropertySetsEvent<'_>,
        ) {
            self.0.lock().unwrap().push(context.origin());
            let add = |set: &str, items: &[&str]| {
                mod_api::RecipePropertyAddition::new(
                    set,
                    items.iter().map(|i| (*i).to_owned()).collect(),
                )
            };
            event.additions.push(add(
                "minecraft:furnace_input",
                &["minecraft:emerald", "minecraft:air"],
            ));
            event
                .additions
                .push(add("minecraft:no_such_set", &["minecraft:dirt"]));
            event
                .additions
                .push(add("brewing_reagent", &["minecraft:diamond"]));
        }
    }

    /// The `recipe_property_sets` hook is asked once per load with the load's
    /// origin; the resolved set keeps the known sets' items (a set key with or
    /// without `minecraft:`), drops air and unknown sets, and the furnace
    /// takes what a mod added though no recipe names it.
    #[test]
    fn property_sets_are_asked_once_per_load_and_validated() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "props",
            Box::new(PropertyListener(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let resolved = ResolvedSources::resolve(
                Vec::new(),
                &crate::items::Items::empty(),
                &load(EngineDataLoadOrigin::ServerDataLoad),
            )
            .unwrap();
            assert_eq!(*heard.lock().unwrap(), [DataLoadOrigin::ServerDataLoad]);
            let added = &resolved.property_additions;
            assert!(added.adds("furnace_input", "minecraft:emerald"));
            assert!(!added.adds("furnace_input", "minecraft:air"));
            assert!(added.adds("brewing_reagent", "minecraft:diamond"));
            assert!(added.get("no_such_set").is_none());
            assert!(!added.adds("smoker_input", "minecraft:emerald"));
        });
    }

    /// Every vanilla recipe type and ingredient slot `Recipes`,
    /// `StationRecipes` and the guests' copy read, the shaped `key` and
    /// firework star `shapes` maps among them, decodes custom ingredients once
    /// during `ResolvedSources::resolve` and projects them without re-asking
    /// the hook.
    #[test]
    fn every_slot_the_readers_use_is_answered_by_the_pass() {
        let calls = std::sync::Arc::new(AspectCalls::default());
        let mods = vec![mod_api::ModEntry::new(
            "aspects",
            Box::new(AspectListener(calls.clone())),
        )];
        let items = sunrise_client::items::Items::for_tests(Default::default());
        let aspect = |label: &str| json!({"type": "mymod:aspect", "aspect": label});
        let result = json!({"id": "minecraft:stick"});
        let recipes = vec![
            (
                "r01_shaped".to_owned(),
                json!({"type": "minecraft:crafting_shaped", "pattern": ["a"], "key": {"a": aspect("key")}, "result": result}),
            ),
            (
                "r02_shapeless".to_owned(),
                json!({"type": "minecraft:crafting_shapeless", "ingredients": [aspect("ingredients")], "result": result}),
            ),
            (
                "r03_banner".to_owned(),
                json!({"type": "minecraft:crafting_special_bannerduplicate", "banner": aspect("banner"), "result": result}),
            ),
            (
                "r04_shield".to_owned(),
                json!({"type": "minecraft:crafting_special_shielddecoration", "banner": aspect("shield_banner"), "target": aspect("shield_target"), "result": result}),
            ),
            (
                "r05_transmute".to_owned(),
                json!({"type": "minecraft:crafting_transmute", "input": aspect("input"), "material": aspect("material"), "result": result}),
            ),
            (
                "r06_dye".to_owned(),
                json!({"type": "minecraft:crafting_dye", "target": aspect("dye_target"), "dye": aspect("dye"), "result": result}),
            ),
            (
                "r07_imbue".to_owned(),
                json!({"type": "minecraft:crafting_imbue", "source": aspect("imbue_source"), "material": aspect("imbue_material"), "result": result}),
            ),
            (
                "r08_book".to_owned(),
                json!({"type": "minecraft:crafting_special_bookcloning", "source": aspect("book_source"), "material": aspect("book_material"), "result": result}),
            ),
            (
                "r09_rocket".to_owned(),
                json!({"type": "minecraft:crafting_special_firework_rocket", "shell": aspect("shell"), "fuel": aspect("rocket_fuel"), "star": aspect("star"), "result": result}),
            ),
            (
                "r10_star".to_owned(),
                json!({"type": "minecraft:crafting_special_firework_star", "dye": aspect("star_dye"), "fuel": aspect("star_fuel"), "trail": aspect("trail"), "twinkle": aspect("twinkle"), "shapes": {"star": aspect("shapes")}, "result": result}),
            ),
            (
                "r11_fade".to_owned(),
                json!({"type": "minecraft:crafting_special_firework_star_fade", "dye": aspect("fade_dye"), "target": aspect("fade_target"), "result": result}),
            ),
            (
                "r12_map".to_owned(),
                json!({"type": "minecraft:crafting_special_mapextending", "map": aspect("map"), "material": aspect("map_material"), "result": result}),
            ),
            (
                "r13_pot".to_owned(),
                json!({"type": "minecraft:crafting_decorated_pot", "back": aspect("back"), "left": aspect("left"), "right": aspect("right"), "front": aspect("front"), "result": result}),
            ),
            (
                "r14_smelt".to_owned(),
                json!({"type": "minecraft:smelting", "ingredient": aspect("smelt"), "result": result}),
            ),
            (
                "r15_cut".to_owned(),
                json!({"type": "minecraft:stonecutting", "ingredient": aspect("cut"), "result": result}),
            ),
            (
                "r16_smith".to_owned(),
                json!({"type": "minecraft:smithing_transform", "template": aspect("smith_template"), "base": aspect("smith_base"), "addition": aspect("smith_addition"), "result": result}),
            ),
            (
                "r17_trim".to_owned(),
                json!({"type": "minecraft:smithing_trim", "template": aspect("trim_template"), "base": aspect("trim_base"), "addition": aspect("trim_addition"), "pattern": "minecraft:coast"}),
            ),
        ];
        let expected_calls = 37;
        mod_platform::testing::with_mods(mods.into(), || {
            let resolved = ResolvedSources::resolve(
                recipes,
                &items,
                &load(EngineDataLoadOrigin::ServerDataLoad),
            )
            .unwrap();
            assert_eq!(calls.load.lock().unwrap().len(), expected_calls);
            let root = tempfile::tempdir().unwrap();
            let tables = crate::crafting::Recipes::from_resolved(
                root.path(),
                &items,
                &sunrise_world::loot::PackOverlay::NONE,
                &resolved,
            )
            .unwrap();
            let _ = crate::stations::StationRecipes::from_sources(root.path(), &items, &resolved);
            let types = [
                "minecraft:crafting",
                "minecraft:smelting",
                "minecraft:stonecutting",
                "minecraft:smithing",
            ]
            .map(str::to_owned);
            let sent = tables.recipe_content(&types);
            let candidates = json!(["minecraft:emerald", "minecraft:lapis_lazuli"]);
            assert_eq!(sent[0].1["key"]["a"], candidates);
            assert_eq!(sent[9].1["shapes"]["star"], candidates);
            assert_eq!(sent[16].1["addition"], candidates);
            let received = encode_content(&types, &sent, false)
                .into_iter()
                .flat_map(|body| ContentPart::decode(&body).unwrap().recipes)
                .collect();
            let (client, _) = crate::crafting::Recipes::empty()
                .with_recipe_content(&items, received, &types)
                .unwrap();
            assert_eq!(
                client
                    .by_type
                    .values()
                    .map(|entries| entries.len())
                    .sum::<usize>(),
                17
            );
            assert_eq!(calls.load.lock().unwrap().len(), expected_calls);
        });
    }

    #[test]
    fn recipe_content_join_and_reload_keep_server_candidates_and_local_predicates() {
        use crate::crafting::Recipes;
        use crate::items::{ItemStack, Items};
        use sunrise_world::{loot::PackOverlay, native_load_context::LoopSide};
        let calls = Arc::new(AspectCalls::default());
        let mods = vec![mod_api::ModEntry::new(
            "aspects",
            Box::new(AspectListener(calls.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            let root = tempfile::tempdir().unwrap();
            let items = Items::for_tests(Default::default());
            let client_load = load(EngineDataLoadOrigin::ClientPreload);
            let server_load = load(EngineDataLoadOrigin::ServerDataLoad);
            let aspect = json!({"type":"mymod:aspect", "aspect":"plain"});
            let local = Recipes::from_sources(
                root.path(),
                &items,
                &PackOverlay::NONE,
                vec![(
                    "fixture:local".to_owned(),
                    json!({
                        "type":"minecraft:smelting", "ingredient": aspect,
                        "result":{"id":"minecraft:gold_ingot"}
                    }),
                )],
                &client_load,
            )
            .unwrap();
            let kind = "minecraft:crafting";
            let id = "fixture:server";
            let mut recipient = PlayerRecipeSync::default();
            for (candidate, not_candidate, output_count) in [
                ("minecraft:emerald", "minecraft:lapis_lazuli", 2),
                ("minecraft:lapis_lazuli", "minecraft:emerald", 3),
            ] {
                let server = Recipes::from_sources(root.path(), &items, &PackOverlay::NONE, vec![(
                    id.to_owned(), json!({
                        "type":"minecraft:crafting_shapeless",
                        "category":"equipment", "group":"from_sync",
                        "ingredients":[{"type":"neoforge:intersection", "children":[aspect, candidate]}],
                        "result":{"id":"minecraft:iron_sword", "count": output_count,
                            "components":{"minecraft:damage":7}},
                    }),
                )], &server_load).unwrap();
                let types = [kind.to_owned()];
                let sources = server.recipe_content(&types);
                recipient.replace(true, types.to_vec(), &sources);
                let parts: Vec<_> = recipient
                    .payloads
                    .iter()
                    .map(|body| ContentPart::decode(body).unwrap())
                    .collect();
                assert_eq!(
                    parts.last().unwrap().last.as_deref(),
                    Some(types.as_slice())
                );
                let received = parts.into_iter().flat_map(|part| part.recipes).collect();
                let (client, index) = local.with_recipe_content(&items, received, &types).unwrap();
                assert_eq!(index.first_display(id), Some(1));
                assert_eq!(
                    client.meta[id],
                    ("equipment".to_owned(), "from_sync".to_owned())
                );
                let server_context = server_load.loop_context(LoopSide::Server, []);
                let client_context = client_load.loop_context(LoopSide::Client, []);
                let grid = |item, count| [Some(ItemStack::new(item, count))];
                assert!(
                    server
                        .craft(&server_context, &grid(candidate, 1), 1)
                        .is_none()
                );
                assert_eq!(
                    server
                        .craft(&server_context, &grid(candidate, 3), 1)
                        .unwrap()
                        .count,
                    output_count
                );
                let made = client
                    .craft(&client_context, &grid(candidate, 1), 1)
                    .unwrap();
                assert_eq!(
                    (&*made.item, made.count, made.damage),
                    ("minecraft:iron_sword", output_count, 7)
                );
                assert!(
                    client
                        .craft(&client_context, &grid(not_candidate, 1), 1)
                        .is_none()
                );
                assert!(
                    client
                        .cook(
                            &client_context,
                            "minecraft:smelting",
                            &ItemStack::new("minecraft:emerald", 1)
                        )
                        .is_none()
                );
                assert_eq!(
                    &*client
                        .cook(
                            &client_context,
                            "minecraft:smelting",
                            &ItemStack::new("minecraft:emerald", 3)
                        )
                        .unwrap()
                        .result
                        .item,
                    "minecraft:gold_ingot"
                );
                assert_eq!(
                    client.by_type["minecraft:crafting"][0].json["ingredients"],
                    json!([[candidate]])
                );
            }
            assert_eq!(
                *calls.load.lock().unwrap(),
                [
                    (DataLoadOrigin::ClientPreload, "plain".into()),
                    (DataLoadOrigin::ServerDataLoad, "plain".into()),
                    (DataLoadOrigin::ServerDataLoad, "plain".into()),
                ]
            );
        });
    }

    #[test]
    fn content_decode_bounds_before_deserializing() {
        let mut body = br#"{"recipes":[],"last":null}"#.to_vec();
        body.resize(1_047_552, b' ');
        assert_eq!(ContentPart::decode(&body).unwrap().recipes, vec![]);
        body.push(b' ');
        assert!(ContentPart::decode(&body).unwrap_err().contains("exceeds"));
    }

    #[test]
    fn content_frames_the_selected_recipes_and_ends_with_their_types() {
        mod_platform::testing::with_recipe_types(&["mymod:crusher"], || {
            let sources = vec![
                ("furnace".to_owned(), json!({"type": "minecraft:smelting"})),
                ("mymod:crush".to_owned(), json!({"type": "mymod:crusher"})),
            ];
            assert!(encode_content(&[], &sources, false).is_empty());
            // After a sync that sent recipes, asking for none sends an empty last part.
            let reset = encode_content(&[], &sources, true);
            assert_eq!(reset.len(), 1);
            let part: ContentPart = serde_json::from_slice(&reset[0]).unwrap();
            assert!(part.recipes.is_empty());
            assert_eq!(part.last.as_deref(), Some(&[][..]));
            let types = vec![
                "minecraft:smelting".to_owned(),
                "mymod:crusher".to_owned(),
                "x:none".to_owned(),
            ];
            let parts = encode_content(&types, &sources, false);
            assert_eq!(parts.len(), 1);
            let part: ContentPart = serde_json::from_slice(&parts[0]).unwrap();
            let ids: Vec<&str> = part.recipes.iter().map(|(id, _)| id.as_str()).collect();
            assert_eq!(ids, ["minecraft:furnace", "mymod:crush"]);
            assert_eq!(part.last.as_deref(), Some(types.as_slice()));
        });
    }

    #[test]
    fn content_splits_under_the_payload_cap() {
        mod_platform::testing::with_recipe_types(&["m:t"], || {
            let big = "x".repeat(300_000);
            let sources: Vec<_> = (0..10)
                .map(|i| (format!("m:r{i}"), json!({"type": "m:t", "pad": big})))
                .collect();
            let parts = encode_content(&["m:t".to_owned()], &sources, false);
            assert!(parts.len() >= 3);
            let mut n = 0;
            for (i, p) in parts.iter().enumerate() {
                assert!(p.len() <= 1_048_576);
                let part: ContentPart = serde_json::from_slice(p).unwrap();
                assert_eq!(part.last.is_some(), i + 1 == parts.len());
                n += part.recipes.len();
            }
            assert_eq!(n, 10);
        });
    }

    #[test]
    fn a_long_type_list_keeps_the_last_part_under_the_cap() {
        mod_platform::testing::with_recipe_types(&["mod0:type_with_a_long_name"], || {
            let types: Vec<String> = (0..5000)
                .map(|i| format!("mod{i}:type_with_a_long_name"))
                .collect();
            let big = "x".repeat(1_000_000);
            let sources = vec![(
                "m:r".to_owned(),
                json!({"type": "mod0:type_with_a_long_name", "pad": big}),
            )];
            let parts = encode_content(&types, &sources, false);
            assert_eq!(parts.len(), 2);
            assert!(parts.iter().all(|p| p.len() <= 1_048_576));
            let last: ContentPart = serde_json::from_slice(&parts[1]).unwrap();
            assert!(last.recipes.is_empty());
            assert_eq!(last.last.map(|t| t.len()), Some(5000));
        });
    }
}

#[cfg(test)]
mod per_player_tests {
    use super::*;

    #[test]
    fn per_player_content_keeps_grinding_private_and_resets_only_its_owner() {
        mod_platform::testing::with_recipe_types(&["testmod:grinding"], || {
            let sources = vec![(
                "testmod:ore".into(),
                serde_json::json!({"type":"testmod:grinding", "result":"minecraft:iron_ingot"}),
            )];
            let mut a = PlayerRecipeSync::default();
            let mut b = PlayerRecipeSync::default();
            let mut vanilla = PlayerRecipeSync::default();
            a.replace(true, vec!["testmod:grinding".into()], &sources);
            b.replace(true, vec![], &sources);
            vanilla.replace(false, vec!["testmod:grinding".into()], &sources);
            let part = ContentPart::decode(&a.payloads[0]).unwrap();
            assert_eq!(part.recipes, sources);
            assert_eq!(part.last, Some(vec!["testmod:grinding".into()]));
            assert!(b.payloads.is_empty());
            assert!(vanilla.payloads.is_empty());
            a.replace(true, vec![], &sources);
            b.replace(true, vec!["testmod:grinding".into()], &sources);
            vanilla.replace(false, vec!["testmod:grinding".into()], &sources);
            assert_eq!(
                ContentPart::decode(&a.payloads[0]).unwrap().last,
                Some(vec![])
            );
            assert_eq!(
                ContentPart::decode(&b.payloads[0]).unwrap().recipes,
                sources
            );
            assert!(vanilla.payloads.is_empty());
        });
    }
}

#[cfg(test)]
#[test]
fn empty_ingredient_network_consumers_exclude_whole_unplaceable_recipes() {
    use serde_json::json;
    use sunrise_protocol::{
        buf::{Codec, Reader},
        packets::recipes::UpdateRecipes,
        registry::Ids,
    };
    struct EmptyPlacement;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for EmptyPlacement {
        fn ingredient(
            &self,
            _: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            let kind = event.kind;
            (kind == "fixture:predicate")
                .then(|| mod_api::IngredientDefinition::new(vec![], |_, _| true))
        }
    }

    let mut catalog = crate::items::Items::for_tests(Default::default());
    catalog
        .item_tag_order
        .insert("fixture:empty".into(), vec![]);
    let mut sync = SyncData::empty();
    sync.builtin.registries.insert(
        "minecraft:item".into(),
        Ids::from_names(vec![
            "minecraft:air".into(),
            "minecraft:stone".into(),
            "minecraft:stick".into(),
        ]),
    );
    for (source, retained) in [
        (json!([]), false),
        (json!("#fixture:missing"), false),
        (json!("#fixture:empty"), true),
        (
            json!({"type":"fixture:predicate", "item":"minecraft:stone"}),
            true,
        ),
    ] {
        let mut files: Vec<_> = [
            "minecraft:stonecutting",
            "minecraft:smelting",
            "minecraft:smithing_transform",
        ]
        .into_iter()
        .map(|kind| {
            (
                kind.to_owned(),
                json!({"type":kind, "ingredient":source,
                "base":"minecraft:stone", "addition":source, "result":{"id":"minecraft:stick"}}),
            )
        })
        .collect();
        files.push((
            "fixture:placeable".into(),
            json!({
                "type":"minecraft:stonecutting", "ingredient":"minecraft:stone",
                "result":{"id":"minecraft:stick"}
            }),
        ));
        let sources = mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("fixture", Box::new(EmptyPlacement))].into(),
            || {
                crate::crafting::ResolvedSources::resolve_collecting(
                    files,
                    &catalog,
                    &crate::test_resources::server_load(),
                )
                .unwrap()
            },
        );
        assert_eq!(sources.is_complete(), retained);
        if retained {
            for recipe in &sources.recipes {
                if recipe.name != "fixture:placeable" {
                    assert!(!recipe.has_placement_candidates());
                    assert!(
                        !book_projections(&sources)
                            .unwrap()
                            .displays
                            .contains(&crate::crafting::qualified(&recipe.name))
                    );
                }
            }
        }
        let (bytes, buttons) = update_recipes(&sync, &sources);
        let packet = UpdateRecipes::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(
            packet.stonecutter,
            [(
                HolderSet::Items(vec![1]),
                SlotDisplay::ItemStack(sunrise_protocol::item::Stack::new(2, 1)),
            )]
        );
        assert_eq!(
            buttons.recipe("minecraft:stone", 0),
            Some("fixture:placeable")
        );
        assert_eq!(buttons.recipe("minecraft:stone", 1), None);
        assert!(packet.properties.iter().all(|(_, ids)| ids.is_empty()));
    }
}
