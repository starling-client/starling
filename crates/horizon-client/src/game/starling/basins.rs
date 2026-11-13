//! Create's basin and the machines working on it, where this game simulates the world
//! (Starling; ported from Create 6.0.10, MIT, licenses/create-MIT.txt): the basin's items and
//! fluids (BasinBlockEntity, BasinBlock), a mechanical mixer two blocks above it mixing
//! (MechanicalMixerBlockEntity, BasinOperatingBlockEntity) and a press there compacting
//! (MechanicalPressBlockEntity.tryProcessInBasin), each by BasinRecipe.apply. A basin's heat
//! comes from what is under it (a blaze burner, later); none for now.
use super::machine_entities::{Kind, MachineEntity};
use super::{Game, Hand};
use horizon_client::items::ItemStack;
use horizon_core::random::RandomSource;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

/// A basin's input and output slots (BasinInventory: 9 each).
pub(super) const INPUTS: Range<usize> = 0..9;
pub(super) const OUTPUTS: Range<usize> = 9..18;
/// Its input and output tanks, and what each holds (SmartFluidTankBehaviour, 1000 mB).
const IN_TANKS: Range<usize> = 0..2;
const OUT_TANKS: Range<usize> = 2..4;
const TANK: i32 = 1000;
/// `BasinBlock`'s floor: items fall in to it.
const FLOOR: f64 = 2.0 / 16.0;
/// Ticks between a mixer's or a press's looks for a recipe in the basin under it (Create looks
/// when the basin's contents change; the tick spreads the looks out).
const LOOK: i64 = 10;

/// `HeatCondition`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Heat {
    #[default]
    None,
    Heated,
    Superheated,
}

/// One of Create's basin recipes (`create:mixing`, `create:compacting`), or a vanilla crafting
/// one a basin takes (shapeless with more than one ingredient for the mixer; 2x2 and 3x3 of one
/// ingredient for the press).
pub(super) struct BasinRecipe {
    /// Each item ingredient takes one item: the items it allows.
    items: Vec<HashSet<Arc<str>>>,
    /// Each fluid ingredient: the fluids it allows and how many millibuckets.
    fluids: Vec<(HashSet<Arc<str>>, i32)>,
    heat: Heat,
    /// `processing_time` (0: none given).
    time: i32,
    /// Item results with their chances.
    results: Vec<(Arc<str>, u32, f32)>,
    /// Fluid results.
    fluid_results: Vec<(Arc<str>, i32)>,
}

/// The basin recipes by machine: `mixing` and `compacting`, more ingredients first
/// (BasinOperatingBlockEntity.getMatchingRecipes).
#[derive(Default)]
pub(super) struct BasinRecipes {
    by_type: HashMap<&'static str, Vec<Arc<BasinRecipe>>>,
    /// The items that superheat a blaze burner and their burn time (Create's
    /// `superheated_blaze_burner_fuels` data map).
    pub(super) superheated: HashMap<String, i32>,
}

