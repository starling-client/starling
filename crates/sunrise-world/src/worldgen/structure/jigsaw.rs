//! Template pools, pool elements and JigsawPlacement.
use mod_api::JigsawSpace;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock, RwLock},
};
use sunrise_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, RandomSource, position_seed},
};
use {
    super::{
        LazyTemplate, Mirror, Rotation, StructureResources, StructureTemplate,
        legacy::{
            PieceContext, PieceDraws, PieceRandom, PieceWorld, Result as LegacyResult, SolidRender,
        },
        processor::{PlaceSettings, Processor, ProcessorList, ProcessorRandom, place_in_world},
    },
    sunrise_core::bounding_box::BoundingBox,
};
use {
    crate::{
        fluid::{FluidKind, FluidState},
        heightmap::HeightmapType,
        registry::BlockStateId,
        worldgen::{
            DecodeError,
            biome::BiomeId,
            block_feature::FeatureBlockBindings,
            feature::FeatureWorld,
            placement::{PlacementContext, PlacementQueryError},
        },
    },
    sunrise_core::nbt::{Compound, Tag},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Rigid,
    TerrainMatching,
}
impl Projection {
    /// `StructureTemplatePool.Projection.getSerializedName`.
    pub fn saved_name(self) -> &'static str {
        match self {
            Self::Rigid => "rigid",
            Self::TerrainMatching => "terrain_matching",
        }
    }
}
#[derive(Clone, Debug)]
pub enum PoolElement {
    Empty,
    Single {
        template: LazyTemplate,
        processors: ProcessorList,
        /// The element's `processors` as written: a list id or an inline list,
        /// saved back as it was.
        processors_source: Value,
        legacy: bool,
        projection: Projection,
        waterlogging: Option<bool>,
        /// The piece's own terrain kernel in a structure that has one
        /// (`"sunrise:enhanced_terrain_adaptation"`).
        enhanced_terrain: Option<Arc<super::enhanced_terrain::Adaptation>>,
    },
    Feature {
        feature: String,
        projection: Projection,
    },
    List {
        elements: Vec<PoolElement>,
        projection: Projection,
    },
    /// An element of a type a mod registered (`StructurePoolElementType`),
    /// with its registered id (`element_type`).
    Mod {
        id: Arc<str>,
        element: Arc<dyn super::mod_elements::ModPoolElement>,
        /// The element's own projection, or the list's that holds it
        /// (`ListPoolElement.setProjectionOnEachElement`).
        projection: Projection,
    },
}
/// A feature a piece places: a configured feature
/// (`minecraft:worldgen/feature`) or a placed one
/// (`minecraft:worldgen/placed_feature`), by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKey<'a> {
    Configured(&'a str),
    Placed(&'a str),
}
/// Where and how an element is placed: `StructurePoolElement.place`'s
/// arguments besides the level, the random and the chunk's box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementAt {
    pub position: [i32; 3],
    pub reference: [i32; 3],
    pub rotation: Rotation,
    /// `LiquidSettings.APPLY_WATERLOGGING` (true) or `IGNORE_WATERLOGGING`.
    pub waterlogging: bool,
    /// Leave the jigsaw blocks as they are (`/place template`'s and
    /// `/place jigsaw`'s `keepJigsaws`).
    pub keep_jigsaws: bool,
}
#[derive(Clone, Debug)]
pub struct Jigsaw<'a> {
    pub pos: [i32; 3],
    pub front: Direction,
    pub top: Direction,
    pub rollable: bool,
    pub name: Option<std::borrow::Cow<'a, str>>,
    pub pool: std::borrow::Cow<'a, str>,
    pub target: std::borrow::Cow<'a, str>,
    pub placement_priority: i32,
    pub selection_priority: i32,
}
fn direction(name: &str) -> Direction {
    match name {
        "down" => Direction::Down,
        "up" => Direction::Up,
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        _ => Direction::East,
    }
}
fn opposite(d: Direction) -> Direction {
    match d {
        Direction::Down => Direction::Up,
        Direction::Up => Direction::Down,
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::West => Direction::East,
        Direction::East => Direction::West,
    }
}
pub(super) fn step(d: Direction) -> [i32; 3] {
    let n = d.normal();
    [n[0], n[1], n[2]]
}
/// Util.shuffle.
pub fn shuffle<T>(list: &mut [T], random: &mut (impl ProcessorRandom + ?Sized)) {
    for i in (2..=list.len()).rev() {
        let swap = random.int(i as i32) as usize;
        list.swap(i - 1, swap);
    }
}
fn projection(value: &Value) -> Result<Projection, DecodeError> {
    match value["projection"].as_str() {
        Some("rigid") => Ok(Projection::Rigid),
        Some("terrain_matching") => Ok(Projection::TerrainMatching),
        _ => Err(DecodeError::Field("pool element projection")),
    }
}
impl PoolElement {
    fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
        inherited: Option<Projection>,
    ) -> Result<Self, DecodeError> {
        let kind = value["element_type"].as_str().unwrap_or_default();
        let projection = || match inherited {
            Some(p) => Ok(p),
            None => projection(value),
        };
        Ok(match kind {
            "minecraft:empty_pool_element" => Self::Empty,
            // Lithostitched's elements place what their `delegate` places;
            // their limits are the pool entry's (`Pool::limits`).
            "lithostitched:limited" | "lithostitched:guaranteed" | "lithostitched:delegating" => {
                Self::decode(&value["delegate"], resources, bindings, inherited)?
            }
            "minecraft:single_pool_element" | "minecraft:legacy_single_pool_element" => {
                let location = value["location"]
                    .as_str()
                    .ok_or(DecodeError::Field("location"))?;
                Self::Single {
                    template: resources.lazy_template(location),
                    processors: ProcessorList::decode(
                        &value["processors"].to_string(),
                        resources,
                        bindings,
                    )?,
                    processors_source: value["processors"].clone(),
                    legacy: kind == "minecraft:legacy_single_pool_element",
                    projection: projection()?,
                    waterlogging: match value
                        .get("override_liquid_settings")
                        .and_then(Value::as_str)
                    {
                        Some("apply_waterlogging") => Some(true),
                        Some("ignore_waterlogging") => Some(false),
                        _ => None,
                    },
                    enhanced_terrain: value
                        .get("sunrise:enhanced_terrain_adaptation")
                        .map(|a| super::enhanced_terrain::Adaptation::decode(a).map(Arc::new))
                        .transpose()?,
                }
            }
            "minecraft:feature_pool_element" => Self::Feature {
                feature: value["feature"].as_str().unwrap_or_default().to_owned(),
                projection: projection()?,
            },
            "minecraft:list_pool_element" => {
                let p = projection()?;
                Self::List {
                    elements: value["elements"]
                        .as_array()
                        .ok_or(DecodeError::Field("list elements"))?
                        .iter()
                        .map(|e| Self::decode(e, resources, bindings, Some(p)))
                        .collect::<Result<_, _>>()?,
                    projection: p,
                }
            }
            _ => {
                let Some(kind_type) = super::mod_elements::mod_pool_element_type(kind) else {
                    return Err(DecodeError::Field("pool element type"));
                };
                let element = kind_type
                    .decode(value, resources, bindings)
                    .map_err(|e| DecodeError::Registry(format!("{kind}: {e}")))?;
                Self::Mod {
                    id: kind.into(),
                    projection: inherited.unwrap_or_else(|| element.projection()),
                    element,
                }
            }
        })
    }
    /// `StructurePoolElement.CODEC`'s decoding of one element (a template
    /// pool entry's `element`), mods' element types included.
    pub fn decode_element(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        Self::decode(value, resources, bindings, None)
    }
    /// `StructurePoolElement.CODEC`'s encoding, as a jigsaw piece saves it.
    pub fn save(&self) -> Tag {
        let text = |s: &str| Tag::String(s.into());
        match self {
            Self::Empty => Tag::compound([("element_type", text("minecraft:empty_pool_element"))]),
            Self::Single {
                template,
                processors_source,
                legacy,
                projection,
                waterlogging,
                ..
            } => {
                let kind = if *legacy {
                    "minecraft:legacy_single_pool_element"
                } else {
                    "minecraft:single_pool_element"
                };
                let mut entries = vec![
                    ("element_type", text(kind)),
                    ("location", text(&template.location)),
                    (
                        "processors",
                        super::processor::json_to_nbt(processors_source),
                    ),
                    ("projection", text(projection.saved_name())),
                ];
                if let Some(apply) = waterlogging {
                    let settings = if *apply {
                        "apply_waterlogging"
                    } else {
                        "ignore_waterlogging"
                    };
                    entries.push(("override_liquid_settings", text(settings)));
                }
                Tag::compound(entries)
            }
            Self::Feature {
                feature,
                projection,
            } => Tag::compound([
                ("element_type", text("minecraft:feature_pool_element")),
                ("feature", text(feature)),
                ("projection", text(projection.saved_name())),
            ]),
            Self::List {
                elements,
                projection,
            } => Tag::compound([
                ("element_type", text("minecraft:list_pool_element")),
                (
                    "elements",
                    Tag::List(elements.iter().map(Self::save).collect()),
                ),
                ("projection", text(projection.saved_name())),
            ]),
            Self::Mod { id, element, .. } => {
                let mut fields = element.save();
                fields.insert("element_type".into(), text(id));
                Tag::Compound(fields)
            }
        }
    }
    pub fn projection(&self) -> Projection {
        match self {
            Self::Empty => Projection::TerrainMatching,
            Self::Single { projection, .. }
            | Self::Feature { projection, .. }
            | Self::List { projection, .. }
            | Self::Mod { projection, .. } => *projection,
        }
    }
    /// `StructurePoolElement.getBoundingBox`.
    pub fn bounding_box(
        &self,
        resources: &StructureResources,
        position: [i32; 3],
        rotation: Rotation,
    ) -> BoundingBox {
        match self {
            Self::Empty => BoundingBox::new(position, position),
            Self::Feature { .. } => BoundingBox::new(position, position),
            Self::Single { template, .. } => {
                template.bounding_box(position, rotation, [0; 3], Mirror::None)
            }
            Self::Mod { element, .. } => element.bounding_box(resources, position, rotation),
            Self::List { elements, .. } => {
                let mut boxes = elements
                    .iter()
                    .filter(|e| !matches!(e, Self::Empty))
                    .map(|e| e.bounding_box(resources, position, rotation));
                let mut first = boxes.next().unwrap_or(BoundingBox::new(position, position));
                for b in boxes {
                    first.encapsulate(&b);
                }
                first
            }
        }
    }
    /// getShuffledJigsawBlocks.
    pub fn shuffled_jigsaws<'a>(
        &'a self,
        resources: &StructureResources,
        position: [i32; 3],
        rotation: Rotation,
        random: &mut dyn PieceDraws,
    ) -> Vec<Jigsaw<'a>> {
        match self {
            Self::Empty => Vec::new(),
            Self::Mod { element, .. } => {
                element.shuffled_jigsaws(resources, position, rotation, &mut *random)
            }
            Self::Feature { .. } => vec![Jigsaw {
                pos: position,
                front: Direction::Down,
                top: Direction::South,
                rollable: true,
                name: None,
                pool: "minecraft:empty".into(),
                target: "minecraft:empty".into(),
                placement_priority: 0,
                selection_priority: 0,
            }],
            Self::List { elements, .. } => elements
                .first()
                .map(|e| e.shuffled_jigsaws(resources, position, rotation, &mut *random))
                .unwrap_or_default(),
            Self::Single { template, .. } => {
                let mut jigsaws = template_jigsaws(resources, template, position, rotation);
                shuffle(&mut jigsaws, random);
                jigsaws.sort_by_key(|j| std::cmp::Reverse(j.selection_priority));
                jigsaws
            }
        }
    }
    pub fn ground_level_delta(&self) -> i32 {
        match self {
            Self::Mod { element, .. } => element.ground_level_delta(),
            _ => 1,
        }
    }
    /// The piece's own terrain kernel, in place of its structure's.
    pub fn enhanced_terrain(&self) -> Option<&super::enhanced_terrain::Adaptation> {
        match self {
            Self::Single {
                enhanced_terrain, ..
            } => enhanced_terrain.as_deref(),
            _ => None,
        }
    }
}
/// `StructureTemplate.getJigsaws`: palette chosen by a position-seeded random.
/// Connector states use plain `BlockState.rotate`, not the placement-only rotate Part.
fn template_jigsaws<'a>(
    resources: &StructureResources,
    template: &'a StructureTemplate,
    position: [i32; 3],
    rotation: Rotation,
) -> Vec<Jigsaw<'a>> {
    if template.palettes().is_empty() {
        return Vec::new();
    }
    let palette = LegacyRandom::new(position_seed(BlockPos::new(
        position[0],
        position[1],
        position[2],
    )))
    .next_int_bound(template.palettes().len() as i32)
    .unwrap_or(0) as usize;
    template
        .jigsaws(palette, &|state| resources.jigsaw_horizontal(state))
        .iter()
        .map(|j| {
            let relative = StructureTemplate::transform(j.info.pos, Mirror::None, rotation, [0; 3]);
            let pos = std::array::from_fn(|i| relative[i].wrapping_add(position[i]));
            let state = resources.transforms.rotate(j.info.state, rotation);
            let orientation = resources
                .registry
                .state(state)
                .and_then(|s| s.properties.get("orientation").map(String::as_str))
                .unwrap_or_default();
            let (front, top) = orientation.split_once('_').unwrap_or(("north", "up"));
            Jigsaw {
                pos,
                front: direction(front),
                top: direction(top),
                rollable: j.rollable,
                name: Some(std::borrow::Cow::Borrowed(&j.name)),
                pool: std::borrow::Cow::Borrowed(&j.pool),
                target: std::borrow::Cow::Borrowed(&j.target),
                placement_priority: j.placement_priority,
                selection_priority: j.selection_priority,
            }
        })
        .collect()
}
pub(super) fn can_attach(source: &Jigsaw, target: &Jigsaw) -> bool {
    source.front == opposite(target.front)
        && (source.rollable || source.top == target.top)
        && target
            .name
            .as_ref()
            .is_none_or(|name| *name == source.target)
}

