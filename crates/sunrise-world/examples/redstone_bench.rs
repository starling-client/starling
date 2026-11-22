//! Times redstone circuits through the real block-behaviour path.
//!
//! Each scenario builds a circuit on a dense box of block states, drives it,
//! and reports the best round plus the level traffic per step: a step is one
//! lever toggle, or one tick for a circuit that runs on its own.
//!
//! The families are the shapes a world spends its redstone time on:
//!
//! - a wire line, where every wire has two sources;
//! - a wire field, where every wire has four to eight — the shape a redstone
//!   computer's memory is, and the one the per-wire walks are quadratic in;
//! - a repeater-fed chain, which outruns a single wire's fifteen blocks;
//! - a comparator-fed chain, whose output moves without writing a block;
//! - a piston lane, where a toggle is a block event and a moving block;
//! - and a sweep of random machines over the whole component palette, so the
//!   combinations nobody thought to hand-build are measured too.
//!
//! `SUNRISE_RESOURCES` selects the prepared resources (default:
//! `.local/sunrise-client/<version>`), the same as the parity probes.

use std::{cell::Cell, path::PathBuf, time::Instant};
use sunrise_core::{
    geometry::{BlockHit, Direction, Vec3},
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
    ticks::ChunkTicks,
};
use sunrise_world::{
    level::{
        Behaviours, BlockEntityNeighbors, LevelAccess, PlayerContext, Ticks, data::load_registry,
    },
    registry::{BlockRegistry, BlockStateId},
};

const LENGTH: i32 = 128;
const LINES: i32 = 4;
const PLATE: i32 = 16;
const TOGGLES: i32 = 40;
/// Rounds of `TOGGLES`; the run reports each and the best, because this
/// machine's timings drift by tens of percent between runs.
const ROUNDS: usize = 5;

/// Repeater chain: `DIODE_STAGES` stages of `DIODE_RUN` wire, then a lamp.
const DIODE_LANES: i32 = 4;
const DIODE_STAGES: i32 = 3;
const DIODE_RUN: i32 = 12;
/// Comparator chain, same shape, comparators instead of repeaters.
const COMPARE_LANES: i32 = 4;
const COMPARE_STAGES: i32 = 3;
/// Piston lanes: `PISTON_RUN` wire into a piston and the block it pushes.
const PISTON_LANES: i32 = 4;
const PISTON_RUN: i32 = 12;
/// Random machines: a `RANDOM_SIZE` square of floor, `RANDOM_BLOCKS` components
/// and `RANDOM_LEVERS` levers, over `RANDOM_SEEDS` seeds.
const RANDOM_SIZE: i32 = 14;
const RANDOM_BLOCKS: usize = 96;
const RANDOM_LEVERS: usize = 6;
const RANDOM_SEEDS: u64 = 12;

/// Room for the circuits, their support and the probes redstone makes around
/// them, and the corner it starts at.
const BOX: (i32, i32, i32) = (192, 24, 32);
const ORIGIN: (i32, i32, i32) = (-16, 54, -8);

/// A dense box of block states: the circuits are a few hundred cells, so an
/// array read is what the game's chunk read costs. A `HashMap` here would make
/// the run measure hash overhead instead of redstone.
struct Cells {
    states: Vec<BlockStateId>,
    origin: (i32, i32, i32),
    size: (i32, i32, i32),
    air: BlockStateId,
    random: LegacyRandom,
    time: i64,
    /// Level traffic, so the run can say what a circuit costs in reads and
    /// writes rather than only in time.
    reads: Cell<u64>,
    writes: Cell<u64>,
}

impl Cells {
    fn new(air: BlockStateId) -> Self {
        let (sx, sy, sz) = BOX;
        Self {
            states: vec![air; (sx * sy * sz) as usize],
            origin: ORIGIN,
            size: BOX,
            air,
            random: LegacyRandom::new(0),
            time: 0,
            reads: Cell::new(0),
            writes: Cell::new(0),
        }
    }

    fn slot(&self, pos: BlockPos) -> Option<usize> {
        let (ox, oy, oz) = self.origin;
        let (sx, sy, sz) = self.size;
        let (x, y, z) = (pos.x - ox, pos.y - oy, pos.z - oz);
        ((0..sx).contains(&x) && (0..sy).contains(&y) && (0..sz).contains(&z))
            .then(|| ((y * sz + z) * sx + x) as usize)
    }

