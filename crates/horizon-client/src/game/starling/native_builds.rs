// game-module-cfg: target_os = "android"
//! Android: native mods (docs/starling-native-mods.md). A native mod is compiled into the game, so
//! playing with one is playing another build of this version's game library, made by GitHub
//! (native-mods.yml) and published as assets of the native-builds release, the organisation's
//! repository's and Starling's own both asked (starling_releases). This card lists the
//! builds for this version, downloads one (its SHA-256 checked against the build's JSON), and
//! chooses it for the next start: StarlingActivity loads it in place of the APK's library
//! (NativeMods.java). It also stops using one, asks GitHub for another set (an issue the player
//! submits), and says when the activity set a build aside. The Mods page says, above the mods,
//! which game runs and which the next start runs (`native_game`).
//!
//! Compile waits for the service's answer on GitHub, not by asking the service
//! (`starling_store`: its free plan counts every request), and with an APK whose game is a set of
//! Rust libraries takes a library per mod from the store into the folder the game opens at its
//! start (Compile v2, below).
use super::Game;
use horizon_client::{
    hud::HudBatch,
    lang::tr,
    mod_list::{NativeGame, NativeGameAct},
    starling_releases::{self as releases, NativeBuild as Build},
    starling_store as store,
    ui_theme::{Face, Ui, color},
};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use winit::event::{Touch, TouchPhase};

/// The mod build service (packaging/android/mod_build_server.py), while the file that says where it
/// is now (`releases::BUILD_SERVER_URL`) has never been read.
const BUILD_SERVER: &str = "https://191-234-193-205.sslip.io";
/// Beside the builds: the player's own build server (StarlingActivity.askBuildServer), and the last
/// address the project published.
const CUSTOM_SERVER: &str = "build-server";
const PUBLISHED_SERVER: &str = "published-build-server";
/// In the folder of a build from the player's own server: which one.
const CAME_FROM: &str = "server";
/// The button that compiles on GitHub's machines (`Act::CompileOnGithub`).
const USE_GITHUB: &str = "Use GitHub (slower)";
/// What the service takes in one request (`MAX_BODY`).
const LARGEST_REQUEST: usize = 32 << 20;
/// A build's folder here starts with this, and a new one takes the place of the last.
const COMPILED: &str = "compiled-";
/// The service's waits: between looks at the job, and how long it may go on at all.
const POLL: Duration = Duration::from_secs(2);
const LONGEST: Duration = Duration::from_secs(40 * 60);
/// GitHub's machines have fewer cores, and there may be a line for them.
const LONGEST_ON_GITHUB: Duration = Duration::from_secs(90 * 60);
const WAKING: Duration = Duration::from_secs(5 * 60);
/// Tries in a row that brought nothing before a look or a download gives up.
const QUIET_TRIES: u32 = 15;

/// The library's name in a build's folder, as NativeMods.java loads it.
const LIBRARY: &str = "libhorizon_android.so";
/// Beside it: the install of the APK it was made under, the only one that loads it
/// (NativeMods.stamp, which StarlingActivity puts in [`APK_STAMP`]).
const APK: &str = "apk";
const APK_STAMP: &str = "STARLING_APK";
/// Rows of builds the card shows.
const SHOWN: usize = 4;
/// Options > Mods' row that opens the card, after the native mods' own rows: a native mod's page
/// with one button. (Not in native_hooks.rs, which keeps no `cfg`.)
const ID: &str = "starling-native-builds";

// The hosted build service and prebuilt libraries currently target ARM64 only.
const SERVICE_SUPPORTED: bool = cfg!(target_arch = "aarch64");
fn unsupported() -> String {
    horizon_client::lang::tr("starling.native_builds.arm64_only", &[])
}

#[derive(Clone, PartialEq)]
enum Stage {
    Loading,
    List,
    Downloading(String),
    /// Chosen: it plays from the next start.
    Chosen(String),
    Failed(String),
    /// What compiling mod `.0` means, before it starts.
    Warn(String),
    /// On the build service: the water to fish in while it takes.
    Compiling,
}

#[derive(Default)]
struct Shared {
    builds: Option<Result<Vec<Build>, String>>,
    done: u64,
    total: u64,
    downloaded: Option<Result<String, String>>,
    /// A compile is under way (its card may be hidden), and what it says.
    compiling: bool,
    status: String,
    /// When the service began compiling: the card counts from it, each frame.
    building_since: Option<Instant>,
}

#[derive(Default)]
pub(super) struct NativeBuilds {
    stage: Option<Stage>,
    shared: Arc<Mutex<Shared>>,
    /// The buttons as last drawn (UI units), with what each does.
    buttons: std::cell::RefCell<Vec<([f32; 4], Act)>>,
    /// The activity's notice was shown this run.
    noticed: Cell<bool>,
    fishing: super::setup::fishing::Fishing,
    mining: super::build_mining::BuildMining,
    /// Under a build: digging, not fishing.
    digging: bool,
    /// What failed last was a compile: Try again compiles, and doesn't show the published builds.
    compile_failed: bool,
    /// The last compile asked for GitHub's machines: Try again asks for them too.
    on_github: bool,
}

#[derive(Clone, PartialEq)]
enum Act {
    Use(Build),
    StopUsing,
    Restart,
    Retry,
    Close,
    Compile,
    /// The same on GitHub's machines, through Starling's service: slower, with limits of its own.
    CompileOnGithub,
    /// Under a build: from the water to the rock and back.
    Switch,
    /// The activity's dialog that chooses another build server.
    Server,
}

fn dir() -> PathBuf {
    horizon_client::logging::private_home().join("native")
}

/// The build server the player chose in place of Starling's, if any.
fn custom_server() -> Option<String> {
    let text = std::fs::read_to_string(dir().join(CUSTOM_SERVER)).ok()?;
    releases::build_server_address(&text)
}

/// Where Starling's own build service is: what the project publishes now, or the last it published
/// when that can't be read.
fn published_server(version: &str) -> String {
    let file = dir().join(PUBLISHED_SERVER);
    let published = get(releases::BUILD_SERVER_URL, version)
        .and_then(|r| r.into_string().map_err(|e| e.to_string()))
        .ok()
        .and_then(|text| releases::build_server_address(&text));
    if let Some(address) = published {
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(&file, &address);
        return address;
    }
    std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| releases::build_server_address(&text))
        .unwrap_or_else(|| BUILD_SERVER.to_owned())
}

/// The other server the build `name` came from, if it did.
fn came_from(name: &str) -> Option<String> {
    let host = std::fs::read_to_string(dir().join(name).join(CAME_FROM)).ok()?;
    let host = host.trim();
    (!host.is_empty()).then(|| host.to_owned())
}

/// `text` in lines that fit `width`.
pub(super) fn wrapped(ui: &Ui<'_>, face: Face, size: f32, width: f32, text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let next = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && ui.measure(face, size, &next) > width {
            lines.push(std::mem::replace(&mut line, word.to_owned()));
        } else {
            line = next;
        }
    }
    lines.push(line);
    lines
}

