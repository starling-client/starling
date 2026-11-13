//! Create's mechanical arm (Starling; ported from Create 6.0.10, MIT, licenses/create-MIT.txt):
//! ArmBlockEntity, ArmInteractionPoint and its types, ArmInteractionPointHandler and
//! ArmAngleTarget. Holding an arm, a use on a depot, belt, basin, millstone, saw, chute, funnel or
//! crushing wheels marks it to take from, then to put into, then not at all; placing the arm
//! gives it the marked points within 5 blocks. Turned, it takes from its inputs in turn what one
//! of its outputs takes, and moves it there, as fast as it turns; the window draws it reaching
//! (kinetics.rs).
use super::Game;
use super::machine_entities::Kind;
use horizon_client::items::ItemStack;
use horizon_core::geometry::Direction;
use horizon_core::nbt::Tag;

/// `mechanicalArmRange`.
const RANGE: f64 = 5.0;
/// The arm's `Phase`.
pub(super) const SEARCH_INPUTS: i32 = 0;
pub(super) const MOVE_TO_INPUT: i32 = 1;
pub(super) const SEARCH_OUTPUTS: i32 = 2;
pub(super) const MOVE_TO_OUTPUT: i32 = 3;
/// KineticBlockEntity's lazy tick, when an arm looks for somewhere to put what it holds.
const LAZY_TICKS: i64 = 10;

/// What a block is to an arm (`ArmInteractionPointType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PointKind {
    Depot,
    Belt,
    Basin,
    Millstone,
    Saw,
    Chute,
    Funnel,
    Crusher,
}

/// An interaction point: where, and whether the arm puts into it (else takes from it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Point {
    pub(super) pos: [i32; 3],
    pub(super) output: bool,
}

/// An arm's work, as its tag keeps it (`Arm`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ArmState {
    pub(super) points: Vec<Point>,
    pub(super) phase: i32,
    /// The point it goes to and the last one it was at (indices into `points`; -1: none), and
    /// whether it set out from rest rather than from that one.
    pub(super) target: i32,
    pub(super) from: i32,
    pub(super) rest: bool,
    /// `chasedPointProgress` when its entity's `since` was, and how much a tick it gains.
    pub(super) progress: f32,
    pub(super) rate: f32,
    pub(super) last_input: i32,
    pub(super) last_output: i32,
}

impl ArmState {
    pub(super) fn read(tag: Option<&Tag>) -> Self {
        let Some(tag) = tag else {
            return ArmState {
                target: -1,
                from: -1,
                rest: true,
                last_input: -1,
                last_output: -1,
                ..ArmState::default()
            };
        };
        let int = |t: &Tag, k: &str| match t.get(k) {
            Some(Tag::Int(v)) => *v,
            _ => 0,
        };
        let float = |k: &str| match tag.get(k) {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            _ => 0.0,
        };
        let points = match tag.get("Points") {
            Some(Tag::List(list)) => list
                .iter()
                .map(|p| Point {
                    pos: [int(p, "X"), int(p, "Y"), int(p, "Z")],
                    output: int(p, "Out") != 0,
                })
                .collect(),
            _ => Vec::new(),
        };
        ArmState {
            points,
            phase: int(tag, "Phase"),
            target: int(tag, "Target"),
            from: int(tag, "From"),
            rest: int(tag, "Rest") != 0,
            progress: float("Progress"),
            rate: float("Rate"),
            last_input: int(tag, "LastIn"),
            last_output: int(tag, "LastOut"),
        }
    }
    fn tag(&self) -> Tag {
        let points = self
            .points
            .iter()
            .map(|p| {
                Tag::compound([
                    ("X", Tag::Int(p.pos[0])),
                    ("Y", Tag::Int(p.pos[1])),
                    ("Z", Tag::Int(p.pos[2])),
                    ("Out", Tag::Int(i32::from(p.output))),
                ])
            })
            .collect();
        Tag::compound([
            ("Points", Tag::List(points)),
            ("Phase", Tag::Int(self.phase)),
            ("Target", Tag::Int(self.target)),
            ("From", Tag::Int(self.from)),
            ("Rest", Tag::Int(i32::from(self.rest))),
            ("Progress", Tag::Float(self.progress.to_bits())),
            ("Rate", Tag::Float(self.rate.to_bits())),
            ("LastIn", Tag::Int(self.last_input)),
            ("LastOut", Tag::Int(self.last_output)),
        ])
    }
    /// The progress `ticks` after its entity's `since`.
    pub(super) fn progress_after(&self, ticks: f32) -> f32 {
        (self.progress + self.rate * ticks).min(1.0)
    }
}

