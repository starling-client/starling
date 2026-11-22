use super::*;
use sunrise_core::{connection_kind::ConnectionKind, geometry::Direction, position::BlockPos};
use sunrise_world::level::{BlockId, LevelAccess, PlacementContext, Ticks};

use crate::game::mod_api_host::{FireOddsLevel, PredictedLevel};
use std::{cell::RefCell, collections::HashSet};

/// The live level behind a prediction, providing mod dispatch when it has a game host.
pub(super) trait PredictionBase: LevelAccess {
    fn fire_odds_level<'a>(
        &'a self,
        _prediction: &'a dyn PredictedLevel,
    ) -> Option<FireOddsLevel<'a>> {
        None
    }
}

impl PredictionBase for crate::game::GameLevel<'_> {
    fn fire_odds_level<'a>(
        &'a self,
        prediction: &'a dyn PredictedLevel,
    ) -> Option<FireOddsLevel<'a>> {
        Some(FireOddsLevel {
            base: self,
            prediction: Some(prediction),
        })
    }
}

/// Scratch state for client prediction or a server placement preview: reads
/// go to the world, writes stay here and are dropped.
pub(super) struct DryLevel<'w> {
    base: &'w dyn PredictionBase,
    pub(super) changed: sunrise_world::level::PreviewBlocks,
    removed_entities: HashSet<BlockPos>,
    /// For a predicted break, the order in which the simulation reaches the after-shape notices.
    pub(super) notice_order: Option<RefCell<Vec<BlockPos>>>,
    random: sunrise_core::random::LegacyRandom,
    /// The sounds the use plays (a door's creak).
    pub(super) sounds: Vec<DrySound>,
    /// The server owns liquid-container writes, including a part's replacement.
    pub(super) filled_container: bool,
    /// The live level's ticks a mod's placement and survival hooks read, where the guess
    /// schedules into scratch ones; `None` where it runs on the live ticks.
    pub(super) reads: Option<&'w sunrise_world::level::Ticks>,
    /// Whether the guess asks mods' placement and survival hooks: a click does, a look at what
    /// a click would do (the controller guide) leaves them to the templates' rules.
    pub(super) asks_mods: bool,
}

impl<'w> DryLevel<'w> {
    pub(super) fn new(base: &'w dyn PredictionBase) -> Self {
        Self {
            base,
            changed: HashMap::new(),
            removed_entities: HashSet::new(),
            notice_order: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            sounds: Vec::new(),
            filled_container: false,
            reads: None,
            asks_mods: true,
        }
    }
}

impl crate::game::Game<'_> {
    /// Ask for the candidate state without spending the level random or scheduling live ticks.
    pub(in crate::game) fn preview_state_for_placement(
        &mut self,
        block: BlockId,
        context: &PlacementContext,
    ) -> Option<BlockStateId> {
        self.with_placement_preview_level(|behaviours, level, ticks| {
            let mut dry = DryLevel::new(level);
            dry.random = (*level.world.random).clone();
            dry.reads = Some(ticks);
            behaviours.state_for_resolved_placement(&mut dry, ticks, block, context)
        })
    }

    /// Trace the optional initial write and state-level callback off the live world.
    /// Both steps use the same scratch level so the callback sees the write's shape updates.
    pub(in crate::game) fn preview_set_placed_by(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        write_flags: Option<i32>,
    ) -> sunrise_world::level::PreviewBlocks {
        self.with_level(|behaviours, level, _| {
            let random = level.random().clone();
            let mut dry = DryLevel::new(level);
            dry.random = random;
            dry.asks_mods = false;
            let mut ticks = Ticks::default();
            if let Some(flags) = write_flags
                && !behaviours.set_block(&mut dry, &mut ticks, pos, state, flags)
            {
                return dry.changed;
            }
            let placed = dry.block_state(pos);
            if behaviours.block(placed) == behaviours.block(state) {
                behaviours.set_placed_by_state(&mut dry, &mut ticks, pos, placed);
            }
            dry.changed
        })
    }
}

