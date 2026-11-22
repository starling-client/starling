//! The `friction` Part method (`IBlockExtension.getFriction`). The client library asks through
//! `native_client_hooks`; only a state whose block has a subscribing part is converted.
use super::api_values::{TryIntoApi, TryIntoEngine, block_state};
use super::{Dim, Game, boats::Boat};
use mod_api::{
    __private::game::Fields, BlockEntityRef, BlockKey, BlockState, Compound, FeatureFlags,
    FluidState, LevelHost, LevelReadError, LevelRef, LightLayer,
};
use std::cell::RefCell;
use sunrise_client::{
    items::Items,
    motion::{FrictionLevel, read_friction},
    world::World,
};
use sunrise_core::position::BlockPos;
use sunrise_world::{
    fluid::FluidState as EngineFluidState,
    native_load_context::LoopSide,
    registry::{BlockRegistry, BlockStateId},
};

/// This side's level data for one tick, built once from the game's tables and lent to the motion,
/// mob and entity ticks that read friction. Fields borrow the game's tables one by one, so the
/// tick's other borrows of the game stand beside it (`game_side!`).
pub(crate) struct GameSide<'a> {
    pub(in crate::game) dim: super::Dim,
    /// The level handle of the open world, absent before a world has an epoch.
    pub(in crate::game) level: Option<LevelRef>,
    pub(in crate::game) facts: &'a sunrise_client::blocks::BlockFacts,
    pub(in crate::game) biomes: &'a sunrise_client::biome::Biomes,
    pub(in crate::game) behaviours: &'a sunrise_world::level::Behaviours,
    pub(in crate::game) features: &'a std::sync::Arc<[String]>,
    pub(in crate::game) items: &'a Items,
    pub(in crate::game) damage_types: &'a sunrise_world::damage_source::Catalog,
    pub(in crate::game) player_equipment: sunrise_client::health::PlayerEquipment<'a>,
    pub(in crate::game) tag_items: Option<&'a Items>,
}

impl sunrise_client::motion::SideLevel for GameSide<'_> {
    fn is_raining_at(&self, world: &World, pos: BlockPos) -> bool {
        world.is_loaded_at(pos.x, pos.y, pos.z)
            && super::native_level_world::rain_at(
                world,
                self.facts,
                self.biomes,
                self.behaviours,
                self.dim,
                pos,
            )
    }

    fn enabled_features(&self) -> &std::sync::Arc<[String]> {
        self.features
    }

    fn tag_items(&self) -> Option<&Items> {
        self.tag_items
    }

    fn player_equipment(&self) -> sunrise_client::health::PlayerEquipment<'_> {
        self.player_equipment
    }

    fn damage_types(&self) -> &sunrise_world::damage_source::Catalog {
        self.damage_types
    }

    fn block_entity(&self, world: &World, pos: BlockPos) -> Option<BlockEntityRef> {
        super::native_block_entity_refs::world_block_entity(
            world,
            self.level?,
            [pos.x, pos.y, pos.z],
        )
    }

    fn block_entity_data(
        &self,
        world: &World,
        registry: &BlockRegistry,
        entity: BlockEntityRef,
    ) -> Option<Compound> {
        if self.level != Some(entity.fields().level) {
            return None;
        }
        super::native_block_entity_refs::world_block_entity_data(
            world, registry, self.items, entity,
        )
    }
}

/// The game's own side, for a tick that only reads it: each query builds the borrowed tables.
impl sunrise_client::motion::SideLevel for Game<'_> {
    fn is_raining_at(&self, world: &World, pos: BlockPos) -> bool {
        game_side!(self).is_raining_at(world, pos)
    }

    fn enabled_features(&self) -> &std::sync::Arc<[String]> {
        game_side!(self).features
    }

    fn tag_items(&self) -> Option<&Items> {
        game_side!(self).tag_items
    }

    fn player_equipment(&self) -> sunrise_client::health::PlayerEquipment<'_> {
        acting_equipment!(self)
    }

    fn damage_types(&self) -> &sunrise_world::damage_source::Catalog {
        &self.health_resources.damage
    }

    fn block_entity(&self, world: &World, pos: BlockPos) -> Option<BlockEntityRef> {
        game_side!(self).block_entity(world, pos)
    }

    fn block_entity_data(
        &self,
        world: &World,
        registry: &BlockRegistry,
        entity: BlockEntityRef,
    ) -> Option<Compound> {
        game_side!(self).block_entity_data(world, registry, entity)
    }
}

