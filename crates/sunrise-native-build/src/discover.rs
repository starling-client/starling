//! Internals discovery: which game internals a native mod names, found by checking it against the pinned game's source with the deprecation lint forced (`native-discover`).
use crate::host::trf;
use crate::rust::Toolchain;
use crate::{MixinsIdentity, api, dependencies, internals, mixins_patch};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Stdio,
};
use sunrise_mod_package::build_data;
use sunrise_mod_package::internals::{self as catalog, ListedInternal};
use sunrise_mod_package::sdk::Mixins;

#[derive(Debug)]
pub enum Error {
    SourceUnavailable { revision: String, reason: String },
    Check(String),
}

impl From<String> for Error {
    fn from(reason: String) -> Self {
        Self::Check(reason)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceUnavailable { revision, reason } => f.write_str(&trf(
                "E_INTERNAL_SOURCE_UNAVAILABLE: internals sync needs the source for {revision}. It is not cached and could not be downloaded: {error}",
                &[("revision", revision), ("error", reason)],
            )),
            Self::Check(reason) => f.write_str(reason),
        }
    }
}
impl std::error::Error for Error {}

/// The internals the mod in `folder` names, each as the pinned game's catalog lists it.
/// `sdk` is only needed when the executing tool came from outside that SDK.
pub fn internals(
    rust: &dyn Toolchain,
    folder: &Path,
    sdk: Option<&Mixins>,
) -> Result<Vec<ListedInternal>, Error> {
    let game = catalog_for_mod(rust, folder, sdk)?;
    discover_catalog(rust, folder, &game)
}

/// The pinned game's `internals.json` with only this mod's modules applied, copied into `output`, which must not exist yet.
/// `sdk` is only needed when the executing tool came from outside that SDK.
pub fn catalog(
    rust: &dyn Toolchain,
    folder: &Path,
    sdk: Option<&Mixins>,
    output: &Path,
) -> Result<PathBuf, Error> {
    let game = catalog_for_mod(rust, folder, sdk)?;
    fs::create_dir(output).map_err(|e| format!("{}: {e}", output.display()))?;
    fs::copy(
        game.join(build_data::INTERNALS),
        output.join(build_data::INTERNALS),
    )
    .map_err(|e| e.to_string())?;
    Ok(output.to_path_buf())
}

/// The pinned game's discovery tree with only this mod's modules applied, from pristine pinned source each time.
fn catalog_for_mod(
    rust: &dyn Toolchain,
    folder: &Path,
    sdk: Option<&Mixins>,
) -> Result<PathBuf, Error> {
    catalog_for_mod_with(
        folder,
        &crate::root().join("internals-sync"),
        sdk,
        crate::source,
        |identity, prepared, input, fresh| {
            let tool = mixins_patch::tool(rust, identity)?;
            crate::run(
                crate::apply(rust, &tool, prepared, input, fresh),
                "horizon-mixins",
                |_| {},
            )
        },
    )
}

fn discover_catalog(
    rust: &dyn Toolchain,
    folder: &Path,
    game: &Path,
) -> Result<Vec<ListedInternal>, Error> {
    let bundle = crate::bundle_in(folder)?.ok_or_else(|| {
        trf(
            "{path} is not a native mod project.",
            &[("path", &folder.display().to_string())],
        )
    })?;
    let projects = [api::Project::read(&bundle).map_err(|e| e.to_string())?];
    let declared = [dependencies::build_dependencies(
        &bundle.manifest,
        sunrise_mod_package::manifest::Side::Both,
        |id| bundle.manifest.id == id,
    )];
    let inputs = [dependencies::Mod {
        id: &bundle.manifest.id,
        version: &bundle.manifest.version,
        dependencies: &declared[0],
        has_api: true,
        code: true,
    }];
    let resolution = dependencies::resolve_links(&inputs).map_err(|e| e.to_string())?;
    let cache = game
        .parent()
        .ok_or_else(|| "invalid discovery cache".to_owned())?
        .to_path_buf();
    crate::host::mark_used(&cache)?;
    let stage = cache.join("stage");
    prepare_stage(game, &stage, bundle.needs_mixins())?;
    let mut listed = internals::Catalog::read(&stage)?;
    listed.discovery = true;
    api::stage(&stage, &projects, &declared, &resolution, &listed)?;
    let mod_dir = stage.join("native-api").join(&bundle.manifest.id);
    let mut cmd = crate::cargo(rust, &stage, "check");
    cmd.args(["--message-format=json", "-p", &projects[0].package, "--lib"])
        .env("CARGO_TARGET_DIR", cache.join("target"))
        .env(
            "RUSTC_WRAPPER",
            std::env::current_exe().map_err(|e| e.to_string())?,
        )
        .env("SUNRISE_INTERNALS_CHECK_MOD", &mod_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut used = BTreeSet::new();
    let mut errors = String::new();
    for line in BufReader::new(child.stdout.take().expect("piped")).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(message) = parse(&line, &mod_dir, &stage) {
            match message {
                Message::Used(kind, path) => {
                    used.insert((kind, path));
                }
                Message::Failure(text) => errors.push_str(&text),
            }
        }
    }
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err(trf(
            "Internals discovery failed; the manifest was not changed. {error}",
            &[("error", &errors)],
        )
        .into());
    }
    used.into_iter()
        .filter(|(kind, _)| kind != "mod")
        .map(|(kind, path)| {
            catalog::resolve(&listed.internals, &bundle.manifest.id, &path, Some(&kind))
                .cloned()
                .map_err(|error| Error::Check(error.to_string()))
        })
        .collect()
}