#[cfg(test)]
impl crate::game::Game<'_> {
    pub(in crate::game) fn check_dry_fire_odds_view(
        &mut self,
        stage: impl FnOnce(&sunrise_world::level::Behaviours, &mut dyn LevelAccess, &mut Ticks),
        check: impl FnOnce(&dyn mod_api::LevelHost),
    ) {
        self.with_level(|behaviours, base, _| {
            let mut dry = DryLevel::new(base);
            stage(behaviours, &mut dry, &mut Ticks::default());
            check(
                &base
                    .fire_odds_level(&dry)
                    .expect("game prediction has a host"),
            );
        });
    }
}

impl sunrise_world::level::BlockEntityNeighbors for DryLevel<'_> {}

impl PredictedLevel for DryLevel<'_> {
    fn block_entity_removed(&self, pos: BlockPos) -> bool {
        self.removed_entities.contains(&pos)
    }
}

impl sunrise_world::level::LevelAccess for DryLevel<'_> {
    fn can_place_liquid(
        &self,
        pos: sunrise_core::position::BlockPos,
        state: BlockStateId,
        fluid: sunrise_world::fluid::FluidKind,
        user: sunrise_world::level::LiquidUser,
    ) -> Option<bool> {
        self.base.can_place_liquid(pos, state, fluid, user)
    }
    fn place_liquid(
        &mut self,
        _: &mut sunrise_world::level::Ticks,
        _: sunrise_core::position::BlockPos,
        _: BlockStateId,
        _: sunrise_world::fluid::FluidState,
    ) -> Option<bool> {
        self.filled_container = true;
        None
    }
    fn before_smoke_source(&self, state: BlockStateId, smoke: &mut Option<bool>) {
        self.base.before_smoke_source(state, smoke);
    }
    fn before_fire_odds(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        odds: &mut Option<(i32, i32)>,
    ) {
        if let Some(level) = self.base.fire_odds_level(self) {
            level.native_fire_odds(level.base.dimension, state, pos, face, odds);
        }
    }
    fn cake_can_eat(&self, vanilla: bool) -> bool {
        self.base.cake_can_eat(vanilla)
    }
    fn emit(&mut self, event: sunrise_world::level::LevelEvent) {
        if let sunrise_world::level::LevelEvent::Sound {
            pos,
            sound,
            source,
            volume,
            pitch,
        } = event
        {
            let source = match source {
                // Block-use prediction already plays uncategorized sounds on Blocks.
                None | Some(sunrise_world::level::LevelSoundSource::Blocks) => {
                    mod_api::SoundSource::Blocks
                }
            };
            self.sounds.push((pos, sound, source, volume, pitch));
        }
    }
    fn block_state(&self, pos: sunrise_core::position::BlockPos) -> BlockStateId {
        self.changed
            .get(&pos)
            .copied()
            .unwrap_or_else(|| self.base.block_state(pos))
    }
    fn set_raw(
        &mut self,
        pos: sunrise_core::position::BlockPos,
        state: BlockStateId,
    ) -> Option<BlockStateId> {
        let previous = self.block_state(pos);
        self.changed.insert(pos, state);
        Some(previous)
    }
    fn observes_block_state_change(&self, _: BlockStateId, _: BlockStateId) -> bool {
        self.notice_order.is_some()
    }
    fn observe_block_state_change(&self, pos: BlockPos, _: BlockStateId, _: BlockStateId) {
        if let Some(order) = &self.notice_order {
            order.borrow_mut().push(pos);
        }
    }
    fn is_loaded(&self, pos: sunrise_core::position::BlockPos) -> bool {
        self.base.is_loaded(pos)
    }
    /// `Level.isClientSide`: the preview keeps its live level's side.
    fn is_client_side(&self) -> bool {
        self.base.is_client_side()
    }
    fn has_native_block_entity(&self, state: BlockStateId) -> bool {
        self.base.has_native_block_entity(state)
    }
    fn has_mod_block_entity(&self, pos: BlockPos) -> bool {
        !self.block_entity_removed(pos) && self.base.has_mod_block_entity(pos)
    }
    fn remove_block_entity(&mut self, pos: BlockPos, _: BlockStateId, _: bool) -> u8 {
        // The simulation decides what survives a state write. Record its removal
        // without running live entity effects or touching the base.
        self.removed_entities.insert(pos);
        0
    }
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        if self.block_entity_removed(pos) {
            return None;
        }
        self.base.shulker_is_closed(pos)
    }
    fn published_collision_shape(
        &self,
        pos: sunrise_core::position::BlockPos,
        state: sunrise_world::registry::BlockStateId,
    ) -> Option<sunrise_core::geometry::VoxelShape> {
        if self.block_entity_removed(pos) {
            return None;
        }
        self.base.published_collision_shape(pos, state)
    }
    fn min_y(&self) -> i32 {
        self.base.min_y()
    }
    fn max_y(&self) -> i32 {
        self.base.max_y()
    }
    fn block_light(&self, pos: sunrise_core::position::BlockPos) -> i32 {
        self.base.block_light(pos)
    }
    fn sky_light(&self, pos: sunrise_core::position::BlockPos) -> i32 {
        self.base.sky_light(pos)
    }
    fn sky_darken(&self) -> i32 {
        self.base.sky_darken()
    }
    fn is_raining(&self) -> bool {
        self.base.is_raining()
    }
    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        self.base.biome_name(pos)
    }
    fn sea_level(&self) -> i32 {
        self.base.sea_level()
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        // The motion-blocking scan reads staged states; start above both the
        // live surface and any blocks staged above it in this column.
        self.changed
            .keys()
            .filter(|pos| pos.x == x && pos.z == z)
            .fold(self.base.world_surface(x, z), |top, pos| top.max(pos.y + 1))
    }
    fn is_unobstructed(&self, pos: sunrise_core::position::BlockPos, state: BlockStateId) -> bool {
        self.base.is_unobstructed(pos, state)
    }
    fn within_world_border(&self, pos: BlockPos) -> bool {
        self.base.within_world_border(pos)
    }
    fn redstone_experiments(&self) -> bool {
        self.base.redstone_experiments()
    }
    fn block_entity_signal(&self, pos: BlockPos, state: BlockStateId, direction: Direction) -> i32 {
        self.base.block_entity_signal(pos, state, direction)
    }
    fn has_signal_hook(&self) -> bool {
        self.asks_mods && self.base.has_signal_hook()
    }
    fn adjust_signal_source(&self, state: BlockStateId, vanilla: bool) -> bool {
        if self.asks_mods {
            self.base.adjust_signal_source(state, vanilla)
        } else {
            vanilla
        }
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        if self.asks_mods {
            self.base
                .adjust_signal(pos, state, direction, direct, vanilla)
        } else {
            vanilla
        }
    }
    fn has_weak_power_hook(&self) -> bool {
        self.asks_mods && self.base.has_weak_power_hook()
    }
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: bool,
    ) -> bool {
        self.base
            .adjust_should_check_weak_power(pos, state, direction, vanilla)
    }
    fn has_redstone_wire_connection_hook(&self) -> bool {
        self.asks_mods && self.base.has_redstone_wire_connection_hook()
    }
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Option<Direction>,
        vanilla: bool,
    ) -> bool {
        self.base
            .adjust_should_redstone_wire_connect_to(pos, state, direction, vanilla)
    }
    fn has_connection_hook(&self) -> bool {
        self.asks_mods && self.base.has_connection_hook()
    }
    fn adjust_can_be_connected_to(
        &self,
        kind: ConnectionKind,
        pos: BlockPos,
        state: BlockStateId,
        connector: BlockStateId,
        direction: Direction,
        face_solid: bool,
        vanilla: bool,
    ) -> bool {
        self.base
            .adjust_can_be_connected_to(kind, pos, state, connector, direction, face_solid, vanilla)
    }
    fn adjust_connects_to(
        &self,
        kind: ConnectionKind,
        pos: BlockPos,
        state: BlockStateId,
        neighbour: BlockStateId,
        direction: Direction,
        face_solid: bool,
        vanilla: bool,
    ) -> bool {
        self.base
            .adjust_connects_to(kind, pos, state, neighbour, direction, face_solid, vanilla)
    }
    fn fence_gate_in_wall(&self, state: BlockStateId) -> Option<bool> {
        self.asks_mods
            .then(|| self.base.fence_gate_in_wall(state))
            .flatten()
    }
    fn random(&mut self) -> &mut sunrise_core::random::LegacyRandom {
        &mut self.random
    }
    /// A mod's bone meal answer is read, never grown here: the preview asks
    /// the level it stands in for, as `isValidBonemealTarget` is read on both
    /// sides.
    fn is_valid_bonemeal_target(
        &self,
        pos: sunrise_core::position::BlockPos,
        state: BlockStateId,
    ) -> Option<bool> {
        self.base.is_valid_bonemeal_target(pos, state)
    }
    /// A mod's placement and survival answers are the level's, asked as
    /// vanilla's client prediction asks them, over this guess's writes: a
    /// neighbour's shape update after the first one reads the blocks the guess
    /// set.
    #[allow(clippy::option_option)]
    fn before_state_for_placement(
        &self,
        ticks: &sunrise_world::level::Ticks,
        _: Option<&sunrise_world::level::PreviewBlocks>,
        state: BlockStateId,
        context: &sunrise_world::level::PlacementContext,
        fluid: sunrise_world::fluid::FluidState,
        answer: &mut Option<Option<BlockStateId>>,
    ) {
        if self.asks_mods {
            self.base.before_state_for_placement(
                self.reads.unwrap_or(ticks),
                Some(&self.changed),
                state,
                context,
                fluid,
                answer,
            );
        }
    }
    fn before_can_survive(
        &self,
        ticks: &sunrise_world::level::Ticks,
        _: Option<&sunrise_world::level::PreviewBlocks>,
        pos: sunrise_core::position::BlockPos,
        state: BlockStateId,
        survives: &mut Option<bool>,
    ) -> bool {
        self.asks_mods
            && self.base.before_can_survive(
                self.reads.unwrap_or(ticks),
                Some(&self.changed),
                pos,
                state,
                survives,
            )
    }
    fn before_can_sustain_plant(
        &self,
        ticks: &sunrise_world::level::Ticks,
        _: Option<&sunrise_world::level::PreviewBlocks>,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: sunrise_core::position::BlockPos,
        sustains: &mut Option<bool>,
    ) -> bool {
        self.asks_mods
            && self.base.before_can_sustain_plant(
                self.reads.unwrap_or(ticks),
                Some(&self.changed),
                state,
                ground,
                ground_pos,
                sustains,
            )
    }
    fn before_may_place_on(
        &self,
        ticks: &sunrise_world::level::Ticks,
        _: Option<&sunrise_world::level::PreviewBlocks>,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: sunrise_core::position::BlockPos,
        may: &mut Option<bool>,
    ) -> bool {
        self.asks_mods
            && self.base.before_may_place_on(
                self.reads.unwrap_or(ticks),
                Some(&self.changed),
                state,
                ground,
                ground_pos,
                may,
            )
    }
    fn must_survive(&self, item: &str) -> Option<bool> {
        if self.asks_mods {
            self.base.must_survive(item)
        } else {
            None
        }
    }
    fn game_time(&self) -> i64 {
        self.base.game_time()
    }
}
