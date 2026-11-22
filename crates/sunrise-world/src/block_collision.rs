//! Cached/dynamic no-context collision queries and explicit contextual block rules.
//! Context-sensitive families override the no-context cache explicitly.
use crate::{
    block_collision_scan::CollisionBlock,
    collision::{CachedCollisionRegistry, ShapeChannel},
    collision_context::CollisionContext,
    dynamic_support::{DynamicSupport, DynamicSupportError, DynamicSupportWorld},
    fluid::{FluidKind, FluidState},
    liquid_collision::{LiquidCollision, LiquidCollisionWorld},
    part_collision,
    piston_shape::PistonBlockBindings,
    powder_snow_collision::{PowderSnowWorld, powder_snow_collision_shape},
    registry::{BlockRegistry, BlockStateId},
};
use std::{
    collections::HashMap,
    io,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use sunrise_core::geometry::{Aabb, Axis, Direction, Vec3, VoxelShape};

#[derive(Clone, Copy, Debug)]
pub struct ShulkerCollisionState {
    pub previous_progress: f32,
    pub progress: f32,
}
impl ShulkerCollisionState {
    pub fn bounding_box(self, direction: Direction) -> Aabb {
        // Native getProgress(1) retains this interpolation, including rounding.
        let progress = 0.5f32 * (self.previous_progress + (self.progress - self.previous_progress));
        let maximum = if progress.is_nan() {
            progress
        } else {
            (-1.0f32).max(progress)
        } as f64;
        let minimum = if progress.is_nan() {
            progress
        } else {
            (-1.0f32).min(progress)
        } as f64;
        let [x, y, z] = direction.normal();
        Aabb::new(Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5))
            .expand_towards(Vec3::new(
                f64::from(x) * maximum,
                f64::from(y) * maximum,
                f64::from(z) * maximum,
            ))
            .contract(Vec3::new(
                f64::from(-x) * (1.0 + minimum),
                f64::from(-y) * (1.0 + minimum),
                f64::from(-z) * (1.0 + minimum),
            ))
            .move_by(Vec3::new(0.5, 0.0, 0.5))
    }
}

