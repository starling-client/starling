//! Call-scoped read access to the parameters of a loot evaluation.

use crate::{
    ComponentId, ExplorationMap, Identifier, ItemKey, ItemStackView, LootStack, Tag, Text,
    TextFormError,
};
use serde_json::Value;
use std::fmt;

// hidden: the engine lends its resident parameter rows; mods read LootParamsView.
// exhaustive: hidden engine boundary struct, constructed only by the evaluator.
#[doc(hidden)]
#[derive(Clone, Copy, Default)]
pub struct LootParamsParts<'a> {
    pub queried_table_id: Option<&'a str>,
    pub queried_stack: Option<QueriedStack<'a>>,
    pub entity: Option<&'a dyn LootEntityAccess>,
    pub last_damage_player: Option<&'a dyn LootEntityAccess>,
    pub interacting_entity: Option<&'a dyn LootEntityAccess>,
    pub damage: Option<&'a dyn LootDamageAccess>,
    pub tool: Option<&'a dyn LootToolAccess>,
    pub block: Option<LootBlockView<'a>>,
    pub biome: Option<&'a str>,
    pub origin: Option<[f64; 3]>,
    pub block_entity: Option<&'a Tag>,
    pub level: Option<&'a dyn LootLevelAccess>,
    pub luck: f32,
}

type NameProjection = fn(&Value) -> Result<Text, TextFormError>;
type NameEncoding = fn(&Text) -> Result<Value, TextFormError>;
type QueriedStack<'a> = (
    &'a dyn LootStackAccess,
    fn(&dyn LootStackAccess) -> Option<ItemStackView<'_>>,
);

/// The parameters supplied to a single loot evaluation. Missing parameters are
/// absent rather than synthesized. Reading facts does not advance the roll's random source.
// no wire: lends the evaluator's call-scoped rows.
// exhaustive: constructed only at the engine integration boundary.
pub struct LootParamsView<'a>(LootParamsParts<'a>, NameProjection);

impl<'a> LootParamsView<'a> {
    // hidden: the evaluator constructs this loan from its existing parameter rows.
    #[doc(hidden)]
    pub fn new(parts: LootParamsParts<'a>, project_name: NameProjection) -> Self {
        Self(parts, project_name)
    }

    /// The original queried table id. The first id wins, nested rolls never set it,
    /// and an unset id reads as `UNKNOWN_LOOT_TABLE` until the root completes.
    pub fn queried_table_id(&self) -> &'a str {
        self.0
            .queried_table_id
            .unwrap_or(sunrise_core::UNKNOWN_LOOT_TABLE)
    }

    /// The nonempty inventory stack being queried, if supplied.
    pub fn queried_stack(&self) -> Option<ItemStackView<'a>> {
        let (stack, view) = self.0.queried_stack?;
        view(stack)
    }

    /// The entity named by `THIS_ENTITY`.
    pub fn entity(&self) -> Option<LootEntityView<'a>> {
        Some(LootEntityView(self.0.entity?, self.1))
    }

    /// The player named by `LAST_DAMAGE_PLAYER`.
    pub fn last_damage_player(&self) -> Option<LootEntityView<'a>> {
        Some(LootEntityView(self.0.last_damage_player?, self.1))
    }

    /// The entity named by `INTERACTING_ENTITY` in a block-interaction roll.
    pub fn interacting_entity(&self) -> Option<LootEntityView<'a>> {
        Some(LootEntityView(self.0.interacting_entity?, self.1))
    }

    /// The damage source supplied to this roll.
    pub fn damage(&self) -> Option<LootDamageView<'a>> {
        Some(LootDamageView(self.0.damage?, self.1))
    }

    /// The tool supplied to this roll.
    pub fn tool(&self) -> Option<LootToolView<'a>> {
        self.0.tool.map(LootToolView)
    }

    /// The `BLOCK_STATE` supplied to a block or block-interaction roll.
    pub fn block(&self) -> Option<LootBlockView<'a>> {
        self.0.block
    }

    /// Biome identifier, when supplied.
    pub fn biome(&self) -> Option<&'a str> {
        self.0.biome
    }

    /// The world-space origin supplied to the roll.
    pub fn origin(&self) -> Option<[f64; 3]> {
        self.0.origin
    }

    /// The block entity's saved tag, with metadata, when supplied.
    pub fn block_entity(&self) -> Option<&'a Tag> {
        self.0.block_entity
    }

    /// The level in which the roll takes place, when supplied.
    pub fn level(&self) -> Option<LootLevelView<'a>> {
        self.0.level.map(LootLevelView)
    }

    /// Luck applied to this roll.
    pub fn luck(&self) -> f32 {
        self.0.luck
    }
}

// hidden: the engine lends its resident properties, without constructing an owned state.
#[doc(hidden)]
pub trait LootBlockProperties {
    fn property(&self, name: &str) -> Option<&str>;
}

/// The identifier and state properties of the block supplied to a loot roll.
// no wire: borrows the evaluated block's state.
// exhaustive: constructed only at the engine integration boundary.
#[derive(Clone, Copy)]
pub struct LootBlockView<'a>(&'a str, &'a dyn LootBlockProperties);

impl<'a> LootBlockView<'a> {
    // hidden: engine-side constructor for loot roll block views.
    #[doc(hidden)]
    pub fn new(id: &'a str, properties: &'a dyn LootBlockProperties) -> Self {
        Self(id, properties)
    }

