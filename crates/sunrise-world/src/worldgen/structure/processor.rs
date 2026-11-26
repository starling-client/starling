//! Structure processors (`worldgen/processor_list`) and template placement.
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};
use sunrise_core::{
    position::BlockPos,
    random::{LegacyRandom, PositionalRandom, RandomSource, position_seed},
};
use {
    super::{BlockInfo, Mirror, Rotation, StructureResources, StructureTemplate},
    sunrise_core::bounding_box::BoundingBox,
};
use {
    crate::{
        fluid::{FluidKind, FluidState},
        heightmap::HeightmapType,
        registry::BlockStateId,
        worldgen::{
            DecodeError,
            block_feature::FeatureBlockBindings,
            feature::FeatureWorld,
            placement::{PlacementContext, PlacementQueryError},
            rule_test::RuleTest,
            value_provider::IntProvider,
        },
    },
    sunrise_core::nbt::{Compound, JavaString, Tag},
};

/// The draws processors make from StructurePlaceSettings' random.
pub trait ProcessorRandom {
    fn float(&mut self) -> f32;
    fn int(&mut self, bound: i32) -> i32;
    fn long(&mut self) -> i64;
    fn int_unbounded(&mut self) -> i32;
    fn double(&mut self) -> f64;
    fn boolean(&mut self) -> bool;
    fn gaussian(&mut self) -> f64;
}
impl<R: RandomSource> ProcessorRandom for R {
    fn float(&mut self) -> f32 {
        self.next_float()
    }
    fn int(&mut self, bound: i32) -> i32 {
        self.next_int_bound(bound).unwrap_or(0)
    }
    fn long(&mut self) -> i64 {
        self.next_long()
    }
    fn int_unbounded(&mut self) -> i32 {
        self.next_int()
    }
    fn double(&mut self) -> f64 {
        self.next_double()
    }
    fn boolean(&mut self) -> bool {
        self.next_boolean()
    }
    fn gaussian(&mut self) -> f64 {
        self.next_gaussian()
    }
}

