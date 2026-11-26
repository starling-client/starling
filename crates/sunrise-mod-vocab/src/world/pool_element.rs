//! Template pools and their elements. A trait mod registers a pool element type
//! with [`Registrar::register_pool_element_type`](crate::Registrar::register_pool_element_type)
//! (NeoForge's registry `STRUCTURE_POOL_ELEMENT`, vanilla's
//! `StructurePoolElementType`). A template pool entry with
//! `"element_type": "modid:corridor"` is then decoded by the mod's code, and the
//! jigsaw assembly lays it out and places it the same way.
//!
//! A structure type reads the data pack's template pools through
//! [`GenerationContext::template_pool`](crate::GenerationContext::template_pool),
//! and their elements' jigsaws and boxes through
//! [`GenerationContext::elements`](crate::GenerationContext::elements). With
//! those reads it can lay a jigsaw out by its own rules and hand the pieces back
//! as [`PoolPiece`]s.
//!
//! As with structure types, there is no `Host` here. Elements are decoded at load
//! and laid out and placed on the worldgen worker threads. So an element keeps no
//! mutable state, and the same random gives the same answers.
use super::structure_template::StructureRotation;
use super::structure_type::{LiquidSettings, PieceContext, StructureError};
use crate::{BlockMetadata, Direction, Identifier, RandomSource};
use std::any::Any;
use std::sync::Arc;
use sunrise_core::{bounding_box::BoundingBox, nbt::Compound, nbt::Tag};

/// A mod's pool element type: the element codec of vanilla's
/// `StructurePoolElementType`. The game implements this once per registered
/// type and keeps it. It must be deterministic and stateless.
pub trait PoolElementType: Send + Sync + 'static {
    /// Decode one element. `config` is the pool entry's `element` value,
    /// converted from the data pack's JSON to NBT, with `element_type`
    /// included. A typed error leaves the pool out: every structure that uses
    /// that pool stops generating, and the error is logged. Vanilla refuses
    /// the whole data pack instead.
    fn decode(
        &self,
        context: &dyn ElementDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn PoolElement>, StructureError>;
}

/// What a [`PoolElementType::decode`] sees while a template pool loads.
pub trait ElementDecodeContext {
    /// Immutable, load-owned block metadata.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// Decode a nested element of any type, vanilla's or a mod's, through
    /// `StructurePoolElement.CODEC`. This is how an element that delegates to
    /// another one holds it.
    fn element(&self, config: &Tag) -> Result<PoolElementRef, StructureError>;
}

/// A decoded element of a mod's type: vanilla's `StructurePoolElement`.
/// The jigsaw assembly asks it for its jigsaws and its box. A pool piece
/// holding it places it once per chunk the piece reaches.
pub trait PoolElement: Any + Send + Sync + std::fmt::Debug {
    /// `getShuffledJigsawBlocks`: the element's jigsaws at `position`, turned
    /// by `rotation`, in the order the assembly tries them. Shuffle with
    /// `random`, as vanilla does, so draws stay in vanilla's order.
    fn shuffled_jigsaws(
        &self,
        context: &dyn ElementContext,
        position: [i32; 3],
        rotation: StructureRotation,
        random: &mut dyn RandomSource,
    ) -> Vec<JigsawBlock>;
    /// `getBoundingBox`: the element's box at `position`, turned by `rotation`.
    fn bounding_box(
        &self,
        context: &dyn ElementContext,
        position: [i32; 3],
        rotation: StructureRotation,
    ) -> BoundingBox;
    /// `place`: write the element's blocks into the chunk of `context`, at
    /// `placement`. Writes outside the chunk's box are dropped. Return false
    /// when the element placed nothing, as `StructurePoolElement.place`
    /// reports.
    fn place(
        &self,
        context: &mut dyn PieceContext,
        placement: &ElementPlacement,
    ) -> Result<bool, StructureError>;
    /// `getProjection`: the element's own projection, as decoded. Inside a
    /// list element the list's projection replaces it, and
    /// [`ElementPlacement::projection`] gives the one in effect.
    fn projection(&self) -> Projection;
    /// `getGroundLevelDelta`: how far above the box's bottom the element's
    /// ground is. Vanilla's elements answer 1.
    fn ground_level_delta(&self) -> i32 {
        1
    }
    /// The element's fields as its codec writes them. The game adds
    /// `element_type` beside them when it saves a piece holding the element.
    /// The saved tag decodes again through the type's `decode`.
    fn save(&self) -> Compound;
}