    /// The block's resource identifier, independent of its properties.
    pub fn id(&self) -> &'a str {
        self.0
    }

    /// Read one state property, or `None` if the state has no such property.
    pub fn property(&self, name: &str) -> Option<&'a str> {
        self.1.property(name)
    }
}

// hidden: an evaluator reads its existing TOOL parameter through this adapter.
#[doc(hidden)]
pub trait LootToolAccess {
    fn item_id(&self) -> &str;
    fn enchantment_at(&self, index: usize) -> Option<(&str, i32)>;
}

/// A borrowed tool parameter. Unlike an output candidate, this view exposes only
/// what both tool representations supply: item identity and enchantments.
// no wire: lends the active roll's tool.
// exhaustive: constructed only by LootParamsView.
pub struct LootToolView<'a>(&'a dyn LootToolAccess);

impl<'a> LootToolView<'a> {
    /// The tool's item identifier.
    pub fn item_id(&self) -> &'a str {
        self.0.item_id()
    }

    /// The tool's item key, if its identifier is registered.
    pub fn item(&self) -> Option<ItemKey> {
        crate::__private::item_key(self.item_id())
    }

    /// Tool enchantment identifiers and levels in stored order.
    pub fn enchantments(&self) -> impl Iterator<Item = (&'a str, i32)> + use<'a> {
        let tool = self.0;
        (0..).map_while(move |index| tool.enchantment_at(index))
    }
}

// hidden: lends entity rows; no entity handle or engine type crosses the mod boundary.
#[doc(hidden)]
pub trait LootEntityAccess {
    fn kind(&self) -> &str;
    fn living(&self) -> bool;
    fn on_fire(&self) -> bool;
    fn baby(&self) -> bool;
    fn vehicle(&self) -> Option<&dyn LootEntityAccess>;
    fn cube_size(&self) -> Option<i32>;
    fn sheep(&self) -> Option<(&str, bool)>;
    fn captain(&self) -> Option<bool>;
    fn frog_variant(&self) -> Option<&str>;
    fn chicken_variant(&self) -> Option<&str>;
    fn powered(&self) -> bool;
    fn equipment(&self, slot: &str) -> Option<&dyn LootStackAccess>;
    fn equipment_at(&self, index: usize) -> Option<(&str, &dyn LootStackAccess)>;
}

/// Entity predicate facts supplied to a loot roll, not a live entity handle.
// no wire: lends the evaluator's entity row and its relationships.
// exhaustive: constructed only from active loot parameters.
pub struct LootEntityView<'a>(&'a dyn LootEntityAccess, NameProjection);

impl<'a> LootEntityView<'a> {
    /// Entity type identifier.
    pub fn kind(&self) -> &'a str {
        self.0.kind()
    }

    /// Whether this entity is living.
    pub fn living(&self) -> bool {
        self.0.living()
    }

    /// Whether this entity is on fire.
    pub fn on_fire(&self) -> bool {
        self.0.on_fire()
    }

    /// Whether this entity is a baby.
    pub fn baby(&self) -> bool {
        self.0.baby()
    }

    /// The entity that this entity is riding, if any.
    pub fn vehicle(&self) -> Option<Self> {
        Some(Self(self.0.vehicle()?, self.1))
    }

    /// A cube mob's size, when applicable.
    pub fn cube_size(&self) -> Option<i32> {
        self.0.cube_size()
    }

    /// A sheep's colour and whether it is sheared, when applicable.
    pub fn sheep(&self) -> Option<(&'a str, bool)> {
        self.0.sheep()
    }

    /// Whether this raider is a captain, or `None` if not a raider.
    pub fn captain(&self) -> Option<bool> {
        self.0.captain()
    }

    /// Frog variant identifier from the world's `minecraft:frog_variant`
    /// registry, when applicable (including variants added by data packs).
    pub fn frog_variant(&self) -> Option<&'a str> {
        self.0.frog_variant()
    }

    /// Chicken variant identifier from the world's `minecraft:chicken_variant`
    /// registry, when applicable.
    pub fn chicken_variant(&self) -> Option<&'a str> {
        self.0.chicken_variant()
    }

    /// Whether this entity is a charged creeper.
    pub fn powered(&self) -> bool {
        self.0.powered()
    }

    /// The raw candidate in the named slot (for example, `mainhand`), if supplied.
    pub fn equipment(&self, slot: &str) -> Option<LootStackView<'a>> {
        Some(LootStackView(self.0.equipment(slot)?, self.1))
    }

    /// Equipment slot names and raw candidates in the source row's order.
    pub fn equipment_iter(&self) -> impl Iterator<Item = (&'a str, LootStackView<'a>)> + use<'a> {
        let Self(entity, project_name) = *self;
        (0..).map_while(move |index| {
            let (slot, stack) = entity.equipment_at(index)?;
            Some((slot, LootStackView(stack, project_name)))
        })
    }
}

// hidden: lends damage source and direct entity without projecting either row.
#[doc(hidden)]
pub trait LootDamageAccess {
    fn kind(&self) -> &str;
    fn source(&self) -> Option<&dyn LootEntityAccess>;
    fn direct(&self) -> Option<&dyn LootEntityAccess>;
}