type Query = (*const World, BlockPos, BlockStateId, Option<u32>);
thread_local! {
    static ACTIVE_FRICTION: RefCell<[Option<Query>; 16]> = const { RefCell::new([None; 16]) };
}

struct QueryGuard(usize);
impl Drop for QueryGuard {
    fn drop(&mut self) {
        ACTIVE_FRICTION.with_borrow_mut(|active| active[self.0] = None);
    }
}

/// The game's side of the friction evaluator. The no-subscriber path doesn't construct
/// a mod view or touch the re-entry stack.
pub(crate) fn friction(
    level: FrictionLevel<'_>,
    entity: Option<(u32, &str)>,
    pos: BlockPos,
    state: BlockStateId,
    friction: &mut f32,
) -> bool {
    if !block_state::block_listens(mod_api::HookId::Friction, state) {
        return true;
    }
    let query = (
        std::ptr::from_ref(level.world),
        pos,
        state,
        entity.map(|(id, _)| id),
    );
    let Some(_guard) = ACTIVE_FRICTION.with_borrow_mut(|active| {
        if active.contains(&Some(query)) {
            return None;
        }
        let slot = active.iter().position(Option::is_none)?;
        active[slot] = Some(query);
        Some(QueryGuard(slot))
    }) else {
        return false;
    };
    FrictionView(level).native_friction(pos, state, entity, friction);
    true
}

pub(crate) struct FrictionView<'a>(pub(in crate::game) FrictionLevel<'a>);

impl FrictionView<'_> {
    fn state_fluid(&self, state: BlockStateId) -> Option<FluidState> {
        let state = self.0.registry.state(state)?;
        super::native_world_read::api_fluid_state(EngineFluidState::from_block(state).ok()?)
    }
}

impl super::mod_api_host::SidedLevel for FrictionView<'_> {
    fn client_side(&self) -> bool {
        self.0.context.side() == LoopSide::Client
    }
}

super::native_tags::impl_tags!(FrictionView<'_>, view, view.0.side.tag_items());

impl LevelHost for FrictionView<'_> {
    fn friction(
        &self,
        [x, y, z]: [i32; 3],
        state: BlockState,
        entity: Option<(u32, &str)>,
    ) -> Option<f32> {
        query_friction(self.0, [x, y, z], state.try_into_engine(&()).ok()?, entity)
    }

    fn block_state(&self, [x, y, z]: [i32; 3]) -> Option<BlockState> {
        self.0.world.block(x, y, z).try_into_api().ok()
    }

    fn state_block(&self, state: BlockState) -> Option<&str> {
        self.0
            .registry
            .state(state.try_into_engine(&()).ok()?)
            .map(|state| &*state.name)
    }

    fn block_state_id(&self, name: &str) -> Option<BlockState> {
        super::native_block_states::parse(
            &super::native_block_states::block_registries(self.0.registry),
            self.0.registry,
            name,
        )?
        .try_into_api()
        .ok()
    }

    fn block_state_default(&self, block: BlockKey) -> Option<BlockState> {
        super::api_values::default_state(block)?.try_into_api().ok()
    }

    fn state_string(&self, state: BlockState) -> Option<String> {
        self.0
            .registry
            .state(state.try_into_engine(&()).ok()?)
            .map(super::native_block_states::serialize)
    }

    fn min_y(&self) -> i32 {
        self.0.world.shape.min_y
    }

    fn height(&self) -> i32 {
        self.0.world.shape.height
    }

    fn block_in_tag(&self, block: BlockKey, tag: &str) -> Option<bool> {
        self.0.side.tag_items()?;
        super::native_world_read::block_in_tag(block, tag)
    }

    fn block_entity(&self, [x, y, z]: [i32; 3]) -> Option<BlockEntityRef> {
        self.0
            .side
            .block_entity(self.0.world, BlockPos::new(x, y, z))
    }

    fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        self.0
            .side
            .block_entity_data(self.0.world, self.0.registry, entity)
    }

    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }

    fn fluid_state(&self, [x, y, z]: [i32; 3]) -> Option<FluidState> {
        if !self.0.world.is_loaded_at(x, y, z) {
            return None;
        }
        self.state_fluid(self.0.world.block(x, y, z))
    }

    fn state_fluid_state(&self, state: BlockState) -> Option<FluidState> {
        self.state_fluid(state.try_into_engine(&()).ok()?)
    }

    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        Some(super::native_world_read::fluid_in_tag(
            self.0.side.tag_items()?,
            fluid,
            tag,
        ))
    }

    fn state_with_property(&self, state: BlockState, key: &str, value: &str) -> Option<BlockState> {
        super::native_hooks::with_property(
            self.0.registry,
            state.try_into_engine(&()).ok()?,
            key,
            value,
        )?
        .try_into_api()
        .ok()
    }

    fn can_see_sky(&self, pos: [i32; 3]) -> Option<bool> {
        Some(super::native_world_read::can_see_sky(self.0.world, pos))
    }

    fn brightness(&self, layer: LightLayer, pos: [i32; 3]) -> i32 {
        super::native_level_refs::world_brightness(self.0.world, layer, pos)
    }

    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        let block = self.brightness(LightLayer::Block, pos);
        let sky = self.brightness(LightLayer::Sky, pos);
        block.max(sky.wrapping_sub(sky_darken))
    }

    fn is_loaded(&self, [x, y, z]: [i32; 3]) -> bool {
        self.0.world.is_loaded_at(x, y, z)
    }

    fn is_raining_at(&self, [x, y, z]: [i32; 3]) -> bool {
        self.0
            .side
            .is_raining_at(self.0.world, BlockPos::new(x, y, z))
    }

    fn enabled_features(&self) -> FeatureFlags {
        FeatureFlags::new(self.0.side.enabled_features().clone())
    }
}

