//! Version-pinned cached block collision data, without entity-context behavior.
//!
//! Java's no-context BlockState.getCollisionShape uses its cache when present.
//! The overload taking CollisionContext always calls block behavior instead.
//! This module covers the cached branch only; uncached states remain explicit.
use crate::collision_context::EntityContextKind;
use std::{collections::HashMap, io};
use sunrise_core::geometry::{Axis, VoxelShape};
use {
    crate::{
        DATA_VERSION,
        chunk::BlockSection,
        compression::invalid,
        registry::{BlockRegistry, BlockStateId, BlockTable},
    },
    sunrise_core::nbt::Tag,
};

/// `PipeBlock`'s shape for `size` pixels across (`makeShapes`): the core
/// cube, with an arm from the centre out to each joined side (`sides` by
/// `Direction`: down, up, north, south, west, east).
pub fn pipe_shape(size: f64, sides: [bool; 6]) -> io::Result<VoxelShape> {
    use sunrise_core::geometry::{Aabb, ShapeOp, Vec3};
    let (lo, hi) = ((8.0 - size / 2.0) / 16.0, (8.0 + size / 2.0) / 16.0);
    let shape = |min: [f64; 3], max: [f64; 3]| {
        VoxelShape::from_box(Aabb::new(
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        ))
        .map_err(|e| invalid(e.to_string()))
    };
    let mut result = shape([lo; 3], [hi; 3])?;
    let arms = [
        ([lo, 0.0, lo], [hi, 0.5, hi]),
        ([lo, 0.5, lo], [hi, 1.0, hi]),
        ([lo, lo, 0.0], [hi, hi, 0.5]),
        ([lo, lo, 0.5], [hi, hi, 1.0]),
        ([0.0, lo, lo], [0.5, hi, hi]),
        ([0.5, lo, lo], [1.0, hi, hi]),
    ];
    for (joined, (min, max)) in sides.into_iter().zip(arms) {
        if joined {
            result = shape(min, max)?
                .join(&result, ShapeOp::OR)
                .map_err(|e| invalid(e.to_string()))?;
        }
    }
    Ok(result)
}

#[derive(PartialEq, Eq, Hash)]
struct GridKey {
    coords: [Vec<u64>; 3],
    occupied: Vec<u8>,
    cube: bool,
    axes: u8,
    min: [i32; 3],
    max: [i32; 3],
}

/// Declared shape channels translated by the state's per-column offset.
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum ShapeChannel {
    Collision = 1,
    ContextCollision = 2,
    ContextOutline = 4,
    Interaction = 8,
}
#[derive(Clone, Copy)]
struct ShapeOffset {
    channels: u8,
    limits: (f32, f32),
}

pub(crate) fn valid_offset_limits((horizontal, vertical): (f32, f32)) -> bool {
    (0.0..=0.25).contains(&horizontal) && (0.0..=0.2).contains(&vertical)
}

pub(crate) fn translate_shape(shape: &VoxelShape, [x, y, z]: [f64; 3]) -> VoxelShape {
    if [x, y, z] == [0.0; 3] {
        shape.clone()
    } else {
        shape.moved(sunrise_core::geometry::Vec3::new(x, y, z))
    }
}

pub struct CachedCollisionRegistry<'r> {
    registry: &'r BlockRegistry,
    shapes: Vec<VoxelShape>,
    state_shapes: Vec<i32>,
    grid_count: usize,
    canonical_identities: bool,
    contexts: HashMap<BlockStateId, [Option<usize>; 9]>,
    outline_edges: HashMap<usize, Vec<[f64; 6]>>,
    /// Whether each state's block entity may publish its shapes (`shapes_from_block_entity`);
    /// empty when no state does.
    published: Vec<bool>,
    offsets: Vec<Option<ShapeOffset>>,
    /// Whether each state's block declares scaffolding's context collision
    /// (`context_collision`); empty when no state does.
    scaffolding: Vec<bool>,
}

