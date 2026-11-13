//! Create's working machines where this game simulates the world (the integrated server, a LAN
//! host or a dedicated server; a remote server runs its own): drills break the block they face
//! and crushing wheels crush the items that fall between them while their kinetic networks
//! turn. Ported from Create 6.0.10 (MIT; licenses/create-MIT.txt): BlockBreakingKineticBlockEntity
//! and DrillBlockEntity for the drill, CrushingWheelBlock.updateControllers and
//! CrushingWheelControllerBlockEntity for the wheels, whose controller is no block here but the
//! air between two wheels. The networks are solved from the block states as the window solves
//! them (game/kinetics.rs); a crank only the window's player turns does not reach here.
use super::{Game, machine_entities};
use horizon_client::items::ItemStack;
use horizon_client::kinetics::{self, KineticPart, Part};
use horizon_core::{position::BlockPos, random::RandomSource};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Ticks between looks over the loaded chunks for parts (sooner when blocks change).
const RESCAN_TICKS: i64 = 20;
/// KineticBlockEntity's lazy tick: a drill with nothing to break looks again this often.
const LAZY_TICKS: i64 = 10;
/// ItemEntity.setDefaultPickUpDelay.
const PICKUP_DELAY: u32 = 10;
/// The controller's collision, `CRUSHING_WHEEL_CONTROLLER_COLLISION` facing down: items wait on
/// it (13 sixteenths up) while the wheels are busy.
const WAITING_HEIGHT: f64 = 13.0 / 16.0;

#[derive(Default)]
pub(super) struct Machines {
    parts: HashMap<[i32; 3], Arc<Part>>,
    pub(super) speeds: HashMap<[i32; 3], f32>,
    /// The world's block revision and chunk set at the last look, and when (game time).
    seen: Option<(u64, u64, i64)>,
    drills: HashMap<[i32; 3], Drill>,
    controllers: HashMap<[i32; 3], Controller>,
    /// Items a controller put out, by entity id: that controller does not take them again
    /// (Create's `BypassCrushingWheel`).
    bypass: HashMap<u32, [i32; 3]>,
    /// The machines with block entities (machine_entities.rs) at the last look.
    entities: Vec<([i32; 3], machine_entities::Kind)>,
    /// Their timers that run every tick, since their tags were last kept (processing.rs), and a
    /// mixer's processing ticks.
    pub(super) timers: HashMap<[i32; 3], i32>,
    pub(super) works: HashMap<[i32; 3], i32>,
    /// Each fan's air current (fans.rs), and how far along the items and depots in one are.
    pub(super) currents: HashMap<[i32; 3], super::fans::Current>,
    pub(super) fan_items: HashMap<u32, (super::fans::Processing, i32)>,
    pub(super) fan_depots: HashMap<[i32; 3], (super::fans::Processing, i32)>,
    /// The belts and their items (belts.rs).
    pub(super) belts: super::belts::Belts,
    /// Each extracting funnel's ticks until it extracts again (funnels.rs).
    pub(super) funnels: HashMap<[i32; 3], i32>,
    /// The points marked for the next arm placed (arms.rs).
    pub(super) arm_selection: Vec<super::arms::Point>,
    /// The trees saws felled, falling (falling_trees.rs).
    pub(super) falling: Vec<super::falling_trees::Felled>,
    /// Drills, crushing wheels and controllers at the last look, for the log.
    counts: (usize, usize, usize),
}

/// BlockBreakingKineticBlockEntity's state.
struct Drill {
    /// `destroyProgress`, 0 to 10.
    progress: i32,
    /// `ticksUntilNextProgress`: -1 waits for the lazy tick.
    wait: i32,
    breaking: [i32; 3],
    /// `breakerId`: negative, apart from players' and mobs'.
    breaker: i32,
}

/// CrushingWheelControllerBlockEntity's state.
struct Controller {
    /// The way crushed items go: the way the wheels' rims move between them.
    facing: [i32; 3],
    /// `crushingspeed`: a wheel's speed over 50.
    speed: f32,
    /// `ProcessingInventory`: the item being crushed, then what it became.
    slots: Vec<ItemStack>,
    remaining: f32,
    applied: bool,
}

/// The types of Create's processing recipes the machines use: one item in, results with chances
/// (`create:<type>`).
const RECIPE_TYPES: [&str; 6] = [
    "crushing",
    "milling",
    "pressing",
    "splashing",
    "haunting",
    "cutting",
];

/// Create's processing recipes the content root holds, by type, each's first ingredient's
/// items (a tag's, as the items list it) to its recipes, in the order they were read.
#[derive(Default)]
pub(super) struct Recipes {
    by_type: HashMap<&'static str, HashMap<Arc<str>, Vec<Arc<Recipe>>>>,
}

pub(super) struct Recipe {
    /// `processing_time`, in ticks at speed one.
    pub(super) time: f32,
    /// Each result: the item, its count and its chance.
    pub(super) results: Vec<(Arc<str>, u32, f32)>,
}

