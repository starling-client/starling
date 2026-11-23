//! Content mods tied to the world: blocks and items a mod adds join the game's registries once a
//! start, before the resources load (`mods::content_root`), so a world plays with the set it was
//! made with. Each world keeps its set in `sunrise-content.json`; opening a world whose set is not
//! the one running starts the game again with that set (`next-start.json` in the private home)
//! and enters the world, instead of opening it without its blocks (which a save would turn into
//! air). A start the player did not ask for that way runs the mods turned on in Options > Mods.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

/// In a world's folder: the content mods it plays with.
const WORLD_FILE: &str = "sunrise-content.json";
/// In the private home: the world the next start enters, and its content mods.
const NEXT_FILE: &str = "next-start.json";
/// In the private home: the content mods the last world opened with, and those Options > Mods
/// turned on then.
const LAST_FILE: &str = "last-content.json";

/// What a restart asked of this start.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct NextStart {
    pub(super) world: String,
    pub(super) content: BTreeSet<String>,
    /// A new world to create with those mods (Create New World's choice), not one to open.
    pub(super) create: Option<Created>,
}

/// Create New World's choices, kept across the restart that loads the world's mods.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Created {
    pub(super) name: String,
    pub(super) seed: Option<i64>,
    pub(super) creative: bool,
    pub(super) options: sunrise_client::title::WorldOptions,
}

fn next_path() -> PathBuf {
    sunrise_client::logging::private_home().join(NEXT_FILE)
}

/// The restart's request, read (and removed) once a process: a second start reads nothing.
pub(super) fn next_start() -> Option<&'static NextStart> {
    static NEXT: OnceLock<Option<NextStart>> = OnceLock::new();
    NEXT.get_or_init(|| {
        let path = next_path();
        let text = std::fs::read_to_string(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        let world = value["world"].as_str()?.to_owned();
        let content = ids(&value["content"]);
        let create = serde_json::from_value(value["create"].clone()).ok();
        log::info!("Sunrise: started again to enter {world} with its mods {content:?}");
        Some(NextStart {
            world,
            content,
            create,
        })
    })
    .as_ref()
}

/// The world a restart was made for: it opens as the title's Play does, or is created as Create
/// New World asked.
pub(super) fn next_entry() -> Option<super::PendingWorldEntry> {
    let next = next_start()?;
    Some(match &next.create {
        Some(c) => super::PendingWorldEntry::Local {
            folder: next.world.clone(),
            name: Some(c.name.clone()),
            seed: c.seed,
            creative: c.creative,
            create: Some(Box::new(c.options.clone())),
        },
        None => super::PendingWorldEntry::Local {
            folder: next.world.clone(),
            name: None,
            seed: None,
            creative: false,
            create: None,
        },
    })
}

fn ids(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

/// The mods adding blocks or items this process loaded (`mods::content_root` keeps it).
fn running_cell() -> &'static Mutex<BTreeSet<String>> {
    static RUNNING: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(Default::default)
}
pub(super) fn set_running(ids: BTreeSet<String>) {
    *running_cell()
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = ids;
}