/// The states a table lists under `key` (an int array), as one flag per registry state; empty
/// when the table has no such list.
fn state_flags(tag: &Tag, key: &str, states: usize) -> io::Result<Vec<bool>> {
    let Some(listed) = tag.get(key) else {
        return Ok(Vec::new());
    };
    let Tag::IntArray(listed) = listed else {
        return Err(invalid(format!("{key} is not an int array")));
    };
    let mut flags = vec![false; states];
    for state in listed {
        *usize::try_from(*state)
            .ok()
            .and_then(|state| flags.get_mut(state))
            .ok_or_else(|| invalid(format!("{key} state outside registry")))? = true;
    }
    Ok(flags)
}
impl<'r> CachedCollisionRegistry<'r> {
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    pub fn from_nbt(tag: &Tag, registry: &'r BlockRegistry) -> io::Result<Self> {
        Self::from_table(BlockTable::check(tag, registry, "collision")?)
    }
    pub fn from_table(table: BlockTable<'_, 'r>) -> io::Result<Self> {
        let (tag, registry) = (table.tag(), table.registry());
        if tag.get("DataVersion") != Some(&Tag::Int(DATA_VERSION)) {
            return Err(invalid("collision DataVersion mismatch"));
        }
        let Some(Tag::List(entries)) = tag.get("shapes") else {
            return Err(invalid("missing collision shapes"));
        };
        // Each state can add four outline, four collision and one interaction shape.
        if entries.len() > registry.len().saturating_mul(10) {
            return Err(invalid("too many cached collision shapes"));
        }
        let mut shapes = Vec::with_capacity(entries.len());
        let mut canonical_identities = true;
        let mut seen_canonical = [false; 3];
        let mut grids = HashMap::<GridKey, VoxelShape>::new();
        for entry in entries {
            let canonical = match entry.get("canonical") {
                None => {
                    canonical_identities = false;
                    0
                }
                Some(Tag::Byte(value @ 0..=2)) => *value as usize,
                _ => return Err(invalid("invalid canonical collision identity")),
            };
            if canonical != 0 {
                if seen_canonical[canonical] {
                    return Err(invalid("duplicate canonical collision identity"));
                }
                seen_canonical[canonical] = true;
            }
            let mut coords: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::new());
            for (i, axis) in ["x", "y", "z"].into_iter().enumerate() {
                let Some(Tag::LongArray(bits)) = entry.get(axis) else {
                    return Err(invalid("missing collision coordinates"));
                };
                coords[i] = bits.iter().map(|b| f64::from_bits(*b as u64)).collect();
            }
            let Some(Tag::ByteArray(bits)) = entry.get("occupied") else {
                return Err(invalid("missing collision occupancy"));
            };
            if bits.iter().any(|b| *b != 0 && *b != 1) {
                return Err(invalid("invalid collision occupancy"));
            }
            let occupied = bits.iter().map(|b| *b != 0).collect::<Vec<_>>();
            let Some(Tag::Byte(cube_axes)) = entry.get("cube_axes") else {
                return Err(invalid("missing cube-coordinate metadata"));
            };
            if *cube_axes < 0 || *cube_axes > 7 {
                return Err(invalid("invalid cube-coordinate metadata"));
            }
            let coordinate_cubes = [cube_axes & 1 != 0, cube_axes & 2 != 0, cube_axes & 4 != 0];
            let discrete_bounds = |name| -> io::Result<[i32; 3]> {
                let Some(Tag::IntArray(values)) = entry.get(name) else {
                    return Err(invalid("missing collision discrete extrema"));
                };
                values
                    .as_slice()
                    .try_into()
                    .map_err(|_| invalid("invalid collision discrete extrema length"))
            };
            let min = discrete_bounds("discrete_min")?;
            let max = discrete_bounds("discrete_max")?;
            let cube = match entry.get("cube") {
                Some(Tag::Byte(0)) => false,
                Some(Tag::Byte(1)) => true,
                _ => return Err(invalid("invalid collision cube flag")),
            };
            let key = GridKey {
                coords: std::array::from_fn(|i| coords[i].iter().map(|v| v.to_bits()).collect()),
                occupied: bits.clone(),
                cube,
                axes: *cube_axes as u8,
                min,
                max,
            };
            let shape = if let Some(existing) = grids.get(&key) {
                existing.fresh_identity()
            } else {
                match entry.get("cube") {
                    Some(Tag::Byte(0)) => {
                        VoxelShape::from_mixed_grid(coords, &occupied, coordinate_cubes)
                    }
                    Some(Tag::Byte(1)) => {
                        if *cube_axes != 7 {
                            return Err(invalid("cube shape has array-coordinate metadata"));
                        }
                        let sizes = std::array::from_fn(|i| coords[i].len().saturating_sub(1));
                        let shape = VoxelShape::from_cube(sizes, &occupied)
                            .map_err(|e| invalid(e.to_string()))?;
                        for axis in Axis::ALL {
                            if shape
                                .coordinates(axis)
                                .iter()
                                .map(|v| v.to_bits())
                                .ne(coords[axis as usize].iter().map(|v| v.to_bits()))
                            {
                                return Err(invalid("cube collision coordinates differ"));
                            }
                        }
                        Ok(shape)
                    }
                    _ => return Err(invalid("invalid collision cube flag")),
                }
                .map_err(|e| invalid(e.to_string()))?
                .with_discrete_bounds(min, max)
                .map_err(|e| invalid(e.to_string()))?
            };
            let shape = if canonical == 0 {
                shape
            } else {
                let reference = if canonical == 1 {
                    VoxelShape::empty()
                } else {
                    VoxelShape::block()
                };
                let expected_count = usize::from(canonical == 2);
                if cube != reference.is_cube()
                    || coordinate_cubes != reference.cube_coordinate_axes()
                    || occupied != vec![true; expected_count]
                    || min != [0; 3]
                    || max != [expected_count as i32; 3]
                    || Axis::ALL.into_iter().any(|axis| {
                        key.coords[axis as usize]
                            .iter()
                            .copied()
                            .ne(reference.coordinates(axis).iter().map(|v| v.to_bits()))
                    })
                {
                    return Err(invalid(
                        "canonical collision identity has different geometry",
                    ));
                }
                reference
            };
            grids.entry(key).or_insert_with(|| shape.clone());
            shapes.push(shape);
        }
        let Some(Tag::IntArray(state_shapes)) = tag.get("cached_shapes") else {
            return Err(invalid("missing cached shape mapping"));
        };
        if state_shapes.len() != registry.len()
            || state_shapes
                .iter()
                .any(|id| *id < -1 || *id >= shapes.len() as i32)
        {
            return Err(invalid("invalid cached shape mapping"));
        }
        let mut contexts = HashMap::new();
        let mut outline_edges = HashMap::new();
        if let Some(rows) = tag.get("context_shapes") {
            let Tag::List(rows) = rows else {
                return Err(invalid("context shapes is not a list"));
            };
            if rows.len() > registry.len() {
                return Err(invalid("too many context shape rows"));
            }
            const KEYS: [&str; 9] = [
                "outline_item",
                "outline_player",
                "outline_living",
                "outline_other",
                "collision_item",
                "collision_player",
                "collision_living",
                "collision_other",
                "interaction",
            ];
            for row in rows {
                let Some(Tag::Int(state)) = row.get("state") else {
                    return Err(invalid("context shape missing state"));
                };
                if *state < 0 || *state as usize >= registry.len() {
                    return Err(invalid("context shape state outside registry"));
                }
                let Tag::Compound(fields) = row else {
                    return Err(invalid("context shape row is not a compound"));
                };
                if fields.keys().any(|key| {
                    key.to_string()
                        .map_or(true, |key| key != "state" && !KEYS.contains(&key.as_str()))
                }) {
                    return Err(invalid("unknown context shape field"));
                }
                let mut ids = [None; 9];
                for (i, key) in KEYS.into_iter().enumerate() {
                    let Some(id) = row.get(key) else {
                        continue;
                    };
                    let Tag::Int(id) = id else {
                        return Err(invalid("context shape reference is not an int"));
                    };
                    if *id < 0 || *id as usize >= shapes.len() {
                        return Err(invalid("context shape reference outside shapes"));
                    }
                    ids[i] = Some(*id as usize);
                    if i < 4 {
                        outline_edges
                            .entry(*id as usize)
                            .or_insert_with(|| shapes[*id as usize].edges());
                    }
                }
                if ids.iter().all(Option::is_none)
                    || contexts.insert(BlockStateId(*state as u32), ids).is_some()
                {
                    return Err(invalid("empty or duplicate context shape row"));
                }
            }
        }
        let published = state_flags(tag, "published_shapes", registry.len())?;
        let scaffolding = state_flags(tag, "scaffolding_collision", registry.len())?;
        let mut offsets = Vec::new();
        if let Some(rows) = tag.get("shape_offsets") {
            let Tag::List(rows) = rows else {
                return Err(invalid("shape_offsets must be a list"));
            };
            offsets = vec![None; registry.len()];
            for row in rows {
                let (
                    Some(Tag::Int(state)),
                    Some(Tag::Byte(channels)),
                    Some(Tag::Float(horizontal)),
                    Some(Tag::Float(vertical)),
                ) = (
                    row.get("state"),
                    row.get("channels"),
                    row.get("horizontal"),
                    row.get("vertical"),
                )
                else {
                    return Err(invalid("invalid shape offset record"));
                };
                let horizontal = f32::from_bits(*horizontal);
                let vertical = f32::from_bits(*vertical);
                let slot = usize::try_from(*state)
                    .ok()
                    .and_then(|id| offsets.get_mut(id))
                    .ok_or_else(|| invalid("shape offset state outside registry"))?;
                if slot.is_some()
                    || !(1..=15).contains(channels)
                    || !valid_offset_limits((horizontal, vertical))
                {
                    return Err(invalid(
                        "shape offset: duplicate state, invalid channels or limits",
                    ));
                }
                *slot = Some(ShapeOffset {
                    channels: *channels as u8,
                    limits: (horizontal, vertical),
                });
            }
        }
        Ok(Self {
            registry,
            shapes,
            state_shapes: state_shapes.clone(),
            grid_count: grids.len(),
            canonical_identities,
            contexts,
            outline_edges,
            published,
            offsets,
            scaffolding,
        })
    }
    /// Whether the block entity of `state`'s block publishes the block's shapes
    /// (`shapes_from_block_entity`), which then replace the cached ones while it holds some.
    pub fn publishes_shapes(&self, state: BlockStateId) -> bool {
        self.published
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
    /// Nonzero offset limits for a declared channel; none for a block-local shape.
    pub(crate) fn offset_limits(
        &self,
        state: BlockStateId,
        channel: ShapeChannel,
    ) -> Option<(f32, f32)> {
        self.offsets
            .get(state.0 as usize)
            .copied()
            .flatten()
            .filter(|offset| offset.channels & channel as u8 != 0 && offset.limits != (0.0, 0.0))
            .map(|offset| offset.limits)
    }

    /// The per-position translation for a declared channel. Publication is resolved separately.
    pub fn shape_offset(
        &self,
        state: BlockStateId,
        channel: ShapeChannel,
        position: [i32; 3],
    ) -> [f64; 3] {
        self.offset_limits(state, channel)
            .map_or([0.0; 3], |limits| {
                sunrise_core::random::column_offset(limits, position[0], position[2])
            })
    }
    /// A block-local declared shape at this position, never changing its cached source.
    pub fn positioned_shape(
        &self,
        state: BlockStateId,
        channel: ShapeChannel,
        position: [i32; 3],
        shape: &VoxelShape,
    ) -> VoxelShape {
        translate_shape(shape, self.shape_offset(state, channel, position))
    }
    /// Sample the cached default collision at the position without allocating a moved shape.
    /// The two transverse coordinates follow `VoxelShape::max_at`'s cyclic order (Z, X for Y).
    pub fn cached_collision_top(
        &self,
        state: BlockStateId,
        position: [i32; 3],
        first: f64,
        second: f64,
    ) -> io::Result<Option<f64>> {
        self.cached_shape(state).map(|shape| {
            shape.map(|shape| {
                if shape.is_empty() {
                    return 0.0;
                }
                let [x, y, z] = self.shape_offset(state, ShapeChannel::Collision, position);
                shape.max_at(Axis::Y, first - z, second - x) + y
            })
        })
    }
    /// Whether translated collision geometry can leave the cell, for collision scan bounds.
    pub fn offset_collision(&self, state: BlockStateId) -> bool {
        self.offsets
            .get(state.0 as usize)
            .copied()
            .flatten()
            .is_some_and(|offset| offset.channels & 3 != 0 && offset.limits != (0.0, 0.0))
    }
    /// Whether `state`'s block declares scaffolding's context collision (`context_collision`).
    pub fn declares_scaffolding_collision(&self, state: BlockStateId) -> bool {
        self.scaffolding
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
    pub fn has_context_shapes(&self, state: BlockStateId) -> bool {
        self.contexts.contains_key(&state)
    }
    fn context_shape_id(
        &self,
        state: BlockStateId,
        kind: EntityContextKind,
        offset: usize,
    ) -> Option<usize> {
        let kind = kind.index()?;
        self.contexts.get(&state)?[offset + kind]
    }
    /// A declared outline override for an actual entity; no entity uses the ordinary outline.
    pub fn context_outline_shape(
        &self,
        state: BlockStateId,
        kind: EntityContextKind,
    ) -> Option<&VoxelShape> {
        Some(&self.shapes[self.context_shape_id(state, kind, 0)?])
    }
    pub fn context_outline_edges(
        &self,
        state: BlockStateId,
        kind: EntityContextKind,
    ) -> Option<&[[f64; 6]]> {
        Some(
            self.outline_edges
                .get(&self.context_shape_id(state, kind, 0)?)?
                .as_slice(),
        )
    }
    pub fn context_collision_shape(
        &self,
        state: BlockStateId,
        kind: EntityContextKind,
    ) -> Option<&VoxelShape> {
        Some(&self.shapes[self.context_shape_id(state, kind, 4)?])
    }
    pub fn interaction_shape(&self, state: BlockStateId) -> Option<&VoxelShape> {
        Some(&self.shapes[self.contexts.get(&state)?[8]?])
    }
    /// Every declared physical variant participates in conservative cursor metadata.
    pub fn collision_variants(&self, state: BlockStateId) -> impl Iterator<Item = &VoxelShape> {
        self.contexts
            .get(&state)
            .into_iter()
            .flat_map(|ids| ids[4..8].iter())
            .filter_map(|id| id.map(|id| &self.shapes[id]))
    }
    /// Older exports omit native canonical identities. Such imports retain their
    /// local alias graph but cannot support identity-dependent world iteration.
    pub fn has_canonical_identities(&self) -> bool {
        self.canonical_identities
    }
    pub fn shape_count(&self) -> usize {
        self.shapes.len()
    }
    pub fn unique_grid_count(&self) -> usize {
        self.grid_count
    }
    pub fn cached_state_count(&self) -> usize {
        self.state_shapes.iter().filter(|id| **id >= 0).count()
    }
    /// None means vanilla must call block behavior for this state. It does not
    /// mean air, an empty shape, or permission to ignore a collision.
    pub fn cached_shape(&self, state: BlockStateId) -> io::Result<Option<&VoxelShape>> {
        let id = *self
            .state_shapes
            .get(state.0 as usize)
            .ok_or_else(|| invalid("collision state outside registry"))?;
        Ok(if id < 0 {
            None
        } else {
            Some(&self.shapes[id as usize])
        })
    }
    pub fn section_shape(
        &self,
        section: &BlockSection<'_>,
        index: usize,
    ) -> io::Result<Option<&VoxelShape>> {
        if !std::ptr::eq(section.registry(), self.registry) {
            return Err(invalid("section uses a different collision registry"));
        }
        self.cached_shape(section.get(index)?)
    }
}

#[cfg(test)]
mod pipe_tests {
    use super::*;
    use crate::registry::BlockTableFile;
    use std::path::PathBuf;
    use sunrise_core::geometry::ShapeOp;

    /// `pipe_shape` at the chorus plant's 10 pixels is vanilla's own shape for
    /// each of its 64 states.
    #[test]
    fn pipe_shapes_match_the_chorus_plant() {
        let root = std::env::var_os("SUNRISE_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        if !root.join("cached-collision.nbt").exists() {
            eprintln!("skipping: no resources at {}", root.display());
            return;
        }
        let registry = crate::level::data::load_registry(&root).unwrap();
        let file = BlockTableFile::open(&root.join("cached-collision.nbt"), &registry).unwrap();
        let cached = CachedCollisionRegistry::from_table(file.table("collision").unwrap()).unwrap();
        let mut checked = 0;
        for id in 0..registry.len() as u32 {
            let state = BlockStateId(id);
            let s = registry.state(state).unwrap();
            if s.name != "minecraft:chorus_plant" {
                continue;
            }
            let sides = ["down", "up", "north", "south", "west", "east"]
                .map(|k| s.properties.get(k).map(String::as_str) == Some("true"));
            let ours = pipe_shape(10.0, sides).unwrap();
            let theirs = cached.cached_shape(state).unwrap().unwrap();
            assert!(
                !ours.join_is_not_empty(theirs, ShapeOp::XOR).unwrap(),
                "{sides:?}"
            );
            assert_eq!(ours.boxes(), theirs.boxes(), "{sides:?}");
            checked += 1;
        }
        assert_eq!(checked, 64);
    }
}