#[derive(Clone, Debug)]
pub enum PosRule {
    Always,
    Linear {
        min_chance: f32,
        max_chance: f32,
        min_dist: i32,
        max_dist: i32,
        axis: Option<usize>,
    },
    Mod(Arc<dyn crate::worldgen::mod_rule_tests::ModPosRuleTest>),
}
#[derive(Clone, Debug)]
enum Modifier {
    Passthrough,
    Clear,
    AppendLoot(String),
    AppendStatic(Compound),
}
impl PosRule {
    /// A rule's `position_predicate` (always true when missing). Types are
    /// identifiers: `"always_true"` is `minecraft:always_true`, as vanilla
    /// reads it.
    pub fn decode(value: Option<&Value>) -> Result<Self, DecodeError> {
        Self::decode_inner(value, 0, &mut 65536)
    }
    pub(crate) fn decode_inner(
        value: Option<&Value>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let Some(p) = value else {
            return Ok(Self::Always);
        };
        if depth > 128 || *remaining == 0 {
            return Err(DecodeError::Field("position rule test depth or node count"));
        }
        *remaining -= 1;
        let kind = p["predicate_type"]
            .as_str()
            .ok_or(DecodeError::Field("predicate_type"))?;
        let canonical = super::super::tags::identifier(kind)?;
        Ok(match canonical.as_str() {
            "minecraft:always_true" => Self::Always,
            kind @ ("minecraft:linear_pos" | "minecraft:axis_aligned_linear_pos") => Self::Linear {
                min_chance: p.get("min_chance").and_then(Value::as_f64).unwrap_or(0.) as f32,
                max_chance: p.get("max_chance").and_then(Value::as_f64).unwrap_or(0.) as f32,
                min_dist: p.get("min_dist").and_then(Value::as_i64).unwrap_or(0) as i32,
                max_dist: p.get("max_dist").and_then(Value::as_i64).unwrap_or(0) as i32,
                axis: (kind == "minecraft:axis_aligned_linear_pos").then(|| {
                    match p.get("axis").and_then(Value::as_str) {
                        Some("x") => 0,
                        Some("z") => 2,
                        _ => 1,
                    }
                }),
            },
            other => {
                if let Some(ty) = crate::worldgen::mod_rule_tests::mod_pos_rule_test_type(other) {
                    let scope = crate::worldgen::mod_rule_tests::PosRuleTestDecodeScope::new(
                        depth, *remaining,
                    );
                    let decoded = ty.decode(p, &scope).map(Self::Mod).map_err(|e| {
                        DecodeError::Registry(format!("position rule test {other}: {e}"))
                    })?;
                    *remaining = scope.remaining();
                    return Ok(decoded);
                }
                return Err(DecodeError::Registry(format!(
                    "unknown position rule test type {other} (field predicate_type)"
                )));
            }
        })
    }
    /// `PosRuleTest.test(inTemplatePos, worldPos, worldReference, random)`.
    pub fn test(
        &self,
        in_template: [i32; 3],
        world: [i32; 3],
        reference: [i32; 3],
        random: &mut impl RandomSource,
    ) -> Result<bool, String> {
        match self {
            Self::Always => Ok(true),
            Self::Linear {
                min_chance,
                max_chance,
                min_dist,
                max_dist,
                axis,
            } => {
                let dist = match axis {
                    None => (0..3).map(|i| (world[i] - reference[i]).abs()).sum::<i32>(),
                    Some(a) => ((world[*a] - reference[*a]) as f32).abs() as i32,
                };
                Ok(random.next_float()
                    <= clamped_chance(*min_chance, *max_chance, *min_dist, *max_dist, dist))
            }
            Self::Mod(test) => {
                let mut rng = crate::worldgen::mod_random::EngineRandom(random);
                test.test(
                    &crate::worldgen::mod_rule_tests::PosRuleTestRunContext,
                    in_template,
                    world,
                    reference,
                    &mut rng,
                )
            }
        }
    }
}
/// Mth.clampedLerp(min_chance, max_chance, Mth.inverseLerp(dist, min_dist, max_dist)).
fn clamped_chance(
    min_chance: f32,
    max_chance: f32,
    min_dist: i32,
    max_dist: i32,
    dist: i32,
) -> f32 {
    let t = (dist - min_dist) as f32 / (max_dist - min_dist) as f32;
    if t < 0. {
        min_chance
    } else if t > 1. {
        max_chance
    } else {
        min_chance + t * (max_chance - min_chance)
    }
}
impl Modifier {
    /// A rule's `block_entity_modifier` (passthrough when missing).
    fn decode(value: Option<&Value>) -> Result<Self, DecodeError> {
        let Some(m) = value else {
            return Ok(Self::Passthrough);
        };
        Ok(
            match super::super::tags::identifier(m["type"].as_str().unwrap_or(""))?.as_str() {
                "minecraft:passthrough" => Self::Passthrough,
                "minecraft:clear" => Self::Clear,
                "minecraft:append_loot" => {
                    Self::AppendLoot(m["loot_table"].as_str().unwrap_or_default().to_owned())
                }
                "minecraft:append_static" => match json_to_nbt(&m["data"]) {
                    Tag::Compound(c) => Self::AppendStatic(c),
                    _ => Self::AppendStatic(Compound::new()),
                },
                _ => return Err(DecodeError::Field("block entity modifier")),
            },
        )
    }
}
impl Modifier {
    /// RuleBlockEntityModifier.apply: the block entity the output gets.
    fn apply(
        &self,
        nbt: Option<Box<Compound>>,
        random: &mut impl ProcessorRandom,
    ) -> Option<Box<Compound>> {
        match self {
            Self::Passthrough => nbt,
            Self::Clear => Some(Box::new(Compound::new())),
            Self::AppendLoot(table) => {
                let mut nbt = nbt.unwrap_or_default();
                nbt.insert("LootTable".into(), Tag::String(table.as_str().into()));
                nbt.insert("LootTableSeed".into(), Tag::Long(random.long()));
                Some(nbt)
            }
            Self::AppendStatic(data) => Some(append_static(nbt, data)),
        }
    }
}
/// A block entity with `data` merged in (AppendStatic).
fn append_static(nbt: Option<Box<Compound>>, data: &Compound) -> Box<Compound> {
    let mut nbt = nbt.unwrap_or_default();
    merge(&mut nbt, data);
    nbt
}
/// CompoundTag.merge: compounds in both are merged, anything else replaced.
fn merge(into: &mut Compound, other: &Compound) {
    for (key, value) in other {
        match (into.get_mut(key), value) {
            (Some(Tag::Compound(mine)), Tag::Compound(theirs)) => merge(mine, theirs),
            _ => {
                into.insert(key.clone(), value.clone());
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Rule {
    input: RuleTest,
    location: RuleTest,
    position: PosRule,
    output: BlockStateId,
    modifier: Modifier,
}
#[derive(Clone, Debug)]
pub enum Processor {
    Nop,
    BlockIgnore(BTreeSet<String>),
    BlockRot {
        rottable: Option<Arc<BTreeSet<BlockStateId>>>,
        integrity: f32,
    },
    Gravity {
        heightmap: HeightmapType,
        offset: i32,
    },
    Rule(Vec<Rule>),
    Protected(Arc<BTreeSet<BlockStateId>>),
    Capped {
        delegate: Box<Processor>,
        limit: IntProvider,
    },
    JigsawReplacement,
    BlockAge(f32),
    LavaSubmerged,
    BlackstoneReplace,
    /// A mod-registered structure processor (`Registries.STRUCTURE_PROCESSOR`),
    /// resolved once at decode.
    Mod(Arc<dyn super::mod_processors::ModStructureProcessor>),
}
#[derive(Clone, Debug, Default)]
pub struct ProcessorList(pub Vec<Processor>);

/// A block holder set in any form `crate::holder_set` reads; a tag no pack
/// defines holds no blocks.
fn holder_set(
    value: &Value,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<Arc<BTreeSet<BlockStateId>>, DecodeError> {
    let lookup = super::super::block_feature::BlockSetLookup {
        bindings,
        unknown_tags_empty: true,
    };
    let ids = match value {
        Value::Array(list) => {
            let mut ids = Vec::new();
            for value in list.iter().filter(|value| value.is_string()) {
                ids.extend(
                    crate::holder_set::resolve(value, &lookup).map_err(DecodeError::Registry)?,
                );
            }
            ids
        }
        _ => crate::holder_set::resolve(value, &lookup).map_err(DecodeError::Registry)?,
    };
    Ok(Arc::new(ids.into_iter().collect()))
}
pub(super) fn json_to_nbt(value: &Value) -> Tag {
    match value {
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) if i32::try_from(i).is_ok() => Tag::Int(i as i32),
            Some(i) => Tag::Long(i),
            None => Tag::Double(n.as_f64().unwrap_or(0.).to_bits()),
        },
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(a) => Tag::List(a.iter().map(json_to_nbt).collect()),
        Value::Object(o) => Tag::Compound(
            o.iter()
                .map(|(k, v)| (JavaString::from(k.as_str()), json_to_nbt(v)))
                .collect(),
        ),
        Value::Null => Tag::End,
    }
}
fn float(value: &Value, key: &'static str) -> Result<f32, DecodeError> {
    value[key]
        .as_f64()
        .map(|v| v as f32)
        .ok_or(DecodeError::Field(key))
}
/// A required float field vanilla reads through `Codec.floatRange`: a value
/// outside the range, or a NaN, is refused at decode rather than clamped.
fn float_range(value: &Value, key: &'static str, min: f32, max: f32) -> Result<f32, DecodeError> {
    let v = float(value, key)?;
    if !(min..=max).contains(&v) {
        return Err(DecodeError::Field(key));
    }
    Ok(v)
}
impl Processor {
    pub fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let kind = value["processor_type"]
            .as_str()
            .ok_or(DecodeError::Field("processor_type"))?;
        let state = |v: &Value| -> Result<BlockStateId, DecodeError> {
            bindings
                .codec
                .decode(&v.to_string(), false)
                .map_err(|e| DecodeError::Registry(e.to_string()))
        };
        Ok(match super::super::tags::identifier(kind)?.as_str() {
            "minecraft:nop" => Self::Nop,
            "minecraft:block_ignore" => Self::BlockIgnore(
                value["blocks"]
                    .as_array()
                    .ok_or(DecodeError::Field("block_ignore blocks"))?
                    .iter()
                    .map(|b| state(b).map(|id| resources.registry.state(id).unwrap().name.clone()))
                    .collect::<Result<_, _>>()?,
            ),
            "minecraft:block_rot" => Self::BlockRot {
                rottable: match value.get("rottable_blocks") {
                    Some(v) => Some(holder_set(v, bindings)?),
                    None => None,
                },
                integrity: float_range(value, "integrity", 0., 1.)?,
            },
            "minecraft:gravity" => Self::Gravity {
                heightmap: match value.get("heightmap").and_then(Value::as_str) {
                    Some(name) => HeightmapType::ALL
                        .into_iter()
                        .find(|k| k.name() == name)
                        .ok_or(DecodeError::Field("gravity heightmap"))?,
                    None => HeightmapType::WorldSurfaceWg,
                },
                offset: value.get("offset").and_then(Value::as_i64).unwrap_or(0) as i32,
            },
            "minecraft:protected_blocks" => Self::Protected(holder_set(&value["value"], bindings)?),
            "minecraft:jigsaw_replacement" => Self::JigsawReplacement,
            "minecraft:block_age" => Self::BlockAge(float(value, "mossiness")?),
            "minecraft:lava_submerged_block" => Self::LavaSubmerged,
            "minecraft:blackstone_replace" => Self::BlackstoneReplace,
            "minecraft:capped" => {
                // CappedProcessor's limit is IntProviders.POSITIVE_CODEC: a
                // provider that can draw 0 or less is refused at decode.
                let limit = IntProvider::from_json(&value["limit"].to_string())?;
                if limit.bounds()[0] < 1 {
                    return Err(DecodeError::Field("capped limit"));
                }
                Self::Capped {
                    delegate: Box::new(Self::decode(&value["delegate"], resources, bindings)?),
                    limit,
                }
            }
            "minecraft:rule" => Self::Rule(
                value["rules"]
                    .as_array()
                    .ok_or(DecodeError::Field("rules"))?
                    .iter()
                    .enumerate()
                    .map(|(index, rule)| {
                        let test = |key| {
                            RuleTest::from_json(&rule[key].to_string(), bindings).map_err(|e| {
                                DecodeError::Registry(format!("rule[{index}] field {key}: {e}"))
                            })
                        };
                        let position =
                            PosRule::decode(rule.get("position_predicate")).map_err(|e| {
                                DecodeError::Registry(format!(
                                    "rule[{index}] field position_predicate: {e}"
                                ))
                            })?;
                        let modifier = Modifier::decode(rule.get("block_entity_modifier"))
                            .map_err(|e| {
                                DecodeError::Registry(format!(
                                    "rule[{index}] field block_entity_modifier: {e}"
                                ))
                            })?;
                        let output = state(&rule["output_state"]).map_err(|e| {
                            DecodeError::Registry(format!("rule[{index}] field output_state: {e}"))
                        })?;
                        Ok(Rule {
                            input: test("input_predicate")?,
                            location: test("location_predicate")?,
                            position,
                            output,
                            modifier,
                        })
                    })
                    .collect::<Result<_, DecodeError>>()?,
            ),
            other => {
                if let Some(ty) = super::mod_processors::mod_structure_processor_type(other) {
                    return ty
                        .decode(value, resources, bindings)
                        .map(Self::Mod)
                        .map_err(DecodeError::Registry);
                }
                return Err(DecodeError::Field("structure processor type"));
            }
        })
    }
    fn evaluates_entire_piece(&self) -> bool {
        match self {
            Self::Capped { .. } => true,
            Self::Mod(processor) => processor.evaluates_entire_piece_state(),
            _ => false,
        }
    }
}
impl ProcessorList {
    /// Data-pack relative path of a named processor list.
    pub fn file_path(id: &str) -> String {
        match id.split_once(':') {
            Some((ns, path)) => format!("data/{ns}/worldgen/processor_list/{path}.json"),
            None => format!("data/minecraft/worldgen/processor_list/{id}.json"),
        }
    }
    /// Decode a named processor list, wrapping errors with its id and data-pack file path.
    pub fn decode_named(
        name: &str,
        json: &str,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let id = super::super::tags::identifier(name)?;
        Self::decode(json, resources, bindings).map_err(|e| {
            let file = Self::file_path(&id);
            DecodeError::Registry(format!("processor list {id} ({file}): {e}"))
        })
    }
    /// A processor list holder: a registry name, `{"processors": [...]}` or a list.
    pub fn decode(
        json: &str,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let value: Value = serde_json::from_str(json)?;
        Self::decode_value(&value, resources, bindings)
    }
    /// Decode a processor list holder from an already-parsed JSON value.
    pub fn decode_value(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        // Lists name lists (a mod's): a cycle of names is refused rather than followed.
        thread_local!(static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) });
        let depth = DEPTH.get();
        if depth >= 16 {
            return Err(DecodeError::Field("processor lists nest too deep"));
        }
        DEPTH.set(depth + 1);
        let decoded = Self::decode_at(value, resources, bindings);
        DEPTH.set(depth);
        decoded
    }
    fn decode_at(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let owned;
        let named = match value {
            Value::String(name) => {
                let name = super::super::tags::identifier(name)?;
                let raw = resources.processor_lists.get(&name).ok_or_else(|| {
                    let file = Self::file_path(&name);
                    DecodeError::Registry(format!("unknown processor list {name} ({file})"))
                })?;
                owned = serde_json::from_str::<Value>(raw).map_err(|e| {
                    let file = Self::file_path(&name);
                    DecodeError::Registry(format!("processor list {name} ({file}): {e}"))
                })?;
                Some((name, &owned))
            }
            _ => None,
        };
        let target = named.as_ref().map_or(value, |(_, v)| *v);
        let list = match target {
            Value::Array(a) => a.as_slice(),
            Value::Object(_) => target["processors"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            _ => return Err(DecodeError::Field("processor list")),
        };
        let processors = list
            .iter()
            .map(|p| Processor::decode(p, resources, bindings))
            .collect::<Result<Vec<_>, _>>();
        match (processors, named) {
            (Ok(processors), _) => Ok(Self(processors)),
            (Err(e), Some((name, _))) => {
                let file = Self::file_path(&name);
                Err(DecodeError::Registry(format!(
                    "processor list {name} ({file}): {e}"
                )))
            }
            (Err(e), None) => Err(e),
        }
    }
}

/// StructurePlaceSettings without entity options.
pub struct PlaceSettings<'a> {
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub pivot: [i32; 3],
    pub bounding_box: Option<BoundingBox>,
    pub processors: Vec<&'a Processor>,
    pub waterlogging: bool,
    pub random: Option<&'a mut dyn ProcessorRandom>,
    /// StructurePlaceSettings.knownShape: skip the neighbor shape updates
    /// after placement (jigsaw pieces).
    pub known_shape: bool,
    /// StructurePlaceSettings.ignoreEntities / finalizeEntities.
    pub ignore_entities: bool,
    pub finalize_entities: bool,
}
impl<'a> PlaceSettings<'a> {
    pub fn new(rotation: Rotation) -> Self {
        Self {
            mirror: Mirror::None,
            rotation,
            pivot: [0; 3],
            bounding_box: None,
            processors: Vec::new(),
            waterlogging: true,
            random: None,
            known_shape: false,
            ignore_entities: false,
            finalize_entities: false,
        }
    }
    pub fn with_processors(mut self, list: &'a ProcessorList) -> Self {
        self.processors.extend(list.0.iter());
        self
    }
    /// settings.getRandom(pos): the shared random, else a position-seeded one.
    fn with_random<T>(
        &mut self,
        pos: [i32; 3],
        f: impl FnOnce(&mut dyn ProcessorRandom) -> T,
    ) -> T {
        match self.random.as_deref_mut() {
            Some(random) => f(random),
            None => f(&mut LegacyRandom::new(position_seed(BlockPos::new(
                pos[0], pos[1], pos[2],
            )))),
        }
    }
}

