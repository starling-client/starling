//! Sunrise's mod registry from inside the game (Options > Mods > Browse mods,
//! docs/design/mod-hotload.md): the same `index.json` the launcher reads, the
//! projects this game can install, and installing one exactly as the launcher
//! does (`https://github.com/sunrise-org/sunrise-launcher/blob/main/src/browser/sunrise.rs`, `install_version`): downloaded into
//! `mod-browser/downloads/`, checked against the index (size, SHA-256, the
//! author's signature, the file's own manifest), moved into `sunrise-mods/`
//! and recorded in `mod-browser.json`, so the launcher lists, updates and
//! removes it as one of its own.
//!
//! The launcher is a separate workspace, so this is the small part of it the
//! game needs. Everything here blocks: the game runs it on a worker
//! (`game/interface/mod_browser.rs`).
use crate::mod_browser::Listing;
use std::fs;
use std::io::Read;
use std::path::Path;
use sunrise_mod_package::durable;
use sunrise_mod_package::index::browser_state::{Installed, content_file};
use sunrise_mod_package::index::{self, Index, Kind, Project, Trust, Version};

/// Where the registry publishes its index (the launcher's `INDEX_URL`).
pub const INDEX_URL: &str = "";
/// The Minecraft version this game is: the registry's `game`.
pub const GAME: &str = sunrise_core::version::VERSION_ID;
/// The registry's pages on the website, `<SITE>/<id>`.
pub const SITE: &str = "";
/// The last index fetched, beside `options.txt`: the launcher keeps its own copy there too.
pub const CACHE: &str = "mod-browser/sunrise-index.json";
/// Where a download waits for its checks.
const DOWNLOADS: &str = "mod-browser/downloads";
/// The game asks the registry by this name.
const AGENT: &str = concat!(
    "horizon-rust/horizon/",
    env!("CARGO_PKG_VERSION"),
    " (in-game mod browser)"
);

/// Where the index comes from: `SUNRISE_MOD_INDEX` (a file or an `https://`
/// address, for a test registry), else [`INDEX_URL`]; as the launcher reads it.
pub fn location() -> String {
    std::env::var("SUNRISE_MOD_INDEX")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| INDEX_URL.to_owned())
}

/// A local test registry: its files may be `file://` paths.
fn is_local(at: &str) -> bool {
    !at.starts_with("https://")
}

/// The index from `at`, kept in `home`'s [`CACHE`]; the kept copy when the
/// registry can't be reached.
pub fn load_index(home: &Path, at: &str) -> Result<Index, String> {
    if at.is_empty() {
        return Err("The Sunrise mod registry is not available yet.".into());
    }
    let local = is_local(at);
    let parse = |text: &str| match local {
        true => Index::parse_local(text),
        false => Index::parse(text),
    };
    let fetched = fetch_text(at).and_then(|text| parse(&text).map(|index| (index, text)));
    let cache = home.join(CACHE);
    match fetched {
        Ok((index, text)) => {
            if let Some(dir) = cache.parent() {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(&cache, text);
            Ok(index)
        }
        Err(e) => {
            let kept = fs::read_to_string(&cache).map_err(|_| e.clone())?;
            parse(&kept).map_err(|_| e)
        }
    }
}

fn fetch_text(at: &str) -> Result<String, String> {
    if !at.starts_with("https://") {
        return fs::read_to_string(at).map_err(|e| format!("{at}: {e}"));
    }
    let response = ureq::get(at)
        .set("User-Agent", AGENT)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("the mod registry answered {code}"),
            ureq::Error::Transport(t) => format!("the mod registry can't be reached: {t}"),
        })?;
    let mut text = String::new();
    response
        .into_reader()
        .take(index::MAX_INDEX)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    Ok(text)
}

/// The version of content mod `p` this game installs: the newest for this
/// Minecraft version that wasn't taken down and has no code.
pub fn installable<'a>(index: &Index, p: &'a Project) -> Option<&'a Version> {
    if p.kind != Kind::Mod {
        return None;
    }
    index
        .installable(p, GAME)
        .filter(|v| v.capabilities.known() == [sunrise_mod_package::Capability::Registries])
}

/// The registry's projects the browser offers, most downloaded first: content
/// mods this game installs, with the version installed in `home` when there is
/// one.
pub fn listings(index: &Index, home: &Path) -> Vec<Listing> {
    let installed = Installed::load(home)
        .inspect_err(|e| log::warn!("Cannot load mod browser state: {e}"))
        .unwrap_or_default();
    let mut out: Vec<Listing> = index
        .projects
        .iter()
        .filter_map(|p| {
            let v = installable(index, p)?;
            Some(Listing {
                id: p.id.clone(),
                name: p.name.clone(),
                summary: p.summary.clone(),
                description: p.description.clone(),
                authors: p.authors.clone(),
                license: p.license.clone(),
                categories: p.categories.clone(),
                tags: p.tags.clone(),
                version: v.version.clone(),
                published: v.published.clone(),
                changelog: v.changelog.clone(),
                downloads: p.downloads,
                page: format!("{SITE}/{}", p.id),
                installed: installed.version_of(home, &p.id),
            })
        })
        .collect();
    out.sort_by(|a, b| b.downloads.cmp(&a.downloads).then(a.name.cmp(&b.name)));
    out
}