/// The build chosen for the next start (files/native/current), if any.
fn chosen() -> Option<String> {
    let name = std::fs::read_to_string(dir().join("current")).ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The build this game runs, when it is one (StarlingActivity names it).
fn running() -> Option<String> {
    let path = PathBuf::from(std::env::var_os("STARLING_GAME_LIBRARY")?);
    Some(path.parent()?.file_name()?.to_string_lossy().into_owned())
}

/// This install of the APK (NativeMods.stamp), none when the activity could not say.
fn apk_stamp() -> Option<String> {
    std::env::var(APK_STAMP).ok().filter(|s| !s.is_empty())
}

/// The build in `folder` marked as made under this install: only it loads it.
fn stamp_build(folder: &std::path::Path) -> Result<(), String> {
    match apk_stamp() {
        Some(stamp) => std::fs::write(folder.join(APK), stamp).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Whether the build `name` loads at the next start, as NativeMods.chosen decides: its library
/// is there, made for `version` and under this very install of the APK.
fn loads(name: &str, version: &str) -> bool {
    let folder = dir().join(name);
    let read = |file: &str| {
        std::fs::read_to_string(folder.join(file))
            .ok()
            .map(|t| t.trim().to_owned())
    };
    folder.join(LIBRARY).is_file()
        && read("for").as_deref() == Some(version)
        && apk_stamp().is_some_and(|stamp| read(APK) == Some(stamp))
}

/// Beside a compiled build: the SHA-256 of the request it was compiled from (`bundles_body`), so a
/// native mod replaced under the same id makes it out of date (`mod_list::native_build_current`).
const SENT: &str = "sent.sha256";

/// The SHA-256 of what Compile would send now, or None when there is nothing to send.
fn request_digest() -> Option<String> {
    let body = bundles_body().ok()?;
    Some(
        Sha256::digest(&body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

/// A build on the phone as the Mods page says it: its mods (build.json), the request it was
/// compiled from when it says it ([`SENT`]), and how long ago its library came.
struct OnPhone {
    mods: BTreeSet<String>,
    sent: Option<String>,
    age: Option<Duration>,
}

fn on_phone(name: &str) -> OnPhone {
    let folder = dir().join(name);
    let about = std::fs::read_to_string(folder.join("build.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or_default();
    let mods = about["mods"]
        .as_array()
        .map(|mods| {
            mods.iter()
                .filter_map(|m| m["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let sent = std::fs::read_to_string(folder.join(SENT))
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let age = std::fs::metadata(folder.join(LIBRARY))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|at| at.elapsed().ok());
    OnPhone { mods, sent, age }
}

impl OnPhone {
    /// Whether this build is the one for the native mods on now (`wanted`, sent as `request`).
    fn current_for(&self, wanted: &BTreeSet<String>, request: Option<&str>) -> bool {
        horizon_client::mod_list::native_build_current(
            self.sent.as_deref(),
            &self.mods,
            wanted,
            request,
        )
    }

    fn label(&self) -> String {
        mods_label(&self.mods.iter().cloned().collect::<Vec<_>>())
    }

    /// "2 h ago".
    fn ago(&self) -> String {
        let seconds = self.age.map_or(0, |a| a.as_secs());
        let (key, n) = match seconds {
            s if s < 3600 => ("starling.mods.native.minutes_ago", (s / 60).max(1)),
            s if s < 48 * 3600 => ("starling.mods.native.hours_ago", s / 3600),
            s => ("starling.mods.native.days_ago", s / 86400),
        };
        tr(key, &[&n])
    }
}

/// A file of `horizon-native-mods/` a native mod may be: Starling's `.hnmod` (Open with) or the
/// `.hmod` Horizon's catalog installs a native mod as, in either format (`native::Any`), which
/// tells the rest apart.
fn native_file(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|e| e == "hnmod" || e == "hmod")
}

/// The native mods in the folder that are on: what Compile sends (`bundles_body`).
fn folder_mods_on(off: &BTreeSet<String>) -> BTreeSet<String> {
    let folder = Game::game_dir().join(horizon_client::mod_list::NATIVE_DIR);
    std::fs::read_dir(&folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| native_file(p))
        .filter_map(|p| horizon_mod_package::index::native::Any::read(&p).ok())
        .map(|bundle| bundle.id().to_owned())
        .filter(|id| !off.contains(id))
        .collect()
}

/// A build on the phone made from the mods `wanted` as they are now (`request`), that loads at
/// the next start.
fn build_with(wanted: &BTreeSet<String>, request: Option<&str>, version: &str) -> Option<String> {
    if wanted.is_empty() {
        return None;
    }
    std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|name| loads(name, version) && on_phone(name).current_for(wanted, request))
}

/// `v0.1.4__wrench__carryblock`'s mods as the card says them: "Wrench, Carryblock".
fn mods_label(mods: &[String]) -> String {
    let word = |m: &String| {
        let mut c = m.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect::<String>())
            .unwrap_or_default()
    };
    mods.iter().map(word).collect::<Vec<_>>().join(", ")
}

/// A name of the release's assets: lowercase letters, digits, `.`, `_` and `-`.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

fn get(url: &str, version: &str) -> Result<ureq::Response, String> {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .build()
        .get(url)
        .set("User-Agent", &format!("Starling/{version}"))
        .call()
        .map_err(|e| e.to_string())
}

/// The builds made for `version`, in every repository's native-builds release (a repository
/// without one has none).
fn list(version: &str) -> Result<Vec<Build>, String> {
    let mut lists = Vec::new();
    let mut errors = Vec::new();
    for repository in releases::REPOSITORIES {
        let release = get(&releases::native_builds_url(repository), version)
            .and_then(|r| r.into_string().map_err(|e| e.to_string()))
            .and_then(|text| {
                serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string())
            });
        match release {
            Ok(release) => lists.push(releases::native_builds_in(&release, version)),
            Err(e) if e.contains("404") => {}
            Err(e) => errors.push(e),
        }
    }
    if lists.is_empty() {
        return Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| "No native mod builds have been made yet.".to_owned()));
    }
    Ok(releases::merge_native_builds(lists))
}

/// The build's library into files/native/NAME/, checked against its JSON, and chosen for the next
/// start.
fn download(build: &Build, version: &str, shared: &Mutex<Shared>) -> Result<String, String> {
    if !SERVICE_SUPPORTED {
        return Err(unsupported());
    }
    let text = get(&build.json, version)?
        .into_string()
        .map_err(|e| e.to_string())?;
    let about: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    // Old metadata predates `abi` and describes the same ARM64-only service.
    if !releases::native_build_matches(&about, releases::process_abi()) {
        return Err(horizon_client::lang::tr(
            "starling.native_builds.wrong_architecture",
            &[],
        ));
    }
    if about["version"].as_str() != Some(version) {
        return Err(format!("That build is not for Starling {version}."));
    }
    let want = about["sha256"].as_str().unwrap_or_default().to_owned();
    if want.len() != 64 {
        return Err("That build does not say its SHA-256.".into());
    }
    let folder = dir().join(&build.name);
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let part = folder.join(format!("{LIBRARY}.part"));
    let response = get(&build.library, version)?;
    let total = response
        .header("Content-Length")
        .and_then(|l| l.parse().ok())
        .or_else(|| about["size"].as_u64())
        .unwrap_or(0);
    shared.lock().unwrap_or_else(PoisonError::into_inner).total = total;
    let mut body = response.into_reader();
    let mut file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut done = 0;
    loop {
        let n = body.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        file.write_all(&buffer[..n])
            .map_err(|e| format!("{}: {e}", part.display()))?;
        done += n as u64;
        shared.lock().unwrap_or_else(PoisonError::into_inner).done = done;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        let _ = std::fs::remove_file(&part);
        return Err("The download came out damaged. Try again.".into());
    }
    // A rename: a library this game runs stays mapped as it was.
    std::fs::rename(&part, folder.join(LIBRARY)).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("for"), version).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("build.json"), &text).map_err(|e| e.to_string())?;
    stamp_build(&folder)?;
    std::fs::write(dir().join("current"), &build.name).map_err(|e| e.to_string())?;
    Ok(build.name.clone())
}

/// The UI's scale for a window of `w` x `h` pixels (as `Ui::new`).
pub(super) fn density(w: f32, h: f32) -> f32 {
    (w / 1040.0).min(h / 700.0).clamp(1.0, 4.0)
}

/// The `horizon-native-mods` folder's mods that are on (Options > Mods), each a length and what the
/// service needs of it: nothing else on the phone goes with them. In an order of their own, not
/// their files' names, so the same mods always make the same request.
fn bundles_body() -> Result<Vec<u8>, String> {
    let off = horizon_client::mod_list::ModList::load(&Game::game_dir()).disabled;
    let folder = Game::game_dir().join(horizon_client::mod_list::NATIVE_DIR);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| native_file(p));
    // The `.hnmod` before a `.hmod` of the same mod: each goes once.
    paths.sort_by_key(|p| (p.extension().is_some_and(|e| e == "hmod"), p.clone()));
    let mut seen = BTreeSet::new();
    let mut mods = Vec::new();
    for path in paths {
        let Ok(bundle) = horizon_mod_package::index::native::Any::read(&path) else {
            continue;
        };
        if off.contains(bundle.id()) || !seen.insert(bundle.id().to_owned()) {
            continue;
        }
        mods.push(
            bundle
                .for_build()
                .map_err(|e| format!("{}: {e}", path.display()))?,
        );
    }
    if mods.is_empty() {
        return Err("There is no native mod to compile.".into());
    }
    mods.sort();
    let mut body = Vec::new();
    for bytes in mods {
        body.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        body.extend_from_slice(&bytes);
    }
    if body.len() > LARGEST_REQUEST {
        return Err(
            "Your native mods are too big to compile together. Take some out of the folder and \
             try again."
                .into(),
        );
    }
    Ok(body)
}

/// What a refused request says (the service's JSON `error`), or that it can't be reached.
fn service_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, response) => response
            .into_string()
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("The build server answered {code}.")),
        ureq::Error::Transport(_) => {
            "The build server can't be reached right now. Try again later.".into()
        }
    }
}

