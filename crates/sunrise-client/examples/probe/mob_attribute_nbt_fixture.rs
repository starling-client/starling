//! Loads, equips and saves entities in the format of `VanillaMobAttributeNbtOracle`
//! (tools/test_mob_attribute_nbt_parity.py): the attribute values and bases asked
//! for, and the saved `attributes` list as exact double bits.
//!
//! usage: mob_attribute_nbt_fixture RESOURCES CASES OUTPUT
use serde_json::Value;
use std::{fmt::Write as _, fs, path::Path, sync::Arc};
use sunrise_client::{
    entity_nbt,
    items::{ItemStack, Items},
    living_attributes::LivingAttributes,
    mobs::MobTypes,
};
use sunrise_items::equipment::{Equipment, SLOT_NAMES};
use {
    sunrise_core::nbt::Tag,
    sunrise_world::{attribute_map::ValueKind, attributes::entity::Catalog, loot::PackOverlay},
};

fn slot(name: &str) -> usize {
    SLOT_NAMES.iter().position(|n| *n == name).unwrap()
}

fn text(tag: Option<&Tag>) -> String {
    match tag {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => "?".into(),
    }
}

fn bits(tag: Option<&Tag>) -> String {
    match tag {
        Some(Tag::Double(b)) => format!("{b:x}"),
        _ => "?".into(),
    }
}

/// `{prefix} id base ... mod id amount operation ...` per attribute, sorted by id.
fn saved(tag: &Tag, prefix: &str) -> String {
    let Some(Tag::List(list)) = tag.get("attributes") else {
        return String::new();
    };
    let mut rows: Vec<(String, String)> = list
        .iter()
        .map(|a| {
            let mut row = format!("base {}", bits(a.get("base")));
            if let Some(Tag::List(mods)) = a.get("modifiers") {
                for m in mods {
                    let _ = write!(
                        row,
                        " mod {} {} {}",
                        text(m.get("id")),
                        bits(m.get("amount")),
                        text(m.get("operation"))
                    );
                }
            }
            (text(a.get("id")), row)
        })
        .collect();
    rows.sort();
    rows.iter().fold(String::new(), |out, (id, row)| {
        format!("{out}\t{prefix} {id} {row}")
    })
}

fn mob_seed(mob: &sunrise_client::mobs::Mob) -> Vec<Tag> {
    entity_nbt::attribute_entries(mob)
        .into_iter()
        .map(|(_, e)| e)
        .collect()
}

fn append_reads(row: &mut String, prefix: &str, names: &[Value], reads: Vec<Option<(f64, f64)>>) {
    for (n, r) in names.iter().zip(reads) {
        let n = n.as_str().unwrap_or_default();
        match r {
            Some((v, b)) => {
                let _ = write!(row, "\t{prefix} {n} {:x} {:x}", v.to_bits(), b.to_bits());
            }
            None => {
                let _ = write!(row, "\t{prefix} {n} none");
            }
        }
    }
}

fn stack(item: &Value) -> Option<ItemStack> {
    item.as_str()
        .filter(|s| !s.is_empty())
        .map(|s| ItemStack::new(s, 1))
}

