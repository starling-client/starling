//! Furnace parity (tools/test_furnace_parity.py): reads "furnace KIND TICKS
//! INPUT FUEL OUTPUT TIMER TOTAL LIT LITTOTAL SPEED CHANGE_AT NEW_INPUT"
//! lines, runs the furnace model and prints the slots, timers, LIT state,
//! RecipesUsed counts and each used recipe's experience.
//!
//! The first input line is `#resources RESOURCES`; usage: furnace_fixture INPUT OUTPUT
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    fs,
};
use sunrise_client::{
    advancements,
    crafting::Recipes,
    furnace::{FurnaceConditions, FurnaceCooking},
    item_data::{canon_stack, parse_stack},
    items::Items,
    save::block_entity_encode::{DataFields, data_with},
};
use sunrise_client_render::{
    chunk::block_entity::{BlockEntity, Furnace, FurnaceKind},
    providers::{Evaluation, Providers},
};
use sunrise_core::{
    nbt::Tag,
    random::{LegacyRandom, LevelRandom, RandomSource},
};
use sunrise_world::{
    chunk::BlockState,
    loot::{PackOverlay, load_predicates},
};

struct FixtureContext<'a> {
    state: &'a BlockState,
    furnace: &'a Furnace,
    items: &'a Items,
    predicates: &'a HashMap<String, serde_json::Value>,
}

impl advancements::World for FixtureContext<'_> {
    fn predicate(&self, id: &str) -> Option<&serde_json::Value> {
        self.predicates.get(id)
    }
    fn block_state(&self, _: [i32; 3]) -> Option<&BlockState> {
        Some(self.state)
    }
    fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
        (
            self.state.name.clone(),
            self.state
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }
    fn block_entity_nbt(&self, _: [i32; 3]) -> Option<Tag> {
        Some(data_with(
            (0, 0, 0),
            &BlockEntity::Furnace(self.furnace.clone()),
            &self.state.name,
            DataFields::default(),
            self.items,
            |_| None,
        ))
    }
    fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool {
        registry == "block"
            && self
                .items
                .block_tags
                .get(tag)
                .is_some_and(|set| set.contains(id))
    }
    fn dimension(&self) -> &str {
        "minecraft:overworld"
    }
    fn weather(&self) -> (bool, bool) {
        (false, false)
    }
    fn biome(&self, _: [i32; 3]) -> String {
        "minecraft:plains".into()
    }
    fn structures(&self, _: [i32; 3]) -> Vec<String> {
        Vec::new()
    }
    fn fluid(&self, _: [i32; 3]) -> Option<String> {
        None
    }
    fn can_see_sky(&self, _: [i32; 3]) -> bool {
        false
    }
    fn light(&self, _: [i32; 3]) -> i32 {
        0
    }
    fn smokey(&self, _: [i32; 3]) -> bool {
        false
    }
}

pub fn main() -> sunrise_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = fs::read_to_string(args.get(1).ok_or("missing INPUT")?)?;
    let root = input
        .lines()
        .find_map(|line| line.strip_prefix("#resources "))
        .map(std::path::PathBuf::from)
        .ok_or("no #resources line")?;
    let defaults = sunrise_world::item_defaults::read(&root)?;
    let items = Items::catalog(&root, &PackOverlay::NONE, &defaults)?;
    let recipes = Recipes::load(
        &root,
        &items,
        &PackOverlay::NONE,
        &crate::test_resources::server_load(),
    )?;
    let providers = Providers::load(&root);
    let predicates = load_predicates(&root, &PackOverlay::NONE)?;
    let fixture_providers =
        Providers::load(&std::path::Path::new(&args[1]).with_file_name("fixture-pack"));
    let mut out = String::new();
    for (n, line) in input
        .lines()
        .filter(|line| !line.starts_with('#'))
        .enumerate()
    {
        let f: Vec<&str> = line.split(' ').collect();
        let kind = match f[1] {
            "furnace" => FurnaceKind::Furnace,
            "blast_furnace" => FurnaceKind::BlastFurnace,
            "smoker" => FurnaceKind::Smoker,
            _ => return Err("unknown furnace kind".into()),
        };
        let providers = if f.get(13) == Some(&"fixture") {
            &fixture_providers
        } else {
            &providers
        };
        let ticks: u32 = f[2].parse()?;
        let mut items3 = [None, None, None];
        for i in 0..3 {
            items3[i] = parse_stack(f[3 + i])?;
        }
        let mut furnace = Furnace {
            kind,
            items: items3,
            lit_remaining: f[8].parse()?,
            lit_total: f[9].parse()?,
            cooking_timer: f[6].parse()?,
            cooking_total: f[7].parse()?,
            speed: f[10].parse()?,
            recipes_used: HashMap::new(),
        };
        let change_at: i32 = f[11].parse()?;
        let new_input = parse_stack(f[12])?;
        let mut lit_state = false;
        let mut state = BlockState {
            name: format!("minecraft:{}", f[1]),
            properties: Default::default(),
        };
        state.properties.insert("facing".into(), "north".into());
        state.properties.insert("lit".into(), "false".into());
        let mut random = LevelRandom::for_level(LegacyRandom::new(0));
        let mut drops = Vec::new();
        for t in 0..ticks {
            if t as i32 == change_at {
                furnace.set_input(
                    crate::test_resources::server_loop_context(),
                    &recipes,
                    new_input.clone(),
                );
            }
            let was_lit = furnace.count_down_burn();
            let plan = furnace.prepare_tick(
                crate::test_resources::server_loop_context(),
                &recipes,
                &items,
                was_lit,
                |fuel| {
                    let world = FixtureContext {
                        state: &state,
                        furnace: &furnace,
                        items: &items,
                        predicates: &predicates,
                    };
                    Evaluation::new(providers, &mut random).cooking_fuel(
                        fuel,
                        &mut FurnaceConditions {
                            world: &world,
                            pos: [0; 3],
                        },
                    )
                },
            );
            if let Some(lit) = plan
                .apply(&mut furnace, &mut drops, |s| {
                    sunrise_client::crafting::CraftingRemainder::crafting_remainder(&items, s)
                })
                .lit
            {
                lit_state = lit;
                state.properties.insert("lit".into(), lit.to_string());
            }
        }
        if change_at == ticks as i32 {
            furnace.set_input(
                crate::test_resources::server_loop_context(),
                &recipes,
                new_input.clone(),
            );
        }
        write!(out, "{n} kind={} ", f[1])?;
        for i in 0..3 {
            write!(
                out,
                "s{i}={} ",
                canon_stack(furnace.items[i].as_ref(), &items)
            )?;
        }
        write!(
            out,
            "timer={} total={} lit={} littotal={} speed={:08x} state={lit_state} used=",
            furnace.cooking_timer,
            furnace.cooking_total,
            furnace.lit_remaining,
            furnace.lit_total,
            furnace.speed.to_bits()
        )?;
        let mut ids: Vec<&String> = furnace.recipes_used.keys().collect();
        ids.sort();
        for (i, id) in ids.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write!(out, "{id}={}", furnace.recipes_used[*id])?;
        }
        out.push_str(" xp=");
        for (i, id) in ids.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write!(out, "{id}={:08x}", recipes.cooking_experience(id).to_bits())?;
        }
        if f.get(14) == Some(&"rng") {
            write!(out, " rng={}", random.next_int())?;
        }
        writeln!(out)?;
    }
    fs::write(args.get(2).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
