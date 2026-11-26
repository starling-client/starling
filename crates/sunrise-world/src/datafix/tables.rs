//! The lookup tables of the 1.13-era data fixers (DataVersion 99 to 1631) and the
//! `BlockStateData` table. They are vanilla's data, so they are not in this repository:
//! `sunrise-prepare` exports them from the player's own server jar
//! (`VanillaDataFixTablesExporter`) into `datafix-tables.json` in the prepared resources.
//! The first lookup reads the file; each table's entries are decoded only when needed.
//!
//! Maps and sets come sorted by key: every use here is a lookup by key or a membership
//! test, so the order is not part of their meaning. `block_state_data` keeps the order of
//! vanilla's `register` calls (see `block_state_data.rs`).

use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// The file in the prepared resources.
pub const FILE: &str = "datafix-tables.json";

type Pairs = &'static [(&'static str, &'static str)];
type Strs = &'static [&'static str];
type IntNames = &'static [(i32, &'static str)];

type BlockIds = &'static [(&'static str, i32)];
type StructureTemplateRenames = &'static [(&'static str, &'static str, Pairs)];
/// The file's tables by name, each row list kept raw until its table is first used.
struct RawTables(BTreeMap<String, Box<RawValue>>);

/// The top level as the struct this map replaced read it (`deny_unknown_fields` and serde's
/// duplicate-field error): every key one of [`NAMES`], uppercased, at most once. No row is
/// decoded here.
impl<'de> serde::Deserialize<'de> for RawTables {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = RawTables;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("the datafix tables by name")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<RawTables, A::Error> {
                use serde::de::Error;
                let mut tables = BTreeMap::new();
                while let Some(name) = map.next_key::<String>()? {
                    if !NAMES.iter().any(|known| known.to_ascii_uppercase() == name) {
                        return Err(A::Error::custom(format!("unknown table {name}")));
                    }
                    if tables.contains_key(&name) {
                        return Err(A::Error::custom(format!("duplicate table {name}")));
                    }
                    let rows = map.next_value::<Box<RawValue>>()?;
                    tables.insert(name, rows);
                }
                Ok(RawTables(tables))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

/// A legacy table could not be loaded or decoded. The cause can be shared by
/// every upgrade that encounters the same cached error.
#[derive(Clone, Debug)]
pub(super) struct Error(Arc<io::Error>);

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self(Arc::new(error))
    }
}

pub(super) type Result<T = ()> = std::result::Result<T, Error>;