fn query_friction(
    level: FrictionLevel<'_>,
    [x, y, z]: [i32; 3],
    state: BlockStateId,
    entity: Option<(u32, &str)>,
) -> Option<f32> {
    read_friction(
        level,
        entity,
        BlockPos::new(x, y, z),
        state,
        (level.coefficient)(state)?,
    )
}

impl Game<'_> {
    pub(super) fn level_friction(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        entity: Option<(u32, &str)>,
    ) -> Result<Option<f32>, LevelReadError> {
        let world = self.native_level_read_world(level).ok_or_else(|| {
            if self.level_handle_current(level) {
                LevelReadError::Unavailable
            } else {
                LevelReadError::Stale
            }
        })?;
        let facts = self.res.facts();
        if facts.states.get(state.0 as usize).is_none() {
            return Err(LevelReadError::Unavailable);
        }
        let level = FrictionLevel {
            world,
            registry: self.pack.assets.registry,
            context: &self.loop_context,
            side: &GameSide {
                dim: Dim::from_id(level.dimension_id()),
                level: Some(level),
                ..game_side!(self)
            },
            coefficient: &|id| facts.states.get(id.0 as usize).map(|facts| facts.friction),
        };
        Ok(query_friction(level, pos, state, entity))
    }

    /// The friction of `state` at `pos` under the boat `b` (`AbstractBoat.getGroundFriction`,
    /// `Leashable.angularFriction`), from `own`, the block's value.
    pub(super) fn boat_friction(
        &self,
        b: &Boat,
        pos: BlockPos,
        state: BlockStateId,
        own: f32,
    ) -> f32 {
        if !block_state::block_listens(mod_api::HookId::Friction, state) {
            return own;
        }
        let facts = self.res.facts();
        let level = FrictionLevel {
            world: &self.world,
            registry: self.pack.assets.registry,
            context: &self.loop_context,
            side: self,
            coefficient: &|id| facts.states.get(id.0 as usize).map(|facts| facts.friction),
        };
        read_friction(level, Some((b.id, b.entity_type_name())), pos, state, own).unwrap_or(own)
    }
}

#[cfg(test)]
mod tests {
    use super::super::Game;
    use super::super::native_entity_refs::tests::with_game;
    use super::TryIntoApi;
    use std::sync::atomic::Ordering;
    use sunrise_client::entities::{ArmorStand, StandMedium};
    use sunrise_client::motion::{MotionWorld, PlayerMotion};
    use sunrise_core::{geometry::Vec3, position::BlockPos};
    use sunrise_world::living_air::{LivingAirWorld, travel_in_air};

