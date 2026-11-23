//! What a phone reads of Starling's mod build service straight from GitHub, so the service (a
//! Cloudflare Worker, whose free plan counts every request) is asked as little as can be
//! (game/starling/native_builds.rs, on Android). Mirrors starling-client/mod-builds:
//! worker/src/index.js and scripts/store.py.
//!
//! - Compile (v1), the game with all the native mods: the service answers a build with its job and
//!   the repository (`repo`) whose releases hold the answer, `<job>.json` or `<job>.failed.json` in
//!   `results-<job's first digit>`, and then `<job>.so`. The phone waits there ([`wait`]) and asks the
//!   service only every [`ASK_SERVICE_EVERY`], for a build that is long in coming.
//! - The store (v2), a library per mod: `store/builds.json` lists the game builds (each the commit
//!   a Nightly's APK is built from), and `mods-<build>/index.json` that build's mod libraries by
//!   the `.smod`'s SHA-256. A mod the index doesn't have is asked of the service
//!   (`POST /v2/mod`), and its answer waited for in `mods-<build>`. A library loads only in the
//!   game build it was made for.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// The repository Starling's own service builds in, when its answer doesn't name one.
pub const STORE_REPO: &str = "starling-client/mod-builds";
/// The store's own release, and its list of builds.
pub const STORE_TAG: &str = "store";
pub const BUILDS: &str = "builds.json";
/// A build's index, in its `mods-<build>` release.
pub const INDEX: &str = "index.json";
/// What the game's loader reads beside the libraries (sunrise-mod-host's native_libraries.rs).
pub const RECORD: &str = "native-mods.json";
/// How often the service is asked while GitHub has no answer: its line (where a job waits), and
/// whether the job's run ended without one.
pub const ASK_SERVICE_EVERY: Duration = Duration::from_secs(10 * 60);

/// The wait before look `n` (from 0) at GitHub for an answer: 15 s, growing to a minute. A build
/// takes four minutes or so (half of them in under 242 s, nine in ten under 15 min).
pub fn wait(n: u32) -> Duration {
    const STEPS: [u64; 4] = [15, 20, 30, 45];
    Duration::from_secs(STEPS.get(n as usize).copied().unwrap_or(60))
}

/// A download of asset `name` of release `tag` in `repo`.
pub fn download(repo: &str, tag: &str, name: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{name}")
}

/// `text` when it is an `owner/name` of GitHub, as the service says its repository.
pub fn plain_repo(text: &str) -> Option<&str> {
    let (owner, name) = text.split_once('/')?;
    let part = |p: &str| {
        !p.is_empty()
            && p.len() <= 100
            && !p.starts_with('.')
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (part(owner) && part(name)).then_some(text)
}

/// The releases a v1 job's answer may be in, in the order to look: its shard, then `results`,
/// which holds the builds of the first evening.
pub fn results_tags(job: &str) -> [String; 2] {
    let first = job.chars().next().unwrap_or('0');
    [format!("results-{first}"), "results".to_owned()]
}

/// What an answer file of the service says (`<job>.json` or `<job>.failed.json`).
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// The library is there: its SHA-256 and size, and the rest of what the file says.
    Done {
        sha256: String,
        size: u64,
        about: serde_json::Value,
    },
    /// No library: why, whether it is for good, and when the build said so (the service's clock),
    /// which tells a failure from before a build again from that build's own.
    Failed {
        error: String,
        settled: bool,
        at: Option<i64>,
    },
}

impl Answer {
    pub fn read(json: &serde_json::Value) -> Option<Answer> {
        match json["state"].as_str()? {
            "done" => {
                let sha256 = json["sha256"].as_str().filter(|s| hex(s, 64))?.to_owned();
                let size = json["size"].as_u64().filter(|&s| s > 0)?;
                Some(Answer::Done {
                    sha256,
                    size,
                    about: json.clone(),
                })
            }
            "failed" => Some(Answer::Failed {
                error: json["error"]
                    .as_str()
                    .unwrap_or("The build failed.")
                    .chars()
                    .take(600)
                    .collect(),
                settled: json["settled"].as_bool().unwrap_or(false),
                at: json["at"].as_i64(),
            }),
            _ => None,
        }
    }
}