/// Borrowed damage-source facts.
// no wire: lends the evaluator's damage row and its entity parameters.
// exhaustive: constructed only from active loot parameters.
pub struct LootDamageView<'a>(&'a dyn LootDamageAccess, NameProjection);

impl<'a> LootDamageView<'a> {
    /// Damage type identifier.
    pub fn kind(&self) -> &'a str {
        self.0.kind()
    }

    /// The source of the damage, such as the shooter of a projectile.
    pub fn source(&self) -> Option<LootEntityView<'a>> {
        Some(LootEntityView(self.0.source()?, self.1))
    }

    /// The entity that inflicted the damage directly, such as the projectile.
    pub fn direct(&self) -> Option<LootEntityView<'a>> {
        Some(LootEntityView(self.0.direct()?, self.1))
    }
}

// hidden: lends the evaluator's level row.
#[doc(hidden)]
pub trait LootLevelAccess {
    fn dimension(&self) -> &str;
    fn game_time(&self) -> i64;
    fn day_time(&self) -> i64;
    fn raining(&self) -> bool;
    fn thundering(&self) -> bool;
}

/// Borrowed dimension, time and weather facts for a loot roll.
// no wire: lends the evaluator's level row.
// exhaustive: constructed only from active loot parameters.
pub struct LootLevelView<'a>(&'a dyn LootLevelAccess);

impl<'a> LootLevelView<'a> {
    /// The dimension identifier.
    pub fn dimension(&self) -> &'a str {
        self.0.dimension()
    }

    /// Elapsed game ticks.
    pub fn game_time(&self) -> i64 {
        self.0.game_time()
    }

    /// The level's day time, not reduced to the current day.
    pub fn day_time(&self) -> i64 {
        self.0.day_time()
    }

    /// Whether the level is raining.
    pub fn raining(&self) -> bool {
        self.0.raining()
    }

    /// Whether it is thundering.
    pub fn thundering(&self) -> bool {
        self.0.thundering()
    }
}

// hidden: lends an existing signed loot candidate, not a normalized inventory stack.
#[doc(hidden)]
pub trait LootStackAccess: LootToolAccess {
    fn count(&self) -> i32;
    fn damage(&self) -> i32;
    fn map(&self) -> bool;
    fn components(&self) -> &[(String, Option<Tag>)];
    fn enchantments(&self) -> &[(String, i32)];
    fn potion(&self) -> Option<&str>;
    fn stew_effects(&self) -> &[(String, i32)];
    fn ominous_amplifier(&self) -> Option<i32>;
    fn exploration(&self) -> Option<LootExplorationView<'_>>;
    fn names(&self) -> &[(String, Value)];
}

/// A borrowed, signed loot candidate, not an occupied inventory slot.
// no wire: lends the existing candidate and its component patches.
// exhaustive: constructed only from active loot storage.
pub struct LootStackView<'a>(&'a dyn LootStackAccess, NameProjection);

impl<'a> LootStackView<'a> {
    // hidden: the evaluator lends a raw loot candidate without converting it to an owned value.
    #[doc(hidden)]
    pub fn new(stack: &'a dyn LootStackAccess, project_name: NameProjection) -> Self {
        Self(stack, project_name)
    }

    /// Candidate item identifier.
    pub fn item_id(&self) -> &'a str {
        self.0.item_id()
    }

    /// Candidate item key, if registered.
    pub fn item(&self) -> Option<ItemKey> {
        crate::__private::item_key(self.item_id())
    }

    /// Signed intermediate count. Non-positive counts need not become inventory stacks.
    pub fn count(&self) -> i32 {
        self.0.count()
    }

    /// Signed intermediate damage value.
    pub fn damage(&self) -> i32 {
        self.0.damage()
    }

    /// Whether to create a map id after the roll.
    pub fn map(&self) -> bool {
        self.0.map()
    }

    /// Component edits in stored order, including repeated keys. `None` removes the component.
    pub fn components(
        &self,
    ) -> impl ExactSizeIterator<Item = (&'a str, Option<&'a Tag>)> + use<'a> {
        self.0
            .components()
            .iter()
            .map(|(id, tag)| (id.as_str(), tag.as_ref()))
    }

    /// Enchantment identifiers and levels in stored order.
    pub fn enchantments(&self) -> impl ExactSizeIterator<Item = (&'a str, i32)> + use<'a> {
        self.0
            .enchantments()
            .iter()
            .map(|(id, level)| (id.as_str(), *level))
    }

    /// Potion identifier, if supplied.
    pub fn potion(&self) -> Option<&'a str> {
        self.0.potion()
    }

    /// Suspicious stew effect identifiers and durations in ticks, in stored order.
    pub fn stew_effects(&self) -> impl ExactSizeIterator<Item = (&'a str, i32)> + use<'a> {
        self.0
            .stew_effects()
            .iter()
            .map(|(id, ticks)| (id.as_str(), *ticks))
    }

    /// Ominous bottle amplifier, if supplied.
    pub fn ominous_amplifier(&self) -> Option<i32> {
        self.0.ominous_amplifier()
    }

    /// Deferred exploration-map request, if supplied.
    pub fn exploration(&self) -> Option<LootExplorationView<'a>> {
        self.0.exploration()
    }

    /// Number of custom or item name entries stored by loot functions.
    pub fn name_count(&self) -> usize {
        self.0.names().len()
    }

    /// Read a name at its exact index. Projects only that name's text tree;
    /// a projection error is distinct from an absent entry.
    pub fn name(&self, index: usize) -> Option<Result<(&'a str, Text), TextFormError>> {
        let (id, raw) = self.0.names().get(index)?;
        Some((self.1)(raw).map(|text| (id.as_str(), text)))
    }

    /// Explicitly copy this candidate into an owned [`LootStack`].
    /// Fails if the candidate's item is unregistered or any stored name cannot be projected.
    pub fn to_owned(&self) -> Result<LootStack, LootStackEditError> {
        let item = self.item().ok_or(LootStackEditError::UnknownItem)?;
        let mut copy = LootStack::new(item, self.count());
        copy.damage = self.damage();
        copy.enchantments = self.0.enchantments().to_vec();
        copy.potion = self.0.potion().map(str::to_owned);
        copy.map = self.map();
        copy.exploration = self.exploration().map(|map| {
            Box::new(ExplorationMap::new(
                map.destination().to_owned(),
                map.decoration().to_owned(),
                map.zoom(),
                map.search_radius(),
                map.skip_existing_chunks(),
            ))
        });
        copy.ominous_amplifier = self.ominous_amplifier();
        copy.stew_effects = self.0.stew_effects().to_vec();
        copy.names = (0..self.name_count())
            .map(|index| {
                self.name(index)
                    .expect("index within name_count")
                    .map(|(id, text)| (id.to_owned(), text))
                    .map_err(LootStackEditError::InvalidName)
            })
            .collect::<Result<_, _>>()?;
        copy.components = self.0.components().to_vec();
        Ok(copy)
    }
}

