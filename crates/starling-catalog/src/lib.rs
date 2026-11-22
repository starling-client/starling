//! The desktop launcher's mod browser (`launcher/src/browser.rs`) without its window, for Starling's
//! catalog on Android (`game/catalog.rs`). Search, compatibility, checked downloads and installs
//! stay one implementation. What the browser asks of the rest of the launcher is answered below.
#![allow(dead_code, unused_imports, clippy::all)]
// The browser's own tests run in the launcher's workspace, against its real data packs and
// registry. This crate's are integration tests, so the unit-test build is empty.
#![cfg(not(test))]

#[path = "../../../launcher/src"]
mod launcher {
    #[rustfmt::skip]
    pub mod browser;
}
pub use launcher::browser;
pub mod list;

use browser::{
    horizon::Registry,
    installed::{self, Entry, Installed},
    modrinth::Modrinth,
    Kind, Project, Query, Source, SourceId, Support, Version, GAME,
};
use serde_json::Value;
use std::collections::HashSet;
use std::{fs, path::Path};

const MODRINTH_API: &str = "https://api.modrinth.com/v2/search";
const USER_AGENT: &str = "starling-client/starling (github.com/starling-client/starling)";
/// The build service makes ARM64 libraries only, so a native mod would stay unbuilt elsewhere.
const NATIVE_MODS_BUILD: bool = cfg!(any(target_arch = "aarch64", not(target_os = "android")));
/// The largest shader pack the catalog downloads.
const LARGEST_SHADER_PACK: u64 = 256 << 20;

/// The catalog's three lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shelf {
    /// Horizon's registry of reviewed mods.
    #[default]
    Mods,
    /// Modrinth's resource packs.
    ResourcePacks,
    /// Modrinth's Iris and OptiFine shader packs, which the game converts. A search also finds
    /// Horizon's own.
    ShaderPacks,
}

/// What an install put in the game folder, which says what the player does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// In `horizon-native-mods/`: it waits for a compile.
    NativeMod,
    /// Loads with the resources, at the next start.
    ContentMod,
    ResourcePack,
    ShaderPack,
}

/// One page of `shelf`. `text` narrows it. `everything` also lists resource packs made for other
/// versions of the game, each marked ([`caution`]).
pub fn search(
    shelf: Shelf,
    text: &str,
    everything: bool,
    offset: usize,
    limit: usize,
) -> Result<Vec<Project>, String> {
    match shelf {
        Shelf::Mods => {
            // Left out first, then paged: a page that lost all its rows would look like the end.
            let mut query = Query::new(text, Kind::Mod);
            query.limit = usize::MAX;
            Ok(Registry
                .search(&query)?
                .into_iter()
                .filter(|p| p.support().installable())
                .filter(|p| NATIVE_MODS_BUILD || !is_native_mod(p))
                .filter(|p| !hidden_native_mod(p))
                .skip(offset)
                .take(limit)
                .collect())
        }
        Shelf::ResourcePacks => {
            let mut query = Query::new(text, Kind::ResourcePack);
            query.everything = everything;
            query.offset = offset;
            query.limit = limit;
            Modrinth.search(&query)
        }
        Shelf::ShaderPacks => {
            // The Java packs are the shelf. The registry's only come up when searched for, ahead
            // of them, and the pages go on into Modrinth's. Its index being out of reach doesn't
            // take Modrinth's away.
            let own = if text.trim().is_empty() {
                Vec::new()
            } else {
                let mut query = Query::new(text, Kind::Shader);
                query.limit = usize::MAX;
                Registry.search(&query).unwrap_or_default()
            };
            let mut page: Vec<Project> = own.iter().skip(offset).take(limit).cloned().collect();
            if page.len() < limit {
                let rest = limit - page.len();
                let past = offset.saturating_sub(own.len());
                page.extend(modrinth_shader_packs(text, past, rest)?);
            }
            Ok(page)
        }
    }
}