// Keep downloads and the external executable at the boundary so fixtures exercise the actual
// module-input preparation, source staging and catalog publication without installing tools.
fn catalog_for_mod_with(
    folder: &Path,
    cache_root: &Path,
    sdk: Option<&Mixins>,
    source: impl FnOnce(&str) -> Result<PathBuf, String>,
    apply: impl FnOnce(&MixinsIdentity, &Path, &Path, &Path) -> Result<(), String>,
) -> Result<PathBuf, Error> {
    let bundle = crate::bundle_in(folder)?
        .filter(|bundle| bundle.manifest.native.is_some())
        .ok_or_else(|| "not a native mod project".to_owned())?;
    let native = bundle.native();
    let needs_mixins = bundle.needs_mixins();
    let cache = if needs_mixins {
        cache_root
            .join(&native.sunrise)
            .join("modules")
            .join(&bundle.manifest.id)
    } else {
        cache_root.join(&native.sunrise)
    };
    let game = cache.join(if needs_mixins { "module-game" } else { "game" });
    if !needs_mixins && game.join(".discovery-ready").is_file() {
        return Ok(game);
    }
    let identity = MixinsIdentity::resolve([&bundle], sdk)?;
    let source = source(&native.sunrise).map_err(|reason| Error::SourceUnavailable {
        revision: native.sunrise.clone(),
        reason,
    })?;
    let fresh = cache.join("game.new");
    if fresh.exists() {
        fs::remove_dir_all(&fresh).map_err(|e| e.to_string())?;
    }
    if needs_mixins {
        let input = cache.join("mixins.incoming");
        crate::stage_mixins(std::slice::from_ref(&bundle), &input)?;
        let prepared = cache.join("mixins.source");
        crate::prepare_mixins_source(&source, &prepared)?;
        apply(&identity, &prepared, &input, &fresh)
            .map_err(|error| format!("{}: {error}", bundle.manifest.id))?;
    } else {
        crate::sync(&source, &fresh)?;
    }
    sunrise_platform::mod_build::api_gen::staging::stage(
        &fresh,
        &fresh.join(internals::INTERNALS),
        sunrise_platform::mod_build::api_gen::staging::Mode::Discovery,
        &sunrise_platform::mod_build::api_gen::staging::Build {
            commit: native.sunrise.clone(),
        },
    )?;
    fs::write(fresh.join(".discovery-ready"), "").map_err(|e| e.to_string())?;
    if game.exists() {
        fs::remove_dir_all(&game).map_err(|e| e.to_string())?;
    }
    fs::rename(&fresh, &game).map_err(|e| e.to_string())?;
    Ok(game)
}