/// Failure reported when an individual [`LootStackEditor`] write or explicit copy is refused.
/// Earlier valid edits on the same editor remain in place; the refused field is unchanged.
// no wire: returned locally by scoped LootStackEditor writes and explicit copies.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LootStackEditError {
    /// The supplied item key is not registered, or the candidate's current item is unregistered.
    UnknownItem,
    /// A component patch identifier or NBT value failed validation (for example, malformed
    /// identifier, `Tag::End`, or container nesting deeper than vanilla's
    /// `NbtAccounter.MAX_STACK_DEPTH`, 512).
    InvalidPatch,
    /// An identifier (enchantment, potion, stew effect, exploration setting, or name target)
    /// is not a valid resource identifier.
    InvalidIdentifier,
    /// A name text tree failed text-boundary validation or projection.
    InvalidName(TextFormError),
}

impl fmt::Display for LootStackEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownItem => f.write_str("unknown loot candidate item key"),
            Self::InvalidPatch => f.write_str("invalid loot candidate component patch"),
            Self::InvalidIdentifier => f.write_str("invalid resource identifier"),
            Self::InvalidName(error) => write!(f, "invalid loot candidate name: {error}"),
        }
    }
}

impl std::error::Error for LootStackEditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidName(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LootStackEditError> for String {
    fn from(error: LootStackEditError) -> Self {
        error.to_string()
    }
}

// hidden: mutates the evaluator's existing signed loot candidate in place.
#[doc(hidden)]
pub trait LootStackEditAccess: LootStackAccess {
    /// Whether the component `id` takes `value` as a payload the candidate can apply: a
    /// component the evaluator models into typed fields refuses a shape it would drop.
    fn accepts_component(&self, _id: &str, _value: &Tag) -> bool {
        true
    }
    fn as_view(&self) -> &dyn LootStackAccess;
    fn set_item_id(&mut self, id: &str);
    fn set_count(&mut self, count: i32);
    fn set_damage(&mut self, damage: i32);
    fn set_map(&mut self, map: bool);
    fn set_ominous_amplifier(&mut self, amplifier: Option<i32>);
    fn set_potion_id(&mut self, potion: Option<String>);
    fn set_exploration(&mut self, exploration: Option<ExplorationMap>);
    fn push_enchantment(&mut self, id: String, level: i32);
    fn set_enchantment(&mut self, id: String, level: i32);
    fn set_enchantments(&mut self, enchantments: Vec<(String, i32)>);
    fn clear_enchantments(&mut self);
    fn push_stew_effect(&mut self, id: String, duration_ticks: i32);
    fn set_stew_effects(&mut self, effects: Vec<(String, i32)>);
    fn clear_stew_effects(&mut self);
    fn push_name_value(&mut self, id: String, value: Value);
    fn set_name_value(&mut self, id: String, value: Value);
    fn set_names_values(&mut self, names: Vec<(String, Value)>);
    fn clear_names(&mut self);
    fn push_component(&mut self, id: String, value: Option<Tag>);
    fn set_components(&mut self, components: Vec<(String, Option<Tag>)>);
    fn clear_components(&mut self);
    fn retain_components(&mut self, keep: &mut dyn FnMut(&str, Option<&Tag>) -> bool);
}

fn validate_identifier(id: &str) -> Result<(), LootStackEditError> {
    Identifier::parse(id)
        .ok()
        .filter(|parsed| !parsed.path().is_empty())
        .map(|_| ())
        .ok_or(LootStackEditError::InvalidIdentifier)
}

fn validate_component_id(id: &str) -> Result<(), LootStackEditError> {
    ComponentId::new(id)
        .map(|_| ())
        .ok_or(LootStackEditError::InvalidPatch)
}

fn validate_component_patch(
    accepts: &dyn Fn(&str, &Tag) -> bool,
    id: &str,
    value: Option<&Tag>,
) -> Result<(), LootStackEditError> {
    let base = id.strip_prefix('!').unwrap_or(id);
    validate_component_id(base)?;
    if let Some(tag) = value
        && (contains_end(tag)
            || !sunrise_core::nbt::within_write_depth(
                tag,
                sunrise_core::nbt::Limits::default().max_depth,
            )
            || (id == base && !accepts(base, tag)))
    {
        return Err(LootStackEditError::InvalidPatch);
    }
    Ok(())
}

/// Whether a `Tag::End` appears at the root or inside any list or compound: it has no
/// encoding there.
fn contains_end(root: &Tag) -> bool {
    let mut work = vec![root];
    while let Some(tag) = work.pop() {
        match tag {
            Tag::End => return true,
            Tag::List(values) => work.extend(values),
            Tag::Compound(values) => work.extend(values.values()),
            _ => {}
        }
    }
    false
}

struct ValidatedLootStack {
    item_id: &'static str,
    count: i32,
    damage: i32,
    enchantments: Vec<(String, i32)>,
    potion: Option<String>,
    map: bool,
    exploration: Option<ExplorationMap>,
    ominous_amplifier: Option<i32>,
    stew_effects: Vec<(String, i32)>,
    names: Vec<(String, Value)>,
    components: Vec<(String, Option<Tag>)>,
}

fn validate_loot_stack(
    stack: LootStack,
    encode_name: NameEncoding,
    accepts: &dyn Fn(&str, &Tag) -> bool,
) -> Result<ValidatedLootStack, LootStackEditError> {
    let item_id =
        crate::__private::item_key_id(stack.item).ok_or(LootStackEditError::UnknownItem)?;
    for (id, _) in &stack.enchantments {
        validate_identifier(id)?;
    }
    if let Some(potion) = &stack.potion {
        validate_identifier(potion)?;
    }
    if let Some(map) = &stack.exploration {
        validate_identifier(&map.destination)?;
        validate_identifier(&map.decoration)?;
    }
    for (effect, _) in &stack.stew_effects {
        validate_identifier(effect)?;
    }
    let mut names = Vec::with_capacity(stack.names.len());
    for (id, text) in &stack.names {
        validate_identifier(id)?;
        let encoded = encode_name(text).map_err(LootStackEditError::InvalidName)?;
        names.push((id.clone(), encoded));
    }
    for (id, value) in &stack.components {
        validate_component_patch(accepts, id, value.as_ref())?;
    }
    Ok(ValidatedLootStack {
        item_id,
        count: stack.count,
        damage: stack.damage,
        enchantments: stack.enchantments,
        potion: stack.potion,
        map: stack.map,
        exploration: stack.exploration.map(|map| *map),
        ominous_amplifier: stack.ominous_amplifier,
        stew_effects: stack.stew_effects,
        names,
        components: stack.components,
    })
}

fn apply_validated_loot_stack(target: &mut dyn LootStackEditAccess, validated: ValidatedLootStack) {
    target.set_item_id(validated.item_id);
    target.set_count(validated.count);
    target.set_damage(validated.damage);
    target.set_enchantments(validated.enchantments);
    target.set_potion_id(validated.potion);
    target.set_map(validated.map);
    target.set_exploration(validated.exploration);
    target.set_ominous_amplifier(validated.ominous_amplifier);
    target.set_stew_effects(validated.stew_effects);
    target.set_names_values(validated.names);
    target.set_components(validated.components);
}

/// Scoped in-place editor for the active loot function candidate.
///
/// Reads go through [`Self::view`], explicit whole-stack copies go through [`Self::to_owned`],
/// and writes validate their own inputs before touching the targeted field. Earlier valid edits
/// are not rolled back if a later write or the callback fails.
// no wire: borrows the evaluator's live candidate for one function invocation.
// exhaustive: constructed only at the engine integration boundary.
pub struct LootStackEditor<'a> {
    stack: &'a mut dyn LootStackEditAccess,
    project_name: NameProjection,
    encode_name: NameEncoding,
}

