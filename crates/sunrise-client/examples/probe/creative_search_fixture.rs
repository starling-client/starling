//! The creative inventory, for tools/test_creative_parity.py: loads an
//! exported `creative-inventory.json` into `sunrise_client::creative::Creative`
//! and prints the tabs it shows (canonical stacks) and the search tab's
//! results for every query of the vanilla oracle's output; with `permissions`,
//! as an operator with the Operator Items Tab option on.
//!
//! usage: creative_search_fixture RESOURCES CREATIVE_JSON VANILLA_JSON OUTPUT [permissions]
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};
use sunrise_client::{
    creative::{Creative, Names},
    item_data::canon_stack,
    items::Items,
    lang::{self, Language},
};
use sunrise_items::creative_catalog::{Holders, Parameters};

pub fn main() -> sunrise_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage =
        "usage: creative_search_fixture RESOURCES CREATIVE_JSON VANILLA_JSON OUTPUT [permissions]";
    let resources = PathBuf::from(args.get(1).ok_or(usage)?);
    let source = sunrise_assets::resource_source::ResourceSource::directory(resources.clone());
    let names = Names::load(&source);
    let catalog_json = fs::read(args.get(2).ok_or(usage)?)?;
    let mut creative = Creative::from_json(&serde_json::from_slice(&catalog_json)?)?;
    let defaults = sunrise_world::item_defaults::read(&resources)?;
    let items = Arc::new(Items::catalog(
        &resources,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )?);
    let permissions = args.get(5).is_some_and(|a| a == "permissions");
    let vanilla: Value = serde_json::from_slice(&fs::read(args.get(3).ok_or(usage)?)?)?;
    let holders: Holders = serde_json::from_value(vanilla["holders"].clone())?;
    let features: Vec<String> = serde_json::from_value(vanilla["features"].clone())?;
    let lists = creative.catalog().build(
        Parameters {
            features: &features,
            operator: permissions,
            holders: &holders,
            items: &items,
        },
        None,
        |_, _, _| {},
    );
    creative.publish(lists, holders.into(), items.clone(), &names);
    let tabs: Vec<Value> = creative
        .tabs
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "title": t.title,
                "icon": t.icon,
                "row": if t.top { "TOP" } else { "BOTTOM" },
                "column": t.column,
                "items": t.items.iter().map(|s| canon_stack(Some(s), &items)).collect::<Vec<_>>(),
            })
        })
        .collect();
    let english = Language::load(&[&source], "en_us");
    let (searches, _) = lang::scoped(english, || {
        query_results(&creative, &names, &vanilla["searches"])
    });
    let french = Language::load(&[&source], "fr_fr");
    let (french_searches, _) = lang::scoped(french, || {
        query_results(&creative, &names, &vanilla["frenchSearches"])
    });
    let search: Vec<String> = creative
        .search_items()
        .iter()
        .map(|s| canon_stack(Some(s), &items))
        .collect();
    let built = creative
        .built_contents()
        .ok_or("screen did not build its catalog")?;
    let built_tabs: Vec<_> = creative.catalog().tabs().iter().zip(built.iter())
        .map(|(tab, contents)| {
            let canon = |stacks: &[sunrise_client::items::ItemStack]| stacks.iter()
                .map(|stack| canon_stack(Some(stack), &items)).collect::<Vec<_>>();
            json!({"id": tab.id, "items": canon(&contents.display), "search_items": canon(&contents.search)})
        }).collect();
    let out = json!({
        "built_tabs": built_tabs,
        "tabs": tabs,
        "search": search,
        "searches": searches?,
        "frenchSearches": french_searches?,
    });
    fs::write(args.get(4).ok_or(usage)?, out.to_string())?;
    Ok(())
}

fn query_results(
    creative: &Creative,
    names: &Names,
    queries: &Value,
) -> sunrise_client::Result<Vec<Value>> {
    let results = queries
        .as_array()
        .ok_or("searches")?
        .iter()
        .map(|s| {
            let query = s["query"].as_str().unwrap_or_default();
            json!({"query": query, "results": creative.search_results(query, names)})
        })
        .collect();
    Ok(results)
}