/// The element reads a structure type or an element makes during layout.
pub trait ElementContext {
    /// Immutable, load-owned block metadata, safe to borrow on this worker.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// `getShuffledJigsawBlocks` of any element, vanilla's or a mod's.
    fn jigsaws(
        &self,
        element: &PoolElementRef,
        position: [i32; 3],
        rotation: StructureRotation,
        random: &mut dyn RandomSource,
    ) -> Vec<JigsawBlock>;
    /// `getBoundingBox` of any element, vanilla's or a mod's.
    fn bounding_box(
        &self,
        element: &PoolElementRef,
        position: [i32; 3],
        rotation: StructureRotation,
    ) -> BoundingBox;
}

/// `StructureTemplatePool.Projection`: how an element meets the terrain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: vanilla's Projection has these two values.
pub enum Projection {
    /// `rigid`: placed as laid out.
    Rigid,
    /// `terrain_matching`: each column follows the surface.
    TerrainMatching,
}

/// An element of a template pool, of any type: a handle the game resolves.
/// Clone it freely, since clones share one element.
#[derive(Clone)]
#[non_exhaustive]
// no wire: a handle to an element the game holds; elements stay on the worldgen workers.
pub struct PoolElementRef {
    engine: Arc<dyn Any + Send + Sync>,
    mod_element: Option<Arc<dyn PoolElement>>,
    projection: Projection,
    ground_level_delta: i32,
    empty: bool,
    template_location: Option<Identifier>,
}
impl std::fmt::Debug for PoolElementRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolElementRef")
            .field("mod_element", &self.mod_element)
            .field("projection", &self.projection)
            .field("ground_level_delta", &self.ground_level_delta)
            .field("empty", &self.empty)
            .field("template_location", &self.template_location)
            .finish_non_exhaustive()
    }
}
impl PoolElementRef {
    /// `getProjection`.
    pub fn projection(&self) -> Projection {
        self.projection
    }
    /// `getGroundLevelDelta`.
    pub fn ground_level_delta(&self) -> i32 {
        self.ground_level_delta
    }
    /// Whether this is `minecraft:empty_pool_element`, which places nothing
    /// and ends a jigsaw's attempts.
    pub fn is_empty(&self) -> bool {
        self.empty
    }
    /// `SinglePoolElement.getTemplateLocation`: the template a single or
    /// legacy single element places, the name a layout rule such as a cap per
    /// piece goes by. `None` for other element types.
    pub fn template_location(&self) -> Option<&Identifier> {
        self.template_location.as_ref()
    }
    /// The element as a mod's element of type `T`, when it is one. Vanilla's
    /// elements, and other types' elements, give `None`.
    pub fn downcast_ref<T: PoolElement>(&self) -> Option<&T> {
        let element: &dyn Any = self.mod_element.as_deref()?;
        element.downcast_ref()
    }
}

/// Construct the handle the game answers element reads with.
// hidden: only the game's adapter builds an element handle; mods receive one from a pool or decode.
#[doc(hidden)]
pub fn pool_element_ref(
    engine: Arc<dyn Any + Send + Sync>,
    mod_element: Option<Arc<dyn PoolElement>>,
    projection: Projection,
    ground_level_delta: i32,
    empty: bool,
    template_location: Option<Identifier>,
) -> PoolElementRef {
    PoolElementRef {
        engine,
        mod_element,
        projection,
        ground_level_delta,
        empty,
        template_location,
    }
}

/// The game's own element behind a handle.
// hidden: only the game's adapter reads its element back from a handle.
#[doc(hidden)]
pub fn pool_element_payload(element: &PoolElementRef) -> &Arc<dyn Any + Send + Sync> {
    &element.engine
}