    fn set(&mut self, pos: BlockPos, state: BlockStateId) {
        let slot = self
            .slot(pos)
            .unwrap_or_else(|| panic!("write outside the benchmark box at {pos:?}"));
        self.states[slot] = state;
    }
}

impl BlockEntityNeighbors for Cells {}
impl LevelAccess for Cells {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.reads.set(self.reads.get() + 1);
        self.slot(pos).map_or(self.air, |slot| self.states[slot])
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        self.writes.set(self.writes.get() + 1);
        let slot = self.slot(pos)?;
        let old = self.states[slot];
        self.states[slot] = state;
        Some(old)
    }
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.time
    }
    fn block_entity_analog_signal(
        &mut self,
        _pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        0
    }
}

/// Block states by name and property, so a circuit can be built the way a
/// player builds it.
struct Palette<'a> {
    registry: &'a BlockRegistry,
}

impl Palette<'_> {
    fn id(&self, name: &str) -> BlockStateId {
        self.registry
            .default_state(name)
            .unwrap_or_else(|| panic!("registry has no {name}"))
    }

    fn with(&self, name: &str, properties: &[(&str, &str)]) -> BlockStateId {
        let base = self.id(name);
        let mut block = self
            .registry
            .state(base)
            .expect("a default state is in the registry")
            .clone();
        for (key, value) in properties {
            block
                .properties
                .insert((*key).to_string(), (*value).to_string());
        }
        self.registry
            .id(&block)
            .unwrap_or_else(|| panic!("{name} has no state with {properties:?}"))
    }
}

struct Scenario {
    name: String,
    level: Cells,
    ticks: Ticks,
    levers: Vec<BlockPos>,
}

/// A fresh level and its ticks. `Ticks` only schedules in a registered chunk,
/// as the game's `LevelTicks` does, so a circuit that never registers one has
/// its repeater ticks dropped and quietly does nothing.
fn scenario(air: BlockStateId) -> (Cells, Ticks) {
    let mut ticks = Ticks::new(0);
    let (sx, _, sz) = BOX;
    let (ox, _, oz) = ORIGIN;
    for cx in ox.div_euclid(16)..=(ox + sx).div_euclid(16) {
        for cz in oz.div_euclid(16)..=(oz + sz).div_euclid(16) {
            ticks
                .add_chunk(
                    ChunkPos::new(cx, cz),
                    ChunkTicks::default(),
                    ChunkTicks::default(),
                )
                .expect("a fresh chunk registers");
        }
    }
    (Cells::new(air), ticks)
}

fn flip(behaviours: &Behaviours, level: &mut Cells, ticks: &mut Ticks, at: BlockPos) {
    let state = level.block_state(at);
    let _ = behaviours.use_without_item(
        level,
        ticks,
        state,
        BlockHit {
            block: at,
            direction: Direction::Up,
            location: Vec3::new(at.x as f64 + 0.5, at.y as f64 + 0.5, at.z as f64 + 0.5),
            inside: false,
        },
        PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: false,
            game_master: false,
            respawn_point: None,
        },
    );
}

/// Whole level ticks until quiet, capped.
fn run(behaviours: &Behaviours, level: &mut Cells, ticks: &mut Ticks) -> u32 {
    let mut t = 0;
    while t < 64
        && (ticks.blocks.count() != 0
            || ticks.fluids.count() != 0
            || !ticks.block_events.is_empty()
            || !ticks.moving_pistons.is_empty())
    {
        t += 1;
        level.time += 1;
        behaviours.tick_scheduled(level, ticks);
        behaviours.run_block_events(level, ticks);
        behaviours.tick_block_entities(level, ticks);
    }
    t
}

/// What one scenario cost. The fastest round's time is per round, so the
/// timing divides by one round's steps; the traffic is summed over every
/// round, so it divides by all of them.
#[derive(Clone, Copy, Default)]
struct Reading {
    /// The fastest round.
    best: std::time::Duration,
    /// Steps in one round, summed over the scenarios merged into this reading.
    steps: u64,
    /// Rounds the traffic was summed over.
    rounds: u64,
    reads: u64,
    writes: u64,
}

impl Reading {
    fn merge(self, other: Self) -> Self {
        Self {
            best: self.best + other.best,
            steps: self.steps + other.steps,
            rounds: self.rounds.max(other.rounds),
            reads: self.reads + other.reads,
            writes: self.writes + other.writes,
        }
    }