/// BlackstoneReplaceProcessor's block map.
fn blackstone(name: &str) -> Option<&'static str> {
    Some(match name.strip_prefix("minecraft:")? {
        "cobblestone" | "mossy_cobblestone" => "blackstone",
        "stone" => "polished_blackstone",
        "stone_bricks" | "mossy_stone_bricks" => "polished_blackstone_bricks",
        "cobblestone_stairs" | "mossy_cobblestone_stairs" => "blackstone_stairs",
        "stone_stairs" => "polished_blackstone_stairs",
        "stone_brick_stairs" | "mossy_stone_brick_stairs" => "polished_blackstone_brick_stairs",
        "cobblestone_slab" | "mossy_cobblestone_slab" => "blackstone_slab",
        "smooth_stone_slab" | "stone_slab" => "polished_blackstone_slab",
        "stone_brick_slab" | "mossy_stone_brick_slab" => "polished_blackstone_brick_slab",
        "stone_brick_wall" | "mossy_stone_brick_wall" => "polished_blackstone_brick_wall",
        "cobblestone_wall" | "mossy_cobblestone_wall" => "blackstone_wall",
        "chiseled_stone_bricks" => "chiseled_polished_blackstone",
        "cracked_stone_bricks" => "cracked_polished_blackstone_bricks",
        "iron_bars" => "iron_chain",
        _ => return None,
    })
}
type ProcessResult = Result<Option<BlockInfo>, PlacementQueryError>;
/// The processors of a list in turn, until one removes the block.
#[allow(clippy::too_many_arguments)]
pub(super) fn process_list<W: PlacementContext>(
    list: &ProcessorList,
    world: &mut W,
    resources: &StructureResources,
    position: [i32; 3],
    reference: [i32; 3],
    original: &BlockInfo,
    info: BlockInfo,
    settings: &mut PlaceSettings<'_>,
) -> ProcessResult {
    let mut current = info;
    for processor in &list.0 {
        match process_block(
            processor, world, resources, position, reference, original, current, settings,
        )? {
            Some(next) => current = next,
            None => return Ok(None),
        }
    }
    Ok(Some(current))
}
/// StructureProcessor.processBlock: `position` is the piece's, `reference`
/// the structure's pivot and `original` the template's untransformed block.
#[allow(clippy::too_many_arguments)]
pub(super) fn process_block<W: PlacementContext>(
    processor: &Processor,
    world: &mut W,
    resources: &StructureResources,
    position: [i32; 3],
    reference: [i32; 3],
    original: &BlockInfo,
    info: BlockInfo,
    settings: &mut PlaceSettings<'_>,
) -> ProcessResult {
    let relative = original.pos;
    let registry = resources.registry;
    let name = |id: BlockStateId| registry.state(id).map(|s| s.name.as_str()).unwrap_or("");
    Ok(match processor {
        Processor::Nop => Some(info),
        Processor::BlockIgnore(blocks) => (!blocks.contains(name(info.state))).then_some(info),
        Processor::BlockRot {
            rottable,
            integrity,
        } => {
            let candidate = rottable
                .as_ref()
                .is_none_or(|set| set.contains(&info.state));
            let keep = settings.with_random(info.pos, |r| !candidate || r.float() <= *integrity);
            keep.then_some(info)
        }
        Processor::Gravity { heightmap, offset } => {
            let height = world.height(*heightmap, info.pos[0], info.pos[2])? + offset;
            Some(BlockInfo {
                pos: [info.pos[0], height + relative[1], info.pos[2]],
                ..info
            })
        }
        Processor::Protected(set) => (!set.contains(&world.block_state(info.pos)?)).then_some(info),
        Processor::JigsawReplacement => {
            if !resources.is_jigsaw(info.state) {
                return Ok(Some(info));
            }
            let Some(nbt) = &info.nbt else {
                return Ok(Some(info));
            };
            let text = match nbt.get(&JavaString::from("final_state")) {
                Some(Tag::String(s)) => s.to_string_lossy(),
                _ => "minecraft:air".to_owned(),
            };
            let Some(state) = super::template::parse_state_string(registry, &text) else {
                return Ok(None);
            };
            (name(state) != "minecraft:structure_void").then_some(BlockInfo {
                pos: info.pos,
                state,
                nbt: None,
            })
        }
        Processor::LavaSubmerged => {
            let lava = name(world.block_state(info.pos)?) == "minecraft:lava";
            if lava && !resources.transforms.full_block(info.state) {
                Some(BlockInfo {
                    state: registry.default_state("minecraft:lava").unwrap(),
                    ..info
                })
            } else {
                Some(info)
            }
        }
        Processor::BlockAge(mossiness) => block_age(*mossiness, info, settings, resources),
        Processor::BlackstoneReplace => {
            let Some(target) = blackstone(name(info.state)) else {
                return Ok(Some(info));
            };
            let old = registry.state(info.state);
            let properties: Vec<(&str, &str)> = ["facing", "half", "type"]
                .into_iter()
                .filter_map(|k| Some((k, old?.properties.get(k)?.as_str())))
                .collect();
            Some(BlockInfo {
                state: super::legacy::state(resources, target, &properties),
                ..info
            })
        }
        Processor::Rule(rules) => {
            let mut random = LegacyRandom::new(position_seed(BlockPos::new(
                info.pos[0],
                info.pos[1],
                info.pos[2],
            )));
            for rule in rules {
                if !rule
                    .input
                    .test(info.state, info.pos, &mut random)
                    .map_err(PlacementQueryError::Failed)?
                {
                    continue;
                }
                let located = world.block_state(info.pos)?;
                if !rule
                    .location
                    .test(located, info.pos, &mut random)
                    .map_err(PlacementQueryError::Failed)?
                {
                    continue;
                }
                let position_ok = rule
                    .position
                    .test(relative, info.pos, reference, &mut random)
                    .map_err(PlacementQueryError::Failed)?;
                if !position_ok {
                    continue;
                }
                return Ok(Some(BlockInfo {
                    pos: info.pos,
                    state: rule.output,
                    nbt: rule.modifier.apply(info.nbt, &mut random),
                }));
            }
            Some(info)
        }
        Processor::Capped { .. } => Some(info),
        Processor::Mod(mod_processor) => {
            let mut ctx = super::mod_processors::ProcessorRunContext::new(
                world, resources, position, reference, settings, info.pos,
            );
            return mod_processor.process_block(&mut ctx, original, info);
        }
    })
}
fn block_age(
    mossiness: f32,
    info: BlockInfo,
    settings: &mut PlaceSettings<'_>,
    resources: &StructureResources,
) -> Option<BlockInfo> {
    let registry = resources.registry;
    let state = registry.state(info.state)?.clone();
    let default = |n: &str| registry.default_state(n).unwrap();
    let with_props = |n: &str| {
        let mut target = registry.state(default(n)).unwrap().clone();
        for (k, v) in &state.properties {
            if target.properties.contains_key(k) {
                target.properties.insert(k.clone(), v.clone());
            }
        }
        registry.id(&target).unwrap_or(default(n))
    };
    let random_stairs = |r: &mut dyn ProcessorRandom, n: &str| {
        let facing = ["north", "east", "south", "west"][r.int(4) as usize];
        let half = ["top", "bottom"][r.int(2) as usize];
        let mut target = registry.state(default(n)).unwrap().clone();
        target.properties.insert("facing".into(), facing.into());
        target.properties.insert("half".into(), half.into());
        registry.id(&target).unwrap()
    };
    let name = state.name.as_str();
    let is_tag = |tag: &str| {
        // Tags are matched by block name through the template resources' bindings.
        resources
            .block_tags
            .get(tag)
            .is_some_and(|set| set.contains(name))
    };
    let new_state = settings.with_random(info.pos, |r| -> Option<BlockStateId> {
        let pick = |r: &mut dyn ProcessorRandom, non: &[BlockStateId], mossy: &[BlockStateId]| {
            if r.float() < mossiness {
                mossy[r.int(mossy.len() as i32) as usize]
            } else {
                non[r.int(non.len() as i32) as usize]
            }
        };
        if matches!(
            name,
            "minecraft:stone_bricks" | "minecraft:stone" | "minecraft:chiseled_stone_bricks"
        ) {
            if r.float() >= 0.5 {
                return None;
            }
            let non = [
                default("minecraft:cracked_stone_bricks"),
                random_stairs(r, "minecraft:stone_brick_stairs"),
            ];
            let mossy = [
                default("minecraft:mossy_stone_bricks"),
                random_stairs(r, "minecraft:mossy_stone_brick_stairs"),
            ];
            Some(pick(r, &non, &mossy))
        } else if is_tag("minecraft:stairs") {
            if r.float() >= 0.5 {
                return None;
            }
            let non = [
                default("minecraft:stone_slab"),
                default("minecraft:stone_brick_slab"),
            ];
            let mossy = [
                with_props("minecraft:mossy_stone_brick_stairs"),
                default("minecraft:mossy_stone_brick_slab"),
            ];
            Some(pick(r, &non, &mossy))
        } else if is_tag("minecraft:slabs") {
            (r.float() < mossiness).then(|| with_props("minecraft:mossy_stone_brick_slab"))
        } else if is_tag("minecraft:walls") {
            (r.float() < mossiness).then(|| with_props("minecraft:mossy_stone_brick_wall"))
        } else if name == "minecraft:obsidian" {
            (r.float() < 0.15).then(|| default("minecraft:crying_obsidian"))
        } else {
            None
        }
    });
    Some(match new_state {
        Some(state) => BlockInfo { state, ..info },
        None => info,
    })
}

