//! Create's machines that hold items, where this game simulates the world (Starling; ported from
//! Create 6.0.10, MIT, licenses/create-MIT.txt): the depot (DepotBehaviour,
//! SharedDepotBlockMethods), the millstone (MillstoneBlockEntity, MillstoneBlock) and the
//! mechanical press (PressingBehaviour, MechanicalPressBlockEntity, BeltPressingCallbacks) on a
//! depot's item or on items lying under it. What each holds and how far its work is is its block
//! entity (machine_entities.rs); its speed its network's (kinetic_machines.rs).
use super::machine_entities::{Kind, MachineEntity};
use super::{Game, Hand};
use horizon_client::items::ItemStack;
use horizon_core::random::RandomSource;
use horizon_world::level::LevelEvent;

/// `PressingBehaviour.CYCLE`: a press's run, in its ticks; it presses half way.
pub(super) const CYCLE: i32 = 240;
/// `PressingBehaviour.ENTITY_SCAN`: ticks between looks for items to press.
const ENTITY_SCAN: i64 = 10;
/// A press's `Running`: `PressingBehaviour.Mode` plus one (0: not pressing).
pub(super) const PRESS_WORLD: i32 = 1;
/// A depot's item, which Create presses as a belt's.
pub(super) const PRESS_BELT: i32 = 2;
/// A basin's items, compacted.
pub(super) const PRESS_BASIN: i32 = 3;
/// A depot's held stack and its outputs (DepotBehaviour.processingOutputBuffer).
const DEPOT_HELD: usize = 0;
const DEPOT_OUTPUTS: std::ops::Range<usize> = 1..9;
/// A millstone's input and outputs (MillstoneBlockEntity.inputInv, outputInv).
const MILL_INPUT: usize = 0;
const MILL_OUTPUTS: std::ops::Range<usize> = 1..10;
/// `CASING_13PX`: the depot's top, where items lie.
pub(super) const DEPOT_TOP: f64 = 13.0 / 16.0;
/// `MILLSTONE`: its top.
const MILLSTONE_TOP: f64 = 1.0;

/// `PressingBehaviour.getRunningTickSpeed`: a press's ticks per tick at `speed`.
pub(super) fn press_rate(speed: f32) -> i32 {
    if speed == 0.0 {
        return 0;
    }
    let t = (speed.abs() / 512.0).clamp(0.0, 1.0);
    (1.0 + (60.0 - 1.0) * t) as i32
}

/// `PressingBehaviour.getRenderedHeadOffset` for `ticks` of a run: how far down its head is
/// (0 up, 1 down).
pub(super) fn press_head(ticks: f32) -> f32 {
    let ticks = ticks.abs();
    if ticks < (CYCLE * 2 / 3) as f32 {
        return (ticks / CYCLE as f32 * 2.0).powi(3).clamp(0.0, 1.0);
    }
    ((CYCLE as f32 - ticks) / CYCLE as f32 * 3.0).clamp(0.0, 1.0)
}

/// `ItemHandlerHelper.insertItemStacked`: `stack` into `slots`, onto stacks of its item first,
/// then into empty ones. What did not fit.
pub(super) fn insert_stacked(
    slots: &mut [Option<ItemStack>],
    mut stack: ItemStack,
    max: u32,
) -> Option<ItemStack> {
    for slot in slots.iter_mut().flatten() {
        if slot.same_item_same_components(&stack) && slot.count < max {
            let moved = (max - slot.count).min(stack.count);
            slot.count += moved;
            stack.count -= moved;
            if stack.count == 0 {
                return None;
            }
        }
    }
    for slot in slots.iter_mut() {
        if slot.is_none() {
            let moved = max.min(stack.count);
            *slot = Some(ItemStack {
                count: moved,
                ..stack.clone()
            });
            stack.count -= moved;
            if stack.count == 0 {
                return None;
            }
        }
    }
    Some(stack)
}