    fn per_step(&self) -> f64 {
        self.best.as_secs_f64() * 1e6 / self.steps as f64
    }

    fn reads_per_step(&self) -> f64 {
        self.reads as f64 / (self.steps * self.rounds) as f64
    }

    fn writes_per_step(&self) -> f64 {
        self.writes as f64 / (self.steps * self.rounds) as f64
    }
}

/// Drive a circuit `ROUNDS` times and report the best round, so a machine that
/// is busy for other reasons cannot pass itself off as a result. Every scenario
/// must change a block: one that does nothing is a broken circuit, not a fast
/// one.
fn measure(behaviours: &Behaviours, scenario: &mut Scenario) -> Reading {
    let mut reading = Reading::default();
    let mut rounds = Vec::new();
    for round in 0..ROUNDS {
        scenario.level.reads.set(0);
        scenario.level.writes.set(0);
        let started = Instant::now();
        for _ in 0..TOGGLES {
            for index in 0..scenario.levers.len() {
                let at = scenario.levers[index];
                flip(behaviours, &mut scenario.level, &mut scenario.ticks, at);
                run(behaviours, &mut scenario.level, &mut scenario.ticks);
            }
        }
        rounds.push(started.elapsed());
        reading.reads += scenario.level.reads.get();
        reading.writes += scenario.level.writes.get();
        if round == 0 && scenario.level.writes.get() == 0 {
            panic!("{} never changed a block", scenario.name);
        }
    }
    reading.steps = TOGGLES as u64 * scenario.levers.len() as u64;
    reading.rounds = ROUNDS as u64;
    reading.best = rounds.iter().min().copied().unwrap_or_default();
    println!(
        "{:44} {:>9.1} us/step {:>8.0} reads {:>6.1} writes   rounds {rounds:.3?}",
        scenario.name,
        reading.per_step(),
        reading.reads_per_step(),
        reading.writes_per_step(),
    );
    reading
}

/// A line: two sources per wire, the shape the per-wire walks handle most
/// cheaply.
fn line(p: &Palette) -> Scenario {
    let (mut level, ticks) = scenario(p.id("minecraft:air"));
    let mut levers = Vec::new();
    for line in 0..LINES {
        let z = line * 2;
        let lever_pos = BlockPos::new(-1, 64, z);
        level.set(lever_pos, p.id("minecraft:lever"));
        levers.push(lever_pos);
        for x in 0..LENGTH {
            level.set(BlockPos::new(x, 63, z), p.id("minecraft:stone"));
            level.set(BlockPos::new(x, 64, z), p.id("minecraft:redstone_wire"));
        }
    }
    Scenario {
        name: format!("{LINES} lines x {LENGTH} wire"),
        level,
        ticks,
        levers,
    }
}

/// A field: up to eight sources per wire, and every one of them probed from
/// every direction.
fn field(p: &Palette) -> Scenario {
    let (mut level, ticks) = scenario(p.id("minecraft:air"));
    let lever = BlockPos::new(-1, 64, 0);
    level.set(lever, p.id("minecraft:lever"));
    for x in 0..PLATE {
        for z in 0..PLATE {
            level.set(BlockPos::new(x, 63, z), p.id("minecraft:stone"));
            level.set(BlockPos::new(x, 64, z), p.id("minecraft:redstone_wire"));
        }
    }
    Scenario {
        name: format!("{PLATE} x {PLATE} wire field"),
        level,
        ticks,
        levers: vec![lever],
    }
}

/// A chain of wire runs separated by one diode family: the wires are the
/// graph's work, but every stage writes its own state as the signal reaches
/// it, and each of those writes retires the run's cache. This is the shape a
/// circuit that outruns a single wire's fifteen blocks has.
fn diode_chain(p: &Palette, diode: BlockStateId, stages: i32, lanes: i32, name: &str) -> Scenario {
    let (mut level, ticks) = scenario(p.id("minecraft:air"));
    let wire = p.id("minecraft:redstone_wire");
    let stone = p.id("minecraft:stone");
    let mut levers = Vec::new();
    for lane in 0..lanes {
        let z = lane * 3;
        let lever = BlockPos::new(-1, 64, z);
        level.set(lever, p.id("minecraft:lever"));
        levers.push(lever);
        let mut x = 0;
        for _ in 0..stages {
            for _ in 0..DIODE_RUN {
                level.set(BlockPos::new(x, 63, z), stone);
                level.set(BlockPos::new(x, 64, z), wire);
                x += 1;
            }
            level.set(BlockPos::new(x, 63, z), stone);
            level.set(BlockPos::new(x, 64, z), diode);
            x += 1;
        }
        for _ in 0..DIODE_RUN {
            level.set(BlockPos::new(x, 63, z), stone);
            level.set(BlockPos::new(x, 64, z), wire);
            x += 1;
        }
        level.set(BlockPos::new(x, 63, z), stone);
        level.set(
            BlockPos::new(x, 64, z),
            p.with("minecraft:redstone_lamp", &[("lit", "false")]),
        );
    }
    Scenario {
        name: format!("{lanes} x {stages}-stage {name}"),
        level,
        ticks,
        levers,
    }
}