/// What [`install`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub id: String,
    pub version: String,
    /// Checked against the author's key, or unsigned (the SHA-256 still pins
    /// the reviewed file).
    pub verified: bool,
}

/// Installs the newest installable version of the registry's content mod `id`
/// into the game folder `home`, as the launcher does: `sunrise-mods/<id>.smod`,
/// recorded in `mod-browser.json`. Turning it on is the caller's
/// (`sunrise-mods.txt` belongs to the game's settings). `get` downloads a file
/// of at most `max` bytes (the network, or a test's files).
pub fn install(
    index: &Index,
    home: &Path,
    id: &str,
    get: &dyn Fn(&str, u64) -> Result<Vec<u8>, String>,
) -> Result<Done, String> {
    let p = index
        .project(id)
        .ok_or_else(|| format!("{id} isn't in Sunrise's registry"))?;
    let v = installable(index, p)
        .ok_or_else(|| format!("{} has no version this game can install", p.name))?;
    let relative = content_file(id);
    let target = home.join(&relative);
    // Share the entire read/modify/write transaction with the launcher's installers.
    let _lock = sunrise_mod_package::install_lock::lock_home(home)
        .map_err(|e| format!("cannot lock mod browser in {}: {e}", home.display()))?;
    let mut list = Installed::load(home)?;
    let ours = list.single(id)?;
    let old_file = ours.and_then(|i| list.entries[i]["file"].as_str().map(str::to_owned));
    // A missing file does not relinquish the launcher's native pin or change its kind.
    if (ours.is_some() && old_file.as_deref() != Some(relative.as_str()))
        || home
            .join(crate::mod_list::NATIVE_DIR)
            .join(format!("{id}.smod"))
            .exists()
    {
        return Err("code mods are installed and updated in the launcher".into());
    }
    list.check_file_claim(home, &relative, id)?;
    if target.exists() && old_file.as_deref() == Some(relative.as_str()) {
        let package = sunrise_mod_package::Package::read(&target)?;
        if package.manifest.native.is_some() {
            return Err("code mods are installed and updated in the launcher".into());
        }
    }
    if let Some(i) = ours
        && list.entries[i]["version"].as_str() == Some(v.version.as_str())
        && target.exists()
    {
        return Ok(Done {
            id: id.to_owned(),
            version: v.version.clone(),
            verified: list.entries[i]["verdict"].as_str() == Some("Verified"),
        });
    }
    // Never over a mod the player put there themselves.
    let theirs = target.exists() && old_file.as_deref() != Some(relative.as_str());
    if theirs || home.join(crate::mod_list::DIR).join(id).is_dir() {
        return Err(format!(
            "a mod with the id {id} is already in {}: move it away first",
            crate::mod_list::DIR
        ));
    }
    let bytes = get(&v.file.url, v.file.size)?;
    let staging = home.join(DOWNLOADS);
    fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    let download = staging.join(format!("{id}.smod.part"));
    durable::write(&download, &bytes).map_err(|e| format!("{}: {e}", download.display()))?;
    let checked = || -> Result<_, String> {
        let trust = index::check_file(index, p, v, &download)?;
        let package = sunrise_mod_package::Package::read(&download)?;
        if package.manifest.native.is_some() {
            return Err("code mods are installed and updated in the launcher".into());
        }
        Ok(trust)
    };
    let trust = checked().inspect_err(|_| {
        let _ = fs::remove_file(&download);
    })?;
    if let Some(dir) = target.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let verified = trust == Trust::Verified;
    let entry = serde_json::json!({
        "source": "sunrise",
        "project": p.id,
        "slug": p.id,
        "title": p.name,
        "kind": "mod",
        "version": v.version,
        "number": v.version,
        "file": relative,
        "checksum": format!("SHA-256:{}", v.file.sha256),
        "page": format!("{SITE}/{}", p.id),
        "dependency": false,
        "verdict": if verified { "Verified" } else { "Unsigned" },
    });
    list.publish(home, id, &download, entry)?;
    Ok(Done {
        id: id.to_owned(),
        version: v.version.clone(),
        verified,
    })
}

/// Downloads `url` (at most `max` bytes): `https://` only, or a `file://` path
/// when the index itself is a local test registry.
pub fn download(url: &str, max: u64) -> Result<Vec<u8>, String> {
    if let Some(path) = url.strip_prefix("file://") {
        if !is_local(&location()) {
            return Err(format!("{url} is not an https address"));
        }
        return fs::read(path).map_err(|e| format!("{path}: {e}"));
    }
    if !url.starts_with("https://") {
        return Err(format!("{url} is not an https address"));
    }
    let response = ureq::get(url)
        .set("User-Agent", AGENT)
        .call()
        .map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(max + 1)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    if body.len() as u64 > max {
        return Err(format!("{url} is larger than the index says"));
    }
    Ok(body)
}

#[cfg(test)]
mod tests;