/// A data pack template pool, `minecraft:worldgen/template_pool`, as
/// `StructureTemplatePool` holds it once loaded.
#[derive(Clone, Debug)]
#[non_exhaustive]
// no wire: holds element handles, which stay on the worldgen workers.
pub struct TemplatePool {
    templates: Vec<PoolElementRef>,
    fallback: Identifier,
    max_size: i32,
}
impl TemplatePool {
    /// Every element, each repeated by its weight, in file order.
    pub fn templates(&self) -> &[PoolElementRef] {
        &self.templates
    }
    /// `getShuffledTemplates`: `templates`, shuffled by `Util.shuffle`.
    pub fn shuffled_templates(&self, random: &mut dyn RandomSource) -> Vec<&PoolElementRef> {
        let mut shuffled: Vec<&PoolElementRef> = self.templates.iter().collect();
        for i in (2..=shuffled.len()).rev() {
            let swap = random.next_int_bound(i as i32) as usize;
            shuffled.swap(i - 1, swap);
        }
        shuffled
    }
    /// `getRandomTemplate`: one element, drawn by weight. `None`, without a
    /// draw, for a pool with no elements; vanilla answers its empty element
    /// there.
    pub fn random_template(&self, random: &mut dyn RandomSource) -> Option<&PoolElementRef> {
        if self.templates.is_empty() {
            return None;
        }
        Some(&self.templates[random.next_int_bound(self.templates.len() as i32) as usize])
    }
    /// The pool a jigsaw tries after this one (`fallback`).
    pub fn fallback(&self) -> &Identifier {
        &self.fallback
    }
    /// `size`: how many entries `templates` holds.
    pub fn size(&self) -> usize {
        self.templates.len()
    }
    /// `getMaxSize`: the tallest box of the pool's non-empty elements, the
    /// way the village roads' expansion hack measures its children.
    pub fn max_size(&self) -> i32 {
        self.max_size
    }
}

/// Construct the pool the game answers `template_pool` with.
// hidden: only the game's adapter builds a template pool; mods receive one from the context.
#[doc(hidden)]
pub fn template_pool(
    templates: Vec<PoolElementRef>,
    fallback: Identifier,
    max_size: i32,
) -> TemplatePool {
    TemplatePool {
        templates,
        fallback,
        max_size,
    }
}

/// The free space a jigsaw's children are placed in: `JigsawPlacement.Placer`'s
/// `MutableObject<VoxelShape>`, a container box less the boxes already placed
/// in it. A layout starts with one over its reach, less the start piece's box.
/// A jigsaw whose attached piece would start inside its own piece's box places
/// that child in a space of its own over that box instead, made once per
/// piece.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: a layout's working state on the worldgen worker.
pub struct JigsawSpace {
    container: BoundingBox,
    placed: Vec<BoundingBox>,
}
impl JigsawSpace {
    /// The whole of `container`, inclusive (`Shapes.create(AABB.of(box))`).
    pub fn new(container: BoundingBox) -> Self {
        Self {
            container,
            placed: Vec::new(),
        }
    }
    /// Whether `bounding_box` lies wholly in the free space: vanilla's
    /// `!Shapes.joinIsNotEmpty(free, Shapes.create(AABB.of(box).deflate(0.25)),
    /// BooleanOp.ONLY_SECOND)`.
    pub fn fits(&self, bounding_box: &BoundingBox) -> bool {
        (0..3).all(|i| {
            bounding_box.min[i] >= self.container.min[i]
                && bounding_box.max[i] <= self.container.max[i]
        }) && !self
            .placed
            .iter()
            .any(|placed| placed.intersects(bounding_box))
    }
    /// Take `bounding_box` out of the free space
    /// (`Shapes.joinUnoptimized(free, Shapes.create(AABB.of(box)), ONLY_FIRST)`).
    pub fn remove(&mut self, bounding_box: BoundingBox) {
        self.placed.push(bounding_box);
    }
}

/// `JigsawBlockEntity.JointType`: whether an attached piece may turn about
/// the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: vanilla's JointType has these two values.
pub enum JigsawJoint {
    /// `rollable`: the attached piece's jigsaw may face any top.
    Rollable,
    /// `aligned`: both jigsaws' tops must match.
    Aligned,
}