impl<'a> LootStackEditor<'a> {
    // hidden: the evaluator constructs this editor over its active candidate.
    #[doc(hidden)]
    pub fn new(
        stack: &'a mut dyn LootStackEditAccess,
        project_name: NameProjection,
        encode_name: NameEncoding,
    ) -> Self {
        Self {
            stack,
            project_name,
            encode_name,
        }
    }

    /// Borrow a read-only view of the candidate's current state, including earlier edits.
    pub fn view(&self) -> LootStackView<'_> {
        LootStackView(self.stack.as_view(), self.project_name)
    }

    /// Explicitly copy the candidate's current state into an owned [`LootStack`].
    pub fn to_owned(&self) -> Result<LootStack, LootStackEditError> {
        self.view().to_owned()
    }

    /// Validate and replace the entire candidate stack.
    /// If any field in `stack` is invalid, the current candidate is left unchanged.
    pub fn replace(&mut self, stack: LootStack) -> Result<(), LootStackEditError> {
        let validated = validate_loot_stack(stack, self.encode_name, &|id, tag| {
            self.stack.accepts_component(id, tag)
        })?;
        apply_validated_loot_stack(self.stack, validated);
        Ok(())
    }

    /// Replace the candidate's item key without disturbing other fields.
    pub fn set_item(&mut self, item: ItemKey) -> Result<(), LootStackEditError> {
        let id = crate::__private::item_key_id(item).ok_or(LootStackEditError::UnknownItem)?;
        self.stack.set_item_id(id);
        Ok(())
    }

    /// Set the signed intermediate count without clamping.
    pub fn set_count(&mut self, count: i32) {
        self.stack.set_count(count);
    }

    /// Set the signed intermediate damage value without clamping.
    pub fn set_damage(&mut self, damage: i32) {
        self.stack.set_damage(damage);
    }

    /// Set whether to create a map id after the roll.
    pub fn set_map(&mut self, map: bool) {
        self.stack.set_map(map);
    }

    /// Set or clear the signed ominous bottle amplifier without clamping.
    pub fn set_ominous_amplifier(&mut self, amplifier: Option<i32>) {
        self.stack.set_ominous_amplifier(amplifier);
    }

    /// Set or clear the potion identifier.
    pub fn set_potion(&mut self, potion: Option<&str>) -> Result<(), LootStackEditError> {
        let potion = match potion {
            Some(id) => {
                validate_identifier(id)?;
                Some(id.to_owned())
            }
            None => None,
        };
        self.stack.set_potion_id(potion);
        Ok(())
    }

    /// Set or clear the deferred exploration-map search settings.
    pub fn set_exploration(
        &mut self,
        exploration: Option<ExplorationMap>,
    ) -> Result<(), LootStackEditError> {
        if let Some(map) = &exploration {
            validate_identifier(&map.destination)?;
            validate_identifier(&map.decoration)?;
        }
        self.stack.set_exploration(exploration);
        Ok(())
    }

    /// Append an enchantment identifier and signed level in stored order.
    pub fn push_enchantment(&mut self, id: &str, level: i32) -> Result<(), LootStackEditError> {
        validate_identifier(id)?;
        self.stack.push_enchantment(id.to_owned(), level);
        Ok(())
    }

    /// Update the first matching enchantment entry or append `(id, level)` at the end.
    pub fn set_enchantment(&mut self, id: &str, level: i32) -> Result<(), LootStackEditError> {
        validate_identifier(id)?;
        self.stack.set_enchantment(id.to_owned(), level);
        Ok(())
    }

    /// Validate every enchantment identifier and replace the enchantment list in order.
    pub fn set_enchantments(
        &mut self,
        enchantments: impl IntoIterator<Item = (String, i32)>,
    ) -> Result<(), LootStackEditError> {
        let validated: Vec<_> = enchantments
            .into_iter()
            .map(|(id, level)| {
                validate_identifier(&id)?;
                Ok((id, level))
            })
            .collect::<Result<_, LootStackEditError>>()?;
        self.stack.set_enchantments(validated);
        Ok(())
    }

    /// Remove all enchantment entries from the candidate.
    pub fn clear_enchantments(&mut self) {
        self.stack.clear_enchantments();
    }

    /// Append a suspicious stew effect identifier and signed duration in ticks.
    pub fn push_stew_effect(
        &mut self,
        effect: &str,
        duration_ticks: i32,
    ) -> Result<(), LootStackEditError> {
        validate_identifier(effect)?;
        self.stack
            .push_stew_effect(effect.to_owned(), duration_ticks);
        Ok(())
    }

    /// Validate every effect identifier and replace the suspicious stew effects in order.
    pub fn set_stew_effects(
        &mut self,
        effects: impl IntoIterator<Item = (String, i32)>,
    ) -> Result<(), LootStackEditError> {
        let validated: Vec<_> = effects
            .into_iter()
            .map(|(effect, ticks)| {
                validate_identifier(&effect)?;
                Ok((effect, ticks))
            })
            .collect::<Result<_, LootStackEditError>>()?;
        self.stack.set_stew_effects(validated);
        Ok(())
    }

    /// Remove all suspicious stew effect entries from the candidate.
    pub fn clear_stew_effects(&mut self) {
        self.stack.clear_stew_effects();
    }

    /// Validate and append a name entry (`minecraft:custom_name` or `minecraft:item_name`).
    pub fn push_name(&mut self, id: &str, text: &Text) -> Result<(), LootStackEditError> {
        validate_identifier(id)?;
        let encoded = (self.encode_name)(text).map_err(LootStackEditError::InvalidName)?;
        self.stack.push_name_value(id.to_owned(), encoded);
        Ok(())
    }

    /// Validate and set a name entry, replacing any prior entry with the same identifier.
    pub fn set_name(&mut self, id: &str, text: &Text) -> Result<(), LootStackEditError> {
        validate_identifier(id)?;
        let encoded = (self.encode_name)(text).map_err(LootStackEditError::InvalidName)?;
        self.stack.set_name_value(id.to_owned(), encoded);
        Ok(())
    }

    /// Validate every name identifier and text tree, then replace the names list in order.
    pub fn set_names(
        &mut self,
        names: impl IntoIterator<Item = (String, Text)>,
    ) -> Result<(), LootStackEditError> {
        let validated: Vec<_> = names
            .into_iter()
            .map(|(id, text)| {
                validate_identifier(&id)?;
                let encoded = (self.encode_name)(&text).map_err(LootStackEditError::InvalidName)?;
                Ok((id, encoded))
            })
            .collect::<Result<_, LootStackEditError>>()?;
        self.stack.set_names_values(validated);
        Ok(())
    }

    /// Remove all name entries from the candidate.
    pub fn clear_names(&mut self) {
        self.stack.clear_names();
    }

    /// Validate and append a component value patch in stored order, without reordering duplicates.
    pub fn push_component(&mut self, id: &str, value: Tag) -> Result<(), LootStackEditError> {
        validate_component_patch(
            &|id, tag| self.stack.accepts_component(id, tag),
            id,
            Some(&value),
        )?;
        self.stack.push_component(id.to_owned(), Some(value));
        Ok(())
    }

    /// Validate and append an ordered component removal (`None`) without reordering earlier patches.
    pub fn remove_component(&mut self, id: &str) -> Result<(), LootStackEditError> {
        validate_component_id(id)?;
        self.stack.push_component(id.to_owned(), None);
        Ok(())
    }

    /// Validate and append an ordered component value or removal patch.
    pub fn push_component_patch(
        &mut self,
        id: &str,
        value: Option<Tag>,
    ) -> Result<(), LootStackEditError> {
        validate_component_patch(
            &|id, tag| self.stack.accepts_component(id, tag),
            id,
            value.as_ref(),
        )?;
        self.stack.push_component(id.to_owned(), value);
        Ok(())
    }

    /// Validate every component patch entry and replace the ordered patch list.
    pub fn set_components(
        &mut self,
        components: impl IntoIterator<Item = (String, Option<Tag>)>,
    ) -> Result<(), LootStackEditError> {
        let validated: Vec<_> = components
            .into_iter()
            .map(|(id, value)| {
                validate_component_patch(
                    &|id, tag| self.stack.accepts_component(id, tag),
                    &id,
                    value.as_ref(),
                )?;
                Ok((id, value))
            })
            .collect::<Result<_, LootStackEditError>>()?;
        self.stack.set_components(validated);
        Ok(())
    }

    /// Retain only component patch entries matching `keep`, preserving stored order.
    pub fn retain_components(&mut self, mut keep: impl FnMut(&str, Option<&Tag>) -> bool) {
        self.stack.retain_components(&mut keep);
    }

    /// Remove all component patch entries from the candidate.
    pub fn clear_components(&mut self) {
        self.stack.clear_components();
    }
}

