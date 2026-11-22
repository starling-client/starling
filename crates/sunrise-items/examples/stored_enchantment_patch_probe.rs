//! Saved component patches; input is label, command argument and the equivalent stack SNBT.
//! The items are the prepared resources' (args[3]), so each item's component prototype
//! decides what the saved patch leaves out, as the game's.
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};
use sunrise_items::{
    blocks::BlockFacts,
    item_data::{canon_stack, parse_stack},
    items::Items,
};
use sunrise_world::{loot::PackOverlay, registry::BlockRegistry};

fn main() -> sunrise_items::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(&args[3]);
    let (registry, _) =
        BlockRegistry::from_facts_json_with_flags(File::open(root.join("facts.json"))?)?;
    let facts = BlockFacts::load(root, &registry)?;
    let defaults = sunrise_world::item_defaults::read(root)?;
    let items = Items::load(root, &registry, &facts, &PackOverlay::NONE, &defaults)?;
    let mut output = BufWriter::new(File::create(&args[2])?);
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let line = line?;
        let fields: Vec<_> = line.splitn(3, '\t').collect();
        if fields.len() != 3 {
            return Err("expected label, command argument and saved stack".into());
        }
        let stack = parse_stack(fields[2])?.ok_or("expected a nonempty stack")?;
        writeln!(
            output,
            "{}\t{}",
            fields[0],
            canon_stack(Some(&stack), &items)
        )?;
    }
    Ok(())
}