/// A page of Modrinth's shader packs. The browser leaves them out: the desktop's game has its own
/// format, and Starling converts these. Only Iris's and OptiFine's: what Modrinth files as a shader
/// for the other loaders is no pack the game can read.
fn modrinth_shader_packs(text: &str, offset: usize, limit: usize) -> Result<Vec<Project>, String> {
    let mut request = ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(30))
        .build()
        .get(MODRINTH_API)
        .query(
            "facets",
            r#"[["project_type:shader"],["categories:iris","categories:optifine"]]"#,
        )
        .query("offset", &offset.to_string())
        .query("limit", &limit.clamp(1, 100).to_string());
    let text = text.trim();
    request = if text.is_empty() {
        request.query("index", "downloads")
    } else {
        request.query("query", text)
    };
    let found: Value = request
        .call()
        .map_err(|e| format!("Modrinth: {e}"))?
        .into_json()
        .map_err(|e| format!("Modrinth: {e}"))?;
    Ok(shader_hits(&found))
}

/// The shader packs of a Modrinth search's answer.
pub fn shader_hits(found: &Value) -> Vec<Project> {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    found["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|hit| {
            let slug = text(&hit["slug"]);
            Some(Project {
                source: SourceId::Modrinth,
                id: hit["project_id"].as_str()?.to_owned(),
                page: format!("https://modrinth.com/shader/{slug}"),
                slug,
                title: text(&hit["title"]),
                summary: text(&hit["description"]),
                author: text(&hit["author"]),
                downloads: hit["downloads"].as_u64().unwrap_or(0),
                icon: hit["icon_url"]
                    .as_str()
                    .filter(|u| !u.is_empty())
                    .map(str::to_owned),
                license: text(&hit["license"]),
                kind: Kind::Shader,
                game_versions: strings(&hit["versions"]),
                loaders: strings(&hit["categories"]),
            })
        })
        .collect()
}

/// What the player should know before installing `project`, if anything.
pub fn caution(project: &Project) -> Option<String> {
    if is_modrinth_shader_pack(project) {
        return None;
    }
    match project.support() {
        Support::Works => None,
        Support::Partial(why) | Support::No(why) => Some(why),
    }
}

/// The native mods this game can't play, which the Mods shelf leaves out: the game says which
/// (`horizon_client::starling_store::catalog_hidden`), once, before its first search.
static HIDDEN_NATIVE_MODS: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();

/// Leaves the native mods `ids` off the Mods shelf. Only the first call counts.
pub fn hide_native_mods(ids: impl IntoIterator<Item = String>) {
    let _ = HIDDEN_NATIVE_MODS.set(ids.into_iter().collect());
}

fn hidden_native_mod(project: &Project) -> bool {
    HIDDEN_NATIVE_MODS
        .get()
        .is_some_and(|ids| ids.contains(&project.id))
        && is_native_mod(project)
}

/// Whether `project` is a native mod, which only runs once compiled into the game. The registry
/// says so: a search of it has its index at hand.
pub fn is_native_mod(project: &Project) -> bool {
    project.source == SourceId::Horizon && browser::horizon::is_native_mod(&project.id)
}

fn is_modrinth_shader_pack(project: &Project) -> bool {
    project.kind == Kind::Shader && project.source == SourceId::Modrinth
}

/// The ids of the projects installed in the game folder `home`: those the catalog installed whose
/// file is still there (one the player deleted by hand is not installed any more), and the
/// registry's mods already there some other way ([`mods_present`]), which an install would only
/// put there twice.
pub fn installed_projects(home: &Path) -> HashSet<String> {
    let mut ids: HashSet<String> = Installed::load(home)
        .entries
        .into_iter()
        .filter(|entry| home.join(&entry.file).exists())
        .map(|entry| entry.project)
        .collect();
    ids.extend(mods_present(home).into_keys());
    ids
}

/// The folders mods are in: content mods, and native ones waiting for a compile.
const MOD_FOLDERS: [&str; 2] = [browser::java::MODS_DIR, native_mods::INSTALLED];

/// The mods in the game folder `home`, by id, with their files (relative to `home`), however they
/// got there: this catalog (`horizon-mods/<id>.hmod`), the launcher's (`<id>-<version>.hmod`), or
/// a file opened with Starling under its own name. A registry project's id is its mod's id.
pub fn mods_present(home: &Path) -> std::collections::HashMap<String, Vec<String>> {
    let mut found = std::collections::HashMap::<String, Vec<String>>::new();
    for folder in MOD_FOLDERS {
        let Ok(read) = fs::read_dir(home.join(folder)) else {
            continue;
        };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if !(name.ends_with(".hmod") || name.ends_with(".hnmod") || path.is_dir()) {
                continue;
            }
            if let Some(id) = package_id(&path, &name) {
                found
                    .entry(id)
                    .or_default()
                    .push(format!("{folder}/{name}"));
            }
        }
    }
    found
}

