//! Soil and plant support rules over an in-memory level.
use super::tests::{TestLevel, fixture};
use super::*;

/// Uses prepared client resources, as `tests` does.
#[test]
fn a_soils_answer_comes_before_the_plants_may_place_on() {
    use std::cell::RefCell;

    /// A level whose soil answers `canSustainPlant` for one block, and whose plant answers
    /// `mayPlaceOn`, recording what the soil was asked.
    struct Soil {
        inner: TestLevel,
        b: &'static Behaviours,
        /// A soil block and its answer.
        soil: Option<(BlockId, bool)>,
        /// A plant and the one block its own `mayPlaceOn` allows.
        plant: Option<(BlockId, BlockId)>,
        asked: RefCell<Vec<(BlockId, BlockId, BlockPos)>>,
    }
    impl BlockEntityNeighbors for Soil {}

    impl LevelAccess for Soil {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.inner.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.inner.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.inner.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.inner.random()
        }
        fn game_time(&self) -> i64 {
            self.inner.game_time()
        }
        fn emit(&mut self, event: LevelEvent) {
            self.inner.emit(event);
        }
        fn before_can_sustain_plant(
            &self,
            _: &Ticks,
            _: Option<&PreviewBlocks>,
            state: BlockStateId,
            ground: BlockStateId,
            ground_pos: BlockPos,
            sustains: &mut Option<bool>,
        ) -> bool {
            self.asked
                .borrow_mut()
                .push((self.b.block(state), self.b.block(ground), ground_pos));
            if let Some((soil, answer)) = self.soil
                && self.b.block(ground) == soil
            {
                *sustains = Some(answer);
            }
            false
        }
        fn before_may_place_on(
            &self,
            _: &Ticks,
            _: Option<&PreviewBlocks>,
            state: BlockStateId,
            ground: BlockStateId,
            _ground_pos: BlockPos,
            may: &mut Option<bool>,
        ) -> bool {
            if let Some((plant, on)) = self.plant
                && self.b.block(state) == plant
            {
                *may = Some(self.b.block(ground) == on);
            }
            false
        }
    }
    let Some(f) = fixture() else { return };
    let b = &f.behaviours;
    let state = |name: &str| f.registry.default_state(name).unwrap();
    let (dandelion, stone, grass, cobblestone) = (
        state("minecraft:dandelion"),
        state("minecraft:stone"),
        state("minecraft:grass_block"),
        state("minecraft:cobblestone"),
    );
    let mut level = Soil {
        inner: TestLevel::new(b),
        b,
        soil: None,
        plant: None,
        asked: RefCell::new(Vec::new()),
    };
    let mut ticks = Ticks::new(0);
    ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    let ground = BlockPos::new(1, 1, 1);
    let pos = BlockPos::new(1, 2, 1);
    let place = |level: &mut Soil, ticks: &mut Ticks| {
        let ctx = PlaceContext {
            clicked_pos: ground,
            face: Direction::Up,
            hit: sunrise_core::geometry::Vec3::new(1.5, 2.0, 1.5),
            inside: false,
            source: PlacementSource::Player {
                yaw: 0.0,
                pitch: 0.0,
                sneaking: false,
            },
            item: "minecraft:dandelion".into(),
            can_use_game_master_blocks: false,
        };
        b.place_block(level, ticks, &ctx)
    };

    // Default: no soil answers, so vanilla's rule runs. A dandelion stands on grass and not on stone.
    level.set_raw(ground, grass);
    assert!(b.can_survive(&mut level, &mut ticks, pos, dandelion));
    level.set_raw(ground, stone);
    assert!(!b.can_survive(&mut level, &mut ticks, pos, dandelion));
    // The soil is asked for the plant above it, at its own position.
    assert_eq!(
        level.asked.borrow().last().copied(),
        Some((b.block(dandelion), b.block(stone), ground))
    );
    assert_eq!(
        place(&mut level, &mut ticks),
        PlaceOutcome::Failed,
        "no answer from the soil refuses the dandelion on stone"
    );

    // Allow: the soil lets the dandelion stand on stone, so it is placed there.
    level.soil = Some((b.block(stone), true));
    assert!(b.can_survive(&mut level, &mut ticks, pos, dandelion));
    assert!(matches!(
        place(&mut level, &mut ticks),
        PlaceOutcome::Placed { pos: p, .. } if p == pos
    ));
    assert_eq!(level.block_state(pos), dandelion);
    // A soil that answers for stone leaves other soil to vanilla.
    level.set_raw(pos, b.air());
    level.set_raw(ground, grass);
    assert!(b.can_survive(&mut level, &mut ticks, pos, dandelion));
    level.set_raw(ground, cobblestone);
    assert!(!b.can_survive(&mut level, &mut ticks, pos, dandelion));

    // Deny: the soil refuses a dandelion on grass, where vanilla allows it.
    level.soil = Some((b.block(grass), false));
    level.set_raw(ground, grass);
    assert!(!b.can_survive(&mut level, &mut ticks, pos, dandelion));
    assert_eq!(place(&mut level, &mut ticks), PlaceOutcome::Failed);
    // A planted dandelion pops off when its ground turns to a soil that refuses it.
    level.soil = None;
    level.set_raw(pos, dandelion);
    level.soil = Some((b.block(grass), false));
    level.set_raw(ground, stone);
    b.set_block(&mut level, &mut ticks, ground, grass, flags::UPDATE_ALL);
    assert_eq!(level.block_state(pos), b.air());

    // The soil's answer comes before the plant's own: the plant allows only stone, the soil
    // refuses stone and allows grass, and the soil's answer wins both ways.
    level.set_raw(pos, b.air());
    level.plant = Some((b.block(dandelion), b.block(stone)));
    level.soil = Some((b.block(stone), false));
    level.set_raw(ground, stone);
    assert!(!b.can_survive(&mut level, &mut ticks, pos, dandelion));
    level.soil = Some((b.block(grass), true));
    level.set_raw(ground, grass);
    assert!(b.can_survive(&mut level, &mut ticks, pos, dandelion));
    // With no answer from that soil the plant's `mayPlaceOn` decides.
    level.soil = Some((b.block(cobblestone), true));
    level.set_raw(ground, stone);
    assert!(b.can_survive(&mut level, &mut ticks, pos, dandelion));
    level.set_raw(ground, grass);
    assert!(!b.can_survive(&mut level, &mut ticks, pos, dandelion));
}

