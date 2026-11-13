//! Create's mechanical saw where this game simulates the world (Starling; ported from Create
//! 6.0.10, MIT, licenses/create-MIT.txt): SawBlockEntity and SawBlock. Facing up, it cuts the
//! items put on it (from above, a belt or a hopper; one at a time) by its cutting and
//! stonecutting recipes and moves them on the way it turns; facing sideways, it is a block
//! breaker (kinetic_machines.rs) for what it saws, and fells the tree it cut (TreeCutter).
use super::Game;
use super::kinetic_machines::Recipe;
use super::machine_entities::{Kind, MachineEntity};
use horizon_client::items::ItemStack;
use horizon_core::position::BlockPos;
use horizon_core::random::RandomSource;
use horizon_world::registry::BlockStateId;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

/// `ProcessingInventory`'s slots: the input, then what it was cut into.
pub(super) const SLOTS: usize = 32;
const INPUT: usize = 0;
/// `remainingTime` and `recipeDuration` are kept in 24ths of a tick: a tick at speed 24 is one.
pub(super) const SCALE: i32 = 24;
/// A saw's `Running`: idle (`remainingTime` -1), cutting, cut and on its way off
/// (`appliedRecipe`).
pub(super) const IDLE: i32 = 0;
pub(super) const CUTTING: i32 = 1;
pub(super) const CUT: i32 = 2;
/// `CASING_12PX` facing up: its top, where items land.
pub(super) const TOP: f64 = 12.0 / 16.0;

/// `getItemMovementVec`: the way a saw facing up moves its items, by its state's
/// `axis_along_first` and its speed's sign.
pub(super) fn item_movement(along_first: bool, speed: f32) -> [i32; 3] {
    let offset = if speed < 0.0 { -1 } else { 1 };
    if along_first {
        [0, 0, -offset]
    } else {
        [offset, 0, 0]
    }
}

/// `processingSpeed` in 24ths: `clamp(|speed| / 24, 1, 128)`.
fn processing_speed(speed: f32) -> i32 {
    speed.abs().clamp(SCALE as f32, (128 * SCALE) as f32) as i32
}

/// What TreeCutter asks of a block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Wood {
    /// `isLog`: the logs tag, Create's slimy logs, a mushroom stem.
    log: bool,
    /// `isRoot`: Create's roots tag.
    root: bool,
    /// `isLeaf`: its `distance` (no scaffolding's).
    leaf: Option<i32>,
    /// Create's tree attachments tag (vines, cocoa, bee nests...).
    attachment: bool,
    /// `nonDecayingLeafDistance`: how far out a mushroom's or a nether tree's cap goes (-1).
    non_decaying: i32,
    /// `isVerticalPlant`: bamboo, cactus, sugar cane, kelp.
    vertical: bool,
    /// `isChorus`.
    chorus: bool,
}

/// `TreeCutter.Tree`: what falls, in the order it falls.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Tree {
    logs: Vec<[i32; 3]>,
    leaves: Vec<[i32; 3]>,
    attachments: Vec<[i32; 3]>,
}

fn offset(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    [p[0] + d[0], p[1] + d[1], p[2] + d[2]]
}

const DIRECTIONS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];

/// `forNeighbours`: the blocks round `p` from `min_y` to `max_y` above it (BlockPos.betweenClosed's
/// order, x fastest), each not yet `visited` as it comes.
fn neighbours(
    p: [i32; 3],
    min_y: i32,
    max_y: i32,
    visited: &mut HashSet<[i32; 3]>,
    mut each: impl FnMut([i32; 3], &mut HashSet<[i32; 3]>),
) {
    for z in -1..=1 {
        for y in min_y..=max_y {
            for x in -1..=1 {
                let at = offset(p, [x, y, z]);
                if !visited.contains(&at) {
                    each(at, visited);
                }
            }
        }
    }
}