/// A package's mod id, kept by its path, size and time: the screen looks every two seconds, and
/// opening each package again on the phone's shared storage would hold it up.
fn package_id(path: &Path, name: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Seen = HashMap<std::path::PathBuf, (u64, Option<std::time::SystemTime>, String)>;
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    let meta = fs::metadata(path).ok()?;
    let stamp = (meta.len(), meta.modified().ok());
    let seen = SEEN.get_or_init(Default::default);
    if let Some((len, time, id)) = seen.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        if (*len, *time) == stamp {
            return Some(id.clone());
        }
    }
    let id = read_package_id(path, name)?;
    seen.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_owned(), (stamp.0, stamp.1, id.clone()));
    Some(id)
}

/// The `id` of a package's `horizon-mod.toml` (a top-level `id = "..."`), a `.hnmod` bundle's own
/// id, or else the file's name without its extension.
fn read_package_id(path: &Path, name: &str) -> Option<String> {
    let manifest = || -> Option<String> {
        let text = if path.is_dir() {
            fs::read_to_string(path.join("horizon-mod.toml")).ok()?
        } else {
            let mut archive = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
            let mut text = String::new();
            std::io::Read::read_to_string(
                &mut archive.by_name("horizon-mod.toml").ok()?,
                &mut text,
            )
            .ok()?;
            text
        };
        manifest_id(&text)
    };
    manifest()
        .or_else(|| {
            name.ends_with(".hnmod")
                .then(|| horizon_mod_package::index::native::Any::read(path).ok())
                .flatten()
                .map(|bundle| bundle.id().to_owned())
        })
        .or_else(|| {
            let stem = name
                .strip_suffix(".hmod")
                .or_else(|| name.strip_suffix(".hnmod"))
                .unwrap_or(name);
            Some(stem.to_owned())
        })
}

/// A manifest's top-level `id`, before its first table.
fn manifest_id(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return None;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "id" {
            let id = value.trim().strip_prefix('"')?.split('"').next()?;
            let plain = !id.is_empty()
                && id.len() <= 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            return plain.then(|| id.to_owned());
        }
    }
    None
}

/// Takes off the list what the player deleted by hand, so an install puts it back instead of
/// finding its version already there.
pub fn forget_missing(home: &Path) {
    let mut list = Installed::load(home);
    let before = list.entries.len();
    list.entries.retain(|entry| home.join(&entry.file).exists());
    if list.entries.len() != before {
        let _ = list.save(home);
    }
}

/// Removes what the catalog installed for `project`: its file (a mod's folder) and its entry.
/// Gives the entry, which says what was removed and where it was.
pub fn remove(home: &Path, project: &Project) -> Result<Entry, String> {
    let ours = Installed::load(home)
        .entries
        .iter()
        .any(|e| e.source == project.source && e.project == project.id);
    if ours || project.source != SourceId::Horizon || project.kind != Kind::Mod {
        return installed::remove(home, &project.id);
    }
    // A registry mod that came some other way ([`mods_present`]): its files go, and it goes off.
    let files = mods_present(home)
        .remove(&project.id)
        .ok_or_else(|| format!("{} isn't in the game folder.", project.title))?;
    for file in &files {
        let path = home.join(file);
        let gone = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        match gone {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{file}: {e}")),
        }
    }
    browser::java::set_enabled(home, &project.id, false)?;
    Ok(Entry {
        source: project.source,
        project: project.id.clone(),
        slug: project.slug.clone(),
        title: project.title.clone(),
        kind: project.kind,
        version: String::new(),
        number: String::new(),
        file: files.into_iter().next().unwrap_or_default(),
        checksum: String::new(),
        page: project.page.clone(),
        dependency: false,
        verdict: None,
        skipped: Vec::new(),
        namespaces: Vec::new(),
    })
}