// The pinned game never changes in this cache. Keep Cargo.lock after the first check:
// sunrise-world watches it, so restoring the pristine lock would rebuild the entire game
// every time. Reset only the workspace membership and the generated mod source copies.
// The first copy is published by a rename, so an interrupted copy is never taken as warm.
// `regenerated`: the mod's own mixins rewrote the game tree, so its changed sources and catalogs
// are taken too.
fn prepare_stage(game: &Path, stage: &Path, regenerated: bool) -> Result<(), String> {
    if !stage.is_dir() {
        let fresh = stage.with_extension("new");
        if fresh.exists() {
            fs::remove_dir_all(&fresh).map_err(|e| e.to_string())?;
        }
        crate::sync(game, &fresh)?;
        fs::rename(&fresh, stage).map_err(|e| e.to_string())?;
    } else if regenerated {
        for (name, path) in crate::files(game)? {
            if name != "Cargo.lock" {
                crate::copy_changed(&path, &stage.join(name))?;
            }
        }
    } else {
        fs::copy(game.join("Cargo.toml"), stage.join("Cargo.toml")).map_err(|e| e.to_string())?;
    }
    for relative in ["native-api", "native-internals"] {
        let path = stage.join(relative);
        if path.exists() {
            fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Entry point used only when Cargo launches this executable as the discovery rustc wrapper.
/// Force the lint for the selected mod, including macro expansions and local allow/deny
/// attributes, without enabling warnings in the game's own crates or other dependencies.
pub fn rustc_wrapper() -> Option<i32> {
    let selected = std::env::var_os("SUNRISE_INTERNALS_CHECK_MOD")?;
    let mut args = std::env::args_os().skip(1);
    let compiler = args.next()?;
    if Some(&compiler) != std::env::var_os("RUSTC").as_ref() {
        return None;
    }
    let mut cmd = std::process::Command::new(compiler);
    cmd.args(args);
    if std::env::var_os("CARGO_MANIFEST_DIR").as_ref() == Some(&selected) {
        cmd.arg("--force-warn=deprecated");
    }
    Some(match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("rustc: {error}");
            1
        }
    })
}

#[derive(Deserialize)]
struct Diagnostic {
    level: String,
    message: String,
    code: Option<Code>,
    spans: Vec<Span>,
    rendered: Option<String>,
}
#[derive(Deserialize)]
struct Code {
    code: String,
}
#[derive(Deserialize)]
struct Span {
    file_name: PathBuf,
    #[serde(default)]
    expansion: Option<Box<Expansion>>,
}
#[derive(Deserialize)]
struct Expansion {
    span: Span,
}
#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    message: Option<Diagnostic>,
    manifest_path: Option<PathBuf>,
}

enum Message {
    Used(String, String),
    Failure(String),
}

// Rustc paths are workspace-relative (or absolute). Macro expansions can point at the
// dependency's macro body: follow the expansion back to the mod's call site, never a string prefix.
fn owned(span: &Span, mod_dir: &Path, stage: &Path) -> bool {
    let path = if span.file_name.is_absolute() {
        span.file_name.clone()
    } else {
        stage.join(&span.file_name)
    };
    path.starts_with(mod_dir)
        || span
            .expansion
            .as_ref()
            .is_some_and(|expansion| owned(&expansion.span, mod_dir, stage))
}