impl Recipes {
    /// `data/<namespace>/recipe/<type>/**.json` of the content root. Recipes with conditions
    /// (other mods' compat) are left out; of two for one item, the first read is the one most
    /// machines use. With Create's, the ones it makes as the game loads (RuntimeDataGenerator).
    pub(super) fn load(root: &Path, items: &horizon_client::items::Items) -> Self {
        let tag = |t: &str| items.item_tag_order.get(t).cloned().unwrap_or_default();
        let mut out = Self::read(root, &tag);
        if !out.by_type.is_empty() {
            out.add_generated(&items.registry_order);
        }
        if !out.by_type.is_empty() {
            let counts: Vec<String> = RECIPE_TYPES
                .iter()
                .map(|k| format!("{} {k}", out.by_type.get(k).map_or(0, HashMap::len)))
                .collect();
            log::info!("Horizon: Create recipes by item: {}", counts.join(", "));
        }
        out
    }
    fn read(root: &Path, tag: &dyn Fn(&str) -> Vec<String>) -> Self {
        let mut out = Recipes::default();
        let Ok(namespaces) = std::fs::read_dir(root.join("data")) else {
            return out;
        };
        let mut namespaces: Vec<_> = namespaces.flatten().map(|e| e.path()).collect();
        namespaces.sort();
        for ns in namespaces {
            for kind in RECIPE_TYPES {
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
                        let Some((inputs, recipe)) = std::fs::read(&path)
                            .ok()
                            .and_then(|b| serde_json::from_slice(&b).ok())
                            .and_then(|v| Recipe::parse(&v, kind, tag))
                        else {
                            continue;
                        };
                        out.add(kind, inputs, recipe);
                    }
                }
            }
        }
        out
    }
    fn add(&mut self, kind: &'static str, inputs: Vec<Arc<str>>, recipe: Recipe) {
        let recipe = Arc::new(recipe);
        let map = self.by_type.entry(kind).or_default();
        for input in inputs {
            map.entry(input).or_default().push(recipe.clone());
        }
    }
    /// `RuntimeDataGenerator.insertIntoPack` over the items there are (`registry`): cutting
    /// recipes for every wood (its logs and woods stripped, then planks, the planks into
    /// stairs, slabs, fences, gates, doors, trapdoors, plates, buttons and signs) and splashing
    /// for every concrete powder, each made once.
    fn add_generated(&mut self, registry: &[Arc<str>]) {
        let known: std::collections::HashSet<&str> = registry.iter().map(|i| &**i).collect();
        let mut made = std::collections::HashSet::new();
        for (kind, from, to, count) in generated_recipes(registry, &|id| known.contains(id)) {
            if !made.insert((from.clone(), to.clone())) {
                continue;
            }
            let recipe = Recipe {
                time: if kind == "cutting" { 50.0 } else { 100.0 },
                results: vec![(to.into(), count, 1.0)],
            };
            self.add(kind, vec![from.into()], recipe);
        }
    }
    /// The first recipe of `kind` (`pressing`...) for `item`.
    pub(super) fn find(&self, kind: &str, item: &str) -> Option<&Recipe> {
        self.find_all(kind, item).first().map(|r| &**r)
    }
    /// Every recipe of `kind` for `item`, in the order they were read.
    pub(super) fn find_all(&self, kind: &str, item: &str) -> &[Arc<Recipe>] {
        self.by_type
            .get(kind)
            .and_then(|m| m.get(item))
            .map_or(&[], Vec::as_slice)
    }
    /// The crusher's: crushing, then milling.
    fn crushing(&self, item: &str) -> Option<&Recipe> {
        self.find("crushing", item)
            .or_else(|| self.find("milling", item))
    }
}

impl Recipe {
    /// A recipe of `create:<kind>` and the items its first ingredient takes: `{"item": id}`,
    /// `{"tag": t}` (`tag` lists a tag's items) or a list of them, as 1.21.1 writes them.
    fn parse(
        v: &serde_json::Value,
        kind: &str,
        tag: &dyn Fn(&str) -> Vec<String>,
    ) -> Option<(Vec<Arc<str>>, Recipe)> {
        if v["type"].as_str()? != format!("create:{kind}")
            || v.get("neoforge:conditions").is_some()
            || v.get("conditions").is_some()
        {
            return None;
        }
        let first = v["ingredients"].get(0)?;
        let mut inputs: Vec<Arc<str>> = Vec::new();
        let alternatives = first
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![first.clone()]);
        for a in &alternatives {
            if let Some(id) = a["item"].as_str() {
                inputs.push(id.into());
            } else if let Some(t) = a["tag"].as_str() {
                inputs.extend(tag(t).into_iter().map(Arc::from));
            }
        }
        if inputs.is_empty() {
            return None;
        }
        let results = v["results"]
            .as_array()?
            .iter()
            .filter_map(|r| {
                let id = r["id"].as_str().or_else(|| r["item"].as_str())?;
                let count = r["count"].as_u64().unwrap_or(1) as u32;
                let chance = r["chance"].as_f64().unwrap_or(1.0) as f32;
                Some((id.into(), count, chance))
            })
            .collect();
        let time = v["processing_time"].as_f64().unwrap_or(100.0) as f32;
        Some((inputs, Recipe { time, results }))
    }

    /// `ProcessingRecipe.rollResults` for one item: each result kept by its chance.
    pub(super) fn roll(&self, random: &mut impl RandomSource) -> Vec<ItemStack> {
        let mut out: Vec<ItemStack> = Vec::new();
        for (id, n, chance) in &self.results {
            if *chance < 1.0 && random.next_float() >= *chance {
                continue;
            }
            out.push(ItemStack {
                item: id.clone(),
                count: *n,
                damage: 0,
                extra: None,
            });
        }
        out
    }
}

