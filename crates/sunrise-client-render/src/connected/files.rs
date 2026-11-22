//! A mod's connected textures as asset files: `assets/<mod>/sunrise/connected/<name>.json`, one
//! `table` rule each, whose table is inline or a shared file
//! `assets/<mod>/sunrise/connected_tables/<name>.json` (docs/modding/src/how-to/client/connected-textures.md).
//! Read through the resource source like any asset: a resource pack overrides a file by its path,
//! and everything reloads with the packs. Nothing here runs on a mesh worker; a file that is
//! refused is logged by name and left out, like a broken OptiFine one.

use crate::pack_tags::Tags;
use crate::resource_source::ResourceSource;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use sunrise_connected::{BlockSel, Connect, HeightJoin, Orient, Rule, Table, TableSheet, faces};

/// The most a rule or table file may hold: both are a few lines.
const MAX_FILE: u64 = 1 << 20;

/// The named tables read so far in one load, each parsed once, refused ones too.
type Tables = HashMap<String, Result<Arc<Table>, String>>;

/// Every rule the `sunrise/connected` files of `root` declare, refused ones left out.
pub fn read(root: &ResourceSource) -> Vec<Rule> {
    let files = root
        .list_under("assets", &["sunrise/connected"])
        .into_iter()
        .filter(|name| name.ends_with(".json"))
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Vec::new();
    }
    let tags = Tags::new(root);
    let mut tables = Tables::new();
    files
        .into_iter()
        .filter_map(|path| {
            let rule = root
                .read_limited(&path, MAX_FILE)
                .map_err(|e| e.to_string())
                .and_then(|bytes| parse(&path, &bytes, root, &tags, &mut tables));
            rule.map_err(|e| log::warn!("Sunrise: connected textures {path}: {e}"))
                .ok()
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    /// The only method a file declares; the OptiFine methods stay in packs.
    method: FileMethod,
    #[serde(default)]
    blocks: Vec<FileBlock>,
    /// A block tag, as another way to name the blocks.
    tag: Option<String>,
    /// The sprites whose faces the rule replaces (`<mod>:block/casing`).
    #[serde(default)]
    sprites: Vec<String>,
    #[serde(default = "all_faces")]
    faces: Vec<String>,
    #[serde(default)]
    connect: FileConnect,
    /// Require the block and each joining neighbour to share a mod-published connection group
    /// (`LevelRef::set_connection_group`). Blocks with no group join nothing.
    #[serde(default)]
    connect_group: bool,
    #[serde(default)]
    orient: FileOrient,
    /// The sprite the tiles are cut from.
    sheet: String,
    table: FileTable,
    #[serde(default)]
    weight: i32,
}

fn all_faces() -> Vec<String> {
    vec!["all".to_owned()]
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FileMethod {
    Table,
}

/// A string, or an object of `T`: the object's own error (the field it names) is the error, which
/// an untagged enum would replace with "did not match any variant".
fn string_or_object<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
    what: &str,
) -> Result<Result<String, T>, String> {
    match value {
        serde_json::Value::String(name) => Ok(Ok(name)),
        serde_json::Value::Object(_) => serde_json::from_value(value)
            .map(Err)
            .map_err(|e| e.to_string()),
        _ => Err(format!("{what} is a string or an object")),
    }
}

#[derive(Deserialize)]
#[serde(try_from = "serde_json::Value")]
enum FileBlock {
    Id(String),
    State(StateBlock),
}

impl TryFrom<serde_json::Value> for FileBlock {
    type Error = String;
    fn try_from(value: serde_json::Value) -> Result<Self, String> {
        Ok(match string_or_object(value, "a block")? {
            Ok(id) => Self::Id(id),
            Err(state) => Self::State(state),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateBlock {
    block: String,
    state: BTreeMap<String, OneOrMany>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize, Default)]
#[serde(try_from = "serde_json::Value")]
enum FileConnect {
    /// The same block.
    #[default]
    Block,
    State,
    Sprite,
    /// Any block of a tag.
    Tag(String),
    /// The same block with equal values of these state properties.
    Same(Vec<String>),
    /// By the collision heights of the neighbour.
    Height(HeightFields),
}

/// The object form of `connect.height`: the clauses and the blocks `same` compares.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeightFields {
    #[serde(default)]
    same: bool,
    #[serde(default)]
    higher: bool,
    #[serde(default)]
    lower: bool,
    #[serde(default)]
    above: bool,
    #[serde(default)]
    side: bool,
    /// Block ids, and block tags with a `#`.
    #[serde(default)]
    of: Vec<String>,
}

/// The object form of `connect`: exactly one of its fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectFields {
    tag: Option<String>,
    same: Option<Vec<String>>,
    height: Option<HeightFields>,
}

impl TryFrom<serde_json::Value> for FileConnect {
    type Error = String;
    fn try_from(value: serde_json::Value) -> Result<Self, String> {
        match string_or_object::<ConnectFields>(value, "connect")? {
            Ok(name) => match name.as_str() {
                "block" => Ok(Self::Block),
                "state" => Ok(Self::State),
                "sprite" => Ok(Self::Sprite),
                other => Err(format!("unknown connect {other}")),
            },
            Err(ConnectFields {
                tag: Some(tag),
                same: None,
                height: None,
            }) => Ok(Self::Tag(tag)),
            Err(ConnectFields {
                tag: None,
                same: Some(same),
                height: None,
            }) => Ok(Self::Same(same)),
            Err(ConnectFields {
                tag: None,
                same: None,
                height: Some(height),
            }) => Ok(Self::Height(height)),
            Err(_) => Err("connect takes one of tag, same and height".into()),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum FileOrient {
    #[default]
    Default,
    Axis,
    Texture,
}

#[derive(Deserialize)]
#[serde(try_from = "serde_json::Value")]
enum FileTable {
    /// A file in `connected_tables`.
    Named(String),
    Inline(TableFile),
}

impl TryFrom<serde_json::Value> for FileTable {
    type Error = String;
    fn try_from(value: serde_json::Value) -> Result<Self, String> {
        Ok(match string_or_object(value, "table")? {
            Ok(name) => Self::Named(name),
            Err(inline) => Self::Inline(inline),
        })
    }
}

/// A table as `connected_tables` files hold it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableFile {
    tests: Vec<String>,
    #[serde(default)]
    cover: bool,
    /// Columns and rows of the sheet.
    grid: (u32, u32),
    tiles: Vec<u16>,
}

fn json<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

/// `name` (`mod:block/casing`, or `block/casing` in the file's own mod) as a texture's resource
/// path, which must exist.
fn texture(root: &ResourceSource, namespace: &str, name: &str) -> Result<String, String> {
    let (ns, path) = name.split_once(':').unwrap_or((namespace, name));
    let resource = format!("assets/{ns}/textures/{path}.png");
    if !sunrise_assets::resource_source::stays_inside(&resource) {
        return Err(format!("sprite {name} is outside the resources"));
    }
    if !root.contains(&resource) {
        return Err(format!("sprite {name} not found ({resource})"));
    }
    Ok(resource)
}

fn block_id(namespace: &str, id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("{namespace}:{id}")
    }
}

/// The ids of block tag `tag` (`mod:casings`, with or without the `#`), one selector each.
fn tag_blocks(tags: &Tags, tag: &str) -> Result<Vec<BlockSel>, String> {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    let mut ids: Vec<_> = tags.members("block", tag).iter().cloned().collect();
    if ids.is_empty() {
        return Err(format!("unknown block tag {tag}"));
    }
    ids.sort();
    Ok(ids
        .into_iter()
        .map(|id| BlockSel {
            id,
            states: Vec::new(),
        })
        .collect())
}

fn face_bits(names: &[String]) -> Result<u8, String> {
    names.iter().try_fold(0, |bits, name| {
        Ok(bits
            | match name.as_str() {
                "down" => faces::DOWN,
                "up" => faces::UP,
                "north" => faces::NORTH,
                "south" => faces::SOUTH,
                "west" => faces::WEST,
                "east" => faces::EAST,
                "sides" => faces::SIDES,
                "all" => faces::ALL,
                other => return Err(format!("unknown face {other}")),
            })
    })
}

fn build(file: TableFile) -> Result<Table, String> {
    let tests = file
        .tests
        .iter()
        .map(|name| Table::test(name).ok_or(format!("unknown test {name}")))
        .collect::<Result<Vec<_>, _>>()?;
    Table::new(tests, file.cover, file.grid, file.tiles)
}

fn table(
    root: &ResourceSource,
    namespace: &str,
    table: FileTable,
    cache: &mut Tables,
) -> Result<Arc<Table>, String> {
    match table {
        FileTable::Inline(file) => build(file).map(Arc::new),
        FileTable::Named(name) => {
            let (ns, name) = name.split_once(':').unwrap_or((namespace, &name));
            let path = format!("assets/{ns}/sunrise/connected_tables/{name}.json");
            cache
                .entry(path.clone())
                .or_insert_with(|| {
                    root.read_limited(&path, MAX_FILE)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| json::<TableFile>(&bytes))
                        .and_then(build)
                        .map(Arc::new)
                        .map_err(|e| format!("table {ns}:{name} ({path}): {e}"))
                })
                .clone()
        }
    }
}

fn parse(
    path: &str,
    bytes: &[u8],
    root: &ResourceSource,
    tags: &Tags,
    tables: &mut Tables,
) -> Result<Rule, String> {
    let file: File = json(bytes)?;
    let FileMethod::Table = file.method;
    let namespace = path
        .strip_prefix("assets/")
        .and_then(|p| p.split('/').next())
        .unwrap_or("minecraft");

    let mut match_blocks = file
        .blocks
        .into_iter()
        .map(|block| match block {
            FileBlock::Id(id) => BlockSel {
                id: block_id(namespace, &id),
                states: Vec::new(),
            },
            FileBlock::State(StateBlock { block, state }) => BlockSel {
                id: block_id(namespace, &block),
                states: state
                    .into_iter()
                    .map(|(key, values)| match values {
                        OneOrMany::One(v) => (key, vec![v]),
                        OneOrMany::Many(v) => (key, v),
                    })
                    .collect(),
            },
        })
        .collect::<Vec<_>>();
    if let Some(tag) = &file.tag {
        match_blocks.extend(tag_blocks(tags, tag)?);
    }
    let match_tiles = file
        .sprites
        .iter()
        .map(|s| texture(root, namespace, s))
        .collect::<Result<Vec<_>, _>>()?;
    if match_blocks.is_empty() && match_tiles.is_empty() {
        return Err("names no block, tag or sprite".into());
    }
    texture(root, namespace, &file.sheet)?;
    let sheet = match file.sheet.contains(':') {
        true => file.sheet,
        false => format!("{namespace}:{}", file.sheet),
    };
    let table = table(root, namespace, file.table, tables)?;

    let (connect, connect_blocks, connect_properties) = match file.connect {
        FileConnect::Block => (Connect::Block, Vec::new(), Vec::new()),
        FileConnect::State => (Connect::State, Vec::new(), Vec::new()),
        FileConnect::Sprite => (Connect::Tile, Vec::new(), Vec::new()),
        FileConnect::Tag(tag) => (Connect::Blocks, tag_blocks(tags, &tag)?, Vec::new()),
        FileConnect::Same(same) => (Connect::Properties, Vec::new(), same),
        FileConnect::Height(h) => {
            let join = HeightJoin {
                same: h.same,
                higher: h.higher,
                lower: h.lower,
                above: h.above,
                side: h.side,
            };
            if !join.any() {
                return Err("connect.height needs one of same, higher, lower and above".into());
            }
            if h.side && !(h.higher || h.above) {
                return Err("connect.height side needs higher or above".into());
            }
            if !h.of.is_empty() && !h.same {
                return Err("connect.height of is for same".into());
            }
            let mut of = Vec::new();
            for entry in &h.of {
                if entry.starts_with('#') {
                    of.extend(tag_blocks(tags, entry)?);
                } else {
                    of.push(BlockSel {
                        id: block_id(namespace, entry),
                        states: Vec::new(),
                    });
                }
            }
            (Connect::Height(join), of, Vec::new())
        }
    };
    let sheet = TableSheet {
        sprite: sheet,
        table,
    };
    Ok(Rule {
        prioritize: !match_tiles.is_empty(),
        match_tiles,
        match_blocks,
        weight: file.weight,
        faces: face_bits(&file.faces)?,
        connect,
        connect_blocks,
        connect_properties,
        connect_group: file.connect_group,
        orient: match file.orient {
            FileOrient::Default => Orient::None,
            FileOrient::Axis => Orient::StateAxis,
            FileOrient::Texture => Orient::Texture,
        },
        ..Rule::table(path.to_owned(), sheet)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_connected::Method;

    /// The folder, removed when the test ends.
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const DIR: &str = "assets/demo/sunrise/connected";

    /// A resource folder holding the `files` (path, text), the textures `demo:block/casing` and
    /// `demo:block/casing_sheet`, and the block tag `demo:casings`.
    fn folder(name: &str, files: &[(&str, &str)]) -> (Scratch, ResourceSource) {
        let root =
            std::env::temp_dir().join(format!("sunrise-ct-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let all = [
            ("assets/demo/textures/block/casing.png", ""),
            ("assets/demo/textures/block/casing_sheet.png", ""),
            (
                "data/demo/tags/block/casings.json",
                r#"{"values":["demo:brass_casing","demo:copper_casing"]}"#,
            ),
        ];
        for (path, text) in all.iter().chain(files) {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        (Scratch(root.clone()), ResourceSource::directory(root))
    }

    fn parse_in(root: &ResourceSource, text: &str) -> Result<Rule, String> {
        parse(
            &format!("{DIR}/casing.json"),
            text.as_bytes(),
            root,
            &Tags::new(root),
            &mut Tables::new(),
        )
    }

    const ROW: &str = r#"{"tests":["left","right"],"grid":[4,1],"tiles":[3,2,0,1]}"#;

    #[test]
    fn a_file_is_a_table_rule() {
        let (_scratch, root) = folder(
            "good",
            &[("assets/demo/sunrise/connected_tables/row.json", ROW)],
        );
        let rule = parse_in(
            &root,
            r##"{"method":"table","blocks":["demo:casing",{"block":"demo:log","state":{"axis":["x","z"]}}],
                "sprites":["demo:block/casing"],"faces":["sides"],"connect":{"tag":"#demo:casings"},
                "orient":"axis","sheet":"demo:block/casing_sheet","table":"row"}"##,
        )
        .unwrap();
        assert_eq!(rule.method, Method::Table);
        assert_eq!(rule.faces, faces::SIDES);
        assert_eq!(rule.orient, Orient::StateAxis);
        assert_eq!(rule.match_tiles, ["assets/demo/textures/block/casing.png"]);
        assert!(rule.prioritize);
        assert_eq!(rule.connect, Connect::Blocks);
        let ids: Vec<_> = rule.connect_blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["demo:brass_casing", "demo:copper_casing"]);
        assert!(rule.matches_block("demo:log", |_| Some("z")));
        assert!(!rule.matches_block("demo:log", |_| Some("y")));
        let sheet = rule.table.unwrap();
        assert_eq!(sheet.sprite, "demo:block/casing_sheet");
        assert_eq!(sheet.table.grid(), (4, 1));
        // The inline form reads the same.
        let inline = parse_in(
            &root,
            &format!(r#"{{"method":"table","tag":"demo:casings","sheet":"demo:block/casing_sheet","table":{ROW}}}"#),
        )
        .unwrap();
        assert_eq!(inline.table.unwrap().table, sheet.table);
        assert_eq!(inline.match_blocks.len(), 2);
        let pillar = parse_in(
            &root,
            &format!(r#"{{"method":"table","blocks":["demo:log"],"connect":{{"same":["axis"]}},"sheet":"demo:block/casing_sheet","table":{ROW}}}"#),
        )
        .unwrap();
        assert_eq!(pillar.connect, Connect::Properties);
        assert_eq!(pillar.connect_properties, ["axis"]);
    }

    #[test]
    fn a_file_can_join_by_height() {
        let (_scratch, root) = folder(
            "height",
            &[("assets/demo/sunrise/connected_tables/row.json", ROW)],
        );
        let file = |connect: &str| {
            parse_in(
                &root,
                &format!(
                    r#"{{"method":"table","blocks":["demo:roof"],"connect":{connect},"sheet":"demo:block/casing_sheet","table":"row"}}"#
                ),
            )
        };
        let rule = file(
            r#"{"height":{"same":true,"above":true,"side":true,"of":["demo:shingles","roof"]}}"#,
        )
        .unwrap();
        assert_eq!(
            rule.connect,
            Connect::Height(HeightJoin {
                same: true,
                above: true,
                side: true,
                ..Default::default()
            })
        );
        let of: Vec<_> = rule.connect_blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(of, ["demo:shingles", "demo:roof"]);
        // A rule with no clause, a `side` with nothing to look for, an `of` with nothing to
        // compare, an unknown clause and a second way to join are all refused.
        for connect in [
            r#"{"height":{}}"#,
            r#"{"height":{"same":true,"side":true}}"#,
            r#"{"height":{"higher":true,"of":["demo:roof"]}}"#,
            r#"{"height":{"heigher":true}}"#,
            r#"{"height":{"same":true},"same":["axis"]}"#,
        ] {
            assert!(file(connect).is_err(), "{connect}");
        }
        // An unknown clause is named.
        let err = file(r#"{"height":{"heigher":true}}"#).unwrap_err();
        assert!(err.contains("heigher"), "{err}");
    }

    #[test]
    fn a_file_that_cannot_load_is_refused_by_name() {
        let (_scratch, root) = folder(
            "bad",
            &[
                ("assets/demo/sunrise/connected_tables/row.json", ROW),
                (
                    "assets/demo/sunrise/connected_tables/short.json",
                    r#"{"tests":["left","right"],"grid":[4,1],"tiles":[0,1,2]}"#,
                ),
            ],
        );
        // The fields in `extra` replace the base file's: a repeated key is refused by serde.
        let file = |extra: &str| {
            let mut json: serde_json::Value = serde_json::from_str(
                r#"{"method":"table","blocks":["demo:casing"],"sheet":"demo:block/casing_sheet","table":"row"}"#,
            )
            .unwrap();
            let replaced: serde_json::Value =
                serde_json::from_str(&format!("{{{}}}", extra.trim_start_matches(','))).unwrap();
            json.as_object_mut()
                .unwrap()
                .extend(replaced.as_object().unwrap().clone());
            parse_in(&root, &json.to_string())
        };
        assert!(file("").is_ok());
        let err = file(r#","sheet":"demo:block/missing""#).unwrap_err();
        assert!(err.contains("demo:block/missing"), "{err}");
        let err = file(r#","sprites":["demo:block/gone"]"#).unwrap_err();
        assert!(err.contains("demo:block/gone"), "{err}");
        let err = file(r#","table":"short""#).unwrap_err();
        assert!(
            err.contains("demo:short") && err.contains("3 tiles for 2 tests"),
            "{err}"
        );
        let err = file(r#","table":"absent""#).unwrap_err();
        assert!(err.contains("demo:absent"), "{err}");
        let err = file(r#","connect":{"tag":"demo:nothing"}"#).unwrap_err();
        assert!(err.contains("unknown block tag demo:nothing"), "{err}");
        let err = parse_in(&root, r#"{"method":"table","tag":"demo:nothing","sheet":"demo:block/casing_sheet","table":"row"}"#).unwrap_err();
        assert!(err.contains("unknown block tag demo:nothing"), "{err}");
        let err = file(r#","faces":["front"]"#).unwrap_err();
        assert!(err.contains("front"), "{err}");
        let err = parse_in(
            &root,
            r#"{"method":"table","sheet":"demo:block/casing_sheet","table":"row"}"#,
        )
        .unwrap_err();
        assert!(err.contains("names no block"), "{err}");
        let err = file(r#","colour":1"#).unwrap_err();
        assert!(err.contains("colour"), "{err}");
        // Combined or misspelt fields of a variant fail too, never dropped.
        let err = file(r#","connect":{"tag":"demo:casings","same":["axis"]}"#).unwrap_err();
        assert!(err.contains("one of tag, same and height"), "{err}");
        let err = file(r#","connect":{"same":["axis"],"sames":[]}"#).unwrap_err();
        assert!(err.contains("sames"), "{err}");
        let err = file(r#","blocks":[{"block":"demo:log","state":{},"stat":{}}]"#).unwrap_err();
        assert!(err.contains("stat"), "{err}");
    }

    #[test]
    fn only_the_valid_files_of_a_folder_load() {
        let (_scratch, root) = folder(
            "read",
            &[
                ("assets/demo/sunrise/connected_tables/row.json", ROW),
                (
                    "assets/demo/sunrise/connected/good.json",
                    r#"{"method":"table","blocks":["demo:casing"],"sheet":"demo:block/casing_sheet","table":"row"}"#,
                ),
                ("assets/demo/sunrise/connected/broken.json", "{"),
            ],
        );
        let rules = read(&root);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].path, "assets/demo/sunrise/connected/good.json");
        assert!(!rules[0].connect_group);
        // Nothing declared: nothing read (and the tag files are not even opened).
        assert!(read(&folder("none", &[]).1).is_empty());
    }

    #[test]
    fn connect_group_field_defaults_false_and_parses_true() {
        let (_scratch, root) = folder(
            "connect_group",
            &[
                ("assets/demo/sunrise/connected_tables/row.json", ROW),
                (
                    "assets/demo/sunrise/connected/grouped.json",
                    r#"{"method":"table","blocks":["demo:casing"],"connect_group":true,"sheet":"demo:block/casing_sheet","table":"row"}"#,
                ),
            ],
        );
        let rules = read(&root);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].connect_group);
    }
}
