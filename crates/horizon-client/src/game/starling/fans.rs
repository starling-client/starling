//! Create's encased fan, where this game simulates the world (Starling; ported from Create
//! 6.0.10, MIT, licenses/create-MIT.txt): EncasedFanBlockEntity's air current (AirCurrent) pushes
//! the items along it, and what the current passes through processes them (AllFanProcessingTypes,
//! FanProcessing): water washes them by Create's splashing recipes, lava blasts them by vanilla's
//! smelting and blasting ones, fire smokes them by its smoking ones and soul fire haunts them by
//! Create's haunting ones. A depot's item in the current is processed too.
use super::Game;
use horizon_client::blocks::Fluid;
use horizon_client::items::ItemStack;
use std::collections::HashMap;

/// `fanPushDistance`, `fanPullDistance`: the farthest the current reaches.
const FAN_DISTANCE: f32 = 20.0;
/// `fanRotationArgmax`: the speed at which it reaches farthest.
const FAN_ARGMAX: f32 = 256.0;
/// `fanBlockCheckRate`: ticks between looks at what the current passes through.
const CHECK_RATE: i64 = 30;
/// `fanProcessingTime`: ticks an item takes to be processed, per 16 of the stack.
const PROCESSING_TIME: i32 = 150;

/// `FanProcessingType`: what a current does to the items it carries past a catalyst.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Processing {
    Splashing,
    Blasting,
    Smoking,
    Haunting,
}

/// A fan's current, as `AirCurrent.rebuild` lays it out.
pub(super) struct Current {
    /// The fan's facing: the side the air leaves or comes in by.
    facing: [i32; 3],
    /// Air out of the front (true) or into it.
    pushing: bool,
    /// How far it reaches, in blocks from the fan's face.
    reach: f32,
    /// The processing on each block of it, from the fan's front (1) on.
    types: Vec<Option<Processing>>,
    /// When it was laid out (game time).
    built: i64,
}