/// The wood types RuntimeDataGenerator's patterns end on (and `wood` right after a letter, as
/// Botania's livingwood).
const WOOD_TYPES: [&str; 5] = ["_log", "_wood", "_stem", "_hyphae", "_block"];

fn word(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A wood type ending `path` from `from` on, the wood's name before it: the shortest type, so
/// the longest name, as `(\w*)(_log|...|(?<!_)wood)$` matches.
fn wood_type_at_end(path: &str, from: usize) -> Option<(&str, &'static str)> {
    (from..=path.len()).rev().find_map(|g| {
        let rest = &path[g..];
        let kind = WOOD_TYPES.into_iter().find(|t| rest == *t).or_else(|| {
            (rest == "wood" && (g == 0 || !path[..g].ends_with('_'))).then_some("wood")
        })?;
        Some((&path[from..g], kind))
    })
}

/// `STRIPPED_WOODS_PREFIX_REGEX`: a variant prefix, `stripped_`, the wood's name and its type
/// (Rust's regex has no look-behind; the patterns are by hand, for ids of word characters).
fn stripped_prefix(path: &str) -> Option<(&str, &str, &'static str)> {
    if !word(path) {
        return None;
    }
    path.match_indices("stripped_").find_map(|(i, _)| {
        let (name, kind) = wood_type_at_end(path, i + "stripped_".len())?;
        Some((&path[..i], name, kind))
    })
}

/// `STRIPPED_WOOD_SUFFIX_REGEX`: the wood's name, its type, then `_stripped` with variant
/// suffixes around it (joined, as the generator joins them).
fn stripped_suffix(path: &str) -> Option<(&str, &'static str, String)> {
    if !word(path) {
        return None;
    }
    (0..=path.len()).rev().find_map(|t| {
        let rest = &path[t..];
        let kind = WOOD_TYPES
            .into_iter()
            .find(|k| rest.starts_with(k))
            .or_else(|| {
                (rest.starts_with("wood") && (t == 0 || !path[..t].ends_with('_')))
                    .then_some("wood")
            })?;
        let after = &rest[kind.len()..];
        let k = after.rfind("_stripped")?;
        let variant = format!("{}{}", &after[..k], &after[k + "_stripped".len()..]);
        Some((&path[..t], kind, variant))
    })
}

/// `NON_STRIPPED_WOODS_REGEX`: a wood with no stripped kind, its name and type (anything after
/// the type that does not end in `_stripped` is a variant).
fn non_stripped(path: &str) -> Option<(&str, &'static str)> {
    if path.starts_with("stripped_") || !path.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        return None;
    }
    (1..path.len()).rev().find_map(|t| {
        let rest = &path[t..];
        let kind = WOOD_TYPES.into_iter().find(|k| {
            rest.starts_with(k) && (*k != "_block" || !path[..t].ends_with("bioshroom"))
        })?;
        let after = &rest[kind.len()..];
        (after.is_empty() || !after.ends_with("_stripped")).then_some((&path[..t], kind))
    })
}