/// StructureTemplate.processBlockInfos.
fn process_infos<W: FeatureWorld + PlacementContext>(
    world: &mut W,
    resources: &StructureResources,
    template: &StructureTemplate,
    palette: usize,
    position: [i32; 3],
    reference: [i32; 3],
    settings: &mut PlaceSettings<'_>,
) -> Result<Vec<BlockInfo>, PlacementQueryError> {
    let processors = settings.processors.clone();
    let whole = processors.iter().any(|p| p.evaluates_entire_piece());
    let mut original = Vec::new();
    let mut processed = Vec::new();
    for info in template.palettes()[palette].blocks() {
        let relative = StructureTemplate::transform(
            info.pos,
            settings.mirror,
            settings.rotation,
            settings.pivot,
        );
        let pos = std::array::from_fn(|i| relative[i] + position[i]);
        if !whole && settings.bounding_box.is_some_and(|b| !b.is_inside(pos)) {
            continue;
        }
        let mut current = Some(BlockInfo {
            pos,
            state: info.state,
            nbt: info.nbt.clone(),
        });
        for &processor in &processors {
            let Some(value) = current.take() else { break };
            current = process_block(
                processor, world, resources, position, reference, info, value, settings,
            )?;
        }
        if let Some(value) = current {
            processed.push(value);
            if whole {
                original.push(info.clone());
            }
        }
    }
    for &processor in &processors {
        match processor {
            Processor::Capped { delegate, limit } => {
                if limit.bounds()[1] == 0
                    || processed.is_empty()
                    || original.len() != processed.len()
                {
                    continue;
                }
                let seed = world.world_seed()?;
                let mut random = LegacyRandom::new(seed).fork_positional().at(BlockPos::new(
                    position[0],
                    position[1],
                    position[2],
                ));
                let max = limit
                    .sample(&mut random)
                    .unwrap_or(0)
                    .min(processed.len() as i32);
                if max < 1 {
                    continue;
                }
                let mut indices: Vec<usize> = (0..processed.len()).collect();
                for i in (2..=indices.len()).rev() {
                    let swap = random.next_int_bound(i as i32).unwrap_or(0) as usize;
                    indices.swap(i - 1, swap);
                }
                let mut replaced = 0;
                for index in indices {
                    if replaced >= max {
                        break;
                    }
                    let before = processed[index].clone();
                    let altered = process_block(
                        delegate,
                        world,
                        resources,
                        position,
                        reference,
                        &original[index],
                        before.clone(),
                        settings,
                    )?;
                    if let Some(altered) = altered
                        && altered != before
                    {
                        replaced += 1;
                        processed[index] = altered;
                    }
                }
            }
            Processor::Mod(mod_processor) => {
                let mut ctx = super::mod_processors::ProcessorRunContext::new(
                    world, resources, position, reference, settings, position,
                );
                processed = mod_processor.finalize_processing(&mut ctx, &original, processed)?;
            }
            _ => {}
        }
    }
    Ok(processed)
}