/// `tickMovementProgress`: what an arm gains a tick at `speed`.
fn rate(speed: f32) -> f32 {
    speed.abs().min(256.0) / 1024.0
}

/// `ArmAngleTarget`: the angles an arm at `arm` reaches `target` with, its claw facing `claw`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Angles {
    pub(super) base: f32,
    pub(super) lower: f32,
    pub(super) upper: f32,
    pub(super) head: f32,
}

/// `ArmAngleTarget.NO_TARGET`: at rest.
pub(super) const REST: Angles = Angles {
    base: 0.0,
    lower: 135.0,
    upper: 45.0,
    head: 0.0,
};

pub(super) fn angles(arm: [i32; 3], target: [f64; 3], claw: [i32; 3], ceiling: bool) -> Angles {
    let origin = [
        f64::from(arm[0]) + 0.5,
        f64::from(arm[1]) + 0.5 + if ceiling { -6.0 / 16.0 } else { 6.0 / 16.0 },
        f64::from(arm[2]) + 0.5,
    ];
    let claw_target = target;
    let target = [0, 1, 2].map(|i| target[i] - f64::from(claw[i]) * 0.5);
    let mut diff = [0, 1, 2].map(|i| target[i] - origin[i]);
    let horizontal = (diff[0] * diff[0] + diff[2] * diff[2]).sqrt() as f32;
    let mut base = (diff[0].atan2(diff[2]) as f32).to_degrees() + 180.0;
    if ceiling {
        diff[1] = -diff[1];
        base = 180.0 - base;
    }
    let alpha_offset = (diff[1] as f32).atan2(horizontal).to_degrees();
    let (a, b) = (14.0f32 / 16.0, 15.0f32 / 16.0);
    let length = ((diff[1] * diff[1]) as f32 + horizontal * horizontal)
        .sqrt()
        .clamp(1.0 / 8.0, a + b);
    let alpha_ratio = (-b * b + a * a + length * length) / (2.0 * a * length);
    let mut alpha = alpha_ratio.acos().to_degrees() + alpha_offset;
    let beta_ratio = (-length * length + a * a + b * b) / (2.0 * b * a);
    let mut beta = beta_ratio.acos().to_degrees();
    if alpha.is_nan() {
        alpha = 0.0;
    }
    if beta.is_nan() {
        beta = 0.0;
    }
    // VecHelper.rotate about x and y, as the arm's head ends up.
    let rot_x = |v: [f32; 3], deg: f32| {
        let (s, c) = deg.to_radians().sin_cos();
        [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
    };
    let rot_y = |v: [f32; 3], deg: f32| {
        let (s, c) = deg.to_radians().sin_cos();
        [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
    };
    let mut head = rot_x([0.0, b, 0.0], beta + 180.0);
    head = rot_x([head[0], head[1] + a, head[2]], alpha - 90.0);
    head = rot_y(head, base);
    if ceiling {
        head = rot_x(head, 180.0);
    }
    let head = [0, 1, 2].map(|i| f64::from(head[i]) + origin[i]);
    let mut head_diff = [0, 1, 2].map(|i| claw_target[i] - head[i]);
    if ceiling {
        head_diff[1] = -head_diff[1];
    }
    let horizontal_head = (head_diff[0] * head_diff[0] + head_diff[2] * head_diff[2]).sqrt() as f32;
    let head_angle =
        alpha + beta + 135.0 - (head_diff[1] as f32).atan2(horizontal_head).to_degrees();
    Angles {
        base,
        lower: alpha,
        upper: beta,
        head: -head_angle,
    }
}

impl Game<'_> {
    /// What the block at `pos` is to an arm, if anything (`canCreatePoint`).
    pub(super) fn arm_point_kind(&self, pos: [i32; 3]) -> Option<PointKind> {
        if self.crusher_at(pos) {
            return Some(PointKind::Crusher);
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let s = self.pack.assets.registry.state(state)?;
        let prop = |k: &str| s.properties.get(k).map(String::as_str);
        Some(match self.machine_kind(pos)? {
            Kind::Depot => PointKind::Depot,
            Kind::Belt => PointKind::Belt,
            Kind::Basin => PointKind::Basin,
            Kind::Millstone => PointKind::Millstone,
            Kind::Saw if prop("facing") == Some("up") => PointKind::Saw,
            Kind::Chute => PointKind::Chute,
            // Not an extracting one, nor a belt funnel pushing.
            Kind::Funnel
                if prop("extracting") != Some("true") && prop("shape") != Some("pushing") =>
            {
                PointKind::Funnel
            }
            _ => return None,
        })
    }

    /// `getInteractionPositionVector` and `getInteractionDirection` of a point: where the
    /// claw goes and the way it faces.
    pub(super) fn arm_point_target(&self, pos: [i32; 3]) -> ([f64; 3], [i32; 3]) {
        let corner = pos.map(f64::from);
        match self.arm_point_kind(pos) {
            Some(PointKind::Depot | PointKind::Saw) => (
                [corner[0] + 0.5, corner[1] + 14.0 / 16.0, corner[2] + 0.5],
                [0, -1, 0],
            ),
            Some(PointKind::Funnel) => {
                let state = self.world.block(pos[0], pos[1], pos[2]);
                let facing = self
                    .pack
                    .assets
                    .registry
                    .state(state)
                    .and_then(|s| s.properties.get("facing"))
                    .and_then(|f| horizon_client::kinetics::direction(f))
                    .unwrap_or([0, 1, 0]);
                let at = [0, 1, 2].map(|i| corner[i] + 0.5 - f64::from(facing[i]) * 0.15);
                (at, facing.map(|v| -v))
            }
            _ => (
                [corner[0] + 0.5, corner[1] + 1.0, corner[2] + 0.5],
                [0, -1, 0],
            ),
        }
    }

    /// `ArmInteractionPointHandler.rightClickingBlocksSelectsThem`: holding an arm, a use on a
    /// block it works with marks it to take from, to put into, then neither. True when it
    /// took the click.
    pub(super) fn use_arm_item(&mut self, pos: [i32; 3]) -> bool {
        if self
            .held()
            .is_none_or(|h| &*h.item != "create:mechanical_arm")
            || self.arm_point_kind(pos).is_none()
        {
            return false;
        }
        let selection = &mut self.machines.arm_selection;
        let (key, fallback) = match selection.iter().position(|p| p.pos == pos) {
            None => {
                selection.push(Point { pos, output: false });
                ("create.mechanical_arm.selection_mode.take", "Take")
            }
            Some(i) if !selection[i].output => {
                selection[i].output = true;
                ("create.mechanical_arm.selection_mode.deposit", "Deposit")
            }
            Some(i) => {
                selection.remove(i);
                ("create.mechanical_arm.deselected", "Deselected")
            }
        };
        self.message = Some((
            horizon_client::lang::get_or(key, fallback),
            std::time::Instant::now(),
        ));
        true
    }

    /// `ArmInteractionPointHandler.flushSettings`: an arm just placed at `pos` takes the marked
    /// points within its reach.
    pub(super) fn arm_placed(&mut self, pos: [i32; 3]) {
        if self.machine_kind(pos) != Some(Kind::Arm) {
            return;
        }
        let selection = std::mem::take(&mut self.machines.arm_selection);
        let center = pos.map(|v| f64::from(v) + 0.5);
        let points: Vec<Point> = selection
            .into_iter()
            .filter(|p| {
                let d: [f64; 3] = std::array::from_fn(|i| f64::from(p.pos[i]) + 0.5 - center[i]);
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < RANGE
            })
            .collect();
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        let mut a = ArmState::read(None);
        let (inputs, outputs) = (
            points.iter().filter(|p| !p.output).count(),
            points.iter().filter(|p| p.output).count(),
        );
        a.points = points;
        e.arm = Some(a.tag());
        e.since = self.world.game_time;
        self.set_machine_entity(pos, &e);
        let text = format!("{inputs} → {outputs}");
        self.message = Some((text, std::time::Instant::now()));
    }

    /// What the point at `pos` gives an arm now, if anything (`extract`, simulated): a depot's
    /// held stack then its outputs, a basin's or millstone's outputs, the item at a belt
    /// segment's middle, a chute's item.
    fn arm_peek(&self, pos: [i32; 3]) -> Option<ItemStack> {
        match self.arm_point_kind(pos)? {
            PointKind::Depot => {
                let e = super::machine_entities::read(&self.world, pos, Kind::Depot);
                e.items.into_iter().flatten().next()
            }
            PointKind::Basin => {
                let e = super::machine_entities::read(&self.world, pos, Kind::Basin);
                e.items[super::basins::OUTPUTS]
                    .iter()
                    .flatten()
                    .next()
                    .cloned()
            }
            PointKind::Millstone => {
                let e = super::machine_entities::read(&self.world, pos, Kind::Millstone);
                e.items[1..].iter().flatten().next().cloned()
            }
            PointKind::Chute => {
                let e = super::machine_entities::read(&self.world, pos, Kind::Chute);
                e.items[0].clone()
            }
            PointKind::Belt => {
                let (chain, segment) = self.belt_at(pos)?;
                let items = self
                    .machines
                    .belts
                    .items
                    .get(&chain.start())
                    .cloned()
                    .unwrap_or_else(|| self.read_belt(chain.start()));
                let middle = segment as f32 + 0.5;
                items
                    .into_iter()
                    .find(|i| (i.position - middle).abs() <= 0.5)
                    .map(|i| i.stack)
            }
            _ => None,
        }
    }

    /// Whether the point at `pos` would take some of `stack` (`insert`, simulated).
    fn arm_accepts(&self, pos: [i32; 3], stack: &ItemStack) -> bool {
        let Some(kind) = self.arm_point_kind(pos) else {
            return false;
        };
        match kind {
            PointKind::Depot => {
                let e = super::machine_entities::read(&self.world, pos, Kind::Depot);
                e.items[0].is_none()
            }
            PointKind::Saw | PointKind::Chute => {
                let k = if kind == PointKind::Saw {
                    Kind::Saw
                } else {
                    Kind::Chute
                };
                let e = super::machine_entities::read(&self.world, pos, k);
                e.stacks().next().is_none()
            }
            PointKind::Basin => {
                let e = super::machine_entities::read(&self.world, pos, Kind::Basin);
                e.items[0..9].iter().any(|s| {
                    s.as_ref()
                        .is_none_or(|s| s.same_item_same_components(stack) && s.count < 64)
                })
            }
            PointKind::Millstone => {
                let e = super::machine_entities::read(&self.world, pos, Kind::Millstone);
                self.res
                    .create_recipes()
                    .find("milling", &stack.item)
                    .is_some()
                    && e.items[0]
                        .as_ref()
                        .is_none_or(|s| s.same_item_same_components(stack) && s.count < 64)
            }
            PointKind::Belt => {
                let Some((chain, segment)) = self.belt_at(pos) else {
                    return false;
                };
                let items = self
                    .machines
                    .belts
                    .items
                    .get(&chain.start())
                    .cloned()
                    .unwrap_or_else(|| self.read_belt(chain.start()));
                let middle = segment as f32 + 0.5;
                items.iter().all(|i| (i.position - middle).abs() >= 1.0)
            }
            PointKind::Funnel => true,
            PointKind::Crusher => true,
        }
    }

    /// `extract` for real: what the point at `pos` gives, up to a stack of one item.
    fn arm_take(&mut self, pos: [i32; 3]) -> Option<ItemStack> {
        match self.arm_point_kind(pos)? {
            PointKind::Belt => {
                let (chain, segment) = self.belt_at(pos).map(|(c, s)| (c.clone(), s))?;
                let start = chain.start();
                let mut items = match self.machines.belts.items.remove(&start) {
                    Some(items) => items,
                    None => self.read_belt(start),
                };
                let middle = segment as f32 + 0.5;
                let found = items
                    .iter()
                    .position(|i| (i.position - middle).abs() <= 0.5);
                let taken = found.map(|i| items.remove(i).stack);
                if taken.is_some() {
                    self.write_belt(start, &items);
                }
                self.machines.belts.items.insert(start, items);
                taken
            }
            _ => self.inventory_extract(pos, Direction::Up, 64).flatten(),
        }
    }

    /// `insert` for real: `stack` into the point at `pos`. What it did not take.
    fn arm_put(&mut self, pos: [i32; 3], stack: ItemStack) -> Option<ItemStack> {
        match self.arm_point_kind(pos) {
            Some(PointKind::Belt) => self.belt_insert_from_side(pos, [0, -1, 0], stack),
            Some(PointKind::Crusher) => {
                if self.controller_insert(pos, &stack) {
                    None
                } else {
                    Some(stack)
                }
            }
            Some(PointKind::Funnel) => {
                let mut one = stack.clone();
                one.count = stack.count;
                self.funnel_insert_at(pos, one)
            }
            Some(_) => self
                .inventory_insert(pos, Direction::Up, stack.clone())
                .unwrap_or(Some(stack)),
            None => Some(stack),
        }
    }

    /// `ArmBlockEntity.tick` for the arm at `pos` its network turns at `speed`.
    pub(super) fn tick_arm(&mut self, pos: [i32; 3], speed: f32) {
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        let mut a = ArmState::read(e.arm.as_ref());
        if a.points.is_empty() {
            return;
        }
        let now = self.world.game_time;
        let progress = a.progress_after((now - e.since) as f32);
        // Its speed changed: the progress it had, kept with the new rate.
        if (rate(speed) - a.rate).abs() > 1e-6 {
            a.progress = progress;
            a.rate = rate(speed);
            e.since = now;
            e.arm = Some(a.tag());
            self.set_machine_entity(pos, &e);
            return;
        }
        if progress < 1.0 {
            return;
        }
        let points = a.points.clone();
        let index_of = |output: bool, nth: i32| -> Option<usize> {
            points
                .iter()
                .enumerate()
                .filter(|(_, p)| p.output == output)
                .nth(usize::try_from(nth).ok()?)
                .map(|(i, _)| i)
        };
        let inputs: Vec<usize> = (0..points.len()).filter(|i| !points[*i].output).collect();
        let outputs: Vec<usize> = (0..points.len()).filter(|i| points[*i].output).collect();
        let held = e.items[0].clone();
        // A new move: from the point it was at (its base keeps facing the last one it was at,
        // its arms start at rest when it was at none).
        let moved = |a: &mut ArmState, phase: i32, target: i32| {
            a.rest = a.target < 0;
            if a.target >= 0 {
                a.from = a.target;
            }
            a.phase = phase;
            a.target = target;
            a.progress = 0.0;
        };
        match a.phase {
            MOVE_TO_INPUT => {
                // collectItem.
                let at = usize::try_from(a.target).ok().map(|t| points[t].pos);
                let taken = at.and_then(|p| self.arm_take(p));
                let next = if taken.is_some() {
                    SEARCH_OUTPUTS
                } else {
                    SEARCH_INPUTS
                };
                if taken.is_some() {
                    self.machine_sound(pos, "minecraft:entity.item.pickup", 0.125, 0.6);
                }
                e.items[0] = taken;
                moved(&mut a, next, -1);
            }
            MOVE_TO_OUTPUT => {
                // depositItem.
                let at = usize::try_from(a.target).ok().map(|t| points[t].pos);
                let rest = match (at, held) {
                    (Some(p), Some(stack)) => self.arm_put(p, stack),
                    (_, held) => held,
                };
                let next = if rest.is_some() {
                    SEARCH_OUTPUTS
                } else {
                    SEARCH_INPUTS
                };
                e.items[0] = rest;
                moved(&mut a, next, -1);
            }
            SEARCH_INPUTS => {
                // searchForItem, round robin from the last input on.
                let start = usize::try_from(a.last_input + 1).unwrap_or(0);
                let found = (start..inputs.len()).find(|n| {
                    self.arm_peek(points[inputs[*n]].pos).is_some_and(|stack| {
                        outputs
                            .iter()
                            .any(|o| self.arm_accepts(points[*o].pos, &stack))
                    })
                });
                match found {
                    Some(n) => {
                        a.last_input = n as i32;
                        let target = index_of(false, n as i32).map_or(-1, |i| i as i32);
                        moved(&mut a, MOVE_TO_INPUT, target);
                    }
                    None => {
                        a.last_input = -1;
                        return;
                    }
                }
                if a.last_input == inputs.len() as i32 - 1 {
                    a.last_input = -1;
                }
            }
            _ => {
                // searchForDestination, on its lazy tick.
                if now % LAZY_TICKS != 0 {
                    return;
                }
                let Some(stack) = held else {
                    moved(&mut a, SEARCH_INPUTS, -1);
                    e.since = now;
                    e.arm = Some(a.tag());
                    self.set_machine_entity(pos, &e);
                    return;
                };
                let start = usize::try_from(a.last_output + 1).unwrap_or(0);
                let found = (start..outputs.len())
                    .find(|n| self.arm_accepts(points[outputs[*n]].pos, &stack));
                match found {
                    Some(n) => {
                        a.last_output = if n == outputs.len() - 1 { -1 } else { n as i32 };
                        let target = index_of(true, n as i32).map_or(-1, |i| i as i32);
                        moved(&mut a, MOVE_TO_OUTPUT, target);
                    }
                    None => {
                        a.last_output = -1;
                        return;
                    }
                }
            }
        }
        e.since = now;
        e.arm = Some(a.tag());
        self.set_machine_entity(pos, &e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arm_reaches_a_depot_beside_it() {
        // A depot a block east: the base turns towards it, the arms fold a little.
        let a = angles([0, 0, 0], [1.5, 14.0 / 16.0, 0.5], [0, -1, 0], false);
        assert!(
            (a.base - 270.0).abs() < 1e-3 || (a.base + 90.0).abs() < 1e-3,
            "{a:?}"
        );
        assert!(a.lower > 0.0 && a.lower < 180.0, "{a:?}");
        assert!(a.upper > 0.0 && a.upper < 180.0, "{a:?}");
        assert!(rate(16.0) == 1.0 / 64.0 && rate(-512.0) == 0.25);
    }
}