/// The library of job `job` into `part`, going on from what is there, checked by `want`. A
/// download from the start asks for no range: GitHub answers `bytes=0-` with a 206, which is not
/// more of a file already there. One that goes on takes a 206 only for the bytes it asked for.
fn fetch_library(
    agent: &ureq::Agent,
    url: &str,
    part: &std::path::Path,
    size: u64,
    want: &str,
    shared: &Mutex<Shared>,
) -> Result<(), String> {
    let on_disk = |p: &std::path::Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let mut quiet = 0;
    while on_disk(part) < size {
        let have = on_disk(part);
        let tried = (|| -> Result<(), String> {
            let mut request = agent.get(url);
            if have > 0 {
                request = request.set("Range", &format!("bytes={have}-"));
            }
            let response = request.call().map_err(service_error)?;
            let from = response
                .header("Content-Range")
                .and_then(|r| r.strip_prefix("bytes "))
                .and_then(|r| r.split('-').next())
                .and_then(|n| n.parse::<u64>().ok());
            let resumed = response.status() == 206 && have > 0 && from == Some(have);
            if response.status() == 206 && !resumed && from != Some(0) {
                return Err(format!("the download went on from {from:?}, not {have}"));
            }
            let mut file = if resumed {
                std::fs::OpenOptions::new().append(true).open(part)
            } else {
                std::fs::File::create(part)
            }
            .map_err(|e| format!("{}: {e}", part.display()))?;
            let mut done = if resumed { have } else { 0 };
            let mut body = response.into_reader();
            let mut buffer = vec![0; 256 * 1024];
            loop {
                let n = body.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    return Ok(());
                }
                file.write_all(&buffer[..n])
                    .map_err(|e| format!("{}: {e}", part.display()))?;
                done += n as u64;
                if done > size {
                    return Err("The build server sent more than it said.".into());
                }
                shared.lock().unwrap_or_else(PoisonError::into_inner).status = format!(
                    "Downloading: {:.0} of {:.0} MB",
                    done as f64 / 1e6,
                    size as f64 / 1e6
                );
            }
        })();
        quiet = if on_disk(part) > have { 0 } else { quiet + 1 };
        if let Err(e) = tried {
            log::warn!("Horizon: the compiled game stopped downloading ({e}); trying again");
            if quiet >= QUIET_TRIES {
                return Err(e);
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    let mut hash = Sha256::new();
    let mut file = std::fs::File::open(part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != want {
        let _ = std::fs::remove_file(part);
        return Err("The download came out damaged. Try again.".into());
    }
    Ok(())
}

// ---- Compile v2: a library per mod, from the store ---------------------------------------------
//
// With the game as a set of Rust libraries (an APK built with -C prefer-dynamic), each native mod
// is a small library of its own, built once per game build (starling-client/mod-builds,
// scripts/store.py). Compile reads the store on GitHub, asks the service only for a mod the store
// doesn't have, and leaves in the app's own files:
//
//   HORIZON_PRIVATE/native-libs/          what the game opens at its start (android_lib.rs):
//                                         native-mods.json, each lib<id>.so, each mod's whole
//                                         package (its .hmod unpacked) in horizon-native-mods/<id>/,
//                                         and build.json and sent.sha256 for the Mods page
//   HORIZON_PRIVATE/native/store-libs/    every library downloaded, <job>.so beside its <job>.json,
//                                         so turning a mod back on is no download
//
// A library is for one game build, and the game refuses one of another (its identity), so only
// this APK's own build is taken from the store.

/// What the game opens at its start, in the app's own files (android_lib.rs `NATIVE_LIBRARIES`).
const NATIVE_LIBS: &str = "native-libs";
/// The libraries downloaded, in files/native/.
const STORE_LIBS: &str = "store-libs";
/// Each mod's package beside the libraries (horizon-mod-host's `packages_beside`).
const PACKAGES: &str = "horizon-native-mods";
/// What the store says of a mod the per-mod route can't build (scripts/mod_lib.py).
const WHOLE_GAME: &str = "whole-game Compile";

fn native_libs() -> PathBuf {
    horizon_client::logging::private_home().join(NATIVE_LIBS)
}

/// Whether this APK plays mods as libraries of their own: its game is a set of Rust libraries
/// (built with -C prefer-dynamic), the mod host one of them, which a usual build folds into its one
/// library. STARLING_STORE_LOADER=1 or 0 says so in place of the look.
fn loads_store_builds() -> bool {
    match std::env::var("STARLING_STORE_LOADER").as_deref() {
        Ok("1") => return true,
        Ok("0") => return false,
        _ => {}
    }
    std::fs::read_to_string("/proc/self/maps")
        .is_ok_and(|maps| maps.contains("/libhorizon_mod_host.so"))
}

/// This game's own build, the commit its libraries come from, when the APK says it: a mod's
/// library is for that build alone.
fn own_build() -> Option<&'static str> {
    option_env!("HORIZON_ANDROID_BUILD_COMMIT")
        .or(option_env!("HORIZON_GAME_BUILD_ID"))
        .filter(|commit| commit.len() == 40)
}

/// The native mods on, as [`store_wanted`] finds them.
struct StoreWanted {
    /// Those the store knows: each one's `.hmod` and its SHA-256, once each as `bundles_body`
    /// takes them.
    mods: Vec<(PathBuf, store::Wanted)>,
    /// Starling's old bundles (`.hnmod`, or a `.hmod` in its format), by id: only the whole-game
    /// Compile builds them.
    old: Vec<String>,
}

/// The native mods on (`StoreWanted`).
fn store_wanted() -> Result<StoreWanted, String> {
    let off = horizon_client::mod_list::ModList::load(&Game::game_dir()).disabled;
    let folder = Game::game_dir().join(horizon_client::mod_list::NATIVE_DIR);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&folder)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| native_file(p));
    paths.sort_by_key(|p| (p.extension().is_some_and(|e| e == "hmod"), p.clone()));
    let mut seen = BTreeSet::new();
    let mut wanted = Vec::new();
    let mut old = Vec::new();
    for path in paths {
        let Ok(bundle) = horizon_mod_package::index::native::Any::read(&path) else {
            continue;
        };
        if off.contains(bundle.id()) || !seen.insert(bundle.id().to_owned()) {
            continue;
        }
        if !matches!(bundle, horizon_mod_package::index::native::Any::Flat(_)) {
            old.push(bundle.id().to_owned());
            continue;
        }
        let source =
            file_sha256(&path).ok_or_else(|| format!("{}: can't be read", path.display()))?;
        wanted.push((
            path,
            store::Wanted {
                id: bundle.id().to_owned(),
                source,
            },
        ));
    }
    Ok(StoreWanted { mods: wanted, old })
}

/// A text file on GitHub, or None when it isn't there.
fn github_text(agent: &ureq::Agent, url: &str) -> Result<Option<String>, String> {
    match agent.get(url).call() {
        Ok(response) => response.into_string().map(Some).map_err(|e| e.to_string()),
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(service_error(e)),
    }
}

/// What the store did with the mods on.
enum FromStore {
    /// They play from the next start, from [`NATIVE_LIBS`].
    Built,
    /// The store can't play them: why, said to the player. A game made of libraries has no
    /// whole-game Compile to send them to (`compile`).
    NotHere(String),
}

/// Compile v2: the mods on, as libraries of this game's own build from the store, the ones it
/// lacks built by the service, into [`NATIVE_LIBS`] for the next start. The store is read from
/// GitHub; the service is asked once for each mod the store lacks.
fn compile_from_store(
    server: &str,
    agent: &ureq::Agent,
    shared: &Mutex<Shared>,
    asked: &mut u32,
) -> Result<FromStore, String> {
    let say = |text: String| {
        let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
        s.status = text;
        s.building_since = None;
    };
    let Some(own) = own_build() else {
        return Ok(FromStore::NotHere(
            "This Starling doesn't say its build, so the mod store can't serve it.".into(),
        ));
    };
    let StoreWanted { mods: wanted, old } = store_wanted()?;
    // Starling's old bundles alone go to the whole-game Compile, as before. Beside mods the store
    // plays they are left out of this game: one sent the whole set there, which builds no Preview
    // and no game made of libraries, and the store's mods failed with it (2026-11-20).
    if wanted.is_empty() && !old.is_empty() {
        return Ok(FromStore::NotHere(format!(
            "{} {} Starling's old native mod format, which no longer compiles: install the \
             Horizon version from the catalog.",
            old.join(", "),
            if old.len() == 1 { "is in" } else { "are in" }
        )));
    }
    if wanted.is_empty() {
        return Err("There is no native mod to compile.".into());
    }
    if !old.is_empty() {
        log::warn!(
            "Horizon: left out of this game, as Starling's old bundles only the whole-game \
             Compile builds: {}",
            old.join(", ")
        );
    }
    let abi = releases::process_abi();
    say("Looking for your mods in the store...".into());
    let repo = store::STORE_REPO;
    let Some(text) = github_text(
        agent,
        &store::download(repo, store::STORE_TAG, store::BUILDS),
    )?
    else {
        return Ok(FromStore::NotHere(
            "The mod store has no builds yet. Try again later.".into(),
        ));
    };
    // Only this game's build: a library of another one doesn't load here.
    let builds: Vec<store::Build> = store::builds(&text)?
        .into_iter()
        .filter(|b| b.build == own)
        .collect();
    let list: Vec<store::Wanted> = wanted.iter().map(|(_, w)| w.clone()).collect();
    let plan = store::plan(
        &builds,
        |b| {
            let url = store::download(repo, &store::mods_tag(&b.build), store::INDEX);
            let text = github_text(agent, &url).ok().flatten()?;
            store::index(&text, &b.build).ok()
        },
        &list,
        abi,
        Some(own),
    );
    let (build, libraries) = match plan {
        store::Plan::Ready { build, libraries } => (build, libraries),
        store::Plan::Ask {
            build,
            mut have,
            missing,
        } => {
            have.extend(build_in_store(
                agent, server, &build, &missing, &wanted, shared, asked,
            )?);
            (build, have)
        }
        store::Plan::Fails { id, error } if error.contains(WHOLE_GAME) => {
            log::warn!("Horizon: {id} needs the whole-game Compile: {error}");
            return Ok(FromStore::NotHere(format!(
                "{id} can't be compiled on this Starling yet (it needs the whole-game Compile). \
                 Turn it off on the Mods page to compile the others."
            )));
        }
        store::Plan::Fails { id, error } => return Err(format!("{id}: {error}")),
        store::Plan::NoBuild => {
            return Ok(FromStore::NotHere(format!(
                "The mod store has no libraries for this Starling ({}, {abi}) yet. Try again in \
                 a while.",
                &own[..own.len().min(10)]
            )));
        }
    };
    let cache = dir().join(STORE_LIBS);
    std::fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
    let tag = store::mods_tag(&build.build);
    for library in &libraries {
        let path = cache.join(&library.file);
        if file_sha256(&path).as_deref() != Some(library.sha256.as_str()) {
            say(format!(
                "Downloading {}...",
                mods_label(std::slice::from_ref(&library.id))
            ));
            let part = cache.join(format!("{}.part", library.file));
            let url = store::download(repo, &tag, &library.file);
            fetch_library(agent, &url, &part, library.size, &library.sha256, shared)?;
            std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
        }
        let row = serde_json::to_string(library).map_err(|e| e.to_string())?;
        std::fs::write(cache.join(format!("{}.json", library.job)), row)
            .map_err(|e| e.to_string())?;
    }
    put_native_libs(&libraries, &wanted)?;
    let _ = std::fs::remove_file(dir().join("current"));
    let _ = std::fs::remove_file(dir().join("starts"));
    log::info!(
        "Horizon: {} native mods from the store, for build {}, with {asked} requests to the \
         build service",
        libraries.len(),
        build.build
    );
    Ok(FromStore::Built)
}

