#!/usr/bin/env python3
"""Remove engine copies of third-party structure processors (#4922 part 2).

Deletes sunrise-world's lithostitched/YUNG/Repurposed/Moog processor modules
and arms, and removes sunrise-port-pack's rewrites into those removed engine
processor names (leaving vanilla rewrites, sunrise:matter_phase for #5145,
and internal sunrise:finalize entity marker intact).
"""

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parent.parent


def replace_exact(path: Path, replacements: list[tuple[str, str]]) -> None:
    text = path.read_text(encoding="utf-8")
    for idx, (old, new) in enumerate(replacements):
        count = text.count(old)
        if count != 1:
            raise RuntimeError(
                f"{path}: replacement #{idx} matched {count} times (expected 1):\n{old[:160]!r}"
            )
        text = text.replace(old, new, 1)
    path.write_text(text, encoding="utf-8")


def remove_processor_modules() -> None:
    proc_dir = ROOT / "crates/sunrise-world/src/worldgen/structure/processor"
    for name in ("litho.rs", "pillars.rs", "sea_level.rs"):
        target = proc_dir / name
        if target.exists():
            target.unlink()
    if proc_dir.exists() and not any(proc_dir.iterdir()):
        proc_dir.rmdir()


def edit_processor_rs() -> None:
    path = ROOT / "crates/sunrise-world/src/worldgen/structure/processor.rs"
    replace_exact(
        path,
        [
            (
                "mod litho;\nmod pillars;\nmod sea_level;\n\n",
                "",
            ),
            (
                """    /// `sunrise:piece_origin_axis_aligned_linear_pos`: Repurposed Structures'
    /// PieceOriginAxisAlignedLinearPosRuleTest, vanilla's axis-aligned chance
    /// over the block's distance from the template's origin along one axis.
    PieceOrigin {
        min_chance: f32,
        max_chance: f32,
        min_dist: i32,
        max_dist: i32,
        axis: usize,
    },
    /// `sunrise:y_value`: Repurposed Structures' YValuePosRuleTest, the
    /// block's world height in `min_y_value..=max_y_value`.
    YValue {
        min: i32,
        max: i32,
    },
}""",
                "}",
            ),
            (
                """    /// `sunrise:edit`: `remove`'s keys taken out of the block entity, then
    /// `data` merged in as append_static does. The data form of the mods'
    /// processors that copy a block entity and change parts of it (Moog's
    /// trial spawners and vaults drop their saved state).
    Edit {
        remove: Vec<String>,
        data: Compound,
    },
}""",
                "}",
            ),
            (
                """            "sunrise:piece_origin_axis_aligned_linear_pos" => {
                let int = |key| p.get(key).and_then(Value::as_i64).unwrap_or(0) as i32;
                let (min_dist, max_dist) = (int("min_dist"), int("max_dist"));
                if min_dist >= max_dist {
                    return Err(DecodeError::Field("position predicate distance range"));
                }
                Self::PieceOrigin {
                    min_chance: p.get("min_chance").and_then(Value::as_f64).unwrap_or(0.) as f32,
                    max_chance: p.get("max_chance").and_then(Value::as_f64).unwrap_or(0.) as f32,
                    min_dist,
                    max_dist,
                    axis: match p.get("axis").map(|a| a.as_str()) {
                        Some(Some("x")) => 0,
                        None | Some(Some("y")) => 1,
                        Some(Some("z")) => 2,
                        _ => return Err(DecodeError::Field("position predicate axis")),
                    },
                }
            }
            "sunrise:y_value" => {
                let int = |key| {
                    p.get(key)
                        .and_then(Value::as_i64)
                        .map(|v| v as i32)
                        .ok_or(DecodeError::Field("position predicate y value"))
                };
                let (min, max) = (int("min_y_value")?, int("max_y_value")?);
                if min > max {
                    return Err(DecodeError::Field("position predicate y range"));
                }
                Self::YValue { min, max }
            }
            _ => return Err(DecodeError::Field("position predicate")),""",
                '            _ => return Err(DecodeError::Field("position predicate")),',
            ),
            (
                """            Self::PieceOrigin {
                min_chance,
                max_chance,
                min_dist,
                max_dist,
                axis,
            } => {
                let dist = (relative[axis] as f32).abs() as i32;
                random.next_float()
                    <= clamped_chance(min_chance, max_chance, min_dist, max_dist, dist)
            }
            Self::YValue { min, max } => (min..=max).contains(&world[1]),
        }""",
                "        }",
            ),
            (
                """                "sunrise:edit" => Self::Edit {
                    remove: m["remove"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|k| k.as_str().map(str::to_owned))
                        .collect::<Option<_>>()
                        .ok_or(DecodeError::Field("block entity modifier keys"))?,
                    data: match json_to_nbt(&m["data"]) {
                        Tag::Compound(c) => c,
                        _ => Compound::new(),
                    },
                },
                _ => return Err(DecodeError::Field("block entity modifier")),""",
                '                _ => return Err(DecodeError::Field("block entity modifier")),',
            ),
            (
                """            Self::AppendStatic(data) => Some(edited(nbt, &[], data)),
            Self::Edit { remove, data } => Some(edited(nbt, remove, data)),""",
                "            Self::AppendStatic(data) => Some(append_static(nbt, data)),",
            ),
            (
                """/// A block entity with `remove`'s keys taken out and `data` merged in
/// (append_static is the one with nothing to remove).
fn edited(nbt: Option<&Compound>, remove: &[String], data: &Compound) -> Compound {
    let mut nbt = nbt.cloned().unwrap_or_default();
    for key in remove {
        nbt.remove(&JavaString::from(key.as_str()));
    }
    merge(&mut nbt, data);
    nbt
}""",
                """/// A block entity with `data` merged in (AppendStatic).
fn append_static(nbt: Option<&Compound>, data: &Compound) -> Compound {
    let mut nbt = nbt.cloned().unwrap_or_default();
    merge(&mut nbt, data);
    nbt
}""",
            ),
            (
                """    BlackstoneReplace,
    SpawnerMobs(Box<SpawnerMobs>),
    Pick(Box<Pick>),
    CountCap(Box<CountCap>),
    /// `lithostitched:block_swap`: blocks by name become others, keeping
    /// the properties both have (a block the game lacks is left alone).
    BlockSwap(std::collections::BTreeMap<String, String>),
    /// `lithostitched:apply_random`: a processor list drawn by weight runs
    /// on the block.
    ApplyRandom(Box<litho::ApplyRandom>),
    /// `lithostitched:reference`: the named lists run on the block in turn.
    Reference(Vec<ProcessorList>),
    /// `lithostitched:discard_input`: the world's block stays.
    DiscardInput,
    /// `lithostitched:condition`: `then` or `else`
    /// runs on the block, by a test of it.
    Condition(Box<litho::Conditional>),
    /// `lithostitched:set_block`: the block from a state provider.
    SetBlock(Box<litho::SetBlock>),
    /// `sunrise:pillar` (Repurposed Structures' and Moog's pillars): a
    /// trigger block starts a column down to the ground.
    Pillar(Box<pillars::Pillar>),
    /// `sunrise:bottom_pillar` (Repurposed Structures'): the bottom layer
    /// continues down to the ground.
    BottomPillar(Box<pillars::BottomPillar>),
    /// `sunrise:close_off_fluid_sources` (Repurposed Structures' and Moog's):
    /// fluid sources beside open blocks are walled off.
    CloseOffFluidSources(Box<pillars::CloseOffFluidSources>),
    /// `sunrise:close_off_air_sources` (Repurposed Structures'): air and
    /// other liquids beside the template's fluids are walled off.
    CloseOffAirSources(Box<pillars::CloseOffAirSources>),
    /// `sunrise:yung_pillar` (YUNG's pillars: Better Fortresses, Better
    /// Jungle Temples).
    YungPillar(Box<pillars::YungPillar>),
    /// `sunrise:stair_pillar` (Better Fortresses' stair supports).
    StairPillar(Box<pillars::StairPillar>),
    /// `sunrise:flood_air` (Better Ocean Monuments'): air below sea level
    /// becomes water.
    FloodAir(Box<sea_level::FloodAir>),
    /// `sunrise:waterlog` (Better Ocean Monuments'): blocks below sea level
    /// are waterlogged.
    Waterlog,
    /// `sunrise:keep_world_block` (Better Ocean Monuments' structure void).
    KeepWorldBlock(Box<sea_level::KeepWorldBlock>),
    /// `sunrise:waterlog_in_world_water` (Repurposed Structures'): blocks
    /// are waterlogged where the world holds water, dry where it does not.
    WaterlogInWorldWater,
    /// `sunrise:mineshaft_sky_view` (Repurposed Structures' mineshafts):
    /// nothing above the ocean floor but rails on solid ground.
    MineshaftSkyView,
    /// A mod-registered structure processor (`Registries.STRUCTURE_PROCESSOR`),
    /// resolved once at decode.
    Mod(Arc<dyn super::mod_processors::ModStructureProcessor>),""",
                """    BlackstoneReplace,
    /// A mod-registered structure processor (`Registries.STRUCTURE_PROCESSOR`),
    /// resolved once at decode.
    Mod(Arc<dyn super::mod_processors::ModStructureProcessor>),""",
            ),
            (
                """/// The spawner fields `sunrise:spawner_mobs` writes: its JSON key, the NBT
/// key and the default (BaseSpawner's).
const SPAWNER_FIELDS: [(&str, &str, i16); 7] = [
    ("delay", "Delay", 20),
    ("min_spawn_delay", "MinSpawnDelay", 200),
    ("max_spawn_delay", "MaxSpawnDelay", 800),
    ("spawn_count", "SpawnCount", 4),
    ("max_nearby_entities", "MaxNearbyEntities", 6),
    ("required_player_range", "RequiredPlayerRange", 16),
    ("spawn_range", "SpawnRange", 4),
];

/// `sunrise:pick`, which `sunrise-port-pack` writes for the mods' processors
/// that turn a placeholder block into one of several (YUNG's, written in
/// Java): a block of `block` takes the first of `picks` whose `below` is past
/// one `settings.getRandom(pos).nextFloat()`, else `default` (no float is
/// drawn when there are no picks; with no default the block stays).
#[derive(Clone, Debug)]
pub struct Pick {
    block: String,
    picks: Vec<(f32, Outcome)>,
    default: Option<Outcome>,
    /// Each pick draws its own `nextFloat` (`if (r.nextFloat() < a) ...
    /// else if (r.nextFloat() < b)`), Better Ocean Monuments' way.
    redraw: bool,
    /// The picked block takes the placeholder's properties
    /// (Block.withPropertiesOf).
    keep_properties: bool,
}
#[derive(Clone, Debug)]
struct Outcome {
    /// The state, or one of them by `nextInt(len)` (Util.getRandom), drawn
    /// after the pick.
    states: Vec<BlockStateId>,
    /// The new block entity; the block's own stays when missing.
    nbt: Option<Compound>,
    /// Keys set to lists of `count` strings, each the first of `values`
    /// whose bound is past a new `nextFloat`, else `default` (pot sherds).
    lists: Vec<PickedList>,
}
#[derive(Clone, Debug)]
struct PickedList {
    key: String,
    count: usize,
    values: Vec<(f32, String)>,
    default: String,
}
fn below(v: &Value) -> Result<f32, DecodeError> {
    v["below"]
        .as_f64()
        .map(|b| b as f32)
        .ok_or(DecodeError::Field("pick bound"))
}
impl Pick {
    fn decode(
        value: &Value,
        state: impl Fn(&Value) -> Result<BlockStateId, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let outcome = |v: &Value| -> Result<Outcome, DecodeError> {
            let states = match v.get("states") {
                Some(list) => list
                    .as_array()
                    .ok_or(DecodeError::Field("pick states"))?
                    .iter()
                    .map(&state)
                    .collect::<Result<Vec<_>, _>>()?,
                None => vec![state(&v["state"])?],
            };
            if states.is_empty() {
                return Err(DecodeError::Field("pick states"));
            }
            let nbt = match v.get("nbt").map(json_to_nbt) {
                Some(Tag::Compound(c)) => Some(c),
                Some(_) => return Err(DecodeError::Field("pick nbt")),
                None => None,
            };
            let text = |v: &Value| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or(DecodeError::Field("pick list"))
            };
            let lists = v["lists"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|l| {
                    Ok(PickedList {
                        key: text(&l["key"])?,
                        count: l["count"].as_u64().ok_or(DecodeError::Field("pick list"))? as usize,
                        values: l["values"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|e| Ok((below(e)?, text(&e["value"])?)))
                            .collect::<Result<_, DecodeError>>()?,
                        default: text(&l["default"])?,
                    })
                })
                .collect::<Result<_, DecodeError>>()?;
            Ok(Outcome { states, nbt, lists })
        };
        Ok(Self {
            block: value["block"]
                .as_str()
                .map(|b| super::super::tags::identifier(b).map(|i| i.to_string()))
                .ok_or(DecodeError::Field("pick block"))??,
            picks: value["picks"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| Ok((below(p)?, outcome(p)?)))
                .collect::<Result<_, DecodeError>>()?,
            default: value.get("default").map(outcome).transpose()?,
            redraw: value["redraw"].as_bool().unwrap_or(false),
            keep_properties: value["keep_properties"].as_bool().unwrap_or(false),
        })
    }
    fn apply(&self, info: BlockInfo, random: &mut dyn ProcessorRandom) -> BlockInfo {
        let outcome = match self.picks.is_empty() {
            true => self.default.as_ref(),
            false if self.redraw => self
                .picks
                .iter()
                .find(|(b, _)| random.float() < *b)
                .map(|(_, o)| o)
                .or(self.default.as_ref()),
            false => {
                let f = random.float();
                self.picks
                    .iter()
                    .find(|(b, _)| f < *b)
                    .map(|(_, o)| o)
                    .or(self.default.as_ref())
            }
        };
        let Some(outcome) = outcome else {
            return info;
        };
        let state = match outcome.states.as_slice() {
            [one] => *one,
            many => many[random.int(many.len() as i32) as usize],
        };
        let mut nbt = outcome.nbt.clone().map(Box::new).or(info.nbt);
        for list in &outcome.lists {
            let values = (0..list.count)
                .map(|_| {
                    let f = random.float();
                    let v = list
                        .values
                        .iter()
                        .find(|(b, _)| f < *b)
                        .map_or(&list.default, |(_, v)| v);
                    Tag::String(v.as_str().into())
                })
                .collect();
            nbt.get_or_insert_with(Default::default)
                .insert(list.key.as_str().into(), Tag::List(values));
        }
        BlockInfo {
            pos: info.pos,
            state,
            nbt,
        }
    }
}

/// `sunrise:count_cap`, which `sunrise-port-pack` writes for Better
/// Dungeons' chest processor (Java in the mod): of the `blocks` one
/// placement meets, the first `min` stay, the rest each stay when
/// `settings.getRandom(pos).nextFloat() <= chance` until `max` have, and the
/// others become `replacement`. The count is the placement's (the mod's
/// DungeonContext, set up per StructureTemplate.placeInWorld: one chunk of
/// the piece), shared by every such processor in it.
#[derive(Clone, Debug)]
pub struct CountCap {
    blocks: BTreeSet<String>,
    min: u32,
    max: u32,
    chance: f32,
    replacement: BlockStateId,
}
impl CountCap {
    fn decode(
        value: &Value,
        state: impl Fn(&Value) -> Result<BlockStateId, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let count = |key: &'static str| {
            value[key]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or(DecodeError::Field(key))
        };
        Ok(Self {
            blocks: value["blocks"]
                .as_array()
                .ok_or(DecodeError::Field("blocks"))?
                .iter()
                .map(|b| {
                    let b = b.as_str().ok_or(DecodeError::Field("blocks"))?;
                    Ok(super::super::tags::identifier(b)?.to_string())
                })
                .collect::<Result<_, DecodeError>>()?,
            min: count("min")?,
            max: count("max")?,
            chance: float(value, "chance")?,
            replacement: state(&value["replacement"])?,
        })
    }
    /// Whether a counted block stays, counting it if so.
    fn keeps(&self, counted: &mut u32, roll: impl FnOnce() -> f32) -> bool {
        let keep = *counted < self.min || (*counted < self.max && roll() <= self.chance);
        *counted += u32::from(keep);
        keep
    }
}

/// `sunrise:spawner_mobs`, which `sunrise-port-pack` writes for the spawner
/// randomizing processors of Moog's Structure Lib and Repurposed Structures
/// (Java code in the mods): a spawner gets one mob of a weighted list,
/// picked with `settings.getRandom(pos).nextFloat()` as theirs do, and the
/// spawner NBT they write. With no mob to pick it becomes `replacement`.
#[derive(Clone, Debug)]
pub struct SpawnerMobs {
    /// Each mob's entity compound (its `id` and any NBT) and weight.
    mobs: Vec<(Compound, f64)>,
    /// [`SPAWNER_FIELDS`]' values.
    fields: [i16; 7],
    /// `custom_spawn_rules`, in `SpawnData` and the potential alike.
    rules: Option<Compound>,
    replacement: BlockStateId,
}
impl SpawnerMobs {
    fn decode(
        value: &Value,
        replacement: impl FnOnce(Option<&Value>) -> Result<BlockStateId, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let mobs = value["mobs"]
            .as_array()
            .ok_or(DecodeError::Field("spawner_mobs mobs"))?
            .iter()
            .map(
                |m| match (json_to_nbt(&m["entity"]), m["weight"].as_f64()) {
                    (Tag::Compound(entity), Some(weight))
                        if entity.contains_key(&JavaString::from("id")) =>
                    {
                        Ok((entity, weight))
                    }
                    _ => Err(DecodeError::Field("spawner_mobs mob")),
                },
            )
            .collect::<Result<_, _>>()?;
        let mut fields = [0; 7];
        for (field, (key, _, default)) in fields.iter_mut().zip(SPAWNER_FIELDS) {
            // The mods' codecs read ints; the spawner saves them as shorts.
            *field = value
                .get(key)
                .map_or(Some(default), |v| v.as_i64().map(|v| v as i16))
                .ok_or(DecodeError::Field("spawner_mobs field"))?;
        }
        let rules = match value.get("custom_spawn_rules").map(json_to_nbt) {
            None => None,
            Some(Tag::Compound(c)) => Some(c),
            Some(_) => return Err(DecodeError::Field("spawner_mobs custom_spawn_rules")),
        };
        Ok(Self {
            mobs,
            fields,
            rules,
            replacement: replacement(value.get("replacement"))?,
        })
    }
    /// The mob for a spawner, drawing from `random` only when there is one
    /// to pick (pickWeightedRandom; Repurposed Structures' getSpawnerMob,
    /// whose float product can round differently only on a boundary).
    fn pick(&self, random: impl FnOnce() -> f32) -> Option<&Compound> {
        let total: f64 = self.mobs.iter().map(|m| m.1).sum();
        if self.mobs.is_empty() || total <= 0. {
            return None;
        }
        let mut pick = f64::from(random()) * total;
        for (entity, weight) in &self.mobs {
            pick -= weight;
            if pick <= 0. {
                return Some(entity);
            }
        }
        self.mobs.last().map(|m| &m.0)
    }
    /// The spawner's NBT with `entity` as its only potential and next spawn.
    fn nbt(&self, entity: &Compound) -> Compound {
        let mut data = Compound::new();
        data.insert("entity".into(), Tag::Compound(entity.clone()));
        if let Some(rules) = &self.rules {
            data.insert("custom_spawn_rules".into(), Tag::Compound(rules.clone()));
        }
        let mut potential = Compound::new();
        potential.insert("data".into(), Tag::Compound(data.clone()));
        potential.insert("weight".into(), Tag::Int(1));
        let mut nbt: Compound = SPAWNER_FIELDS
            .iter()
            .zip(self.fields)
            .map(|((_, key, _), v)| (JavaString::from(*key), Tag::Short(v)))
            .collect();
        nbt.insert(
            "SpawnPotentials".into(),
            Tag::List(vec![Tag::Compound(potential)]),
        );
        nbt.insert("SpawnData".into(), Tag::Compound(data));
        nbt
    }
}

""",
                "",
            ),
            (
                """            "sunrise:spawner_mobs" => {
                Self::SpawnerMobs(Box::new(SpawnerMobs::decode(value, |v| match v {
                    Some(v) => state(v),
                    None => Ok(resources.registry.default_state("minecraft:air").unwrap()),
                })?))
            }
            "sunrise:pick" => Self::Pick(Box::new(Pick::decode(value, state)?)),
            "lithostitched:block_swap" => Self::BlockSwap(
                value["blocks"]
                    .as_object()
                    .ok_or(DecodeError::Field("blocks"))?
                    .iter()
                    .map(|(from, to)| {
                        let to = to.as_str().ok_or(DecodeError::Field("blocks"))?;
                        Ok((
                            super::super::tags::identifier(from)?.to_string(),
                            super::super::tags::identifier(to)?.to_string(),
                        ))
                    })
                    .collect::<Result<_, DecodeError>>()?,
            ),
            "sunrise:count_cap" => Self::CountCap(Box::new(CountCap::decode(value, state)?)),
            "lithostitched:apply_random" => Self::ApplyRandom(Box::new(
                litho::ApplyRandom::decode(value, resources, bindings)?,
            )),
            "lithostitched:set_block" => {
                Self::SetBlock(Box::new(litho::SetBlock::decode(value, state)?))
            }
            "lithostitched:reference" => {
                Self::Reference(litho::reference(value, resources, bindings)?)
            }
            "lithostitched:discard_input" => Self::DiscardInput,
            "sunrise:pillar" => Self::Pillar(Box::new(pillars::Pillar::decode(
                value, resources, bindings, state,
            )?)),
            "sunrise:bottom_pillar" => Self::BottomPillar(Box::new(pillars::BottomPillar::decode(
                value, resources, bindings,
            )?)),
            "sunrise:close_off_fluid_sources" => Self::CloseOffFluidSources(Box::new(
                pillars::CloseOffFluidSources::decode(value, resources, bindings)?,
            )),
            "sunrise:close_off_air_sources" => Self::CloseOffAirSources(Box::new(
                pillars::CloseOffAirSources::decode(value, resources, bindings)?,
            )),
            "sunrise:yung_pillar" => Self::YungPillar(Box::new(pillars::YungPillar::decode(
                value, bindings, state,
            )?)),
            "sunrise:stair_pillar" => Self::StairPillar(Box::new(pillars::StairPillar::decode(
                value, resources, bindings,
            )?)),
            "sunrise:flood_air" => {
                Self::FloodAir(Box::new(sea_level::FloodAir::decode(resources.registry)?))
            }
            "sunrise:waterlog" => Self::Waterlog,
            "sunrise:waterlog_in_world_water" => Self::WaterlogInWorldWater,
            "sunrise:mineshaft_sky_view" => Self::MineshaftSkyView,
            "sunrise:keep_world_block" => {
                Self::KeepWorldBlock(Box::new(sea_level::KeepWorldBlock::decode(value)?))
            }
            "lithostitched:condition" => Self::Condition(Box::new(litho::Conditional::decode(
                value, resources, bindings,
            )?)),
            other => {""",
                "            other => {",
            ),
            (
                """    /// The blocks `sunrise:count_cap` kept in this placement.
    counted: u32,
}""",
                "}",
            ),
            (
                """            finalize_entities: false,
            counted: 0,
        }""",
                """            finalize_entities: false,
        }""",
            ),
            (
                """/// Block.withPropertiesOf: `block`'s default state with each property of
/// `from` it has too (by name, when the value is one of its own). None for
/// a block the registry does not know (the mod keeps the block then).
fn with_properties_of(
    registry: &crate::registry::BlockRegistry,
    block: &str,
    from: BlockStateId,
) -> Option<BlockStateId> {
    let default = registry.default_state(block)?;
    let mut state = registry.state(default)?.clone();
    for (key, value) in &registry.state(from)?.properties {
        if let Some(own) = state.properties.get(key)
            && own != value
        {
            let mut changed = state.clone();
            changed.properties.insert(key.clone(), value.clone());
            if registry.id(&changed).is_some() {
                state = changed;
            }
        }
    }
    registry.id(&state)
}

""",
                "",
            ),
            (
                """        Processor::Capped { .. } => Some(info),
        Processor::BlockSwap(blocks) => {
            let swapped = blocks
                .get(name(info.state))
                .and_then(|to| with_properties_of(registry, to, info.state));
            Some(match swapped {
                Some(state) => BlockInfo { state, ..info },
                None => info,
            })
        }
        Processor::CountCap(cap) => match cap.blocks.contains(name(info.state)) {
            true => {
                let pos = info.pos;
                let mut counted = settings.counted;
                let keep = cap.keeps(&mut counted, || settings.with_random(pos, |r| r.float()));
                settings.counted = counted;
                Some(match keep {
                    true => info,
                    false => BlockInfo {
                        pos,
                        state: cap.replacement,
                        nbt: None,
                    },
                })
            }
            false => Some(info),
        },
        Processor::ApplyRandom(apply) => {
            let mut random = apply.draws.random(world.world_seed()?, position, info.pos);
            match apply.pick(&mut random) {
                Some(list) => {
                    return process_list(
                        list, world, resources, position, reference, original, info, settings,
                    );
                }
                None => Some(info),
            }
        }
        Processor::Reference(lists) => {
            let mut current = info;
            for list in lists {
                match process_list(
                    list, world, resources, position, reference, original, current, settings,
                )? {
                    Some(next) => current = next,
                    None => return Ok(None),
                }
            }
            Some(current)
        }
        Processor::DiscardInput => None,
        Processor::SetBlock(set) => {
            let mut random = set.draws.random(world.world_seed()?, position, info.pos);
            let state = set.state(&mut random);
            let state = match set.preserve {
                true => registry
                    .state(state)
                    .and_then(|s| with_properties_of(registry, &s.name, info.state))
                    .unwrap_or(state),
                false => state,
            };
            let nbt = set
                .modifier
                .apply(info.nbt.as_deref(), &mut random)
                .map(Box::new);
            Some(BlockInfo { state, nbt, ..info })
        }
        Processor::Condition(c) => {
            let mut random = c.draws.random(world.world_seed()?, position, info.pos);
            let location = world.block_state(info.pos)?;
            let passed = c
                .condition
                .test(info.state, location, registry, &mut random);
            let list = if passed { &c.then } else { &c.otherwise };
            return process_list(
                list, world, resources, position, reference, original, info, settings,
            );
        }
        Processor::Pillar(p) => return p.process(world, resources, position, info, settings),
        Processor::BottomPillar(p) => {
            return p.process(world, resources, position, relative, info, settings);
        }
        Processor::CloseOffFluidSources(p) => return p.process(world, info, settings),
        Processor::CloseOffAirSources(p) => return p.process(world, info, settings),
        Processor::YungPillar(p) => return p.process(world, resources, info, settings),
        Processor::StairPillar(p) => return p.process(world, resources, info, settings),
        Processor::FloodAir(p) => return p.process(world, info),
        Processor::Waterlog => {
            return sea_level::Waterlog::process(world, resources.registry, info, settings);
        }
        Processor::KeepWorldBlock(p) => {
            return p.process(world, resources.registry, info, settings);
        }
        Processor::WaterlogInWorldWater => {
            return sea_level::WaterlogInWorldWater::process(
                world,
                resources.registry,
                info,
                settings,
            );
        }
        Processor::MineshaftSkyView => {
            return sea_level::MineshaftSkyView::process(world, resources.registry, info);
        }
        Processor::Pick(pick) => match name(info.state) == pick.block {
            true => {
                let mut out = settings.with_random(info.pos, |r| pick.apply(info.clone(), r));
                if pick.keep_properties
                    && let Some(kept) = registry
                        .state(out.state)
                        .and_then(|s| with_properties_of(registry, &s.name, info.state))
                {
                    out.state = kept;
                }
                Some(out)
            }
            false => Some(info),
        },
        Processor::SpawnerMobs(spawner) => {
            if name(info.state) != "minecraft:spawner" {
                return Ok(Some(info));
            }
            let pos = info.pos;
            let entity = spawner.pick(|| settings.with_random(pos, |r| r.float()));
            Some(match entity {
                Some(entity) => BlockInfo {
                    nbt: Some(Box::new(spawner.nbt(entity))),
                    ..info
                },
                None => BlockInfo {
                    pos,
                    state: spawner.replacement,
                    nbt: None,
                },
            })
        }
        Processor::Mod(mod_processor) => {""",
                """        Processor::Capped { .. } => Some(info),
        Processor::Mod(mod_processor) => {""",
            ),
            (
                "    settings.counted = 0;\n",
                "",
            ),
            (
                """    fn spawner(value: Value) -> SpawnerMobs {
        SpawnerMobs::decode(&value, |v| {
            assert_eq!(v, Some(&json!({"id": "minecraft:cobblestone"})));
            Ok(BlockStateId(7))
        })
        .unwrap()
    }
    fn id(entity: Option<&Compound>) -> Option<String> {
        match entity?.get(&JavaString::from("id")) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        }
    }

    #[test]
    fn spawner_mobs_pick_by_weight() {
        let s = spawner(json!({
            "mobs": [
                {"entity": {"id": "minecraft:zombie"}, "weight": 1},
                {"entity": {"id": "minecraft:husk"}, "weight": 3}],
            "replacement": {"id": "minecraft:cobblestone"}}));
        // pick = nextFloat * 4, less each weight in turn, until <= 0.
        for (r, mob) in [
            (0., "zombie"),
            (0.25, "zombie"),
            (0.2501, "husk"),
            (0.999, "husk"),
        ] {
            assert_eq!(id(s.pick(|| r)), Some(format!("minecraft:{mob}")), "{r}");
        }
        let empty = spawner(json!({"mobs": [], "replacement": {"id": "minecraft:cobblestone"}}));
        assert_eq!(empty.pick(|| unreachable!()), None);
        assert_eq!(empty.replacement, BlockStateId(7));
    }

    #[test]
    fn spawner_mobs_write_the_mods_nbt() {
        let s = spawner(json!({
            "mobs": [{"entity": {"id": "minecraft:skeleton", "Health": 30}, "weight": 2}],
            "delay": 10, "spawn_count": 2,
            "custom_spawn_rules": {"block_light_limit": {"min_inclusive": 0, "max_inclusive": 7}},
            "replacement": {"id": "minecraft:cobblestone"}}));
        let nbt = s.nbt(s.pick(|| 0.5).unwrap());
        let short = |k: &str| nbt.get(&JavaString::from(k)).cloned();
        for (k, v) in [
            ("Delay", 10),
            ("MinSpawnDelay", 200),
            ("MaxSpawnDelay", 800),
            ("SpawnCount", 2),
            ("MaxNearbyEntities", 6),
            ("RequiredPlayerRange", 16),
            ("SpawnRange", 4),
        ] {
            assert_eq!(short(k), Some(Tag::Short(v)), "{k}");
        }
        let Some(Tag::Compound(data)) = short("SpawnData") else {
            panic!("SpawnData")
        };
        assert!(data.contains_key(&JavaString::from("custom_spawn_rules")));
        let Some(Tag::Compound(entity)) = data.get(&JavaString::from("entity")) else {
            panic!("entity")
        };
        assert_eq!(entity.get(&JavaString::from("Health")), Some(&Tag::Int(30)));
        let Some(Tag::List(potentials)) = short("SpawnPotentials") else {
            panic!("SpawnPotentials")
        };
        let Tag::Compound(potential) = &potentials[0] else {
            panic!("potential")
        };
        assert_eq!(
            potential.get(&JavaString::from("weight")),
            Some(&Tag::Int(1))
        );
        assert_eq!(
            potential.get(&JavaString::from("data")),
            Some(&Tag::Compound(data.clone()))
        );
    }

    #[test]
    fn spawner_mobs_refuse_a_mob_without_an_id() {
        let bad = json!({"mobs": [{"entity": {}, "weight": 1}]});
        assert!(SpawnerMobs::decode(&bad, |_| Ok(BlockStateId(0))).is_err());
    }

    #[test]
    fn repurposed_position_rules() {
        let decode = |v: Value| PosRule::decode(Some(&v));
        // Over 3..4 blocks from the template's origin along y, 0 to 1.
        let rule = decode(
            json!({"predicate_type": "sunrise:piece_origin_axis_aligned_linear_pos",
            "axis": "y", "min_chance": 0, "max_chance": 1, "min_dist": 3, "max_dist": 4}),
        )
        .unwrap();
        let mut random = LegacyRandom::new(1);
        let world = [100, 64, -100];
        // At or under min_dist the chance is 0, a roll of 0 aside; past max_dist it is 1.
        assert!(!rule.test([9, 2, 9], world, world, &mut random));
        assert!(rule.test([0, -5, 0], world, world, &mut random));
        // The world position and the piece's do not count.
        assert!(rule.test([0, 4, 0], [0, 0, 0], [50, 50, 50], &mut random));
        // Repurposed refuses an empty distance range and an unknown axis.
        for bad in [
            json!({"predicate_type": "sunrise:piece_origin_axis_aligned_linear_pos"}),
            json!({"predicate_type": "sunrise:piece_origin_axis_aligned_linear_pos",
                "min_dist": 0, "max_dist": 1, "axis": "w"}),
            json!({"predicate_type": "sunrise:y_value", "min_y_value": 5, "max_y_value": 4}),
            json!({"predicate_type": "sunrise:y_value", "min_y_value": 5}),
        ] {
            assert!(decode(bad.clone()).is_err(), "{bad}");
        }
        let y = decode(
            json!({"predicate_type": "sunrise:y_value", "min_y_value": 0, "max_y_value": 31}),
        )
        .unwrap();
        assert!(y.test([0; 3], [7, 31, 7], [0; 3], &mut random));
        assert!(y.test([0, 99, 0], [7, 0, 7], [0; 3], &mut random));
        assert!(!y.test([0; 3], [7, 32, 7], [0; 3], &mut random));
        assert!(!y.test([0; 3], [7, -1, 7], [0; 3], &mut random));
    }

""",
                "",
            ),
            (
                """        let merged = edited(Some(&vault), &[], &data);
        assert_eq!(
            merged,
            nbt(json!({"id": "minecraft:vault",
            "config": {"loot_table": "x:new", "key_item": {"id": "minecraft:trial_key"}},
            "server_data": {"state_updating_resumes_at": 533162741}}))
        );
        let Ok(Modifier::Edit { remove, data }) = Modifier::decode(Some(&json!({
            "type": "sunrise:edit", "remove": ["server_data"],
            "data": {"config": {"loot_table": "x:new"}}})))
        else {
            panic!("sunrise:edit")
        };
        assert_eq!(
            edited(Some(&vault), &remove, &data),
            nbt(json!({"id": "minecraft:vault",
                "config": {"loot_table": "x:new", "key_item": {"id": "minecraft:trial_key"}}}))
        );
        assert_eq!(
            edited(None, &remove, &data),
            nbt(json!({"config": {"loot_table": "x:new"}}))
        );
        let bad = json!({"type": "sunrise:edit", "remove": [1], "data": {}});
        assert!(Modifier::decode(Some(&bad)).is_err());
    }

    /// Scripted draws: floats in turn, then ints.
    struct Draws(Vec<f32>, Vec<i32>);
    impl ProcessorRandom for Draws {
        fn float(&mut self) -> f32 {
            self.0.remove(0)
        }
        fn int(&mut self, bound: i32) -> i32 {
            let i = self.1.remove(0);
            assert!(i < bound);
            i
        }
        fn long(&mut self) -> i64 {
            unreachable!()
        }
        fn int_unbounded(&mut self) -> i32 {
            unreachable!()
        }
        fn double(&mut self) -> f64 {
            unreachable!()
        }
        fn boolean(&mut self) -> bool {
            unreachable!()
        }
        fn gaussian(&mut self) -> f64 {
            unreachable!()
        }
    }

    #[test]
    fn redrawn_picks_draw_a_float_each() {
        // Better Ocean Monuments' copper: < 0.1 exposed, else a new float
        // < 0.3 weathered, else oxidized.
        let id = |v: &Value| Ok(BlockStateId(v["Name"].as_str().unwrap().len() as u32));
        let pick = Pick::decode(
            &json!({"block": "oxidized_copper", "redraw": true, "keep_properties": true,
                "picks": [{"below": 0.1, "state": {"Name": "a"}},
                          {"below": 0.3, "state": {"Name": "bb"}}],
                "default": {"state": {"Name": "ccc"}}}),
            id,
        )
        .unwrap();
        assert!(pick.keep_properties);
        let run = |f: Vec<f32>| {
            pick.apply(
                BlockInfo {
                    pos: [0; 3],
                    state: BlockStateId(0),
                    nbt: None,
                },
                &mut Draws(f, vec![]),
            )
            .state
        };
        assert_eq!(run(vec![0.05]), BlockStateId(1));
        // One float would give weathered at 0.2; the second draw decides.
        assert_eq!(run(vec![0.2, 0.5]), BlockStateId(3));
        assert_eq!(run(vec![0.2, 0.25]), BlockStateId(2));
    }

    #[test]
    fn picks_follow_yungs_cascade() {
        // States by name: sand 1, sandstone 2, the four stairs 10..13.
        let id = |v: &Value| {
            Ok(BlockStateId(match v["Name"].as_str() {
                Some("minecraft:sand") => 1,
                Some("minecraft:sandstone") => 2,
                Some("minecraft:suspicious_sand") => 3,
                _ => {
                    10 + ["north", "east", "south", "west"]
                        .iter()
                        .position(|f| v["Properties"]["facing"] == *f)
                        .unwrap_or(9) as u32
                }
            }))
        };
        let stairs: Vec<Value> = ["north", "east", "south", "west"]
            .iter()
            .map(|f| json!({"Name": "minecraft:sandstone_stairs", "Properties": {"facing": f}}))
            .collect();
        let pick = Pick::decode(
            &json!({"block": "red_stained_glass", "picks": [
                {"below": 0.01, "state": {"Name": "minecraft:suspicious_sand"},
                 "nbt": {"LootTable": "minecraft:archaeology/desert_pyramid"}},
                {"below": 0.3, "state": {"Name": "minecraft:sand"}},
                {"below": 0.75, "state": {"Name": "minecraft:sandstone"}}],
                "default": {"states": stairs}}),
            id,
        )
        .unwrap();
        assert_eq!(pick.block, "minecraft:red_stained_glass");
        let glass = || BlockInfo {
            pos: [1, 2, 3],
            state: BlockStateId(0),
            nbt: Some(Box::default()),
        };
        let run = |f: f32, i: Vec<i32>| pick.apply(glass(), &mut Draws(vec![f], i));
        let sus = run(0.005, vec![]);
        assert_eq!(sus.state, BlockStateId(3));
        assert!(
            sus.nbt
                .unwrap()
                .contains_key(&JavaString::from("LootTable"))
        );
        assert_eq!(run(0.01, vec![]).state, BlockStateId(1));
        assert_eq!(run(0.2999, vec![]).nbt, Some(Box::default()));
        assert_eq!(run(0.3, vec![]).state, BlockStateId(2));
        // The stairs' facing: Plane.HORIZONTAL[nextInt(4)] after the pick.
        assert_eq!(run(0.75, vec![2]).state, BlockStateId(12));
        assert_eq!(run(0.99, vec![1]).state, BlockStateId(11));

        let pot = Pick::decode(
            &json!({"block": "minecraft:decorated_pot", "default": {
                "state": {"Name": "minecraft:sand"},
                "lists": [{"key": "sherds", "count": 4, "default": "minecraft:brick",
                           "values": [{"below": 0.05, "value": "minecraft:archer_pottery_sherd"},
                                      {"below": 0.1, "value": "minecraft:miner_pottery_sherd"}]}]}}),
            id,
        )
        .unwrap();
        // No picks: no float for them; one float per sherd.
        let out = pot.apply(
            BlockInfo {
                nbt: None,
                ..glass()
            },
            &mut Draws(vec![0.5, 0.01, 0.07, 0.1], vec![]),
        );
        let Some(Tag::List(sherds)) = out.nbt.unwrap().get(&JavaString::from("sherds")).cloned()
        else {
            panic!("sherds")
        };
        let names: Vec<String> = sherds
            .iter()
            .map(|t| match t {
                Tag::String(s) => s.to_string_lossy(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(
            names,
            [
                "minecraft:brick",
                "minecraft:archer_pottery_sherd",
                "minecraft:miner_pottery_sherd",
                "minecraft:brick"
            ]
        );
        assert!(Pick::decode(&json!({"block": "a", "default": {"states": []}}), id).is_err());
    }

    #[test]
    fn count_caps_keep_the_first_then_some() {
        let cap = CountCap::decode(
            &json!({"blocks": ["chest", "minecraft:trapped_chest"], "min": 1, "max": 2,
                    "chance": 0.2, "replacement": {"id": "minecraft:cave_air"}}),
            |_| Ok(BlockStateId(5)),
        )
        .unwrap();
        assert!(cap.blocks.contains("minecraft:chest"));
        let mut counted = 0;
        // The first stays without a roll.
        assert!(cap.keeps(&mut counted, || unreachable!()));
        // Then each stays on nextFloat <= 0.2, until two have.
        assert!(!cap.keeps(&mut counted, || 0.21));
        assert!(cap.keeps(&mut counted, || 0.2));
        assert!(!cap.keeps(&mut counted, || unreachable!()));
        assert_eq!(counted, 2);
        assert!(
            CountCap::decode(&json!({"blocks": [], "min": -1}), |_| Ok(BlockStateId(5))).is_err()
        );
    }

    #[test]
    fn block_swaps_keep_the_properties_both_blocks_have() {
        let row = |name: &str, props: Value| json!([name, false, false, "", {"id": name, "properties": props}]);
        let facts = json!({"defaults": {"a:log": 1, "a:stairs": 4, "a:stone": 6},
            "states": [
                row("a:log", json!({"axis": "x"})), row("a:log", json!({"axis": "y"})),
                row("a:stairs", json!({"facing": "east", "half": "top"})),
                row("a:stairs", json!({"facing": "east", "half": "bottom"})),
                row("a:stairs", json!({"facing": "north", "half": "bottom"})),
                row("a:stairs", json!({"facing": "north", "half": "top"})),
                ["a:stone", false, false, "", "a:stone"]]});
        let registry =
            crate::registry::BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap();
        // facing and half carried over; the default where `from` has none.
        assert_eq!(
            with_properties_of(&registry, "a:stairs", BlockStateId(2)),
            Some(BlockStateId(2))
        );
        assert_eq!(
            with_properties_of(&registry, "a:stairs", BlockStateId(0)),
            Some(BlockStateId(4))
        );
        assert_eq!(
            with_properties_of(&registry, "a:stone", BlockStateId(5)),
            Some(BlockStateId(6))
        );
        assert_eq!(
            with_properties_of(&registry, "a:missing", BlockStateId(0)),
            None
        );
    }""",
                """        let merged = append_static(Some(&vault), &data);
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
    }""",
            ),
        ],
    )


