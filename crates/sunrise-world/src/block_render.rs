//! Neighbor face visibility over immutable block definitions and occlusion grids.
//! This decides whether a culled model face is emitted; it does not shade it or
//! implement translucent sorting, fluid meshing, or block updates.
use crate::{
    chunk::BlockState,
    collision::CachedCollisionRegistry,
    compression::invalid,
    fluid::{FluidKind, FluidState},
    registry::{BlockRegistry, BlockStateId},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use sunrise_core::geometry::{Direction, ShapeOp, VoxelShape};

const NO_RULES: u32 = u32::MAX;
const FLAG_CAN_OCCLUDE: u8 = 1 << 0;
const FLAG_BARS_TAG: u8 = 1 << 1;
const FLAG_SUPPORTS_EXTERNAL: u8 = 1 << 2;
const FLAG_IS_AIR: u8 = 1 << 3;
const FLAG_IS_SOLID_RENDER: u8 = 1 << 4;
const FLAG_LIQUID: u8 = 1 << 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipRendering {
    Never,
    SameBlock,
    Bars,
    Leaves,
    Liquid,
    MangroveRoots,
    /// Per-state compiled data rules (`FaceVisibility::rule_lists[index]`).
    Rules(u32),
}

#[derive(Clone, Debug)]
pub struct RenderDefinition {
    pub skip: SkipRendering,
    pub can_occlude: bool,
    pub bars_tag: bool,
    /// Data rules replacing `skip` (`BlockBehaviour.skipRendering`). `None` keeps `skip`.
    pub skip_rules: Option<Vec<mod_api::FaceRule>>,
    /// Data rules for NeoForge `IBlockExtension.hidesNeighborFace`.
    pub hides_rules: Vec<mod_api::FaceRule>,
    /// NeoForge `IBlockExtension.supportsExternalFaceHiding` (default `true`).
    pub supports_external_face_hiding: bool,
}

impl RenderDefinition {
    /// A vanilla definition with no data face rules and external face hiding enabled.
    pub fn simple(skip: SkipRendering, can_occlude: bool, bars_tag: bool) -> Self {
        Self {
            skip,
            can_occlude,
            bars_tag,
            skip_rules: None,
            hides_rules: Vec::new(),
            supports_external_face_hiding: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CompiledRule {
    faces: u8,
    fact_mask: u8,
    fact_expected: u8,
    neighbors: NeighborSet,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum NeighborSet {
    /// Matches any neighbor state without listing every state in the registry.
    Any,
    /// Sorted, non-overlapping inclusive ranges `(start, end)` of `BlockStateId.0`.
    Ranges(Box<[(u32, u32)]>),
}

impl NeighborSet {
    #[inline]
    fn contains(&self, id: u32) -> bool {
        match self {
            Self::Any => true,
            Self::Ranges(ranges) => {
                let idx = ranges.partition_point(|&(_, end)| end < id);
                ranges.get(idx).is_some_and(|&(start, _)| id >= start)
            }
        }
    }
}

/// Source rules for blocks that reference at least one `#tag` neighbor, retained so a
/// client tag sync (join or `/reload`) can recompile those blocks without reloading
/// occlusion geometry.
struct TagRuleSource {
    skip_blocks: BTreeMap<String, Vec<mod_api::FaceRule>>,
    hides_blocks: BTreeMap<String, Vec<mod_api::FaceRule>>,
}

#[derive(Clone, Copy)]
struct State {
    block: BlockStateId,
    faces: [u32; 6],
    skip: SkipRendering,
    hides: u32,
    flags: u8,
    connections: u8,
    connection_properties: u8,
    fluid: FluidKind,
}

/// Prepared face geometry and a bounded cache shared across state pairs. Geometry
/// is deduplicated by the imported immutable shape, not copied for every state.
/// A clone (each meshing worker keeps one, with its own cache) shares the states,
/// faces and compiled rule tables, which never change once made.
#[derive(Clone)]
pub struct FaceVisibility {
    states: Arc<[State]>,
    faces: Arc<[VoxelShape]>,
    rule_lists: Arc<[Box<[CompiledRule]>]>,
    tag_sources: Option<Arc<TagRuleSource>>,
    cache: HashMap<(u32, u32), bool>,
}

fn rule_uses_tag(rule: &mod_api::FaceRule) -> bool {
    matches!(rule.neighbor, mod_api::FaceNeighbor::Tag { .. })
}

fn facts_mask(facts: mod_api::FaceFacts) -> (u8, u8) {
    let mut mask = 0u8;
    let mut expected = 0u8;
    for (flag, value) in [
        (FLAG_IS_AIR, facts.is_air),
        (FLAG_CAN_OCCLUDE, facts.can_occlude),
        (FLAG_IS_SOLID_RENDER, facts.is_solid_render),
        (FLAG_LIQUID, facts.liquid),
    ] {
        if let Some(want) = value {
            mask |= flag;
            if want {
                expected |= flag;
            }
        }
    }
    (mask, expected)
}

fn coalesce_ranges(ids: &[u32]) -> Box<[(u32, u32)]> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for &id in ids {
        if let Some(last) = out.last_mut()
            && id <= last.1.saturating_add(1)
        {
            if id > last.1 {
                last.1 = id;
            }
        } else {
            out.push((id, id));
        }
    }
    out.into_boxed_slice()
}

fn matches_properties(
    state: &BlockState,
    required: &BTreeMap<String, String>,
    same: &[(&str, &str)],
) -> bool {
    for (k, v) in required {
        if state.properties.get(k) != Some(v) {
            return false;
        }
    }
    for &(k, v) in same {
        if state.properties.get(k).map(String::as_str) != Some(v) {
            return false;
        }
    }
    true
}

fn compile_rule_for_state(
    rule: &mod_api::FaceRule,
    own_state: &BlockState,
    registry: &BlockRegistry,
    block_states: &HashMap<&str, Vec<u32>>,
    block_tags: &BTreeMap<String, BTreeSet<String>>,
) -> Option<CompiledRule> {
    let faces = rule.faces.mask();
    if faces == 0 {
        return None;
    }
    for (k, v) in &rule.state {
        if own_state.properties.get(k) != Some(v) {
            return None;
        }
    }
    let mut same_values: Vec<(&str, &str)> = Vec::with_capacity(rule.same.len());
    for prop in &rule.same {
        let val = own_state.properties.get(prop)?;
        same_values.push((prop.as_str(), val.as_str()));
    }
    let (fact_mask, fact_expected) = facts_mask(rule.facts);
    let neighbors = match &rule.neighbor {
        mod_api::FaceNeighbor::Any => {
            if same_values.is_empty() {
                NeighborSet::Any
            } else {
                let mut ids = Vec::new();
                for id in 0..registry.len() {
                    if let Some(candidate) = registry.state(BlockStateId(id as u32))
                        && matches_properties(candidate, &BTreeMap::new(), &same_values)
                    {
                        ids.push(id as u32);
                    }
                }
                if ids.is_empty() {
                    return None;
                }
                NeighborSet::Ranges(coalesce_ranges(&ids))
            }
        }
        mod_api::FaceNeighbor::SameBlock { state: req } => {
            let candidates = block_states.get(own_state.name.as_str())?;
            let ids: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|&id| {
                    registry
                        .state(BlockStateId(id))
                        .is_some_and(|s| matches_properties(s, req, &same_values))
                })
                .collect();
            if ids.is_empty() {
                return None;
            }
            NeighborSet::Ranges(coalesce_ranges(&ids))
        }
        mod_api::FaceNeighbor::Block { id, state: req } => {
            let candidates = block_states.get(id.as_str())?;
            let ids: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|&cid| {
                    registry
                        .state(BlockStateId(cid))
                        .is_some_and(|s| matches_properties(s, req, &same_values))
                })
                .collect();
            if ids.is_empty() {
                return None;
            }
            NeighborSet::Ranges(coalesce_ranges(&ids))
        }
        mod_api::FaceNeighbor::Tag { id, state: req } => {
            let members = block_tags.get(id.as_str())?;
            let mut ids = Vec::new();
            for block_name in members {
                if let Some(candidates) = block_states.get(block_name.as_str()) {
                    for &cid in candidates {
                        if registry
                            .state(BlockStateId(cid))
                            .is_some_and(|s| matches_properties(s, req, &same_values))
                        {
                            ids.push(cid);
                        }
                    }
                }
            }
            if ids.is_empty() {
                return None;
            }
            ids.sort_unstable();
            ids.dedup();
            NeighborSet::Ranges(coalesce_ranges(&ids))
        }
    };
    Some(CompiledRule {
        faces,
        fact_mask,
        fact_expected,
        neighbors,
    })
}

fn compile_rules_for_state(
    rules: &[mod_api::FaceRule],
    own_state: &BlockState,
    registry: &BlockRegistry,
    block_states: &HashMap<&str, Vec<u32>>,
    block_tags: &BTreeMap<String, BTreeSet<String>>,
) -> Box<[CompiledRule]> {
    rules
        .iter()
        .filter_map(|rule| {
            compile_rule_for_state(rule, own_state, registry, block_states, block_tags)
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn intern_rules(
    compiled: Box<[CompiledRule]>,
    rule_lists: &mut Vec<Box<[CompiledRule]>>,
    dedup: &mut HashMap<Box<[CompiledRule]>, u32>,
) -> u32 {
    if let Some(&idx) = dedup.get(&compiled) {
        return idx;
    }
    let idx = rule_lists.len() as u32;
    dedup.insert(compiled.clone(), idx);
    rule_lists.push(compiled);
    idx
}

fn block_states_index(registry: &BlockRegistry) -> HashMap<&str, Vec<u32>> {
    let mut map: HashMap<&str, Vec<u32>> = HashMap::new();
    for id in 0..registry.len() {
        if let Some(state) = registry.state(BlockStateId(id as u32)) {
            map.entry(state.name.as_str()).or_default().push(id as u32);
        }
    }
    map
}

impl FaceVisibility {
    /// No state's faces, for a server, which draws nothing: every lookup
    /// answers "unknown render state".
    pub fn empty() -> Self {
        Self {
            states: Arc::new([]),
            faces: Arc::new([VoxelShape::empty(), VoxelShape::block()]),
            rule_lists: Arc::new([]),
            tag_sources: None,
            cache: HashMap::new(),
        }
    }

    /// [`Self::empty`]: no state is known.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Whether any block in this visibility table has a tag-based face rule.
    pub fn has_tag_rules(&self) -> bool {
        self.tag_sources.is_some()
    }

    pub fn new(
        occlusion: &CachedCollisionRegistry<'_>,
        definitions: &BTreeMap<String, RenderDefinition>,
    ) -> io::Result<Self> {
        Self::new_with_tags(occlusion, definitions, &BTreeMap::new())
    }

    pub fn new_with_tags(
        occlusion: &CachedCollisionRegistry<'_>,
        definitions: &BTreeMap<String, RenderDefinition>,
        block_tags: &BTreeMap<String, BTreeSet<String>>,
    ) -> io::Result<Self> {
        let registry = occlusion.registry();
        let has_any_rules = definitions
            .values()
            .any(|d| d.skip_rules.is_some() || !d.hides_rules.is_empty());
        let block_states = if has_any_rules {
            block_states_index(registry)
        } else {
            HashMap::new()
        };
        let mut tag_skip_blocks: BTreeMap<String, Vec<mod_api::FaceRule>> = BTreeMap::new();
        let mut tag_hides_blocks: BTreeMap<String, Vec<mod_api::FaceRule>> = BTreeMap::new();
        for (name, def) in definitions {
            if let Some(rules) = &def.skip_rules
                && rules.iter().any(rule_uses_tag)
            {
                tag_skip_blocks.insert(name.clone(), rules.clone());
            }
            if def.hides_rules.iter().any(rule_uses_tag) {
                tag_hides_blocks.insert(name.clone(), def.hides_rules.clone());
            }
        }
        let mut states = Vec::with_capacity(registry.len());
        // IDs 0 and 1 represent the canonical empty/full face singletons used
        // by BlockState.initCache; other empty-like shapes retain their identity.
        let mut faces = vec![VoxelShape::empty(), VoxelShape::block()];
        let mut shape_faces: HashMap<*const VoxelShape, ([u32; 6], bool)> = HashMap::new();
        let mut rule_lists: Vec<Box<[CompiledRule]>> = Vec::new();
        let mut dedup: HashMap<Box<[CompiledRule]>, u32> = HashMap::new();
        for id in 0..registry.len() {
            let id = BlockStateId(id as u32);
            let state = registry
                .state(id)
                .ok_or_else(|| invalid("unknown render state"))?;
            let definition = definitions
                .get(&state.name)
                .ok_or_else(|| invalid("missing render definition"))?;
            let shape = occlusion
                .cached_shape(id)?
                .ok_or_else(|| invalid("missing occlusion geometry"))?;
            let (face_ids, is_solid_render) = if !definition.can_occlude || shape.is_empty() {
                ([0; 6], false)
            } else if let Some(&entry) = shape_faces.get(&std::ptr::from_ref(shape)) {
                entry
            } else {
                let mut ids = [1; 6];
                let not_full = shape
                    .join_is_not_empty(&faces[1], ShapeOp::XOR)
                    .map_err(|e| invalid(e.to_string()))?;
                if not_full {
                    for direction in Direction::ALL {
                        let face = shape
                            .face_shape(direction)
                            .map_err(|e| invalid(e.to_string()))?;
                        ids[direction as usize] = if face.same_identity(&faces[0]) {
                            0
                        } else if face.same_identity(&faces[1]) {
                            1
                        } else {
                            let id = faces.len() as u32;
                            faces.push(face);
                            id
                        };
                    }
                }
                let solid = !not_full;
                shape_faces.insert(std::ptr::from_ref(shape), (ids, solid));
                (ids, solid)
            };
            let mut connections = 0;
            let mut connection_properties = 0;
            for (name, direction) in [
                ("north", Direction::North),
                ("south", Direction::South),
                ("west", Direction::West),
                ("east", Direction::East),
            ] {
                if let Some(value @ ("false" | "true")) =
                    state.properties.get(name).map(String::as_str)
                {
                    connection_properties |= 1 << direction as u8;
                    if value == "true" {
                        connections |= 1 << direction as u8;
                    }
                }
            }
            let skip = if let Some(rules) = &definition.skip_rules {
                let compiled =
                    compile_rules_for_state(rules, state, registry, &block_states, block_tags);
                if compiled.is_empty() {
                    SkipRendering::Never
                } else {
                    SkipRendering::Rules(intern_rules(compiled, &mut rule_lists, &mut dedup))
                }
            } else {
                if definition.skip == SkipRendering::Bars && connection_properties != 0b111100 {
                    return Err(invalid("bars rendering requires four boolean connections"));
                }
                definition.skip
            };
            let hides = if definition.hides_rules.is_empty() {
                NO_RULES
            } else {
                let compiled = compile_rules_for_state(
                    &definition.hides_rules,
                    state,
                    registry,
                    &block_states,
                    block_tags,
                );
                if compiled.is_empty() {
                    NO_RULES
                } else {
                    intern_rules(compiled, &mut rule_lists, &mut dedup)
                }
            };
            let fluid = FluidState::from_block(state)?.kind();
            let mut flags = 0u8;
            if definition.can_occlude {
                flags |= FLAG_CAN_OCCLUDE;
            }
            if definition.bars_tag {
                flags |= FLAG_BARS_TAG;
            }
            if definition.supports_external_face_hiding {
                flags |= FLAG_SUPPORTS_EXTERNAL;
            }
            if matches!(
                state.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            ) {
                flags |= FLAG_IS_AIR;
            }
            if is_solid_render {
                flags |= FLAG_IS_SOLID_RENDER;
            }
            if matches!(
                state.name.as_str(),
                "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
            ) || matches!(fluid, FluidKind::Mod(_) | FluidKind::ModFlowing(_))
            {
                flags |= FLAG_LIQUID;
            }
            states.push(State {
                block: registry
                    .default_state(&state.name)
                    .ok_or_else(|| invalid("missing render block identity"))?,
                faces: face_ids,
                skip,
                hides,
                flags,
                connections,
                connection_properties,
                fluid,
            });
        }
        let tag_sources = if tag_skip_blocks.is_empty() && tag_hides_blocks.is_empty() {
            None
        } else {
            Some(Arc::new(TagRuleSource {
                skip_blocks: tag_skip_blocks,
                hides_blocks: tag_hides_blocks,
            }))
        };
        Ok(Self {
            states: states.into(),
            faces: faces.into(),
            rule_lists: rule_lists.into(),
            tag_sources,
            cache: HashMap::new(),
        })
    }

    /// Recompile tag-based face rules against `block_tags`. Returns `None` when no block
    /// uses tag rules or when the compiled tables are identical to the current ones.
    pub fn rebuild_for_tags(
        &self,
        registry: &BlockRegistry,
        block_tags: &BTreeMap<String, BTreeSet<String>>,
    ) -> Option<Self> {
        let sources = self.tag_sources.as_ref()?;
        let block_states = block_states_index(registry);
        let mut new_states = self.states.to_vec();
        let mut new_rule_lists: Vec<Box<[CompiledRule]>> = Vec::new();
        let mut dedup: HashMap<Box<[CompiledRule]>, u32> = HashMap::new();
        for (id, entry) in new_states.iter_mut().enumerate() {
            let state = registry.state(BlockStateId(id as u32))?;
            if let Some(rules) = sources.skip_blocks.get(&state.name) {
                let compiled =
                    compile_rules_for_state(rules, state, registry, &block_states, block_tags);
                entry.skip = if compiled.is_empty() {
                    SkipRendering::Never
                } else {
                    SkipRendering::Rules(intern_rules(compiled, &mut new_rule_lists, &mut dedup))
                };
            } else if let SkipRendering::Rules(old_idx) = entry.skip {
                let cloned = self.rule_lists[old_idx as usize].clone();
                entry.skip =
                    SkipRendering::Rules(intern_rules(cloned, &mut new_rule_lists, &mut dedup));
            }
            if let Some(rules) = sources.hides_blocks.get(&state.name) {
                let compiled =
                    compile_rules_for_state(rules, state, registry, &block_states, block_tags);
                entry.hides = if compiled.is_empty() {
                    NO_RULES
                } else {
                    intern_rules(compiled, &mut new_rule_lists, &mut dedup)
                };
            } else if entry.hides != NO_RULES {
                let cloned = self.rule_lists[entry.hides as usize].clone();
                entry.hides = intern_rules(cloned, &mut new_rule_lists, &mut dedup);
            }
        }
        let unchanged = new_rule_lists.as_slice() == self.rule_lists.as_ref()
            && new_states
                .iter()
                .zip(self.states.iter())
                .all(|(a, b)| a.skip == b.skip && a.hides == b.hides);
        if unchanged {
            return None;
        }
        Some(Self {
            states: new_states.into(),
            faces: self.faces.clone(),
            rule_lists: new_rule_lists.into(),
            tag_sources: self.tag_sources.clone(),
            cache: HashMap::new(),
        })
    }

    fn state(&self, id: BlockStateId) -> io::Result<State> {
        self.states
            .get(id.0 as usize)
            .copied()
            .ok_or_else(|| invalid("unknown render state ID"))
    }

    #[inline]
    fn matches_rules(
        &self,
        index: u32,
        neighbor_id: u32,
        neighbor: State,
        direction: Direction,
    ) -> bool {
        let dir_bit = 1u8 << direction as u8;
        let Some(rules) = self.rule_lists.get(index as usize) else {
            return false;
        };
        rules.iter().any(|rule| {
            rule.faces & dir_bit != 0
                && (neighbor.flags & rule.fact_mask) == rule.fact_expected
                && rule.neighbors.contains(neighbor_id)
        })
    }

    fn skips(
        &self,
        a: State,
        neighbor_id: BlockStateId,
        b: State,
        direction: Direction,
        cutout_leaves: bool,
    ) -> bool {
        let vertical = matches!(direction, Direction::Down | Direction::Up);
        let bit = 1 << direction as u8;
        let opposite = 1 << direction.opposite() as u8;
        match a.skip {
            SkipRendering::Never => false,
            SkipRendering::SameBlock => a.block == b.block,
            SkipRendering::Leaves => !cutout_leaves && b.skip == SkipRendering::Leaves,
            SkipRendering::Liquid => a.fluid.same_family(b.fluid),
            SkipRendering::MangroveRoots => a.block == b.block && vertical,
            SkipRendering::Bars => {
                (a.block == b.block
                    || ((a.flags & FLAG_BARS_TAG) != 0
                        && (b.flags & FLAG_BARS_TAG) != 0
                        && b.connection_properties & opposite != 0))
                    && (vertical || (a.connections & bit != 0 && b.connections & opposite != 0))
            }
            SkipRendering::Rules(index) => self.matches_rules(index, neighbor_id.0, b, direction),
        }
    }

    /// The state's occlusion face toward `direction` (`getFaceOcclusionShape`) and
    /// its shared ID: 0 is the empty singleton, 1 the full-block singleton.
    pub fn occlusion_face(
        &self,
        state: BlockStateId,
        direction: Direction,
    ) -> Option<(u32, &VoxelShape)> {
        let id = self.states.get(state.0 as usize)?.faces[direction as usize];
        Some((id, &self.faces[id as usize]))
    }

    pub fn skip_rendering(
        &self,
        state: BlockStateId,
        neighbor: BlockStateId,
        direction: Direction,
        cutout_leaves: bool,
    ) -> io::Result<bool> {
        Ok(self.skips(
            self.state(state)?,
            neighbor,
            self.state(neighbor)?,
            direction,
            cutout_leaves,
        ))
    }

    /// Vanilla `Block.shouldRenderFace` with NeoForge's `hidesNeighborFace` /
    /// `supportsExternalFaceHiding` check between `skipRendering` and the empty-occluder test.
    pub fn should_render_face(
        &mut self,
        state: BlockStateId,
        neighbor: BlockStateId,
        direction: Direction,
        cutout_leaves: bool,
    ) -> io::Result<bool> {
        let a = self.state(state)?;
        let b = self.state(neighbor)?;
        let neighbor_face = b.faces[direction.opposite() as usize];
        if neighbor_face == 1 || self.skips(a, neighbor, b, direction, cutout_leaves) {
            return Ok(false);
        }
        if (a.flags & FLAG_SUPPORTS_EXTERNAL) != 0
            && b.hides != NO_RULES
            && self.matches_rules(b.hides, state.0, a, direction.opposite())
        {
            return Ok(false);
        }
        let own_face = a.faces[direction as usize];
        if neighbor_face == 0 || own_face == 0 {
            return Ok(true);
        }
        let key = (own_face, neighbor_face);
        if let Some(value) = self.cache.get(&key) {
            return Ok(*value);
        }
        let visible = self.faces[own_face as usize]
            .join_is_not_empty(&self.faces[neighbor_face as usize], ShapeOp::ONLY_FIRST)
            .map_err(|e| invalid(e.to_string()))?;
        // Cache policy affects speed only. Keep bounded memory while retaining
        // more shared face pairs than the native 256-entry per-thread cache.
        if self.cache.len() == 4096 {
            self.cache.clear();
        }
        self.cache.insert(key, visible);
        Ok(visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{FaceDirections, FaceFacts, FaceNeighbor, FaceRule};
    use sunrise_core::nbt::{Compound, Tag};

    fn empty_shape_nbt() -> Tag {
        Tag::compound([
            ("x", Tag::LongArray(vec![0])),
            ("y", Tag::LongArray(vec![0])),
            ("z", Tag::LongArray(vec![0])),
            ("occupied", Tag::ByteArray(vec![])),
            ("cube", Tag::Byte(0)),
            ("cube_axes", Tag::Byte(0)),
            ("discrete_min", Tag::IntArray(vec![0, 0, 0])),
            ("discrete_max", Tag::IntArray(vec![0, 0, 0])),
        ])
    }

    fn box_shape_nbt(x: [f64; 2], y: [f64; 2], z: [f64; 2]) -> Tag {
        let bits = |pair: [f64; 2]| pair.into_iter().map(|v| v.to_bits() as i64).collect();
        Tag::compound([
            ("x", Tag::LongArray(bits(x))),
            ("y", Tag::LongArray(bits(y))),
            ("z", Tag::LongArray(bits(z))),
            ("occupied", Tag::ByteArray(vec![1])),
            ("cube", Tag::Byte(0)),
            ("cube_axes", Tag::Byte(0)),
            ("discrete_min", Tag::IntArray(vec![0, 0, 0])),
            ("discrete_max", Tag::IntArray(vec![1, 1, 1])),
        ])
    }

    struct FixtureBuilder {
        states: Vec<Tag>,
        defaults: Compound,
        shapes: Vec<Tag>,
        state_shapes: Vec<i32>,
        defs: BTreeMap<String, RenderDefinition>,
    }

    impl FixtureBuilder {
        fn new() -> Self {
            Self {
                states: Vec::new(),
                defaults: Compound::new(),
                shapes: vec![
                    empty_shape_nbt(),
                    box_shape_nbt([0.0, 1.0], [0.0, 1.0], [0.0, 1.0]),
                    box_shape_nbt([0.0, 1.0], [0.0, 0.5], [0.0, 1.0]),
                    box_shape_nbt([0.0, 1.0], [0.5, 1.0], [0.0, 1.0]),
                ],
                state_shapes: Vec::new(),
                defs: BTreeMap::new(),
            }
        }

        const EMPTY: i32 = 0;
        const FULL: i32 = 1;
        const BOTTOM_SLAB: i32 = 2;
        const TOP_SLAB: i32 = 3;

        fn add_state(&mut self, name: &str, props: &[(&str, &str)], shape: i32) -> BlockStateId {
            let id = self.states.len() as u32;
            let mut p = Compound::new();
            for &(k, v) in props {
                p.insert(k.into(), Tag::String(v.into()));
            }
            self.states.push(Tag::compound([
                ("id", Tag::String(name.into())),
                ("properties", Tag::Compound(p)),
            ]));
            self.defaults
                .entry(name.into())
                .or_insert_with(|| Tag::Int(id as i32));
            self.state_shapes.push(shape);
            BlockStateId(id)
        }

        fn set_def(&mut self, name: &str, def: RenderDefinition) {
            self.defs.insert(name.into(), def);
        }

        fn build(
            self,
            tags: &BTreeMap<String, BTreeSet<String>>,
        ) -> (BlockRegistry, FaceVisibility) {
            let registry = BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(self.states.clone())),
                ("defaults", Tag::Compound(self.defaults)),
            ]))
            .unwrap();
            let collision_tag = Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(self.states)),
                ("shapes", Tag::List(self.shapes)),
                ("cached_shapes", Tag::IntArray(self.state_shapes)),
            ]);
            let occlusion = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
            let vis = FaceVisibility::new_with_tags(&occlusion, &self.defs, tags).unwrap();
            (registry, vis)
        }
    }

    /// A server's empty visibility knows no state: lookups fail instead of
    /// panicking.
    #[test]
    fn empty_visibility_knows_no_state() {
        let mut visibility = FaceVisibility::empty();
        assert!(visibility.is_empty());
        let stone = BlockStateId(1);
        assert!(visibility.occlusion_face(stone, Direction::Up).is_none());
        assert!(
            visibility
                .skip_rendering(stone, stone, Direction::Up, true)
                .is_err()
        );
        assert!(
            visibility
                .should_render_face(stone, stone, Direction::Up, true)
                .is_err()
        );
    }

    #[test]
    fn skip_rendering_self_rule_hides_same_block_only() {
        let mut f = FixtureBuilder::new();
        let air = f.add_state("minecraft:air", &[], FixtureBuilder::EMPTY);
        let glass = f.add_state("test:glass", &[], FixtureBuilder::FULL);
        let other = f.add_state("test:other", &[], FixtureBuilder::FULL);
        f.set_def(
            "minecraft:air",
            RenderDefinition::simple(SkipRendering::Never, false, false),
        );
        f.set_def(
            "test:glass",
            RenderDefinition {
                skip_rules: Some(vec![FaceRule {
                    neighbor: FaceNeighbor::SameBlock {
                        state: BTreeMap::new(),
                    },
                    ..FaceRule::default()
                }]),
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        f.set_def(
            "test:other",
            RenderDefinition::simple(SkipRendering::Never, false, false),
        );
        let (_, mut vis) = f.build(&BTreeMap::new());
        assert!(
            !vis.should_render_face(glass, glass, Direction::North, true)
                .unwrap()
        );
        assert!(
            vis.should_render_face(glass, other, Direction::North, true)
                .unwrap()
        );
        assert!(
            vis.should_render_face(glass, air, Direction::North, true)
                .unwrap()
        );
    }

    #[test]
    fn skip_rendering_same_axis_matches_equal_axis_only() {
        let mut f = FixtureBuilder::new();
        let px = f.add_state("test:pillar", &[("axis", "x")], FixtureBuilder::FULL);
        let py = f.add_state("test:pillar", &[("axis", "y")], FixtureBuilder::FULL);
        f.set_def(
            "test:pillar",
            RenderDefinition {
                skip_rules: Some(vec![FaceRule {
                    neighbor: FaceNeighbor::SameBlock {
                        state: BTreeMap::new(),
                    },
                    same: vec!["axis".into()],
                    ..FaceRule::default()
                }]),
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        let (_, mut vis) = f.build(&BTreeMap::new());
        assert!(
            !vis.should_render_face(px, px, Direction::East, true)
                .unwrap()
        );
        assert!(!vis.should_render_face(py, py, Direction::Up, true).unwrap());
        assert!(
            vis.should_render_face(px, py, Direction::East, true)
                .unwrap()
        );
        assert!(vis.should_render_face(py, px, Direction::Up, true).unwrap());
    }

    #[test]
    fn skip_rendering_faces_horizontal_leaves_vertical_visible() {
        let mut f = FixtureBuilder::new();
        let pane = f.add_state("test:pane", &[], FixtureBuilder::FULL);
        f.set_def(
            "test:pane",
            RenderDefinition {
                skip_rules: Some(vec![FaceRule {
                    neighbor: FaceNeighbor::SameBlock {
                        state: BTreeMap::new(),
                    },
                    faces: FaceDirections::Horizontal,
                    ..FaceRule::default()
                }]),
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        let (_, mut vis) = f.build(&BTreeMap::new());
        for dir in [
            Direction::North,
            Direction::South,
            Direction::West,
            Direction::East,
        ] {
            assert!(!vis.should_render_face(pane, pane, dir, true).unwrap());
        }
        for dir in [Direction::Down, Direction::Up] {
            assert!(vis.should_render_face(pane, pane, dir, true).unwrap());
        }
    }

    #[test]
    fn any_neighbor_rule_compiles_to_any_flag() {
        let mut f = FixtureBuilder::new();
        let air = f.add_state("minecraft:air", &[], FixtureBuilder::EMPTY);
        let hidden = f.add_state("test:always_skip", &[], FixtureBuilder::FULL);
        let other = f.add_state("test:other", &[], FixtureBuilder::FULL);
        f.set_def(
            "minecraft:air",
            RenderDefinition::simple(SkipRendering::Never, false, false),
        );
        f.set_def(
            "test:always_skip",
            RenderDefinition {
                skip_rules: Some(vec![FaceRule {
                    neighbor: FaceNeighbor::Any,
                    faces: FaceDirections::Vertical,
                    ..FaceRule::default()
                }]),
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        f.set_def(
            "test:other",
            RenderDefinition::simple(SkipRendering::Never, false, false),
        );
        let (_, mut vis) = f.build(&BTreeMap::new());
        // Verify ruling 4: an "any" rule compiles to NeighborSet::Any, not a state list.
        let SkipRendering::Rules(idx) = vis.states[hidden.0 as usize].skip else {
            panic!("expected compiled Rules");
        };
        assert_eq!(vis.rule_lists[idx as usize][0].neighbors, NeighborSet::Any);
        assert!(
            !vis.should_render_face(hidden, air, Direction::Up, true)
                .unwrap()
        );
        assert!(
            !vis.should_render_face(hidden, other, Direction::Down, true)
                .unwrap()
        );
        assert!(
            vis.should_render_face(hidden, other, Direction::North, true)
                .unwrap()
        );
    }

    #[test]
    fn hides_neighbor_face_with_state_filters_on_both_sides_and_external_flag() {
        let mut f = FixtureBuilder::new();
        let cover_bottom = f.add_state("test:cover", &[("half", "bottom")], FixtureBuilder::FULL);
        let cover_top = f.add_state("test:cover", &[("half", "top")], FixtureBuilder::FULL);
        let target_bottom = f.add_state("test:target", &[("half", "bottom")], FixtureBuilder::FULL);
        let target_top = f.add_state("test:target", &[("half", "top")], FixtureBuilder::FULL);
        let opt_out = f.add_state("test:opt_out", &[("half", "bottom")], FixtureBuilder::FULL);
        f.set_def(
            "test:cover",
            RenderDefinition {
                hides_rules: vec![
                    FaceRule {
                        neighbor: FaceNeighbor::Block {
                            id: "test:target".into(),
                            state: BTreeMap::from([("half".into(), "bottom".into())]),
                        },
                        state: BTreeMap::from([("half".into(), "bottom".into())]),
                        faces: FaceDirections::Only(vec![Direction::West]),
                        ..FaceRule::default()
                    },
                    FaceRule {
                        neighbor: FaceNeighbor::Block {
                            id: "test:opt_out".into(),
                            state: BTreeMap::from([("half".into(), "bottom".into())]),
                        },
                        state: BTreeMap::from([("half".into(), "bottom".into())]),
                        faces: FaceDirections::Only(vec![Direction::West]),
                        ..FaceRule::default()
                    },
                ],
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        f.set_def(
            "test:target",
            RenderDefinition::simple(SkipRendering::Never, false, false),
        );
        f.set_def(
            "test:opt_out",
            RenderDefinition {
                supports_external_face_hiding: false,
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        let (_, mut vis) = f.build(&BTreeMap::new());
        // Target's East face touches cover's West face (Direction::East.opposite() == West).
        assert!(
            !vis.should_render_face(target_bottom, cover_bottom, Direction::East, true)
                .unwrap()
        );
        // Wrong face direction: target's West face touches cover's East face -> not hidden.
        assert!(
            vis.should_render_face(target_bottom, cover_bottom, Direction::West, true)
                .unwrap()
        );
        // Neighbor state filter mismatch (target_top != bottom) -> not hidden.
        assert!(
            vis.should_render_face(target_top, cover_bottom, Direction::East, true)
                .unwrap()
        );
        // Own state filter mismatch on cover (cover_top != bottom) -> not hidden.
        assert!(
            vis.should_render_face(target_bottom, cover_top, Direction::East, true)
                .unwrap()
        );
        // supports_external_face_hiding: false blocks cover's hides_neighbor_face rule.
        assert!(
            vis.should_render_face(opt_out, cover_bottom, Direction::East, true)
                .unwrap()
        );
    }

    #[test]
    fn facts_rule_matches_vanilla_block_state_methods() {
        let mut f = FixtureBuilder::new();
        let air = f.add_state("minecraft:air", &[], FixtureBuilder::EMPTY);
        let stone = f.add_state("minecraft:stone", &[], FixtureBuilder::FULL);
        let water = f.add_state("minecraft:water", &[("level", "0")], FixtureBuilder::EMPTY);
        let slab = f.add_state("test:slab", &[], FixtureBuilder::BOTTOM_SLAB);
        let watcher = f.add_state("test:watcher", &[], FixtureBuilder::FULL);
        f.set_def(
            "minecraft:air",
            RenderDefinition::simple(SkipRendering::Never, false, false),
        );
        f.set_def(
            "minecraft:stone",
            RenderDefinition::simple(SkipRendering::Never, true, false),
        );
        f.set_def(
            "minecraft:water",
            RenderDefinition::simple(SkipRendering::Liquid, false, false),
        );
        f.set_def(
            "test:slab",
            RenderDefinition::simple(SkipRendering::Never, true, false),
        );
        f.set_def(
            "test:watcher",
            RenderDefinition {
                skip_rules: Some(vec![
                    // Skip North face against non-occluding non-air non-liquid neighbors.
                    FaceRule {
                        faces: FaceDirections::Only(vec![Direction::North]),
                        facts: FaceFacts {
                            is_air: Some(false),
                            can_occlude: Some(false),
                            is_solid_render: None,
                            liquid: Some(false),
                        },
                        ..FaceRule::default()
                    },
                    // Skip South face against liquid neighbors.
                    FaceRule {
                        faces: FaceDirections::Only(vec![Direction::South]),
                        facts: FaceFacts {
                            liquid: Some(true),
                            ..FaceFacts::default()
                        },
                        ..FaceRule::default()
                    },
                    // Skip East face against occluding non-solid-render neighbors (like a slab).
                    FaceRule {
                        faces: FaceDirections::Only(vec![Direction::East]),
                        facts: FaceFacts {
                            can_occlude: Some(true),
                            is_solid_render: Some(false),
                            ..FaceFacts::default()
                        },
                        ..FaceRule::default()
                    },
                ]),
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        let (_, mut vis) = f.build(&BTreeMap::new());
        // North: air has is_air=true -> visible; water has liquid=true -> visible; watcher has can_occlude=false, is_air=false, liquid=false -> hidden.
        assert!(
            vis.should_render_face(watcher, air, Direction::North, true)
                .unwrap()
        );
        assert!(
            vis.should_render_face(watcher, water, Direction::North, true)
                .unwrap()
        );
        assert!(
            !vis.should_render_face(watcher, watcher, Direction::North, true)
                .unwrap()
        );
        // South: water has liquid=true -> hidden; air has liquid=false -> visible.
        assert!(
            !vis.should_render_face(watcher, water, Direction::South, true)
                .unwrap()
        );
        assert!(
            vis.should_render_face(watcher, air, Direction::South, true)
                .unwrap()
        );
        // East: slab has can_occlude=true, is_solid_render=false -> hidden; stone has is_solid_render=true (already hidden by full occluder); air has can_occlude=false -> visible.
        assert!(
            !vis.should_render_face(watcher, slab, Direction::East, true)
                .unwrap()
        );
        assert!(
            vis.should_render_face(watcher, air, Direction::East, true)
                .unwrap()
        );
        let _ = stone;
    }

    #[test]
    fn should_render_face_order_skip_then_hides_then_shapes() {
        let mut f = FixtureBuilder::new();
        // Two non-full occluders (bottom slab vs top slab) whose shapes do NOT cover each other on North:
        // - Without rules, shape join says North is visible.
        // - With hides_neighbor_face on top_slab (even when empty occluder on North!), hides beats empty-occluder/shapes.
        // - With skip_rendering on bottom_slab, skip beats supports_external_face_hiding: false.
        let bottom = f.add_state("test:bottom", &[], FixtureBuilder::BOTTOM_SLAB);
        let top_hider = f.add_state("test:top_hider", &[], FixtureBuilder::TOP_SLAB);
        let empty_hider = f.add_state("test:empty_hider", &[], FixtureBuilder::EMPTY);
        f.set_def(
            "test:bottom",
            RenderDefinition {
                skip_rules: Some(vec![FaceRule {
                    neighbor: FaceNeighbor::Block {
                        id: "test:top_hider".into(),
                        state: BTreeMap::new(),
                    },
                    faces: FaceDirections::Only(vec![Direction::East]),
                    ..FaceRule::default()
                }]),
                supports_external_face_hiding: true,
                ..RenderDefinition::simple(SkipRendering::Never, true, false)
            },
        );
        f.set_def(
            "test:top_hider",
            RenderDefinition {
                hides_rules: vec![FaceRule {
                    neighbor: FaceNeighbor::Block {
                        id: "test:bottom".into(),
                        state: BTreeMap::new(),
                    },
                    faces: FaceDirections::Only(vec![Direction::South]),
                    ..FaceRule::default()
                }],
                ..RenderDefinition::simple(SkipRendering::Never, true, false)
            },
        );
        f.set_def(
            "test:empty_hider",
            RenderDefinition {
                hides_rules: vec![FaceRule {
                    neighbor: FaceNeighbor::Block {
                        id: "test:bottom".into(),
                        state: BTreeMap::new(),
                    },
                    faces: FaceDirections::All,
                    ..FaceRule::default()
                }],
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        let (_, mut vis) = f.build(&BTreeMap::new());
        // East: skip_rendering hides it.
        assert!(
            !vis.should_render_face(bottom, top_hider, Direction::East, true)
                .unwrap()
        );
        // North (touches top_hider's South): hides_neighbor_face hides it before shape join.
        assert!(
            !vis.should_render_face(bottom, top_hider, Direction::North, true)
                .unwrap()
        );
        // West: neither skip nor hides applies, and bottom slab vs top slab shapes don't cover -> visible.
        assert!(
            vis.should_render_face(bottom, top_hider, Direction::West, true)
                .unwrap()
        );
        // Against empty_hider (can_occlude=false -> neighbor_face == 0): hides_neighbor_face runs
        // BEFORE the empty-occluder test (NeoForge Block.java.patch), so it STILL hides!
        assert!(
            !vis.should_render_face(bottom, empty_hider, Direction::North, true)
                .unwrap()
        );
    }

    #[test]
    fn tag_rules_recompile_on_tag_sync() {
        let mut f = FixtureBuilder::new();
        let glass = f.add_state("test:glass", &[], FixtureBuilder::FULL);
        let buddy = f.add_state("test:buddy", &[], FixtureBuilder::FULL);
        f.set_def(
            "test:glass",
            RenderDefinition {
                skip_rules: Some(vec![FaceRule {
                    neighbor: FaceNeighbor::Tag {
                        id: "test:connects".into(),
                        state: BTreeMap::new(),
                    },
                    ..FaceRule::default()
                }]),
                ..RenderDefinition::simple(SkipRendering::Never, false, false)
            },
        );
        f.set_def(
            "test:buddy",
            RenderDefinition::simple(SkipRendering::Never, false, false),
        );
        let (registry, mut vis) = f.build(&BTreeMap::new());
        assert!(vis.has_tag_rules());
        // Before tags include test:buddy, face is visible.
        assert!(
            vis.should_render_face(glass, buddy, Direction::North, true)
                .unwrap()
        );
        // Same empty tags -> rebuild returns None.
        assert!(vis.rebuild_for_tags(&registry, &BTreeMap::new()).is_none());
        // Tag sync adds test:buddy -> rebuild returns Some and face is now hidden.
        let tags = BTreeMap::from([(
            "test:connects".into(),
            BTreeSet::from(["test:buddy".into()]),
        )]);
        let mut rebuilt = vis.rebuild_for_tags(&registry, &tags).unwrap();
        assert!(
            !rebuilt
                .should_render_face(glass, buddy, Direction::North, true)
                .unwrap()
        );
        // Rebuilding again with the same tags is a no-op.
        assert!(rebuilt.rebuild_for_tags(&registry, &tags).is_none());
    }
}