// Keep one cache per table so a malformed row fails only lookups of its own table.
macro_rules! legacy_tables {
    ($($(#[$doc:meta])* $name:ident: $ty:ty = $convert:ident;)*) => {
        /// Every table's name, as the file writes it uppercased.
        const NAMES: &[&str] = &[$(stringify!($name)),*];
        $(
            $(#[$doc])*
            pub(super) fn $name() -> Result<$ty> {
                static TABLE: OnceLock<Result<$ty>> = OnceLock::new();
                if let Some(table) = TABLE.get() {
                    return table.clone();
                }
                // A file-level failure must not become a cached per-table error.
                let raw = try_raw_tables()?;
                TABLE.get_or_init(|| {
                    let name = stringify!($name).to_ascii_uppercase();
                    Ok($convert(decode(raw, &name)?))
                }).clone()
            }
        )*
    };
}

legacy_tables! {
    /// `ItemStackTheFlatteningFix.MAP`: `id.damage` to the flattened item.
    item_flattening: Pairs = pairs;
    /// `ItemStackTheFlatteningFix.DAMAGE_IDS`: items that keep their damage in `tag.Damage`.
    item_damage_ids: Strs = strs;
    /// `EntityBlockStateFix.MAP`: 1.12 block name to numeric id.
    block_ids: BlockIds = leak_block_ids;
    /// `ItemIdFix.ITEM_NAMES`: numeric item id to name.
    item_ids: IntNames = int_names;
    /// `StatsCounterFix`'s tables.
    special_objective_criteria: Strs = strs;
    stats_skip: Strs = strs;
    stats_custom: Pairs = pairs;
    stats_item_keys: Pairs = pairs;
    stats_entity_keys: Pairs = pairs;
    stats_entities: Pairs = pairs;
    /// `RecipesFix.RECIPES`, `RecipesRenameningFix.RECIPES`.
    recipes: Pairs = pairs;
    recipes_renamening: Pairs = pairs;
    /// `AdvancementsFix.RENAMES`.
    advancements: Pairs = pairs;
    /// `LevelDataGeneratorOptionsFix.MAP`.
    generator_biomes: Pairs = pairs;
    /// `ItemStackSpawnEggFix.MAP`.
    spawn_eggs: Pairs = pairs;
    /// `ItemStackEnchantmentNamesFix.MAP`.
    enchantments: IntNames = int_names;
    /// `RenamedCoralFix.RENAMED_IDS`, `RenamedCoralFansFix.RENAMED_IDS`.
    coral: Pairs = pairs;
    coral_fans: Pairs = pairs;
    /// `EntityTheRenameningFix.RENAMED_IDS`, `RENAMED_BLOCKS`.
    the_renamening_entities: Pairs = pairs;
    the_renamening_blocks: Pairs = pairs;
    /// `EntityCodSalmonFix.RENAMED_IDS`, `RENAMED_EGG_IDS`.
    cod_salmon: Pairs = pairs;
    cod_salmon_eggs: Pairs = pairs;
    /// The entity ids V1460 registers (V705 has the same ones).
    entities_v1460: Strs = strs;
    /// The entity and block entity ids V99 registers.
    entities_v99: Strs = strs;
    block_entities_v99: Strs = strs;
    /// `ItemSpawnEggFix.ID_TO_ENTITY`: spawn egg damage to entity id.
    spawn_egg_entities: IntNames = int_names;
    /// `EntityIdFix.ID_MAP`, `BlockEntityIdFix.ID_MAP`.
    entity_ids: Pairs = pairs;
    block_entity_ids: Pairs = pairs;
    /// `V99.ITEM_TO_BLOCKENTITY`.
    item_to_block_entity_v99: Pairs = pairs;
    /// `ItemPotionFix.POTIONS`: damage & 127 to potion (absent: null).
    item_potions: IntNames = int_names;
    /// `ChunkStructuresTemplateRenameFix.RENAMES`: structure id, child id, template renames.
    structure_template_renames: StructureTemplateRenames = leak_structure_renames;
    /// `BlockStateData`: one line per `register` call, in call order (block_state_data.rs).
    block_state_data: Strs = strs;
}

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Read the tables from the prepared resources at `root` (the game's `--resources`). Without
/// it, `SUNRISE_RESOURCES`, then the checkout's `.local/sunrise-client/<version>` (tests and
/// probes). Only the first call counts, and only before the tables are first read.
pub fn set_resources(root: &Path) {
    let _ = ROOT.set(root.to_path_buf());
}

fn path() -> PathBuf {
    ROOT.get()
        .cloned()
        .or_else(|| std::env::var_os("SUNRISE_RESOURCES").map(PathBuf::from))
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/sunrise-client")
                .join(sunrise_core::version::VANILLA_VERSION)
        })
        .join(FILE)
}

/// Whether the prepared tables are there (tests skip without them).
pub fn available() -> bool {
    path().is_file()
}

fn text(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn pairs(v: Vec<(String, String)>) -> Pairs {
    v.into_iter()
        .map(|(a, b)| (text(a), text(b)))
        .collect::<Vec<_>>()
        .leak()
}

fn strs(v: Vec<String>) -> Strs {
    v.into_iter().map(text).collect::<Vec<_>>().leak()
}

fn int_names(v: Vec<(i32, String)>) -> IntNames {
    v.into_iter()
        .map(|(i, s)| (i, text(s)))
        .collect::<Vec<_>>()
        .leak()
}

fn leak_block_ids(rows: Vec<(String, i32)>) -> BlockIds {
    rows.into_iter()
        .map(|(name, id)| (text(name), id))
        .collect::<Vec<_>>()
        .leak()
}

fn leak_structure_renames(
    rows: Vec<(String, String, Vec<(String, String)>)>,
) -> StructureTemplateRenames {
    rows.into_iter()
        .map(|(structure, child, renames)| (text(structure), text(child), pairs(renames)))
        .collect::<Vec<_>>()
        .leak()
}

fn decode<T: DeserializeOwned>(raw: &RawTables, name: &str) -> io::Result<T> {
    let value = raw.0.get(name).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, format!("missing table {name}"))
    })?;
    serde_json::from_str(value.get()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid table {name}: {error}"),
        )
    })
}