#[derive(Debug)]
pub struct Pool {
    /// Each element repeated by its weight.
    pub(super) templates: Vec<Arc<PoolElement>>,
    /// The entry of `raw` each of `templates` repeats.
    entries: Vec<usize>,
    /// The elements with their weights, in file order (`rawTemplates`).
    pub(super) raw: Vec<(Arc<PoolElement>, i32)>,
    /// Per entry of `raw`: a Lithostitched `limited` or `guaranteed`
    /// element's limit.
    limits: Vec<Option<Limit>>,
    /// The entries of `raw` that are Lithostitched's `guaranteed` elements.
    guaranteed: Vec<usize>,
    pub(super) fallback: String,
    max_size: OnceLock<i32>,
}
impl Pool {
    /// `StructureTemplatePool.templates`: each element repeated by its weight.
    pub fn templates(&self) -> &[Arc<PoolElement>] {
        &self.templates
    }
    /// The fallback pool's id.
    pub fn fallback(&self) -> &str {
        &self.fallback
    }
    pub(super) fn random_template(&self, random: &mut impl RandomSource) -> Arc<PoolElement> {
        if self.templates.is_empty() {
            return Arc::new(PoolElement::Empty);
        }
        self.templates[random
            .next_int_bound(self.templates.len() as i32)
            .unwrap_or(0) as usize]
            .clone()
    }
    /// The entries of `templates`, shuffled.
    fn shuffled(&self, random: &mut impl RandomSource) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.templates.len()).collect();
        shuffle(&mut order, random);
        order.into_iter().map(|i| self.entries[i]).collect()
    }
    /// `StructureTemplatePool.getMaxSize`: the tallest element's height.
    pub fn max_size(&self, resources: &StructureResources) -> i32 {
        *self.max_size.get_or_init(|| {
            self.templates
                .iter()
                .filter(|t| !matches!(t.as_ref(), PoolElement::Empty))
                .map(|t| t.bounding_box(resources, [0; 3], Rotation::None).span()[1])
                .max()
                .unwrap_or(0)
        })
    }
}
/// How many pieces of a Lithostitched `limited` or `guaranteed` element one
/// start may hold, and where (spec-lithostitched §3).
#[derive(Clone, Debug)]
struct Limit {
    /// The pieces counted together: the element itself (its JSON), wherever
    /// it is listed.
    key: Arc<str>,
    max: i32,
    /// The parent piece's least depth (the start is at 0).
    min_depth: i32,
    /// `lithostitched:guaranteed`: tried before its pool's other entries.
    guaranteed: bool,
}
impl Limit {
    fn of(element: &Value) -> Option<Self> {
        let int = |key: &str| {
            element[key]
                .as_i64()
                .map(|v| v.clamp(0, i32::MAX.into()) as i32)
        };
        let (max, guaranteed) = match element["element_type"].as_str()? {
            "lithostitched:limited" => (int("limit")?, false),
            "lithostitched:guaranteed" => (int("count").unwrap_or(1), true),
            _ => return None,
        };
        Some(Self {
            key: element.to_string().into(),
            max,
            min_depth: int("min_depth").unwrap_or(0),
            guaranteed,
        })
    }
    /// Whether one more piece may go below a parent at `depth`.
    fn allows(&self, counts: &HashMap<Arc<str>, i32>, depth: i32) -> bool {
        depth >= self.min_depth && counts.get(&self.key).copied().unwrap_or(0) < self.max
    }
}
/// Lazily decoded template pools.
pub struct Pools {
    raw: BTreeMap<String, String>,
    decoded: RwLock<HashMap<String, Option<Arc<Pool>>>>,
}
impl std::fmt::Debug for Pools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pools")
    }
}
impl Pools {
    pub fn new(raw: BTreeMap<String, String>) -> Self {
        Self {
            raw,
            decoded: RwLock::new(HashMap::new()),
        }
    }
    pub fn get(
        &self,
        name: &str,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Option<Arc<Pool>> {
        if let Some(found) = self.decoded.read().unwrap().get(name) {
            return found.clone();
        }
        let pool = self.raw.get(name).and_then(|json| {
            let value: Value = serde_json::from_str(json).ok()?;
            let (mut templates, mut entries, mut raw, mut limits) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for entry in value["elements"].as_array()? {
                let element =
                    match PoolElement::decode(&entry["element"], resources, bindings, None) {
                        Ok(element) => Arc::new(element),
                        Err(error) => {
                            // Vanilla refuses the data pack; here every structure using the
                            // pool stops generating, so the reason is the only clue a mod author gets.
                            // Once per pool: each worker decodes its own copy.
                            static WARNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
                            if WARNED.lock().unwrap().insert(name.to_owned()) {
                                let file = match name.split_once(':') {
                                    Some((ns, path)) => {
                                        format!("data/{ns}/worldgen/template_pool/{path}.json")
                                    }
                                    None => {
                                        format!("data/minecraft/worldgen/template_pool/{name}.json")
                                    }
                                };
                                log::warn!(
                                    "Sunrise: template pool {name} ({file}) is left out: {error}"
                                );
                            }
                            return None;
                        }
                    };
                let weight = entry["weight"].as_i64().unwrap_or(1);
                for _ in 0..weight {
                    templates.push(element.clone());
                    entries.push(raw.len());
                }
                raw.push((element, weight as i32));
                limits.push(Limit::of(&entry["element"]));
            }
            let guaranteed = (limits.iter().enumerate())
                .filter(|(_, l)| l.as_ref().is_some_and(|l| l.guaranteed))
                .map(|(i, _)| i)
                .collect();
            Some(Arc::new(Pool {
                templates,
                entries,
                raw,
                limits,
                guaranteed,
                fallback: value["fallback"]
                    .as_str()
                    .unwrap_or("minecraft:empty")
                    .to_owned(),
                max_size: OnceLock::new(),
            }))
        });
        self.decoded
            .write()
            .unwrap()
            .insert(name.to_owned(), pool.clone());
        pool
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Junction {
    pub source_x: i32,
    pub source_ground_y: i32,
    pub source_z: i32,
    /// Saved only (`JigsawJunction.deltaY`, `destProjection`).
    pub delta_y: i32,
    pub dest_projection: Projection,
}
#[derive(Debug)]
pub struct PoolPiece {
    pub element: Arc<PoolElement>,
    pub position: [i32; 3],
    pub ground_level_delta: i32,
    pub rotation: Rotation,
    pub bounding_box: BoundingBox,
    pub junctions: Vec<Junction>,
    pub waterlogging: bool,
}
impl PoolPiece {
    /// A piece as a saved start lists it (PoolElementStructurePiece's tag:
    /// `pool_element`, `PosX`/`PosY`/`PosZ`, `rotation`, `ground_level_delta`,
    /// `BB`, `junctions`, `liquid_settings`), in JSON. Harnesses place another
    /// game's layout with it.
    pub fn from_saved(
        v: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let int = |v: &Value, key: &'static str| {
            v[key]
                .as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .ok_or(DecodeError::Field(key))
        };
        let corners: Vec<i32> = v["BB"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i32::try_from(i.as_i64()?).ok())
            .collect();
        let [x0, y0, z0, x1, y1, z1]: [i32; 6] =
            corners.try_into().map_err(|_| DecodeError::Field("BB"))?;
        let rotation = match v["rotation"].as_str() {
            Some("NONE") => Rotation::None,
            Some("CLOCKWISE_90") => Rotation::Clockwise90,
            Some("CLOCKWISE_180") => Rotation::Clockwise180,
            Some("COUNTERCLOCKWISE_90") => Rotation::Counterclockwise90,
            _ => return Err(DecodeError::Field("rotation")),
        };
        let junctions = v["junctions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|j| {
                Ok(Junction {
                    source_x: int(j, "source_x")?,
                    source_ground_y: int(j, "source_ground_y")?,
                    source_z: int(j, "source_z")?,
                    delta_y: int(j, "delta_y")?,
                    dest_projection: match j["dest_proj"].as_str() {
                        Some("rigid") => Projection::Rigid,
                        Some("terrain_matching") => Projection::TerrainMatching,
                        _ => return Err(DecodeError::Field("dest_proj")),
                    },
                })
            })
            .collect::<Result<_, DecodeError>>()?;
        Ok(Self {
            element: Arc::new(PoolElement::decode(
                &v["pool_element"],
                resources,
                bindings,
                None,
            )?),
            position: [int(v, "PosX")?, int(v, "PosY")?, int(v, "PosZ")?],
            ground_level_delta: int(v, "ground_level_delta")?,
            rotation,
            bounding_box: BoundingBox::new([x0, y0, z0], [x1, y1, z1]),
            junctions,
            waterlogging: v["liquid_settings"].as_str() != Some("ignore_waterlogging"),
        })
    }
    /// `PoolElementStructurePiece`'s tag (always genDepth 0, no orientation).
    pub fn save(&self) -> Compound {
        let mut tag = super::legacy::piece_tag("minecraft:jigsaw", self.bounding_box, None, 0);
        let junctions = self
            .junctions
            .iter()
            .map(|j| {
                Tag::compound([
                    ("source_x", Tag::Int(j.source_x)),
                    ("source_ground_y", Tag::Int(j.source_ground_y)),
                    ("source_z", Tag::Int(j.source_z)),
                    ("delta_y", Tag::Int(j.delta_y)),
                    (
                        "dest_proj",
                        Tag::String(j.dest_projection.saved_name().into()),
                    ),
                ])
            })
            .collect();
        for (key, value) in [
            ("PosX", Tag::Int(self.position[0])),
            ("PosY", Tag::Int(self.position[1])),
            ("PosZ", Tag::Int(self.position[2])),
            ("ground_level_delta", Tag::Int(self.ground_level_delta)),
            ("pool_element", self.element.save()),
            ("rotation", Tag::String(self.rotation.saved_name().into())),
            ("junctions", Tag::List(junctions)),
        ] {
            tag.insert(key.into(), value);
        }
        // LiquidSettings.APPLY_WATERLOGGING, the default, is not written.
        if !self.waterlogging {
            tag.insert(
                "liquid_settings".into(),
                Tag::String("ignore_waterlogging".into()),
            );
        }
        tag
    }
    pub(super) fn moved(&mut self, dy: i32) {
        self.position[1] = self.position[1].wrapping_add(dy);
        self.bounding_box = self.bounding_box.moved([0, dy, 0]);
    }
    /// PoolElementStructurePiece.postProcess.
    pub fn place<W: FeatureWorld + PlacementContext, R: RandomSource + 'static>(
        &self,
        site: &mut Site<'_, '_, W, R>,
        random: &mut R,
        reference: [i32; 3],
        keep_jigsaws: bool,
    ) -> Result<bool, PlacementQueryError> {
        site.place(
            random,
            &self.element,
            ElementAt {
                position: self.position,
                reference,
                rotation: self.rotation,
                waterlogging: self.waterlogging,
                keep_jigsaws,
            },
        )
    }
}
static GRAVITY: OnceLock<Processor> = OnceLock::new();
static IGNORE_STRUCTURE_BLOCK: OnceLock<Processor> = OnceLock::new();
static IGNORE_STRUCTURE_AND_AIR: OnceLock<Processor> = OnceLock::new();