/// `RuntimeDataGenerator.cuttingRecipes` and `washingRecipes` for every item of `registry`
/// (`exists` says whether an item is there): each recipe's type, input, result and count.
fn generated_recipes(
    registry: &[Arc<str>],
    exists: &dyn Fn(&str) -> bool,
) -> Vec<(&'static str, String, String, u32)> {
    let mut out = Vec::new();
    for id in registry {
        let Some((ns, path)) = id.split_once(':') else {
            continue;
        };
        let at = |p: &str| format!("{ns}:{p}");
        let found = if let Some((prefix, name, kind)) = stripped_prefix(path) {
            Some((prefix, name, kind, String::new(), false))
        } else if let Some((name, kind, variant)) = stripped_suffix(path) {
            Some(("", name, kind, variant, false))
        } else if !exists(&at(&format!("stripped_{path}")))
            && !exists(&at(&format!("{path}_stripped")))
        {
            non_stripped(path).map(|(name, kind)| ("", name, kind, String::new(), true))
        } else {
            None
        };
        if let Some((prefix, name, kind, suffix, no_stripped)) = found {
            let base = if kind == "wood" {
                format!("{name}wood")
            } else {
                name.to_owned()
            };
            let planks = at(&format!("{base}_planks"));
            let count = if kind.contains("block") { 3 } else { 6 };
            let mut wood = |from: &str, to: String, n: u32| {
                if exists(&to) {
                    out.push(("cutting", from.to_owned(), to, n));
                }
            };
            if !no_stripped {
                let unstripped = at(&format!("{prefix}{name}{kind}{suffix}"));
                if exists(&unstripped) {
                    wood(&unstripped, id.to_string(), 1);
                }
                wood(id, planks.clone(), count);
            } else if exists(&planks) {
                // A tag of its own in Create; the item alone here.
                wood(id, planks.clone(), count);
            }
            if !path.contains("_wood") && !path.contains("_hyphae") && exists(&planks) {
                for (made, n) in [
                    ("stairs", 1),
                    ("slab", 2),
                    ("fence", 1),
                    ("fence_gate", 1),
                    ("door", 1),
                    ("trapdoor", 1),
                    ("pressure_plate", 1),
                    ("button", 1),
                    ("sign", 1),
                ] {
                    wood(&planks, at(&format!("{base}_{made}")), n);
                }
            }
        }
        // ConcretePowderBlock's concrete.
        if let Some(concrete) = path.strip_suffix("_concrete_powder") {
            let to = at(&format!("{concrete}_concrete"));
            if exists(&to) {
                out.push(("splashing", id.to_string(), to, 1));
            }
        }
    }
    out
}