impl BasinRecipes {
    /// `data/<namespace>/recipe/{mixing,compacting}/**.json` of the content root, and the
    /// crafting recipes a basin takes from `crafting` (vanilla's and the mods' shapeless and
    /// shaped ones: (shape, ingredients, result id and count)).
    pub(super) fn load(
        root: &Path,
        item_tag: &dyn Fn(&str) -> Vec<String>,
        fluid_tag: &dyn Fn(&str) -> Vec<String>,
        crafting: &[CraftingRecipe],
    ) -> Self {
        let mut out = BasinRecipes::default();
        let Ok(namespaces) = std::fs::read_dir(root.join("data")) else {
            return out;
        };
        let map = root.join("data/create/data_maps/item/superheated_blaze_burner_fuels.json");
        if let Some(v) = std::fs::read(map)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            && let Some(values) = v["values"].as_object()
        {
            for (item, fuel) in values {
                let time = fuel["burn_time"].as_i64().unwrap_or(3200) as i32;
                out.superheated.insert(item.clone(), time);
            }
        }
        let mut namespaces: Vec<_> = namespaces.flatten().map(|e| e.path()).collect();
        namespaces.sort();
        for ns in namespaces {
            for kind in ["mixing", "compacting"] {
                let mut stack = vec![ns.join("recipe").join(kind)];
                while let Some(dir) = stack.pop() {
                    let mut entries: Vec<_> = std::fs::read_dir(&dir)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path())
                        .collect();
                    entries.sort();
                    for path in entries {
                        if path.is_dir() {
                            stack.push(path);
                            continue;
                        }
                        let recipe = std::fs::read(&path)
                            .ok()
                            .and_then(|b| serde_json::from_slice(&b).ok())
                            .and_then(|v| BasinRecipe::parse(&v, kind, item_tag, fluid_tag));
                        if let Some(r) = recipe {
                            out.by_type.entry(kind).or_default().push(Arc::new(r));
                        }
                    }
                }
            }
        }
        for c in crafting {
            // allowShapelessInMixer: shapeless with more than one ingredient (not a square);
            // allowShapedSquareInPress: 2x2 or 3x3 of one ingredient (MechanicalPressBlockEntity
            // .canCompress).
            let square = (c.ingredients.len() == 4 || c.ingredients.len() == 9)
                && c.ingredients.windows(2).all(|w| w[0] == w[1]);
            let kind = match (c.shapeless, square) {
                (_, true) => "compacting",
                (true, false) if c.ingredients.len() > 1 => "mixing",
                _ => continue,
            };
            out.by_type
                .entry(kind)
                .or_default()
                .push(Arc::new(BasinRecipe {
                    items: c.ingredients.clone(),
                    fluids: Vec::new(),
                    heat: Heat::None,
                    time: 0,
                    results: vec![(c.result.clone(), c.count, 1.0)],
                    fluid_results: Vec::new(),
                }));
        }
        for list in out.by_type.values_mut() {
            // A stable sort: of two with as many, the first read stays first.
            list.sort_by_key(|r| std::cmp::Reverse(r.items.len() + r.fluids.len()));
        }
        let counts: Vec<String> = ["mixing", "compacting"]
            .iter()
            .map(|k| format!("{} {k}", out.by_type.get(k).map_or(0, Vec::len)))
            .collect();
        if out.by_type.values().any(|l| !l.is_empty()) {
            log::info!("Horizon: basin recipes: {}", counts.join(", "));
        }
        out
    }
    fn of(&self, kind: &str) -> &[Arc<BasinRecipe>] {
        self.by_type.get(kind).map_or(&[], Vec::as_slice)
    }
}

/// A crafting recipe as a basin may take it.
pub(super) struct CraftingRecipe {
    pub(super) shapeless: bool,
    /// One set of items per ingredient, empty slots left out.
    pub(super) ingredients: Vec<HashSet<Arc<str>>>,
    pub(super) result: Arc<str>,
    pub(super) count: u32,
}