/// Every content mod (`Capability::Registries`) this process loaded, blocks and items or not
/// (`mods::content_root` keeps it).
fn loaded_cell() -> &'static Mutex<BTreeSet<String>> {
    static LOADED: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    LOADED.get_or_init(Default::default)
}
pub(super) fn set_loaded(ids: BTreeSet<String>) {
    *loaded_cell().lock().unwrap_or_else(PoisonError::into_inner) = ids;
}
/// Whether this process loaded any content mod, blocks and items or not.
pub(super) fn any_loaded() -> bool {
    !loaded().is_empty()
}
fn loaded() -> BTreeSet<String> {
    loaded_cell()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The set a world that keeps none plays with: the content mods Options > Mods turns on, but not
/// one that loaded here and adds no blocks or items. Such a mod (a wasm mod declaring
/// `registries` for its hooks) never joins `running`, so with it the world never matched the
/// running set: it started the game again on every open, and then called the mod missing.
fn world_default(
    turned_on: BTreeSet<String>,
    loaded: &BTreeSet<String>,
    running: &BTreeSet<String>,
) -> BTreeSet<String> {
    turned_on
        .into_iter()
        .filter(|id| !loaded.contains(id) || running.contains(id))
        .collect()
}

/// The mods adding blocks or items that Options > Mods turns on (`mods::content_root` keeps it):
/// the set every world played with before worlds kept their own.
fn enabled_cell() -> &'static Mutex<BTreeSet<String>> {
    static ENABLED: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    ENABLED.get_or_init(Default::default)
}
pub(super) fn set_enabled(ids: BTreeSet<String>) {
    *enabled_cell()
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = ids;
}
fn enabled() -> BTreeSet<String> {
    enabled_cell()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The mods adding blocks or items in the mods folder, on or off: a world needing one that is
/// not there is not worth starting again for.
fn installed_cell() -> &'static Mutex<Option<BTreeSet<String>>> {
    static INSTALLED: OnceLock<Mutex<Option<BTreeSet<String>>>> = OnceLock::new();
    INSTALLED.get_or_init(Default::default)
}
pub(super) fn set_installed(ids: BTreeSet<String>) {
    *installed_cell()
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(ids);
}
fn installed() -> Option<BTreeSet<String>> {
    installed_cell()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
pub(super) fn running() -> BTreeSet<String> {
    running_cell()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn world_dir(folder: &str) -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("saves")
        .join(folder)
}

/// The content mods world `folder` plays with, when it says.
pub(super) fn recorded(folder: &str) -> Option<BTreeSet<String>> {
    read_set(&world_dir(folder).join(WORLD_FILE))
}

fn read_set(path: &Path) -> Option<BTreeSet<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(ids(&value["mods"]))
}

/// Keeps `mods` as world `folder`'s set; false when the world has no folder yet.
pub(super) fn record(folder: &str, mods: &BTreeSet<String>) -> bool {
    let dir = world_dir(folder);
    if !dir.join("level.dat").is_file() {
        return false;
    }
    let body = serde_json::json!({ "mods": mods });
    match std::fs::write(dir.join(WORLD_FILE), format!("{body:#}\n")) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("Sunrise: {folder}'s content mods were not kept: {e}");
            false
        }
    }
}

/// What opening world `folder` needs.
#[derive(Debug, PartialEq)]
pub(super) enum Entry {
    /// It plays with the running set.
    Open,
    /// Start again with this set, then enter it.
    Restart(BTreeSet<String>),
    /// This start was made for it, and its mods still did not load: entering would lose their
    /// blocks.
    Missing(BTreeSet<String>),
}

/// A world that keeps no set yet played with the mods Options > Mods turns on (worlds kept none
/// before): that set is its own, kept once it opens with it.
pub(super) fn entry(folder: &str) -> Entry {
    // Calibrate...'s test world (perf_lab.rs) is made, timed and deleted in one go: it plays with
    // what runs. Starting again for the content mods a new world takes lost the calibration under
    // way, and the world opened as an ordinary one.
    if folder == sunrise_client::benchmark::WORLD {
        return Entry::Open;
    }
    let kept = recorded(folder);
    let running = running();
    let set = kept
        .clone()
        .unwrap_or_else(|| world_default(enabled(), &loaded(), &running));
    let code = mod_api::__private::installed_code_ids()
        .map(str::to_owned)
        .collect();
    let now = for_world(running, &set, &code);
    // A mod with code that loaded is there, though not in the mods folder's content (`start_set`
    // reads no native package): a world made with a native mod's blocks counted it missing.
    let installed = installed().map(|installed| &installed | &code);
    decide(folder, set, &now, installed.as_ref(), next_start(), || {
        if kept.is_none() {
            record(folder, &now);
        }
    })
}

/// What runs, as world `set` sees it: a mod with code loads with the game whatever a world plays
/// with, so one the world doesn't name is left out. Starting again never takes it away, and
/// comparing with it called the world's own mods missing (a native mod's blocks on the POCO,
/// "This world plays with , which did not load", 2026-11-19).
fn for_world(
    running: BTreeSet<String>,
    set: &BTreeSet<String>,
    code: &BTreeSet<String>,
) -> BTreeSet<String> {
    running
        .into_iter()
        .filter(|id| set.contains(id) || !code.contains(id))
        .collect()
}

fn decide(
    folder: &str,
    set: BTreeSet<String>,
    running: &BTreeSet<String>,
    installed: Option<&BTreeSet<String>>,
    next: Option<&NextStart>,
    keep: impl FnOnce(),
) -> Entry {
    if set == *running {
        keep();
        return Entry::Open;
    }
    // Not in the mods folder at all: starting again would not bring it.
    if let Some(installed) = installed
        && !set.is_subset(installed)
    {
        return Entry::Missing(set.difference(installed).cloned().collect());
    }
    if next.is_some_and(|n| n.world == folder && n.content == set) {
        Entry::Missing(set.difference(running).cloned().collect())
    } else {
        Entry::Restart(set)
    }
}