// hidden: lends the evaluator's active generated loot list without cloning it.
#[doc(hidden)]
pub trait LootListAccess: std::any::Any {
    /// As [`LootStackEditAccess::accepts_component`], for a stack not yet in the list.
    fn accepts_component(&self, _id: &str, _value: &Tag) -> bool {
        true
    }
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn get(&self, index: usize) -> Option<&dyn LootStackAccess>;
    fn get_mut(&mut self, index: usize) -> Option<&mut dyn LootStackEditAccess>;
    fn push_empty(&mut self, item_id: &str) -> &mut dyn LootStackEditAccess;
    fn insert_empty(&mut self, index: usize, item_id: &str)
    -> Option<&mut dyn LootStackEditAccess>;
    fn remove(&mut self, index: usize) -> bool;
    fn clear(&mut self);
    fn truncate(&mut self, len: usize);
    fn swap(&mut self, a: usize, b: usize) -> bool;
    fn reverse(&mut self);
    fn reorder(&mut self, from: usize, to: usize) -> bool;
}

/// Scoped in-place editor for the generated loot list passed to a global loot modifier.
///
/// Reading lends [`LootStackView`], per-entry edits lend [`LootStackEditor`], and explicit
/// insertions validate only the newly supplied [`LootStack`]. All index operations are
/// out-of-range-safe and never panic. Earlier valid edits remain in place if a later
/// operation or callback fails.
// no wire: borrows the evaluator's live generated loot list for one modifier pass.
// exhaustive: constructed only at the engine integration boundary.
pub struct LootList<'a> {
    list: &'a mut dyn LootListAccess,
    project_name: NameProjection,
    encode_name: NameEncoding,
}