impl BasinRecipe {
    fn parse(
        v: &serde_json::Value,
        kind: &str,
        item_tag: &dyn Fn(&str) -> Vec<String>,
        fluid_tag: &dyn Fn(&str) -> Vec<String>,
    ) -> Option<Self> {
        if v["type"].as_str()? != format!("create:{kind}")
            || v.get("neoforge:conditions").is_some()
            || v.get("conditions").is_some()
        {
            return None;
        }
        let mut items = Vec::new();
        let mut fluids = Vec::new();
        for ingredient in v["ingredients"].as_array()? {
            if let Some(amount) = ingredient["amount"].as_i64() {
                // A fluid: `neoforge:single` names one, `neoforge:tag` a tag.
                let mut allowed: HashSet<Arc<str>> = HashSet::new();
                if let Some(f) = ingredient["fluid"].as_str() {
                    allowed.insert(f.into());
                }
                if let Some(t) = ingredient["tag"].as_str() {
                    allowed.extend(fluid_tag(t).into_iter().map(Arc::from));
                }
                if allowed.is_empty() {
                    return None;
                }
                fluids.push((allowed, amount as i32));
                continue;
            }
            let alternatives = ingredient
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![ingredient.clone()]);
            let mut allowed: HashSet<Arc<str>> = HashSet::new();
            for a in &alternatives {
                if let Some(id) = a["item"].as_str() {
                    allowed.insert(id.into());
                } else if let Some(t) = a["tag"].as_str() {
                    allowed.extend(item_tag(t).into_iter().map(Arc::from));
                }
            }
            if allowed.is_empty() {
                // A tag nothing is in: the recipe can never match.
                return None;
            }
            items.push(allowed);
        }
        let mut results = Vec::new();
        let mut fluid_results = Vec::new();
        for r in v["results"].as_array()? {
            let id = r["id"].as_str().or_else(|| r["item"].as_str())?;
            if let Some(amount) = r["amount"].as_i64() {
                fluid_results.push((Arc::from(id), amount as i32));
            } else {
                let count = r["count"].as_u64().unwrap_or(1) as u32;
                let chance = r["chance"].as_f64().unwrap_or(1.0) as f32;
                results.push((Arc::from(id), count, chance));
            }
        }
        let heat = match v["heat_requirement"].as_str() {
            Some("heated") => Heat::Heated,
            Some("superheated") => Heat::Superheated,
            _ => Heat::None,
        };
        Some(BasinRecipe {
            items,
            fluids,
            heat,
            time: v["processing_time"].as_i64().unwrap_or(0) as i32,
            results,
            fluid_results,
        })
    }
}

/// `BasinRecipe.apply` on `e` (a basin): its ingredients taken from the inputs, its results put
/// in the outputs, all or nothing. False, `e` untouched, when it does not match or its results
/// do not fit.
fn apply(
    e: &mut MachineEntity,
    r: &BasinRecipe,
    heat: Heat,
    random: &mut impl RandomSource,
) -> bool {
    // HeatCondition.testBlazeBurner: a heated recipe wants heat, a superheated one more.
    if r.heat > heat {
        return false;
    }
    let mut next = e.clone();
    for allowed in &r.items {
        let slot = INPUTS.clone().find(|s| {
            next.items[*s]
                .as_ref()
                .is_some_and(|stack| allowed.contains(&stack.item))
        });
        let Some(slot) = slot else {
            return false;
        };
        let stack = next.items[slot].as_mut().expect("found above");
        stack.count -= 1;
        if stack.count == 0 {
            next.items[slot] = None;
        }
    }
    for (allowed, amount) in &r.fluids {
        let mut needed = *amount;
        for tank in IN_TANKS {
            let Some((fluid, held)) = next.fluids[tank].as_mut() else {
                continue;
            };
            if !allowed.contains(fluid.as_str()) {
                continue;
            }
            let drained = needed.min(*held);
            *held -= drained;
            needed -= drained;
            if *held == 0 {
                next.fluids[tank] = None;
            }
            if needed == 0 {
                break;
            }
        }
        if needed > 0 {
            return false;
        }
    }
    // BasinBlockEntity.acceptOutputs: every item and fluid must fit.
    for (id, count, chance) in &r.results {
        if *chance < 1.0 && random.next_float() >= *chance {
            continue;
        }
        let stack = ItemStack {
            item: id.clone(),
            count: *count,
            damage: 0,
            extra: None,
        };
        if super::processing::insert_stacked(&mut next.items[OUTPUTS], stack, 64).is_some() {
            return false;
        }
    }
    for (fluid, amount) in &r.fluid_results {
        let tank = OUT_TANKS.clone().find(|t| match &next.fluids[*t] {
            None => true,
            Some((f, held)) => f.as_str() == &**fluid && held + amount <= TANK,
        });
        let Some(tank) = tank else {
            return false;
        };
        match &mut next.fluids[tank] {
            Some((_, held)) => *held += amount,
            slot => *slot = Some((fluid.to_string(), *amount)),
        }
    }
    *e = next;
    true
}

