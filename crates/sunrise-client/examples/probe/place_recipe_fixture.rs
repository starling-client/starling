//! Recipe book placements for tools/test_place_recipe_parity.py: reads one
//! JSON case per line (menu, inventory, grid, recipe, max, creative,
//! selected) and the oracle's order of the counted items, runs
//! `Menu::place_recipe` and prints the oracle's line.
//!
//! usage: place_recipe_fixture RESOURCES CASES ORDERS OUTPUT
use serde_json::Value;
use std::{fs, path::Path};
use sunrise_client::{
    crafting::Recipes,
    item_data::{canon_stack, parse_stack},
    items::Items,
    menu::{Env, Kind, Menu, place::PostPlace},
};
use sunrise_world::loot::PackOverlay;

fn state(menu: &Menu, catalog: &Items) -> String {
    let slots: Vec<String> = (0..menu.slot_count())
        .map(|i| canon_stack(menu.slot(i), catalog))
        .collect();
    let data: String = menu.data.iter().map(|d| format!(" {d}")).collect();
    let inv: Vec<String> = (0..41)
        .map(|i| canon_stack(menu.player_item(i), catalog))
        .collect();
    format!(
        "{} | {} |{data} | {} | inv {}",
        slots.join(" "),
        canon_stack(menu.carried.as_ref(), catalog),
        menu.drops.join(";"),
        inv.join(" ")
    )
}

pub fn main() -> sunrise_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: place_recipe_fixture RESOURCES CASES ORDERS OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let orders = fs::read_to_string(args.get(3).ok_or(usage)?)?;
    let defaults = sunrise_world::item_defaults::read(root)?;
    let items = Items::catalog(root, &PackOverlay::NONE, &defaults)?;
    let recipes = Recipes::load(
        root,
        &items,
        &PackOverlay::NONE,
        &crate::test_resources::server_load(),
    )?;
    let brewing = sunrise_client::brewing::Brewing::load(root, &items, &PackOverlay::NONE);
    let banners = sunrise_client::banner::Banners::default();
    let station_recipes = sunrise_client::stations::StationRecipes::default();
    let enchantments = sunrise_client::enchant::Enchantments::default();
    let names = sunrise_client::creative::Names::default();
    let env = Env {
        crafting_remainder: None,
        player: None,
        player_entity_id: None,
        loop_context: &crate::test_resources::loop_context(
            sunrise_world::native_load_context::LoopSide::Server,
        ),
        items: &items,
        recipes: &recipes,
        brewing: &brewing,
        banners: &banners,
        station_recipes: &station_recipes,
        enchantments: &enchantments,
        names: &names,
        trade: None,
        maps: None,
        limited_crafting: false,
        can_equip: None,
        known_recipes: None,
        slot_rules: None,
        stacked: None,
    };
    let mut out = String::new();
    for (line, order) in cases.lines().zip(orders.lines()) {
        let case: Value = serde_json::from_str(line)?;
        let kind = Kind::parse(case["menu"].as_str().unwrap_or_default()).ok_or("menu kind")?;
        let mut menu = Menu::new(kind);
        menu.creative = case["creative"].as_bool().unwrap_or(false);
        menu.selected = case["selected"].as_u64().unwrap_or(0) as usize;
        for pair in case["inventory"].as_array().ok_or("inventory")? {
            menu.set_player_item(
                pair[0].as_u64().unwrap_or(0) as usize,
                parse_stack(pair[1].as_str().unwrap_or("-"))?,
            );
        }
        for pair in case["grid"].as_array().ok_or("grid")? {
            menu.set_slot(
                &env,
                pair[0].as_u64().unwrap_or(0) as usize,
                parse_stack(pair[1].as_str().unwrap_or("-"))?,
            );
        }
        menu.drops.clear();
        let order: Vec<String> = order.split_whitespace().map(str::to_owned).collect();
        let recipe = case["recipe"].as_str().unwrap_or_default();
        let impossible = recipes.placement(recipe).is_none_or(|p| p.impossible());
        let post = if impossible {
            "-"
        } else {
            match menu.place_recipe(
                &env,
                recipe,
                case["max"].as_bool().unwrap_or(false),
                menu.creative,
                Some(&order),
            ) {
                PostPlace::Nothing => "NOTHING",
                PostPlace::Ghost => "PLACE_GHOST_RECIPE",
            }
        };
        out += &format!("post {post} | {}\n", state(&menu, &items));
    }
    fs::write(args.get(4).ok_or(usage)?, out)?;
    Ok(())
}