/// Keeps the set running now (a world opened with it) for the next start the player makes:
/// opening that world again then needs no restart, which on a phone shows as the game closing
/// and opening again.
pub(super) fn remember_running() {
    let body = serde_json::json!({ "mods": running(), "enabled": enabled() });
    let path = sunrise_client::logging::private_home().join(LAST_FILE);
    if let Err(e) = std::fs::write(&path, body.to_string()) {
        log::warn!("Sunrise: the last world's mods were not kept: {e}");
    }
}

/// The set a start the player made (no restart asked for one) loads: the last world's, while
/// Options > Mods turns on the same mods as then (`enabled_now`); none once that changed, so the
/// mods turned on there are the ones a world made next takes.
pub(super) fn last_start(enabled_now: &BTreeSet<String>) -> Option<BTreeSet<String>> {
    let text =
        std::fs::read_to_string(sunrise_client::logging::private_home().join(LAST_FILE)).ok()?;
    last_from(&text, enabled_now)
}

fn last_from(text: &str, enabled_now: &BTreeSet<String>) -> Option<BTreeSet<String>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    (ids(&value["enabled"]) == *enabled_now).then(|| ids(&value["mods"]))
}

/// The content mods this start loads (`ModsState::select` takes them for Options > Mods' own):
/// the ones turned on there, but for the mods adding blocks or items (`Capability::Registries`)
/// the set a restart asked for (`next_start`), else the last world's while Options > Mods turns
/// on the same ones as then (`last_start`). Keeps which of those are installed and turned on, for
/// `entry`.
pub(super) fn start_set(
    game_dir: &Path,
    enabled: &BTreeSet<String>,
    dev: &[PathBuf],
) -> BTreeSet<String> {
    use sunrise_mod_package::{Capability, Package, find, mod_list::DIR};
    let mut paths = find(&game_dir.join(DIR));
    paths.retain(|p| !dev.contains(p));
    paths.extend(dev.iter().cloned());
    let (mut installed, mut turned_on) = (BTreeSet::new(), BTreeSet::new());
    for path in &paths {
        let Ok(package) = Package::read(path) else {
            continue;
        };
        let m = &package.manifest;
        // Only a mod adding blocks or items is a world's (what `running` keeps): one with data
        // only (structures, recipes, loot) follows Options > Mods whatever world opens. It was
        // left out of a world whose set didn't name it, and that world then counted it missing
        // (YUNG's structures on the POCO, 2026-11-19).
        if package.is_native()
            || !m.capabilities.contains(&Capability::Registries)
            || !adds_blocks_or_items(path, &m.id)
        {
            continue;
        }
        installed.insert(m.id.clone());
        if enabled.contains(&m.id) || dev.contains(path) {
            turned_on.insert(m.id.clone());
        }
    }
    let asked = match next_start() {
        Some(next) => Some(next.content.clone()),
        None => last_start(&turned_on).inspect(|last| {
            log::info!("Sunrise: starting with the last world's mods {last:?}");
        }),
    };
    set_enabled(turned_on);
    set_installed(installed.clone());
    match asked {
        Some(set) => (enabled.iter())
            .filter(|id| !installed.contains(*id))
            .cloned()
            .chain(set)
            .collect(),
        None => enabled.clone(),
    }
}

