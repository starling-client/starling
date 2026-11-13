//! Create's mechanical and windmill bearings (Starling; ported from Create 6.0.10, MIT,
//! licenses/create-MIT.txt): MechanicalBearingBlockEntity, WindmillBearingBlockEntity,
//! BearingContraption and the block attachment rules (BlockMovementChecks) they assemble by.
//! A bearing assembles the blocks in front of it (its anchor, and every block attached to what
//! it holds: sails in their plane, things hung or stood on a block, what slime or honey holds)
//! out of the world into its block entity and turns them round its facing's axis; the window
//! draws them turning (kinetics.rs). A mechanical one assembles when its network turns it and
//! puts the blocks back, turned to the nearest quarter, when it stops; a windmill one is
//! assembled with an empty hand and is a source, 1 RPM for every 8 sails (16 at most).
use super::Game;
use horizon_core::nbt::Tag;
use horizon_core::position::BlockPos;
use horizon_world::level::flags;
use horizon_world::registry::BlockStateId;
use std::collections::{HashMap, HashSet, VecDeque};

/// `maxBlocksMoved`.
const MAX_BLOCKS: usize = 2048;
/// `minimumWindmillSails`, `windmillSailsPerRPM`.
const MIN_SAILS: usize = 8;
const SAILS_PER_RPM: usize = 8;
/// `KineticBlockEntity.convertToAngular`: degrees a tick per RPM.
pub(super) const DEGREES_PER_RPM: f32 = 0.3;

/// A bearing's contraption as its tag keeps it.
#[derive(Clone, Debug, Default)]
pub(super) struct Contraption {
    /// Each block, from the anchor, and its state.
    pub(super) blocks: Vec<([i32; 3], BlockStateId)>,
    pub(super) sails: usize,
    /// Its angle (degrees) when `since` was, and how fast it turns (degrees a tick).
    pub(super) angle: f32,
    pub(super) speed: f32,
}

impl Contraption {
    pub(super) fn read(tag: &Tag, registry: &horizon_world::registry::BlockRegistry) -> Self {
        let float = |k: &str| match tag.get(k) {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            _ => 0.0,
        };
        let int = |t: &Tag, k: &str| match t.get(k) {
            Some(Tag::Int(v)) => *v,
            _ => 0,
        };
        let mut blocks = Vec::new();
        if let Some(Tag::List(list)) = tag.get("Blocks") {
            for b in list {
                let state = b
                    .get("State")
                    .and_then(|s| horizon_world::chunk::BlockState::from_nbt(s).ok())
                    .and_then(|s| registry.id(&s));
                if let Some(state) = state {
                    blocks.push(([int(b, "X"), int(b, "Y"), int(b, "Z")], state));
                }
            }
        }
        Contraption {
            blocks,
            sails: int(tag, "Sails").max(0) as usize,
            angle: float("Angle"),
            speed: float("Speed"),
        }
    }
    fn tag(&self, registry: &horizon_world::registry::BlockRegistry) -> Tag {
        let blocks = self
            .blocks
            .iter()
            .filter_map(|(p, s)| {
                let state = registry.state(*s)?.to_nbt();
                Some(Tag::compound([
                    ("X", Tag::Int(p[0])),
                    ("Y", Tag::Int(p[1])),
                    ("Z", Tag::Int(p[2])),
                    ("State", state),
                ]))
            })
            .collect();
        Tag::compound([
            ("Blocks", Tag::List(blocks)),
            ("Sails", Tag::Int(self.sails as i32)),
            ("Angle", Tag::Float(self.angle.to_bits())),
            ("Speed", Tag::Float(self.speed.to_bits())),
        ])
    }
    /// `WindmillBearingBlockEntity.getGeneratedSpeed` (turning clockwise).
    pub(super) fn windmill_rpm(&self) -> f32 {
        (self.sails / SAILS_PER_RPM).clamp(1, 16) as f32
    }
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// A position turned a quarter `turns` times about `axis` (0 x, 1 y, 2 z), the way the window
/// turns the contraption (right-handed about the positive axis).
pub(super) fn turn(p: [i32; 3], axis: usize, turns: i32) -> [i32; 3] {
    let mut p = p;
    for _ in 0..turns.rem_euclid(4) {
        p = match axis {
            0 => [p[0], -p[2], p[1]],
            1 => [p[2], p[1], -p[0]],
            _ => [-p[1], p[0], p[2]],
        };
    }
    p
}

const DIRECTIONS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];