pub trait DynamicCollisionWorld: DynamicSupportWorld {
    /// One block-entity lookup. None denotes absence or another entity type.
    fn shulker_collision_state(
        &mut self,
        position: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, Self::Error>;
}

#[derive(Debug)]
pub enum CollisionQueryError<E> {
    Data(io::Error),
    Dynamic(DynamicSupportError<E>),
    Context(E),
    Unsupported(BlockStateId),
}
impl<E: std::fmt::Display> std::fmt::Display for CollisionQueryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(error) => error.fmt(f),
            Self::Dynamic(error) => error.fmt(f),
            Self::Context(error) => error.fmt(f),
            Self::Unsupported(state) => write!(f, "unsupported block collision state {}", state.0),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for CollisionQueryError<E> {}

pub struct BlockCollisionRegistry<'r> {
    cached: Arc<CachedCollisionRegistry<'r>>,
    dynamic: HashMap<BlockStateId, DynamicSupport>,
    contextual: HashMap<BlockStateId, ContextualRule>,
    /// The scan metadata the registry was built with.
    base_flags: Vec<u8>,
    /// What a scan reads: `base_flags` with the states whose parts answer collision queries
    /// marked (bit 5), large and never empty. Relaxed: set between ticks.
    scan_flags: Vec<AtomicU8>,
    /// Piston and piston-head states for moving-piston collision; None in
    /// registries without pistons.
    pistons: Option<PistonBlockBindings<'r>>,
}
enum ContextualRule {
    Scaffolding(ScaffoldingCollision),
    Liquid(LiquidCollision),
    PowderSnow,
}
impl<'r> BlockCollisionRegistry<'r> {
    pub fn new(cached: Arc<CachedCollisionRegistry<'r>>) -> io::Result<Self> {
        let mut dynamic = HashMap::new();
        let mut contextual = HashMap::new();
        let mut scan_flags = Vec::with_capacity(cached.registry().len());
        for index in 0..cached.registry().len() {
            let id = BlockStateId(index as u32);
            let state = cached.registry().state(id).unwrap();
            let published = cached.publishes_shapes(id);
            // A block entity's shape can leave the cell, so a state that may publish is
            // always scanned for in the cells around its own, as vanilla's dynamic shapes are.
            let mut large = published
                || cached.offset_collision(id)
                || cached.cached_shape(id)?.is_none_or(|shape| {
                    Axis::ALL
                        .into_iter()
                        .any(|axis| shape.min(axis) < 0.0 || shape.max(axis) > 1.0)
                });
            if cached.declares_scaffolding_collision(id) || state.name == VANILLA_SCAFFOLDING {
                contextual.insert(
                    id,
                    ContextualRule::Scaffolding(ScaffoldingCollision::from_state(state)?),
                );
            } else if let Some(rule) = LiquidCollision::from_state(state)? {
                contextual.insert(id, ContextualRule::Liquid(rule));
            } else if state.name == "minecraft:powder_snow" {
                contextual.insert(id, ContextualRule::PowderSnow);
            }
            let mut declared_collision = false;
            let mut declared_nonempty = false;
            for shape in cached.collision_variants(id) {
                declared_collision = true;
                declared_nonempty |= !shape.is_empty();
                large |= Axis::ALL
                    .into_iter()
                    .any(|axis| shape.min(axis) < 0.0 || shape.max(axis) > 1.0);
            }
            // No contextual rule and an empty cached shape: empty everywhere.
            let empty = !published
                && !declared_nonempty
                && !contextual.contains_key(&id)
                && cached.cached_shape(id)?.is_some_and(VoxelShape::is_empty);
            scan_flags.push(
                u8::from(large)
                    | (u8::from(state.name == "minecraft:moving_piston") << 1)
                    | (u8::from(empty) << 2)
                    | (u8::from(contextual.contains_key(&id) || declared_collision) << 3)
                    | (u8::from(published) << 4),
            );
            if cached.cached_shape(id)?.is_none()
                && let Some(rule) =
                    DynamicSupport::from_state(cached.registry().state(id).unwrap())?
            {
                dynamic.insert(id, rule);
            }
        }
        let pistons = PistonBlockBindings::new(cached.registry()).ok();
        Ok(Self {
            cached,
            dynamic,
            contextual,
            scan_flags: scan_flags.iter().copied().map(AtomicU8::new).collect(),
            base_flags: scan_flags,
            pistons,
        })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.cached.registry()
    }
    fn flags(&self, state: BlockStateId) -> Option<u8> {
        self.scan_flags
            .get(state.0 as usize)
            .map(|flags| flags.load(Ordering::Relaxed))
    }
    /// Marks the states whose parts answer collision queries (`BlockPart::collision_shape`), and
    /// only them: such a state is scanned for in the cells around its own, as a published one
    /// is, and never counts as empty, since its shape depends on who asks. Called between
    /// ticks, once the game's parts are resolved; an earlier marking is replaced.
    pub fn set_part_states(&self, states: impl IntoIterator<Item = BlockStateId>) {
        for (flags, base) in self.scan_flags.iter().zip(&self.base_flags) {
            flags.store(*base, Ordering::Relaxed);
        }
        for state in states {
            if let (Some(flags), Some(base)) = (
                self.scan_flags.get(state.0 as usize),
                self.base_flags.get(state.0 as usize),
            ) {
                flags.store((base | 1 | 32) & !4, Ordering::Relaxed);
            }
        }
    }
    /// The piston states behind `PistonMovingBlockEntity.getCollisionShape`.
    pub fn pistons(&self) -> Option<&PistonBlockBindings<'r>> {
        self.pistons.as_ref()
    }
    pub fn cached_shapes(&self) -> &CachedCollisionRegistry<'r> {
        &self.cached
    }
    /// Immutable cursor-filter metadata, prepared once for this registry.
    pub fn collision_block(&self, state: BlockStateId) -> io::Result<CollisionBlock> {
        let flags = self
            .flags(state)
            .ok_or_else(|| crate::compression::invalid("collision scan state outside registry"))?;
        Ok(CollisionBlock {
            state,
            large_collision_shape: flags & 1 != 0,
            moving_piston: flags & 2 != 0,
            empty_shape: flags & 4 != 0,
        })
    }
    /// Whether the block entity of `state`'s block publishes the block's shapes
    /// (`shapes_from_block_entity`). States outside the registry do not.
    pub fn publishes_shapes(&self, state: BlockStateId) -> bool {
        self.flags(state).is_some_and(|flags| flags & 16 != 0)
    }
    /// Whether the default collision can change with position through publication or an offset.
    pub fn has_positioned_collision(&self, state: BlockStateId) -> bool {
        self.publishes_shapes(state)
            || self
                .cached
                .offset_limits(state, ShapeChannel::Collision)
                .is_some()
    }
    /// The state's cached shape, unless it depends on the position or its parts may answer for
    /// the entity: callers that read the cache go through [`Self::collision_shape`] (or
    /// [`Self::collision_shape_with_context`], with a context) for those states.
    pub fn static_shape(&self, state: BlockStateId) -> io::Result<Option<&VoxelShape>> {
        if self.has_positioned_collision(state) || self.has_part(state) {
            return Ok(None);
        }
        self.cached.cached_shape(state)
    }
    /// Whether `state` can have a collision shape: false for states whose shape is always
    /// empty, whatever the context. States outside the registry count.
    pub fn may_collide(&self, state: BlockStateId) -> bool {
        self.flags(state).is_none_or(|flags| flags & 4 == 0)
    }
    /// Whether `state` has a contextual collision rule (scaffolding, a liquid, powder snow, or
    /// parts that answer collision queries): its shape with an entity's context may differ from
    /// the cached one. States outside the
    /// registry count.
    pub fn has_context_rule(&self, state: BlockStateId) -> bool {
        self.flags(state).is_none_or(|flags| flags & (8 | 32) != 0)
    }
    fn has_part(&self, state: BlockStateId) -> bool {
        self.flags(state).is_some_and(|flags| flags & 32 != 0)
    }
    /// Whether `state` takes scaffolding's context collision (vanilla's scaffolding, or a block
    /// declaring `context_collision` scaffolding): also what lets a sneaking entity inside it
    /// slide down (`isScaffolding`).
    pub fn is_scaffolding(&self, state: BlockStateId) -> bool {
        self.has_context_rule(state)
            && matches!(
                self.contextual.get(&state),
                Some(ContextualRule::Scaffolding(_))
            )
    }
    /// Whether a scan keeps `state` in the cells around its box: a large
    /// collision shape or a moving piston. States outside the registry count.
    pub fn reaches_out(&self, state: BlockStateId) -> bool {
        self.flags(state).is_none_or(|flags| flags & 3 != 0)
    }
    pub fn collision_shape<W: DynamicCollisionWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<VoxelShape, CollisionQueryError<W::Error>> {
        if self.publishes_shapes(state)
            && let Some(shape) = world
                .published_collision_shape(position, state)
                .map_err(|e| CollisionQueryError::Dynamic(DynamicSupportError::World(e)))?
        {
            return Ok(shape);
        }
        if let Some(shape) = self
            .cached
            .cached_shape(state)
            .map_err(CollisionQueryError::Data)?
        {
            return Ok(self.cached.positioned_shape(
                state,
                ShapeChannel::Collision,
                position,
                shape,
            ));
        }
        self.dynamic
            .get(&state)
            .ok_or(CollisionQueryError::Unsupported(state))?
            .collision_shape(world, position)
            .map_err(CollisionQueryError::Dynamic)
    }

    /// Ordinary entity/position/placement contexts. The remaining pinned block
    /// families share their no-context geometry; the native registry audit checks
    /// cached states with rejecting context/world providers before using that path.
    /// Experimental minecart position exclusions are a separate context layer.
    pub fn collision_shape_with_context<W, E>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: sunrise_core::position::BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, CollisionQueryError<E>>
    where
        W: DynamicCollisionWorld<Error = E>
            + LiquidCollisionWorld<Error = E>
            + PowderSnowWorld<Error = E>,
    {
        let flags = self.flags(state);
        // A part that answers for the entity replaces whatever the block would otherwise have,
        // a published shape included; `None` keeps it.
        if flags.is_some_and(|flags| flags & 32 != 0)
            && let Some(shape) = part_collision::ask(state, position, context, &mut || {
                let Some(entity) = context.living_entity() else {
                    return &[];
                };
                let mut stands = |kind| {
                    world
                        .entity_can_stand(entity, FluidState::new(kind, 8, false))
                        .unwrap_or(false)
                };
                match (stands(FluidKind::Water), stands(FluidKind::Lava)) {
                    (false, false) => &[],
                    (true, false) => &["minecraft:water"],
                    (false, true) => &["minecraft:lava"],
                    (true, true) => &["minecraft:water", "minecraft:lava"],
                }
            })
        {
            return Ok(shape);
        }
        // A published shape stands over every declared one, whatever the entity.
        if self.publishes_shapes(state)
            && let Some(shape) = world
                .published_collision_shape([position.x, position.y, position.z], state)
                .map_err(|e| CollisionQueryError::Dynamic(DynamicSupportError::World(e)))?
        {
            return Ok(shape);
        }
        // Most states have no contextual rule: their flag spares the lookup.
        let contextual = flags.is_none_or(|flags| flags & 8 != 0);
        if contextual
            && let Some(shape) = self
                .cached
                .context_collision_shape(state, context.entity_kind())
        {
            return Ok(self.cached.positioned_shape(
                state,
                ShapeChannel::ContextCollision,
                [position.x, position.y, position.z],
                shape,
            ));
        }
        match contextual.then(|| self.contextual.get(&state)).flatten() {
            Some(ContextualRule::Scaffolding(rule)) => Ok(rule.shape(context, position)),
            Some(ContextualRule::Liquid(rule)) => rule
                .shape(world, position, context)
                .map_err(CollisionQueryError::Context),
            Some(ContextualRule::PowderSnow) => {
                powder_snow_collision_shape(world, position, context)
                    .map_err(CollisionQueryError::Context)
            }
            None => self.collision_shape(world, state, [position.x, position.y, position.z]),
        }
    }
}