/// Whether the package at `path` (a folder or a `.smod`) declares blocks or items: anything under
/// `data/<id>/sunrise/block/` or `item/`, where content mods declare them.
fn adds_blocks_or_items(path: &Path, id: &str) -> bool {
    let prefixes = [
        format!("data/{id}/sunrise/block/"),
        format!("data/{id}/sunrise/item/"),
    ];
    if path.is_dir() {
        return prefixes.iter().any(|prefix| path.join(prefix).is_dir());
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(archive) = zip::ZipArchive::new(file) else {
        return false;
    };
    // A name that doesn't decode is no declaration.
    archive.file_names().flatten().any(|name| {
        prefixes
            .iter()
            .any(|prefix| name.starts_with(prefix.as_str()))
    })
}

/// Writes the next start's request: world `folder` with `content`. A world with no level data
/// yet (named by the control's `open` or `--world`, not made by Create New World) is created
/// then as `new` says: its game mode, seed and name came back as the defaults (survival) after
/// the restart.
pub(super) fn request_restart(
    folder: &str,
    content: &BTreeSet<String>,
    new: Created,
) -> std::io::Result<()> {
    let exists = world_dir(folder).join("level.dat").is_file();
    write_next(restart_body(folder, content, exists, new))
}

fn restart_body(
    folder: &str,
    content: &BTreeSet<String>,
    exists: bool,
    new: Created,
) -> serde_json::Value {
    if exists {
        return serde_json::json!({ "world": folder, "content": content });
    }
    // Made with the set the restart loads: the start after it creates the world at once.
    let created = Created {
        options: sunrise_client::title::WorldOptions {
            mods: Some(content.clone()),
            ..new.options
        },
        ..new
    };
    serde_json::json!({ "world": folder, "content": content, "create": created })
}

/// Writes the next start's request: a new world `folder` made with `created`, with `content`.
pub(super) fn request_create(
    folder: &str,
    content: &BTreeSet<String>,
    created: &Created,
) -> std::io::Result<()> {
    write_next(serde_json::json!({ "world": folder, "content": content, "create": created }))
}

fn write_next(body: serde_json::Value) -> std::io::Result<()> {
    let path = next_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, body.to_string())
}

/// Starts the game again: Android's launcher activity, or this program with its arguments.
pub(super) fn restart() {
    #[cfg(target_os = "android")]
    crate::android::restart_app_saying(&sunrise_client::lang::get_or(
        "starling.restart.world_mods",
        "Opening the world with its mods…",
    ));
    #[cfg(not(target_os = "android"))]
    {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        match std::env::current_exe()
            .and_then(|exe| std::process::Command::new(exe).args(args).spawn())
        {
            Ok(_) => std::process::exit(0),
            Err(e) => log::warn!("Sunrise: could not start again: {e}"),
        }
    }
}