impl Game<'_> {
    pub(super) fn machine_sound(&mut self, pos: [i32; 3], sound: &str, volume: f32, pitch: f32) {
        self.level_events.push(LevelEvent::Sound {
            pos: horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            sound: sound.into(),
            volume,
            pitch,
        });
        let _ = self.process_level_events();
    }

    /// Once a tick: the millstones and presses turning, and items falling onto depots and
    /// millstones. `speed` is a part's network speed (none: still).
    pub(super) fn tick_processing(&mut self, machines: &[([i32; 3], Kind, f32)]) {
        for &(pos, kind, speed) in machines {
            match kind {
                Kind::Depot => self.catch_on_top(pos, kind, DEPOT_TOP),
                Kind::Millstone => {
                    self.catch_on_top(pos, kind, MILLSTONE_TOP);
                    if speed != 0.0 {
                        self.tick_millstone(pos, speed);
                    }
                }
                Kind::Press => self.tick_press(pos, speed),
                Kind::Basin => self.tick_basin(pos),
                Kind::Mixer => self.tick_mixer(pos, speed),
                Kind::Burner => self.tick_burner(pos),
                Kind::Saw => self.tick_saw(pos, speed),
                Kind::Funnel => self.tick_funnel(pos),
                Kind::Chute => self.tick_chute(pos),
                Kind::Bearing => self.tick_bearing(pos, speed),
                Kind::Arm => self.tick_arm(pos, speed),
                Kind::Belt => {}
            }
        }
    }

    /// `SharedDepotBlockMethods.onLanded`, `MillstoneBlock.updateEntityAfterFallOn`,
    /// `SawBlock.updateEntityAfterFallOn`: an item lying on the machine's top goes in, as much
    /// as fits.
    pub(super) fn catch_on_top(&mut self, pos: [i32; 3], kind: Kind, top: f64) {
        let lo = [f64::from(pos[0]), f64::from(pos[2])];
        let y = f64::from(pos[1]) + top;
        let found = self.dropped.iter().position(|d| {
            d.position.x >= lo[0]
                && d.position.x < lo[0] + 1.0
                && d.position.z >= lo[1]
                && d.position.z < lo[1] + 1.0
                && (d.position.y - y).abs() < 0.2
                && d.on_ground
        });
        let Some(i) = found else {
            return;
        };
        let stack = self.dropped[i].stack.clone();
        let rest = match kind {
            Kind::Depot => self.depot_insert(pos, stack.clone()),
            Kind::Millstone => self.millstone_insert(pos, stack.clone()),
            Kind::Saw => self.saw_insert(pos, stack.clone()),
            Kind::Chute => self.chute_insert(pos, stack.clone()),
            _ => Some(stack.clone()),
        };
        match rest {
            None => {
                self.dropped.remove(i);
            }
            Some(rest) if rest.count != stack.count => {
                // A new entity for what is left: the old one's count goes to the players so.
                let d = self.dropped.remove(i);
                let at = [d.position.x, d.position.y, d.position.z];
                self.spawn_item(rest, at, [0.0; 3], 0);
            }
            Some(_) => {}
        }
    }

    // ---- depot ---------------------------------------------------------

    /// `DepotBehaviour.insert` (no merging: a depot holds one stack, whatever is left over
    /// comes back), refused while it holds one or its outputs wait to be taken (`isOccupied`:
    /// a belt queues behind it). What did not go in.
    pub(super) fn depot_insert(&mut self, pos: [i32; 3], stack: ItemStack) -> Option<ItemStack> {
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return Some(stack);
        };
        if e.items[DEPOT_HELD].is_some() || e.items[DEPOT_OUTPUTS].iter().any(Option::is_some) {
            return Some(stack);
        }
        let max = self.max_stack(&stack);
        let mut rest = None;
        let mut held = stack;
        if held.count > max {
            rest = Some(ItemStack {
                count: held.count - max,
                ..held.clone()
            });
            held.count = max;
        }
        e.items[DEPOT_HELD] = Some(held);
        self.set_machine_entity(pos, &e);
        rest
    }

    /// `SharedDepotBlockMethods.onUse` on the depot's top: the player takes what it holds and
    /// its outputs, and puts down what they hold (`AllSoundEvents.DEPOT_SLIDE`).
    fn use_depot(&mut self, pos: [i32; 3], face: [i32; 3]) -> bool {
        if face != [0, 1, 0] {
            return false;
        }
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return false;
        };
        let held = self.held().cloned();
        let center = pos.map(|v| f64::from(v) + 0.5);
        if let Some(stack) = e.items[DEPOT_HELD].take() {
            self.give_or_drop(stack);
            let pitch = 1.0 + self.world.random.next_float();
            self.audio
                .play("minecraft:entity.item.pickup", Some(center), 0.2, pitch);
        }
        for slot in DEPOT_OUTPUTS {
            if let Some(stack) = e.items[slot].take() {
                self.give_or_drop(stack);
            }
        }
        if let Some(stack) = held {
            e.items[DEPOT_HELD] = Some(stack);
            *self.held_mut() = None;
            self.machine_sound(pos, "create:depot_slide", 1.0, 1.0);
        }
        self.set_machine_entity(pos, &e);
        true
    }

    // ---- millstone -----------------------------------------------------

    /// `MillstoneInventoryHandler.insertItem` into the input: only what it can mill.
    pub(super) fn millstone_insert(
        &mut self,
        pos: [i32; 3],
        stack: ItemStack,
    ) -> Option<ItemStack> {
        if self
            .res
            .create_recipes()
            .find("milling", &stack.item)
            .is_none()
        {
            return Some(stack);
        }
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return Some(stack);
        };
        let max = self.max_stack(&stack);
        let rest = match &mut e.items[MILL_INPUT] {
            None => {
                let moved = stack.count.min(max);
                e.items[MILL_INPUT] = Some(ItemStack {
                    count: moved,
                    ..stack.clone()
                });
                (stack.count > moved).then(|| ItemStack {
                    count: stack.count - moved,
                    ..stack.clone()
                })
            }
            Some(input) if input.same_item_same_components(&stack) && input.count < max => {
                let moved = (max - input.count).min(stack.count);
                input.count += moved;
                (stack.count > moved).then(|| ItemStack {
                    count: stack.count - moved,
                    ..stack.clone()
                })
            }
            Some(_) => return Some(stack),
        };
        self.set_machine_entity(pos, &e);
        rest
    }

    /// `MillstoneBlockEntity.tick`: the timer runs down by the speed over 16, and an item is
    /// milled when it ends (none while an output slot is full).
    fn tick_millstone(&mut self, pos: [i32; 3], speed: f32) {
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        let full = e.items[MILL_OUTPUTS]
            .iter()
            .flatten()
            .any(|i| i.count >= 64);
        if full {
            return;
        }
        if e.timer > 0 {
            e.timer -= ((speed.abs() / 16.0) as i32).clamp(1, 512);
            if e.timer <= 0 {
                self.mill(&mut e);
                self.set_machine_entity(pos, &e);
            } else {
                self.keep_machine_timer(pos, &e);
            }
            return;
        }
        let Some(input) = &e.items[MILL_INPUT] else {
            return;
        };
        let time = self
            .res
            .create_recipes()
            .find("milling", &input.item)
            .map_or(100.0, |r| r.time);
        e.timer = time as i32;
        e.since = self.world.game_time;
        self.set_machine_entity(pos, &e);
    }

    /// `MillstoneBlockEntity.process`: one input item into its results.
    fn mill(&mut self, e: &mut MachineEntity) {
        let Some(input) = e.items[MILL_INPUT].as_mut() else {
            return;
        };
        let res = std::sync::Arc::clone(&self.res);
        let Some(recipe) = res.create_recipes().find("milling", &input.item) else {
            return;
        };
        input.count -= 1;
        if input.count == 0 {
            e.items[MILL_INPUT] = None;
        }
        for stack in recipe.roll(&mut self.world.random) {
            // Whatever does not fit is lost, as insertItemStacked leaves it.
            let _ = insert_stacked(&mut e.items[MILL_OUTPUTS], stack, 64);
        }
    }

    /// The timer of a machine that changes every tick, kept beside its tag without telling the
    /// players (Create sends its data only when the work starts and ends; the window draws the
    /// run from when it started). A save keeps the tag's: the work starts over from there.
    fn keep_machine_timer(&mut self, pos: [i32; 3], e: &MachineEntity) {
        self.machines.timers.insert(pos, e.timer);
    }

    /// `MillstoneBlock.useItemOn` with an empty hand: the outputs to the player, else the input.
    fn use_millstone(&mut self, pos: [i32; 3]) -> bool {
        if self.held().is_some() {
            return false;
        }
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return false;
        };
        let outputs: Vec<ItemStack> = MILL_OUTPUTS.filter_map(|s| e.items[s].take()).collect();
        if outputs.is_empty() {
            if let Some(input) = e.items[MILL_INPUT].take() {
                self.give_or_drop(input);
            }
        } else {
            for stack in outputs {
                self.give_or_drop(stack);
            }
        }
        self.set_machine_entity(pos, &e);
        true
    }

    // ---- press ---------------------------------------------------------

    /// `PressingBehaviour.tick` for the press at `pos` turning at `speed`.
    fn tick_press(&mut self, pos: [i32; 3], speed: f32) {
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        if e.running == 0 {
            if speed == 0.0
                || (self.world.game_time + i64::from(pos[0] ^ pos[2])) % ENTITY_SCAN != 0
            {
                return;
            }
            let below2 = [pos[0], pos[1] - 2, pos[2]];
            // A depot's item (pressed as a belt's), else items lying under it.
            let mode = if self.machine_kind(below2) == Some(Kind::Depot) {
                let held = super::machine_entities::read(&self.world, below2, Kind::Depot).items
                    [DEPOT_HELD]
                    .clone();
                held.filter(|s| self.press_recipe(&s.item))
                    .map(|_| PRESS_BELT)
            } else if self.machine_kind(below2) == Some(Kind::Basin) {
                // BasinOperatingBlockEntity.updateBasin: a compacting recipe it can make.
                self.press_basin_ready(pos).then_some(PRESS_BASIN)
            } else {
                self.pressable_on_ground(pos).map(|_| PRESS_WORLD)
            };
            if let Some(mode) = mode {
                e.running = mode;
                e.timer = 0;
                e.since = self.world.game_time;
                self.set_machine_entity(pos, &e);
            }
            return;
        }
        // Half way: the press is down.
        let pressed = e.timer == CYCLE / 2 && speed != 0.0;
        if pressed {
            match e.running {
                PRESS_WORLD => self.press_in_world(pos),
                PRESS_BELT => self.press_on_depot([pos[0], pos[1] - 2, pos[2]]),
                PRESS_BASIN => {
                    self.press_compact(pos);
                }
                _ => {}
            }
            let pitch = 0.75 + speed.abs() / 1024.0;
            self.machine_sound(pos, "create:mechanical_press_activation", 0.5, pitch);
        }
        if e.timer > CYCLE {
            // onPressingCompleted: on a basin that can go on, it presses again.
            let again = e.running == PRESS_BASIN && speed != 0.0 && self.press_basin_ready(pos);
            e.running = if again { PRESS_BASIN } else { 0 };
            e.timer = 0;
            e.since = self.world.game_time;
            self.set_machine_entity(pos, &e);
            return;
        }
        let before = e.timer;
        e.timer += press_rate(speed);
        if before < CYCLE / 2 && e.timer >= CYCLE / 2 {
            e.timer = CYCLE / 2;
        }
        if pressed {
            self.set_machine_entity(pos, &e);
        } else {
            self.keep_machine_timer(pos, &e);
        }
    }

    fn press_recipe(&self, item: &str) -> bool {
        self.res.create_recipes().find("pressing", item).is_some()
    }

    /// The first item lying in the block under the press (its box shrunk by an eighth) that
    /// it can press.
    fn pressable_on_ground(&self, pos: [i32; 3]) -> Option<usize> {
        let (lo, hi) = (
            [pos[0], pos[1] - 1, pos[2]].map(|v| f64::from(v) + 0.125),
            [pos[0], pos[1] - 1, pos[2]].map(|v| f64::from(v) + 0.875),
        );
        self.dropped.iter().position(|d| {
            let p = [d.position.x, d.position.y, d.position.z];
            d.on_ground
                && (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i])
                && self.press_recipe(&d.stack.item)
        })
    }

    /// `PressingBehaviour.applyInWorld` and `tryProcessInWorld`: one item of the first stack
    /// under it pressed (not in bulk), the results dropped where it lay.
    fn press_in_world(&mut self, pos: [i32; 3]) {
        let Some(i) = self.pressable_on_ground(pos) else {
            return;
        };
        let res = std::sync::Arc::clone(&self.res);
        let Some(recipe) = res
            .create_recipes()
            .find("pressing", &self.dropped[i].stack.item)
        else {
            return;
        };
        let results = recipe.roll(&mut self.world.random);
        let d = self.dropped.remove(i);
        let at = [d.position.x, d.position.y, d.position.z];
        if d.stack.count > 1 {
            self.spawn_item(
                ItemStack {
                    count: d.stack.count - 1,
                    ..d.stack.clone()
                },
                at,
                [0.0; 3],
                0,
            );
        }
        for stack in results {
            // VecHelper.offsetRandomly(Vec3.ZERO, random, .05).
            let r = &mut self.world.random;
            let motion: [f64; 3] =
                std::array::from_fn(|_| f64::from(r.next_float() * 2.0 - 1.0) * 0.05);
            self.spawn_item(stack, at, motion, 10);
        }
    }

    /// `BeltPressingCallbacks.whenItemHeld` on a depot: one of its held items pressed, the
    /// results into its outputs (what does not fit drops beside it).
    fn press_on_depot(&mut self, depot: [i32; 3]) {
        let Some((_, mut e)) = self.machine_entity(depot) else {
            return;
        };
        let Some(held) = e.items[DEPOT_HELD].as_mut() else {
            return;
        };
        let res = std::sync::Arc::clone(&self.res);
        let Some(recipe) = res.create_recipes().find("pressing", &held.item) else {
            return;
        };
        held.count -= 1;
        if held.count == 0 {
            e.items[DEPOT_HELD] = None;
        }
        let at = [
            f64::from(depot[0]) + 0.5,
            f64::from(depot[1]) + 1.0,
            f64::from(depot[2]) + 0.5,
        ];
        // DepotBehaviour.applyToAllItems: the first result held where the pressed one was, if
        // it was the last, the rest into the outputs.
        for stack in recipe.roll(&mut self.world.random) {
            if e.items[DEPOT_HELD].is_none() {
                e.items[DEPOT_HELD] = Some(stack);
                continue;
            }
            if let Some(rest) = insert_stacked(&mut e.items[DEPOT_OUTPUTS], stack, 64) {
                self.spawn_item(rest, at, [0.0; 3], 10);
            }
        }
        self.set_machine_entity(depot, &e);
    }

    /// A hopper's item into the machine at `at` (its inventory's `insertItem`): a depot's held
    /// stack, a millstone's input, a basin's inputs. None when no machine with an inventory is
    /// there; else what did not go in.
    pub(in crate::game) fn machine_insert(
        &mut self,
        at: [i32; 3],
        stack: ItemStack,
    ) -> Option<Option<ItemStack>> {
        match self.machine_kind(at)? {
            Kind::Depot => Some(self.depot_insert(at, stack)),
            Kind::Millstone => Some(self.millstone_insert(at, stack)),
            Kind::Basin => Some(self.basin_insert(at, stack)),
            Kind::Saw => Some(self.saw_insert(at, stack)),
            Kind::Chute => Some(self.chute_insert(at, stack)),
            _ => None,
        }
    }

    /// One item a hopper under the machine at `at` takes (its inventory's `extractItem`): its
    /// outputs, then a depot's held stack when no press is working on it. None when no machine
    /// with an inventory is there; else the item, if any.
    pub(in crate::game) fn machine_take_one(&mut self, at: [i32; 3]) -> Option<Option<ItemStack>> {
        let (kind, mut e) = self.machine_entity(at)?;
        let slots: Vec<usize> = match kind {
            Kind::Depot => {
                let above = [at[0], at[1] + 2, at[2]];
                let pressing = self.machine_kind(above) == Some(Kind::Press)
                    && self
                        .machine_entity(above)
                        .is_some_and(|(_, p)| p.running != 0);
                let mut s: Vec<usize> = DEPOT_OUTPUTS.collect();
                if !pressing {
                    s.push(DEPOT_HELD);
                }
                s
            }
            Kind::Millstone => MILL_OUTPUTS.collect(),
            Kind::Basin => super::basins::OUTPUTS.collect(),
            // ProcessingInventory.extractItem: nothing comes out of a saw.
            Kind::Saw => Vec::new(),
            _ => return None,
        };
        for slot in slots {
            let Some(stack) = e.items[slot].as_mut() else {
                continue;
            };
            let one = ItemStack {
                count: 1,
                ..stack.clone()
            };
            stack.count -= 1;
            if stack.count == 0 {
                e.items[slot] = None;
            }
            self.set_machine_entity(at, &e);
            return Some(Some(one));
        }
        Some(None)
    }

    /// Whether a use on the machine at `pos` with `held` is the machine's (the window's
    /// prediction of [`Game::use_machine`]).
    pub(super) fn machine_takes(
        &self,
        pos: [i32; 3],
        face: [i32; 3],
        held: Option<&ItemStack>,
    ) -> bool {
        match self.machine_kind(pos) {
            Some(Kind::Depot) => face == [0, 1, 0],
            Some(Kind::Millstone) => held.is_none(),
            // An empty hand or a bucket (BasinBlock.useItemOn).
            Some(Kind::Basin) => held.is_none_or(|s| s.item.ends_with("bucket")),
            Some(Kind::Belt) => held.is_none(),
            Some(Kind::Saw) => self.saw_takes(pos, held),
            Some(Kind::Funnel) => self.funnel_takes(pos, face, held),
            Some(Kind::Bearing) => held.is_none(),
            // Fuel (BlazeBurnerBlock.useItemOn).
            Some(Kind::Burner) => held.is_some_and(|h| {
                self.res
                    .items()
                    .get(&h.item)
                    .is_some_and(|i| i.fuel.is_some())
                    || self.res.basin_recipes().superheated.contains_key(&*h.item)
            }),
            _ => false,
        }
    }

    /// A use on a machine (the main hand's turn): true when it took the click.
    pub(super) fn use_machine(&mut self, pos: [i32; 3], face: [i32; 3]) -> bool {
        if self.hand != Hand::Main {
            return false;
        }
        match self.machine_kind(pos) {
            Some(Kind::Depot) => self.use_depot(pos, face),
            Some(Kind::Millstone) => self.use_millstone(pos),
            Some(Kind::Basin) => self.use_basin(pos),
            Some(Kind::Belt) => self.use_belt(pos),
            Some(Kind::Burner) => self.use_burner(pos),
            Some(Kind::Saw) => self.use_saw(pos),
            Some(Kind::Funnel) => self.use_funnel(pos, face),
            Some(Kind::Bearing) => self.use_bearing(pos),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_comes_down_half_way_through_its_run() {
        assert_eq!(press_rate(0.0), 0);
        assert_eq!(press_rate(16.0), 2);
        assert_eq!(press_rate(256.0), 30);
        assert_eq!(press_rate(-512.0), 60);
        assert_eq!(press_head(0.0), 0.0);
        assert!((press_head(CYCLE as f32 / 2.0) - 1.0).abs() < 1e-6);
        assert_eq!(press_head(CYCLE as f32), 0.0);
    }

    #[test]
    fn stacks_go_onto_their_own_kind_first() {
        let mut slots = vec![None, Some(ItemStack::new("x:a", 60)), None];
        assert_eq!(
            insert_stacked(&mut slots, ItemStack::new("x:a", 10), 64),
            None
        );
        assert_eq!(slots[1].as_ref().map(|s| s.count), Some(64));
        assert_eq!(slots[0].as_ref().map(|s| s.count), Some(6));
        let rest = insert_stacked(&mut slots[..2], ItemStack::new("x:b", 1), 64);
        assert_eq!(rest.map(|s| s.count), Some(1));
    }
}
