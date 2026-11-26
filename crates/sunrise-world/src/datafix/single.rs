//! The world's datafix chain applied to one saved compound, without a chunk or level.
use super::{CURRENT, Ty, dynamic, identifier, key, schema, source_schema_key, tables, try_update};
use mod_api::{NbtDataType, NbtUpgradeError};
use sunrise_core::nbt::{Compound, JavaString, Tag};

/// The deepest nesting accepted: a list or compound at depth `MAX_DEPTH` (the root compound is at 0) is refused.
/// The depth cap of every NBT read the game makes (`nbt::Limits`, vanilla's `NbtAccounter` default of 512), counted
/// on the encoded form: the compound the NBT writer puts around an element of a mixed list is a level of its own,
/// so such an element sits two levels below its list. There is no size cap: vanilla puts no quota on saved data
/// and never limits fixing or writing it.
pub const MAX_DEPTH: usize = 512;

/// `DataFixer.update` at one vanilla root reference. No world context is added or used.
pub fn upgrade_compound(
    kind: NbtDataType,
    data_version: i32,
    tag: Compound,
) -> Result<Compound, NbtUpgradeError> {
    let refusal = if data_version > CURRENT {
        Some(NbtUpgradeError::FutureVersion(data_version))
    } else {
        (!within_depth(&tag)).then_some(NbtUpgradeError::TooDeep)
    };
    if let Some(error) = refusal {
        dismantle(Tag::Compound(tag));
        return Err(error);
    }
    let root = match kind {
        NbtDataType::ItemStack => Ty::ItemStack,
        NbtDataType::BlockState => Ty::BlockState,
        NbtDataType::BlockEntity => Ty::BlockEntity,
        NbtDataType::Entity => Ty::Entity,
        NbtDataType::EntityTree => Ty::EntityTree,
    };
    let from = source_schema_key(data_version);
    // BlockStateFieldNamesFix (V5006) renamed `Name` to `id`.
    let discriminator = if root == Ty::BlockState && from < key(5006, 0) {
        "Name"
    } else {
        "id"
    };
    let field = tag.get(&JavaString::from(discriminator));
    let name = match field {
        Some(Tag::String(name)) if !name.0.is_empty() => Some(name),
        // V99's item name is a number or a string. `ItemIdFix` removes the numeric form in V102.
        Some(id)
            if root == Ty::ItemStack && from < key(102, 0) && dynamic::as_number(id).is_some() =>
        {
            None
        }
        _ => return Err(NbtUpgradeError::MalformedCompound),
    };
    let tagged_choice = matches!(root, Ty::Entity | Ty::EntityTree | Ty::BlockEntity);
    // The entity choices do not read mod namespaces. Item names are not a tagged choice:
    // the item fixers keep the id but still update shared stack fields.
    if data_version == CURRENT
        || tagged_choice
            && name.is_some_and(|name| {
                identifier(&name.to_string_lossy()).is_some_and(|(ns, _)| ns != "minecraft")
            })
    {
        return Ok(tag);
    }
    let mut value = Tag::Compound(tag);
    let mut prefix = None;
    let mut run = || -> tables::Result {
        if root == Ty::BlockEntity && !schema::block_entity_known(&value, from)?
            || matches!(root, Ty::Entity | Ty::EntityTree)
                && from < schema::flat::END
                && !schema::flat::entity_tree_known(&value, from)?
        {
            return Ok(());
        }
        // `Type.readAndWrite` lays the fixed keys over the source codec's remainder.
        // Tagged-choice roots leave the original compound there; the other roots don't.
        if tagged_choice {
            prefix = Some(value.clone());
        }
        try_update(root, &mut value, data_version, CURRENT)
    };
    if run().is_err() {
        dismantle(value);
        return Err(NbtUpgradeError::LegacyTablesUnavailable);
    }
    let Tag::Compound(mut upgraded) = value else {
        dismantle(value);
        return Err(NbtUpgradeError::MalformedCompound);
    };
    if !within_depth(&upgraded) {
        dismantle(Tag::Compound(upgraded));
        return Err(NbtUpgradeError::UpgradedTooDeep);
    }
    if let Some(Tag::Compound(mut prefix)) = prefix {
        for (key, tag) in upgraded {
            prefix.insert(key, tag);
        }
        upgraded = prefix;
    }
    Ok(upgraded)
}

/// Whether no list or compound of `tag` sits [`MAX_DEPTH`] levels deep or more as the NBT writer encodes it, walked
/// without recursion. A list written as a list of compounds (all compounds, or mixed types) puts each element that
/// is no compound, or is a compound that looks like a wrapper (one entry with an empty name), in a compound of its
/// own: that wrapper is a level, as it is on disk and on the wire, and the element sits below it.
fn within_depth(tag: &Compound) -> bool {
    let mut stack: Vec<(&Tag, usize)> = tag.values().map(|value| (value, 1)).collect();
    while let Some((value, depth)) = stack.pop() {
        match value {
            Tag::List(items) => {
                if depth >= MAX_DEPTH {
                    return false;
                }
                let first = items.first().map_or(0, Tag::id);
                let compounds = first == 10 || items.iter().any(|item| item.id() != first);
                for item in items {
                    let wrapper = matches!(item, Tag::Compound(c)
                        if c.len() == 1 && c.contains_key(&JavaString::default()));
                    if compounds && (item.id() != 10 || wrapper) {
                        // The wrapper at `depth + 1`, the element inside it at `depth + 2`.
                        if depth + 1 >= MAX_DEPTH {
                            return false;
                        }
                        stack.push((item, depth + 2));
                    } else {
                        stack.push((item, depth + 1));
                    }
                }
            }
            Tag::Compound(entries) => {
                if depth >= MAX_DEPTH {
                    return false;
                }
                stack.extend(entries.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    true
}

/// Drops `tag` without recursion: every nested list and compound is emptied into a work list first.
fn dismantle(tag: Tag) {
    let mut pending = vec![tag];
    while let Some(value) = pending.pop() {
        match value {
            Tag::List(items) => pending.extend(items),
            Tag::Compound(entries) => pending.extend(entries.into_values()),
            _ => {}
        }
    }
}