/// The mod id of an installed mod's entry, which the Mods page and `horizon-mods.txt` know it by:
/// a registry mod's project id, else its file's name in `horizon-mods/` or `horizon-native-mods/`.
pub fn mod_id(entry: &Entry) -> Option<String> {
    if entry.source == SourceId::Horizon && entry.kind == Kind::Mod {
        return Some(entry.project.clone());
    }
    browser::horizon::mod_id(entry).map(str::to_owned)
}

/// Installs `project` (and what it needs) into the game folder `home`, its download checked.
/// `progress` gets a line and the fraction done.
pub fn install(
    project: &Project,
    everything: bool,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<Done, String> {
    forget_missing(home);
    if project.source == SourceId::Horizon && project.kind == Kind::Mod {
        let ours = Installed::load(home)
            .entries
            .iter()
            .any(|e| e.source == project.source && e.project == project.id);
        if let Some(files) = mods_present(home).get(&project.id).filter(|_| !ours) {
            return Err(format!(
                "{} is already in the game folder ({}).",
                project.title,
                files.join(", ")
            ));
        }
    }
    if is_modrinth_shader_pack(project) {
        install_modrinth_shader_pack(project, home, progress)?;
        return Ok(Done::ShaderPack);
    }
    let source = browser::source(project.source)
        .ok_or_else(|| format!("{} can't be installed from here.", project.title))?;
    let entries = installed::install(source.as_ref(), project, everything, home, progress)?;
    Ok(match project.kind {
        Kind::ResourcePack => Done::ResourcePack,
        Kind::Shader => Done::ShaderPack,
        _ if entries
            .iter()
            .any(|e| e.file.starts_with(native_mods::INSTALLED)) =>
        {
            Done::NativeMod
        }
        _ => Done::ContentMod,
    })
}

/// The newest ZIP of a shader pack, preferring one made for the version the game plays.
fn shader_pack_version(versions: &[Version]) -> Option<&Version> {
    let usable = |v: &&Version| {
        v.blocked.is_none()
            && v.file()
                .is_some_and(|f| f.name.to_ascii_lowercase().ends_with(".zip"))
    };
    versions
        .iter()
        .filter(usable)
        .find(|v| v.game_versions.iter().any(|g| g == GAME))
        .or_else(|| versions.iter().find(usable))
}

/// Modrinth's shader pack into `shaderpacks/`, and on the list of what the catalog installed.
/// Adapted from Horizon Android's `shader_catalog.rs` (Vasysik, MIT).
fn install_modrinth_shader_pack(
    project: &Project,
    home: &Path,
    progress: &dyn Fn(&str, f32),
) -> Result<(), String> {
    let versions = Modrinth.versions(project)?;
    let version = shader_pack_version(&versions)
        .ok_or_else(|| format!("{} has no ZIP to download.", project.title))?;
    if !version.required.is_empty() {
        return Err(format!(
            "{} needs other projects, which can't be installed from here.",
            project.title
        ));
    }
    let file = version
        .file()
        .ok_or_else(|| format!("{} has no file.", project.title))?;
    if file.size > LARGEST_SHADER_PACK {
        return Err(format!("{} is over 256 MB.", project.title));
    }
    let name = browser::safe_name(&file.name)
        .ok_or_else(|| format!("{:?} isn't a plain file name.", file.name))?;
    let relative = format!("shaderpacks/{name}");
    let target = home.join(&relative);
    let mut list = Installed::load(home);
    let ours = |e: &Entry| e.source == project.source && e.project == project.id;
    let old = list.entries.iter().find(|e| ours(e)).cloned();
    if old.as_ref().is_some_and(|o| o.version == version.id) {
        return Ok(());
    }
    let replaces_its_own = old.as_ref().is_some_and(|o| o.file == relative);
    if target.exists() && !replaces_its_own {
        return Err(format!("{relative} is already there: move it away first."));
    }
    let stage = home
        .join("mod-browser/downloads")
        .join(format!("shader-{}", version.id));
    let line = format!("{} {}", project.title, version.number);
    let result = (|| {
        let path = Modrinth.fetch(file, &stage, &|f| progress(&line, f))?;
        if !is_shader_pack(&path)? {
            return Err(format!("{} is not a shader pack.", project.title));
        }
        fs::create_dir_all(home.join("shaderpacks")).map_err(|e| e.to_string())?;
        fs::rename(&path, &target).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_dir_all(&stage);
    result?;
    if let Some(old) = old.as_ref().filter(|o| o.file != relative) {
        let _ = fs::remove_file(home.join(&old.file));
    }
    let (algo, hash) = file
        .checksum()
        .ok_or_else(|| format!("{} gives no checksum.", project.title))?;
    list.entries.retain(|e| !ours(e));
    list.entries.push(Entry {
        source: project.source,
        project: project.id.clone(),
        slug: project.slug.clone(),
        title: project.title.clone(),
        kind: project.kind,
        version: version.id.clone(),
        number: version.number.clone(),
        file: relative,
        checksum: format!("{algo}:{hash}"),
        page: project.page.clone(),
        dependency: false,
        verdict: None,
        skipped: Vec::new(),
        namespaces: Vec::new(),
    });
    list.save(home)
}

/// Whether the ZIP at `path` holds an OptiFine or Iris pack: a `shaders/` folder at its top, or
/// in the one folder its files are in (as `import.rs` tells them).
pub fn is_shader_pack(path: &Path) -> Result<bool, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?;
        let inner = name.split_once('/').map_or("", |(_, rest)| rest);
        if name.starts_with("shaders/") || inner.starts_with("shaders/") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The launcher's data and identity, as the browser reads them.
mod backend {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    /// What the browser's requests say they are.
    pub const VERSION: &str = "android";
    /// Whose app the requests come from (their user agent).
    pub const REPO: &str = "starling-client/starling";
    static HOME: OnceLock<PathBuf> = OnceLock::new();

    pub fn game_home() -> PathBuf {
        HOME.get().cloned().unwrap_or_default()
    }

    pub(super) fn set_game_home(home: PathBuf) {
        let _ = HOME.set(home);
    }
}

/// Where the browser keeps its cache and packs: the game's folder. Called once, at start.
pub fn set_game_home(home: std::path::PathBuf) {
    backend::set_game_home(home);
}

/// Where a native mod's file goes (`mod_list::NATIVE_DIR`).
mod native_mods {
    pub const INSTALLED: &str = "horizon-native-mods";
}

/// The launcher's languages: the catalog's texts stay in English, the game's own pages translate.
mod i18n {
    pub fn tr(en: &str) -> &str {
        en
    }

    /// `en` with its `{name}` holes filled.
    pub fn trf(en: &str, args: &[(&str, &str)]) -> String {
        let mut out = en.to_string();
        for (name, value) in args {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
}

/// The launcher's instances: the phone has one game, and the browser is asked with none.
mod instances {
    pub struct Instance;
}

/// How a native mod fits an instance's build: no instance, nothing to say.
mod native_fit {
    pub fn line(_instance: &super::instances::Instance, _pin: &str) -> Option<String> {
        None
    }
}

/// Which installed files are native mods (the launcher's `native_status::is_native_file`).
mod native_status {
    pub fn is_native_file(file: &str) -> bool {
        // Starling's `.hnmod` (what the catalog installs), or Horizon's `.hmod`.
        file.strip_prefix(super::native_mods::INSTALLED)
            .is_some_and(|rest| {
                rest.starts_with('/') && (rest.ends_with(".hnmod") || rest.ends_with(".hmod"))
            })
    }
}

/// The launcher's proxy setting: the phone's own network settings apply instead.
mod proxy {
    pub fn spec_for(_url: &str) -> Option<String> {
        None
    }
}

/// Data packs go into a world from the launcher's Home page: the catalog doesn't offer them.
mod datapacks {
    use std::path::Path;

    const NOT_HERE: &str = "Data packs can't be installed from here.";

    pub struct DataPack;

    pub struct Coverage {
        pub read: Vec<String>,
    }

    impl Coverage {
        pub fn not_read(&self) -> String {
            String::new()
        }
    }

    pub fn coverage(_file: &Path) -> Result<Coverage, String> {
        Err(NOT_HERE.to_owned())
    }

    pub fn add(_world: &Path, _file: &Path) -> Result<DataPack, String> {
        Err(NOT_HERE.to_owned())
    }
}