/// A part that breaks the block it faces: a drill, a saw facing sideways (`SawBlockEntity.
/// shouldRun`).
fn breaker(part: &Part) -> bool {
    part.kind == KineticPart::Drill || (part.kind == KineticPart::Saw && part.face != [0; 3])
}

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [i32; 3], k: i32) -> [i32; 3] {
    a.map(|v| v * k)
}
fn cross(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(axis: usize, sign: i32) -> [i32; 3] {
    let mut v = [0; 3];
    v[axis] = sign;
    v
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
    /// Once a tick where the world is simulated: the drills and the crushing wheels at work.
    pub(super) fn tick_kinetic_machines(&mut self) -> super::Result<()> {
        if self.res.kinetics().is_empty() {
            return Ok(());
        }
        let now = self.world.game_time;
        let world = (self.world.revision, self.world.chunks.membership_revision());
        let stale = self.machines.seen.is_none_or(|(revision, chunks, at)| {
            (revision, chunks) != world || now - at >= RESCAN_TICKS
        });
        if stale {
            self.find_machines();
            self.machines.seen = Some((world.0, world.1, now));
        }
        let drills: Vec<([i32; 3], f32)> = self
            .machines
            .parts
            .iter()
            .filter(|(_, p)| breaker(p))
            .filter_map(|(pos, _)| Some((*pos, *self.machines.speeds.get(pos)?)))
            .filter(|(_, s)| *s != 0.0)
            .collect();
        self.tick_falling_trees()?;
        for (pos, speed) in drills {
            self.tick_drill(pos, speed)?;
        }
        // Fans: their air currents (fans.rs), facing away from their backs.
        let m = &self.machines;
        let fans: Vec<([i32; 3], [i32; 3], f32)> = m
            .parts
            .iter()
            .filter(|(_, p)| p.kind == KineticPart::Fan)
            .map(|(pos, p)| {
                let speed = m.speeds.get(pos).copied().unwrap_or(0.0);
                (*pos, p.face.map(|v| -v), speed)
            })
            .collect();
        for (pos, facing, speed) in fans {
            self.tick_fan(pos, facing, speed);
        }
        let controllers: Vec<[i32; 3]> = self.machines.controllers.keys().copied().collect();
        for pos in controllers {
            self.tick_controller(pos);
        }
        // The machines with items of their own: depots, millstones, presses (processing.rs).
        let m = &self.machines;
        let machines: Vec<([i32; 3], machine_entities::Kind, f32)> = m
            .entities
            .iter()
            .map(|(pos, kind)| (*pos, *kind, m.speeds.get(pos).copied().unwrap_or(0.0)))
            .collect();
        self.tick_processing(&machines);
        self.tick_belts();
        Ok(())
    }

    /// The kinetic parts in the loaded chunks, their speeds, and the controllers between
    /// crushing wheels turning towards each other (`CrushingWheelBlock.updateControllers`).
    fn find_machines(&mut self) {
        let table = self.res.kinetics();
        let kinetic = |id: u32| {
            let state = horizon_world::registry::BlockStateId(id);
            table.get(state).is_some() || table.machine(state).is_some()
        };
        let mut parts = HashMap::new();
        let mut entities = Vec::new();
        for ((cx, cz), chunk) in self.world.chunks.iter() {
            for (si, section) in chunk.sections.iter().enumerate() {
                if !section.blocks.maybe_has(kinetic) {
                    continue;
                }
                for i in 0..4096 {
                    let state = horizon_world::registry::BlockStateId(section.get(i));
                    let part = table.get(state);
                    let machine = table.machine(state);
                    if part.is_none() && machine.is_none() {
                        continue;
                    }
                    let pos = [
                        cx * 16 + (i & 15) as i32,
                        horizon_client::chunk::MIN_Y + si as i32 * 16 + (i >> 8) as i32,
                        cz * 16 + ((i >> 4) & 15) as i32,
                    ];
                    if let Some(kind) = machine_entities::Kind::of(machine, part.map(|p| p.kind)) {
                        entities.push((pos, kind));
                    }
                    if let Some(part) = part {
                        parts.insert(pos, part.clone());
                    }
                }
            }
        }
        entities.sort_unstable_by_key(|(pos, _)| *pos);
        // Water wheels turned by the water past them are sources too.
        let (world, fluids) = (&self.world, &self.res.motion().fluids);
        let mut wheels = kinetics::water_wheels(&parts, |p| {
            let v = horizon_client::entities::flow_vector(world, fluids, p);
            [v.x, v.y, v.z]
        });
        // And windmills by their sails (bearings.rs).
        let bearings = entities
            .iter()
            .filter(|(_, k)| *k == machine_entities::Kind::Bearing)
            .map(|(p, _)| *p);
        wheels.extend(self.windmill_speeds(bearings));
        // The belts: each one's segments turn together (belts.rs).
        let belt_positions: Vec<[i32; 3]> = entities
            .iter()
            .filter(|(_, k)| *k == machine_entities::Kind::Belt)
            .map(|(p, _)| *p)
            .collect();
        let chains = super::belts::chains(&self.belt_segments(&belt_positions));
        let groups: Vec<Vec<[i32; 3]>> = chains.iter().map(|c| c.segments.clone()).collect();
        let speeds = kinetics::solve_linked(&parts, &wheels, &groups);
        self.machines
            .belts
            .items
            .retain(|start, _| chains.iter().any(|c| c.start() == *start));
        self.machines.belts.chains = chains;
        let mut found: HashMap<[i32; 3], ([i32; 3], f32)> = HashMap::new();
        for (pos, part) in &parts {
            if part.kind != KineticPart::CrushingWheel {
                continue;
            }
            let Some(&speed) = speeds.get(pos).filter(|s| **s != 0.0) else {
                continue;
            };
            for side in DIRECTIONS {
                if side[part.axis] != 0 {
                    continue;
                }
                let other = add(*pos, scale(side, 2));
                let opposite = parts
                    .get(&other)
                    .filter(|o| o.kind == KineticPart::CrushingWheel && o.axis == part.axis)
                    .and_then(|_| speeds.get(&other))
                    .is_some_and(|s| *s != 0.0 && (*s > 0.0) != (speed > 0.0));
                let between = add(*pos, side);
                if !opposite
                    || self.world.block(between[0], between[1], between[2]) != self.world.air
                {
                    continue;
                }
                // The rim at the gap moves along the axis turned towards the gap, by the
                // wheel's sign (about the axis's positive way).
                let along = cross(unit(part.axis, 1), side);
                let facing = scale(along, if speed > 0.0 { 1 } else { -1 });
                found.insert(between, (facing, speed.abs() / 50.0));
            }
        }
        let wheels = parts
            .values()
            .filter(|p| p.kind == KineticPart::CrushingWheel)
            .count();
        let drills = parts.values().filter(|p| breaker(p)).count();
        let counts = (drills, wheels, found.len());
        if counts != self.machines.counts {
            log::info!(
                "Horizon: Create machines: {drills} drills, {wheels} crushing wheels, {} turning \
                 between them ({} turning of {} parts)",
                found.len(),
                speeds.values().filter(|s| **s != 0.0).count(),
                parts.len()
            );
            self.machines.counts = counts;
        }
        let m = &mut self.machines;
        m.controllers
            .retain(|pos, c| found.get(pos).is_some_and(|f| f.0 == c.facing));
        for (pos, (facing, speed)) in found {
            m.controllers
                .entry(pos)
                .or_insert_with(|| Controller {
                    facing,
                    speed,
                    slots: Vec::new(),
                    remaining: 0.0,
                    applied: false,
                })
                .speed = speed;
        }
        m.drills
            .retain(|pos, _| parts.get(pos).is_some_and(|p| breaker(p)));
        let ids: std::collections::HashSet<u32> = self.dropped.iter().map(|d| d.id).collect();
        m.bypass.retain(|id, _| ids.contains(id));
        super::fans::forget(&mut m.fan_items, &ids);
        m.currents
            .retain(|pos, _| parts.get(pos).is_some_and(|p| p.kind == KineticPart::Fan));
        m.parts = parts;
        m.speeds = speeds;
        let here = |pos: &[i32; 3]| entities.binary_search_by_key(pos, |(p, _)| *p).is_ok();
        m.timers.retain(|pos, _| here(pos));
        m.works.retain(|pos, _| here(pos));
        m.funnels.retain(|pos, _| here(pos));
        m.entities = entities;
    }

    /// `BlockBreakingKineticBlockEntity.tick` for the drill at `pos` turning at `speed`: a
    /// step of `destroyProgress` every hardness over a hundredth of the speed ticks, the block
    /// broken at ten.
    fn tick_drill(&mut self, pos: [i32; 3], speed: f32) -> super::Result<()> {
        let Some(part) = self.machines.parts.get(&pos) else {
            return Ok(());
        };
        let saw = part.kind == KineticPart::Saw;
        // Its face is its back: it breaks the block the other way.
        let breaking = add(pos, part.face.map(|v| -v));
        let lazy = self.world.game_time % LAZY_TICKS == 0;
        let breaker = -1000 - self.machines.drills.len() as i32;
        let drill = self.machines.drills.entry(pos).or_insert(Drill {
            progress: 0,
            wait: 0,
            breaking,
            breaker,
        });
        if drill.breaking != breaking {
            drill.breaking = breaking;
            drill.progress = 0;
        }
        if drill.wait < 0 {
            if !lazy {
                return Ok(());
            }
            drill.wait = 1;
        }
        let waited = drill.wait;
        drill.wait -= 1;
        if waited > 0 {
            return Ok(());
        }
        let (progress, breaker) = (drill.progress, drill.breaker);
        let state = self.world.block(breaking[0], breaking[1], breaking[2]);
        let facts = self.res.facts().get(state);
        let hardness = facts.hardness;
        let liquid = matches!(
            &*facts.name,
            "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
        );
        let air = state == self.world.air
            || matches!(&*facts.name, "minecraft:cave_air" | "minecraft:void_air");
        // SawBlockEntity.canBreak: a saw only what it saws.
        let unsawable = saw && !self.sawable(state);
        if air || liquid || hardness < 0.0 || unsawable {
            if progress != 0 {
                if let Some(d) = self.machines.drills.get_mut(&pos) {
                    d.progress = 0;
                }
                self.drill_cracks(breaker, breaking, -1);
            }
            return Ok(());
        }
        let break_speed = speed.abs() / 100.0;
        let step = ((break_speed / hardness) as i32).clamp(1, 10 - progress);
        let progress = progress + step;
        if progress >= 10 {
            if let Some(d) = self.machines.drills.get_mut(&pos) {
                d.progress = 0;
                d.wait = -1;
            }
            self.drill_cracks(breaker, breaking, -1);
            self.drill_break(breaking, state)?;
            // SawBlockEntity.onBlockBroken: the tree (or the plant) it stood under falls too.
            if saw {
                self.fell_tree(pos, breaking, state)?;
            }
            return Ok(());
        }
        if let Some(d) = self.machines.drills.get_mut(&pos) {
            d.progress = progress;
            d.wait = (hardness / break_speed) as i32;
        }
        self.drill_cracks(breaker, breaking, progress);
        Ok(())
    }

    /// `ServerLevel.destroyBlockProgress` for a drill: its cracks where every player nearby
    /// sees them (-1 takes them away).
    fn drill_cracks(&mut self, breaker: i32, pos: [i32; 3], progress: i32) {
        self.destroy_block_progress(breaker, pos, progress);
        if let Some(host) = self.lan.as_ref() {
            host.hub.broadcast_near(
                [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
                64.0,
                &horizon_protocol::packets::play::Clientbound::BlockDestruction(
                    horizon_protocol::packets::play::BlockDestruction {
                        id: breaker,
                        pos,
                        progress: if (0..10).contains(&progress) {
                            progress as u8
                        } else {
                            255
                        },
                    },
                ),
                None,
            );
        }
    }

    /// `BlockBreakingKineticBlockEntity.onBlockBroken`: the block gone with its particles and
    /// sound, its drops (no tool) still at its middle, a little apart.
    fn drill_break(
        &mut self,
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    ) -> super::Result<()> {
        let registry = self.pack.assets.registry;
        let drops = self.res.items().block_drops(
            registry,
            state,
            Default::default(),
            &mut self.world.random,
            None,
        );
        let air = self.world.air;
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, air, 3));
        self.process_level_events()?;
        self.level_event_at(2001, pos, state.0 as i32);
        for stack in drops {
            let r = &mut self.world.random;
            // VecHelper.offsetRandomly(center, random, 1/8).
            let mut at = [0.0; 3];
            for (i, v) in at.iter_mut().enumerate() {
                *v = f64::from(pos[i]) + 0.5 + f64::from(r.next_float() * 2.0 - 1.0) * 0.125;
            }
            self.spawn_item(stack, at, [0.0; 3], PICKUP_DELAY);
        }
        Ok(())
    }

    /// `CrushingWheelControllerBlockEntity.tick` for the controller at `pos`: an item falling in
    /// is taken (one at a time; the others wait on it), crushed by its recipe as fast as the
    /// wheels turn, and what it became put out the way the rims move.
    fn tick_controller(&mut self, pos: [i32; 3]) {
        let Some(c) = self.machines.controllers.get(&pos) else {
            return;
        };
        if c.speed == 0.0 {
            return;
        }
        let (facing, speed, occupied) = (c.facing, c.speed * 4.0, !c.slots.is_empty());
        let (lo, hi) = (pos.map(f64::from), pos.map(|v| f64::from(v) + 1.0));
        // An item entity's box (a quarter block wide and tall, from its feet) in the cell.
        let inside = |p: &horizon_core::geometry::Vec3| {
            let h = 0.125;
            p.x - h < hi[0]
                && p.x + h > lo[0]
                && p.y < hi[1]
                && p.y + 2.0 * h > lo[1]
                && p.z - h < hi[2]
                && p.z + h > lo[2]
        };
        if !occupied {
            let bypass = &self.machines.bypass;
            if let Some(i) = self
                .dropped
                .iter()
                .position(|d| inside(&d.position) && bypass.get(&d.id) != Some(&pos))
            {
                let item = self.dropped.remove(i);
                self.controller_take(pos, item.stack);
            }
            return;
        }
        // CRUSHING_WHEEL_CONTROLLER_COLLISION: the others wait on it.
        if facing == [0, -1, 0] {
            let floor = lo[1] + WAITING_HEIGHT;
            for d in &mut self.dropped {
                if inside(&d.position) && self.machines.bypass.get(&d.id) != Some(&pos) {
                    if d.position.y < floor {
                        d.position.y = floor;
                    }
                    d.velocity.y = d.velocity.y.max(0.0);
                }
            }
        }
        let res = Arc::clone(&self.res);
        let recipes = res.create_recipes();
        let Some(c) = self.machines.controllers.get_mut(&pos) else {
            return;
        };
        // log2 of one is nought: a single item crushes at the most, as in Create.
        let count = c.slots.first().map_or(1, |s| s.count.max(1)) as f32;
        let divisor = if c.applied { 1.0 } else { count.log2() };
        c.remaining -= (speed / divisor).clamp(0.25, 20.0);
        if c.remaining < 20.0 && !c.applied {
            let input = c.slots.first().cloned();
            c.slots.clear();
            c.applied = true;
            let Some(input) = input else {
                return;
            };
            let Some(recipe) = recipes.crushing(&input.item) else {
                // No recipe: Create's wheels crush it to nothing.
                return;
            };
            let mut out: Vec<ItemStack> = Vec::new();
            for _ in 0..input.count {
                for (id, n, chance) in &recipe.results {
                    if *chance < 1.0 && self.world.random.next_float() >= *chance {
                        continue;
                    }
                    match out.iter_mut().find(|s| s.item == *id && s.count + n <= 64) {
                        Some(s) => s.count += n,
                        None => out.push(ItemStack {
                            item: id.clone(),
                            count: *n,
                            damage: 0,
                            extra: None,
                        }),
                    }
                }
            }
            if let Some(c) = self.machines.controllers.get_mut(&pos) {
                c.slots = out;
            }
            return;
        }
        if c.remaining > 0.0 {
            return;
        }
        c.remaining = 0.0;
        let slots = std::mem::take(&mut c.slots);
        let center = pos.map(|v| f64::from(v) + 0.5);
        let out_pos: [f64; 3] = std::array::from_fn(|i| center[i] + 0.55 * f64::from(facing[i]));
        let out_speed = [
            0.25 * f64::from(facing[0]),
            if facing[1] > 0 { 0.5 } else { 0.0 },
            0.25 * f64::from(facing[2]),
        ];
        for stack in slots {
            self.spawn_item(stack, out_pos, out_speed, 0);
            if let Some(item) = self.dropped.last() {
                self.machines.bypass.insert(item.id, pos);
            }
        }
    }

    /// An item into the controller at `pos` (`intakeItem`, `itemInserted`): its recipe's time
    /// to go, else a hundred ticks.
    fn controller_take(&mut self, pos: [i32; 3], stack: ItemStack) {
        let time = self
            .res
            .create_recipes()
            .crushing(&stack.item)
            .map_or(100.0, |r| r.time);
        if let Some(c) = self.machines.controllers.get_mut(&pos) {
            c.slots = vec![stack];
            c.remaining = time;
            c.applied = false;
        }
    }

    /// Whether a crushing controller is at `pos` (a hopper facing it puts items in it).
    pub(in crate::game) fn crusher_at(&self, pos: [i32; 3]) -> bool {
        self.machines.controllers.contains_key(&pos)
    }

    /// A hopper's item into the crushing controller at `pos` when one is there and empty (its
    /// inventory takes items); true when it took it.
    pub(in crate::game) fn controller_insert(&mut self, pos: [i32; 3], stack: &ItemStack) -> bool {
        let free = self
            .machines
            .controllers
            .get(&pos)
            .is_some_and(|c| c.speed != 0.0 && c.slots.is_empty());
        if free {
            self.controller_take(pos, stack.clone());
        }
        free
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crushing_recipes_read_their_input_results_and_time() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"type": "create:crushing", "ingredients": [{"item": "minecraft:gravel"}],
                "processing_time": 250, "results": [{"id": "minecraft:sand"},
                {"chance": 0.1, "id": "minecraft:flint"}]}"#,
        )
        .unwrap();
        let no_tags = |_: &str| Vec::new();
        let (input, recipe) = Recipe::parse(&v, "crushing", &no_tags).unwrap();
        assert_eq!(input, [Arc::from("minecraft:gravel")]);
        assert_eq!(recipe.time, 250.0);
        assert_eq!(recipe.results.len(), 2);
        assert_eq!(recipe.results[1].2, 0.1);
        // Milling read as milling only, and another mod's compat (with conditions) left out.
        assert!(Recipe::parse(&v, "milling", &no_tags).is_none());
        let compat: serde_json::Value = serde_json::from_str(
            r#"{"type": "create:crushing", "neoforge:conditions": [], "ingredients":
                [{"item": "x:ore"}], "results": [{"id": "x:dust"}]}"#,
        )
        .unwrap();
        assert!(Recipe::parse(&compat, "crushing", &no_tags).is_none());
    }

    #[test]
    fn rims_between_wheels_turning_towards_each_other_go_one_way() {
        // Wheels along z at x 0 and 2: the one at 0 turning the positive way about z moves its
        // rim at the gap (x + 1) along z × x = +y, so the other turns the other way.
        assert_eq!(cross(unit(2, 1), [1, 0, 0]), [0, 1, 0]);
        // From the other wheel, the side is -x and its speed's sign the other: the same way.
        assert_eq!(scale(cross(unit(2, 1), [-1, 0, 0]), -1), [0, 1, 0]);
    }

    #[test]
    fn woods_are_found_as_the_generators_patterns_find_them() {
        assert_eq!(
            stripped_prefix("stripped_oak_log"),
            Some(("", "oak", "_log"))
        );
        assert_eq!(
            stripped_prefix("stripped_dark_oak_wood"),
            Some(("", "dark_oak", "_wood"))
        );
        assert_eq!(
            stripped_prefix("stripped_livingwood"),
            Some(("", "living", "wood"))
        );
        assert_eq!(stripped_prefix("oak_log"), None);
        assert_eq!(
            stripped_suffix("maple_log_stripped"),
            Some(("maple", "_log", String::new()))
        );
        assert_eq!(non_stripped("bone_block"), Some(("bone", "_block")));
        assert_eq!(non_stripped("bioshroom_block"), None);
        assert_eq!(non_stripped("stripped_oak_log"), None);
    }

    #[test]
    fn every_wood_and_concrete_powder_gets_its_recipes() {
        let registry: Vec<Arc<str>> = [
            "oak_log",
            "stripped_oak_log",
            "oak_wood",
            "stripped_oak_wood",
            "oak_planks",
            "oak_slab",
            "oak_stairs",
            "crimson_stem",
            "stripped_crimson_stem",
            "crimson_planks",
            "bamboo_block",
            "stripped_bamboo_block",
            "bamboo_planks",
            "bone_block",
            "red_concrete_powder",
            "red_concrete",
        ]
        .into_iter()
        .map(|p| Arc::from(format!("minecraft:{p}")))
        .collect();
        let known: std::collections::HashSet<&str> = registry.iter().map(|i| &**i).collect();
        let made = generated_recipes(&registry, &|id| known.contains(id));
        let has = |kind: &str, from: &str, to: &str, n: u32| {
            made.iter().any(|(k, f, t, c)| {
                *k == kind
                    && *f == format!("minecraft:{from}")
                    && *t == format!("minecraft:{to}")
                    && *c == n
            })
        };
        assert!(has("cutting", "oak_log", "stripped_oak_log", 1));
        assert!(has("cutting", "stripped_oak_log", "oak_planks", 6));
        assert!(has("cutting", "oak_wood", "stripped_oak_wood", 1));
        assert!(has("cutting", "stripped_oak_wood", "oak_planks", 6));
        assert!(has("cutting", "oak_planks", "oak_slab", 2));
        assert!(has("cutting", "oak_planks", "oak_stairs", 1));
        assert!(has("cutting", "stripped_crimson_stem", "crimson_planks", 6));
        assert!(has("cutting", "stripped_bamboo_block", "bamboo_planks", 3));
        assert!(has("splashing", "red_concrete_powder", "red_concrete", 1));
        // Nothing for a block that is no wood, nor for items not there (oak doors).
        assert!(!made.iter().any(|(_, f, _, _)| f.ends_with("bone_block")));
        assert!(!made.iter().any(|(_, _, t, _)| t.ends_with("oak_door")));
    }
}
