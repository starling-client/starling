//! Persistent `ParticleTypes.CODEC` options. Model sprites and registry IDs are
//! resolved by the receiving level, not stored in the environment attribute.
//!
//! A block or an item is named here and refused by the level that draws the option when its
//! block registry or item table does not know the name (`Game::emit_model_ambient`). Vanilla's
//! codecs make the same lookup while the biome file is decoded: `Registry.byNameCodec` refuses
//! a block outside the registry, and `Item.CODEC` an item outside it or air. The attribute
//! files are read before a level's registries exist, so the registry check waits for the level;
//! air needs no registry, so `Item.CODEC`'s air check refuses it here, while decoding.

use crate::{
    chunk::BlockState,
    particle_type::ParticleType,
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use sunrise_core::{identifier::Identifier, json_nbt, nbt::Tag};

#[derive(Clone, Debug, PartialEq)]
pub enum ParticleOptions {
    Simple(ParticleType),
    ElderGuardian,
    Block(ParticleType, BlockState),
    Item(Tag),
    Dust(i32, f32),
    DustTransition(i32, i32, f32),
    Color(ParticleType, i32),
    Spell(ParticleType, i32, f32),
    Power(f32),
    Charge(f32),
    Shriek(i32),
    Geyser(ParticleType, i32, Option<f32>),
    Trail([f64; 3], i32, i32),
    Vibration([i32; 3], i32),
}

fn id(name: &str) -> Option<String> {
    Some(Identifier::parse(name).ok()?.to_string())
}

fn integer(value: &Value) -> Option<i32> {
    let value = value.as_number()?;
    value
        .as_i64()
        .map(|n| n as i32)
        .or_else(|| value.as_u64().map(|n| n as i32))
        .or_else(|| value.as_f64().map(|n| n as i64 as i32))
}

fn number(value: &Value) -> Option<f32> {
    Some(value.as_f64()? as f32)
}

fn coordinates<T>(value: &Value, parse: fn(&Value) -> Option<T>) -> Option<[T; 3]> {
    let [x, y, z] = value.as_array()?.as_slice() else {
        return None;
    };
    Some([parse(x)?, parse(y)?, parse(z)?])
}

fn color(value: &Value, len: usize) -> Option<i32> {
    if let Some(color) = integer(value) {
        return Some(color);
    }
    let list = value.as_array().filter(|l| l.len() == len)?;
    let mut packed = if len == 3 { 0xFF_u32 << 24 } else { 0 };
    for (component, shift) in list.iter().zip([16, 8, 0, 24]) {
        // ARGB.colorFromFloat uses Mth.floor and packs the low byte.
        let channel = number(component)? * 255.0;
        let truncated = channel as i32;
        let floor = if channel < truncated as f32 {
            truncated.wrapping_sub(1)
        } else {
            truncated
        };
        packed |= ((floor as u32) & 255) << shift;
    }
    Some(packed as i32)
}

impl ParticleOptions {
    /// Decode the registered type and that type's own persistent fields.
    pub(crate) fn parse(value: &Value) -> Option<Self> {
        use ParticleType as T;
        let name = id(value.get("type")?.as_str()?)?;
        if name == "minecraft:elder_guardian" {
            return Some(Self::ElderGuardian);
        }
        let ty = T::from_name(&name)?;
        let optional_power = || value.get("power").map_or(Some(1.0), number);
        let scale = || number(value.get("scale")?).filter(|n| (0.01..=4.0).contains(n));
        let positive = |key| integer(value.get(key)?).filter(|n| *n > 0);
        Some(match ty {
            T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
                let state = value.get("block_state")?;
                let name = id(state.as_str().or_else(|| state.get("id")?.as_str())?)?;
                let properties = state["properties"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                    .collect();
                Self::Block(ty, BlockState { name, properties })
            }
            T::Item => {
                let item = value.get("item")?;
                let name = id(item.as_str().or_else(|| item.get("id")?.as_str())?)?;
                // `Item.CODEC`: "Item must not be minecraft:air".
                if name == "minecraft:air" {
                    return None;
                }
                let count = item.get("count").map_or(Some(1), integer)?;
                if !(1..=99).contains(&count) {
                    return None;
                }
                let mut fields = vec![
                    ("id", Tag::String(name.as_str().into())),
                    ("count", Tag::Int(count)),
                ];
                if let Some(components) = item.get("components") {
                    let tag =
                        Tag::Compound(json_nbt::json_to_compound(components.as_object()?).ok()?);
                    crate::item_defaults::validate_component_map_tag(&tag).ok()?;
                    fields.push(("components", tag));
                }
                Self::Item(Tag::compound(fields))
            }
            T::Dust => Self::Dust(color(value.get("color")?, 3)?, scale()?),
            T::DustColorTransition => Self::DustTransition(
                color(value.get("from_color")?, 3)?,
                color(value.get("to_color")?, 3)?,
                scale()?,
            ),
            T::EntityEffect | T::TintedLeaves | T::Flash => {
                Self::Color(ty, color(value.get("color")?, 4)?)
            }
            T::Effect | T::InstantEffect => Self::Spell(
                ty,
                value.get("color").map_or(Some(-1), |v| color(v, 3))?,
                optional_power()?,
            ),
            T::DragonBreath => Self::Power(optional_power()?),
            T::SculkCharge => Self::Charge(number(value.get("roll")?)?),
            T::Shriek => Self::Shriek(integer(value.get("delay")?)?),
            T::Geyser | T::GeyserPlume => Self::Geyser(ty, positive("water_blocks")?, None),
            T::GeyserBase | T::GeyserPoof => Self::Geyser(
                ty,
                positive("water_blocks")?,
                Some(number(value.get("burst_impulse_base")?)?),
            ),
            T::Trail => Self::Trail(
                coordinates(value.get("target")?, Value::as_f64)?,
                color(value.get("color")?, 3)?,
                positive("duration")?,
            ),
            T::Vibration => {
                let dest = value.get("destination")?;
                if id(dest.get("type")?.as_str()?)? != "minecraft:block" {
                    return None;
                }
                Self::Vibration(
                    coordinates(dest.get("pos")?, integer)?,
                    integer(value.get("arrival_in_ticks")?)?,
                )
            }
            _ => Self::Simple(ty),
        })
    }

    pub(crate) fn to_nbt(&self) -> Tag {
        use ParticleType as T;
        let mut fields = Vec::new();
        let float = |n: f32| Tag::Float(n.to_bits());
        let ty = match self {
            Self::Simple(ty) => *ty,
            Self::ElderGuardian => {
                return Tag::compound([("type", Tag::String("minecraft:elder_guardian".into()))]);
            }
            Self::Block(ty, state) => {
                let mut block = vec![("id", Tag::String(state.name.as_str().into()))];
                if !state.properties.is_empty() {
                    block.push((
                        "properties",
                        Tag::compound(
                            state
                                .properties
                                .iter()
                                .map(|(k, v)| (k.as_str(), Tag::String(v.as_str().into()))),
                        ),
                    ));
                }
                fields.push(("block_state", Tag::compound(block)));
                *ty
            }
            Self::Item(item) => {
                fields.push(("item", item.clone()));
                T::Item
            }
            Self::Dust(color, scale) => {
                fields.extend([("color", Tag::Int(*color)), ("scale", float(*scale))]);
                T::Dust
            }
            Self::DustTransition(from, to, scale) => {
                fields.extend([
                    ("from_color", Tag::Int(*from)),
                    ("to_color", Tag::Int(*to)),
                    ("scale", float(*scale)),
                ]);
                T::DustColorTransition
            }
            Self::Color(ty, color) => {
                fields.push(("color", Tag::Int(*color)));
                *ty
            }
            Self::Spell(ty, color, power) => {
                fields.extend([("color", Tag::Int(*color)), ("power", float(*power))]);
                *ty
            }
            Self::Power(value) => {
                fields.push(("power", float(*value)));
                T::DragonBreath
            }
            Self::Charge(value) => {
                fields.push(("roll", float(*value)));
                T::SculkCharge
            }
            Self::Shriek(value) => {
                fields.push(("delay", Tag::Int(*value)));
                T::Shriek
            }
            Self::Geyser(ty, blocks, base) => {
                fields.push(("water_blocks", Tag::Int(*blocks)));
                if let Some(base) = base {
                    fields.push(("burst_impulse_base", float(*base)));
                }
                *ty
            }
            Self::Trail(pos, color, duration) => {
                fields.extend([
                    (
                        "target",
                        Tag::List(pos.iter().map(|p| Tag::Double(p.to_bits())).collect()),
                    ),
                    ("color", Tag::Int(*color)),
                    ("duration", Tag::Int(*duration)),
                ]);
                T::Trail
            }
            Self::Vibration(pos, ticks) => {
                fields.extend([
                    (
                        "destination",
                        Tag::compound([
                            ("type", Tag::String("minecraft:block".into())),
                            (
                                "pos",
                                Tag::List(pos.iter().copied().map(Tag::Int).collect()),
                            ),
                        ]),
                    ),
                    ("arrival_in_ticks", Tag::Int(*ticks)),
                ]);
                T::Vibration
            }
        };
        fields.push((
            "type",
            Tag::String(format!("minecraft:{}", ty.name()).as_str().into()),
        ));
        Tag::compound(fields)
    }
}

/// `StateDefinition` applies known, valid properties independently from the
/// default state. A registry lookup is part of resolving persistent block options.
pub fn block_state_id(registry: &BlockRegistry, state: &BlockState) -> Option<BlockStateId> {
    let mut id = registry.default_state(&state.name)?;
    if state.properties.is_empty() {
        return Some(id);
    }
    let mut resolved = registry.state(id)?.clone();
    for (key, value) in &state.properties {
        if let Some(previous) = resolved.properties.get(key).cloned() {
            resolved.properties.insert(key.clone(), value.clone());
            if let Some(next) = registry.id(&resolved) {
                id = next;
            } else {
                resolved.properties.insert(key.clone(), previous);
            }
        }
    }
    Some(id)
}