/// Vanilla's `ScaffoldingBlock`: the vanilla block whose states take [`ScaffoldingCollision`],
/// as a declared block's do when it sets `context_collision` to scaffolding. The prepared
/// collision table lists only declared blocks' states (`scaffolding_collision`).
const VANILLA_SCAFFOLDING: &str = "minecraft:scaffolding";

/// Entity-context scaffold selection (`ScaffoldingBlock.getCollisionShape`), independent of
/// world queries, for vanilla's scaffolding and every block declaring `context_collision`
/// scaffolding. Construction validates the descriptor so missing properties cannot silently
/// change behavior.
#[derive(Clone, Copy, Debug)]
pub struct ScaffoldingCollision {
    supported: bool,
    bottom: bool,
}
impl ScaffoldingCollision {
    /// The rule of a state with scaffolding's `distance` (0 to 7) and `bottom` values; an error
    /// names the one that is missing or out of range.
    pub fn from_values(distance: Option<&str>, bottom: Option<&str>) -> io::Result<Self> {
        let distance = distance
            .and_then(crate::registry::java_integer)
            .filter(|v| (0..=7).contains(v))
            .ok_or_else(|| crate::compression::invalid("invalid scaffold distance"))?;
        let bottom = match bottom {
            Some("true") => true,
            Some("false") => false,
            _ => return Err(crate::compression::invalid("invalid scaffold bottom")),
        };
        Ok(Self {
            supported: distance == 0,
            bottom,
        })
    }
    pub fn from_state(state: &crate::chunk::BlockState) -> io::Result<Self> {
        let value = |name| state.properties.get(name).map(String::as_str);
        Self::from_values(value("distance"), value("bottom"))
    }