/// Whether `r` matches what `e` holds and its results would fit, as [`apply`] with a random
/// that keeps every result.
fn matches(e: &MachineEntity, r: &BasinRecipe, heat: Heat) -> bool {
    let mut copy = e.clone();
    let mut always = horizon_core::random::LegacyRandom::new(0);
    apply_keeping(&mut copy, r, heat, &mut always)
}

/// [`apply`] with every result kept whatever its chance (the simulated pass sees the most).
fn apply_keeping(
    e: &mut MachineEntity,
    r: &BasinRecipe,
    heat: Heat,
    random: &mut impl RandomSource,
) -> bool {
    let sure = BasinRecipe {
        items: r.items.clone(),
        fluids: r.fluids.clone(),
        heat: r.heat,
        time: r.time,
        results: r
            .results
            .iter()
            .map(|(i, c, _)| (i.clone(), *c, 1.0))
            .collect(),
        fluid_results: r.fluid_results.clone(),
    };
    apply(e, &sure, heat, random)
}

/// A mixer's `Running`: going down and mixing (1), done and going up (2).
pub(super) const MIX_DOWN: i32 = 1;
pub(super) const MIX_UP: i32 = 2;

/// `BlazeBurnerBlockEntity.MAX_HEAT_CAPACITY`, `INSERTION_THRESHOLD`: the most burn time a
/// burner holds, and how low it must be before more of the same fuel goes in.
const MAX_HEAT: i32 = 10_000;
const INSERT_BELOW: i32 = 500;
/// A burner's `Running`: its fuel (`FuelType`).
const FUEL_NONE: i32 = 0;
const FUEL_NORMAL: i32 = 1;
const FUEL_SPECIAL: i32 = 2;