/// StructureTemplate.placeInWorld. Returns false for a template with no
/// palette, with no blocks and no entities to place, or with a side below 1.
#[allow(clippy::too_many_arguments)]
pub fn place_in_world<W: FeatureWorld + PlacementContext>(
    world: &mut W,
    resources: &StructureResources,
    template: &StructureTemplate,
    position: [i32; 3],
    reference: [i32; 3],
    settings: &mut PlaceSettings<'_>,
    mut loot_random: Option<&mut dyn ProcessorRandom>,
    flags: i32,
) -> Result<bool, PlacementQueryError> {
    if template.palettes().is_empty() {
        return Ok(false);
    }
    let palette = match settings.random.as_deref_mut() {
        Some(r) => r.int(template.palettes().len() as i32) as usize,
        None => LegacyRandom::new(position_seed(BlockPos::new(
            position[0],
            position[1],
            position[2],
        )))
        .next_int_bound(template.palettes().len() as i32)
        .unwrap_or(0) as usize,
    };
    let has_entities = !settings.ignore_entities && !template.entities.is_empty();
    if (template.palettes()[palette].blocks().is_empty() && !has_entities)
        || template.size.iter().any(|s| *s < 1)
    {
        return Ok(false);
    }
    let infos = process_infos(
        world, resources, template, palette, position, reference, settings,
    )?;
    let barrier = resources
        .registry
        .default_state("minecraft:barrier")
        .unwrap();
    let mut to_fill = Vec::new();
    let mut locked = Vec::new();
    let mut placed = Vec::new();
    let world_err = |_| PlacementQueryError::Failed("template write".into());
    for info in infos {
        if settings
            .bounding_box
            .is_some_and(|b| !b.is_inside(info.pos))
        {
            continue;
        }
        let previous = if settings.waterlogging {
            Some(world.fluid_state(info.pos)?)
        } else {
            None
        };
        let state = resources.transforms.rotate_at(
            resources
                .transforms
                .mirror_at(info.state, info.pos, settings.mirror),
            info.pos,
            settings.rotation,
        );
        if info.nbt.is_some() {
            world.set_block(info.pos, barrier, 820).map_err(world_err)?;
        }
        if !world.set_block(info.pos, state, flags).map_err(world_err)? {
            continue;
        }
        placed.push(info.pos);
        if let Some(mut nbt) = info.nbt {
            if resources.transforms.loot_container(state) {
                let seed = match loot_random.as_deref_mut() {
                    Some(random) => random.long(),
                    None => settings.random.as_deref_mut().map_or(0, |r| r.long()),
                };
                nbt.insert("LootTableSeed".into(), Tag::Long(seed));
            }
            if let Some(transform) = resources.registry.structure_block_entity_transform.get() {
                transform.transform(info.pos, &mut nbt, settings.rotation, settings.mirror);
            }
            world.set_block_entity(info.pos, *nbt)?;
        }
        if let Some(previous) = previous {
            if resources.source_fluid(state) {
                locked.push(info.pos);
            } else {
                let part = world.has_template_liquid_override(state);
                if part || resources.waterlogged.contains_key(&state) {
                    let restored =
                        restore_template_liquid(world, resources, info.pos, state, previous)?;
                    if !previous.is_source() && (part || !restored) {
                        to_fill.push(info.pos);
                    }
                }
            }
        }
    }
    let mut filled = true;
    while filled && !to_fill.is_empty() {
        filled = false;
        let mut remaining = Vec::new();
        for pos in std::mem::take(&mut to_fill) {
            let mut fluid = world.fluid_state(pos)?;
            for d in [[0, 1, 0], [0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]] {
                if fluid.is_source() {
                    break;
                }
                let n = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                let neighbor = world.fluid_state(n)?;
                if neighbor.is_source() && !locked.contains(&n) {
                    fluid = neighbor;
                }
            }
            let state = world.block_state(pos)?;
            if fluid.is_source() && restore_template_liquid(world, resources, pos, state, fluid)? {
                filled = true;
            } else {
                remaining.push(pos);
            }
        }
        to_fill = remaining;
    }
    if !settings.known_shape && !placed.is_empty() {
        update_shape_at_edge(world, &placed, flags)?;
        for &pos in &placed {
            world.update_from_neighbour_shapes(pos, flags & !1 | 16)?;
        }
    }
    if !settings.ignore_entities {
        place_entities(world, template, position, settings)?;
    }
    Ok(true)
}