def edit_port_pack() -> None:
    litho_path = ROOT / "crates/sunrise-port-pack/src/lithostitched.rs"
    replace_exact(
        litho_path,
        [
            (
                "use crate::{processors::ProcessorData, worldgen::Upgrade};\n",
                "use crate::worldgen::Upgrade;\n",
            ),
            (
                """const MODULAR_COMPAT: &str = "ctov:modular_compat";
/// The processor types the game reads.
const PROCESSORS: [&str; 6] = [
    "lithostitched:block_swap",
    "lithostitched:reference",
    "lithostitched:apply_random",
    "lithostitched:condition",
    "lithostitched:set_block",
    "lithostitched:discard_input",
];

""",
                "",
            ),
            (
                """/// Whether the game reads a Lithostitched processor as it is: one of its
/// types, with no processor list named by tag (the game has no processor
/// list tags) and a state provider it runs.
pub(crate) fn game_reads(p: &Value) -> bool {
    let Some(kind) = p["processor_type"]
        .as_str()
        .filter(|t| PROCESSORS.contains(t))
    else {
        return false;
    };
    let tag = |v: &Value| v.as_str().is_some_and(|s| s.starts_with('#'));
    match kind {
        "lithostitched:reference" => match &p["processor_lists"] {
            Value::Array(a) => !a.iter().any(tag),
            other => other.is_string() && !tag(other),
        },
        "lithostitched:apply_random" => p["processor_lists"]
            .as_array()
            .is_some_and(|a| a.iter().all(|e| !tag(e) && !tag(&e["data"]))),
        "lithostitched:set_block" => matches!(
            p["state_provider"]["type"].as_str(),
            Some("minecraft:simple_state_provider" | "minecraft:weighted_state_provider")
        ),
        "lithostitched:block_swap" => p["blocks"].is_object(),
        "lithostitched:condition" => !p["if_true"].is_null(),
        _ => true,
    }
}

/// The processor lists inside a Lithostitched processor, for the general
/// code that walks lists (`then`, `else`, `apply_random`'s inline data).
pub(crate) fn inner_lists(p: &mut Value) -> Vec<&mut Value> {
    match p["processor_type"].as_str() {
        Some("lithostitched:apply_random") => (p["processor_lists"].as_array_mut().into_iter())
            .flatten()
            .filter_map(|e| e.get_mut("data").filter(|d| !d.is_string()))
            .collect(),
        Some("lithostitched:condition") => (p.as_object_mut().into_iter())
            .flat_map(|o| o.iter_mut())
            .filter(|(k, _)| *k == "then" || *k == "else")
            .map(|(_, v)| v)
            .collect(),
        _ => Vec::new(),
    }
}

/// A Lithostitched processor with the processors nested in it carried
/// (`Some`, when one of them changed); `None` for any other processor.
pub(crate) fn processor(
    kind: &str,
    p: &Value,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> Option<Vec<Value>> {
    if !PROCESSORS.contains(&kind) {
        return None;
    }
    let mut out = p.clone();
    let mut changed = false;
    for list in inner_lists(&mut out) {
        // One processor alone is a list of one.
        if list.get("processor_type").is_some() {
            *list = Value::Array(vec![list.take()]);
            changed = true;
        }
        if let Some(processors) = crate::processors::processors(list) {
            let (inner, inner_changed) =
                crate::processors::carry_all(std::mem::take(processors), data, carried);
            *processors = inner;
            changed |= inner_changed;
        }
    }
    changed.then(|| vec![out])
}

/// The list with CTOV's modular compat processors written as the game's
/// `reference` (its mod is ported along) or taken out; whether any was.
pub(crate) fn expand_references(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    let mut changed = false;
    let mut out = Vec::with_capacity(list.len());
    for p in list {
        if p["processor_type"] != MODULAR_COMPAT {
            out.push(p);
            continue;
        }
        changed = true;
        count(carried, MODULAR_COMPAT);
        let loaded = p["modid"]
            .as_str()
            .is_some_and(|m| data.compat_loaded.contains(m));
        if loaded {
            out.push(json!({"processor_type": "lithostitched:reference",
                "processor_lists": p["reference"].clone()}));
        }
    }
    (out, changed)
}

/// Notes which mods CTOV's modular compat processors name are `loaded`.
pub(crate) fn mark_loaded(data: &mut ProcessorData, loaded: &dyn Fn(&str) -> bool) {
    fn visit(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                if o.get("processor_type").and_then(Value::as_str) == Some(MODULAR_COMPAT)
                    && let Some(m) = o.get("modid").and_then(Value::as_str)
                {
                    out.push(m.to_owned());
                }
                o.values().for_each(|v| visit(v, out));
            }
            Value::Array(a) => a.iter().for_each(|v| visit(v, out)),
            _ => {}
        }
    }
    let mut named = Vec::new();
    data.lists.values().for_each(|l| visit(l, &mut named));
    data.compat_loaded
        .extend(named.into_iter().filter(|m| loaded(m)));
}

""",
                "",
            ),
            (
                "    use crate::worldgen::upgrade as upgrade_file;\n",
                "",
            ),
            (
                """    /// A CTOV list (spec-ctov §4): references, a condition with a single
    /// processor and a nested condition, a random list holding the compat
    /// hook, and processors the game cannot read.
    #[test]
    fn processors_stay_lithostitcheds_with_their_insides_carried() {
        let hook = json!({"processor_type": MODULAR_COMPAT, "modid": "natures_spirit",
            "reference": "ctov:village/mesa/house_wisteria"});
        let list = json!({"processors": [
            {"processor_type": "lithostitched:reference", "processor_lists": ["ctov:village/mesa/house"]},
            {"processor_type": "lithostitched:condition", "random_mode": "per_block",
                "if_true": {"type": "lithostitched:matching_blocks", "blocks": "minecraft:water",
                    "match_type": "location"},
                "then": {"processor_type": "lithostitched:set_block", "state_provider": {
                    "type": "minecraft:simple_state_provider", "state": {"Name": "minecraft:spruce_planks"}}},
                "else": [{"processor_type": "somemod:java_processor"}]},
            {"processor_type": "lithostitched:apply_random", "mode": "per_piece", "processor_lists": [
                {"weight": 1, "data": "minecraft:empty"},
                {"weight": 1, "data": {"processors": [hook]}}]},
            {"processor_type": "lithostitched:reference", "processor_lists": "#ctov:palettes"},
            {"processor_type": "lithostitched:set_block", "state_provider": {
                "type": "minecraft:noise_provider"}}]});
        for (loaded, wisteria) in [
            (false, json!([])),
            (
                true,
                json!([{"processor_type":
            "lithostitched:reference", "processor_lists": "ctov:village/mesa/house_wisteria"}]),
            ),
        ] {
            let mut data = ProcessorData::default();
            data.lists
                .insert("ctov:village/mesa/house".into(), list.clone());
            mark_loaded(&mut data, &|m| loaded && m == "natures_spirit");
            let mut v = list.clone();
            let mut report = Upgrade::default();
            crate::processors::carry(
                "processor_list",
                &mut v,
                &data,
                &mut report.processors_carried,
            );
            upgrade_file("processor_list", &mut v, &mut report, &BTreeMap::new());
            let p = &v["processors"];
            assert_eq!(p.as_array().unwrap().len(), 3, "{p}");
            assert_eq!(p[0], list["processors"][0]);
            // One processor is a list of one; the Java one is taken out.
            assert_eq!(
                p[1]["then"][0]["state_provider"]["state"]["id"],
                "minecraft:spruce_planks"
            );
            assert_eq!(p[1]["else"], json!([]));
            assert_eq!(p[2]["processor_lists"][0]["data"], "minecraft:empty");
            assert_eq!(p[2]["processor_lists"][1]["data"]["processors"], wisteria);
            assert_eq!(report.processors_carried[MODULAR_COMPAT], 1);
            assert_eq!(
                report.processors_dropped,
                BTreeMap::from([
                    ("lithostitched:reference".into(), 1),
                    ("lithostitched:set_block".into(), 1),
                    ("somemod:java_processor".into(), 1),
                ])
            );
        }
    }
""",
                "",
            ),
        ],
    )

    proc_path = ROOT / "crates/sunrise-port-pack/src/processors.rs"
    replace_exact(
        proc_path,
        [
            ("use crate::lithostitched;\n", ""),
            (
                """    /// Notes which mods its compat processors name are `loaded`.
    pub fn mark_loaded(&mut self, loaded: &dyn Fn(&str) -> bool) {
        lithostitched::mark_loaded(self, loaded);
    }

""",
                "",
            ),
            (
                """/// The processors of one list carried: references written in, then each
/// one with a data form replaced by it. Whether anything changed.
pub(crate) fn carry_all(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    carry_at(list, data, carried, 0)
}

/// [`carry_all`] for a list `depth` pillars deep (see [`pillar`]).
fn carry_at(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
    depth: usize,
) -> (Vec<Value>, bool) {
    let (expanded, mut changed) = lithostitched::expand_references(list, data, carried);
    let mut out = Vec::with_capacity(expanded.len());
    for p in expanded {
        let kind = p["processor_type"].as_str().unwrap_or_default().to_owned();
        // Lithostitched's processors stay theirs, with what they hold carried.
        let replacement = lithostitched::processor(&kind, &p, data, carried).or_else(|| {
            match pillar_type(&kind) {
                Some(to) => Some(vec![pillar(&p, to, data, carried, depth)]),
                None => as_data(&kind, &p, data),
            }
        });
        match replacement {
            Some(replacement) => {
                *carried.entry(kind).or_default() += 1;
                out.extend(replacement);
                changed = true;
            }
            None => out.push(p),
        }
    }
    (out, changed)
}

/// The game's type for the mods' processors that place blocks past their
/// template, which keep their fields (`sunrise-world`'s `pillars.rs`).
fn pillar_type(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "repurposed_structures:pillar_processor" | "moogs_structures:pillar_processor" => {
            "sunrise:pillar"
        }
        "repurposed_structures:bottom_pillar_processor" => "sunrise:bottom_pillar",
        "repurposed_structures:close_off_fluid_sources_processor"
        | "moogs_structures:close_off_fluid_sources_processor" => "sunrise:close_off_fluid_sources",
        "repurposed_structures:close_off_air_sources_processor" => "sunrise:close_off_air_sources",
        _ => return None,
    })
}

/// A pillar processor as the game's `to`. Its `pillar_processor_list`
/// (which runs on each block of the column; Repurposed's often names the
/// list it is in) is written in, carried; the pillars inside that copy lose
/// theirs, as a column's blocks are never the mods' triggers.
fn pillar(
    p: &Value,
    to: &str,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
    depth: usize,
) -> Value {
    let mut out = p.clone();
    out["processor_type"] = to.into();
    let Some(o) = out.as_object_mut() else {
        return out;
    };
    let Some(list) = o.remove("pillar_processor_list") else {
        return out;
    };
    let inner = match &list {
        Value::String(id) if id == "minecraft:empty" => None,
        _ if depth > 0 => None,
        other => referenced(other, data),
    };
    if let Some(inner) = inner {
        let (inner, _) = carry_at(inner, data, carried, depth + 1);
        o.insert("pillar_processor_list".into(), json!({"processors": inner}));
    }
    out
}""",
                """/// The processors of one list carried: each one with a vanilla data form
/// replaced by it. Whether anything changed.
pub(crate) fn carry_all(
    list: Vec<Value>,
    data: &ProcessorData,
    carried: &mut BTreeMap<String, usize>,
) -> (Vec<Value>, bool) {
    let mut changed = false;
    let mut out = Vec::with_capacity(list.len());
    for p in list {
        let kind = p["processor_type"].as_str().unwrap_or_default().to_owned();
        match as_data(&kind, &p, data) {
            Some(replacement) => {
                *carried.entry(kind).or_default() += 1;
                out.extend(replacement);
                changed = true;
            }
            None => out.push(p),
        }
    }
    (out, changed)
}""",
            ),
            (
                """const RULE_TESTS: [(&str, &str); 3] = [
    (
        "repurposed_structures:matter_phase_rule_test",
        "sunrise:matter_phase",
    ),
    (
        "repurposed_structures:piece_origin_axis_aligned_linear_pos_rule_test",
        "sunrise:piece_origin_axis_aligned_linear_pos",
    ),
    (
        "repurposed_structures:y_value_pos_rule_test",
        "sunrise:y_value",
    ),
];""",
                """const RULE_TESTS: [(&str, &str); 1] = [(
    "repurposed_structures:matter_phase_rule_test",
    "sunrise:matter_phase",
)];""",
            ),
            (
                """/// The processors of the lists a reference names: an id, an inline list, or
/// a list of either.
pub(crate) fn referenced(lists: &Value, data: &ProcessorData) -> Option<Vec<Value>> {
    let one = |l: &Value| -> Option<Vec<Value>> {
        let mut list = match l {
            Value::String(id) if !id.starts_with('#') => {
                let id = if id.contains(':') {
                    id.clone()
                } else {
                    format!("minecraft:{id}")
                };
                data.lists.get(&id)?.clone()
            }
            Value::Object(_) | Value::Array(_) => l.clone(),
            _ => return None,
        };
        processors(&mut list).map(std::mem::take)
    };
    match lists {
        // A bare list of processors is one inline list; else a list of lists.
        Value::Array(a) if a.iter().any(|x| x.get("processor_type").is_some()) => Some(a.clone()),
        Value::Array(a) => Some(a.iter().map(one).collect::<Option<Vec<_>>>()?.concat()),
        other => one(other),
    }
}

""",
                "",
            ),
            (
                """        "repurposed_structures:structure_void_processor" => vec![json!({
            "processor_type": "minecraft:block_ignore",
            "blocks": ["minecraft:structure_void"]})],
        "moogs_structures:spawner_randomizing_processor" => {
            let mobs = p["weighted_entities"]
                .as_array()?
                .iter()
                .map(|e| {
                    // The entry's NBT, then its id over it.
                    let mut entity = match &e["nbt"] {
                        Value::Object(o) => o.clone(),
                        _ => serde_json::Map::new(),
                    };
                    entity.insert("id".into(), e["entity"].as_str()?.into());
                    Some(json!({"entity": entity, "weight": e["weight"].as_f64()?}))
                })
                .collect::<Option<Vec<_>>>()?;
            vec![spawner_mobs(p, mobs, None)]
        }
        "repurposed_structures:spawner_randomizing_processor" => {
            // A list the mod does not ship falls back to a vanilla dungeon
            // mob in the mod; left out here.
            let list = data
                .spawners
                .get(p["rs_spawner_resourcelocation"].as_str()?)?;
            let mobs = list
                .iter()
                .map(|(id, weight)| json!({"entity": {"id": id}, "weight": weight}))
                .collect();
            vec![spawner_mobs(p, mobs, repurposed_light(p)?)]
        }
        // Wet where the world holds water, dry where it does not.
        "repurposed_structures:waterlogging_when_replacing_water_processor" => {
            vec![json!({"processor_type": "sunrise:waterlog_in_world_water"})]
        }
        // Its mineshafts: nothing above the ocean floor but grounded rails.
        "repurposed_structures:mineshaft_sky_view_processor" => {
            vec![json!({"processor_type": "sunrise:mineshaft_sky_view"})]
        }
        "moogs_structures:vanilla_loot_swap_processor" => {
            let key = (
                p["modid"].as_str()?.to_owned(),
                p["vanilla_key"].as_str()?.to_owned(),
            );
            if data.replace_vanilla.contains(&key) {
                return None;
            }
            Vec::new()
        }
        "moogs_structures:trial_spawner_randomizing_processor" => {
            // The configs by id: 26.3 reads them from `trial_spawner/`, the
            // folder Moog's loads them from.
            let mut nbt = json!({"normal_config": p["normal_config"].as_str()?});
            if let Some(ominous) = p["ominous_config"].as_str() {
                nbt["ominous_config"] = ominous.into();
            }
            // The state the template was saved with goes.
            let saved = [
                "server_data",
                "shared_data",
                "spawn_data",
                "cooldown_end_at_tick",
                "next_mob_spawns_at",
            ];
            vec![same_state_rule(&TRIAL_SPAWNER_STATES, &saved, |_| {
                nbt.clone()
            })]
        }
        "moogs_structures:vault_randomizing_processor" => {
            let normal = p["loot_table"].as_str()?;
            let config = |ominous: bool| {
                let (table, key) = match ominous {
                    true => (
                        p["ominous_loot_table"].as_str().unwrap_or(normal),
                        p["ominous_key_item"]
                            .as_str()
                            .unwrap_or("minecraft:ominous_trial_key"),
                    ),
                    false => (
                        normal,
                        p["key_item"].as_str().unwrap_or("minecraft:trial_key"),
                    ),
                };
                // Merged into the vault's config, as the mod puts both in.
                json!({"config": {"loot_table": table, "key_item": {"id": key, "count": 1}}})
            };
            vec![same_state_rule(
                &VAULT_STATES,
                &["server_data", "shared_data"],
                |s| config(s.get("ominous") == Some(&"true")),
            )]
        }
        _ => return None,""",
                """        "repurposed_structures:structure_void_processor" => vec![json!({
            "processor_type": "minecraft:block_ignore",
            "blocks": ["minecraft:structure_void"]})],
        "moogs_structures:vanilla_loot_swap_processor" => {
            let key = (
                p["modid"].as_str()?.to_owned(),
                p["vanilla_key"].as_str()?.to_owned(),
            );
            if data.replace_vanilla.contains(&key) {
                return None;
            }
            Vec::new()
        }
        _ => return None,""",
            ),
            (
                """/// A block's states, as (block, [(property, values)]).
type States = (
    &'static str,
    &'static [(&'static str, &'static [&'static str])],
);
const TRIAL_SPAWNER_STATES: States = (
    "minecraft:trial_spawner",
    &[
        (
            "trial_spawner_state",
            &[
                "inactive",
                "waiting_for_players",
                "active",
                "waiting_for_reward_ejection",
                "ejecting_reward",
                "cooldown",
            ],
        ),
        ("ominous", &["false", "true"]),
    ],
);
const VAULT_STATES: States = (
    "minecraft:vault",
    &[
        ("facing", &["north", "south", "west", "east"]),
        (
            "vault_state",
            &["inactive", "active", "unlocking", "ejecting"],
        ),
        ("ominous", &["false", "true"]),
    ],
);

/// Moog's trial spawner and vault processors keep the block, take keys out
/// of its block entity and set others. As vanilla's `rule`: one rule per
/// state of the block, matching it and writing it back, with the game's
/// `sunrise:edit` modifier taking out `remove` and merging `nbt(state)` in.
fn same_state_rule(
    states: &States,
    remove: &[&str],
    nbt: impl Fn(&BTreeMap<&str, &str>) -> Value,
) -> Value {
    let (block, properties) = states;
    let mut all = vec![BTreeMap::new()];
    for (key, values) in properties.iter() {
        all = all
            .into_iter()
            .flat_map(|s| {
                values.iter().map(move |v| {
                    let mut s = s.clone();
                    s.insert(*key, *v);
                    s
                })
            })
            .collect();
    }
    let rules: Vec<Value> = all
        .iter()
        .map(|s| {
            let state = json!({"id": block, "properties": s});
            json!({
                "input_predicate": {"predicate_type": "minecraft:blockstate_match", "block_state": state},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": state,
                "block_entity_modifier": {"type": "sunrise:edit", "remove": remove, "data": nbt(s)},
            })
        })
        .collect();
    json!({"processor_type": "minecraft:rule", "rules": rules})
}

""",
                "",
            ),
            (
                """/// [`as_data`]'s `sunrise:spawner_mobs` for the mods' spawner randomizing
/// processors, whose fields have the same names and defaults.
fn spawner_mobs(p: &Value, mobs: Vec<Value>, rules: Option<Value>) -> Value {
    let mut out = json!({"processor_type": "sunrise:spawner_mobs", "mobs": mobs});
    for key in [
        "delay",
        "min_spawn_delay",
        "max_spawn_delay",
        "spawn_count",
        "max_nearby_entities",
        "required_player_range",
        "spawn_range",
    ] {
        if let Some(n) = p.get(key).filter(|n| n.is_i64()) {
            out[key] = n.clone();
        }
    }
    if let Some(r) = rules {
        out["custom_spawn_rules"] = r;
    }
    if let Some(b) = p.get("spawner_replacement_block") {
        out["replacement"] = b.clone();
    }
    out
}

/// Repurposed Structures' `custom_spawn_rules` from its light level ranges:
/// `Some(None)` without either, `None` when one does not read. The sky
/// light's upper bound is written as `max_exclusive`, as the mod does.
fn repurposed_light(p: &Value) -> Option<Option<Value>> {
    let range = |key: &str| -> Option<Option<(i64, i64)>> {
        match p.get(key) {
            None => Some(None),
            Some(Value::Number(n)) => n.as_i64().map(|n| Some((n, n))),
            Some(v) => Some(Some((
                v["min_inclusive"].as_i64()?,
                v["max_inclusive"].as_i64()?,
            ))),
        }
    };
    let (block, sky) = (
        range("valid_block_light_level")?,
        range("valid_sky_light_level")?,
    );
    if block.is_none() && sky.is_none() {
        return Some(None);
    }
    let mut rules = json!({});
    if let Some((min, max)) = block {
        rules["block_light_limit"] = json!({"min_inclusive": min, "max_inclusive": max});
    }
    if let Some((min, max)) = sky {
        rules["sky_light_limit"] = json!({"min_inclusive": min, "max_exclusive": max});
    }
    Some(Some(rules))
}

""",
                "",
            ),
            (
                """    #[test]
    fn trial_spawners_and_vaults_drop_their_saved_state() {
        let mut list = json!({"processors": [
            {"processor_type": "moogs_structures:trial_spawner_randomizing_processor",
             "normal_config": "mns:arena/normal", "ominous_config": "mns:arena/ominous"},
            {"processor_type": "moogs_structures:vault_randomizing_processor",
             "loot_table": "mns:vaults/main", "ominous_loot_table": "mns:vaults/ominous"}]});
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &data(), &mut carried));
        let p = list["processors"].as_array().unwrap();
        let rules = |i: usize| p[i]["rules"].as_array().unwrap().clone();
        // Every state of the block: 6 x 2 trial spawners, 4 x 4 x 2 vaults.
        assert_eq!((rules(0).len(), rules(1).len()), (12, 32));
        let spawner = &rules(0)[0];
        assert_eq!(
            spawner["input_predicate"]["block_state"],
            spawner["output_state"]
        );
        assert_eq!(
            spawner["block_entity_modifier"],
            json!({"type": "sunrise:edit",
                   "remove": ["server_data", "shared_data", "spawn_data",
                              "cooldown_end_at_tick", "next_mob_spawns_at"],
                   "data": {"normal_config": "mns:arena/normal",
                            "ominous_config": "mns:arena/ominous"}})
        );
        for rule in rules(1) {
            let ominous = rule["output_state"]["properties"]["ominous"] == "true";
            let (table, key) = match ominous {
                true => ("mns:vaults/ominous", "minecraft:ominous_trial_key"),
                false => ("mns:vaults/main", "minecraft:trial_key"),
            };
            assert_eq!(
                rule["block_entity_modifier"],
                json!({"type": "sunrise:edit", "remove": ["server_data", "shared_data"],
                       "data": {"config": {"loot_table": table,
                                           "key_item": {"id": key, "count": 1}}}})
            );
        }
        assert_eq!(carried.len(), 2);
    }

""",
                "",
            ),
            (
                """    #[test]
    fn repurposed_waterlogging_follows_the_world() {
        let mut list = json!({"processors": [{"processor_type":
            "repurposed_structures:waterlogging_when_replacing_water_processor"},
            {"processor_type": "repurposed_structures:mineshaft_sky_view_processor"}]});
        let mut carried = BTreeMap::new();
        assert!(carry(
            "processor_list",
            &mut list,
            &ProcessorData::default(),
            &mut carried
        ));
        assert_eq!(
            list["processors"],
            json!([{"processor_type": "sunrise:waterlog_in_world_water"},
                {"processor_type": "sunrise:mineshaft_sky_view"}])
        );
    }

""",
                "",
            ),
            (
                """    #[test]
    fn pillars_become_the_games() {
        let mut d = ProcessorData::default();
        // Repurposed's city list names itself as its pillar's list.
        let city = json!({"processors": [
            {"processor_type": "repurposed_structures:pillar_processor", "direction": "down",
             "pillar_length": 15, "pillar_processor_list": "repurposed_structures:cities/x",
             "pillar_trigger_and_replacements": [{"trigger": {"Name": "minecraft:red_stained_glass"},
                 "replacement": {"Name": "minecraft:cobblestone"}}]},
            {"processor_type": "minecraft:block_rot", "integrity": 0.5}]});
        d.lists
            .insert("repurposed_structures:cities/x".into(), city.clone());
        let mut list = city;
        list["processors"].as_array_mut().unwrap().extend([
            json!({"processor_type": "repurposed_structures:bottom_pillar_processor",
                   "pillar_processor_list": "minecraft:empty"}),
            json!({"processor_type": "moogs_structures:close_off_fluid_sources_processor",
                   "weighted_list_of_replacement_blocks": [{"block": "minecraft:stone", "weight": 1}]}),
            json!({"processor_type": "repurposed_structures:close_off_air_sources_processor",
                   "weighted_list_of_replacement_blocks": [{"block": "minecraft:prismarine", "weight": 1}]}),
            json!({"processor_type": "betterfortresses:pillar_processor",
                   "target_block": {"Name": "minecraft:orange_terracotta"},
                   "target_block_output": {"defaultBlockState": {"Name": "minecraft:red_nether_bricks"}, "entries": []},
                   "pillar_states": {"defaultBlockState": {"Name": "minecraft:nether_bricks"}, "entries": []}}),
            json!({"processor_type": "betterfortresses:stair_pillar_processor"}),
        ]);
        let mut carried = BTreeMap::new();
        assert!(carry("processor_list", &mut list, &d, &mut carried));
        let p = list["processors"].as_array().unwrap();
        let types: Vec<&str> = p
            .iter()
            .map(|p| p["processor_type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            [
                "sunrise:pillar",
                "minecraft:block_rot",
                "sunrise:bottom_pillar",
                "sunrise:close_off_fluid_sources",
                "sunrise:close_off_air_sources",
                // YUNG's processors stay Java (dropped later) until the
                // specification covers them.
                "betterfortresses:pillar_processor",
                "betterfortresses:stair_pillar_processor"
            ]
        );
        // The named list is written in once; its own pillar loses its list.
        let inner = &p[0]["pillar_processor_list"]["processors"];
        assert_eq!(inner[0]["processor_type"], "sunrise:pillar");
        assert!(inner[0].get("pillar_processor_list").is_none());
        assert_eq!(
            inner[0]["pillar_trigger_and_replacements"],
            p[0]["pillar_trigger_and_replacements"]
        );
        assert_eq!(inner[1]["processor_type"], "minecraft:block_rot");
        assert!(p[2].get("pillar_processor_list").is_none());
        assert_eq!(
            p[4]["weighted_list_of_replacement_blocks"],
            json!([{"block": "minecraft:prismarine", "weight": 1}])
        );
        assert_eq!(carried["repurposed_structures:pillar_processor"], 2);
        assert!(!carried.contains_key("betterfortresses:stair_pillar_processor"));
    }
""",
                "",
            ),
        ],
    )

    wg_path = ROOT / "crates/sunrise-port-pack/src/worldgen.rs"
    replace_exact(
        wg_path,
        [
            (
                """    processors.retain(|p| {
        let t = p["processor_type"].as_str().unwrap_or_default();
        let java = !t.starts_with("minecraft:")
            && !t.starts_with("sunrise:")
            && !lithostitched::game_reads(p);
        if java {
            *dropped.entry(t.to_owned()).or_default() += 1;
        }
        !java
    });
    let mut changed = processors.len() != before;
    // The inline lists inside Lithostitched's apply_random and condition.
    for p in processors.iter_mut() {
        let inner: Vec<&mut Value> = match p["processor_type"].as_str() {
            Some(t) if t.starts_with("lithostitched:") => lithostitched::inner_lists(p),
            Some("sunrise:pillar" | "sunrise:bottom_pillar") => {
                p.get_mut("pillar_processor_list").into_iter().collect()
            }
            _ => Vec::new(),
        };
        for list in inner {
            changed |= drop_java_processors(list, dropped);
        }
    }
    changed""",
                """    processors.retain(|p| {
        let t = p["processor_type"].as_str().unwrap_or_default();
        let keep = t.starts_with("minecraft:");
        if !keep {
            *dropped.entry(t.to_owned()).or_default() += 1;
        }
        keep
    });
    processors.len() != before""",
            ),
        ],
    )


def main() -> int:
    remove_processor_modules()
    edit_processor_rs()
    edit_port_pack()
    return 0


if __name__ == "__main__":
    sys.exit(main())