fn hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A v2 job: the `.smod`'s SHA-256 (its first 32 digits) and the ABI (the Worker's `modJobOf`).
pub fn job_of(source: &str, abi: &str) -> String {
    format!("{}.{abi}", &source[..source.len().min(32)])
}

/// The release of game build `build` in the store.
pub fn mods_tag(build: &str) -> String {
    format!("mods-{build}")
}

/// One game build the store serves (`store/builds.json`): the commit an APK is built from.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Build {
    pub build: String,
    #[serde(default)]
    pub nightly: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    /// The Cargo profile its APK is built with, and its mods.
    #[serde(default)]
    pub profile: Option<String>,
    /// The ABIs the store builds its mods for.
    #[serde(default)]
    pub abis: BTreeMap<String, Abi>,
}

/// One ABI of a build in the store.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Abi {
    /// The Nightly's APK the store's build of the game was checked against.
    #[serde(default)]
    pub apk: Option<String>,
}

/// The builds of `store/builds.json`, newest first, without a row whose build isn't a commit.
pub fn builds(text: &str) -> Result<Vec<Build>, String> {
    #[derive(Deserialize)]
    struct File {
        schema: u32,
        #[serde(default)]
        builds: Vec<serde_json::Value>,
    }
    let file: File = serde_json::from_str(text).map_err(|e| format!("{BUILDS}: {e}"))?;
    if file.schema != 1 {
        return Err(format!(
            "{BUILDS}: schema {} is newer than this Starling",
            file.schema
        ));
    }
    Ok(file
        .builds
        .into_iter()
        .filter_map(|row| serde_json::from_value::<Build>(row).ok())
        .filter(|b| hex(&b.build, 40))
        .collect())
}

/// A mod's library in a build's index, or in its own answer file (`<job>.json`).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Library {
    pub job: String,
    pub id: String,
    pub version: String,
    /// The `.smod`'s SHA-256.
    pub source: String,
    pub abi: String,
    /// Its asset in `mods-<build>` (`<job>.so`).
    pub file: String,
    /// Its name on the phone, which native-mods.json gives the loader (`lib<crate>.so`).
    pub library: String,
    pub sha256: String,
    pub size: u64,
    /// What the library says it is (`sunrise_native_build`): "<build> <mod> <version> <rustc>".
    pub identity: String,
}

impl Library {
    /// Whether this row is one a phone may load for `build`: plain names, the job its source's,
    /// and an identity for that build.
    fn plain(&self, build: &str) -> bool {
        hex(&self.source, 64)
            && hex(&self.sha256, 64)
            && self.size > 0
            && self.job == job_of(&self.source, &self.abi)
            && self.file == format!("{}.so", self.job)
            && plain_file(&self.library)
            && self.library.starts_with("lib")
            && self.library.ends_with(".so")
            && self.identity.split(' ').next() == Some(build)
            && self.identity.split(' ').nth(1) == Some(self.id.as_str())
    }

    /// A library from its own answer file, once its build said it is done.
    pub fn from_answer(about: &serde_json::Value, build: &str) -> Option<Library> {
        let mut row = about.clone();
        let job = job_of(row["source"].as_str()?, row["abi"].as_str()?);
        row["job"] = job.clone().into();
        row["file"] = format!("{job}.so").into();
        let library: Library = serde_json::from_value(row).ok()?;
        library.plain(build).then_some(library)
    }
}

/// A mod that failed on a build, as its index says.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Failure {
    pub job: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub settled: bool,
}

/// A build's index (`mods-<build>/index.json`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Index {
    pub mods: BTreeMap<String, Library>,
    pub failed: BTreeMap<String, Failure>,
}

/// What a store's failure says of a mod only the whole-game Compile builds (mod-builds'
/// scripts/mod_lib.py).
const WHOLE_GAME: &str = "whole-game Compile";