/// `TreeCutter.validateCut`: whether the layer above the cut at `pos` stands on no other log.
fn validate_cut(pos: [i32; 3], at: &dyn Fn([i32; 3]) -> Wood) -> bool {
    let mut visited = HashSet::new();
    let mut frontier = VecDeque::from([pos, offset(pos, [0, 1, 0])]);
    while let Some(current) = frontier.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        let below = offset(current, [0, -1, 0]);
        let lower = current[1] == pos[1];
        let (here, under) = (at(current), at(below));
        if !here.log && !here.root {
            continue;
        }
        if !lower && below != pos && (under.log || under.root) {
            return false;
        }
        for d in DIRECTIONS {
            if d[1] < 0 || (d[1] > 0 && !lower) {
                continue;
            }
            let next = offset(current, d);
            if !visited.contains(&next) {
                frontier.push_back(next);
            }
        }
    }
    true
}

/// `TreeCutter.findTree`: what falls when a saw cuts `broken` at `pos` (now air), `at` saying
/// what each block is, up to `top`.
pub(super) fn find_tree(
    pos: [i32; 3],
    broken: Wood,
    at: &dyn Fn([i32; 3]) -> Wood,
    top: i32,
) -> Tree {
    let mut tree = Tree::default();
    let above = offset(pos, [0, 1, 0]);
    // Bamboo, sugar cane, cactus: the stalk above, from the top down.
    if broken.vertical {
        if !at(above).vertical {
            return tree;
        }
        let mut y = above[1];
        while y < top && at([pos[0], y, pos[2]]).vertical {
            tree.logs.push([pos[0], y, pos[2]]);
            y += 1;
        }
        tree.logs.reverse();
        return tree;
    }
    // Chorus: every chorus block joined to the one above.
    if broken.chorus {
        if !at(above).chorus {
            return tree;
        }
        let mut visited = HashSet::new();
        let mut frontier = VecDeque::from([above]);
        while let Some(current) = frontier.pop_front() {
            if !visited.insert(current) {
                continue;
            }
            tree.logs.push(current);
            for d in DIRECTIONS {
                let next = offset(current, d);
                if !visited.contains(&next) && at(next).chorus {
                    frontier.push_back(next);
                }
            }
        }
        tree.logs.reverse();
        return tree;
    }
    if !validate_cut(pos, at) {
        return tree;
    }
    // The logs and roots, up from the cut.
    let mut visited = HashSet::from([pos]);
    let mut frontier = VecDeque::new();
    for z in -1..=1 {
        for y in 0..=1 {
            for x in -1..=1 {
                frontier.push_back(offset(pos, [x, y, z]));
            }
        }
    }
    let mut roots = false;
    while let Some(current) = frontier.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        let here = at(current);
        if here.root {
            roots = true;
        } else if !here.log {
            continue;
        }
        tree.logs.push(current);
        neighbours(current, 0, 1, &mut visited, |p, _| frontier.push_back(p));
    }
    let mut visited: HashSet<[i32; 3]> = tree.logs.iter().copied().collect();
    let mut frontier: VecDeque<[i32; 3]> = tree.logs.iter().copied().collect();
    // Roots: down from them too.
    if roots {
        let old: HashSet<[i32; 3]> = visited.clone();
        while let Some(current) = frontier.pop_front() {
            if !at(current).root {
                continue;
            }
            if !old.contains(&current) {
                tree.logs.push(current);
            }
            neighbours(current, -1, 0, &mut visited, |p, visited| {
                if visited.insert(p) {
                    frontier.push_back(p);
                }
            });
        }
        visited = tree.logs.iter().copied().collect();
        frontier = tree.logs.iter().copied().collect();
    }
    // The leaves: each further from a log than the one it was found from.
    while let Some(prev) = frontier.pop_front() {
        let prev_distance = at(prev).leaf.unwrap_or(0);
        neighbours(prev, -1, 1, &mut visited, |current, visited| {
            let here = at(current);
            if here.attachment {
                tree.attachments.push(current);
                visited.insert(current);
                return;
            }
            let horizontal = (current[0] - pos[0]).abs().max((current[2] - pos[2]).abs());
            if horizontal <= here.non_decaying && visited.insert(current) {
                tree.leaves.push(current);
                frontier.push_back(current);
                return;
            }
            if here.leaf.is_some_and(|d| d > prev_distance) && visited.insert(current) {
                tree.leaves.push(current);
                frontier.push_back(current);
            }
        });
    }
    tree
}

