//! Upgrade a copy of a world saved by 1.20 to 26.2 to the 26.3 format, as
//! vanilla 26.3 does when it opens it and reads every file:
//! `datafix_world IN_WORLD OUT_WORLD [TARGET]` (used by tools/test_datafix_parity.py).
//! With a TARGET DataVersion only the data fixes up to it run, on a world in the
//! layout of before 1.16. `--single CASES VANILLA_NBT` checks standalone upgrades.
use mod_api::NbtDataType;
use std::{fs, io, path::Path, process::ExitCode};
use sunrise_core::nbt::{Limits, Tag, parse_snbt, read_named};
use sunrise_world::datafix::{files, upgrade_compound};

fn copy(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy(&entry.path(), &target)?;
        } else if entry.file_name() != "session.lock" {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub fn main() -> ExitCode {
    let args: Vec<String> = crate::args().skip(1).collect();
    if let [flag, cases, expected] = args.as_slice()
        && flag == "--single"
    {
        return match single_compounds(Path::new(cases), Path::new(expected)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("datafix_world --single: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let (input, output, target) = match args.as_slice() {
        [input, output] => (input, output, None),
        [input, output, target] => (input, output, target.parse::<i32>().ok()),
        _ => {
            eprintln!(
                "usage: datafix_world IN_WORLD OUT_WORLD [TARGET] | --single CASES VANILLA_NBT"
            );
            return ExitCode::from(2);
        }
    };
    let output = Path::new(output);
    let result = (|| {
        if output.exists() {
            fs::remove_dir_all(output)?;
        }
        copy(Path::new(input), output)?;
        match target {
            Some(target) => files::upgrade_band(output, target).map(|()| target),
            None => files::upgrade_world(output),
        }
    })();
    match result {
        Ok(version) => {
            println!("upgraded {} from DataVersion {version}", output.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("datafix_world: {e}");
            ExitCode::FAILURE
        }
    }
}

fn single_compounds(cases: &Path, expected: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let expected = read_named(&fs::read(expected)?, Limits::saved())?.0.tag;
    for line in fs::read_to_string(cases)?.lines() {
        let fields: Vec<_> = line.splitn(4, '\t').collect();
        let &[name, kind, version, snbt] = fields.as_slice() else {
            return Err(format!("bad standalone datafix case: {line}").into());
        };
        let kind = match kind {
            "item_stack" => NbtDataType::ItemStack,
            "block_state" => NbtDataType::BlockState,
            "block_entity" => NbtDataType::BlockEntity,
            "entity" => NbtDataType::Entity,
            "entity_tree" => NbtDataType::EntityTree,
            _ => return Err(format!("unknown datafix type: {kind}").into()),
        };
        let (Tag::Compound(tag), _) = parse_snbt(snbt)? else {
            return Err(format!("{name}: expected a compound").into());
        };
        let actual = upgrade_compound(kind, version.parse()?, tag)
            .map_err(|error| format!("{name}: {error}"))?;
        // Compare tags, not rendered JSON: a count written as a byte rather than an int differs.
        assert_eq!(Some(&Tag::Compound(actual)), expected.get(name), "{name}");
    }
    Ok(())
}