    pub fn shape(
        &self,
        context: &crate::collision_context::CollisionContext<'_>,
        position: sunrise_core::position::BlockPos,
    ) -> VoxelShape {
        use std::sync::OnceLock;
        if context.is_placement() {
            return VoxelShape::empty();
        }
        if context.is_above(&VoxelShape::block(), position, true) && !context.is_descending() {
            return crate::dynamic_support::scaffolding_stable_shape();
        }
        if !self.supported && self.bottom {
            static BELOW: OnceLock<VoxelShape> = OnceLock::new();
            let below = BELOW.get_or_init(|| {
                VoxelShape::block()
                    .moved(Vec3::new(0.0, -1.0, 0.0))
                    .optimized()
                    .expect("bounded scaffold below shape")
            });
            if context.is_above(below, position, true) {
                static BOTTOM: OnceLock<VoxelShape> = OnceLock::new();
                return BOTTOM
                    .get_or_init(|| {
                        VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.125, 1.0)))
                            .expect("bounded scaffold bottom")
                    })
                    .clone();
            }
        }
        VoxelShape::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DATA_VERSION,
        chunk::BlockState,
        collision_context::{CollisionEntity, EntityContextKind},
        entity_obstruction::EntityIdentity,
        fluid::FluidState,
        powder_snow_collision::PowderSnowEntity,
    };
    use std::convert::Infallible;
    use sunrise_core::{nbt::Tag, position::BlockPos};

    /// A world no scaffolding query reads.
    struct NoWorld;
    impl DynamicSupportWorld for NoWorld {
        type Error = Infallible;
        fn shulker_is_closed(&mut self, _: [i32; 3]) -> Result<Option<bool>, Infallible> {
            unreachable!()
        }
        fn moving_piston_shape(&mut self, _: [i32; 3]) -> Result<Option<VoxelShape>, Infallible> {
            unreachable!()
        }
    }
    impl DynamicCollisionWorld for NoWorld {
        fn shulker_collision_state(
            &mut self,
            _: [i32; 3],
        ) -> Result<Option<ShulkerCollisionState>, Infallible> {
            unreachable!()
        }
    }
    impl LiquidCollisionWorld for NoWorld {
        type Error = Infallible;
        fn entity_liquid_shape(&mut self, _: EntityIdentity) -> Result<VoxelShape, Infallible> {
            unreachable!()
        }
        fn fluid(&mut self, _: BlockPos) -> Result<FluidState, Infallible> {
            unreachable!()
        }
        fn entity_can_stand(
            &mut self,
            _: EntityIdentity,
            _: FluidState,
        ) -> Result<bool, Infallible> {
            unreachable!()
        }
    }
    impl PowderSnowWorld for NoWorld {
        type Error = Infallible;
        fn powder_snow_entity(
            &mut self,
            _: EntityIdentity,
        ) -> Result<PowderSnowEntity, Infallible> {
            unreachable!()
        }
        fn powder_snow_walkable_tag(&mut self, _: EntityIdentity) -> Result<bool, Infallible> {
            unreachable!()
        }
        fn feet_are_leather_boots(&mut self, _: EntityIdentity) -> Result<bool, Infallible> {
            unreachable!()
        }
    }

    fn state(name: &str, distance: &str, bottom: &str) -> BlockState {
        BlockState {
            name: name.into(),
            properties: [("bottom", bottom), ("distance", distance)]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        }
    }

    /// A declared block listed under `scaffolding_collision` takes vanilla scaffolding's
    /// collision: solid on top, open to a descending entity, and the bottom row's floor.
    #[test]
    fn a_declared_scaffolding_collision_matches_vanilla_scaffolding() {
        let states = [
            state("minecraft:scaffolding", "0", "false"),
            state("minecraft:scaffolding", "7", "true"),
            state("test:metal_scaffolding", "0", "false"),
            state("test:metal_scaffolding", "7", "true"),
        ];
        let nbt = || Tag::List(states.iter().map(BlockState::to_nbt).collect());
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", nbt()),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:scaffolding", Tag::Int(0)),
                    ("test:metal_scaffolding", Tag::Int(2)),
                ]),
            ),
        ]))
        .unwrap();
        let cube = Tag::compound([
            ("canonical", Tag::Byte(0)),
            ("cube", Tag::Byte(1)),
            ("cube_axes", Tag::Byte(7)),
            ("x", Tag::LongArray(vec![0, 1.0f64.to_bits() as i64])),
            ("y", Tag::LongArray(vec![0, 1.0f64.to_bits() as i64])),
            ("z", Tag::LongArray(vec![0, 1.0f64.to_bits() as i64])),
            ("occupied", Tag::ByteArray(vec![1])),
            ("discrete_min", Tag::IntArray(vec![0; 3])),
            ("discrete_max", Tag::IntArray(vec![1; 3])),
        ]);
        let cached = CachedCollisionRegistry::from_nbt(
            &Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("states", nbt()),
                ("shapes", Tag::List(vec![cube])),
                ("cached_shapes", Tag::IntArray(vec![0; 4])),
                ("scaffolding_collision", Tag::IntArray(vec![2, 3])),
            ]),
            &registry,
        )
        .unwrap();
        let binding = BlockCollisionRegistry::new(Arc::new(cached)).unwrap();
        let at = BlockPos::new(0, 64, 0);
        let shape = |state: u32, y: f64, descending: bool| {
            let context = CollisionContext::for_entity(
                CollisionEntity {
                    identity: EntityIdentity(1),
                    kind: EntityContextKind::Player,
                    y,
                    descending,
                    main_hand: None,
                },
                false,
            );
            binding
                .collision_shape_with_context(&mut NoWorld, BlockStateId(state), at, &context)
                .unwrap()
                .boxes()
                .to_vec()
        };
        // (supported state, entity y, descending): standing on top, descending through it,
        // and in the bottom row of an unsupported bottom scaffold.
        for (supported, y, descending) in [
            (true, 65.0, false),
            (true, 65.0, true),
            (false, 64.5, false),
        ] {
            let (vanilla, declared) = if supported { (0, 2) } else { (1, 3) };
            assert_eq!(
                shape(declared, y, descending),
                shape(vanilla, y, descending)
            );
        }
        assert!((0..4).all(|state| binding.is_scaffolding(BlockStateId(state))));
        assert!(!shape(2, 65.0, false).is_empty());
        assert!(shape(2, 65.0, true).is_empty());
        assert_eq!(
            shape(3, 64.5, false),
            vec![Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.125, 1.0))]
        );
    }
}