    fn item_and_orb(game: &Game<'_>) -> (f64, f64) {
        use sunrise_client::entities::{ExperienceOrb, ItemEntity};
        let at = Vec3::new(5.5, 65.0, 5.5);
        let mut item = ItemEntity {
            attachment_lifetime: Default::default(),
            rotation: [0.0; 2],
            join: Default::default(),
            health: 5,
            fire_ticks: 0,
            id: 8,
            stack: sunrise_client::items::ItemStack {
                item: "minecraft:stone".into(),
                count: 1,
                damage: 0,
                extra: None,
            },
            thrower: None,
            target: None,
            position: at,
            previous: at,
            velocity: Vec3::new(0.2, -0.04, 0.0),
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            fall_distance: 0.0,
            age: 0,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            no_gravity: false,
            vertical_collision: false,
            random: sunrise_core::random::LegacyRandom::new(0),
        };
        let step = item.tick_move(
            &game.world,
            game.res.facts(),
            game.collision,
            false,
            (0.0, 0.0),
            &[],
            None,
            None,
            &game.loop_context,
        );
        assert!(item.ground.on_ground);
        assert!(item.tick_settle(
            &game.world,
            game.pack.assets.registry,
            game.res.facts(),
            step,
            &game.loop_context,
            game,
        ));
        let mut orb = ExperienceOrb::new(9, at, Vec3::new(0.2, 0.0, 0.0), 1);
        orb.ground.on_ground = true;
        let (alive, _) = orb.tick_among(
            &game.world,
            game.res.facts(),
            game.collision,
            None,
            false,
            &mut || 0.0,
            None,
            &game.motion.fluids,
            &game.loop_context,
            Some(&game.motion.blocks),
            game,
            &mut |_, _| {},
            &[],
        );
        assert!(alive);
        assert!(orb.ground.on_ground);
        (item.velocity.x, orb.velocity.x)
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_answers_the_friction_an_entity_reads() {
        // Glass asks for its material's friction for players. White wool answers 0.98;
        // red wool answers NaN, blue 3.0, green infinity, black -0.5 and orange 1.5.
        let calls = mod_platform::__private::part_tests::friction_game_thread();
        // The client library asks the mods through the executable's callbacks.
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let floor = BlockPos::new(5, 64, 5);
            // The player's ground travel reads the block through its motion world.
            let player = |game: &mut Game<'_>| {
                let mut player = PlayerMotion::new(game.motion, Vec3::new(5.5, 65.0, 5.5)).unwrap();
                player.set_walking_speed_base(0.1).unwrap();
                player.ground.on_ground = true;
                player.velocity = Vec3::new(0.2, 0.0, 0.0);
                let side = game_side!(game);
                let mut world = MotionWorld {
                    side: &side,
                    entities: &[],
                    player: &mut player,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    tag_items: Some(game.res.items()),
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    block_registries: Some(&game.ui.registries),
                };
                let coefficient = LivingAirWorld::block_friction(&mut world, floor).unwrap();
                calls.store(0, Ordering::SeqCst);
                travel_in_air(
                    &mut world,
                    &mut sunrise_world::move_gate::MoveGate::new(),
                    Vec3::new(1.0, 0.0, 0.0),
                )
                .unwrap();
                let reads = calls.load(Ordering::SeqCst);
                (
                    coefficient,
                    world.player.position.x - 5.5,
                    world.player.velocity.x,
                    reads,
                )
            };
            // An armor stand moving 0.2 along x on the ground, after one travel: 0.2 × friction × 0.91.
            let travel_stand = |game: &Game<'_>, stand: &mut ArmorStand| {
                let scene = sunrise_client::collisions::SceneCollisions {
                    world: &game.world,
                    binding: game.collision,
                    subject: None,
                };
                let step = stand
                    .tick_move(
                        &scene,
                        game.res.facts(),
                        Some(&game.motion.blocks),
                        &game.motion.climbing,
                        false,
                        None,
                        &[],
                        &game.loop_context,
                        game,
                    )
                    .unwrap();
                stand.tick_settle(StandMedium::Air, step);
            };
            let stand = |game: &Game<'_>| {
                let mut stand = ArmorStand::new(7, Vec3::new(5.5, 65.0, 5.5), 0.0);
                stand.ground.on_ground = true;
                stand.velocity = Vec3::new(0.2, 0.0, 0.0);
                travel_stand(game, &mut stand);
                stand.velocity.x
            };
            let registry = game.pack.assets.registry;
            let material = registry.default_state("minecraft:white_wool").unwrap();
            game.world
                .set(floor.x, floor.y, floor.z + 1, material)
                .unwrap();
            let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
            // The player's ground travel moves 0.2 along x with input (1, 0, 0) and a walking speed
            // of 0.1F. Travel clamps the coefficient to 0..=1 (`computeModifiedFriction`) before
            // it uses it, so the speed is scaled by 0.21600002F / friction³ above 0.6 of that
            // clamped value, the distance is 0.2 + that speed, and the velocity after is the
            // distance times (friction × 0.91F). The coefficient the part answered is read as given.
            let mut ice_travel = None;
            for (block, player_friction, expected_distance, expected_velocity, answers) in [
                // No part: the block's own value.
                (
                    "minecraft:stone",
                    0.6_f32,
                    0.3000000014901163_f64,
                    0.16380001983940612_f64,
                    0,
                ),
                (
                    "minecraft:ice",
                    0.98_f32,
                    0.2229496210813524_f64,
                    0.19882648232869793_f64,
                    0,
                ),
                // Glass queries the material at its own position, only for the player.
                (
                    "minecraft:glass",
                    0.98_f32,
                    0.2229496210813524_f64,
                    0.19882648232869793_f64,
                    2,
                ),
                (
                    "minecraft:white_wool",
                    0.98_f32,
                    0.2229496210813524_f64,
                    0.19882648232869793_f64,
                    1,
                ),
                // A finite answer is read as given, whatever its size or sign; travel then clamps it to
                // 0..=1.
                (
                    "minecraft:orange_wool",
                    1.5_f32,
                    0.221600002422929_f64,
                    0.2016560080165566_f64,
                    1,
                ),
                (
                    "minecraft:blue_wool",
                    3.0_f32,
                    0.221600002422929_f64,
                    0.2016560080165566_f64,
                    1,
                ),
                (
                    "minecraft:black_wool",
                    -0.5_f32,
                    0.3000000014901163_f64,
                    0.0_f64,
                    1,
                ),
                // A non-finite answer is dropped for the block's own.
                (
                    "minecraft:red_wool",
                    0.6_f32,
                    0.3000000014901163_f64,
                    0.16380001983940612_f64,
                    1,
                ),
                (
                    "minecraft:green_wool",
                    0.6_f32,
                    0.3000000014901163_f64,
                    0.16380001983940612_f64,
                    1,
                ),
            ] {
                let state = registry.default_state(block).unwrap();
                game.world.set(floor.x, floor.y, floor.z, state).unwrap();
                let (coefficient, distance, velocity, reads) = player(game);
                assert_eq!(coefficient, player_friction, "{block}");
                assert_eq!(distance.to_bits(), expected_distance.to_bits(), "{block}");
                assert_eq!(velocity.to_bits(), expected_velocity.to_bits(), "{block}");
                assert_eq!(reads, answers, "part answers per travel tick: {block}");
                if block == "minecraft:ice" {
                    ice_travel = Some((distance, velocity));
                } else if block == "minecraft:glass" {
                    assert_eq!(Some((distance, velocity)), ice_travel);
                    // LevelRef asks the material at glass's position even without an entity.
                    assert_eq!(
                        level.friction(
                            game,
                            [floor.x, floor.y, floor.z],
                            material.try_into_api().unwrap(),
                            None,
                        ),
                        Ok(Some(0.98)),
                    );
                }
            }
            // An armor stand's velocity after a tick is 0.2 × (friction × 0.91f32), and an
            // item's or orb's 0.2 × (0.98f32 × friction), as the entities scale them. A finite
            // answer is used as given, whatever its size or sign.
            for (block, stand_x, item_x) in [
                (
                    "minecraft:stone",
                    0.10920001268386842_f64,
                    0.11760001182556153_f64,
                ),
                ("minecraft:ice", 0.17836000919342043, 0.19208000898361208),
                // Glass answers for the player only.
                ("minecraft:glass", 0.10920001268386842, 0.11760001182556153),
                (
                    "minecraft:white_wool",
                    0.17836000919342043,
                    0.19208000898361208,
                ),
                (
                    "minecraft:orange_wool",
                    0.27300000190734863,
                    0.29400000572204593,
                ),
                (
                    "minecraft:blue_wool",
                    0.5460000038146973,
                    0.5880000114440919,
                ),
                (
                    "minecraft:black_wool",
                    -0.09100000262260438,
                    -0.09800000190734864,
                ),
                // A non-finite answer is dropped for the block's own.
                (
                    "minecraft:red_wool",
                    0.10920001268386842,
                    0.11760001182556153,
                ),
                (
                    "minecraft:green_wool",
                    0.10920001268386842,
                    0.11760001182556153,
                ),
            ] {
                let state = registry.default_state(block).unwrap();
                game.world.set(floor.x, floor.y, floor.z, state).unwrap();
                assert_eq!(stand(game).to_bits(), stand_x.to_bits(), "{block}");
                let (item_got, orb_got) = item_and_orb(game);
                assert_eq!(item_got.to_bits(), item_x.to_bits(), "item {block}");
                assert_eq!(orb_got.to_bits(), item_x.to_bits(), "orb {block}");
            }

            // Its feet are over the lower path but the edge of its box lands on wool.
            // The next travel reads the wool's Part rather than the path's own friction.
            let path = registry.default_state("minecraft:dirt_path").unwrap();
            game.world.set(floor.x, floor.y, floor.z, material).unwrap();
            game.world.set(floor.x + 1, floor.y, floor.z, path).unwrap();
            let mut edge = ArmorStand::new(7, Vec3::new(6.125, 65.0, 5.5), 0.0);
            edge.velocity = Vec3::new(0.0, -0.08, 0.0);
            travel_stand(game, &mut edge);
            assert_eq!(edge.ground.supporting_block, Some(floor));
            edge.velocity = Vec3::new(0.2, -0.08, 0.0);
            calls.store(0, Ordering::SeqCst);
            travel_stand(game, &mut edge);
            assert_eq!(edge.velocity.x, 0.2 * f64::from(0.98_f32 * 0.91));
            assert_eq!(calls.load(Ordering::SeqCst), 1, "wool's friction Part");
        });
    }
    #[test]
    #[ignore = "daily"]
    fn nested_friction_queries_stop_at_the_depth_bound_and_unknown_states_are_refused() {
        let calls = mod_platform::__private::part_tests::friction_game_thread();
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let registry = game.pack.assets.registry;
            let lime = registry.default_state("minecraft:lime_wool").unwrap();
            let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
            calls.store(0, Ordering::SeqCst);
            // Each nested ask names another entity id, so no query repeats: only the bound stops
            // the chain. The innermost part is refused and answers 0.5, which every part above passes on.
            assert_eq!(
                level.friction(
                    game,
                    [5, 64, 5],
                    lime.try_into_api().unwrap(),
                    Some((100, "minecraft:player")),
                ),
                Ok(Some(0.5)),
            );
            assert_eq!(calls.load(Ordering::SeqCst), 16);
            // Two parts on one block: the second sees the first's coefficient, but not its
            // changes to the position, state or entity.
            let pink = registry.default_state("minecraft:pink_wool").unwrap();
            calls.store(0, Ordering::SeqCst);
            assert_eq!(
                level.friction(
                    game,
                    [5, 64, 5],
                    pink.try_into_api().unwrap(),
                    Some((100, "minecraft:player")),
                ),
                Ok(Some(0.9)),
            );
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            let unknown = sunrise_world::registry::BlockStateId(u32::MAX);
            assert_eq!(
                game.level_friction(level, [5, 64, 5], unknown, None),
                Err(mod_api::LevelReadError::Unavailable),
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_server_friction_part_reads_its_block_entitys_saved_material() {
        use super::super::native_entity_refs::tests::{
            fixture_root, run_fresh_cases, with_game_at,
        };
        use super::FrictionLevel;
        use mod_api::{
            Block, BlockEntityEvent, BlockEntityPart, BlockPart, BlockUpdateFlags, BlocksEvent,
            Compound, FrictionEvent, Hooks, LevelSide, ModEntry, Mods, Registrar,
            RegistrationContext, Tag,
        };
        use std::sync::Arc;
        use sunrise_client::chunk::Chunk;

        if run_fresh_cases(
            "game::native_surface::tests::a_server_friction_part_reads_its_block_entitys_saved_material",
            "SUNRISE_FRICTION_BLOCK_ENTITY_CHILD",
            &["server"],
        ) {
            return;
        }

        const PANEL: &str = "friction_data_test:panel";
        struct Panel;
        struct PanelEntity;
        #[mod_api::__subscriptions]
        impl Hooks for Panel {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                let panel = registrar.block_key(PANEL);
                registrar.block_part(panel, Panel);
                let kind = registrar
                    .block_entity_type("panel")
                    .blocks(&[PANEL])
                    .build();
                registrar.block_entity_part(&kind, PanelEntity).unwrap();
            }
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event
                    .register(Block::new(PANEL, "minecraft:stone"))
                    .unwrap();
            }
        }
        #[mod_api::sunrise_part]
        impl BlockPart for PanelEntity {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for PanelEntity {
            fn block_entity(
                &self,
                _: LevelSide<'_>,
                _: &mut BlockEntityEvent<'_>,
            ) -> Option<Compound> {
                Some(Compound::from([(
                    "Material".into(),
                    Tag::String("minecraft:blue_ice".into()),
                )]))
            }
        }
        #[mod_api::sunrise_part]
        impl BlockPart for Panel {
            fn friction(&self, side: LevelSide<'_>, event: &mut FrictionEvent<'_>) {
                let LevelSide::Server(level) = side else {
                    return;
                };
                let pos = [event.pos.x, event.pos.y, event.pos.z];
                let entity = level
                    .block_entity(pos)
                    .expect("the panel has a block entity");
                let Some(data) = level.block_entity_data(entity) else {
                    return;
                };
                let Some(Tag::String(material)) = data.get(&"Material".into()) else {
                    panic!("the saved panel has a material");
                };
                let state = level.block_state_id(&material.to_string_lossy()).unwrap();
                event.friction = level.friction(pos, state, None).unwrap();
            }
        }

        let mods: Mods = vec![ModEntry::new("friction_data_test", Box::new(Panel))].into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let root = crate::game::mods::content_root(&fixture_root(), &Default::default()).unwrap();
        with_game_at(root.prepared().to_path_buf(), |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let state = registry.default_state(PANEL).unwrap();
            assert_eq!(
                game.level_set_block(level, [3, 64, 3], state, BlockUpdateFlags::NONE),
                Ok(true)
            );
            let facts = game.res.facts();
            let side = game_side!(game);
            let view = FrictionLevel {
                world: &game.world,
                registry,
                context: &game.loop_context,
                side: &side,
                coefficient: &|id| facts.states.get(id.0 as usize).map(|facts| facts.friction),
            };
            let mut value = facts.get(state).friction;
            assert!(super::friction(
                view,
                None,
                BlockPos::new(3, 64, 3),
                state,
                &mut value
            ));
            assert_eq!(value, 0.989);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_friction_view_reads_tags_and_resident_block_entities_of_its_side() {
        use super::super::native_entity_refs::tests::with_game;
        use super::{FrictionLevel, FrictionView};
        use mod_api::{
            __private::game::{Fields, block_entity_ref},
            LevelHost, Tag,
        };
        let _parts = mod_platform::__private::part_tests::friction_game_thread();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let dir = tempfile::tempdir().unwrap();
            game.session_lock =
                Some(sunrise_client::save::SessionLock::acquire(dir.path()).unwrap());
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(3, 64, 3, chest).unwrap();
            game.world.block_entities.insert(
                (3, 64, 3),
                sunrise_client::containers::BlockEntity::Chest(vec![None; 27]),
            );
            let side = game_side!(game);
            let coefficient = |_: sunrise_world::registry::BlockStateId| None;
            let view = FrictionView(FrictionLevel {
                world: &game.world,
                registry: game.pack.assets.registry,
                context: &game.loop_context,
                side: &side,
                coefficient: &coefficient,
            });
            // A tagged block, an untagged one and a fluid tag answer as the game's own host does.
            let stone = mod_platform::__private::part_tests::block_key("stone");
            assert_eq!(view.block_in_tag(stone, "base_stone_overworld"), Some(true));
            assert_eq!(
                view.block_in_tag(stone, "base_stone_overworld"),
                game.block_in_tag(stone, "base_stone_overworld")
            );
            assert_eq!(view.block_in_tag(stone, "minecraft:climbable"), Some(false));
            assert_eq!(
                view.fluid_in_tag("minecraft:water", "minecraft:water"),
                Some(true)
            );
            // A resident block entity has the handle of this side's level; an empty position none.
            let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
            let found = view.block_entity([3, 64, 3]).unwrap();
            assert_eq!(Some(found), game.block_entity(level, [3, 64, 3]));
            let saved = view.block_entity_data(found).unwrap();
            assert_eq!(saved.get(&"Items".into()), Some(&Tag::List(Vec::new())));
            let fields = found.fields();
            let other_level = game.level_ref(level.epoch(), mod_api::DimensionKey::NETHER);
            let other_entity = block_entity_ref(other_level, fields.position, fields.generation);
            assert_eq!(view.block_entity_data(other_entity), None);
            assert_eq!(view.block_entity([4, 64, 3]), None);
            assert_eq!(
                view.enabled_features().iter().collect::<Vec<_>>(),
                game.enabled_feature_ids()
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn mob_travel_and_both_leash_sites_use_the_parts_answer_once() {
        use super::super::boats::Boat;
        use sunrise_client::mobs::{Mob, leash};
        let calls = mod_platform::__private::part_tests::friction_game_thread();
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let at = Vec3::new(5.5, 65.0, 5.5);
            let horse = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let ghast = game.res.mob_types().get("minecraft:ghast").unwrap().clone();
            game.mobs.push(Mob::new(99, ghast, at, 0.0));
            let mut damage = sunrise_client::health::TestLevel::new(0);
            for (block, drag, spin, listens) in [
                ("minecraft:stone", 0.10920001, 1.0920001, 0),
                ("minecraft:white_wool", 0.17836001, 1.7836001, 1),
            ] {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                game.world.set(5, 64, 5, state).unwrap();
                let snapshot: Vec<_> = game
                    .mobs
                    .iter()
                    .map(|m| m.seen(None, game.items()))
                    .collect();
                {
                    let world = game.mob_world_base(&snapshot);
                    let mut mob = Mob::new(10, horse.clone(), at, 0.0);
                    mob.ground.on_ground = true;
                    mob.velocity = Vec3::new(0.2, 0.0, 0.0);
                    calls.store(0, Ordering::SeqCst);
                    mob.travel(&world, &mut damage.level(), Vec3::ZERO, &mut Vec::new());
                    assert!(
                        (mob.velocity.x - drag).abs() < 1.0e-7,
                        "mob {block}: {}",
                        mob.velocity.x
                    );
                    assert_eq!(
                        calls.load(Ordering::SeqCst),
                        listens,
                        "one answer per mob travel tick"
                    );
                    mob.position = at;
                    mob.ground.on_ground = true;
                    mob.velocity = Vec3::ZERO;
                    mob.extra.leash = Some(leash::Holder::Mob(99));
                    mob.extra.leash_spin = 2.0;
                    calls.store(0, Ordering::SeqCst);
                    leash::tick(&mut mob, &world, &mut Vec::new());
                    assert!(
                        (mob.extra.leash_spin - spin).abs() < 1.0e-6,
                        "mob lead {block}: {}",
                        mob.extra.leash_spin
                    );
                    assert_eq!(calls.load(Ordering::SeqCst), listens);
                }
                let mut boat = Boat::new(11, "oak".into(), at, 0.0);
                boat.ground.on_ground = true;
                boat.leash_spin = 2.0;
                boat.leash = Some(leash::Holder::Mob(99));
                calls.store(0, Ordering::SeqCst);
                game.tick_boat_leash(&mut boat);
                assert!(
                    (boat.leash_spin - spin).abs() < 1.0e-6,
                    "boat lead {block}: {}",
                    boat.leash_spin
                );
                assert_eq!(calls.load(Ordering::SeqCst), listens);
            }

            let registry = game.pack.assets.registry;
            let wool = registry.default_state("minecraft:white_wool").unwrap();
            let path = registry.default_state("minecraft:dirt_path").unwrap();
            game.world.set(5, 64, 5, wool).unwrap();
            game.world.set(6, 64, 5, path).unwrap();
            let world = game.mob_world_base(&[]);
            let mut edge = Mob::new(12, horse, Vec3::new(6.25, 65.0, 5.5), 0.0);
            edge.velocity = Vec3::new(0.0, -0.08, 0.0);
            edge.travel(&world, &mut damage.level(), Vec3::ZERO, &mut Vec::new());
            assert_eq!(edge.ground.supporting_block, Some(BlockPos::new(5, 64, 5)));
            edge.velocity = Vec3::new(0.2, -0.08, 0.0);
            calls.store(0, Ordering::SeqCst);
            edge.travel(&world, &mut damage.level(), Vec3::ZERO, &mut Vec::new());
            assert_eq!(edge.velocity.x, 0.2 * f64::from(0.98_f32 * 0.91));
            assert_eq!(calls.load(Ordering::SeqCst), 1, "wool's friction Part");
        });
    }
}