impl<'a> LootList<'a> {
    // hidden: the evaluator constructs this list over its active loot output buffer.
    #[doc(hidden)]
    pub fn new(
        list: &'a mut dyn LootListAccess,
        project_name: NameProjection,
        encode_name: NameEncoding,
    ) -> Self {
        Self {
            list,
            project_name,
            encode_name,
        }
    }

    // hidden: lends the evaluator's backing list directly to `ModifierRoll::append_raw`.
    #[doc(hidden)]
    pub fn access_mut(&mut self) -> &mut dyn LootListAccess {
        self.list
    }

    /// Number of loot candidates currently in the list.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Whether the list currently contains no loot candidates.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Borrow a read-only view of the candidate at `index`, or `None` if out of bounds.
    pub fn get(&self, index: usize) -> Option<LootStackView<'_>> {
        Some(LootStackView(self.list.get(index)?, self.project_name))
    }

    /// Borrow an in-place editor for the candidate at `index`, or `None` if out of bounds.
    pub fn get_mut(&mut self, index: usize) -> Option<LootStackEditor<'_>> {
        Some(LootStackEditor::new(
            self.list.get_mut(index)?,
            self.project_name,
            self.encode_name,
        ))
    }

    /// Iterate over every candidate in order without copying or projecting names.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = LootStackView<'_>> + '_ {
        (0..self.list.len()).map(|index| {
            LootStackView(
                self.list.get(index).expect("index within LootList::len"),
                self.project_name,
            )
        })
    }

    /// Validate and append an explicitly constructed [`LootStack`] to the end of the list.
    /// If `stack` is invalid, the list is left unchanged.
    pub fn push(&mut self, stack: LootStack) -> Result<(), LootStackEditError> {
        let validated = validate_loot_stack(stack, self.encode_name, &|id, tag| {
            self.list.accepts_component(id, tag)
        })?;
        let slot = self.list.push_empty(validated.item_id);
        apply_validated_loot_stack(slot, validated);
        Ok(())
    }

    /// Validate and insert an explicitly constructed [`LootStack`] at `index`.
    /// Returns `Ok(false)` without panicking if `index > self.len()`, or
    /// `Err(LootStackEditError)` without mutating the list if `stack` is invalid.
    pub fn insert(&mut self, index: usize, stack: LootStack) -> Result<bool, LootStackEditError> {
        if index > self.list.len() {
            return Ok(false);
        }
        let validated = validate_loot_stack(stack, self.encode_name, &|id, tag| {
            self.list.accepts_component(id, tag)
        })?;
        let Some(slot) = self.list.insert_empty(index, validated.item_id) else {
            return Ok(false);
        };
        apply_validated_loot_stack(slot, validated);
        Ok(true)
    }

    /// Validate and replace the candidate at `index` with an explicitly constructed [`LootStack`].
    /// Returns `Ok(false)` without panicking if `index >= self.len()`, or
    /// `Err(LootStackEditError)` leaving the existing candidate intact if `stack` is invalid.
    pub fn replace(&mut self, index: usize, stack: LootStack) -> Result<bool, LootStackEditError> {
        let Some(mut editor) = self.get_mut(index) else {
            return Ok(false);
        };
        editor.replace(stack)?;
        Ok(true)
    }

    /// Remove the candidate at `index`, preserving the order of remaining entries.
    /// Returns `false` without panicking if `index >= self.len()`.
    pub fn remove(&mut self, index: usize) -> bool {
        self.list.remove(index)
    }

    /// Remove the last candidate in the list. Returns `false` if the list is empty.
    pub fn pop(&mut self) -> bool {
        let Some(last) = self.list.len().checked_sub(1) else {
            return false;
        };
        self.list.remove(last)
    }

    /// Remove every candidate from the list.
    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// Shorten the list to at most `len` candidates.
    pub fn truncate(&mut self, len: usize) {
        self.list.truncate(len);
    }

    /// Swap the candidates at `a` and `b` in place.
    /// Returns `false` without panicking if either index is out of bounds.
    pub fn swap(&mut self, a: usize, b: usize) -> bool {
        self.list.swap(a, b)
    }

    /// Reverse the order of candidates in place.
    pub fn reverse(&mut self) {
        self.list.reverse();
    }

    /// Move the candidate at `from` to `to` in place, shifting intervening entries in order.
    /// Returns `false` without panicking if `from` or `to` is out of bounds.
    pub fn reorder(&mut self, from: usize, to: usize) -> bool {
        self.list.reorder(from, to)
    }

    /// Retain only the candidates for which `keep` returns `true`, preserving order in place.
    pub fn retain(&mut self, mut keep: impl FnMut(LootStackView<'_>) -> bool) {
        let len = self.list.len();
        let mut write = 0;
        for read in 0..len {
            let keep_entry = {
                let view = LootStackView(
                    self.list.get(read).expect("index within LootList::len"),
                    self.project_name,
                );
                keep(view)
            };
            if keep_entry {
                if write != read {
                    self.list.swap(write, read);
                }
                write += 1;
            }
        }
        self.list.truncate(write);
    }
}