fn parse(line: &str, mod_dir: &Path, stage: &Path) -> Option<Message> {
    let event: CargoMessage = serde_json::from_str(line).ok()?;
    if event.reason != "compiler-message" {
        return None;
    }
    let diagnostic = event.message?;
    if diagnostic.level == "error" {
        return Some(Message::Failure(
            diagnostic.rendered.unwrap_or(diagnostic.message),
        ));
    }
    if event
        .manifest_path
        .as_deref()
        .is_none_or(|path| path.parent() != Some(mod_dir))
    {
        return None;
    }
    if diagnostic.code?.code != "deprecated"
        || !diagnostic
            .spans
            .iter()
            .any(|span| owned(span, mod_dir, stage))
    {
        return None;
    }
    let (_, note) = diagnostic.message.rsplit_once("sunrise-internal:")?;
    let (kind, path) = note.split_once(':')?;
    Some(Message::Used(kind.into(), path.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_regenerated_game_tree_reaches_the_warm_stage_and_the_resolved_lock_stays() {
        let root = crate::tests::dir("discovery-regenerated");
        let game = root.join("game");
        let stage = root.join("stage");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
        fs::write(game.join("Cargo.lock"), "original").unwrap();
        fs::write(game.join("module.rs"), "first").unwrap();
        fs::write(game.join(internals::INTERNALS), "first catalog").unwrap();
        prepare_stage(&game, &stage, true).unwrap();
        fs::write(stage.join("Cargo.lock"), "resolved").unwrap();
        fs::write(game.join("module.rs"), "second").unwrap();
        fs::write(game.join(internals::INTERNALS), "second catalog").unwrap();
        prepare_stage(&game, &stage, true).unwrap();
        assert_eq!(fs::read(stage.join("module.rs")).unwrap(), b"second");
        assert_eq!(
            fs::read(stage.join(internals::INTERNALS)).unwrap(),
            b"second catalog"
        );
        assert_eq!(fs::read(stage.join("Cargo.lock")).unwrap(), b"resolved");
    }

    #[test]
    fn discovery_cache_keeps_the_resolved_lock_and_game_sources_but_replaces_mods() {
        let root = crate::tests::dir("discovery-cache");
        let game = root.join("game");
        let stage = root.join("stage");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
        fs::write(game.join("Cargo.lock"), "original").unwrap();
        fs::write(game.join("game.rs"), "struct Game;").unwrap();
        prepare_stage(&game, &stage, false).unwrap();
        let modified = fs::metadata(stage.join("game.rs"))
            .unwrap()
            .modified()
            .unwrap();
        fs::write(stage.join("Cargo.lock"), "resolved with mod dependencies").unwrap();
        fs::write(stage.join("Cargo.toml"), "with a generated mod").unwrap();
        fs::create_dir_all(stage.join("native-api/old/src")).unwrap();
        fs::write(stage.join("native-api/old/src/lib.rs"), "old mod").unwrap();
        prepare_stage(&game, &stage, false).unwrap();
        assert_eq!(
            fs::read(stage.join("Cargo.lock")).unwrap(),
            b"resolved with mod dependencies"
        );
        assert_eq!(
            fs::metadata(stage.join("game.rs"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert_eq!(
            fs::read(stage.join("Cargo.toml")).unwrap(),
            fs::read(game.join("Cargo.toml")).unwrap()
        );
        assert!(!stage.join("native-api").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_cache_discards_an_interrupted_first_copy() {
        let root = crate::tests::dir("discovery-interrupted");
        let game = root.join("game");
        let stage = root.join("stage");
        fs::create_dir_all(game.join("crates")).unwrap();
        fs::write(game.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
        fs::write(game.join("internals.json"), "[]").unwrap();
        fs::write(game.join("crates/lib.rs"), "struct Game;").unwrap();
        // A copy cut off after its first file.
        let partial = root.join("stage.new");
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
        prepare_stage(&game, &stage, false).unwrap();
        assert_eq!(fs::read(stage.join("internals.json")).unwrap(), b"[]");
        assert!(stage.join("crates/lib.rs").is_file());
        assert!(!partial.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_filters_diagnostics_by_crate_span_and_lint_and_keeps_macro_calls() {
        let stage = Path::new("/cache/stage");
        let own = stage.join("native-api/example");
        let mut event = json!({"reason":"compiler-message", "manifest_path":own.join("Cargo.toml"), "message": {
            "level":"warning", "message":"use of deprecated method: sunrise-internal:method:sunrise_client::game::Thing::method",
            "code":{"code":"deprecated"}, "spans":[{"file_name":"native-api/example/src/lib.rs"}], "rendered":null
        }});
        assert!(
            matches!(parse(&event.to_string(), &own, stage), Some(Message::Used(kind, path)) if kind == "method" && path.ends_with("Thing::method"))
        );
        event["message"]["spans"][0] = json!({"file_name":"crates/dependency/src/lib.rs", "expansion":{"span":{"file_name":"native-api/example/src/lib.rs"}}});
        assert!(matches!(
            parse(&event.to_string(), &own, stage),
            Some(Message::Used(..))
        ));
        event["manifest_path"] = json!(stage.join("crates/dependency/Cargo.toml"));
        assert!(parse(&event.to_string(), &own, stage).is_none());
        event["manifest_path"] = json!(own.join("Cargo.toml"));
        event["message"]["spans"][0] = json!({"file_name":"native-api/example_other/src/lib.rs"});
        assert!(parse(&event.to_string(), &own, stage).is_none());
        event["message"]["spans"][0] = json!({"file_name":own.join("src/lib.rs")});
        event["message"]["code"]["code"] = json!("unused");
        assert!(parse(&event.to_string(), &own, stage).is_none());
        event["message"]["level"] = json!("error");
        assert!(matches!(
            parse(&event.to_string(), &own, stage),
            Some(Message::Failure(..))
        ));
    }
}

#[cfg(test)]
#[path = "discover_tests.rs"]
mod catalog_tests;