impl Game<'_> {
    /// The heat under the basin at `pos` (BasinBlockEntity.getHeatLevel): a blaze burner's,
    /// by its state's `blaze` (BlazeBurnerBlock.getHeatLevelOf).
    fn basin_heat(&self, pos: [i32; 3]) -> Heat {
        let below = self.world.block(pos[0], pos[1] - 1, pos[2]);
        if &*self.res.facts().get(below).name != "create:blaze_burner" {
            return Heat::None;
        }
        let registry = self.pack.assets.registry;
        match registry
            .state(below)
            .and_then(|s| s.properties.get("blaze"))
            .map(String::as_str)
        {
            Some("fading" | "kindled") => Heat::Heated,
            Some("seething") => Heat::Superheated,
            _ => Heat::None,
        }
    }

    /// `BlazeBurnerBlockEntity.tick`: its fuel burns down, a special one into the normal for
    /// half its capacity, and its state's `blaze` follows (`getHeatLevel`).
    pub(super) fn tick_burner(&mut self, pos: [i32; 3]) {
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        let before = (e.running, e.timer);
        if e.timer > 0 {
            e.timer -= 1;
        }
        if e.timer == 0 {
            if e.running == FUEL_SPECIAL {
                e.running = FUEL_NORMAL;
                e.timer = MAX_HEAT / 2;
            } else {
                e.running = FUEL_NONE;
            }
        }
        let heat = match e.running {
            FUEL_SPECIAL => "seething",
            FUEL_NORMAL if (e.timer as f64 / f64::from(MAX_HEAT)) < 0.0125 => "fading",
            FUEL_NORMAL => "kindled",
            _ => "smouldering",
        };
        self.set_blaze(pos, heat);
        // Its fuel changed, or now and then for a save; its timer meanwhile beside its tag.
        if e.running != before.0 || self.world.game_time % 100 == 0 {
            self.set_machine_entity(pos, &e);
        } else if e.timer != before.1 {
            self.machines.timers.insert(pos, e.timer);
        }
    }

    /// `setBlockHeat`: the burner's state with `blaze` as `heat`.
    fn set_blaze(&mut self, pos: [i32; 3], heat: &str) {
        let registry = self.pack.assets.registry;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let Some(mut s) = registry.state(state).cloned() else {
            return;
        };
        if s.properties.get("blaze").map(String::as_str) == Some(heat) {
            return;
        }
        s.properties.insert("blaze".into(), heat.into());
        let Some(new) = registry.id(&s) else {
            return;
        };
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, new, 3));
    }

    /// `BlazeBurnerBlock.useItemOn` with fuel (`tryUpdateFuel`): a superheating one (Create's
    /// data map: a blaze cake) as special fuel, anything a furnace burns as normal fuel; more of
    /// the same only once it is low, a lesser one not at all.
    pub(super) fn use_burner(&mut self, pos: [i32; 3]) -> bool {
        let Some(held) = self.held().cloned() else {
            return false;
        };
        let res = Arc::clone(&self.res);
        let special = res.basin_recipes().superheated.get(&*held.item).copied();
        let (fuel, burn) = match special {
            Some(time) => (FUEL_SPECIAL, time),
            None => {
                let time = res.providers().burn_time(res.items(), &held, false);
                if time <= 0 {
                    return false;
                }
                (FUEL_NORMAL, time)
            }
        };
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return false;
        };
        if fuel < e.running {
            return false;
        }
        let burn = if fuel == e.running {
            if e.timer > INSERT_BELOW {
                return false;
            }
            burn + e.timer
        } else {
            burn
        };
        e.running = fuel;
        e.timer = burn.min(MAX_HEAT);
        if !self.creative {
            self.consume_held();
        }
        let at = pos.map(|v| f64::from(v) + 0.5);
        self.audio
            .play("minecraft:entity.blaze.shoot", Some(at), 0.25, 1.0);
        self.set_machine_entity(pos, &e);
        self.tick_burner(pos);
        true
    }

    /// Once a tick for each basin: items falling into it go in (BasinBlock's entityInside).
    pub(super) fn tick_basin(&mut self, pos: [i32; 3]) {
        let lo = [f64::from(pos[0]), f64::from(pos[2])];
        let floor = f64::from(pos[1]) + FLOOR;
        let found = self.dropped.iter().position(|d| {
            d.position.x >= lo[0] + 0.125
                && d.position.x < lo[0] + 0.875
                && d.position.z >= lo[1] + 0.125
                && d.position.z < lo[1] + 0.875
                && d.position.y >= floor - 0.01
                && d.position.y < f64::from(pos[1]) + 0.9
        });
        let Some(i) = found else {
            return;
        };
        let stack = self.dropped[i].stack.clone();
        let rest = self.basin_insert(pos, stack.clone());
        match rest {
            None => {
                self.dropped.remove(i);
            }
            Some(rest) if rest.count != stack.count => {
                let d = self.dropped.remove(i);
                let at = [d.position.x, d.position.y, d.position.z];
                self.spawn_item(rest, at, [0.0; 3], 0);
            }
            Some(_) => {}
        }
    }

    /// `BasinInventory.insertItem` into the inputs: onto stacks of its item, then empty slots.
    /// What did not go in.
    pub(super) fn basin_insert(&mut self, pos: [i32; 3], stack: ItemStack) -> Option<ItemStack> {
        let Some((Kind::Basin, mut e)) = self.machine_entity(pos) else {
            return Some(stack);
        };
        let max = self.max_stack(&stack);
        let count = stack.count;
        let rest = super::processing::insert_stacked(&mut e.items[INPUTS], stack, max);
        if rest.as_ref().is_none_or(|r| r.count != count) {
            self.set_machine_entity(pos, &e);
        }
        rest
    }

    /// `BasinBlock.useItemOn`: a bucket of water or lava fills an input tank, an empty bucket
    /// takes a full output (else input) tank's thousand, an empty hand takes every item.
    pub(super) fn use_basin(&mut self, pos: [i32; 3]) -> bool {
        if self.hand != Hand::Main {
            return false;
        }
        let Some((Kind::Basin, mut e)) = self.machine_entity(pos) else {
            return false;
        };
        let held = self.held().map(|s| s.item.clone());
        let center = pos.map(|v| f64::from(v) + 0.5);
        match held.as_deref() {
            Some("minecraft:water_bucket" | "minecraft:lava_bucket") => {
                let fluid = if held.as_deref() == Some("minecraft:water_bucket") {
                    "minecraft:water"
                } else {
                    "minecraft:lava"
                };
                let tank = IN_TANKS.clone().find(|t| match &e.fluids[*t] {
                    None => true,
                    Some((f, held)) => f == fluid && held + TANK <= TANK,
                });
                if let Some(tank) = tank {
                    e.fluids[tank] = Some((fluid.to_owned(), TANK));
                    if !self.creative {
                        *self.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
                    }
                    self.audio
                        .play("minecraft:item.bucket.empty", Some(center), 1.0, 1.0);
                    self.set_machine_entity(pos, &e);
                }
                true
            }
            Some("minecraft:bucket") => {
                let tank = OUT_TANKS
                    .chain(IN_TANKS)
                    .find(|t| e.fluids[*t].as_ref().is_some_and(|(_, held)| *held >= TANK));
                let filled = tank.and_then(|t| {
                    let (fluid, _) = e.fluids[t].take()?;
                    match fluid.as_str() {
                        "minecraft:water" => Some("minecraft:water_bucket"),
                        "minecraft:lava" => Some("minecraft:lava_bucket"),
                        "minecraft:milk" => Some("minecraft:milk_bucket"),
                        _ => {
                            // A fluid no bucket holds stays.
                            e.fluids[t] = Some((fluid, TANK));
                            None
                        }
                    }
                });
                if let Some(bucket) = filled {
                    self.consume_held();
                    self.give_or_drop(ItemStack::new(bucket, 1));
                    self.audio
                        .play("minecraft:item.bucket.fill", Some(center), 1.0, 1.0);
                    self.set_machine_entity(pos, &e);
                }
                true
            }
            None => {
                let taken: Vec<ItemStack> = e.items.iter_mut().filter_map(Option::take).collect();
                if taken.is_empty() {
                    return true;
                }
                for stack in taken {
                    self.give_or_drop(stack);
                }
                self.audio
                    .play("minecraft:entity.item.pickup", Some(center), 0.2, 1.0);
                self.set_machine_entity(pos, &e);
                true
            }
            Some(_) => false,
        }
    }

    /// The first recipe of `kind` the basin at `pos` can make now.
    fn basin_recipe(&self, pos: [i32; 3], kind: &str) -> Option<Arc<BasinRecipe>> {
        let (k, e) = self.machine_entity(pos)?;
        if k != Kind::Basin
            || e.items[INPUTS].iter().all(Option::is_none) && e.fluids.iter().all(Option::is_none)
        {
            return None;
        }
        let heat = self.basin_heat(pos);
        self.res
            .basin_recipes()
            .of(kind)
            .iter()
            .find(|r| matches(&e, r, heat))
            .cloned()
    }

    /// `BasinOperatingBlockEntity.applyBasinRecipe` of `kind` on the basin at `pos`: true when it
    /// made something, and whether it can go on making it.
    fn apply_basin_recipe(&mut self, pos: [i32; 3], kind: &str) -> (bool, bool) {
        let Some(recipe) = self.basin_recipe(pos, kind) else {
            return (false, false);
        };
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return (false, false);
        };
        let heat = self.basin_heat(pos);
        if !apply(&mut e, &recipe, heat, &mut self.world.random) {
            return (false, false);
        }
        let again = matches(&e, &recipe, heat);
        self.set_machine_entity(pos, &e);
        (true, again)
    }

    /// `MechanicalMixerBlockEntity.tick` (and BasinOperatingBlockEntity's look for a recipe) for
    /// the mixer at `pos` turning at `speed`: down into the basin two blocks below in 20 ticks,
    /// mixing as long as the recipe and the speed say, then up in 20.
    pub(super) fn tick_mixer(&mut self, pos: [i32; 3], speed: f32) {
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        let basin = [pos[0], pos[1] - 2, pos[2]];
        if e.running == 0 {
            if speed == 0.0
                || (self.world.game_time + i64::from(pos[0] ^ pos[2])) % LOOK != 0
                || self.basin_recipe(basin, "mixing").is_none()
            {
                return;
            }
            // startProcessingBasin.
            e.running = MIX_DOWN;
            e.timer = 0;
            e.work = -1;
            e.since = self.world.game_time;
            self.set_machine_entity(pos, &e);
            return;
        }
        if e.timer >= 40 {
            e.running = 0;
            e.timer = 0;
            self.set_machine_entity(pos, &e);
            return;
        }
        let mut changed = false;
        if speed == 0.0 {
            // Stopped: back up from where it is.
            if e.timer < 20 {
                e.timer = 40 - e.timer;
            } else if e.timer == 20 {
                e.timer += 1;
            }
            e.running = MIX_UP;
            e.since = self.world.game_time - i64::from(e.timer - 20);
            changed = true;
        }
        if e.timer == 20 {
            if e.work < 0 {
                let recipe = self.basin_recipe(basin, "mixing");
                let recipe_speed = recipe
                    .as_ref()
                    .filter(|r| r.time != 0)
                    .map_or(1.0, |r| r.time as f32 / 100.0);
                let speed = speed.abs().max(1.0);
                let log2 = ((512.0 / speed) as i32).max(1).ilog2() as i32;
                e.work = (log2 * (recipe_speed * 15.0).ceil() as i32 + 1).max(1);
                let tanks = self
                    .machine_entity(basin)
                    .is_some_and(|(_, b)| b.fluids.iter().any(Option::is_some));
                if tanks {
                    let pitch = if speed < 65.0 { 0.75 } else { 1.5 };
                    self.audio.play(
                        "minecraft:block.bubble_column.whirlpool_ambient",
                        Some(pos.map(|v| f64::from(v) + 0.5)),
                        0.75,
                        pitch,
                    );
                }
            } else {
                e.work -= 1;
                if e.work == 0 {
                    e.timer += 1;
                    e.work = -1;
                    let (made, again) = self.apply_basin_recipe(basin, "mixing");
                    if made && again {
                        // continueWithPreviousRecipe.
                        e.timer = 20;
                    } else {
                        e.running = MIX_UP;
                        e.since = self.world.game_time - 1;
                    }
                    changed = true;
                }
            }
        }
        if e.timer != 20 {
            e.timer += 1;
        }
        if changed {
            self.set_machine_entity(pos, &e);
        } else {
            self.machines.timers.insert(pos, e.timer);
            self.machines.works.insert(pos, e.work);
        }
    }

    /// Whether the press at `pos` has a basin under it with something to compact.
    pub(super) fn press_basin_ready(&self, pos: [i32; 3]) -> bool {
        self.basin_recipe([pos[0], pos[1] - 2, pos[2]], "compacting")
            .is_some()
    }

    /// `MechanicalPressBlockEntity.tryProcessInBasin` at the press's bottom: true when it can go
    /// on compacting (onPressingCompleted starts it again).
    pub(super) fn press_compact(&mut self, pos: [i32; 3]) -> bool {
        self.apply_basin_recipe([pos[0], pos[1] - 2, pos[2]], "compacting")
            .1
    }
}