/// Read the file on demand, indexing raw JSON values without decoding any table's rows.
/// A file-level failure does not publish a cache entry.
fn try_raw_tables() -> io::Result<&'static RawTables> {
    static TABLES: OnceLock<RawTables> = OnceLock::new();
    static LOADING: Mutex<()> = Mutex::new(());
    if let Some(tables) = TABLES.get() {
        return Ok(tables);
    }
    let _loading = LOADING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(tables) = TABLES.get() {
        return Ok(tables);
    }
    let tables = crate::json_file::read(&path())?;
    Ok(TABLES.get_or_init(|| tables))
}

/// World loads keep their missing-resource diagnostic; the standalone API
/// returns a typed refusal instead.
pub(super) fn required<T>(result: Result<T>) -> T {
    result.unwrap_or_else(|error| {
        panic!(
            "{} ({}): this upgrade needs the data fixers' tables of the prepared resources; \
         prepare them again (sunrise-prepare --prepare)",
            error.0,
            path().display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::NbtUpgradeError;

    #[test]
    #[ignore = "daily"]
    fn standalone_upgrade_only_refuses_at_a_failed_table_lookup() {
        use crate::datafix::{dynamic, upgrade_compound};
        use mod_api::NbtDataType;
        use sunrise_core::nbt::{Compound, Tag};

        const CHILD: &str = "SUNRISE_DATAFIX_TABLE_REFUSAL_CHILD";
        let Ok(case) = std::env::var(CHILD) else {
            for case in ["key", "shape", "replacement"] {
                let scratch = std::env::temp_dir().join(format!(
                    "sunrise-datafix-table-refusal-{}-{case}",
                    std::process::id()
                ));
                std::fs::create_dir(&scratch).unwrap();
                let result = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--ignored",
                        "--exact",
                        "datafix::tables::tests::standalone_upgrade_only_refuses_at_a_failed_table_lookup",
                    ])
                    .env("SUNRISE_RESOURCES", &scratch)
                    .env(CHILD, case)
                    .output();
                std::fs::remove_dir_all(scratch).unwrap();
                let result = result.unwrap();
                let output = String::from_utf8_lossy(&result.stdout);
                assert!(
                    result.status.success() && output.contains("1 passed"),
                    "{case}: {output}"
                );
            }
            return;
        };

        let state = |name| Compound::from_iter([("Name".into(), dynamic::string(name))]);
        let sign = state("minecraft:sign");
        // 1.13.2 predates the sign rename (1802), but no fix on this path needs the tables.
        assert_eq!(
            upgrade_compound(NbtDataType::BlockState, 1631, sign),
            Ok(Compound::from_iter([(
                "id".into(),
                dynamic::string("minecraft:oak_sign")
            )])),
        );
        // The coral-fan rename (1515) is in range but cannot see a block state here.
        let sign = Compound::from_iter([("id".into(), dynamic::string("minecraft:sign"))]);
        assert!(upgrade_compound(NbtDataType::BlockEntity, 1510, sign).is_ok());
        assert_eq!(
            upgrade_compound(NbtDataType::BlockState, 1400, state("minecraft:stone")),
            Err(NbtUpgradeError::LegacyTablesUnavailable),
        );

        let prepared = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let mut raw: serde_json::Value = crate::json_file::read(&prepared.join(FILE)).unwrap();
        raw.as_object_mut().unwrap().remove("ENTITIES_V99");
        raw["BLOCK_STATE_DATA"] = serde_json::json!(["not a registration"]);
        raw["ITEM_FLATTENING"] = match case.as_str() {
            "key" => serde_json::json!([["minecraft:wool", "minecraft:red_wool"]]),
            "shape" => serde_json::json!([["minecraft:wool.14", 17]]),
            "replacement" => serde_json::json!([["minecraft:wool.14", "minecraft:red wool"]]),
            _ => panic!("unknown malformed table case: {case}"),
        };
        std::fs::write(path(), serde_json::to_vec(&raw).unwrap()).unwrap();
        // The item walk needs V99's entity table for its nested tree, before the first fix.
        assert_eq!(
            upgrade_compound(
                NbtDataType::ItemStack,
                99,
                Compound::from_iter([
                    ("id".into(), dynamic::string("minecraft:spawn_egg")),
                    (
                        "tag".into(),
                        Tag::compound([(
                            "EntityTag",
                            Tag::compound([("id", dynamic::string("Pig"))])
                        )])
                    ),
                ]),
            ),
            Err(NbtUpgradeError::LegacyTablesUnavailable),
        );
        // V1515's coral-fan fix needs CORAL_FANS, not the missing schema or malformed item tables.
        assert_eq!(
            upgrade_compound(NbtDataType::BlockState, 1514, state("minecraft:sign")),
            Ok(Compound::from_iter([(
                "id".into(),
                dynamic::string("minecraft:oak_sign")
            )])),
        );
        assert_eq!(
            upgrade_compound(NbtDataType::BlockState, 1400, state("minecraft:stone")),
            Err(NbtUpgradeError::LegacyTablesUnavailable),
        );
        assert_eq!(
            upgrade_compound(
                NbtDataType::ItemStack,
                1343,
                Compound::from_iter([
                    ("id".into(), dynamic::string("minecraft:wool")),
                    ("Damage".into(), Tag::Short(14)),
                    ("Count".into(), Tag::Byte(1)),
                ]),
            ),
            Err(NbtUpgradeError::LegacyTablesUnavailable),
        );
        assert_eq!(
            upgrade_compound(
                NbtDataType::ItemStack,
                1343,
                Compound::from_iter([("id".into(), Tag::Int(35))]),
            ),
            Err(NbtUpgradeError::MalformedCompound),
        );
    }

    /// A duplicated or unknown table name refuses the whole file, as the struct read before
    /// the raw map did, so the upgrade answers `LegacyTablesUnavailable`; one of each known
    /// name reads, and no row is decoded until its table is asked for.
    #[test]
    fn duplicate_and_unknown_table_names_refuse_the_file() {
        let read = |text: &str| serde_json::from_str::<RawTables>(text);
        let tables =
            read(r#"{"ITEM_IDS": [[1, "minecraft:stone"]], "BLOCK_STATE_DATA": "not rows"}"#)
                .unwrap();
        assert_eq!(tables.0.len(), 2);
        assert!(decode::<Vec<String>>(&tables, "BLOCK_STATE_DATA").is_err());
        for text in [
            r#"{"ITEM_IDS": [], "ITEM_IDS": []}"#,
            r#"{"ITEM_IDS": [], "NOT_A_TABLE": []}"#,
            r#"{"item_ids": []}"#,
        ] {
            assert!(read(text).is_err(), "{text}");
        }
    }

    /// The prepared file holds every table, each non-empty, and `BlockStateData` registers
    /// air at id 0 first, as its `bootstrap0` does.
    #[test]
    fn prepared_tables_load() {
        if !available() {
            eprintln!("skipping: no {FILE} at {}", path().display());
            return;
        }
        assert!(block_state_data().unwrap()[0].starts_with("0\tminecraft:air\t"));
        let id = |name: &str| {
            block_ids()
                .unwrap()
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, i)| *i)
        };
        assert_eq!(id("minecraft:stone"), Some(1));
        assert_eq!(
            item_ids()
                .unwrap()
                .iter()
                .find(|(i, _)| *i == 276)
                .map(|(_, n)| *n),
            Some("minecraft:diamond_sword")
        );
    }
}