/// A piston lane: each lever drives a wire into a piston that pushes a block,
/// so a toggle is a block event, a moving-piston entity and a final write, none
/// of which the wire graph touches.
fn piston_lane(p: &Palette, piston: BlockStateId) -> Scenario {
    let (mut level, ticks) = scenario(p.id("minecraft:air"));
    let mut levers = Vec::new();
    for lane in 0..PISTON_LANES {
        let z = lane * 4;
        let lever = BlockPos::new(-1, 64, z);
        level.set(lever, p.id("minecraft:lever"));
        levers.push(lever);
        for x in 0..PISTON_RUN {
            level.set(BlockPos::new(x, 63, z), p.id("minecraft:stone"));
            level.set(BlockPos::new(x, 64, z), p.id("minecraft:redstone_wire"));
        }
        level.set(BlockPos::new(PISTON_RUN, 64, z), piston);
        level.set(
            BlockPos::new(PISTON_RUN + 1, 64, z),
            p.id("minecraft:stone"),
        );
    }
    let name = if piston
        == p.with(
            "minecraft:piston",
            &[("facing", "east"), ("extended", "false")],
        ) {
        "piston lane"
    } else {
        "sticky piston lane"
    };
    Scenario {
        name: format!("{PISTON_LANES} x {name}"),
        level,
        ticks,
        levers,
    }
}

/// The blocks a random machine is built from: their name, and the properties
/// that make them face a random way or run at a random delay.
const RANDOM_PALETTE: &[(&str, &[(&str, &str)])] = &[
    ("minecraft:redstone_wire", &[]),
    ("minecraft:redstone_wire", &[]),
    ("minecraft:redstone_wire", &[]),
    ("minecraft:redstone_wire", &[]),
    ("minecraft:redstone_wire", &[]),
    ("minecraft:redstone_wire", &[]),
    ("minecraft:repeater", &[("facing", "north")]),
    ("minecraft:repeater", &[("facing", "south")]),
    ("minecraft:repeater", &[("facing", "east")]),
    ("minecraft:repeater", &[("facing", "west")]),
    ("minecraft:comparator", &[("facing", "north")]),
    ("minecraft:comparator", &[("facing", "south")]),
    ("minecraft:comparator", &[("facing", "east")]),
    ("minecraft:comparator", &[("facing", "west")]),
    ("minecraft:redstone_torch", &[]),
    ("minecraft:observer", &[("facing", "up")]),
    ("minecraft:observer", &[("facing", "down")]),
    ("minecraft:observer", &[("facing", "north")]),
    (
        "minecraft:piston",
        &[("facing", "up"), ("extended", "false")],
    ),
    (
        "minecraft:piston",
        &[("facing", "north"), ("extended", "false")],
    ),
    (
        "minecraft:sticky_piston",
        &[("facing", "east"), ("extended", "false")],
    ),
    ("minecraft:redstone_lamp", &[("lit", "false")]),
    (
        "minecraft:lever",
        &[("face", "floor"), ("powered", "false")],
    ),
    ("minecraft:note_block", &[]),
    ("minecraft:stone_button", &[("powered", "false")]),
    ("minecraft:stone", &[]),
    ("minecraft:slime_block", &[]),
];