/// Both `StructureTemplate.placeInWorld` restores call `placeLiquid` directly.
/// An explicit answer, even false, takes the place of the template write.
fn restore_template_liquid<W: FeatureWorld + PlacementContext>(
    world: &mut W,
    resources: &StructureResources,
    pos: [i32; 3],
    state: BlockStateId,
    fluid: FluidState,
) -> Result<bool, PlacementQueryError> {
    if world.place_template_liquid(pos, state, fluid).is_some() {
        return Ok(true);
    }
    let Some(wet) = resources
        .waterlogged
        .get(&state)
        .filter(|_| fluid.kind() == FluidKind::Water)
    else {
        return Ok(false);
    };
    world
        .set_block(pos, *wet, 3)
        .map_err(|_| PlacementQueryError::Failed("template write".into()))?;
    world.schedule_fluid_tick(pos, fluid.kind(), 5).ok();
    Ok(true)
}

/// StructureTemplate.placeEntities: each saved entity moved, turned and
/// mirrored with the template, its NBT handed to the world.
fn place_entities<W: PlacementContext>(
    world: &mut W,
    template: &StructureTemplate,
    position: [i32; 3],
    settings: &PlaceSettings<'_>,
) -> Result<(), PlacementQueryError> {
    let (mirror, rotation, pivot) = (settings.mirror, settings.rotation, settings.pivot);
    for e in &template.entities {
        let b = StructureTemplate::transform(e.block_pos, mirror, rotation, pivot);
        let block_pos = [b[0] + position[0], b[1] + position[1], b[2] + position[2]];
        if settings
            .bounding_box
            .is_some_and(|bb| !bb.is_inside(block_pos))
        {
            continue;
        }
        let r = StructureTemplate::transform_vec(e.pos, mirror, rotation, pivot);
        let pos = [
            r[0] + f64::from(position[0]),
            r[1] + f64::from(position[1]),
            r[2] + f64::from(position[2]),
        ];
        let mut nbt = e.nbt.clone();
        nbt.insert(
            "Pos".into(),
            Tag::List(pos.iter().map(|v| Tag::Double(v.to_bits())).collect()),
        );
        nbt.remove(&"UUID".into());
        if nbt.contains_key(&"block_pos".into()) {
            nbt.insert("block_pos".into(), Tag::IntArray(block_pos.to_vec()));
        }
        for (key, i) in [("TileX", 0), ("TileY", 1), ("TileZ", 2)] {
            if nbt.contains_key(&key.into()) {
                nbt.insert(key.into(), Tag::Int(block_pos[i]));
            }
        }
        // Entity.rotate + Entity.mirror on the yaw.
        let wrap = |a: f32| {
            let a = a % 360.0;
            if a >= 180.0 {
                a - 360.0
            } else if a < -180.0 {
                a + 360.0
            } else {
                a
            }
        };
        if let Some(Tag::List(rot)) = nbt.get(&"Rotation".into()).cloned()
            && let Some(Tag::Float(bits)) = rot.first()
        {
            let yaw = wrap(f32::from_bits(*bits));
            let rotated = match rotation {
                Rotation::Clockwise180 => yaw + 180.0,
                Rotation::Counterclockwise90 => yaw + 270.0,
                Rotation::Clockwise90 => yaw + 90.0,
                Rotation::None => yaw,
            };
            let mirrored = match mirror {
                Mirror::FrontBack => -yaw,
                Mirror::LeftRight => 180.0 - yaw,
                Mirror::None => yaw,
            };
            let mut rot = rot.clone();
            rot[0] = Tag::Float((rotated + mirrored - yaw).to_bits());
            nbt.insert("Rotation".into(), Tag::List(rot));
        }
        // HangingEntity.rotate/mirror: the facing turns with the template.
        if let Some(&Tag::Byte(f)) = nbt.get(&"Facing".into()) {
            nbt.insert("Facing".into(), Tag::Byte(turn_facing(f, mirror, rotation)));
        }
        if settings.finalize_entities {
            nbt.insert("sunrise:finalize".into(), Tag::Byte(1));
        }
        world.add_entity(nbt)?;
    }
    Ok(())
}
/// A 3D data value (2 north, 3 south, 4 west, 5 east) mirrored then rotated.
fn turn_facing(f: i8, mirror: Mirror, rotation: Rotation) -> i8 {
    if !(2..=5).contains(&f) {
        return f;
    }
    let mirrored = match (mirror, f) {
        (Mirror::LeftRight, 2) => 3,
        (Mirror::LeftRight, 3) => 2,
        (Mirror::FrontBack, 4) => 5,
        (Mirror::FrontBack, 5) => 4,
        (_, f) => f,
    };
    // Clockwise order: north, east, south, west.
    let order = [2, 5, 3, 4];
    let i = order.iter().position(|&d| d == mirrored).unwrap_or(0);
    let steps = match rotation {
        Rotation::None => 0,
        Rotation::Clockwise90 => 1,
        Rotation::Clockwise180 => 2,
        Rotation::Counterclockwise90 => 3,
    };
    order[(i + steps) % 4]
}