/// [`NATIVE_LIBS`] made again with `libraries` (each in [`STORE_LIBS`]) and the packages of the
/// mods `wanted` (each `.hmod` unpacked), whole or not at all: made beside it, then put in its
/// place. The game opens it at its next start; one that runs now keeps the libraries it opened.
fn put_native_libs(
    libraries: &[store::Library],
    wanted: &[(PathBuf, store::Wanted)],
) -> Result<(), String> {
    let io = |path: &std::path::Path| {
        let path = path.display().to_string();
        move |e: std::io::Error| format!("{path}: {e}")
    };
    let folder = native_libs();
    let next = folder.with_extension("next");
    let _ = std::fs::remove_dir_all(&next);
    std::fs::create_dir_all(next.join(PACKAGES)).map_err(io(&next))?;
    let cache = dir().join(STORE_LIBS);
    for library in libraries {
        let to = next.join(&library.library);
        // A real file, never a link: the loader refuses links.
        if std::fs::hard_link(cache.join(&library.file), &to).is_err() {
            std::fs::copy(cache.join(&library.file), &to).map_err(io(&to))?;
        }
        let Some((path, _)) = wanted.iter().find(|(_, w)| w.id == library.id) else {
            return Err(format!("{} went from the folder.", library.id));
        };
        let Ok(horizon_mod_package::index::native::Any::Flat(bundle)) =
            horizon_mod_package::index::native::Any::read(path)
        else {
            return Err(format!("{}: can't be read", path.display()));
        };
        // The whole package, its source too: the game refuses a `[native]` manifest without it
        // ("inconsistent code declaration"). Only plain names inside the package's folder.
        let package = next.join(PACKAGES).join(&library.id);
        std::fs::create_dir_all(&package).map_err(io(&package))?;
        for (name, bytes) in &bundle.files {
            let inside = std::path::Path::new(name);
            let plain = !name.contains('\\')
                && inside.components().count() > 0
                && inside
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)));
            if !plain {
                return Err(format!("{}: {name} is outside the package", library.id));
            }
            let to = package.join(inside);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            std::fs::write(&to, bytes).map_err(io(&to))?;
        }
    }
    std::fs::write(next.join(store::RECORD), store::record(libraries)).map_err(io(&next))?;
    let mods: Vec<serde_json::Value> = libraries
        .iter()
        .map(|l| serde_json::json!({"id": l.id, "version": l.version}))
        .collect();
    let about = serde_json::json!({"mods": mods, "build": own_build()});
    std::fs::write(next.join("build.json"), about.to_string()).map_err(io(&next))?;
    // What it was made from: a mod replaced under the same id makes it out of date.
    if let Some(sent) = request_digest() {
        std::fs::write(next.join(SENT), sent).map_err(io(&next))?;
    }
    let old = folder.with_extension("old");
    let _ = std::fs::remove_dir_all(&old);
    if folder.exists() {
        std::fs::rename(&folder, &old).map_err(io(&folder))?;
    }
    std::fs::rename(&next, &folder).map_err(io(&folder))?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// The mods in the libraries the next start opens ([`NATIVE_LIBS`]), by id. None when this APK