impl Game<'_> {
    fn bearing_facing(&self, pos: [i32; 3]) -> Option<[i32; 3]> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let s = self.pack.assets.registry.state(state)?;
        horizon_client::kinetics::direction(s.properties.get("facing")?)
    }

    fn is_windmill(&self, pos: [i32; 3]) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.res.facts().get(state).name.contains("windmill")
    }

    /// The contraption a bearing at `pos` holds, if it is running.
    pub(super) fn contraption(&self, pos: [i32; 3]) -> Option<Contraption> {
        let tag = super::spill::saved_entity(&self.world, pos)?;
        let c = tag.get("Contraption")?;
        Some(Contraption::read(c, self.pack.assets.registry))
    }

    /// The running windmills among `bearings` and the speed each gives (a source's, outwards).
    pub(super) fn windmill_speeds(
        &self,
        bearings: impl Iterator<Item = [i32; 3]>,
    ) -> HashMap<[i32; 3], f32> {
        bearings
            .filter(|p| self.is_windmill(*p))
            .filter_map(|p| Some((p, self.contraption(p)?.windmill_rpm())))
            .collect()
    }

    /// `MechanicalBearingBlockEntity.tick` for the bearing at `pos` its network turns at `speed`.
    pub(super) fn tick_bearing(&mut self, pos: [i32; 3], speed: f32) {
        let windmill = self.is_windmill(pos);
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        let running = e.contraption.is_some();
        if !windmill {
            if !running && speed != 0.0 {
                self.assemble_bearing(pos);
                return;
            }
            if running && speed == 0.0 {
                self.disassemble_bearing(pos);
                return;
            }
        }
        if !running {
            return;
        }
        let registry = self.pack.assets.registry;
        let Some(mut c) = e
            .contraption
            .as_ref()
            .map(|t| Contraption::read(t, registry))
        else {
            return;
        };
        // getAngularSpeed: a windmill turns by its sails while its network turns at all.
        let rpm = if windmill {
            if speed == 0.0 {
                0.0
            } else {
                speed.signum() * c.windmill_rpm()
            }
        } else {
            speed
        };
        let angular = rpm * DEGREES_PER_RPM;
        let now = self.world.game_time;
        let angle = (c.angle + c.speed * (now - e.since) as f32).rem_euclid(360.0);
        if (angular - c.speed).abs() > 1e-4 {
            // Its speed changed: the angle it is at, kept with the new speed.
            c.angle = angle;
            c.speed = angular;
            e.since = now;
            e.contraption = Some(c.tag(registry));
            self.set_machine_entity(pos, &e);
        }
    }

    /// `BearingContraption.assemble` and `removeBlocksFromWorld` for the bearing at `pos`: true
    /// when it assembled.
    pub(super) fn assemble_bearing(&mut self, pos: [i32; 3]) -> bool {
        let Some(facing) = self.bearing_facing(pos) else {
            return false;
        };
        let windmill = self.is_windmill(pos);
        let anchor = add(pos, facing);
        let Some(found) = self.find_contraption(pos, anchor) else {
            return false;
        };
        let (blocks, sails) = found;
        if blocks.is_empty() || (windmill && sails < MIN_SAILS) {
            return false;
        }
        // Brittle ones first, so nothing they hang on goes before them.
        let mut order: Vec<&([i32; 3], BlockStateId)> = blocks.iter().collect();
        order.sort_by_key(|(_, s)| !self.brittle(*s));
        for (rel, state) in order {
            let at = add(anchor, *rel);
            let left = self.fluid_left(*state);
            let p = BlockPos::new(at[0], at[1], at[2]);
            let f = flags::UPDATE_CLIENTS
                | flags::UPDATE_KNOWN_SHAPE
                | flags::UPDATE_SUPPRESS_DROPS
                | flags::UPDATE_MOVE_BY_PISTON;
            self.with_level(|b, level, ticks| b.set_block(level, ticks, p, left, f));
        }
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return false;
        };
        let c = Contraption {
            blocks,
            sails,
            angle: 0.0,
            speed: 0.0,
        };
        e.contraption = Some(c.tag(self.pack.assets.registry));
        e.since = self.world.game_time;
        self.set_machine_entity(pos, &e);
        self.machine_sound(pos, "create:contraption_assemble", 1.0, 1.0);
        true
    }

    /// `ControlledContraptionEntity.disassemble`: the blocks back where they are now (a
    /// windmill's where they were), turned to the nearest quarter, each one's state turned with
    /// it; where something is in the way, it drops as its item.
    pub(super) fn disassemble_bearing(&mut self, pos: [i32; 3]) {
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        let registry = self.pack.assets.registry;
        let Some(c) = e
            .contraption
            .take()
            .map(|t| Contraption::read(&t, registry))
        else {
            return;
        };
        let facing = self.bearing_facing(pos).unwrap_or([0, 1, 0]);
        let axis = facing.iter().position(|v| *v != 0).unwrap_or(1);
        let anchor = add(pos, facing);
        let turns = if self.is_windmill(pos) {
            0
        } else {
            let now = self.world.game_time;
            let angle = (c.angle + c.speed * (now - e.since) as f32).rem_euclid(360.0);
            (angle / 90.0).round() as i32
        };
        for (rel, state) in &c.blocks {
            let at = add(anchor, turn(*rel, axis, turns));
            let state = self.turn_state(*state, axis, turns);
            let here = self.world.block(at[0], at[1], at[2]);
            let p = BlockPos::new(at[0], at[1], at[2]);
            if self.res.facts().get(here).replaceable {
                let f = flags::UPDATE_ALL | flags::UPDATE_MOVE_BY_PISTON;
                self.with_level(|b, level, ticks| b.set_block(level, ticks, p, state, f));
            } else {
                let drops = self.res.items().block_drops(
                    registry,
                    state,
                    Default::default(),
                    &mut self.world.random,
                    None,
                );
                for stack in drops {
                    self.spawn_item(stack, at.map(|v| f64::from(v) + 0.5), [0.0; 3], 10);
                }
            }
        }
        let _ = self.process_level_events();
        e.since = self.world.game_time;
        self.set_machine_entity(pos, &e);
        self.machine_sound(pos, "create:contraption_disassemble", 1.0, 1.0);
    }

    /// `BearingBlock.useItemOn` with an empty hand: a running bearing comes apart, else it is
    /// to assemble (a mechanical one only while turned). True when it took the click.
    pub(super) fn use_bearing(&mut self, pos: [i32; 3]) -> bool {
        if self.held().is_some() {
            return false;
        }
        if self.contraption(pos).is_some() {
            self.disassemble_bearing(pos);
        } else if self.is_windmill(pos) || self.machines.speeds.get(&pos).is_some_and(|s| *s != 0.0)
        {
            self.assemble_bearing(pos);
        }
        true
    }

    /// `Contraption.searchMovedStructure` from `anchor` for the bearing at `bearing`: the blocks
    /// (from the anchor) and how many are sails, or None when one may not move.
    fn find_contraption(
        &self,
        bearing: [i32; 3],
        anchor: [i32; 3],
    ) -> Option<(Vec<([i32; 3], BlockStateId)>, usize)> {
        let mut blocks = Vec::new();
        let mut sails = 0;
        let mut visited = HashSet::new();
        let mut frontier = VecDeque::from([anchor]);
        while let Some(p) = frontier.pop_front() {
            if !visited.insert(p) || p == bearing {
                continue;
            }
            let state = self.world.block(p[0], p[1], p[2]);
            if !self.movement_necessary(state) {
                continue;
            }
            let facts = self.res.facts().get(state);
            // Unbreakable ones (and, here, block entities of vanilla's) stop it.
            if facts.hardness < 0.0 || facts.block_entity {
                return None;
            }
            let name = facts.name.clone();
            if self
                .res
                .items()
                .block_in_tag("create:windmill_sails", &name)
                || self.res.items().block_in_tag("minecraft:wool", &name)
            {
                sails += 1;
            }
            blocks.push((
                [p[0] - anchor[0], p[1] - anchor[1], p[2] - anchor[2]],
                state,
            ));
            if blocks.len() > MAX_BLOCKS {
                return None;
            }
            for d in DIRECTIONS {
                let q = add(p, d);
                if q == bearing || visited.contains(&q) {
                    continue;
                }
                let other = self.world.block(q[0], q[1], q[2]);
                let toward = d.map(|v| -v);
                if self.attached_towards(other, toward) || self.sticks(state, other) {
                    frontier.push_back(q);
                }
            }
        }
        Some((blocks, sails))
    }

    /// `isMovementNecessary`: no air, nothing replaceable (grass, water).
    fn movement_necessary(&self, state: BlockStateId) -> bool {
        let facts = self.res.facts().get(state);
        state != self.world.air && (!facts.replaceable || self.brittle(state))
    }

    /// `BlockMovementChecks.isBrittle`: what hangs on or stands on another block.
    fn brittle(&self, state: BlockStateId) -> bool {
        let name = &*self.res.facts().get(state).name;
        let n = name.strip_prefix("minecraft:").unwrap_or(name);
        n.ends_with("torch")
            || n.ends_with("sign")
            || n.ends_with("pressure_plate")
            || n.ends_with("carpet")
            || n.ends_with("button")
            || n.ends_with("rail")
            || n.ends_with("_door")
            || n.ends_with("lantern")
            || matches!(
                n,
                "ladder"
                    | "lever"
                    | "redstone_wire"
                    | "repeater"
                    | "comparator"
                    | "flower_pot"
                    | "bell"
            )
    }

    /// `isBlockAttachedTowardsFallback`: whether `state` holds on to the block `toward` of it.
    fn attached_towards(&self, state: BlockStateId, toward: [i32; 3]) -> bool {
        let registry = self.pack.assets.registry;
        let Some(s) = registry.state(state) else {
            return false;
        };
        let name = s.name.strip_prefix("minecraft:").unwrap_or(&s.name);
        let prop = |k: &str| s.properties.get(k).map(String::as_str);
        let facing = prop("facing").and_then(horizon_client::kinetics::direction);
        let down = toward == [0, -1, 0];
        // A sail: anything beside it in its plane.
        if name.ends_with("_sail") || name.ends_with("sail_frame") {
            let axis = facing.map_or(1, |f| f.iter().position(|v| *v != 0).unwrap_or(1));
            return toward[axis] == 0;
        }
        if name == "ladder" || name.ends_with("wall_torch") || name.ends_with("wall_sign") {
            return facing.map(|f| f.map(|v| -v)) == Some(toward);
        }
        if name.ends_with("_door") {
            return down || (prop("half") == Some("lower") && toward == [0, 1, 0]);
        }
        if name.ends_with("button") || name == "lever" {
            return match prop("face") {
                Some("ceiling") => toward == [0, 1, 0],
                Some("floor") => down,
                _ => facing.map(|f| f.map(|v| -v)) == Some(toward),
            };
        }
        if let Some(hanging) = prop("hanging") {
            return toward
                == if hanging == "true" {
                    [0, 1, 0]
                } else {
                    [0, -1, 0]
                };
        }
        if name.ends_with("torch")
            || name.ends_with("sign")
            || name.ends_with("pressure_plate")
            || name.ends_with("carpet")
            || name.ends_with("rail")
            || matches!(
                name,
                "redstone_wire" | "repeater" | "comparator" | "flower_pot"
            )
        {
            return down;
        }
        false
    }

    /// `canStickTo` both ways: slime or honey holds what is beside it (not each other), unless
    /// one is brittle.
    fn sticks(&self, a: BlockStateId, b: BlockStateId) -> bool {
        let facts = self.res.facts();
        let (na, nb) = (&*facts.get(a).name, &*facts.get(b).name);
        let sticky = |n: &str| n == "minecraft:slime_block" || n == "minecraft:honey_block";
        if !(sticky(na) || sticky(nb)) || self.brittle(a) || self.brittle(b) {
            return false;
        }
        b != self.world.air
            && !(sticky(na) && sticky(nb) && na != nb)
            && !self.res.facts().get(b).replaceable
    }

    /// A state turned a quarter `turns` times about `axis`: its `facing`, `horizontal_facing`
    /// and `axis` with it (the same state where it has no such turned value).
    fn turn_state(&self, state: BlockStateId, axis: usize, turns: i32) -> BlockStateId {
        let registry = self.pack.assets.registry;
        if turns.rem_euclid(4) == 0 {
            return state;
        }
        let Some(mut s) = registry.state(state).cloned() else {
            return state;
        };
        let name_of = |d: [i32; 3]| match d {
            [0, 1, 0] => "up",
            [0, -1, 0] => "down",
            [0, 0, -1] => "north",
            [0, 0, 1] => "south",
            [-1, 0, 0] => "west",
            _ => "east",
        };
        for key in ["facing", "horizontal_facing"] {
            if let Some(d) = s
                .properties
                .get(key)
                .and_then(|f| horizon_client::kinetics::direction(f))
            {
                s.properties
                    .insert(key.into(), name_of(turn(d, axis, turns)).into());
            }
        }
        if let Some(a) = s.properties.get("axis").cloned() {
            let unit = match a.as_str() {
                "x" => [1, 0, 0],
                "y" => [0, 1, 0],
                _ => [0, 0, 1],
            };
            let t = turn(unit, axis, turns);
            let name = ["x", "y", "z"][t.iter().position(|v| *v != 0).unwrap_or(1)];
            s.properties.insert("axis".into(), name.into());
        }
        registry.id(&s).unwrap_or(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_turn_goes_round_the_axis() {
        // About y: east goes north... (right-handed: x to -z).
        assert_eq!(turn([1, 0, 0], 1, 1), [0, 0, -1]);
        assert_eq!(turn([1, 0, 0], 1, 4), [1, 0, 0]);
        assert_eq!(turn([0, 1, 0], 2, 1), [-1, 0, 0]);
        assert_eq!(turn([0, 1, 0], 0, 1), [0, 0, 1]);
        assert_eq!(turn([0, 1, 0], 0, -1), [0, 0, -1]);
    }

    #[test]
    fn a_windmill_turns_by_its_sails() {
        let c = |sails| Contraption {
            sails,
            ..Contraption::default()
        };
        assert_eq!(c(8).windmill_rpm(), 1.0);
        assert_eq!(c(30).windmill_rpm(), 3.0);
        assert_eq!(c(500).windmill_rpm(), 16.0);
    }
}