/// A jigsaw block of an element, as vanilla's `StructureTemplate.JigsawBlockInfo`
/// holds it once placed and turned.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct JigsawBlock {
    /// Where the jigsaw sits, in world blocks.
    pub pos: [i32; 3],
    /// The side the jigsaw faces: where an attached piece goes.
    pub front: Direction,
    /// The jigsaw's top.
    pub top: Direction,
    /// Whether an attached piece may turn about the connection.
    pub joint: JigsawJoint,
    /// The name other jigsaws target (`name`). `None` matches any target;
    /// only a feature element's jigsaw has none.
    pub name: Option<Identifier>,
    /// The pool an attached piece is drawn from (`pool`).
    pub pool: Identifier,
    /// The name of the jigsaw an attached piece connects by (`target`).
    pub target: Identifier,
    /// `placement_priority`: children of higher priority are laid out first.
    pub placement_priority: i32,
    /// `selection_priority`: higher-priority jigsaws are tried first.
    pub selection_priority: i32,
}
impl JigsawBlock {
    /// A jigsaw with the defaults `JigsawBlockInfo.parse` reads for missing
    /// fields: name `minecraft:empty`, both priorities 0, and the joint
    /// `JigsawBlockEntity` gives a jigsaw without one (aligned when `front`
    /// is horizontal, rollable when it is up or down).
    pub fn new(
        pos: [i32; 3],
        front: Direction,
        top: Direction,
        pool: Identifier,
        target: Identifier,
    ) -> Self {
        let vertical = matches!(front, Direction::Up | Direction::Down);
        Self {
            pos,
            front,
            top,
            joint: if vertical {
                JigsawJoint::Rollable
            } else {
                JigsawJoint::Aligned
            },
            name: Some(Identifier::from_parts("minecraft", "empty").expect("a valid identifier")),
            pool,
            target,
            placement_priority: 0,
            selection_priority: 0,
        }
    }
    /// `JigsawBlock.canAttach`: whether `target`, a jigsaw of a candidate
    /// piece, connects to this one. The fronts must face each other. The tops
    /// must match unless this jigsaw is rollable. And `target`'s name must be
    /// this jigsaw's target, unless `target` has no name.
    pub fn can_attach(&self, target: &JigsawBlock) -> bool {
        self.front == target.front.opposite()
            && (self.joint == JigsawJoint::Rollable || self.top == target.top)
            && target.name.as_ref().is_none_or(|name| *name == self.target)
    }
}

/// `JigsawJunction`: where a child piece meets its parent, which the terrain
/// around a rigid parent blends towards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: vanilla's JigsawJunction has these five fields.
pub struct JigsawJunction {
    /// The junction's X.
    pub source_x: i32,
    /// The ground height on the parent's side.
    pub source_ground_y: i32,
    /// The junction's Z.
    pub source_z: i32,
    /// The child's ground height minus the parent's.
    pub delta_y: i32,
    /// The child's projection.
    pub dest_projection: Projection,
}
impl JigsawJunction {
    /// A junction with these fields.
    pub fn new(
        source_x: i32,
        source_ground_y: i32,
        source_z: i32,
        delta_y: i32,
        dest_projection: Projection,
    ) -> Self {
        Self {
            source_x,
            source_ground_y,
            source_z,
            delta_y,
            dest_projection,
        }
    }
}