/// Borrowed settings for a deferred exploration-map search.
// no wire: lends the search's identifiers from a loot candidate.
// exhaustive: constructed only from active loot storage.
pub struct LootExplorationView<'a> {
    destination: &'a str,
    decoration: &'a str,
    zoom: u8,
    search_radius: i32,
    skip_existing_chunks: bool,
}

impl<'a> LootExplorationView<'a> {
    // hidden: borrow the evaluator's already decoded map settings.
    #[doc(hidden)]
    pub fn new(
        destination: &'a str,
        decoration: &'a str,
        zoom: u8,
        search_radius: i32,
        skip_existing_chunks: bool,
    ) -> Self {
        Self {
            destination,
            decoration,
            zoom,
            search_radius,
            skip_existing_chunks,
        }
    }

    /// Destination structure tag identifier, without `#`.
    pub fn destination(&self) -> &'a str {
        self.destination
    }

    /// Map decoration identifier.
    pub fn decoration(&self) -> &'a str {
        self.decoration
    }

    /// Map zoom.
    pub fn zoom(&self) -> u8 {
        self.zoom
    }

    /// Search radius in chunks.
    pub fn search_radius(&self) -> i32 {
        self.search_radius
    }

    /// Whether to skip previously discovered structures.
    pub fn skip_existing_chunks(&self) -> bool {
        self.skip_existing_chunks
    }
}