/// The fluid tags the content root holds (`data/<ns>/tags/fluid/**.json`), resolved to their
/// fluids: `c:milk` and the like, which basin recipes name.
pub(super) fn fluid_tags(root: &Path) -> HashMap<String, Vec<String>> {
    let mut raw: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(namespaces) = std::fs::read_dir(root.join("data")) else {
        return raw;
    };
    for ns in namespaces.flatten() {
        let name = ns.file_name().to_string_lossy().into_owned();
        let base = ns.path().join("tags").join("fluid");
        let mut stack = vec![base.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(rel) = path
                    .strip_prefix(&base)
                    .ok()
                    .and_then(|r| r.to_str())
                    .and_then(|r| r.strip_suffix(".json"))
                else {
                    continue;
                };
                let values: Vec<String> = std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|v| {
                        Some(
                            v["values"]
                                .as_array()?
                                .iter()
                                .filter_map(|x| {
                                    x.as_str().or_else(|| x["id"].as_str()).map(str::to_owned)
                                })
                                .collect(),
                        )
                    })
                    .unwrap_or_default();
                raw.entry(format!("{name}:{}", rel.replace('\\', "/")))
                    .or_default()
                    .extend(values);
            }
        }
    }
    // Nested tags in place, once each.
    let mut out = HashMap::new();
    for tag in raw.keys() {
        let mut seen = HashSet::new();
        let mut list = Vec::new();
        let mut todo = vec![tag.clone()];
        while let Some(t) = todo.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            for v in raw.get(&t).into_iter().flatten() {
                match v.strip_prefix('#') {
                    Some(inner) => todo.push(inner.to_owned()),
                    None if !list.contains(v) => list.push(v.clone()),
                    None => {}
                }
            }
        }
        out.insert(tag.clone(), list);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basin(items: &[(&str, u32)], fluids: &[(&str, i32)]) -> MachineEntity {
        let mut e = MachineEntity::empty(Kind::Basin);
        for (i, (id, n)) in items.iter().enumerate() {
            e.items[i] = Some(ItemStack::new(id, *n));
        }
        for (i, (id, n)) in fluids.iter().enumerate() {
            e.fluids[i] = Some(((*id).to_owned(), *n));
        }
        e
    }

    fn recipe(json: &str) -> BasinRecipe {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let tags = |t: &str| match t {
            "c:nuggets/iron" => vec!["minecraft:iron_nugget".to_owned()],
            _ => Vec::new(),
        };
        let kind = v["type"]
            .as_str()
            .unwrap()
            .trim_start_matches("create:")
            .to_owned();
        BasinRecipe::parse(&v, &kind, &tags, &|_| Vec::new()).unwrap()
    }

    #[test]
    fn a_basin_mixes_its_ingredients_into_its_outputs() {
        let alloy = recipe(
            r#"{"type": "create:mixing", "ingredients": [{"item": "minecraft:andesite"},
                {"tag": "c:nuggets/iron"}], "results": [{"id": "create:andesite_alloy"}]}"#,
        );
        let mut random = horizon_core::random::LegacyRandom::new(1);
        let mut e = basin(
            &[("minecraft:andesite", 2), ("minecraft:iron_nugget", 1)],
            &[],
        );
        assert!(apply(&mut e, &alloy, Heat::None, &mut random));
        assert_eq!(e.items[0].as_ref().map(|s| s.count), Some(1));
        assert!(e.items[1].is_none());
        assert_eq!(
            e.items[OUTPUTS.start].as_ref().map(|s| &*s.item),
            Some("create:andesite_alloy")
        );
        // No nugget left: nothing more, the basin as it was.
        let before = e.clone();
        assert!(!apply(&mut e, &alloy, Heat::None, &mut random));
        assert_eq!(e, before);
    }

    #[test]
    fn fluids_and_heat_are_ingredients_too() {
        let dough = recipe(
            r#"{"type": "create:mixing", "ingredients": [{"item": "create:wheat_flour"},
                {"type": "neoforge:single", "amount": 1000, "fluid": "minecraft:water"}],
                "results": [{"id": "create:dough"}]}"#,
        );
        let brass = recipe(
            r#"{"type": "create:mixing", "heat_requirement": "heated", "ingredients":
                [{"item": "minecraft:copper_ingot"}], "results": [{"count": 2, "id": "x:b"}]}"#,
        );
        let mut random = horizon_core::random::LegacyRandom::new(1);
        let mut dry = basin(&[("create:wheat_flour", 1)], &[]);
        assert!(!matches(&dry, &dough, Heat::None));
        let mut wet = basin(&[("create:wheat_flour", 1)], &[("minecraft:water", 1000)]);
        assert!(apply(&mut wet, &dough, Heat::None, &mut random));
        assert!(wet.fluids[0].is_none());
        dry.items[0] = Some(ItemStack::new("minecraft:copper_ingot", 1));
        assert!(!matches(&dry, &brass, Heat::None));
        assert!(matches(&dry, &brass, Heat::Heated));
    }
}