/// Places a feature (feature pool elements, and pieces placing one) with the
/// piece random. An id the registry lacks places nothing: `Ok(false)`.
pub type PlaceFeature<'a, W, R> =
    dyn FnMut(&mut W, &mut R, FeatureKey<'_>, [i32; 3]) -> Result<bool, PlacementQueryError> + 'a;

/// A generating structure's pieces' view of one chunk: the level, and how
/// features are placed into it. Pieces reach it as their [`PieceWorld`], so a
/// mod's piece or element places features and pool elements through it with
/// the piece random.
pub struct Site<'s, 'f, W, R> {
    pub world: &'s mut W,
    pub resources: &'s StructureResources,
    /// `BlockState.isSolidRender` of the blocks around the pieces.
    pub blocks: &'s dyn SolidRender,
    /// The chunk's writable box the pieces are clipped to.
    pub chunk_box: BoundingBox,
    pub place_feature: &'s mut PlaceFeature<'f, W, R>,
}
impl<W: FeatureWorld + PlacementContext, R: RandomSource + 'static> Site<'_, '_, W, R> {
    /// `StructurePoolElement.place`, clipped to the chunk's box.
    pub fn place(
        &mut self,
        random: &mut R,
        element: &PoolElement,
        at: ElementAt,
    ) -> Result<bool, PlacementQueryError> {
        match element {
            PoolElement::Empty => Ok(true),
            PoolElement::Feature { feature, .. } => {
                (self.place_feature)(self.world, random, FeatureKey::Placed(feature), at.position)
            }
            PoolElement::List { elements, .. } => {
                for e in elements {
                    if !self.place(random, e, at)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            PoolElement::Mod {
                element,
                projection,
                ..
            } => {
                let (resources, blocks, chunk_box) = (self.resources, self.blocks, self.chunk_box);
                let mut context = PieceContext {
                    world: self,
                    resources,
                    blocks,
                    random,
                    chunk_box,
                    chunk: ChunkPos::new(chunk_box.min[0] >> 4, chunk_box.min[2] >> 4),
                    reference: at.reference,
                };
                element.place(&mut context, at, *projection)
            }
            PoolElement::Single {
                template,
                processors,
                legacy,
                projection,
                waterlogging: override_liquid,
                ..
            } => {
                let ignore = if *legacy {
                    IGNORE_STRUCTURE_AND_AIR.get_or_init(|| {
                        Processor::BlockIgnore(
                            [
                                "minecraft:structure_block".to_owned(),
                                "minecraft:air".to_owned(),
                            ]
                            .into(),
                        )
                    })
                } else {
                    IGNORE_STRUCTURE_BLOCK.get_or_init(|| {
                        Processor::BlockIgnore(["minecraft:structure_block".to_owned()].into())
                    })
                };
                let mut settings = PlaceSettings::new(at.rotation);
                settings.known_shape = true;
                // SinglePoolElement.getSettings.
                settings.finalize_entities = true;
                settings.bounding_box = Some(self.chunk_box);
                settings.waterlogging = override_liquid.unwrap_or(at.waterlogging);
                if !*legacy {
                    settings.processors.push(ignore);
                }
                // SinglePoolElement.getSettings: keepJigsaws skips the replacement.
                if !at.keep_jigsaws {
                    settings.processors.push(&Processor::JigsawReplacement);
                }
                settings.processors.extend(processors.0.iter());
                if *projection == Projection::TerrainMatching {
                    settings
                        .processors
                        .push(GRAVITY.get_or_init(|| Processor::Gravity {
                            heightmap: HeightmapType::WorldSurfaceWg,
                            offset: -1,
                        }));
                }
                // LegacySinglePoolElement pops the structure-block ignore and
                // appends STRUCTURE_AND_AIR last, after jigsaw replacement.
                if *legacy {
                    settings.processors.push(ignore);
                }
                place_in_world(
                    self.world,
                    self.resources,
                    template,
                    at.position,
                    at.reference,
                    &mut settings,
                    Some(random),
                    18,
                )
            }
        }
    }
    /// The piece's random as the site's own; a piece hands back the random it
    /// was given, so another is refused.
    fn own_random(random: &mut dyn PieceRandom) -> Result<&mut R, PlacementQueryError> {
        random
            .as_any()
            .downcast_mut::<R>()
            .ok_or_else(|| PlacementQueryError::Failed("not the piece's random".into()))
    }
}
impl<W: FeatureWorld + PlacementContext, R: RandomSource + 'static> PieceWorld
    for Site<'_, '_, W, R>
{
    fn block(&mut self, pos: [i32; 3]) -> LegacyResult<BlockStateId> {
        PieceWorld::block(self.world, pos)
    }
    fn set(&mut self, pos: [i32; 3], state: BlockStateId, flags: i32) -> LegacyResult<bool> {
        PieceWorld::set(self.world, pos, state, flags)
    }
    fn fluid(&mut self, pos: [i32; 3]) -> LegacyResult<FluidState> {
        PieceWorld::fluid(self.world, pos)
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> LegacyResult<i32> {
        PieceWorld::height(self.world, kind, x, z)
    }
    fn min_y(&mut self) -> LegacyResult<i32> {
        PieceWorld::min_y(self.world)
    }
    fn max_y(&mut self) -> LegacyResult<i32> {
        PieceWorld::max_y(self.world)
    }
    fn entity(&mut self, nbt: Compound) -> LegacyResult<()> {
        PieceWorld::entity(self.world, nbt)
    }
    fn block_entity(&mut self, pos: [i32; 3], nbt: Compound) -> LegacyResult<()> {
        PieceWorld::block_entity(self.world, pos, nbt)
    }
    fn post_process(&mut self, pos: [i32; 3]) -> LegacyResult<()> {
        PieceWorld::post_process(self.world, pos)
    }
    fn fluid_tick(&mut self, pos: [i32; 3], fluid: FluidKind, delay: i32) -> LegacyResult<()> {
        PieceWorld::fluid_tick(self.world, pos, fluid, delay)
    }
    fn seed(&mut self) -> LegacyResult<i64> {
        PieceWorld::seed(self.world)
    }
    fn level_boolean(&mut self) -> LegacyResult<bool> {
        PieceWorld::level_boolean(self.world)
    }
    fn level_float(&mut self) -> LegacyResult<f32> {
        PieceWorld::level_float(self.world)
    }
    fn level_int(&mut self, bound: i32) -> LegacyResult<i32> {
        PieceWorld::level_int(self.world, bound)
    }
    fn sea_level(&mut self) -> LegacyResult<i32> {
        PieceWorld::sea_level(self.world)
    }
    fn face_full(&mut self, pos: [i32; 3], direction: Direction) -> LegacyResult<bool> {
        PieceWorld::face_full(self.world, pos, direction)
    }
    fn face_sturdy(&mut self, pos: [i32; 3], face: Direction) -> LegacyResult<bool> {
        PieceWorld::face_sturdy(self.world, pos, face)
    }
    fn center_sturdy(&mut self, pos: [i32; 3], face: Direction) -> LegacyResult<bool> {
        PieceWorld::center_sturdy(self.world, pos, face)
    }
    fn biome(&mut self, pos: [i32; 3]) -> LegacyResult<BiomeId> {
        PieceWorld::biome(self.world, pos)
    }
    fn place_template(
        &mut self,
        resources: &StructureResources,
        template: &StructureTemplate,
        position: [i32; 3],
        reference: [i32; 3],
        settings: &mut PlaceSettings<'_>,
        random: &mut dyn ProcessorRandom,
        flags: i32,
    ) -> LegacyResult<bool> {
        PieceWorld::place_template(
            self.world, resources, template, position, reference, settings, random, flags,
        )
    }
    fn place_feature(
        &mut self,
        random: &mut dyn PieceRandom,
        feature: FeatureKey<'_>,
        pos: [i32; 3],
    ) -> LegacyResult<bool> {
        let random = Self::own_random(random)?;
        (self.place_feature)(self.world, random, feature, pos)
    }
    fn place_element(
        &mut self,
        random: &mut dyn PieceRandom,
        element: &PoolElement,
        at: ElementAt,
    ) -> LegacyResult<bool> {
        let random = Self::own_random(random)?;
        self.place(random, element, at)
    }
}

/// The world queries jigsaw assembly needs from the chunk generator.
pub trait JigsawContext {
    fn min_y(&self) -> i32;
    fn max_y(&self) -> i32;
    /// ChunkGenerator.getBaseHeight (getFirstFreeHeight).
    fn base_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32;
    /// couldStructureExistInColumn over inclusive block Y bounds.
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool;
}
/// A borrowed context, so a `&mut dyn` start context (a mod structure type's) reaches the
/// generic assembly.
impl<T: JigsawContext + ?Sized> JigsawContext for &mut T {
    fn min_y(&self) -> i32 {
        (**self).min_y()
    }
    fn max_y(&self) -> i32 {
        (**self).max_y()
    }
    fn base_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32 {
        (**self).base_height(x, z, kind)
    }
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool {
        (**self).column_has_valid_biome(x, z, min_y, max_y)
    }
}
/// The elements a jigsaw tries, in order: the target pool's weight-expanded entries, then the
/// fallback's, each shuffled up front (`JigsawPlacement.Placer.tryPlacingChildren`). A pool's
/// Lithostitched `guaranteed` entries come first when it is reached, in file order, and are
/// not drawn again (spec-lithostitched §3, OPEN-L4).
struct Candidates(std::vec::IntoIter<(Arc<Pool>, usize)>);
/// An element to try.
struct Pick {
    element: Arc<PoolElement>,
    /// Its Lithostitched limit's key, counted when it is placed.
    name: Option<Arc<str>>,
}
impl Candidates {
    fn new(
        target: &Arc<Pool>,
        fallback: &Arc<Pool>,
        below_max_depth: bool,
        random: &mut impl RandomSource,
    ) -> Self {
        let pools = if below_max_depth {
            vec![target.clone(), fallback.clone()]
        } else {
            vec![fallback.clone()]
        };
        let mut listed = Vec::new();
        for pool in &pools {
            listed.extend(pool.guaranteed.iter().map(|&e| (pool.clone(), e)));
            let drawn = pool.shuffled(random).into_iter();
            let drawn = drawn.filter(|&e| !pool.guaranteed.contains(&e));
            listed.extend(drawn.map(|e| (pool.clone(), e)));
        }
        Self(listed.into_iter())
    }
    /// The next element to try for a jigsaw of a piece at `depth`.
    fn next(&mut self, counts: &HashMap<Arc<str>, i32>, depth: i32) -> Option<Pick> {
        self.0.find_map(|(pool, entry)| {
            let limit = pool.limits[entry].as_ref();
            limit.is_none_or(|l| l.allows(counts, depth)).then(|| Pick {
                element: pool.raw[entry].0.clone(),
                name: limit.map(|l| l.key.clone()),
            })
        })
    }
}

pub struct JigsawConfig<'a> {
    pub start_pool: &'a str,
    pub start_jigsaw: Option<&'a str>,
    pub max_depth: i32,
    pub use_expansion_hack: bool,
    pub project_to: Option<HeightmapType>,
    pub max_distance: [i32; 2],
    pub aliases: &'a BTreeMap<String, String>,
    pub padding: [i32; 2],
    pub waterlogging: bool,
}

struct PieceState {
    piece: usize,
    free: Rc<RefCell<JigsawSpace>>,
    depth: i32,
}

/// `JigsawPlacement.addPieces` from a start pool: the start piece drawn, rotated, projected
/// and checked against the height limits ([`place_start`]), then its children laid out to
/// `config.max_depth` within `config.max_distance` of it ([`lay_out_children`]). Returns the
/// structure start position and the pieces, or None when the start is invalid. A `max_depth`
/// of zero or less places no piece at all, as vanilla's stub adds pieces only when it is
/// positive.
///
/// This is the jigsaw assembly every structure kind shares, and the one a mod's structure
/// type ([`ModStructureType`](super::mod_types::ModStructureType)) calls with its own decoded
/// settings: it keeps vanilla's placement checks (the biome and height of the projected start,
/// the dimension padding, each child's free space and the container box) but not
/// `JigsawStructure`'s codec bounds on `size` and `max_distance_from_center`, which are the
/// `minecraft:jigsaw` type's own and apply only where that type is decoded.
pub fn assemble(
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    position: [i32; 3],
) -> Option<([i32; 3], Vec<PoolPiece>)> {
    let start = place_start(
        config, context, pools, resources, bindings, random, position,
    )?;
    Some(lay_out_children(
        start, config, context, pools, resources, bindings, random,
    ))
}

/// A jigsaw's start piece in place ([`place_start`]), its children not laid out yet: vanilla's
/// `Structure.GenerationStub`, whose position the structure's biome is checked at before
/// `JigsawPlacement.addPieces` lays the rest out.
pub struct PlacedStart {
    /// The start position (`[centre x, start y, centre z]`).
    pub position: [i32; 3],
    center: PoolPiece,
}
impl PlacedStart {
    /// The start piece as placed, before any child is laid out.
    pub fn piece(&self) -> &PoolPiece {
        &self.center
    }
}

/// The first half of [`assemble`]: the start piece drawn, rotated, projected to the
/// heightmap and checked against the height limits. None when the start is invalid. The
/// draws it makes come first in `random`, as in `assemble`.
pub fn place_start(
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    position: [i32; 3],
) -> Option<PlacedStart> {
    let alias = |name: &str| {
        config
            .aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_owned())
    };
    let center_pool = pools.get(&alias(config.start_pool), resources, bindings)?;
    let rotation = Rotation::random(random);
    let element = center_pool.random_template(random);
    if matches!(element.as_ref(), PoolElement::Empty) {
        return None;
    }
    let anchored = match config.start_jigsaw {
        Some(name) => {
            element
                .shuffled_jigsaws(resources, position, rotation, &mut *random)
                .into_iter()
                .find(|j| j.name.as_deref() == Some(name))?
                .pos
        }
        None => position,
    };
    // Java's int arithmetic wraps here; a mod type's position, reach and padding are its own.
    let local: [i32; 3] = std::array::from_fn(|i| anchored[i].wrapping_sub(position[i]));
    let adjusted: [i32; 3] = std::array::from_fn(|i| position[i].wrapping_sub(local[i]));
    let mut center = PoolPiece {
        bounding_box: element.bounding_box(resources, adjusted, rotation),
        ground_level_delta: element.ground_level_delta(),
        element: element.clone(),
        position: adjusted,
        rotation,
        junctions: Vec::new(),
        waterlogging: config.waterlogging,
    };
    let b = center.bounding_box;
    let cx = b.max[0].wrapping_add(b.min[0]) / 2;
    let cz = b.max[2].wrapping_add(b.min[2]) / 2;
    let bottom = match config.project_to {
        None => adjusted[1],
        Some(kind) => {
            if !context.column_has_valid_biome(cx, cz, context.min_y(), context.max_y()) {
                return None;
            }
            position[1].wrapping_add(context.base_height(cx, cz, kind))
        }
    };
    let old_ground = b.min[1].wrapping_add(center.ground_level_delta);
    center.moved(bottom.wrapping_sub(old_ground));
    let b = center.bounding_box;
    if config.padding != [0, 0]
        && (b.min[1] < context.min_y().wrapping_add(config.padding[0])
            || b.max[1] > context.max_y().wrapping_sub(config.padding[1]))
    {
        return None;
    }
    let cy = bottom.wrapping_add(local[1]);
    Some(PlacedStart {
        position: [cx, cy, cz],
        center,
    })
}

/// The second half of [`assemble`]: the children of a [`place_start`]ed start laid out
/// to `config.max_depth`, with the same `random`. Returns the start position and every piece.
pub fn lay_out_children(
    start: PlacedStart,
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
) -> ([i32; 3], Vec<PoolPiece>) {
    let PlacedStart {
        position: [cx, cy, cz],
        center,
    } = start;
    // JigsawPlacement.addPieces adds its pieces, the start among them, to the
    // builder only when `maxDepth > 0`.
    if config.max_depth <= 0 {
        return ([cx, cy, cz], Vec::new());
    }
    let mut pieces = vec![center];
    // Java's int arithmetic wraps; a mod type's reach and padding are its own.
    let [h, v] = config.max_distance;
    let low_y = cy
        .wrapping_sub(v)
        .max(context.min_y().wrapping_add(config.padding[0]));
    let high_y = cy.wrapping_add(v).wrapping_add(1).min(
        context
            .max_y()
            .wrapping_add(1)
            .wrapping_sub(config.padding[1]),
    );
    let container = BoundingBox::new(
        [cx.wrapping_sub(h), low_y, cz.wrapping_sub(h)],
        [
            cx.wrapping_add(h),
            high_y.wrapping_sub(1),
            cz.wrapping_add(h),
        ],
    );
    let mut space = JigsawSpace::new(container);
    space.remove(pieces[0].bounding_box);
    let mut placer = Placer {
        config,
        pools,
        resources,
        bindings,
        pieces: &mut pieces,
        queue: BTreeMap::new(),
        counts: HashMap::new(),
    };
    placer.children(0, Rc::new(RefCell::new(space)), 0, context, random);
    while let Some(state) = placer.next() {
        placer.children(state.piece, state.free, state.depth, context, random);
    }
    ([cx, cy, cz], pieces)
}

/// One target element tried on one jigsaw of `source`: the inner loops of
/// `JigsawPlacement.Placer.tryPlacingChildren` for that element. Its rotations
/// in `Rotation.getShuffled` order, in each its shuffled jigsaws, the
/// expansion hack, `JigsawBlock.canAttach`, the child's height, and the
/// check against `space`. `None`, with no draws, for the empty element, where
/// vanilla's candidate loop stops. On the first fit the box leaves `space`, `source`
/// gets its junction and the child is returned with its own. `pool` looks a
/// pool up by a jigsaw's name, aliases applied; `base_height` keeps the
/// `WORLD_SURFACE_WG` height under `jigsaw` once read, as vanilla does across
/// one jigsaw's candidates.
#[allow(clippy::too_many_arguments)]
pub fn attach<C: JigsawContext>(
    source: &mut PoolPiece,
    jigsaw: &Jigsaw<'_>,
    target: &Arc<PoolElement>,
    space: &mut JigsawSpace,
    use_expansion_hack: bool,
    pool: &dyn Fn(&str) -> Option<Arc<Pool>>,
    resources: &StructureResources,
    base_height: &mut Option<i32>,
    context: &mut C,
    random: &mut impl RandomSource,
) -> Option<PoolPiece> {
    if matches!(target.as_ref(), PoolElement::Empty) {
        return None;
    }
    let source_rigid = source.element.projection() == Projection::Rigid;
    let source_y = source.bounding_box.min[1];
    let source_ground = source.ground_level_delta;
    let s = step(jigsaw.front);
    let target_pos: [i32; 3] = std::array::from_fn(|i| jigsaw.pos[i].wrapping_add(s[i]));
    let local_y = jigsaw.pos[1].wrapping_sub(source_y);
    let surface = |base_height: &mut Option<i32>, context: &mut C| {
        *base_height.get_or_insert_with(|| {
            context.base_height(jigsaw.pos[0], jigsaw.pos[2], HeightmapType::WorldSurfaceWg)
        })
    };
    let mut rotations = Rotation::ALL;
    shuffle(&mut rotations, random);
    for rotation in rotations {
        let target_jigsaws = target.shuffled_jigsaws(resources, [0; 3], rotation, &mut *random);
        let hack_box = target.bounding_box(resources, [0; 3], rotation);
        let expand_to = if use_expansion_hack && hack_box.span()[1] <= 16 {
            target_jigsaws
                .iter()
                .map(|tj| {
                    let st = step(tj.front);
                    if !hack_box.is_inside(std::array::from_fn(|i| tj.pos[i].wrapping_add(st[i]))) {
                        return 0;
                    }
                    let child = pool(&tj.pool);
                    let child_size = child.as_ref().map_or(0, |p| p.max_size(resources));
                    let fallback_size = child
                        .and_then(|p| pool(&p.fallback))
                        .map_or(0, |p| p.max_size(resources));
                    child_size.max(fallback_size)
                })
                .max()
                .unwrap_or(0)
        } else {
            0
        };
        for target_jigsaw in &target_jigsaws {
            if !can_attach(jigsaw, target_jigsaw) {
                continue;
            }
            let target_local = target_jigsaw.pos;
            let raw_position: [i32; 3] =
                std::array::from_fn(|i| target_pos[i].wrapping_sub(target_local[i]));
            let raw_box = target.bounding_box(resources, raw_position, rotation);
            let raw_y = raw_box.min[1];
            let target_rigid = target.projection() == Projection::Rigid;
            let target_local_y = target_local[1];
            let delta_y = local_y
                .wrapping_sub(target_local_y)
                .wrapping_add(step(jigsaw.front)[1]);
            let target_y = if source_rigid && target_rigid {
                source_y.wrapping_add(delta_y)
            } else {
                surface(base_height, context).wrapping_sub(target_local_y)
            };
            let offset = target_y.wrapping_sub(raw_y);
            let mut target_box = raw_box.moved([0, offset, 0]);
            let target_position = [
                raw_position[0],
                raw_position[1].wrapping_add(offset),
                raw_position[2],
            ];
            if expand_to > 0 {
                let size = expand_to
                    .wrapping_add(1)
                    .max(target_box.max[1].wrapping_sub(target_box.min[1]));
                target_box.max[1] = target_box.max[1].max(target_box.min[1].wrapping_add(size));
            }
            if !space.fits(&target_box) {
                continue;
            }
            space.remove(target_box);
            let target_ground = if target_rigid {
                source_ground.wrapping_sub(delta_y)
            } else {
                target.ground_level_delta()
            };
            let junction_y = if source_rigid {
                source_y.wrapping_add(local_y)
            } else if target_rigid {
                target_y.wrapping_add(target_local_y)
            } else {
                surface(base_height, context).wrapping_add(delta_y / 2)
            };
            source.junctions.push(Junction {
                source_x: target_pos[0],
                source_ground_y: junction_y.wrapping_sub(local_y).wrapping_add(source_ground),
                source_z: target_pos[2],
                delta_y,
                dest_projection: target.projection(),
            });
            return Some(PoolPiece {
                element: target.clone(),
                position: target_position,
                ground_level_delta: target_ground,
                rotation,
                bounding_box: target_box,
                junctions: vec![Junction {
                    source_x: jigsaw.pos[0],
                    source_ground_y: junction_y
                        .wrapping_sub(target_local_y)
                        .wrapping_add(target_ground),
                    source_z: jigsaw.pos[2],
                    delta_y: delta_y.wrapping_neg(),
                    dest_projection: if source_rigid {
                        Projection::Rigid
                    } else {
                        Projection::TerrainMatching
                    },
                }],
                waterlogging: source.waterlogging,
            });
        }
    }
    None
}
struct Placer<'a, 'p> {
    config: &'a JigsawConfig<'a>,
    pools: &'a Pools,
    resources: &'a StructureResources,
    bindings: &'a FeatureBlockBindings<'a>,
    pieces: &'p mut Vec<PoolPiece>,
    /// SequencedPriorityIterator: highest priority first, FIFO within one.
    queue: BTreeMap<i32, VecDeque<PieceState>>,
    /// Pieces placed per limited element (`Limit::key`).
    counts: HashMap<Arc<str>, i32>,
}
impl Placer<'_, '_> {
    fn next(&mut self) -> Option<PieceState> {
        let mut entry = self.queue.last_entry()?;
        let state = entry.get_mut().pop_front();
        if entry.get().is_empty() {
            entry.remove();
        }
        state
    }
    fn children(
        &mut self,
        source_index: usize,
        context_free: Rc<RefCell<JigsawSpace>>,
        depth: i32,
        context: &mut impl JigsawContext,
        random: &mut impl RandomSource,
    ) {
        let (config, pools, resources, bindings) =
            (self.config, self.pools, self.resources, self.bindings);
        let pool = |name: &str| {
            let name = config.aliases.get(name).map_or(name, String::as_str);
            pools.get(name, resources, bindings)
        };
        let source = &self.pieces[source_index];
        let (source_element, source_position, source_rotation, source_box) = (
            source.element.clone(),
            source.position,
            source.rotation,
            source.bounding_box,
        );
        let mut source_free: Option<Rc<RefCell<JigsawSpace>>> = None;
        let jigsaws =
            source_element.shuffled_jigsaws(resources, source_position, source_rotation, random);
        'jigsaws: for jigsaw in jigsaws {
            let s = step(jigsaw.front);
            let target_pos: [i32; 3] = std::array::from_fn(|i| jigsaw.pos[i].wrapping_add(s[i]));
            let mut base_height = None;
            let Some(target_pool) = pool(&jigsaw.pool) else {
                continue;
            };
            if target_pool.templates.is_empty() && jigsaw.pool != "minecraft:empty" {
                continue;
            }
            let Some(fallback) = pool(&target_pool.fallback) else {
                continue;
            };
            if fallback.templates.is_empty() && target_pool.fallback != "minecraft:empty" {
                continue;
            }
            let children_free = if source_box.is_inside(target_pos) {
                source_free
                    .get_or_insert_with(|| Rc::new(RefCell::new(JigsawSpace::new(source_box))))
                    .clone()
            } else {
                context_free.clone()
            };
            let mut candidates =
                Candidates::new(&target_pool, &fallback, depth != config.max_depth, random);
            while let Some(Pick {
                element: target,
                name,
            }) = candidates.next(&self.counts, depth)
            {
                if matches!(target.as_ref(), PoolElement::Empty) {
                    break;
                }
                let Some(child) = attach(
                    &mut self.pieces[source_index],
                    &jigsaw,
                    &target,
                    &mut children_free.borrow_mut(),
                    config.use_expansion_hack,
                    &pool,
                    resources,
                    &mut base_height,
                    context,
                    random,
                ) else {
                    continue;
                };
                self.pieces.push(child);
                if let Some(name) = name {
                    *self.counts.entry(name).or_default() += 1;
                }
                if depth < config.max_depth {
                    self.queue
                        .entry(jigsaw.placement_priority)
                        .or_default()
                        .push_back(PieceState {
                            piece: self.pieces.len() - 1,
                            free: children_free.clone(),
                            depth: depth + 1,
                        });
                }
                continue 'jigsaws;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A pool of empty elements, told apart by identity, with these limits.
    fn pool(entries: &[(i32, Value)]) -> Arc<Pool> {
        let (mut templates, mut indices, mut raw, mut limits) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (i, (weight, element)) in entries.iter().enumerate() {
            let e = Arc::new(PoolElement::Empty);
            for _ in 0..*weight {
                templates.push(e.clone());
                indices.push(i);
            }
            raw.push((e, *weight));
            limits.push(Limit::of(element));
        }
        let guaranteed = (0..entries.len())
            .filter(|&i| limits[i].as_ref().is_some_and(|l: &Limit| l.guaranteed))
            .collect();
        Arc::new(Pool {
            templates,
            entries: indices,
            raw,
            limits,
            guaranteed,
            fallback: "minecraft:empty".into(),
            max_size: OnceLock::new(),
        })
    }
    fn delegate() -> Value {
        json!({"element_type": "minecraft:single_pool_element", "location": "a:b"})
    }
    /// The entries a vanilla jigsaw tries below a parent at `depth`: the
    /// target's by index, the fallback's by index plus 100.
    fn tried(
        target: &Arc<Pool>,
        fallback: &Arc<Pool>,
        counts: &HashMap<Arc<str>, i32>,
        depth: i32,
        below_max: bool,
    ) -> Vec<usize> {
        let mut random = LegacyRandom::new(7);
        let mut candidates = Candidates::new(target, fallback, below_max, &mut random);
        let index = |pool: &Arc<Pool>, element: &Arc<PoolElement>| {
            pool.raw.iter().position(|(e, _)| Arc::ptr_eq(e, element))
        };
        let mut out = Vec::new();
        while let Some(pick) = candidates.next(counts, depth) {
            out.push(
                index(target, &pick.element)
                    .or_else(|| index(fallback, &pick.element).map(|i| i + 100))
                    .unwrap(),
            );
        }
        out
    }

    #[test]
    fn limits_read_lithostitcheds_fields() {
        let limited =
            json!({"element_type": "lithostitched:limited", "limit": 1, "delegate": delegate()});
        let l = Limit::of(&limited).unwrap();
        assert_eq!((l.max, l.min_depth, l.guaranteed), (1, 0, false));
        assert_eq!(*l.key, *limited.to_string());
        let guaranteed = json!({"element_type": "lithostitched:guaranteed", "count": 2, "min_depth": 3,
            "delegate": delegate()});
        let g = Limit::of(&guaranteed).unwrap();
        assert_eq!((g.max, g.min_depth, g.guaranteed), (2, 3, true));
        // No limit: none to keep here.
        for none in [
            json!({"element_type": "lithostitched:limited", "delegate": delegate()}),
            delegate(),
        ] {
            assert!(Limit::of(&none).is_none(), "{none}");
        }
    }

    #[test]
    fn vanilla_jigsaws_keep_lithostitcheds_limits() {
        let limited =
            json!({"element_type": "lithostitched:limited", "limit": 1, "delegate": delegate()});
        let guaranteed = json!({"element_type": "lithostitched:guaranteed", "count": 1, "min_depth": 3,
            "delegate": delegate()});
        let p = pool(&[
            (3, delegate()),
            (2, limited.clone()),
            (1, guaranteed.clone()),
        ]);
        let empty = pool(&[]);
        let mut counts = HashMap::new();
        // Too shallow for the guaranteed entry: the weight-expanded list without it.
        let mut shallow = tried(&p, &empty, &counts, 2, true);
        shallow.sort();
        assert_eq!(shallow, [0, 0, 0, 1, 1]);
        // Deep enough: it comes first, and is not drawn again.
        let deep = tried(&p, &empty, &counts, 3, true);
        assert_eq!(deep[0], 2);
        assert_eq!(deep[1..].iter().filter(|&&e| e == 2).count(), 0);
        // At the most depth only the fallback is tried.
        assert!(tried(&p, &empty, &counts, 3, false).is_empty());
        // Entries at their limit are skipped.
        counts.insert(Arc::from(limited.to_string()), 1);
        counts.insert(Arc::from(guaranteed.to_string()), 1);
        let full = tried(&p, &empty, &counts, 3, true);
        assert_eq!(full, [0, 0, 0]);
    }

    /// A fallback pool's guaranteed entries come first once it is reached,
    /// below the most depth as at it (rdbtCVS's review of #1777).
    #[test]
    fn a_fallbacks_guaranteed_entries_are_tried() {
        let guaranteed = json!({"element_type": "lithostitched:guaranteed", "count": 1,
            "delegate": delegate()});
        let target = pool(&[(2, delegate())]);
        let fallback = pool(&[(1, delegate()), (1, guaranteed)]);
        let counts = HashMap::new();
        let below = tried(&target, &fallback, &counts, 0, true);
        assert_eq!(below[..2], [0, 0]);
        assert_eq!(below[2..], [101, 100]);
        assert_eq!(tried(&target, &fallback, &counts, 0, false), [101, 100]);
    }

    /// A pool entry of a registered mod type decodes through that type, after
    /// the vanilla types, and saves its fields back beside `element_type`; a
    /// pool with an entry of a type nobody registered is left out. Inside a
    /// list, the list's projection is the one in effect.
    #[test]
    fn mod_element_types_decode_and_unknown_ones_leave_the_pool_out() {
        use super::super::mod_elements::{
            ModPoolElement, ModPoolElementType, ModPoolElementTypes, with_test_pool_element_types,
        };
        #[derive(Debug)]
        struct Post(i32);
        impl ModPoolElement for Post {
            fn shuffled_jigsaws(
                &self,
                _: &StructureResources,
                _: [i32; 3],
                _: Rotation,
                _: &mut dyn PieceDraws,
            ) -> Vec<Jigsaw<'static>> {
                Vec::new()
            }
            fn bounding_box(
                &self,
                _: &StructureResources,
                p: [i32; 3],
                _: Rotation,
            ) -> BoundingBox {
                BoundingBox::new(p, [p[0], p[1] + self.0 - 1, p[2]])
            }
            fn place(
                &self,
                _: &mut super::super::legacy::PieceContext<'_>,
                _: ElementAt,
                _: Projection,
            ) -> super::super::legacy::Result<bool> {
                Ok(false)
            }
            fn projection(&self) -> Projection {
                Projection::Rigid
            }
            fn ground_level_delta(&self) -> i32 {
                1
            }
            fn save(&self) -> Compound {
                Compound::from_iter([("height".into(), Tag::Int(self.0))])
            }
        }
        struct PostType;
        impl ModPoolElementType for PostType {
            fn decode(
                &self,
                config: &Value,
                _: &StructureResources,
                _: &FeatureBlockBindings<'_>,
            ) -> Result<Arc<dyn ModPoolElement>, String> {
                let height = config["height"].as_i64().ok_or("no height")?;
                Ok(Arc::new(Post(height as i32)))
            }
        }
        let names = ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"];
        let facts = json!({
            "states": names.map(|name| json!([name, true, false, "", {"id": name, "properties": {}}])),
            "defaults": names.iter().enumerate().map(|(i, name)| (name.to_string(), json!(i))).collect::<serde_json::Map<_, _>>(),
        });
        let registry: &'static _ = Box::leak(Box::new(
            crate::registry::BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap(),
        ));
        let resources = StructureResources::new(
            registry,
            super::super::StateTransforms::default(),
            std::path::Path::new("/nonexistent"),
            BTreeMap::new(),
            &BTreeMap::new(),
        );
        let bindings = FeatureBlockBindings::new(registry, Default::default()).unwrap();
        let entry = |element: Value| json!({"weight": 2, "element": element}).to_string();
        let pools = Pools::new(BTreeMap::from([
            (
                "test:posts".to_owned(),
                format!(
                    r#"{{"fallback":"minecraft:empty","elements":[{}]}}"#,
                    entry(json!({"element_type": "test:post", "height": 4}))
                ),
            ),
            (
                "test:listed".to_owned(),
                format!(
                    r#"{{"fallback":"minecraft:empty","elements":[{}]}}"#,
                    entry(json!({"element_type": "minecraft:list_pool_element",
                        "projection": "terrain_matching",
                        "elements": [{"element_type": "test:post", "height": 2}]}))
                ),
            ),
            (
                "test:unknown".to_owned(),
                format!(
                    r#"{{"fallback":"minecraft:empty","elements":[{}]}}"#,
                    entry(json!({"element_type": "test:missing"}))
                ),
            ),
            (
                "test:refused".to_owned(),
                format!(
                    r#"{{"fallback":"minecraft:empty","elements":[{}]}}"#,
                    entry(json!({"element_type": "test:post"}))
                ),
            ),
        ]));
        let types: ModPoolElementTypes =
            BTreeMap::from([("test:post".to_owned(), Arc::new(PostType) as _)]);
        with_test_pool_element_types(&types, || {
            let posts = pools.get("test:posts", &resources, &bindings).unwrap();
            assert_eq!(posts.templates().len(), 2);
            let element = &posts.templates()[0];
            assert!(matches!(&**element, PoolElement::Mod { id, .. } if &**id == "test:post"));
            assert_eq!(
                element.bounding_box(&resources, [1, 2, 3], Rotation::None),
                BoundingBox::new([1, 2, 3], [1, 5, 3])
            );
            assert_eq!(
                element.save(),
                Tag::compound([
                    ("height", Tag::Int(4)),
                    ("element_type", Tag::String("test:post".into())),
                ])
            );
            assert_eq!(posts.max_size(&resources), 4);
            assert_eq!(element.projection(), Projection::Rigid);
            let listed = pools.get("test:listed", &resources, &bindings).unwrap();
            let PoolElement::List { elements, .. } = &*listed.templates()[0] else {
                panic!("a list element");
            };
            assert_eq!(elements[0].projection(), Projection::TerrainMatching);
            assert!(pools.get("test:unknown", &resources, &bindings).is_none());
            assert!(pools.get("test:refused", &resources, &bindings).is_none());
        });
    }
}