impl Game<'_> {
    /// What TreeCutter asks of the block `state`.
    fn wood(&self, state: BlockStateId) -> Wood {
        let name = self.res.facts().get(state).name.clone();
        let items = self.res.items();
        let tag = |t: &str| items.block_in_tag(t, &name);
        let leaf = (&*name != "minecraft:scaffolding")
            .then(|| {
                self.pack
                    .assets
                    .registry
                    .state(state)?
                    .properties
                    .get("distance")?
                    .parse()
                    .ok()
            })
            .flatten();
        Wood {
            log: tag("minecraft:logs")
                || tag("create:slimy_logs")
                || &*name == "minecraft:mushroom_stem",
            root: tag("create:roots"),
            leaf,
            attachment: tag("create:tree_attachments"),
            non_decaying: match &*name {
                "minecraft:red_mushroom_block" => 2,
                "minecraft:brown_mushroom_block"
                | "minecraft:weeping_vines"
                | "minecraft:weeping_vines_plant" => 3,
                _ if tag("minecraft:wart_blocks") => 3,
                _ => -1,
            },
            vertical: matches!(
                &*name,
                "minecraft:bamboo"
                    | "minecraft:cactus"
                    | "minecraft:sugar_cane"
                    | "minecraft:kelp"
                    | "minecraft:kelp_plant"
            ),
            chorus: matches!(&*name, "minecraft:chorus_plant" | "minecraft:chorus_flower"),
        }
    }

    /// `SawBlockEntity.isSawable`: wood, leaves and plants a saw cuts (no saplings).
    pub(super) fn sawable(&self, state: BlockStateId) -> bool {
        let name = self.res.facts().get(state).name.clone();
        if self.res.items().block_in_tag("minecraft:saplings", &name) {
            return false;
        }
        let wood = self.wood(state);
        wood.log
            || self.res.items().block_in_tag("minecraft:leaves", &name)
            || wood.root
            || wood.vertical
            || matches!(
                &*name,
                "minecraft:pumpkin" | "minecraft:melon" | "minecraft:chorus_plant"
            )
    }

    /// `SawBlockEntity.onBlockBroken` after the cut: `TreeCutter.findTree` at `breaking` (cut
    /// from `broken`) by the saw at `saw`, and its blocks broken (attachments, logs, leaves),
    /// their drops thrown away from the saw (`dropItemFromCutTree`), or, felled sideways,
    /// falling over first (Starling).
    pub(super) fn fell_tree(
        &mut self,
        saw: [i32; 3],
        breaking: [i32; 3],
        broken: BlockStateId,
    ) -> super::Result<()> {
        let broken = self.wood(broken);
        let top = horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT;
        let tree = {
            let at = |p: [i32; 3]| self.wood(self.world.block(p[0], p[1], p[2]));
            find_tree(breaking, broken, &at, top)
        };
        let back = [
            f64::from(breaking[0] - saw[0]),
            f64::from(breaking[1] - saw[1]),
            f64::from(breaking[2] - saw[2]),
        ];
        let falls = tree
            .attachments
            .iter()
            .map(|p| (*p, 1.0 / 32.0))
            .chain(tree.logs.iter().map(|p| (*p, 1.0 / 2.0)))
            .chain(tree.leaves.iter().map(|p| (*p, 1.0 / 8.0)));
        let falls: Vec<([i32; 3], f32)> = falls.collect();
        // Starling: felled sideways, it falls over first and breaks where it lands
        // (falling_trees.rs).
        let falls_over = back[1] == 0.0;
        let mut fallen = Vec::new();
        for (pos, effect) in falls {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            if state == self.world.air {
                continue;
            }
            // BlockHelper.destroyBlockAs, with no tool and no player.
            let burst = self.world.random.next_float() < effect;
            if burst && !falls_over {
                self.level_event_at(2001, pos, state.0 as i32);
            }
            let registry = self.pack.assets.registry;
            let drops = self.res.items().block_drops(
                registry,
                state,
                Default::default(),
                &mut self.world.random,
                None,
            );
            if falls_over {
                fallen.push(super::falling_trees::Fallen {
                    pos,
                    state,
                    drops,
                    burst,
                });
            } else {
                let d = [0, 1, 2].map(|i| f64::from(pos[i] - breaking[i]));
                let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                let motion = back.map(|v| v * distance / 20.0);
                let center = pos.map(|v| f64::from(v) + 0.5);
                for stack in drops {
                    self.spawn_item(stack, center, motion, 0);
                }
            }
            let left = self.fluid_left(state);
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| b.set_block(level, ticks, at, left, 3));
        }
        if !fallen.is_empty() {
            let toward = [0, 1, 2].map(|i| breaking[i] - saw[i]);
            self.start_falling_tree(saw, breaking, toward, fallen);
        }
        self.process_level_events()?;
        Ok(())
    }

    /// `FluidState.createLegacyBlock`: what a broken block leaves (water where it was
    /// waterlogged).
    pub(super) fn fluid_left(&self, state: BlockStateId) -> BlockStateId {
        if self.res.facts().get(state).waterlogged {
            return self
                .pack
                .assets
                .registry
                .default_state("minecraft:water")
                .unwrap_or(self.world.air);
        }
        self.world.air
    }

    // ---- cutting items -------------------------------------------------

    /// The saw at `pos` faces up (`canProcess`), and its `axis_along_first`.
    fn saw_up(&self, pos: [i32; 3]) -> Option<bool> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let props = &self.pack.assets.registry.state(state)?.properties;
        (props.get("facing").map(String::as_str) == Some("up"))
            .then(|| props.get("axis_along_first").map(String::as_str) == Some("true"))
    }

    /// `getRecipes`: the cutting recipes for `item`, then the stonecutter's.
    fn saw_recipes(&self, item: &str) -> Vec<Arc<Recipe>> {
        let mut out = self.res.create_recipes().find_all("cutting", item).to_vec();
        let stack = ItemStack::new(item, 1);
        for s in &self.res.stations().recipes.stonecutting {
            if s.input.test_in(&self.loop_context, &stack) {
                out.push(Arc::new(Recipe {
                    time: 50.0,
                    results: vec![(s.result.item.clone(), s.result.count, 1.0)],
                }));
            }
        }
        out
    }

    /// `SawBlockEntity.insertItem` and the inventory's `insertItem` (a hopper's, a belt's): one
    /// item in when it is empty and faces up (not bulk cutting), and its work started. What did
    /// not go in.
    pub(super) fn saw_insert(&mut self, pos: [i32; 3], stack: ItemStack) -> Option<ItemStack> {
        self.saw_up(pos)?;
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return Some(stack);
        };
        if e.stacks().next().is_some() {
            return Some(stack);
        }
        e.items = vec![None; SLOTS];
        e.items[INPUT] = Some(ItemStack {
            count: 1,
            ..stack.clone()
        });
        self.saw_start(&mut e);
        self.set_machine_entity(pos, &e);
        (stack.count > 1).then(|| ItemStack {
            count: stack.count - 1,
            ..stack
        })
    }

    /// `SawBlockEntity.start`: the next of its recipes for the input, and how long it takes
    /// (a moment to let what it has none for through).
    fn saw_start(&mut self, e: &mut MachineEntity) {
        let Some(input) = e.items[INPUT].clone() else {
            return;
        };
        let recipes = self.saw_recipes(&input.item);
        let time = if recipes.is_empty() {
            10
        } else {
            e.recipe += 1;
            if e.recipe as usize >= recipes.len() {
                e.recipe = 0;
            }
            let time = recipes[e.recipe as usize].time as i32;
            time * (input.count as i32 / 5).max(1)
        };
        e.timer = time * SCALE;
        e.work = e.timer;
        e.running = CUTTING;
        e.since = self.world.game_time;
    }

    /// `SawBlockEntity.applyRecipe`: the input, item by item, into its recipe's results.
    fn saw_apply(&mut self, e: &mut MachineEntity) {
        let Some(input) = e.items[INPUT].clone() else {
            return;
        };
        let recipes = self.saw_recipes(&input.item);
        if recipes.is_empty() {
            return;
        }
        if e.recipe as usize >= recipes.len() {
            e.recipe = 0;
        }
        let recipe = &recipes[e.recipe as usize];
        e.items = vec![None; SLOTS];
        // ItemHelper.addToList: onto the stacks of the same item first.
        let mut list: Vec<ItemStack> = Vec::new();
        for _ in 0..input.count {
            for mut stack in recipe.roll(&mut self.world.random) {
                let max = self.max_stack(&stack);
                for s in list.iter_mut() {
                    if s.same_item_same_components(&stack) {
                        let moved = max.saturating_sub(s.count).min(stack.count);
                        s.count += moved;
                        stack.count -= moved;
                    }
                }
                if stack.count > 0 {
                    list.push(stack);
                }
            }
        }
        for (slot, stack) in list.into_iter().enumerate() {
            if slot + 1 < SLOTS {
                e.items[slot + 1] = Some(stack);
            }
        }
    }

    /// `SawBlockEntity.tick` for a saw facing up at `pos` turning at `speed`: items landing on
    /// it go in, the input is cut when its time is nearly up, and what it became goes on to
    /// what takes items from a belt the way it moves them, else off it.
    pub(super) fn tick_saw(&mut self, pos: [i32; 3], speed: f32) {
        let Some(along_first) = self.saw_up(pos) else {
            return;
        };
        if speed == 0.0 {
            return;
        }
        self.catch_on_top(pos, Kind::Saw, TOP);
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        if e.running == IDLE {
            if e.items[INPUT].is_some() {
                self.saw_start(&mut e);
                self.set_machine_entity(pos, &e);
            }
            return;
        }
        e.timer -= processing_speed(speed);
        if e.timer < 5 * SCALE && e.running == CUTTING {
            if let Some(input) = e.items[INPUT].clone() {
                let wood = self
                    .res
                    .items()
                    .get(&input.item)
                    .and_then(|i| i.block)
                    .is_some_and(|s| {
                        &*self.res.facts().get(s).sounds.break_sound == "minecraft:block.wood.break"
                    });
                let (sound, volume, pitch) = if wood {
                    ("create:saw_activate_wood", 2.25, 1.5)
                } else {
                    ("create:saw_activate_stone", 0.375, 1.25)
                };
                self.machine_sound(pos, sound, volume, pitch);
            }
            self.saw_apply(&mut e);
            e.running = CUT;
            e.timer = 20 * SCALE;
            e.work = e.timer;
            e.since = self.world.game_time;
            self.set_machine_entity(pos, &e);
            return;
        }
        if e.timer > 0 {
            self.machines.timers.insert(pos, e.timer);
            return;
        }
        e.timer = 0;
        let way = item_movement(along_first, speed);
        let next = offset(pos, way);
        // DirectBeltInputBehaviour there: as much as it takes.
        if self.takes_from_belt(next) {
            let mut changed = false;
            for slot in 0..e.items.len() {
                let Some(stack) = e.items[slot].clone() else {
                    continue;
                };
                let rest = self.belt_insert_into(next, way, stack.clone());
                if rest.as_ref() == Some(&stack) {
                    continue;
                }
                e.items[slot] = rest;
                changed = true;
            }
            if changed {
                self.set_machine_entity(pos, &e);
            } else {
                self.machines.timers.insert(pos, e.timer);
            }
            return;
        }
        // Off it, moving the way it moves them.
        let w = way.map(f64::from);
        let at = [
            f64::from(pos[0]) + 0.5 + w[0] * 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5 + w[2] * 0.5,
        ];
        let motion = [w[0] * 0.0625, 0.125, w[2] * 0.0625];
        for stack in e.items.iter_mut().filter_map(Option::take) {
            self.spawn_item(stack, at, motion, 0);
        }
        e.running = IDLE;
        e.timer = 0;
        e.work = 0;
        e.since = self.world.game_time;
        self.set_machine_entity(pos, &e);
    }

    /// `SawBlock.useItemOn` with an empty hand on a saw facing up: what it holds to the player.
    pub(super) fn use_saw(&mut self, pos: [i32; 3]) -> bool {
        if self.held().is_some() || self.saw_up(pos).is_none() {
            return false;
        }
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return false;
        };
        for stack in e.items.iter_mut().filter_map(Option::take) {
            self.give_or_drop(stack);
        }
        e.running = IDLE;
        e.timer = 0;
        e.work = 0;
        self.set_machine_entity(pos, &e);
        true
    }

    /// Whether a use on the saw at `pos` is the saw's ([`Game::use_saw`]).
    pub(super) fn saw_takes(&self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        held.is_none() && self.saw_up(pos).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn log() -> Wood {
        Wood {
            log: true,
            non_decaying: -1,
            ..Wood::default()
        }
    }
    fn leaf(distance: i32) -> Wood {
        Wood {
            leaf: Some(distance),
            non_decaying: -1,
            ..Wood::default()
        }
    }

    #[test]
    fn a_cut_trunk_takes_its_logs_and_leaves_with_it() {
        // A trunk of four over the cut at y 0, leaves round its top, a vine on one.
        let mut world: HashMap<[i32; 3], Wood> = HashMap::new();
        for y in 1..5 {
            world.insert([0, y, 0], log());
        }
        world.insert([1, 4, 0], leaf(1));
        world.insert([2, 4, 0], leaf(2));
        world.insert([0, 5, 0], leaf(1));
        world.insert(
            [2, 3, 0],
            Wood {
                attachment: true,
                non_decaying: -1,
                ..Wood::default()
            },
        );
        // A leaf no further than the one it touches stays (another tree's).
        world.insert([3, 4, 0], leaf(1));
        let empty = Wood {
            non_decaying: -1,
            ..Wood::default()
        };
        let at = |p: [i32; 3]| world.get(&p).copied().unwrap_or(empty);
        let tree = find_tree([0, 0, 0], log(), &at, 320);
        assert_eq!(tree.logs, vec![[0, 1, 0], [0, 2, 0], [0, 3, 0], [0, 4, 0]]);
        let mut leaves = tree.leaves.clone();
        leaves.sort_unstable();
        assert_eq!(leaves, vec![[0, 5, 0], [1, 4, 0], [2, 4, 0]]);
        assert_eq!(tree.attachments, vec![[2, 3, 0]]);
    }

    #[test]
    fn a_trunk_standing_on_another_log_does_not_fall() {
        // Cut at (0, 0): the log above also stands on the log at (1, 0, 0) beside the cut.
        let mut world: HashMap<[i32; 3], Wood> = HashMap::new();
        world.insert([0, 1, 0], log());
        world.insert([1, 1, 0], log());
        world.insert([1, 0, 0], log());
        let empty = Wood::default();
        let at = |p: [i32; 3]| world.get(&p).copied().unwrap_or(empty);
        assert_eq!(find_tree([0, 0, 0], log(), &at, 320), Tree::default());
    }

    #[test]
    fn a_stalk_falls_from_the_top_down() {
        let cane = Wood {
            vertical: true,
            ..Wood::default()
        };
        let at = |p: [i32; 3]| {
            if p[0] == 0 && p[2] == 0 && (1..4).contains(&p[1]) {
                cane
            } else {
                Wood::default()
            }
        };
        let tree = find_tree([0, 0, 0], cane, &at, 320);
        assert_eq!(tree.logs, vec![[0, 3, 0], [0, 2, 0], [0, 1, 0]]);
    }

    #[test]
    fn a_saw_moves_its_items_by_its_axis_and_speed() {
        assert_eq!(item_movement(false, 16.0), [1, 0, 0]);
        assert_eq!(item_movement(false, -16.0), [-1, 0, 0]);
        assert_eq!(item_movement(true, 16.0), [0, 0, -1]);
        assert_eq!(processing_speed(8.0), SCALE);
        assert_eq!(processing_speed(48.0), 2 * SCALE);
    }
}
