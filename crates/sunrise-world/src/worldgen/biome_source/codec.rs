//! Registry-bound biome-source definitions. Inline full biome objects are separate.
use super::{BiomeSourceError, CheckerboardError, ResolvedBiomeSource};
use crate::worldgen::{
    DecodeError, Object, RawValue,
    climate::{ParameterPoint, decode_parameter_point, unquantize},
    density::identifier,
    int_range,
    mod_biome_sources::{LevelBiomeSource, mod_biome_sources},
    optional, required,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BiomeSet {
    Direct(Vec<String>),
    Tag { name: String, members: Vec<String> },
}
impl BiomeSet {
    pub fn members(&self) -> &[String] {
        match self {
            Self::Direct(values) => values,
            Self::Tag { members, .. } => members,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BiomeSourceDefinition {
    Fixed(String),
    Checkerboard {
        biomes: BiomeSet,
        scale: i32,
    },
    End,
    MultiNoise(Vec<(ParameterPoint, String)>),
    Preset {
        name: String,
        inline: bool,
    },
    /// A biome source a mod registered, by its id (`modid:chaos`).
    Registered(String),
}
#[derive(Clone, Debug)]
pub struct BiomeSourceCatalog {
    biomes: BTreeSet<String>,
    tags: BTreeMap<String, Vec<String>>,
    presets: BTreeSet<String>,
}
impl BiomeSourceCatalog {
    pub fn new(
        biomes: &BTreeSet<String>,
        tags: &BTreeMap<String, Vec<String>>,
        presets: &BTreeSet<String>,
    ) -> Result<Self, DecodeError> {
        let biomes = biomes
            .iter()
            .map(|v| identifier(v))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let presets = presets
            .iter()
            .map(|v| identifier(v))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut resolved = BTreeMap::new();
        for (name, members) in tags {
            let members = members
                .iter()
                .map(|v| identifier(v))
                .collect::<Result<Vec<_>, _>>()?;
            if members.iter().any(|v| !biomes.contains(v)) {
                return Err(DecodeError::Field("unknown biome tag member"));
            }
            if resolved.insert(identifier(name)?, members).is_some() {
                return Err(DecodeError::Field("duplicate biome tag"));
            }
        }
        Ok(Self {
            biomes,
            tags: resolved,
            presets,
        })
    }
    fn biome(&self, raw: &RawValue) -> Result<String, DecodeError> {
        self.biome_id(&serde_json::from_str::<String>(raw.get())?)
    }
    fn biome_id(&self, name: &str) -> Result<String, DecodeError> {
        let name = identifier(name)?;
        if !self.biomes.contains(&name) {
            return Err(DecodeError::MissingRegistryEntry(format!(
                "unknown biome {name}"
            )));
        }
        Ok(name)
    }
    fn tag_members(&self, name: &str) -> Result<(String, Vec<String>), DecodeError> {
        let name = identifier(name)?;
        let members = self.tags.get(&name).ok_or_else(|| {
            DecodeError::MissingRegistryEntry(format!("unknown biome tag #{name}"))
        })?;
        Ok((name, members.clone()))
    }
    fn set(&self, raw: &RawValue) -> Result<BiomeSet, DecodeError> {
        // NeoForge's set objects resolve through the shared decoder, in set order;
        // a plain list keeps its own order (checkerboard reads it).
        if raw.get().starts_with('{') {
            let set: serde_json::Value = serde_json::from_str(raw.get())?;
            return crate::holder_set::resolve(&set, self).map(BiomeSet::Direct);
        }
        if raw.get().starts_with('"') {
            let name: String = serde_json::from_str(raw.get())?;
            if let Some(name) = name.strip_prefix('#') {
                let (name, members) = self.tag_members(name)?;
                return Ok(BiomeSet::Tag { name, members });
            }
            return Ok(BiomeSet::Direct(vec![self.biome_id(&name)?]));
        }
        let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
        Ok(BiomeSet::Direct(
            values
                .iter()
                .map(|v| self.biome(v))
                .collect::<Result<_, _>>()?,
        ))
    }
    fn inline(&self, raw: &RawValue) -> Result<BiomeSourceDefinition, DecodeError> {
        let entries: Vec<Object> = serde_json::from_str(raw.get())?;
        if entries.is_empty() {
            return Err(DecodeError::Field("multi-noise entry count"));
        }
        let mut result = Vec::with_capacity(entries.len());
        for entry in entries {
            result.push((
                decode_parameter_point(required(&entry, "parameters")?.get())?,
                self.biome(required(&entry, "biome")?)?,
            ));
        }
        Ok(BiomeSourceDefinition::MultiNoise(result))
    }
    pub fn decode(&self, json: &str) -> Result<BiomeSourceDefinition, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        Ok(match kind.as_str() {
            "minecraft:fixed" => {
                BiomeSourceDefinition::Fixed(self.biome(required(&object, "biome")?)?)
            }
            "minecraft:checkerboard" => BiomeSourceDefinition::Checkerboard {
                biomes: self.set(required(&object, "biomes")?)?,
                scale: optional(&object, "scale")
                    .map_or(Ok(2), |v| int_range(v, "biome scale", 0, 62))?,
            },
            "minecraft:the_end" => {
                for name in [
                    "the_end",
                    "end_highlands",
                    "end_midlands",
                    "small_end_islands",
                    "end_barrens",
                ] {
                    let name = format!("minecraft:{name}");
                    if !self.biomes.contains(&name) {
                        return Err(DecodeError::MissingRegistryEntry(format!(
                            "missing End biome {name}"
                        )));
                    }
                }
                BiomeSourceDefinition::End
            }
            "minecraft:multi_noise" => {
                // mapEither prefers a successful inline list, then tries preset.
                let preset = optional(&object, "preset");
                if let Some(raw) = optional(&object, "biomes") {
                    match self.inline(raw) {
                        Ok(value) => return Ok(value),
                        Err(error) if preset.is_none() => return Err(error),
                        Err(_) => {}
                    }
                }
                let raw = required(&object, "preset")?;
                if raw.get().starts_with('"') {
                    let name = identifier(&serde_json::from_str::<String>(raw.get())?)?;
                    if !self.presets.contains(&name) {
                        return Err(DecodeError::MissingRegistryEntry(format!(
                            "unknown biome preset holder {name}"
                        )));
                    }
                    BiomeSourceDefinition::Preset {
                        name,
                        inline: false,
                    }
                } else {
                    let preset: Object = serde_json::from_str(raw.get())?;
                    let name = identifier(&serde_json::from_str::<String>(
                        required(&preset, "preset")?.get(),
                    )?)?;
                    if !matches!(name.as_str(), "minecraft:overworld" | "minecraft:nether") {
                        return Err(DecodeError::MissingRegistryEntry(format!(
                            "unknown built-in biome preset {name}"
                        )));
                    }
                    BiomeSourceDefinition::Preset { name, inline: true }
                }
            }
            _ => {
                // A mod's source: its possible biomes must be the world's.
                LevelBiomeSource::resolve(&kind, mod_biome_sources(), 0, |name| {
                    self.biomes.contains(name).then_some(())
                })?;
                BiomeSourceDefinition::Registered(kind)
            }
        })
    }
    /// Bind a custom stem's source before admitting the level. An empty checkerboard holder set
    /// cannot be sampled (`CheckerboardColumnBiomeSource.getNoiseBiome`).
    pub fn bind_custom_source(
        &self,
        json: &str,
        presets: &BTreeMap<String, String>,
        seed: i64,
    ) -> Result<ResolvedBiomeSource, BiomeSourceError> {
        let definition = self.decode(json)?;
        if let BiomeSourceDefinition::Checkerboard { biomes, .. } = &definition
            && biomes.members().is_empty()
        {
            return Err(CheckerboardError::EmptyBiomes.into());
        }
        definition.resolve(presets, seed)
    }
    /// Encode definitions in the vanilla canonical shape; decoding the result
    /// validates registry references and bounds before publishing JSON.
    pub fn encode(&self, definition: &BiomeSourceDefinition) -> Result<String, DecodeError> {
        let (kind, fields) = match definition {
            BiomeSourceDefinition::Fixed(name) => ("fixed", format!(",\"biome\":{}", quoted(name))),
            BiomeSourceDefinition::End => ("the_end", String::new()),
            BiomeSourceDefinition::Checkerboard { biomes, scale } => {
                let values = match biomes {
                    BiomeSet::Tag { name, .. } => quoted(&format!("#{name}")),
                    BiomeSet::Direct(values) if values.len() == 1 => quoted(&values[0]),
                    BiomeSet::Direct(values) => serde_json::to_string(values)?,
                };
                (
                    "checkerboard",
                    format!(
                        ",\"biomes\":{values}{}",
                        if *scale == 2 {
                            String::new()
                        } else {
                            format!(",\"scale\":{scale}")
                        }
                    ),
                )
            }
            BiomeSourceDefinition::MultiNoise(entries) => {
                let values = entries
                    .iter()
                    .map(|(point, name)| {
                        format!(
                            "{{\"parameters\":{},\"biome\":{}}}",
                            point_json(point),
                            quoted(name)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                ("multi_noise", format!(",\"biomes\":[{values}]"))
            }
            BiomeSourceDefinition::Preset { name, inline } => {
                let value = if *inline {
                    format!("{{\"preset\":{}}}", quoted(name))
                } else {
                    quoted(name)
                };
                ("multi_noise", format!(",\"preset\":{value}"))
            }
            BiomeSourceDefinition::Registered(id) => {
                let json = format!("{{\"type\":{}}}", quoted(id));
                self.decode(&json)?;
                return Ok(json);
            }
        };
        let json = format!("{{\"type\":\"minecraft:{kind}\"{fields}}}");
        self.decode(&json)?;
        Ok(json)
    }
}
fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}
fn point_json(point: &ParameterPoint) -> String {
    let names = [
        "temperature",
        "humidity",
        "continentalness",
        "erosion",
        "depth",
        "weirdness",
    ];
    let mut fields = Vec::with_capacity(7);
    for (name, p) in names.into_iter().zip(point.coordinates) {
        let min = serde_json::to_string(&unquantize(p.min)).expect("finite coordinate");
        let max = serde_json::to_string(&unquantize(p.max)).expect("finite coordinate");
        let value = if min == max {
            min
        } else {
            format!("[{min},{max}]")
        };
        fields.push(format!("{}:{value}", quoted(name)));
    }
    fields.push(format!(
        "\"offset\":{}",
        serde_json::to_string(&unquantize(point.offset)).expect("finite offset")
    ));
    format!("{{{}}}", fields.join(","))
}

impl crate::holder_set::HolderLookup<DecodeError> for BiomeSourceCatalog {
    type Id = String;
    fn what(&self) -> &str {
        "biome"
    }
    fn entry(&self, id: &str) -> Result<Vec<String>, DecodeError> {
        self.biome_id(id).map(|id| vec![id])
    }
    fn tag(&self, tag: &str) -> Result<Vec<String>, DecodeError> {
        self.tag_members(tag).map(|(_, members)| members)
    }
    fn all(&self) -> Result<Vec<String>, DecodeError> {
        Ok(self.biomes.iter().cloned().collect())
    }
}