#[test]
fn piston_settlement_keeps_a_plant_with_an_inert_support_part() {
    struct InertSupport {
        level: TestLevel,
        plant: BlockStateId,
        soil: BlockStateId,
        on_soil: bool,
    }
    impl BlockEntityNeighbors for InertSupport {}
    impl LevelAccess for InertSupport {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.level.block_state(pos)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            self.level.set_raw(pos, state)
        }
        fn is_loaded(&self, pos: BlockPos) -> bool {
            self.level.is_loaded(pos)
        }
        fn random(&mut self) -> &mut LegacyRandom {
            self.level.random()
        }
        fn game_time(&self) -> i64 {
            self.level.game_time()
        }
        fn before_can_sustain_plant(
            &self,
            _: &Ticks,
            _: Option<&PreviewBlocks>,
            state: BlockStateId,
            ground: BlockStateId,
            _: BlockPos,
            _: &mut Option<bool>,
        ) -> bool {
            self.on_soil && state == self.plant && ground == self.soil
        }
        fn before_may_place_on(
            &self,
            _: &Ticks,
            _: Option<&PreviewBlocks>,
            state: BlockStateId,
            _: BlockStateId,
            _: BlockPos,
            _: &mut Option<bool>,
        ) -> bool {
            !self.on_soil && state == self.plant
        }
    }

    let b = data::inert_support_fixture();
    let state = |name| b.default_of(b.table.block_by_name(name).unwrap());
    let plant = state("test:plant");
    let soil = state("test:soil");
    let moving = state("minecraft:moving_piston");
    let pos = BlockPos::new(1, 2, 1);
    for on_soil in [true, false] {
        let mut level = InertSupport {
            level: TestLevel::new(&b),
            plant,
            soil,
            on_soil,
        };
        let mut ticks = Ticks::new(0);
        ticks.add_empty_chunk(pos.chunk()).unwrap();
        level.set_raw(pos.offset(0, -1, 0), soil);
        assert!(!b.can_survive(&mut level, &mut ticks, pos, plant));

        level.set_raw(pos, moving);
        ticks.load_moving_piston(
            pos,
            MovingPiston {
                moved_state: plant,
                direction: Direction::East,
                extending: true,
                source: false,
                progress: 1.0,
                progress_o: 0.5,
                last_ticked: 0,
                death_ticks: 0,
                id: 0,
            },
        );
        b.tick_block_entities(&mut level, &mut ticks);
        assert_eq!(
            level.block_state(pos),
            plant,
            "inert Part on soil: {on_soil}"
        );
    }
}