/// `mods` as a line names them: "Create, Farmer's Delight", by id.
pub(super) fn names(mods: &BTreeSet<String>) -> String {
    mods.iter().cloned().collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_world_keeps_its_set_and_reads_it_back() {
        let dir =
            std::env::temp_dir().join(format!("sunrise-world-content-{}", std::process::id()));
        let world = dir.join("saves").join("w");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("level.dat"), b"").unwrap();
        let file = world.join(WORLD_FILE);
        std::fs::write(&file, serde_json::json!({ "mods": ["create"] }).to_string()).unwrap();
        assert_eq!(read_set(&file), Some(set(&["create"])));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_loaded_mod_without_blocks_or_items_is_not_a_worlds_content() {
        let probe_and_create = set(&["create", "probe"]);
        // Both loaded, only Create adds blocks: a new world plays with Create alone, which is
        // what runs, so it opens without starting again.
        let default = world_default(
            probe_and_create.clone(),
            &probe_and_create,
            &set(&["create"]),
        );
        assert_eq!(default, set(&["create"]));
        assert_eq!(
            decide("w", default, &set(&["create"]), None, None, || {}),
            Entry::Open
        );
        // A mod turned on that did not load here is still asked for.
        assert_eq!(
            world_default(probe_and_create, &set(&[]), &set(&[])),
            set(&["create", "probe"])
        );
    }

    #[test]
    fn a_world_with_another_set_starts_again_once() {
        let (create, none) = (set(&["create"]), set(&[]));
        assert_eq!(
            decide("w", create.clone(), &create, None, None, || {}),
            Entry::Open
        );
        assert_eq!(
            decide("w", none.clone(), &create, None, None, || {}),
            Entry::Restart(none.clone())
        );
        // Started again for it, and Create still did not load: never open without it.
        let next = NextStart {
            world: "w".into(),
            content: create.clone(),
            create: None,
        };
        assert_eq!(
            decide("w", create.clone(), &none, None, Some(&next), || {}),
            Entry::Missing(create.clone())
        );
        // Another world after that start is a restart of its own.
        assert_eq!(
            decide("v", create.clone(), &none, None, Some(&next), || {}),
            Entry::Restart(create)
        );
    }

    #[test]
    fn a_mod_not_in_the_folder_is_not_started_again_for() {
        let (create, none) = (set(&["create"]), set(&[]));
        assert_eq!(
            decide("w", create.clone(), &none, Some(&none), None, || {}),
            Entry::Missing(create.clone())
        );
        assert_eq!(
            decide("w", create.clone(), &none, Some(&create), None, || {}),
            Entry::Restart(create)
        );
    }

    /// Only a mod declaring blocks or items is a world's: a data-only one (structures) is not.
    #[test]
    fn only_blocks_and_items_make_a_mod_a_worlds() {
        let dir = std::env::temp_dir().join(format!("sunrise-adds-{}", std::process::id()));
        let blocks = dir.join("glass");
        std::fs::create_dir_all(blocks.join("data/glass/sunrise/block")).unwrap();
        let items = dir.join("gems");
        std::fs::create_dir_all(items.join("data/gems/sunrise/item")).unwrap();
        let data = dir.join("temples");
        std::fs::create_dir_all(data.join("data/temples/worldgen/structure")).unwrap();
        assert!(adds_blocks_or_items(&blocks, "glass"));
        assert!(adds_blocks_or_items(&items, "gems"));
        assert!(!adds_blocks_or_items(&data, "temples"));
        assert!(!adds_blocks_or_items(&dir.join("absent.smod"), "absent"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A native mod's blocks load with the game: a world made without them opens as it is, and
    /// one made with them still needs them.
    #[test]
    fn a_mod_with_code_a_world_does_not_name_is_not_running_for_it() {
        let world = set(&["mcwdoors"]);
        let running = set(&["mcwdoors", "cloudglass"]);
        let code = set(&["cloudglass"]);
        let now = for_world(running.clone(), &world, &code);
        assert_eq!(now, world);
        assert_eq!(
            decide("w", world.clone(), &now, None, None, || {}),
            Entry::Open
        );
        // A content mod without code still asks for a start with the world's set.
        let other = for_world(running, &world, &set(&[]));
        assert_eq!(
            decide("w", world.clone(), &other, None, None, || {}),
            Entry::Restart(world)
        );
        // A world made with it keeps it in what runs for it.
        let with = set(&["mcwdoors", "cloudglass"]);
        assert_eq!(for_world(with.clone(), &with, &code), with);
    }

    /// Calibrate... opens its test world whatever content mods are turned on (seen on a POCO:
    /// a wasm mod turned on made it start again, and the calibration was lost).
    #[test]
    fn the_calibration_world_never_starts_the_game_again() {
        assert_eq!(entry(sunrise_client::benchmark::WORLD), Entry::Open);
    }

    /// A new world opened by name (`open NAME creative`) that has to start the game again for
    /// its mods is created after it in creative, with its seed, not in survival.
    #[test]
    fn a_new_world_keeps_its_game_mode_across_the_restart_for_its_mods() {
        let content = set(&["create"]);
        let new = Created {
            name: "Test".into(),
            seed: Some(7),
            creative: true,
            options: Default::default(),
        };
        let body = restart_body("w", &content, false, new.clone());
        let created: Created = serde_json::from_value(body["create"].clone()).unwrap();
        assert!(created.creative);
        assert_eq!(created.seed, Some(7));
        assert_eq!(created.name, "Test");
        assert_eq!(created.options.mods, Some(content.clone()));
        // A world that exists keeps its own level data: nothing is created.
        assert!(restart_body("w", &content, true, new)["create"].is_null());
    }

    #[test]
    fn a_world_keeps_its_set_only_when_it_opens() {
        let mut kept = false;
        decide("w", set(&["create"]), &set(&[]), None, None, || kept = true);
        assert!(!kept);
        decide("w", set(&[]), &set(&[]), None, None, || kept = true);
        assert!(kept);
    }

    #[test]
    fn a_start_takes_the_last_worlds_mods_while_options_turn_on_the_same() {
        let (create, none) = (set(&["create"]), set(&[]));
        let kept = serde_json::json!({ "mods": ["create"], "enabled": [] }).to_string();
        assert_eq!(last_from(&kept, &none), Some(create.clone()));
        // Options > Mods changed since: its mods, for the worlds made next.
        assert_eq!(last_from(&kept, &create), None);
        assert_eq!(last_from("not json", &none), None);
    }

    #[test]
    fn ids_take_strings_only() {
        let value = serde_json::json!(["create", 3, "farmersdelight"]);
        assert_eq!(ids(&value), set(&["create", "farmersdelight"]));
    }
}