/// plays no such libraries, or they are of another build.
fn native_libs_mods() -> Option<BTreeSet<String>> {
    let folder = native_libs();
    if !loads_store_builds() || !folder.join(store::RECORD).is_file() {
        return None;
    }
    let about = std::fs::read_to_string(folder.join("build.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or_default();
    if about["build"].as_str() != own_build() {
        return None;
    }
    Some(
        about["mods"]
            .as_array()
            .map(|mods| {
                mods.iter()
                    .filter_map(|m| m["id"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// Mod ids as a line of the Mods page lists them.
fn listed(ids: &BTreeSet<String>) -> String {
    ids.iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The libraries the next start opens are the native mods on now (`wanted`, sent as `request`).
fn native_libs_current(wanted: &BTreeSet<String>, request: Option<&str>) -> bool {
    let Some(mods) = native_libs_mods() else {
        return false;
    };
    let sent = std::fs::read_to_string(native_libs().join(SENT))
        .ok()
        .map(|text| text.trim().to_owned());
    horizon_client::mod_list::native_build_current(sent.as_deref(), &mods, wanted, request)
}

/// The native mods on changed: [`NATIVE_LIBS`] made again from the libraries already downloaded,
/// when every mod on has one; with none on, an empty record, so none opens. Else the Mods page
/// offers Compile.
fn native_libs_follow() {
    if !loads_store_builds() || own_build().is_none() || !native_libs().exists() {
        return;
    }
    let Ok(StoreWanted { mods: wanted, old }) = store_wanted() else {
        return;
    };
    // Old bundles alone play from a whole-game build, never from these libraries.
    if wanted.is_empty() && !old.is_empty() {
        return;
    }
    let abi = releases::process_abi();
    let cache = dir().join(STORE_LIBS);
    let mut libraries = Vec::new();
    for (_, w) in &wanted {
        let job = store::job_of(&w.source, abi);
        let row = std::fs::read_to_string(cache.join(format!("{job}.json")))
            .ok()
            .and_then(|text| serde_json::from_str::<store::Library>(&text).ok());
        match row {
            Some(library)
                if file_sha256(&cache.join(&library.file)).as_deref()
                    == Some(library.sha256.as_str()) =>
            {
                libraries.push(library)
            }
            // One the phone doesn't have: Compile fetches it.
            _ => return,
        }
    }
    if let Err(e) = put_native_libs(&libraries, &wanted) {
        log::warn!("Horizon: the native mod libraries were not made again: {e}");
    }
}

/// The SHA-256 of the file at `path`, if it is there.
fn file_sha256(path: &std::path::Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).ok()?;
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The mods `missing` built by the service for `build`: each asked once (all at once, so GitHub
/// builds them side by side), then their answers waited for in the store, as Compile's are
/// (`wait_on_github`).
#[allow(clippy::too_many_arguments)]
fn build_in_store(
    agent: &ureq::Agent,
    server: &str,
    build: &store::Build,
    missing: &[store::Wanted],
    wanted: &[(PathBuf, store::Wanted)],
    shared: &Mutex<Shared>,
    asked: &mut u32,
) -> Result<Vec<store::Library>, String> {
    let abi = releases::process_abi();
    let repo = store::STORE_REPO;
    let tag = store::mods_tag(&build.build);
    let mut pending = Vec::new();
    for w in missing {
        let path = wanted
            .iter()
            .find(|(_, x)| x == w)
            .map(|(p, _)| p)
            .ok_or("A mod went from the folder.")?;
        let body = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if body.len() > LARGEST_REQUEST {
            return Err(format!("{} is too big to compile.", w.id));
        }
        {
            let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
            s.status = format!("Sending {} to GitHub...", mods_label(&[w.id.clone()]));
            s.building_since = None;
        }
        *asked += 1;
        let answer: serde_json::Value = agent
            .post(&format!("{server}/v2/mod?build={}&abi={abi}", build.build))
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&body)
            .map_err(service_error)?
            .into_json()
            .map_err(|e| e.to_string())?;
        let job = store::job_of(&w.source, abi);
        if answer["job"].as_str() != Some(job.as_str()) {
            return Err("The build server gave another job.".into());
        }
        pending.push((w.clone(), job));
    }
    {
        let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
        s.status = "Compiling...".into();
        s.building_since = Some(Instant::now());
    }
    let started = Instant::now();
    let mut stale = BTreeMap::new();
    let mut built = Vec::new();
    let mut looks = 0u32;
    let mut quiet = 0;
    let mut service_asked = Instant::now();
    while !pending.is_empty() {
        if started.elapsed() > LONGEST_ON_GITHUB {
            return Err("The build took too long. Try again later.".into());
        }
        let mut still = Vec::new();
        for (w, job) in pending {
            match answer_in(agent, repo, &tag, &job) {
                Ok(Some((store::Answer::Done { .. }, json))) => {
                    quiet = 0;
                    let library =
                        store::Library::from_answer(&json, &build.build).ok_or_else(|| {
                            format!("{}: the store's answer is not one to load.", w.id)
                        })?;
                    built.push(library);
                    continue;
                }
                Ok(Some((store::Answer::Failed { error, settled, at }, _))) => {
                    quiet = 0;
                    if looks == 0 && !settled {
                        stale.insert(job.clone(), at);
                    } else if settled || stale.get(&job) != Some(&at) {
                        return Err(format!("{}: {error}", mods_label(&[w.id.clone()])));
                    }
                }
                Ok(None) => quiet = 0,
                Err(e) => {
                    quiet += 1;
                    if quiet >= QUIET_TRIES {
                        return Err(e);
                    }
                }
            }
            still.push((w, job));
        }
        pending = still;
        if pending.is_empty() {
            break;
        }
        let mut wait = store::wait(looks);
        if service_asked.elapsed() >= store::ASK_SERVICE_EVERY {
            service_asked = Instant::now();
            for (w, job) in &pending {
                *asked += 1;
                let url = format!("{server}/v2/mod/{}/{job}", build.build);
                match agent.get(&url).call() {
                    Ok(response) => {
                        if let Ok(status) = response.into_json::<serde_json::Value>()
                            && told(status, true, shared)
                                .map_err(|e| format!("{}: {e}", w.id))?
                                .is_some()
                        {
                            wait = Duration::ZERO;
                        }
                    }
                    Err(ureq::Error::Status(404, _)) => {
                        return Err("The build server lost the job. Try again.".into());
                    }
                    Err(_) => {}
                }
            }
        }
        std::thread::sleep(wait);
        looks += 1;
    }
    Ok(built)
}

/// What the service says of a job on its way, on the card: Ok(Some) once done, Ok(None) while it
/// waits or builds, Err when it failed or is lost.
fn told(
    status: serde_json::Value,
    github: bool,
    shared: &Mutex<Shared>,
) -> Result<Option<serde_json::Value>, String> {
    let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
    match status["state"].as_str() {
        Some("starting") => {
            s.status = "Waking the build server up: about a minute...".into();
            s.building_since = None;
        }
        Some("queued") => {
            let ahead = status["ahead"].as_u64().unwrap_or(0);
            s.status = match (github, ahead) {
                (true, 0) => "Waiting for a machine on GitHub...".to_owned(),
                _ => format!("In line: {ahead} before yours"),
            };
            s.building_since = None;
        }
        Some("building") => {
            let seconds = status["seconds"].as_u64().unwrap_or(0);
            if s.building_since.is_none() {
                s.building_since = Instant::now().checked_sub(Duration::from_secs(seconds));
            }
        }
        Some("done") => return Ok(Some(status)),
        Some("failed") => {
            return Err(status["error"]
                .as_str()
                .unwrap_or("The build failed.")
                .to_owned());
        }
        _ => return Err("The build server lost the job. Try again.".into()),
    }
    Ok(None)
}

/// A job's answer from a service that doesn't say where its answer is (a player's own server, an
/// older one): asked again [`POLL`] after each answer, as before.
#[allow(clippy::too_many_arguments)]
fn wait_on_service(
    agent: &ureq::Agent,
    server: &str,
    job: &str,
    github: bool,
    started: Instant,
    longest: Duration,
    shared: &Mutex<Shared>,
    asked: &mut u32,
) -> Result<serde_json::Value, String> {
    let mut quiet = 0;
    loop {
        if started.elapsed() > longest {
            return Err("The build took too long. Try again later.".into());
        }
        std::thread::sleep(POLL);
        *asked += 1;
        let looked = agent
            .get(&format!("{server}/v1/job/{job}"))
            .call()
            .map_err(service_error)
            .and_then(|r| {
                r.into_json::<serde_json::Value>()
                    .map_err(|e| e.to_string())
            });
        match looked {
            Ok(status) => {
                quiet = 0;
                if let Some(done) = told(status, github, shared)? {
                    return Ok(done);
                }
            }
            Err(e) => {
                quiet += 1;
                if quiet >= QUIET_TRIES {
                    return Err(e);
                }
            }
        }
    }
}

/// What release `tag` of `repo` has of job `job` (`<job>.json`, else `<job>.failed.json`), from
/// GitHub's downloads: no request to the service, and none GitHub counts against a limit.
fn answer_in(
    agent: &ureq::Agent,
    repo: &str,
    tag: &str,
    job: &str,
) -> Result<Option<(store::Answer, serde_json::Value)>, String> {
    for name in [format!("{job}.json"), format!("{job}.failed.json")] {
        match agent.get(&store::download(repo, tag, &name)).call() {
            Ok(response) => {
                let json: serde_json::Value = response.into_json().map_err(|e| e.to_string())?;
                let answer = store::Answer::read(&json)
                    .ok_or_else(|| format!("{name} says no state the game knows"))?;
                return Ok(Some((answer, json)));
            }
            Err(ureq::Error::Status(404, _)) => {}
            Err(e) => return Err(service_error(e)),
        }
    }
    Ok(None)
}

/// A job's answer, waited for where the service puts it (`repo`'s results releases), 15 s and
/// then longer between looks ([`store::wait`]), and the release it is in. The service itself is
/// asked every [`store::ASK_SERVICE_EVERY`] (where a job waits in line, whether its run ended
/// without an answer): a build of four minutes asks it nothing after the build is asked for,
/// where the game asked every 27 s before.
#[allow(clippy::too_many_arguments)]
fn wait_on_github(
    agent: &ureq::Agent,
    server: &str,
    repo: &str,
    job: &str,
    github: bool,
    started: Instant,
    longest: Duration,
    shared: &Mutex<Shared>,
    asked: &mut u32,
) -> Result<(serde_json::Value, String), String> {
    let tags = store::results_tags(job);
    {
        let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
        s.status = "Compiling...".into();
        s.building_since = Some(Instant::now());
    }
    // A failure there at the first look that may pass is the last build's, and the service builds
    // the job again (a failure for good it answers as it is): only a failure since is this one's.
    let mut stale = None;
    let mut looks = 0u32;
    let mut quiet = 0;
    let mut service_asked = Instant::now();
    loop {
        if started.elapsed() > longest {
            return Err("The build took too long. Try again later.".into());
        }
        // The first look also asks the release of the first evening's builds; a new one goes to
        // its shard.
        let mut found = None;
        let mut failed_look = None;
        for tag in &tags[..if looks == 0 { 2 } else { 1 }] {
            match answer_in(agent, repo, tag, job) {
                Ok(Some(answer)) => {
                    found = Some((answer, tag.clone()));
                    break;
                }
                Ok(None) => {}
                Err(e) => failed_look = Some(e),
            }
        }
        match (found, failed_look) {
            (Some(((store::Answer::Done { .. }, json), tag)), _) => return Ok((json, tag)),
            (Some(((store::Answer::Failed { error, settled, at }, _), _)), _) => {
                if looks == 0 && !settled {
                    stale = Some(at);
                } else if settled || Some(at) != stale {
                    return Err(error);
                }
                quiet = 0;
            }
            (None, Some(e)) => {
                quiet += 1;
                log::warn!("Horizon: no look at the build's answer on GitHub ({e})");
                if quiet >= QUIET_TRIES {
                    return Err(e);
                }
            }
            (None, None) => quiet = 0,
        }
        let mut wait = store::wait(looks);
        if service_asked.elapsed() >= store::ASK_SERVICE_EVERY {
            service_asked = Instant::now();
            *asked += 1;
            let status = agent
                .get(&format!("{server}/v1/job/{job}"))
                .call()
                .map_err(|e| match e {
                    ureq::Error::Status(404, _) => {
                        Some("The build server lost the job. Try again.".to_owned())
                    }
                    // Can't tell now: GitHub is looked at again, and the service later.
                    _ => None,
                });
            match status {
                Ok(response) => {
                    if let Ok(status) = response.into_json::<serde_json::Value>()
                        && told(status, github, shared)?.is_some()
                    {
                        // Done: its answer is on GitHub, looked at at once.
                        wait = Duration::ZERO;
                    }
                }
                Err(Some(lost)) => return Err(lost),
                Err(None) => {}
            }
        }
        std::thread::sleep(wait);
        looks += 1;
    }
}

/// This version's game compiled with the folder's mods on the build service, or on GitHub's machines
/// through it, downloaded and chosen for the next start: the folder's name.
fn compile(version: &str, github: bool, shared: &Mutex<Shared>) -> Result<String, String> {
    if !SERVICE_SUPPORTED {
        return Err(unsupported());
    }
    let say = |text: String| {
        let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
        s.status = text;
        s.building_since = None;
    };
    let for_development = std::env::var("HORIZON_BUILD_SERVER").ok();
    let custom = for_development.is_none().then(custom_server).flatten();
    // Starling's own service builds in Starling's own repository, also before its answer says so.
    let own_service = for_development.is_none() && custom.is_none();
    let server = for_development
        .or_else(|| custom.clone())
        .unwrap_or_else(|| published_server(version));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(40))
        .build();
    say(if github {
        "Sending the mod to GitHub...".into()
    } else {
        "Sending the mod to the build server...".into()
    });
    let body = bundles_body()?;
    let via = if github { "&via=github" } else { "" };
    let longest = if github { LONGEST_ON_GITHUB } else { LONGEST };
    let started = Instant::now();
    // Every request to the service, for the log: its free plan counts them.
    let mut asked = 0u32;
    // Compile v2: a library per mod from the store, when this APK plays one; else (or when the
    // store can't play these mods) the whole game with them, as before.
    if own_service && loads_store_builds() {
        match compile_from_store(&server, &agent, shared, &mut asked)? {
            FromStore::Built => return Ok(NATIVE_LIBS.to_owned()),
            // The whole-game Compile builds no game made of libraries: sent there, one mod the
            // store can't play failed every mod with "That is not a version of the game"
            // (2026-11-20). The player turns that one off instead.
            FromStore::NotHere(why) => return Err(why),
        }
    }
    // The machine that builds sleeps until it is asked: the gateway says so while it wakes.
    let (job, repo) = loop {
        asked += 1;
        let answer: serde_json::Value = agent
            .post(&format!("{server}/v1/build?version={version}{via}"))
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&body)
            .map_err(service_error)?
            .into_json()
            .map_err(|e| e.to_string())?;
        if answer["starting"].as_bool() != Some(true) {
            let repo = answer["repo"]
                .as_str()
                .and_then(store::plain_repo)
                .map(str::to_owned)
                .or_else(|| own_service.then(|| store::STORE_REPO.to_owned()));
            break (answer["job"].as_str().unwrap_or_default().to_owned(), repo);
        }
        if started.elapsed() > WAKING {
            return Err("The build server did not wake up. Try again later.".into());
        }
        say("Waking the build server up: about a minute...".into());
        std::thread::sleep(POLL * 3);
    };
    if !plain(&job) {
        return Err("The build server gave no job.".into());
    }
    let (finished, url) = match &repo {
        // The answer on GitHub, where the service puts it: the service is asked only now and then.
        Some(repo) => {
            let (finished, tag) = wait_on_github(
                &agent, &server, repo, &job, github, started, longest, shared, &mut asked,
            )?;
            (finished, store::download(repo, &tag, &format!("{job}.so")))
        }
        None => {
            let finished = wait_on_service(
                &agent, &server, &job, github, started, longest, shared, &mut asked,
            )?;
            asked += 1;
            (finished, format!("{server}/v1/job/{job}/lib"))
        }
    };
    let want = finished["sha256"].as_str().unwrap_or_default().to_owned();
    if !releases::native_build_matches(&finished["build"], releases::process_abi()) {
        return Err(horizon_client::lang::tr(
            "starling.native_builds.wrong_architecture",
            &[],
        ));
    }
    let size = finished["size"].as_u64().unwrap_or(0);
    if want.len() != 64 || size == 0 {
        return Err("The build does not say its SHA-256.".into());
    }
    say("Downloading...".into());
    let name = format!("{COMPILED}{job}");
    let folder = dir().join(&name);
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let part = folder.join(format!("{LIBRARY}.part"));
    log::info!(
        "Horizon: compiled job {job} answered after {} s, with {asked} requests to the build \
         service",
        started.elapsed().as_secs()
    );
    fetch_library(&agent, &url, &part, size, &want, shared)?;
    // A rename: a library this game runs stays mapped as it was.
    std::fs::rename(&part, folder.join(LIBRARY)).map_err(|e| e.to_string())?;
    std::fs::write(folder.join("for"), version).map_err(|e| e.to_string())?;
    std::fs::write(
        folder.join("build.json"),
        finished["build"].to_string().as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    // What it was compiled from: a mod replaced under the same id makes it out of date.
    let sent: String = Sha256::digest(&body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(folder.join(SENT), sent).map_err(|e| e.to_string())?;
    if let Some(address) = &custom {
        std::fs::write(folder.join(CAME_FROM), releases::build_server_host(address))
            .map_err(|e| e.to_string())?;
    }
    stamp_build(&folder)?;
    std::fs::write(dir().join("current"), &name).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(dir().join("starts"));
    // The compiled builds before it: 60 MB each.
    for entry in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let other = entry.file_name().to_string_lossy().into_owned();
        if other.starts_with(COMPILED) && other != name {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(name)
}

impl Game<'_> {
    /// The Mods page's native mods not in this game: on until turned off, they go into the next
    /// Compile, which the page's top offers (`native_game`), made here once every row is listed.
    pub(super) fn offer_compile(&mut self) {
        for row in &mut self.settings.mods.installed {
            if row.problem.as_deref() == Some(horizon_client::mod_list::NOT_BUILT) {
                if !SERVICE_SUPPORTED {
                    row.problem = Some(unsupported());
                    row.native = None;
                    continue;
                }
                row.native = Some(horizon_client::mod_list::NativePage::default());
            }
        }
        self.settings.mods.native_game = self.native_game();
    }

    /// The Native mods row is gone: the Mods page's top says which game runs (`native_game`).
    pub(super) fn list_native_builds_row(&mut self) {
        self.settings.mods.installed.retain(|row| row.id != ID);
    }

    /// What the Mods page says above the mods: which game runs, which the next start runs, whether
    /// that build still loads, whether the native mods turned on are the ones in it, and the
    /// buttons for each. None without native mods, a build or a chosen one.
    fn native_game(&self) -> Option<NativeGame> {
        if !SERVICE_SUPPORTED {
            return None;
        }
        let (version, _) = crate::android::app_version()?;
        let mods = &self.settings.mods;
        let now = running();
        let next = chosen();
        let native_rows = mods
            .installed
            .iter()
            .any(|m| m.native.is_some() && !m.id.is_empty());
        if now.is_none() && next.is_none() && !native_rows {
            return None;
        }
        let wanted = folder_mods_on(&mods.disabled);
        let compiling = self
            .native_builds
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .compiling;
        let mut game = NativeGame::default();
        let running_build = now.as_deref().map(on_phone);
        // Compile v2's: the mods this start opened from their own libraries.
        let opened: BTreeSet<String> = horizon_client::android::native_libs_opened()
            .iter()
            .map(|id| (*id).to_owned())
            .collect();
        let said = match &running_build {
            Some(b) => tr(
                "starling.mods.native.now_build",
                &[&version, &b.label(), &b.ago()],
            ),
            None if !opened.is_empty() => tr(
                "starling.mods.native.now_libs",
                &[&version, &listed(&opened)],
            ),
            None => tr("starling.mods.native.now_apk", &[&version]),
        };
        game.lines.push((said, false));
        // The next start's: the chosen build when it loads (NativeMods.chosen), else the APK's with
        // the libraries of Compile v2, if any.
        let next_loads = next.clone().filter(|name| loads(name, &version));
        let next_build = next_loads.as_deref().map(on_phone);
        let next_libs = match next_build {
            Some(_) => BTreeSet::new(),
            None => native_libs_mods().unwrap_or_default(),
        };
        if next_loads != now || next_libs != opened {
            let said = match &next_build {
                Some(b) => tr("starling.mods.native.next_build", &[&b.label(), &b.ago()]),
                None if !next_libs.is_empty() => {
                    tr("starling.mods.native.next_libs", &[&listed(&next_libs)])
                }
                None => tr("starling.mods.native.next_apk", &[]),
            };
            game.lines.push((said, false));
        }
        let old = next.as_deref().filter(|_| next_loads.is_none());
        if let Some(name) = old {
            let said = tr(
                "starling.mods.native.out_of_date",
                &[&on_phone(name).label()],
            );
            game.lines.push((said, true));
        }
        // The next start's build is current only for the mods as they are now: one replaced under
        // the same id needs a new Compile (`mod_list::native_build_current`).
        let request = (!wanted.is_empty()).then(request_digest).flatten();
        // Or, with no build chosen, the libraries the next start opens (Compile v2).
        let next_current = next_build
            .as_ref()
            .is_some_and(|b| b.current_for(&wanted, request.as_deref()))
            || (next.is_none() && native_libs_current(&wanted, request.as_deref()));
        if compiling {
            game.lines
                .push((tr("starling.mods.native.compiling", &[]), false));
            game.buttons.push((
                tr("starling.mods.native.show_compile", &[]),
                NativeGameAct::ShowCompile,
            ));
        } else if !wanted.is_empty() && (old.is_some() || !next_current) {
            if old.is_none() {
                let key = if next_loads.is_some() {
                    "starling.mods.native.differs"
                } else {
                    "starling.mods.native.not_in_game"
                };
                game.lines.push((tr(key, &[]), false));
            }
            // One Compile for every native mod chosen below; its card offers GitHub and the build
            // server.
            game.buttons.push((
                tr("starling.mods.native.compile", &[]),
                NativeGameAct::Compile,
            ));
        }
        if next_loads != now {
            game.buttons.push((
                tr("starling.mods.native.restart", &[]),
                NativeGameAct::Restart,
            ));
        }
        if next.is_some() {
            game.buttons.push((
                tr("starling.mods.native.play_without", &[]),
                NativeGameAct::PlayWithout,
            ));
        }
        Some(game)
    }

    fn refresh_native_game(&mut self) {
        self.settings.mods.native_game = self.native_game();
    }

    /// Mod `i` was turned on or off on the Mods page. A native one changes the next start: with
    /// no native mod on, the APK's own game; with the set of a build already on the phone, that
    /// build. Another set waits for a Compile, which the page offers.
    pub(super) fn native_mod_toggled(&mut self, i: usize) {
        let mods = &self.settings.mods;
        if !SERVICE_SUPPORTED || !mods.installed.get(i).is_some_and(|m| m.native.is_some()) {
            return;
        }
        let any_on = mods
            .installed
            .iter()
            .enumerate()
            .any(|(j, m)| m.native.is_some() && !m.id.is_empty() && mods.is_on(j));
        // The libraries the next start opens follow the mods on, from those already here.
        native_libs_follow();
        if !any_on {
            let _ = std::fs::remove_file(dir().join("current"));
            let _ = std::fs::remove_file(dir().join("starts"));
        } else if let Some((version, _)) = crate::android::app_version()
            && let Some(name) = build_with(
                &folder_mods_on(&mods.disabled),
                request_digest().as_deref(),
                &version,
            )
            && chosen().as_deref() != Some(name.as_str())
        {
            let _ = std::fs::write(dir().join("current"), name);
            let _ = std::fs::remove_file(dir().join("starts"));
        }
        self.refresh_native_game();
    }

    /// A button of the Mods page's game with native mods.
    pub(super) fn native_game_act(&mut self, act: NativeGameAct) {
        match act {
            NativeGameAct::Compile | NativeGameAct::CompileOnGithub => {
                let wanted: Vec<String> = folder_mods_on(&self.settings.mods.disabled)
                    .into_iter()
                    .collect();
                let card = &mut self.native_builds;
                let compiling = card
                    .shared
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .compiling;
                // The warning first: the button pressed is its first.
                card.on_github = act == NativeGameAct::CompileOnGithub;
                card.stage = Some(if compiling {
                    Stage::Compiling
                } else {
                    Stage::Warn(mods_label(&wanted))
                });
            }
            NativeGameAct::ShowCompile => self.native_builds.stage = Some(Stage::Compiling),
            NativeGameAct::Restart => restart_with_built_mods(),
            NativeGameAct::PlayWithout => {
                let _ = std::fs::remove_file(dir().join("current"));
                let _ = std::fs::remove_file(dir().join("starts"));
            }
            NativeGameAct::BuildServer => {
                crate::android::ask_build_server(custom_server().as_deref().unwrap_or(""));
            }
        }
        self.refresh_native_game();
    }

    /// Shows the card: the builds for this version, looked up now.
    pub(super) fn open_native_builds(&mut self) {
        if !SERVICE_SUPPORTED {
            return;
        }
        self.native_builds.compile_failed = false;
        let Some((version, _)) = crate::android::app_version() else {
            return;
        };
        let card = &mut self.native_builds;
        card.stage = Some(Stage::Loading);
        let shared = card.shared.clone();
        shared.lock().unwrap_or_else(PoisonError::into_inner).builds = None;
        let _ = std::thread::Builder::new()
            .name("native builds".into())
            .spawn(move || {
                let builds = list(&version);
                if let Err(e) = &builds {
                    log::info!("Horizon: no native mod builds listed: {e}");
                }
                shared.lock().unwrap_or_else(PoisonError::into_inner).builds = Some(builds);
            });
    }

    /// The activity's notice, once: a build it set aside (NativeMods.java), on the title screen.
    pub(super) fn poll_native_builds(&mut self) {
        if self.native_builds.stage == Some(Stage::Compiling) {
            self.native_builds.fishing.update(Instant::now());
            let [w, h] = self.touch_size();
            let d = density(w as f32, h as f32);
            self.native_builds
                .mining
                .prepare(w as f32 / d, h as f32 / d);
            let mut sounds = self.native_builds.fishing.take_sounds();
            sounds.extend(self.native_builds.mining.take_sounds());
            for sound in sounds {
                self.audio
                    .play_samples(sound.samples(), super::setup::fishing::RATE, 0.3);
            }
        }
        let card = &mut self.native_builds;
        if !card.noticed.get() && self.title.is_some() && card.stage.is_none() {
            card.noticed.set(true);
            let notice = dir().join("notice");
            if let Ok(text) = std::fs::read_to_string(&notice) {
                let _ = std::fs::remove_file(&notice);
                card.stage = Some(Stage::Failed(text.trim().to_owned()));
            }
        }
        let mut shared = card.shared.lock().unwrap_or_else(PoisonError::into_inner);
        if card.stage == Some(Stage::Loading)
            && let Some(builds) = &shared.builds
        {
            card.stage = Some(match builds {
                Ok(_) => Stage::List,
                Err(e) => Stage::Failed(e.clone()),
            });
        }
        let mut ended = false;
        if (matches!(card.stage, Some(Stage::Downloading(_))) || shared.compiling)
            && let Some(result) = shared.downloaded.take()
        {
            card.compile_failed = shared.compiling && result.is_err();
            shared.compiling = false;
            card.stage = Some(match result {
                Ok(name) => Stage::Chosen(name),
                Err(e) => Stage::Failed(e),
            });
            ended = true;
        }
        drop(shared);
        // The Mods page says the new build.
        if ended {
            self.refresh_native_game();
        }
    }

    pub(super) fn native_builds_shown(&self) -> bool {
        self.native_builds.stage.is_some()
    }

    /// A card in the middle of the screen, the rest dimmed, as the update's.
    pub(super) fn draw_native_builds(&self, batch: &mut HudBatch) {
        let Some(stage) = self.native_builds.stage.clone() else {
            return;
        };
        if stage == Stage::Compiling {
            self.draw_compiling(batch);
            return;
        }
        let (builds, done, total) = {
            let shared = self
                .native_builds
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let builds = match &shared.builds {
                Some(Ok(b)) => b.clone(),
                _ => Vec::new(),
            };
            (builds, shared.done, shared.total)
        };
        let label = |name: &str| {
            builds.iter().find(|b| b.name == name).map_or_else(
                || match (name.starts_with(COMPILED), came_from(name)) {
                    (true, Some(host)) => format!("your native mods (from {host})"),
                    (true, None) => "your native mods".to_owned(),
                    (false, _) => name.to_owned(),
                },
                |b| mods_label(&b.mods),
            )
        };
        let current = chosen();
        let now = running();
        let custom = custom_server();
        let custom_host = custom.as_deref().map(releases::build_server_host);
        let mut acts: Vec<(String, Act)> = Vec::new();
        // Said in red, above the rest.
        let mut danger = None;
        let (title, body) = match &stage {
            Stage::Loading => ("Native mods".to_owned(), "Looking for builds...".to_owned()),
            Stage::List => {
                for b in builds.iter().take(SHOWN) {
                    let text = if current.as_deref() == Some(&b.name) {
                        format!("{} (chosen)", mods_label(&b.mods))
                    } else {
                        format!("Play with {}", mods_label(&b.mods))
                    };
                    acts.push((text, Act::Use(b.clone())));
                }
                if current.is_some() {
                    acts.push(("Play without native mods".into(), Act::StopUsing));
                }
                acts.push(("Build server...".into(), Act::Server));
                danger = custom_host.map(|host| {
                    format!(
                        "Compiling uses another build server: {host}. We take no responsibility \
                         for it."
                    )
                });
                let playing = match now {
                    Some(n) => format!("This game has {}. ", label(&n)),
                    None => String::new(),
                };
                let found = if builds.is_empty() {
                    "No builds for this version yet."
                } else {
                    "Each is this version's game with those mods compiled in."
                };
                (
                    "Native mods".to_owned(),
                    format!(
                        "{playing}{found} Native mods run with the game's full access. To add \
                         one, open its .hnmod with Starling and press Compile on its row."
                    ),
                )
            }
            Stage::Downloading(name) => (
                format!("Downloading {}", label(name)),
                format!(
                    "{:.0} of {:.0} MB",
                    done as f64 / 1e6,
                    total.max(done) as f64 / 1e6
                ),
            ),
            Stage::Chosen(name) => {
                acts.push(("Restart now".into(), Act::Restart));
                danger = came_from(name).map(|host| {
                    format!("This build came from {host}, not from Starling's server.")
                });
                (
                    if name.starts_with(COMPILED) || name == NATIVE_LIBS {
                        "Your native mods are ready".to_owned()
                    } else {
                        format!("{} is ready", label(name))
                    },
                    "It plays from the next start. If it closes before it shows, Starling starts \
                     without it the time after."
                        .into(),
                )
            }
            Stage::Failed(e) => {
                acts.push(("Try again".into(), Act::Retry));
                if self.native_builds.compile_failed && custom_host.is_none() {
                    acts.push((USE_GITHUB.into(), Act::CompileOnGithub));
                }
                ("Native mods".to_owned(), e.clone())
            }
            Stage::Warn(id) => {
                let github = custom_host
                    .is_none()
                    .then(|| (USE_GITHUB.to_owned(), Act::CompileOnGithub));
                let compile = ("Compile".to_owned(), Act::Compile);
                // Use GitHub pressed on the Mods page: it comes first.
                match github {
                    Some(github) if self.native_builds.on_github => {
                        acts.push(github);
                        acts.push(compile);
                    }
                    github => {
                        acts.push(compile);
                        acts.extend(github);
                    }
                }
                acts.push(("Build server...".into(), Act::Server));
                danger = custom_host.map(|host| {
                    format!(
                        "{host} is not Starling's server. The game it sends back runs with full \
                         access, including your worlds and your signed-in account. We take no \
                         responsibility for it."
                    )
                });
                let on_github = if custom_host.is_none() {
                    " Use GitHub compiles on GitHub's machines instead: it takes longer, and its \
                     log is public."
                } else {
                    ""
                };
                let server = custom_host.map_or_else(
                    || "Starling's build server".to_owned(),
                    |host| format!("the build server at {host}"),
                );
                // Compile v2 (`compile`): with Starling's own service and an APK that plays a
                // library per mod, each comes from the mod store, built once for this game.
                let how = if custom_host.is_none() && loads_store_builds() {
                    format!(
                        "Starling downloads a library of each native mod turned on, made for \
                         this game, from its mod store; {server} compiles the ones the store \
                         doesn't have yet, in a few minutes. Then the game restarts with them."
                    )
                } else {
                    format!(
                        "It is sent to {server}, which compiles the game with your native mods \
                         in a few minutes; then the game restarts with them."
                    )
                };
                (
                    format!("Compile {id}?"),
                    format!(
                        "A native mod runs with the game's full access: it can do anything the \
                         game can on this phone, so only compile mods you trust. {how} If it \
                         closes before it shows, Starling starts without it the time \
                         after.{on_github}"
                    ),
                )
            }
            Stage::Compiling => unreachable!("drawn by draw_compiling"),
        };
        acts.push(("Close".into(), Act::Close));
        let mut ui = Ui::new(&self.gui.atlas, batch, (-1.0, -1.0));
        let (w, h) = (ui.w, ui.h);
        ui.fill(0.0, 0.0, w, h, color::with_alpha(color::bg(), 0.72));
        let card_w = (w - 48.0).min(470.0);
        let pad = 20.0;
        let lines = wrapped(&ui, Face::Regular, 13.0, card_w - 2.0 * pad, &body);
        let danger_lines = danger.as_deref().map_or_else(Vec::new, |text| {
            wrapped(&ui, Face::Semibold, 14.0, card_w - 2.0 * pad, text)
        });
        let danger_h = if danger_lines.is_empty() {
            0.0
        } else {
            danger_lines.len() as f32 * 20.0 + 8.0
        };
        let bar = matches!(stage, Stage::Downloading(_));
        let (button_h, gap) = (34.0, 8.0);
        let card_h = pad
            + 24.0
            + 10.0
            + danger_h
            + lines.len() as f32 * 19.0
            + if bar { 20.0 } else { 0.0 }
            + 14.0
            + acts.len() as f32 * (button_h + gap)
            - gap
            + pad;
        let (x, y) = ((w - card_w) / 2.0, ((h - card_h) / 2.0).max(4.0));
        ui.rounded(x, y, card_w, card_h, 14.0, color::bg());
        ui.border(x, y, card_w, card_h, 14.0, color::border());
        ui.text(
            Face::Semibold,
            17.0,
            x + pad,
            y + pad,
            color::text(),
            &title,
        );
        let mut ty = y + pad + 24.0 + 10.0;
        for line in &danger_lines {
            ui.text(Face::Semibold, 14.0, x + pad, ty, color::DANGER, line);
            ty += 20.0;
        }
        if !danger_lines.is_empty() {
            ty += 8.0;
        }
        for line in &lines {
            ui.text(Face::Regular, 13.0, x + pad, ty, color::MUTED, line);
            ty += 19.0;
        }
        if bar {
            let bw = card_w - 2.0 * pad;
            let f = if total > 0 {
                (done as f32 / total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.rounded(x + pad, ty + 6.0, bw, 8.0, 4.0, color::raised());
            ui.rounded(
                x + pad,
                ty + 6.0,
                (bw * f).max(8.0),
                8.0,
                4.0,
                color::ACCENT,
            );
            ty += 20.0;
        }
        let mut by = ty + 14.0;
        let mut drawn = Vec::new();
        for (i, (text, act)) in acts.into_iter().enumerate() {
            let rect = [x + pad, by, card_w - 2.0 * pad, button_h];
            let (fill, ink) = if i == 0 && !matches!(act, Act::Close | Act::Server) {
                (color::ACCENT, color::ON_ACCENT)
            } else {
                (color::raised(), color::text())
            };
            ui.rounded(rect[0], rect[1], rect[2], rect[3], 10.0, fill);
            ui.text_centered(
                Face::Semibold,
                14.0,
                rect[0] + rect[2] / 2.0,
                rect[1] + (rect[3] - 14.0) / 2.0 - 2.0,
                ink,
                &text,
            );
            drawn.push((rect, act));
            by += button_h + gap;
        }
        ui.finish();
        *self.native_builds.buttons.borrow_mut() = drawn;
    }

    /// The water over the screen while the build goes on: what it says, and a way out of it.
    fn draw_compiling(&self, batch: &mut HudBatch) {
        let status = {
            let s = self
                .native_builds
                .shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match s.building_since {
                Some(since) => {
                    let seconds = since.elapsed().as_secs();
                    format!("Compiling: {}:{:02}", seconds / 60, seconds % 60)
                }
                None => s.status.clone(),
            }
        };
        let now = Instant::now();
        let card = &self.native_builds;
        let mut ui = Ui::new(&self.gui.atlas, batch, (-1.0, -1.0));
        let (w, h) = (ui.w, ui.h);
        if card.digging {
            card.mining.draw(&mut ui, w, h, now);
        } else {
            card.fishing.draw_water(&mut ui, w, h, now);
            card.fishing.draw(&mut ui, w, h, h - 58.0, now);
        }
        let (pad, bar_h, hide_w) = (16.0, 44.0, 84.0);
        ui.rounded(pad, pad, w - 2.0 * pad, bar_h, 12.0, color::bg());
        ui.text(
            Face::Regular,
            14.0,
            pad + 14.0,
            pad + 13.0,
            color::text(),
            &status,
        );
        let rect = [w - pad - hide_w - 6.0, pad + 6.0, hide_w, bar_h - 12.0];
        ui.rounded(rect[0], rect[1], rect[2], rect[3], 8.0, color::raised());
        ui.text_centered(
            Face::Semibold,
            13.0,
            rect[0] + rect[2] / 2.0,
            rect[1] + (rect[3] - 13.0) / 2.0 - 2.0,
            color::text(),
            "Hide",
        );
        let other = [rect[0] - hide_w - 8.0, rect[1], hide_w, rect[3]];
        ui.rounded(other[0], other[1], other[2], other[3], 8.0, color::raised());
        ui.text_centered(
            Face::Semibold,
            13.0,
            other[0] + other[2] / 2.0,
            other[1] + (other[3] - 13.0) / 2.0 - 2.0,
            color::text(),
            if card.digging { "Fish" } else { "Mine" },
        );
        ui.finish();
        *self.native_builds.buttons.borrow_mut() = vec![(rect, Act::Close), (other, Act::Switch)];
    }

    /// While the card shows it takes every touch; a lifted finger on a button acts. Under a build
    /// a finger anywhere else fishes.
    pub(super) fn native_builds_touch(&mut self, touch: &Touch) -> bool {
        if !self.native_builds_shown() {
            return false;
        }
        let [w, h] = self.touch_size();
        let d = density(w as f32, h as f32);
        let at = [touch.location.x as f32 / d, touch.location.y as f32 / d];
        if self.native_builds.stage == Some(Stage::Compiling) {
            let on_button = self.native_builds.buttons.borrow().iter().any(|(r, _)| {
                at[0] >= r[0] && at[0] <= r[0] + r[2] && at[1] >= r[1] && at[1] <= r[1] + r[3]
            });
            let now = Instant::now();
            let digging = self.native_builds.digging;
            match touch.phase {
                TouchPhase::Started if !on_button && digging => {
                    self.native_builds
                        .mining
                        .press(at, w as f32 / d, h as f32 / d);
                }
                TouchPhase::Started if !on_button => self.native_builds.fishing.press(now, at),
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    if !digging {
                        self.native_builds.fishing.release(now);
                    }
                    if on_button {
                        self.native_builds_press(at);
                    }
                }
                _ => {}
            }
            return true;
        }
        if touch.phase != TouchPhase::Ended {
            return true;
        }
        self.native_builds_press(at);
        true
    }

    /// A press at `p` (UI units) on the card's buttons.
    pub(super) fn native_builds_press(&mut self, p: [f32; 2]) {
        let inside = |r: &[f32; 4]| {
            p[0] >= r[0] && p[0] <= r[0] + r[2] && p[1] >= r[1] && p[1] <= r[1] + r[3]
        };
        let act = self
            .native_builds
            .buttons
            .borrow()
            .iter()
            .find(|(r, _)| inside(r))
            .map(|(_, a)| a.clone());
        let Some(act) = act else {
            return;
        };
        let Some((version, _)) = crate::android::app_version() else {
            return;
        };
        let act = match act {
            Act::Retry if self.native_builds.compile_failed => {
                if self.native_builds.on_github {
                    Act::CompileOnGithub
                } else {
                    Act::Compile
                }
            }
            other => other,
        };
        match act {
            Act::Use(build) => {
                let card = &mut self.native_builds;
                card.stage = Some(Stage::Downloading(build.name.clone()));
                let shared = card.shared.clone();
                {
                    let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
                    s.done = 0;
                    s.total = 0;
                    s.downloaded = None;
                }
                let _ = std::thread::Builder::new()
                    .name("native build download".into())
                    .spawn(move || {
                        let result = download(&build, &version, &shared);
                        if let Err(e) = &result {
                            log::warn!("Horizon: the native mod build did not download: {e}");
                        }
                        shared
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .downloaded = Some(result);
                    });
            }
            Act::StopUsing => {
                let _ = std::fs::remove_file(dir().join("current"));
                self.native_builds.stage = Some(Stage::List);
            }
            Act::Restart => restart_with_built_mods(),
            Act::Retry => self.open_native_builds(),
            Act::Close => self.native_builds.stage = None,
            Act::Switch => self.native_builds.digging = !self.native_builds.digging,
            Act::Server => {
                crate::android::ask_build_server(custom_server().as_deref().unwrap_or(""));
            }
            act @ (Act::Compile | Act::CompileOnGithub) => {
                let github = act == Act::CompileOnGithub;
                let card = &mut self.native_builds;
                card.on_github = github;
                card.stage = Some(Stage::Compiling);
                let shared = card.shared.clone();
                {
                    let mut s = shared.lock().unwrap_or_else(PoisonError::into_inner);
                    if s.compiling {
                        return;
                    }
                    s.compiling = true;
                    s.downloaded = None;
                    s.status = String::new();
                }
                let _ = std::thread::Builder::new()
                    .name("native mod compile".into())
                    .spawn(move || {
                        let result = compile(&version, github, &shared);
                        if let Err(e) = &result {
                            log::warn!("Horizon: the native mods did not compile: {e}");
                        }
                        shared
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .downloaded = Some(result);
                    });
            }
        }
        // Play without, a compile begun: the Mods page says so.
        self.refresh_native_game();
    }
}

/// The restart into the game build with the native mods just compiled, saying so meanwhile.
fn restart_with_built_mods() {
    crate::android::restart_app_saying(&horizon_client::lang::get_or(
        "starling.restart.native_mods",
        "Starting the game with your native mods…",
    ));
}