/// The native mods only the whole-game Compile builds, by id, as `index` says: a library of its own
/// holds no mixin, nor a crate beside the mod API. A mod the store built some version of is not
/// one.
pub fn whole_game_only(index: &Index) -> BTreeSet<String> {
    let built: BTreeSet<&str> = index.mods.values().map(|l| l.id.as_str()).collect();
    index
        .failed
        .values()
        .filter(|f| {
            // Starling: and those that failed for good in this build (a mod written for Horizon's
            // API that neither compiles here nor has a port): offering one ends in an error.
            (f.settled || f.error.contains(WHOLE_GAME))
                && !f.id.is_empty()
                && !built.contains(f.id.as_str())
        })
        .map(|f| f.id.clone())
        .collect()
}

/// The mods the catalogs leave out on this game: those Compile v2 can't play, and any other the
/// store's index lists as failed for good (a Horizon WebAssembly mod, say)
/// ([`whole_game_only`]) in this APK's own build, read from GitHub once. It blocks: the catalogs
/// ask from their search threads. None for a game that doesn't say its build, or without the
/// network: the catalogs then list every mod, as before.
pub fn catalog_hidden() -> &'static BTreeSet<String> {
    static HIDDEN: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    HIDDEN.get_or_init(|| {
        let Some(build) = option_env!("SUNRISE_ANDROID_BUILD_COMMIT").filter(|c| c.len() == 40)
        else {
            return BTreeSet::new();
        };
        let text = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(20))
            .build()
            .get(&download(STORE_REPO, &mods_tag(build), INDEX))
            .call()
            .ok()
            .and_then(|response| response.into_string().ok());
        text.and_then(|text| index(&text, build).ok())
            .map(|index| whole_game_only(&index))
            .unwrap_or_default()
    })
}

/// The index of `build`, its rows by job; a row a phone shouldn't load is left out.
pub fn index(text: &str, build: &str) -> Result<Index, String> {
    #[derive(Deserialize)]
    struct File {
        schema: u32,
        build: String,
        #[serde(default)]
        mods: Vec<serde_json::Value>,
        #[serde(default)]
        failed: Vec<serde_json::Value>,
    }
    let file: File = serde_json::from_str(text).map_err(|e| format!("{INDEX}: {e}"))?;
    if file.schema != 1 || file.build != build {
        return Err(format!("{INDEX}: not build {build}'s"));
    }
    let mods = file
        .mods
        .into_iter()
        .filter_map(|row| serde_json::from_value::<Library>(row).ok())
        .filter(|l| l.plain(build))
        .map(|l| (l.job.clone(), l))
        .collect();
    let failed = file
        .failed
        .into_iter()
        .filter_map(|row| serde_json::from_value::<Failure>(row).ok())
        .map(|f| (f.job.clone(), f))
        .collect();
    Ok(Index { mods, failed })
}

/// A native mod the phone has on: its id and its `.smod`'s SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Wanted {
    pub id: String,
    pub source: String,
}

/// Where the mods on can play.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Build `build` has every library.
    Ready {
        build: Build,
        libraries: Vec<Library>,
    },
    /// Build `build` has some (`have`); the service is asked for the rest (`missing`).
    Ask {
        build: Build,
        have: Vec<Library>,
        missing: Vec<Wanted>,
    },
    /// Every build has a mod that fails there for good: the first such, and why.
    Fails { id: String, error: String },
    /// The store has no build for this ABI.
    NoBuild,
}