pub fn main() -> sunrise_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: mob_attribute_nbt_fixture RESOURCES CASES OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let defaults = sunrise_world::item_defaults::read(root)?;
    let items = Items::catalog(root, &PackOverlay::NONE, &defaults)?;
    let types = MobTypes::load(root)?;
    let mobs: std::collections::HashMap<_, _> = types
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_slice(&fs::read(
        root.join("attributes.json"),
    )?)?)?);
    let mut out = String::new();
    for line in cases.lines().filter(|l| !l.trim().is_empty()) {
        let case: Value = serde_json::from_str(line)?;
        let name = case["name"].as_str().unwrap_or_default();
        let entity = case["entity"].as_str().unwrap_or_default();
        let (tag, _) = sunrise_core::nbt::parse_snbt(case["snbt"].as_str().unwrap_or_default())?;
        let passes = [&case["equip"], &case["then"]];
        let mut row = name.to_owned();
        // Equips the slots a pass names, then runs the equipment pass on `live`.
        let run = |live: &mut LivingAttributes,
                   equipment: &mut Equipment,
                   seed: &dyn Fn() -> Vec<Tag>| {
            for pass in passes {
                for (slot_name, item) in pass.as_object().into_iter().flatten() {
                    equipment.set(slot(slot_name), stack(item));
                }
                live.collect_equipment_changes(&catalog, entity, equipment, &items, seed)
                    .unwrap();
            }
        };
        let read_names: &[Value] = case["read"].as_array().map_or(&[], Vec::as_slice);
        let (reads, health, saved_tag) = if entity == "minecraft:armor_stand" {
            let mut stand = entity_nbt::armor_stand_from_nbt(&tag, 1).ok_or("stand")?;
            let mut live = std::mem::take(&mut stand.live);
            run(&mut live, &mut stand.equipment, &Vec::new);
            stand.live = live;
            let reads: Vec<Option<(f64, f64)>> = read_names
                .iter()
                .map(|n| read(&stand.live, &catalog, entity, n, Vec::new()))
                .collect();
            let default_max = catalog
                .id("minecraft:max_health")
                .and_then(|id| catalog.default_base(entity, id))
                .unwrap_or(20.0);
            let max_health = stand
                .live
                .read(
                    &catalog,
                    entity,
                    "minecraft:max_health",
                    ValueKind::Computed,
                    Vec::new,
                )
                .unwrap_or(default_max) as f32;
            (
                reads,
                (stand.health, max_health, default_max),
                entity_nbt::armor_stand_to_nbt(&stand, [0; 4], None, &Default::default()),
            )
        } else {
            let mut mob = entity_nbt::mob_from_nbt(&tag, &mobs, 1, None).ok_or("mob")?;
            let mut live = std::mem::take(&mut mob.extra.live);
            let mut equipment = std::mem::take(&mut mob.extra.equipment);
            run(&mut live, &mut equipment, &|| mob_seed(&mob));
            mob.extra.live = live;
            mob.extra.equipment = equipment;
            let reads: Vec<Option<(f64, f64)>> = read_names
                .iter()
                .map(|n| read(&mob.extra.live, &catalog, entity, n, mob_seed(&mob)))
                .collect();
            let default_max = mob
                .kind
                .attributes
                .get("minecraft:max_health")
                .copied()
                .unwrap_or_else(|| f64::from(mob.kind.max_health));
            (
                reads,
                (mob.health, mob.attrs.max_health, default_max),
                entity_nbt::mob_to_nbt(&mob, [0; 4], None, None, |_| None, &Default::default()),
            )
        };
        append_reads(&mut row, "read", read_names, reads);
        if case["health"].as_bool().unwrap_or(false) {
            let (loaded_health, loaded_max, default_max) = health;
            let saved_health = match saved_tag.get("Health") {
                Some(Tag::Float(bits)) => *bits,
                _ => f32::NAN.to_bits(),
            };
            let _ = write!(
                row,
                "\thealth {:08x} {:08x} {:08x} {:x}",
                loaded_health.to_bits(),
                loaded_max.to_bits(),
                saved_health,
                default_max.to_bits(),
            );
        }
        row.push_str(&saved(&saved_tag, "saved"));
        if case["reload"].as_bool().unwrap_or(false) {
            let (reload_reads, reload_saved) = if entity == "minecraft:armor_stand" {
                let stand = entity_nbt::armor_stand_from_nbt(&saved_tag, 1).ok_or("stand")?;
                let reads: Vec<Option<(f64, f64)>> = read_names
                    .iter()
                    .map(|n| read(&stand.live, &catalog, entity, n, Vec::new()))
                    .collect();
                (
                    reads,
                    entity_nbt::armor_stand_to_nbt(&stand, [0; 4], None, &Default::default()),
                )
            } else {
                let mob = entity_nbt::mob_from_nbt(&saved_tag, &mobs, 1, None).ok_or("mob")?;
                let reads: Vec<Option<(f64, f64)>> = read_names
                    .iter()
                    .map(|n| read(&mob.extra.live, &catalog, entity, n, mob_seed(&mob)))
                    .collect();
                (
                    reads,
                    entity_nbt::mob_to_nbt(&mob, [0; 4], None, None, |_| None, &Default::default()),
                )
            };
            append_reads(&mut row, "reload_read", read_names, reload_reads);
            row.push_str(&saved(&reload_saved, "reload_saved"));
        }
        out.push_str(&row);
        out.push('\n');
    }
    fs::write(args.get(3).ok_or(usage)?, out)?;
    Ok(())
}

fn read(
    live: &LivingAttributes,
    catalog: &Arc<Catalog>,
    entity: &str,
    name: &Value,
    seed: Vec<Tag>,
) -> Option<(f64, f64)> {
    let name = name.as_str()?;
    let value = live.read(catalog, entity, name, ValueKind::Computed, || seed.clone())?;
    let base = live.read(catalog, entity, name, ValueKind::Base, || seed)?;
    Some((value, base))
}