/// StructureTemplate.updateShapeAtEdge over the placed positions' voxel
/// shape (DiscreteVoxelShape.forAllFaces order: X-Y rows along Z, then
/// Z-X rows along Y, then Y-Z rows along X).
fn update_shape_at_edge<W: PlacementContext>(
    world: &mut W,
    placed: &[[i32; 3]],
    flags: i32,
) -> Result<(), PlacementQueryError> {
    use sunrise_core::geometry::Direction;
    let mut min = placed[0];
    let mut max = placed[0];
    for p in placed {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    let size: [i32; 3] = std::array::from_fn(|i| max[i] - min[i] + 1);
    let index = |[x, y, z]: [i32; 3]| ((x * size[1] + y) * size[2] + z) as usize;
    let mut bits = vec![false; size.iter().map(|v| *v as usize).product()];
    for p in placed {
        bits[index(std::array::from_fn(|i| p[i] - min[i]))] = true;
    }
    let full = |p: [i32; 3]| (0..3).all(|i| (0..size[i]).contains(&p[i])) && bits[index(p)];
    type Map = fn(i32, i32, i32) -> [i32; 3];
    let passes: [(Direction, Direction, i32, i32, i32, Map); 3] = [
        (
            Direction::North,
            Direction::South,
            size[0],
            size[1],
            size[2],
            |a, b, c| [a, b, c],
        ),
        (
            Direction::Down,
            Direction::Up,
            size[2],
            size[0],
            size[1],
            |a, b, c| [b, c, a],
        ),
        (
            Direction::West,
            Direction::East,
            size[1],
            size[2],
            size[0],
            |a, b, c| [c, a, b],
        ),
    ];
    for (negative, positive, a_size, b_size, c_size, map) in passes {
        for a in 0..a_size {
            for b in 0..b_size {
                let mut last = false;
                for c in 0..=c_size {
                    let here = c != c_size && full(map(a, b, c));
                    let face = if !last && here {
                        Some((negative, map(a, b, c)))
                    } else if last && !here {
                        Some((positive, map(a, b, c - 1)))
                    } else {
                        None
                    };
                    if let Some((direction, local)) = face {
                        let p = std::array::from_fn(|i| min[i] + local[i]);
                        world.update_shape_at_face(p, direction, flags)?;
                    }
                    last = here;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn plain_block_lists_expand_tags_and_ignore_non_strings() {
        let (mut tag, _) = sunrise_core::nbt::parse_snbt(r#"{DataVersion:0,states:[{id:"minecraft:air"},{id:"minecraft:cave_air"},{id:"minecraft:void_air"},{id:"minecraft:oak_log"}],defaults:{"minecraft:air":0,"minecraft:cave_air":1,"minecraft:void_air":2,"minecraft:oak_log":3}}"#).unwrap();
        *tag.get_mut("DataVersion").unwrap() = Tag::Int(crate::DATA_VERSION);
        let registry = crate::registry::BlockRegistry::from_nbt(&tag).unwrap();
        let tags = BTreeMap::from([("minecraft:logs".into(), vec!["minecraft:oak_log".into()])]);
        let bindings = FeatureBlockBindings::with_tags(&registry, BTreeSet::new(), &tags).unwrap();
        assert_eq!(
            *holder_set(&json!(["#minecraft:logs", 19, false, {}]), &bindings).unwrap(),
            BTreeSet::from([BlockStateId(3)])
        );
        assert!(
            holder_set(&json!(["#minecraft:missing", null]), &bindings)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rule_parts_read_ids_without_a_namespace() {
        // CTOV writes "always_true"; vanilla reads it as minecraft:always_true.
        for kind in ["always_true", "minecraft:always_true"] {
            let p = json!({"predicate_type": kind});
            assert!(
                matches!(PosRule::decode(Some(&p)), Ok(PosRule::Always)),
                "{kind}"
            );
        }
        let linear = json!({"predicate_type": "axis_aligned_linear_pos", "axis": "z",
                            "min_chance": 0.1, "max_dist": 8});
        assert!(matches!(
            PosRule::decode(Some(&linear)),
            Ok(PosRule::Linear {
                axis: Some(2),
                max_dist: 8,
                ..
            })
        ));
        assert!(PosRule::decode(Some(&json!({"predicate_type": "x:unknown"}))).is_err());
        assert!(matches!(PosRule::decode(None), Ok(PosRule::Always)));
        let loot = json!({"type": "append_loot", "loot_table": "minecraft:chests/x"});
        assert!(
            matches!(Modifier::decode(Some(&loot)), Ok(Modifier::AppendLoot(t)) if t == "minecraft:chests/x")
        );
        assert!(matches!(
            Modifier::decode(Some(&json!({"type": "clear"}))),
            Ok(Modifier::Clear)
        ));
        assert!(Modifier::decode(Some(&json!({"type": ""}))).is_err());
    }

    #[test]
    fn block_entity_edits_merge_deep_and_drop_keys() {
        let nbt = |v: Value| match json_to_nbt(&v) {
            Tag::Compound(c) => Box::new(c),
            _ => unreachable!(),
        };
        let vault = nbt(json!({"id": "minecraft:vault",
            "config": {"loot_table": "x:old", "key_item": {"id": "minecraft:trial_key"}},
            "server_data": {"state_updating_resumes_at": 533162741}}));
        // append_static merges compounds the way CompoundTag.merge does.
        let Ok(Modifier::AppendStatic(data)) = Modifier::decode(Some(&json!({
            "type": "append_static", "data": {"config": {"loot_table": "x:new"}}})))
        else {
            panic!("append_static")
        };
        let merged = append_static(Some(vault.clone()), &data);
        assert_eq!(
            merged,
            nbt(json!({"id": "minecraft:vault",
            "config": {"loot_table": "x:new", "key_item": {"id": "minecraft:trial_key"}},
            "server_data": {"state_updating_resumes_at": 533162741}}))
        );
        assert_eq!(
            append_static(None, &data),
            nbt(json!({"config": {"loot_table": "x:new"}}))
        );
    }
}