/// Where the mods `wanted` play on `abi`: this game's own build (`own`) when the store has every
/// library for it, else the newest build that has, else the build (own, or the newest) the service
/// is asked to fill. A build where a mod fails for good is passed over: that is how a mod that
/// needs a newer Nightly moves the game to it. `index_of` reads a build's index (from GitHub) only
/// when it is needed, own first.
pub fn plan(
    builds: &[Build],
    mut index_of: impl FnMut(&Build) -> Option<Index>,
    wanted: &[Wanted],
    abi: &str,
    own: Option<&str>,
) -> Plan {
    let mut order: Vec<&Build> = builds.iter().filter(|b| b.abis.contains_key(abi)).collect();
    if order.is_empty() {
        return Plan::NoBuild;
    }
    // Own build first, then newest first as builds.json lists them.
    order.sort_by_key(|b| own != Some(b.build.as_str()));
    let wanted: BTreeSet<&Wanted> = wanted.iter().collect();
    let mut fillable: Option<(&Build, Vec<Library>, Vec<Wanted>)> = None;
    let mut first_failure = None;
    for build in order {
        let index = index_of(build).unwrap_or_default();
        let mut have = Vec::new();
        let mut missing = Vec::new();
        let mut fails = None;
        for w in &wanted {
            let job = job_of(&w.source, abi);
            if let Some(library) = index.mods.get(&job) {
                have.push(library.clone());
            } else if let Some(f) = index.failed.get(&job).filter(|f| f.settled) {
                fails = Some((w.id.clone(), f.error.clone()));
            } else {
                missing.push((*w).clone());
            }
        }
        if let Some(failure) = fails {
            first_failure.get_or_insert(failure);
            continue;
        }
        if missing.is_empty() {
            return Plan::Ready {
                build: build.clone(),
                libraries: have,
            };
        }
        if fillable.is_none() {
            fillable = Some((build, have, missing));
        }
    }
    match (fillable, first_failure) {
        (Some((build, have, missing)), _) => Plan::Ask {
            build: build.clone(),
            have,
            missing,
        },
        (None, Some((id, error))) => Plan::Fails { id, error },
        (None, None) => Plan::NoBuild,
    }
}

/// native-mods.json for `libraries`, as the game's loader reads it beside them.
pub fn record(libraries: &[Library]) -> String {
    let rows: Vec<serde_json::Value> = libraries
        .iter()
        .map(|l| {
            serde_json::json!({
                "file": l.library,
                "sha256": l.sha256,
                "mod": l.id,
                "identity": l.identity,
            })
        })
        .collect();
    serde_json::json!({ "libraries": rows }).to_string()
}