/// Splitmix: a deterministic value per (seed, index), so the sweep covers the
/// same machines on every run and on every tree.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A machine of random components on a stone floor, driven by its levers.
///
/// The components are laid along a random walk rather than scattered, because
/// scattered parts do not touch and a machine whose wires never meet measures
/// nothing. The components are placed the way a map would hold them, not the
/// way a player would have to earn the placement, so some are unsupported and
/// fall on their first tick: a world does that too, and both trees do it.
fn random_machine(p: &Palette, seed: u64) -> Scenario {
    let (mut level, ticks) = scenario(p.id("minecraft:air"));
    for x in 0..RANDOM_SIZE {
        for z in 0..RANDOM_SIZE {
            level.set(BlockPos::new(x, 63, z), p.id("minecraft:stone"));
        }
    }
    let roll = |index: u64| mix(seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut x = (roll(0) % RANDOM_SIZE as u64) as i32;
    let mut z = (roll(1) % RANDOM_SIZE as u64) as i32;
    let mut walk = Vec::new();
    for index in 0..RANDOM_BLOCKS as u64 {
        let (name, properties) = RANDOM_PALETTE[(roll(index) as usize) % RANDOM_PALETTE.len()];
        let at = BlockPos::new(x, 64, z);
        level.set(at, p.with(name, properties));
        walk.push(at);
        let (dx, dz) = match roll(index ^ 0xD00D) % 4 {
            0 => (1, 0),
            1 => (-1, 0),
            2 => (0, 1),
            _ => (0, -1),
        };
        x = (x + dx).clamp(0, RANDOM_SIZE - 1);
        z = (z + dz).clamp(0, RANDOM_SIZE - 1);
    }
    // Levers on the walk, so each one drives the circuit around it rather
    // than standing in the empty corner of the box.
    let mut levers = Vec::new();
    for index in 0..RANDOM_LEVERS {
        let pos = walk[(roll(0x1000 + index as u64) as usize) % walk.len()];
        level.set(
            pos,
            p.with(
                "minecraft:lever",
                &[("face", "floor"), ("powered", "false")],
            ),
        );
        levers.push(pos);
    }
    Scenario {
        name: format!("random machine seed {seed}"),
        level,
        ticks,
        levers,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let resources = std::env::var_os("SUNRISE_RESOURCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::VANILLA_VERSION)
        });
    let registry = load_registry(&resources)?;
    let behaviours = Behaviours::load(&registry, &resources)?;
    let p = Palette {
        registry: &registry,
    };

    let mut scenarios = [
        line(&p),
        field(&p),
        diode_chain(
            &p,
            p.with("minecraft:repeater", &[("facing", "west")]),
            DIODE_STAGES,
            DIODE_LANES,
            "repeater chain",
        ),
        diode_chain(
            &p,
            p.with(
                "minecraft:comparator",
                &[("facing", "west"), ("mode", "compare")],
            ),
            COMPARE_STAGES,
            COMPARE_LANES,
            "comparator chain",
        ),
        piston_lane(
            &p,
            p.with(
                "minecraft:piston",
                &[("facing", "east"), ("extended", "false")],
            ),
        ),
        piston_lane(
            &p,
            p.with(
                "minecraft:sticky_piston",
                &[("facing", "east"), ("extended", "false")],
            ),
        ),
    ];
    let mut random = Vec::new();
    for seed in 0..RANDOM_SEEDS {
        random.push(random_machine(&p, seed));
    }

    // Warm up: the first toggle also gives every wire its connection shape.
    for scenario in scenarios.iter_mut().chain(random.iter_mut()) {
        for at in scenario.levers.clone() {
            flip(&behaviours, &mut scenario.level, &mut scenario.ticks, at);
        }
        run(&behaviours, &mut scenario.level, &mut scenario.ticks);
    }

    let mut fixed = Reading::default();
    for scenario in scenarios.iter_mut() {
        fixed = fixed.merge(measure(&behaviours, scenario));
    }
    let mut swept = Reading::default();
    for scenario in random.iter_mut() {
        swept = swept.merge(measure(&behaviours, scenario));
    }
    println!(
        "{:44} {:>9.1} us/step {:>8.0} reads {:>6.1} writes   {} machines, {} steps",
        "all hand-built circuits",
        fixed.per_step(),
        fixed.reads_per_step(),
        fixed.writes_per_step(),
        scenarios.len(),
        fixed.steps,
    );
    println!(
        "{:44} {:>9.1} us/step {:>8.0} reads {:>6.1} writes   {} machines, {} steps",
        "random machine sweep",
        swept.per_step(),
        swept.reads_per_step(),
        swept.writes_per_step(),
        random.len(),
        swept.steps,
    );
    Ok(())
}