impl Current {
    /// `AirCurrent.getTypeAt`: the processing `offset` blocks from the fan's face.
    fn type_at(&self, offset: f32) -> Option<Processing> {
        if !(0.0..=self.reach).contains(&offset) {
            return None;
        }
        // A pushing current's segment i covers [i - 1, i); a pulling one's (i - 1, i].
        let block = if self.pushing {
            offset.floor() as usize + 1
        } else {
            (offset.ceil() as usize).max(1)
        };
        self.types
            .get(block.min(self.types.len()).saturating_sub(1))
            .copied()
            .flatten()
    }
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

impl Game<'_> {
    /// The processing a block of the world gives a current (`FanProcessingType.getAt`).
    fn processing_at(&self, pos: [i32; 3]) -> Option<Processing> {
        let facts = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]));
        if facts.fluid == Fluid::Water || facts.waterlogged {
            return Some(Processing::Splashing);
        }
        if facts.fluid == Fluid::Lava {
            return Some(Processing::Blasting);
        }
        let lit = |name: &str| {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            self.pack.assets.registry.state(state).is_none_or(|s| {
                s.name != name || s.properties.get("lit").is_none_or(|v| v == "true")
            })
        };
        match &*facts.name {
            "minecraft:soul_fire" => Some(Processing::Haunting),
            "minecraft:soul_campfire" if lit("minecraft:soul_campfire") => {
                Some(Processing::Haunting)
            }
            "minecraft:fire" => Some(Processing::Smoking),
            "minecraft:campfire" if lit("minecraft:campfire") => Some(Processing::Smoking),
            _ => None,
        }
    }

    /// `AirCurrent.rebuild` for the fan at `pos` facing `facing`, its speed (about its facing,
    /// `convertToDirection`) `speed`.
    fn build_current(&self, pos: [i32; 3], facing: [i32; 3], speed: f32) -> Current {
        let pushing = speed > 0.0;
        let factor = (speed.abs() / FAN_ARGMAX).min(1.0);
        let max = 3.0 + (FAN_DISTANCE - 3.0) * factor;
        // getFlowLimit: up to the first block that stops air (a solid one).
        let mut reach = max;
        for i in 0..max.ceil() as i32 {
            let at = add(pos, facing.map(|v| v * (i + 1)));
            if !self.world.chunks.contains_key(&(at[0] >> 4, at[2] >> 4)) {
                reach = i as f32;
                break;
            }
            let facts = self.res.facts().get(self.world.block(at[0], at[1], at[2]));
            if facts.full_collision && self.machine_kind(at).is_none() {
                reach = (i as f32).min(max);
                break;
            }
        }
        let limit = reach.ceil() as usize;
        // The segments: each block takes the processing of the last catalyst the air passed.
        let mut types = vec![None; limit];
        let mut kind = None;
        let order: Vec<usize> = if pushing {
            (1..=limit).collect()
        } else {
            (1..=limit).rev().collect()
        };
        for i in order {
            let at = add(pos, facing.map(|v| v * i as i32));
            if let Some(found) = self.processing_at(at) {
                kind = Some(found);
            }
            types[i - 1] = kind;
        }
        Current {
            facing,
            pushing,
            reach,
            types,
            built: self.world.game_time,
        }
    }

    /// `EncasedFanBlockEntity.tick` for the fan at `pos` facing `facing`, turning at `speed`
    /// about its back's outward way (kinetics.rs): its current laid out again now and then, the
    /// items in it pushed and processed, a depot's item in it processed.
    pub(super) fn tick_fan(&mut self, pos: [i32; 3], facing: [i32; 3], speed: f32) {
        if speed == 0.0 {
            self.machines.currents.remove(&pos);
            return;
        }
        // Its speed about its facing (convertToDirection): the back's outward way turned round.
        let speed = -speed;
        let stale = self.machines.currents.get(&pos).is_none_or(|c| {
            c.facing != facing
                || c.pushing != (speed > 0.0)
                || self.world.game_time - c.built >= CHECK_RATE
        });
        if stale {
            let current = self.build_current(pos, facing, speed);
            self.machines.currents.insert(pos, current);
        }
        let Some(current) = self.machines.currents.get(&pos) else {
            return;
        };
        let (pushing, reach) = (current.pushing, current.reach);
        if reach < 0.25 {
            return;
        }
        let center = pos.map(|v| f64::from(v) + 0.5);
        let flow = if pushing { facing } else { facing.map(|v| -v) };
        // The current's box: the blocks in front of the fan, as far as it reaches.
        let ahead = facing.map(|v| f64::from(v) * (f64::from(reach) - 1.0));
        let first = add(pos, facing).map(f64::from);
        let (lo, hi): ([f64; 3], [f64; 3]) = (
            std::array::from_fn(|i| first[i].min(first[i] + ahead[i])),
            std::array::from_fn(|i| (first[i] + 1.0).max(first[i] + 1.0 + ahead[i])),
        );
        let mut processed: Vec<(usize, Processing)> = Vec::new();
        for (i, d) in self.dropped.iter_mut().enumerate() {
            let p = [d.position.x, d.position.y + 0.125, d.position.z];
            if !(0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k]) {
                continue;
            }
            // tickAffectedEntities: towards the flow, stronger nearer the fan.
            let old = ((p[0] - center[0]).powi(2)
                + (p[1] - center[1]).powi(2)
                + (p[2] - center[2]).powi(2))
            .sqrt()
            .max(0.01);
            let acceleration = f64::from(speed.abs()) / 512.0 / (old / f64::from(reach));
            let motion = [&mut d.velocity.x, &mut d.velocity.y, &mut d.velocity.z];
            for (k, m) in motion.into_iter().enumerate() {
                let push = (f64::from(flow[k]) * acceleration - *m).clamp(-5.0, 5.0);
                *m += push / 8.0;
            }
            // VecHelper.alignedDistanceToFace: along the facing, from the fan's face.
            let along: f64 = (0..3)
                .map(|k| (p[k] - center[k]) * f64::from(facing[k]))
                .sum::<f64>()
                - 0.5;
            if let Some(kind) = current.type_at(along as f32) {
                processed.push((i, kind));
            }
        }
        // Removed and added back to front, so the indices hold.
        for (i, kind) in processed.into_iter().rev() {
            self.fan_process_item(i, kind);
        }
        // findAffectedHandlers: a depot in the current, or under it (a horizontal one).
        let limit = reach.ceil() as i32;
        for i in 1..=limit {
            let Some(kind) = self
                .machines
                .currents
                .get(&pos)
                .and_then(|c| c.type_at(i as f32 - 1.0))
            else {
                continue;
            };
            let at = add(pos, facing.map(|v| v * i));
            let below = [at[0], at[1] - 1, at[2]];
            for depot in [at, below] {
                if self.machine_kind(depot) == Some(super::machine_entities::Kind::Depot) {
                    self.fan_process_depot(depot, kind);
                }
                if facing[1] != 0 {
                    break;
                }
            }
        }
    }

    /// What `kind` makes of `stack` (`FanProcessingType.process`): None when it does nothing
    /// to it; an empty list when it burns it away (food in a blast).
    fn fan_results(&mut self, kind: Processing, stack: &ItemStack) -> Option<Vec<ItemStack>> {
        let res = std::sync::Arc::clone(&self.res);
        let context = std::sync::Arc::clone(&self.loop_context);
        let cooked = |kind: &str| {
            res.recipes()
                .cook(&context, kind, stack)
                .map(|c| ItemStack {
                    count: c.result.count * stack.count,
                    ..c.result.clone()
                })
        };
        let by_recipe = |game: &mut Self, kind: &str| {
            let recipe = res.create_recipes().find(kind, &stack.item)?;
            // RecipeApplier.applyRecipeOn: each of the stack's items rolls the results.
            let mut out: Vec<ItemStack> = Vec::new();
            for _ in 0..stack.count {
                for r in recipe.roll(&mut game.world.random) {
                    match out.iter_mut().find(|s| s.item == r.item && s.count < 64) {
                        Some(s) => s.count += r.count,
                        None => out.push(r),
                    }
                }
            }
            Some(out)
        };
        match kind {
            Processing::Splashing => by_recipe(self, "splashing"),
            Processing::Haunting => by_recipe(self, "haunting"),
            Processing::Smoking => cooked("smoking").map(|s| vec![s]),
            Processing::Blasting => {
                let smelted = cooked("smelting").or_else(|| cooked("blasting"))?;
                // Food the smoker cooks as the furnace does burns away in a blast.
                let smoked = cooked("smoking");
                if smoked.is_some_and(|s| s.item == smelted.item) {
                    return Some(Vec::new());
                }
                Some(vec![smelted])
            }
        }
    }

    /// `FanProcessing.applyProcessing` for the dropped item at `index`: its time runs down, and
    /// at its end it becomes what the processing makes of it.
    fn fan_process_item(&mut self, index: usize, kind: Processing) {
        let Some(d) = self.dropped.get(index) else {
            return;
        };
        let (id, stack) = (d.id, d.stack.clone());
        // decrementProcessingTime: a new kind starts the time again.
        let entry = self.machines.fan_items.get(&id).copied();
        let left = match entry {
            Some((k, t)) if k == kind => t - 1,
            _ => PROCESSING_TIME * ((stack.count as i32 - 1) / 16 + 1),
        };
        if left == -2 {
            // Nothing to make of it (Time -1 in Create's data).
            return;
        }
        if left > 0 {
            self.machines.fan_items.insert(id, (kind, left));
            return;
        }
        let Some(results) = self.fan_results(kind, &stack) else {
            self.machines.fan_items.insert(id, (kind, -1));
            return;
        };
        self.machines.fan_items.remove(&id);
        let d = self.dropped.remove(index);
        let at = [d.position.x, d.position.y, d.position.z];
        let motion = [d.velocity.x, d.velocity.y, d.velocity.z];
        for stack in results {
            self.spawn_item(stack, at, motion, 0);
        }
    }

    /// `FanProcessing.applyProcessing` on a depot's held item.
    fn fan_process_depot(&mut self, depot: [i32; 3], kind: Processing) {
        let Some((_, mut e)) = self.machine_entity(depot) else {
            return;
        };
        let Some(held) = e.items[0].clone() else {
            self.machines.fan_depots.remove(&depot);
            return;
        };
        let left = match self.machines.fan_depots.get(&depot).copied() {
            Some((k, t)) if k == kind => t - 1,
            _ => PROCESSING_TIME * ((held.count as i32 - 1) / 16 + 1) + 1,
        };
        if left == -2 {
            return;
        }
        if left > 0 {
            self.machines.fan_depots.insert(depot, (kind, left));
            return;
        }
        let Some(results) = self.fan_results(kind, &held) else {
            self.machines.fan_depots.insert(depot, (kind, -1));
            return;
        };
        self.machines.fan_depots.remove(&depot);
        // The held stack becomes the first result, the others go to the outputs.
        let mut results = results.into_iter();
        e.items[0] = results.next();
        for rest in results {
            let at = [
                f64::from(depot[0]) + 0.5,
                f64::from(depot[1]) + 1.0,
                f64::from(depot[2]) + 0.5,
            ];
            if let Some(left) = super::processing::insert_stacked(&mut e.items[1..9], rest, 64) {
                self.spawn_item(left, at, [0.0; 3], 10);
            }
        }
        self.set_machine_entity(depot, &e);
    }
}

/// Forgets the processing of items gone and fans gone.
pub(super) fn forget(
    items: &mut HashMap<u32, (Processing, i32)>,
    alive: &std::collections::HashSet<u32>,
) {
    items.retain(|id, _| alive.contains(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_current_carries_the_last_catalyst_it_passed() {
        // Pushing: water at block 2 washes from there on.
        let push = Current {
            facing: [1, 0, 0],
            pushing: true,
            reach: 4.0,
            types: vec![
                None,
                Some(Processing::Splashing),
                Some(Processing::Splashing),
                None,
            ],
            built: 0,
        };
        assert_eq!(push.type_at(0.5), None);
        assert_eq!(push.type_at(1.5), Some(Processing::Splashing));
        assert_eq!(push.type_at(4.5), None);
    }
}