/// A plain file name: no folder, nothing hidden.
pub fn plain_file(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILD: &str = "5e1f0c2d3a4b5c6d7e8f90a1b2c3d4e5f6a7b8c9";
    const NEWER: &str = "6e1f0c2d3a4b5c6d7e8f90a1b2c3d4e5f6a7b8c9";

    fn build(commit: &str) -> Build {
        Build {
            build: commit.into(),
            nightly: None,
            date: None,
            profile: None,
            abis: [("arm64-v8a".to_owned(), Abi::default())].into(),
        }
    }

    fn library(id: &str, source: &str, commit: &str) -> Library {
        let job = job_of(source, "arm64-v8a");
        Library {
            file: format!("{job}.so"),
            job,
            id: id.into(),
            version: "1.0.0".into(),
            source: source.into(),
            abi: "arm64-v8a".into(),
            library: format!("lib{id}.so"),
            sha256: "e".repeat(64),
            size: 10,
            identity: format!("{commit} {id} 1.0.0 1.92.0"),
        }
    }

    /// A library as an index lists it.
    fn row(l: &Library) -> serde_json::Value {
        serde_json::json!({
            "state": "done", "job": l.job, "id": l.id, "version": l.version, "source": l.source,
            "abi": l.abi, "file": l.file, "library": l.library, "sha256": l.sha256, "size": l.size,
            "identity": l.identity, "asset": 7,
        })
    }

    fn wanted(id: &str, source: &str) -> Wanted {
        Wanted {
            id: id.into(),
            source: source.into(),
        }
    }

    #[test]
    fn looks_at_github_wait_longer_up_to_a_minute() {
        let waits: Vec<u64> = (0..7).map(|n| wait(n).as_secs()).collect();
        assert_eq!(waits, [15, 20, 30, 45, 60, 60, 60]);
    }

    #[test]
    fn a_v1_answer_is_in_its_shard_then_the_first_evenings_release() {
        assert_eq!(
            results_tags("7123456789abcdef01234567"),
            ["results-7".to_owned(), "results".to_owned()]
        );
        assert_eq!(
            download("starling-client/mod-builds", "results-7", "j.json"),
            "https://github.com/starling-client/mod-builds/releases/download/results-7/j.json"
        );
    }

    #[test]
    fn the_services_repository_is_an_owner_and_a_name_and_nothing_else() {
        assert_eq!(
            plain_repo("sayo/mod-builds.fork"),
            Some("sayo/mod-builds.fork")
        );
        for bad in [
            "",
            "a",
            "a/b/c",
            "../b",
            "a/.git",
            "https://github.com/a/b",
            "a/b?x",
        ] {
            assert_eq!(plain_repo(bad), None, "{bad}");
        }
    }

    #[test]
    fn answers_read_as_done_or_failed() {
        let done =
            serde_json::json!({"state": "done", "sha256": "a".repeat(64), "size": 5, "build": {}});
        assert!(matches!(
            Answer::read(&done),
            Some(Answer::Done { size: 5, .. })
        ));
        let failed = serde_json::json!({"state": "failed", "error": "error[E0425]", "settled": true, "at": 9});
        assert_eq!(
            Answer::read(&failed),
            Some(Answer::Failed {
                error: "error[E0425]".into(),
                settled: true,
                at: Some(9)
            })
        );
        let short = serde_json::json!({"state": "done", "sha256": "a", "size": 5});
        assert_eq!(Answer::read(&short), None);
    }

    #[test]
    fn a_v2_job_is_the_hmods_hash_and_the_abi() {
        assert_eq!(
            job_of(&"ab".repeat(32), "arm64-v8a"),
            format!("{}.arm64-v8a", "ab".repeat(16))
        );
    }

    #[test]
    fn builds_read_as_the_store_writes_them_and_others_are_left_out() {
        let apk = "https://github.com/a/b/releases/download/nightly/starling-arm64-v8a.apk";
        let text = serde_json::json!({"schema": 1, "builds": [
            {"build": BUILD, "nightly": "nightly-1", "profile": "quick",
             "abis": {"arm64-v8a": {"target": "aarch64-linux-android", "apk": apk}}},
            {"build": NEWER, "abis": {}},
            {"build": "main", "abis": {}},
        ]})
        .to_string();
        let listed = builds(&text).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].profile.as_deref(), Some("quick"));
        assert_eq!(listed[0].abis["arm64-v8a"].apk.as_deref(), Some(apk));
        assert!(builds(r#"{"schema": 2, "builds": []}"#).is_err());
    }

    #[test]
    fn an_index_keeps_only_libraries_for_its_build() {
        let good = library("veinminer", &"a".repeat(64), BUILD);
        let mut other = library("wrench", &"b".repeat(64), NEWER);
        other.identity = format!("{NEWER} wrench 1.0.0 1.92.0");
        let mut path = library("carry", &"c".repeat(64), BUILD);
        path.library = "../libcarry.so".into();
        let rows: Vec<serde_json::Value> = [&good, &other, &path].into_iter().map(row).collect();
        let text = serde_json::json!({
            "schema": 1,
            "build": BUILD,
            "mods": rows,
            "failed": [{"job": "x", "id": "y", "error": "no", "settled": true}],
        })
        .to_string();
        let read = index(&text, BUILD).unwrap();
        assert_eq!(read.mods.keys().cloned().collect::<Vec<_>>(), [good.job]);
        assert!(read.failed.contains_key("x"));
        assert!(index(&text, NEWER).is_err(), "another build's index");
    }

    #[test]
    fn the_catalogs_leave_out_mods_that_fail_for_good_in_this_build() {
        let built = library("zoom", &"a".repeat(64), BUILD);
        let failed = |job: &str, id: &str, error: &str| serde_json::json!({"job": job, "id": id, "error": error, "settled": true});
        let text = serde_json::json!({
            "schema": 1,
            "build": BUILD,
            "mods": [row(&built)],
            "failed": [
                failed("f", "freecam_plus", "This mod has [native.mixin]: it needs the whole-game Compile."),
                failed("o", "oritech", "This mod uses the crate serde_json: it needs the whole-game Compile."),
                // Another version of a mod the store built still plays.
                failed("z", "zoom", "This mod has [native.mixin]: it needs the whole-game Compile."),
                // Its code doesn't compile on this game, for good: left out too.
                failed("b", "broken", "error[E0425]: cannot find value"),
            ],
        })
        .to_string();
        let hidden = whole_game_only(&index(&text, BUILD).unwrap());
        assert_eq!(
            hidden.into_iter().collect::<Vec<_>>(),
            ["broken", "freecam_plus", "oritech"]
        );
    }

    #[test]
    fn the_plan_is_this_games_build_when_it_has_every_library() {
        let builds = [build(NEWER), build(BUILD)];
        let a = library("veinminer", &"a".repeat(64), BUILD);
        let mut read = Vec::new();
        let plan = plan(
            &builds,
            |b| {
                read.push(b.build.clone());
                (b.build == BUILD).then(|| Index {
                    mods: [(a.job.clone(), a.clone())].into(),
                    failed: BTreeMap::new(),
                })
            },
            &[wanted("veinminer", &"a".repeat(64))],
            "arm64-v8a",
            Some(BUILD),
        );
        assert!(matches!(plan, Plan::Ready { ref build, .. } if build.build == BUILD));
        assert_eq!(read, [BUILD], "one index read");
    }

    #[test]
    fn a_mod_that_fails_here_for_good_moves_the_game_to_a_newer_nightly() {
        let builds = [build(NEWER), build(BUILD)];
        let source = "a".repeat(64);
        let job = job_of(&source, "arm64-v8a");
        let newer = library("veinminer", &source, NEWER);
        let plan = plan(
            &builds,
            |b| {
                Some(if b.build == BUILD {
                    Index {
                        mods: BTreeMap::new(),
                        failed: [(
                            job.clone(),
                            Failure {
                                job: job.clone(),
                                id: "veinminer".into(),
                                error: "error[E0425]".into(),
                                settled: true,
                            },
                        )]
                        .into(),
                    }
                } else {
                    Index {
                        mods: [(job.clone(), newer.clone())].into(),
                        failed: BTreeMap::new(),
                    }
                })
            },
            &[wanted("veinminer", &source)],
            "arm64-v8a",
            Some(BUILD),
        );
        assert!(matches!(plan, Plan::Ready { ref build, .. } if build.build == NEWER));
    }

    #[test]
    fn mods_no_build_has_are_asked_for_on_this_games_build() {
        let builds = [build(NEWER), build(BUILD)];
        let a = library("veinminer", &"a".repeat(64), BUILD);
        let plan = plan(
            &builds,
            |b| {
                (b.build == BUILD).then(|| Index {
                    mods: [(a.job.clone(), a.clone())].into(),
                    failed: BTreeMap::new(),
                })
            },
            &[
                wanted("veinminer", &"a".repeat(64)),
                wanted("mine", &"d".repeat(64)),
            ],
            "arm64-v8a",
            Some(BUILD),
        );
        match plan {
            Plan::Ask {
                build,
                have,
                missing,
            } => {
                assert_eq!(build.build, BUILD);
                assert_eq!(have, [a]);
                assert_eq!(missing, [wanted("mine", &"d".repeat(64))]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            super::plan(&builds, |_| None, &[], "x86", None),
            Plan::NoBuild
        );
    }

    #[test]
    fn the_record_is_what_the_loader_reads() {
        let a = library("veinminer", &"a".repeat(64), BUILD);
        let record: serde_json::Value =
            serde_json::from_str(&record(std::slice::from_ref(&a))).unwrap();
        assert_eq!(
            record,
            serde_json::json!({"libraries": [{"file": "libveinminer.so", "sha256": a.sha256, "mod": "veinminer", "identity": a.identity}]})
        );
        let kept: Library = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(kept, a, "a library kept on the phone reads back as it was");
    }

    #[test]
    fn a_library_from_its_answer_file_takes_its_job_from_its_source() {
        let about = serde_json::json!({"state": "done", "id": "veinminer", "version": "1.0.0", "source": "a".repeat(64),
            "abi": "arm64-v8a", "build": BUILD, "library": "libveinminer.so", "sha256": "e".repeat(64), "size": 10,
            "identity": format!("{BUILD} veinminer 1.0.0 1.92.0")});
        let library = Library::from_answer(&about, BUILD).unwrap();
        assert_eq!(library.job, job_of(&"a".repeat(64), "arm64-v8a"));
        assert_eq!(library.file, format!("{}.so", library.job));
        assert_eq!(Library::from_answer(&about, NEWER), None);
    }
}