/// A piece that places one pool element: vanilla's
/// `PoolElementStructurePiece`. The game keeps, saves and places it as it does
/// a `minecraft:jigsaw` structure's pieces.
#[derive(Clone, Debug)]
#[non_exhaustive]
// no wire: holds an element handle, which stays on the worldgen workers.
pub struct PoolPiece {
    /// The element the piece places.
    pub element: PoolElementRef,
    /// Where the element is placed, in world blocks.
    pub position: [i32; 3],
    /// The element's ground level above the box's bottom.
    pub ground_level_delta: i32,
    /// How the element is turned.
    pub rotation: StructureRotation,
    /// The piece's box, in world blocks.
    pub bounding_box: BoundingBox,
    /// Whether the element's blocks take on the water they are placed in.
    pub liquid: LiquidSettings,
    /// Where its children meet it.
    pub junctions: Vec<JigsawJunction>,
}
impl PoolPiece {
    /// A piece with no junctions.
    pub fn new(
        element: PoolElementRef,
        position: [i32; 3],
        ground_level_delta: i32,
        rotation: StructureRotation,
        bounding_box: BoundingBox,
        liquid: LiquidSettings,
    ) -> Self {
        Self {
            element,
            position,
            ground_level_delta,
            rotation,
            bounding_box,
            liquid,
            junctions: Vec::new(),
        }
    }
}

/// Where and how an element is placed: the arguments of
/// `StructurePoolElement.place`, besides the level, the random and the
/// chunk's box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct ElementPlacement {
    /// Where the element is placed, in world blocks.
    pub position: [i32; 3],
    /// The structure's reference point that processors see: the centre of the
    /// start piece's box at its bottom.
    pub reference: [i32; 3],
    /// How the element is turned.
    pub rotation: StructureRotation,
    /// Whether the element's blocks take on the water they are placed in.
    pub liquid: LiquidSettings,
    /// Leave jigsaw blocks in place rather than replace them with their
    /// final state (`/place jigsaw`'s `keepJigsaws`).
    pub keep_jigsaws: bool,
    /// The projection the element is placed with: its own
    /// [`PoolElement::projection`], or the projection of the list element
    /// holding it, which replaces its children's
    /// (`ListPoolElement.setProjectionOnEachElement`). A terrain-matching
    /// template follows the surface, as vanilla's elements add the gravity
    /// processor. An element placed through
    /// [`PieceContext::place_element`](crate::PieceContext::place_element)
    /// keeps its own projection, so this field is not read there.
    pub projection: Projection,
}
impl ElementPlacement {
    /// A placement at `position`, turned by `rotation`. The reference point is
    /// `position`, waterlogging is applied, jigsaws are replaced and the
    /// projection is rigid. Set the fields to change any of that.
    pub fn new(position: [i32; 3], rotation: StructureRotation) -> Self {
        Self {
            position,
            reference: position,
            rotation,
            liquid: LiquidSettings::ApplyWaterlogging,
            keep_jigsaws: false,
            projection: Projection::Rigid,
        }
    }
}

/// A registered API behaviour; the game builds its engine adapter at startup.
pub(crate) struct PoolElementTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) mod_id: &'static str,
    pub(crate) ty: Arc<dyn PoolElementType>,
}

/// Registered pool element behaviours, for the hidden game integration
/// boundary.
// hidden: the game's startup adapter reads the registered behaviours; mods declare them through the registrar.
#[doc(hidden)]
pub fn pool_element_type_declarations() -> Vec<(String, &'static str, Arc<dyn PoolElementType>)> {
    crate::parts::installed_pool_element_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.mod_id, info.ty.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    #[test]
    fn can_attach_follows_vanilla() {
        let source = JigsawBlock::new(
            [0; 3],
            Direction::North,
            Direction::Up,
            id("a:pool"),
            id("a:door"),
        );
        let mut target = JigsawBlock::new(
            [0; 3],
            Direction::South,
            Direction::Up,
            id("a:pool"),
            id("a:any"),
        );
        target.name = Some(id("a:door"));
        assert!(source.can_attach(&target));
        // The fronts must face each other.
        target.front = Direction::East;
        assert!(!source.can_attach(&target));
        target.front = Direction::South;
        // An aligned source needs equal tops; a rollable one does not.
        target.top = Direction::West;
        assert!(!source.can_attach(&target));
        let rollable = JigsawBlock {
            joint: JigsawJoint::Rollable,
            ..source.clone()
        };
        assert!(rollable.can_attach(&target));
        target.top = Direction::Up;
        // The target's name must be the source's target, unless it has none.
        target.name = Some(id("a:window"));
        assert!(!source.can_attach(&target));
        target.name = None;
        assert!(source.can_attach(&target));
    }
}
