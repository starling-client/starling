//! `build` and `run` for a native mod, with no launcher: the pinned SDK's compiled slices fetched
//! and verified into a per-user cache, the Rust the SDK names, then the same compile the launcher's
//! `native-build` runs (`sunrise_native_build::contract::run`, docs/modding/src/reference/sdk/08-native-build.md),
//! and for `run` the result staged beside the SDK's game executable and started.
use crate::sdk_fetch::{self, Base, CacheStamp};
use std::collections::BTreeMap;
use std::fs;
use sunrise_mod_package::Package;

mod libraries;
mod work_dir;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use sunrise_mod_package::sdk::{self, Index};
use sunrise_native_build::{
    Bundle, contract,
    library_record::Library,
    prebuilt::{self, Route},
    rust::Unpacked,
};
use work_dir::work_dir;

/// Names the cache's folder instead of the platform's.
const CACHE_ENV: &str = "SUNRISE_MOD_CACHE";

/// The per-user cache folder: `SUNRISE_MOD_CACHE`, else the platform's cache folder.
pub(crate) fn cache_dir() -> Result<PathBuf, String> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    cache_in(&env, Platform::current())
}

#[derive(Clone, Copy)]
enum Platform {
    Windows,
    Mac,
    Unix,
}

impl Platform {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Unix
        }
    }
}

fn cache_in(env: &dyn Fn(&str) -> Option<OsString>, platform: Platform) -> Result<PathBuf, String> {
    if let Some(folder) = env(CACHE_ENV) {
        return Ok(folder.into());
    }
    let home = || {
        env(if matches!(platform, Platform::Windows) {
            "USERPROFILE"
        } else {
            "HOME"
        })
        .map(PathBuf::from)
        .ok_or_else(|| format!("no home folder to keep the SDK cache in: set {CACHE_ENV}"))
    };
    let base = match platform {
        Platform::Windows => match env("LOCALAPPDATA") {
            Some(folder) => folder.into(),
            None => home()?.join("AppData/Local"),
        },
        Platform::Mac => home()?.join("Library/Caches"),
        Platform::Unix => match env("XDG_CACHE_HOME") {
            Some(folder) => folder.into(),
            None => home()?.join(".cache"),
        },
    };
    Ok(base.join("sunrise-mod"))
}

/// The target triple of the SDK slices for this computer.
pub(crate) fn computer() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok(sunrise_native_build::rust::WINDOWS_X64_TARGET),
        (os, arch) => Err(format!("the SDK has no slices for {arch} {os}")),
    }
}

/// Where a build's Rust comes from.
#[derive(Debug, PartialEq, Eq)]
enum Source {
    /// `rustup` has, or installs, this toolchain.
    Rustup(String),
    /// The `rustc` on `PATH`, which is exactly the version.
    System,
}

/// The `rustc` on `PATH`: its release, host and sysroot.
struct Found {
    version: String,
    host: String,
    sysroot: PathBuf,
}

fn choose(want: &str, target: &str, rustup: bool, found: Option<&Found>) -> Result<Source, String> {
    if rustup {
        return Ok(Source::Rustup(format!("{want}-{target}")));
    }
    match found {
        Some(rustc) if rustc.version == want && rustc.host == target => Ok(Source::System),
        Some(rustc) => Err(format!(
            "this SDK is built with Rust {want} for {target}, and the rustc on PATH is {} for {}. \
             Install rustup (https://rustup.rs): this tool then installs {want}-{target} itself; or install Rust {want} for {target} so `rustc -V` says it",
            rustc.version, rustc.host
        )),
        None => Err(format!(
            "this SDK is built with Rust {want} for {target}, and no rustc or rustup is on PATH. \
             Install rustup (https://rustup.rs): this tool then installs {want}-{target} itself; or install Rust {want} so `rustc -V` says it"
        )),
    }
}

fn output(command: &mut Command) -> Result<String, String> {
    let name = command.get_program().to_string_lossy().into_owned();
    let out = command.output().map_err(|e| format!("{name}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{name} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn found() -> Option<Found> {
    let text = output(Command::new("rustc").arg("-vV")).ok()?;
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|v| v.trim().to_owned())
    };
    Some(Found {
        version: field("release:")?,
        host: field("host:")?,
        sysroot: output(Command::new("rustc").args(["--print", "sysroot"]))
            .ok()?
            .into(),
    })
}

/// The Rust the SDK names, as a plan's toolchain.
fn toolchain(want: &str, target: &str, cargo_home: PathBuf) -> Result<Unpacked, String> {
    let rustup = output(Command::new("rustup").arg("--version")).is_ok();
    let found = found();
    let dir = match choose(want, target, rustup, found.as_ref())? {
        Source::System => found.expect("chosen from it").sysroot,
        Source::Rustup(name) => {
            let which =
                || output(Command::new("rustup").args(["which", "--toolchain", &name, "rustc"]));
            let rustc = match which() {
                Ok(rustc) => rustc,
                Err(_) => {
                    println!("Installing Rust {name} with rustup (the SDK is built with it)");
                    output(Command::new("rustup").args([
                        "toolchain",
                        "install",
                        &name,
                        "--profile",
                        "minimal",
                    ]))?;
                    which()?
                }
            };
            output(Command::new(rustc).args(["--print", "sysroot"]))?.into()
        }
    };
    Ok(Unpacked {
        dir,
        target: target.to_owned(),
        cargo_home,
        cargo_config: Vec::new(),
        c_bin: None,
    })
}

/// Verified SDK metadata and the compiler it pins, shared by native builds and source discovery.
pub(crate) struct NativeSdk {
    pub index: Index,
    pub rust: Unpacked,
    base: Base,
    dir: PathBuf,
    metadata: PathBuf,
    stamp: CacheStamp,
}

impl NativeSdk {
    pub(super) fn surface(&self) -> PathBuf {
        self.metadata.join(sdk_fetch::SURFACE)
    }

    fn slices(&self, side: sdk::Side, route: Route) -> Result<prebuilt::Package, String> {
        sdk_fetch::fetch_slices(
            &self.base,
            &self.index,
            &self.rust.target,
            side,
            route,
            &self.dir,
            &self.stamp,
        )
    }

    fn compile(
        &self,
        bundles: &[Bundle],
        plan: &contract::Plan,
        events: &mut Events,
    ) -> Result<(), String> {
        let mut needs_tool = false;
        for bundle in bundles {
            if prebuilt::needs_pinned_patch_tool(&plan.package, bundle)? {
                needs_tool = true;
                break;
            }
        }
        if needs_tool {
            let (_temporary, tool) =
                sdk_fetch::fetch_tool(&self.base, &self.index, &self.rust.target, &self.dir)?;
            events.run_tool(&tool, plan)
        } else {
            contract::run(plan, &mut std::io::empty(), events)
        }
    }
}

fn native_sdk_from(
    cache: &Path,
    base: Base,
    keys: &[sunrise_release_sign::PublicKey],
    target: &str,
    build: &str,
    rust: impl FnOnce(&str, &str, PathBuf) -> Result<Unpacked, String>,
) -> Result<NativeSdk, String> {
    let stamp = CacheStamp::new(build, keys);
    let dir = sdk_fetch::cache_root(cache, build, &stamp);
    let (index, metadata) = {
        let _lock = sdk_fetch::cache_lock(&dir, &["metadata", stamp.as_str()])?;
        let metadata = dir.join(sdk_fetch::METADATA_DIR);
        let index = sdk_fetch::fetch_index(&base, keys, build, &metadata, false)?;
        (index, metadata)
    };
    let rust = rust(&index.toolchain.rust, target, cache.join("cargo-home"))?;
    Ok(NativeSdk {
        index,
        rust,
        base,
        dir,
        metadata,
        stamp,
    })
}

pub(crate) fn native_sdk(cache: &Path, build: &str) -> Result<NativeSdk, String> {
    let target = computer()?;
    let base = Base::from_env();
    let keys = sdk_fetch::keys();
    native_sdk_from(cache, base, &keys, target, build, toolchain)
}

/// Refuses a `Cargo.lock` that Cargo would change for the mod's manifests and the SDK of `build` in
/// `.sunrise/sdk/`, with the registry's release scan's own command (`cargo fetch --locked`). The
/// lock names the SDK's local packages and their dependencies, so a pin moved without it is stale;
/// the native build reads only the lock's registry pins and would pass it.
pub(crate) fn check_lock(rust: &Unpacked, project: &Path, build: &str) -> Result<(), String> {
    // The toolchain's commands run from outside the project: name it absolutely.
    let project =
        dunce::canonicalize(project).map_err(|e| format!("{}: {e}", project.display()))?;
    let mut fetch = sunrise_native_build::cargo(rust, &project, "fetch");
    fetch.args(["--locked", "--color", "never"]);
    let out = fetch.output().map_err(|e| format!("cargo fetch: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&out.stderr);
    if !error.contains("--locked was passed") {
        return Err(format!("cargo fetch --locked failed: {}", error.trim()));
    }
    Err(format!(
        "Cargo.lock does not match Cargo.toml and the pinned SDK (Sunrise {build}), and the registry refuses such a release. \
         Regenerate it with `cargo sunrise-mod pin {build}`, which brings the lock to the pinned SDK, then commit Cargo.lock and release again."
    ))
}

/// Brings `Cargo.lock` to the SDK in `.sunrise/sdk/`: Cargo resolves the mod's and the SDK's local
/// packages again and keeps every registry pin they still use.
pub(crate) fn refresh_lock(rust: &Unpacked, project: &Path) -> Result<(), String> {
    let project =
        dunce::canonicalize(project).map_err(|e| format!("{}: {e}", project.display()))?;
    let mut update = sunrise_native_build::cargo(rust, &project, "update");
    update.args(["--workspace", "--color", "never"]);
    output(&mut update).map(drop)
}

/// What a native build made.
pub(crate) struct Built {
    binary: PathBuf,
    crate_name: String,
    libraries: Vec<PathBuf>,
    records: Vec<Library>,
    /// Revision for the package manifest: checked for mixins, inherited when they are unused.
    pub(crate) mixins: String,
    prepared: Prepared,
}

/// Turns the build's event lines into the lines a developer reads, and keeps its result.
#[derive(Default)]
struct Events {
    partial: Vec<u8>,
    built: Option<contract::Event>,
    failed: Option<String>,
}

impl Events {
    /// Run the verified SDK tool using the native-build contract. The plan already answers consent.
    fn run_tool(&mut self, tool: &Path, plan: &contract::Plan) -> Result<(), String> {
        let mut input = serde_json::to_vec(plan).map_err(|e| e.to_string())?;
        input.push(b'\n');
        let describe = |error: std::io::Error| format!("{}: {error}", tool.display());
        let mut child = Command::new(tool)
            .arg("native-build")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(describe)?;
        let sent = child.stdin.take().expect("piped stdin").write_all(&input);
        let received = std::io::copy(&mut child.stdout.take().expect("piped stdout"), self);
        let status = child.wait().map_err(describe)?;
        if !status.success() {
            return Err(self.failed.take().unwrap_or_else(|| {
                format!("{}: native-build exited with {status}", tool.display())
            }));
        }
        sent.map_err(describe)?;
        received.map_err(describe)?;
        Ok(())
    }
}

impl Write for Events {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.partial.extend_from_slice(bytes);
        while let Some(end) = self.partial.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            match serde_json::from_slice::<contract::Event>(&line) {
                Ok(contract::Event::Mod { id, done, total }) => {
                    println!("Compiling {id} ({done}/{total})")
                }
                Ok(event @ contract::Event::Built { .. }) => self.built = Some(event),
                Ok(contract::Event::Failed { error, .. }) => self.failed = Some(error),
                _ => {}
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Prepared {
    plan: contract::Plan,
    sdk: NativeSdk,
    staged: Vec<Bundle>,
    id: String,
    crate_name: String,
    mixins: String,
    origins: BTreeMap<String, PathBuf>,
    // The builder keeps its outputs and generated workspace here. Keep the lock through run's
    // copy of the artifacts, not just through compilation.
    _lock: fs::File,
}

impl Prepared {
    fn failed(&self, error: String) -> String {
        let error = self.origins.iter().fold(error, |error, (id, author)| {
            error.replace(
                &format!(
                    "{}{sep}",
                    self.plan.mods.join(stage_name(id)).display(),
                    sep = std::path::MAIN_SEPARATOR
                ),
                &format!("{}{sep}", author.display(), sep = std::path::MAIN_SEPARATOR),
            )
        });
        build_error(error)
    }
}

fn build_error(error: String) -> String {
    let hint = if error.contains("Cargo.lock") {
        "\nTo refresh this project's own Cargo.lock, run: cargo sunrise-mod build --update-lock\nThat command does not update the locks of its libraries.\n"
    } else {
        "\n"
    };
    format!(
        "{error}{hint}Build log: {}",
        sunrise_native_build::log_path().display()
    )
}

/// Avoid names reserved by package discovery at the input root (such as `native` and `rust`).
fn stage_name(id: &str) -> String {
    format!("mod-{id}")
}

fn prepare(
    project: &Path,
    package: &Package,
    with: &[&Path],
    update_lock: bool,
    target: contract::TargetName,
    release_check: bool,
) -> Result<Prepared, String> {
    let project =
        dunce::canonicalize(project).map_err(|e| format!("{}: {e}", project.display()))?;
    let bundle = sunrise_native_build::Bundle {
        manifest: package.manifest.clone(),
        dir: project.clone(),
    };
    let build = bundle.native().sunrise.clone();
    let id = bundle.manifest.id.clone();
    let cargo = sunrise_native_build::api::Project::read(&bundle).map_err(|e| e.to_string())?;
    let crate_name = cargo.crate_name();
    let registry = matches!(Route::of(&[cargo]), Route::Registry);
    let side = sunrise_native_build::prebuilt::side(target.into());
    let cache = cache_dir()?;
    let dependencies = libraries::Libraries::read(
        &bundle.manifest,
        with,
        match target {
            contract::TargetName::Client => sunrise_mod_package::manifest::Side::Client,
            contract::TargetName::Server => sunrise_mod_package::manifest::Side::Server,
        },
        &cache,
    )?;
    let work = work_dir(&cache, &id, &project);
    fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let work = dunce::canonicalize(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let lock_path = work.join("build.lock");
    let work_lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    work_lock
        .lock()
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    let input = tempfile::tempdir_in(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let sdk = native_sdk(&cache, &build)?;
    let mut origins = dependencies.stage(&bundle, &sdk, &cache, input.path(), release_check)?;
    origins.insert(id.clone(), project.clone());
    let mixins = if bundle.needs_mixins() {
        sunrise_native_build::MixinsIdentity::resolve([&bundle], Some(&sdk.index.mixins))?
            .revision()
    } else {
        bundle
            .native()
            .mixins
            .as_deref()
            .unwrap_or(&sdk.index.mixins.rev)
    }
    .to_owned();
    let rust = &sdk.rust;
    crate::native_build::install_host(&work);
    sunrise_native_build::clear_log()?;

    if registry {
        println!("Using Cargo's crates.io dependencies for {id}");
    }
    let lock = project.join("Cargo.lock");
    if registry && (update_lock || !lock.exists()) {
        println!("Pinning dependencies in Cargo.lock");
        // The lock names the SDK's packages, read from `.sunrise/sdk/` (`run` fetches no SDK before).
        if sdk_fetch::game_tree(&project)?.is_none() {
            sdk_fetch::fetch_pinned(&project, &build)?;
        }
        let before = fs::read(&lock).ok();
        sunrise_native_build::registry_generate::generate(
            || Ok(rust.clone()),
            sunrise_native_build::registry_generate::game_lock,
            &bundle,
            update_lock,
        )
        .map_err(build_error)?;
        // Those pins come from a copy without the SDK's path packages; the committed lock also
        // names them, as the registry's `cargo fetch --locked` requires. A lock without them is
        // never left behind: the one before comes back.
        if let Err(error) = refresh_lock(rust, &project) {
            let restored = match &before {
                Some(text) => fs::write(&lock, text),
                None => fs::remove_file(&lock),
            };
            return Err(match restored {
                Ok(()) => format!("Cargo.lock could not name the SDK's packages: {error}"),
                Err(e) => format!(
                    "Cargo.lock could not name the SDK's packages: {error}\n{} was left without them: {e}",
                    lock.display()
                ),
            });
        }
    }
    crate::registry_source::write_files(
        &input.path().join(stage_name(&id)),
        package.directory_files(&project)?,
    )?;
    let staged = sunrise_native_build::scan_bundles(input.path())?;
    let projects = staged
        .iter()
        .map(sunrise_native_build::api::Project::read)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let slices = sdk.slices(side, Route::of(&projects))?;
    let rust = sdk.rust.clone();
    // A stable source path keeps the native builder's Cargo stage and fingerprints reusable.
    let mods = work.join("input");
    match fs::remove_dir_all(&mods) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {error}", mods.display()));
        }
        _ => (),
    }
    fs::rename(input.path(), &mods).map_err(|error| format!("{}: {error}", mods.display()))?;
    Ok(Prepared {
        plan: contract::Plan {
            contract: contract::CONTRACT,
            work,
            toolchain: rust,
            package: slices,
            mods,
            target,
            registry: Some(true),
            mixins_from_source: false,
        },
        sdk,
        staged,
        id,
        crate_name,
        mixins,
        origins,
        _lock: work_lock,
    })
}

pub(crate) fn check(project: &Path, package: &Package, with: &[&Path]) -> Result<(), String> {
    let prepared = prepare(
        project,
        package,
        with,
        false,
        contract::TargetName::Client,
        false,
    )?;
    contract::check_rust(&prepared.plan).map_err(|error| prepared.failed(error))
}

/// Compiles the project and its libraries against the project's pinned SDK.
pub(crate) fn build(
    project: &Path,
    package: &Package,
    with: &[&Path],
    update_lock: bool,
    target: contract::TargetName,
    release_check: bool,
) -> Result<Built, String> {
    let prepared = prepare(project, package, with, update_lock, target, release_check)?;
    let id = &prepared.id;
    let failed = |error| prepared.failed(error);
    let mut events = Events::default();
    prepared
        .sdk
        .compile(&prepared.staged, &prepared.plan, &mut events)
        .map_err(failed)?;
    let Some(contract::Event::Built {
        binary,
        libraries,
        records,
        ..
    }) = events.built
    else {
        return Err(failed("the build ended without a result".into()));
    };
    let record = records
        .iter()
        .find(|row| row.mod_id == *id && row.kind == "mod")
        .ok_or_else(|| failed(format!("the build made no library for {id}")))?;
    let library = libraries
        .iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name == record.file.as_str())
        })
        .ok_or_else(|| {
            failed(format!(
                "the build did not return its library {}",
                record.file
            ))
        })?;
    println!(
        "Built {}\nBuild log: {}",
        library.display(),
        sunrise_native_build::log_path().display()
    );
    Ok(Built {
        binary,
        crate_name: prepared.crate_name.clone(),
        libraries,
        records,
        mixins: prepared.mixins.clone(),
        prepared,
    })
}

/// `native-mods.json`: what the game's loader verifies and loads, in load order.
fn record(records: &[Library]) -> Result<String, String> {
    serde_json::to_string(&serde_json::json!({ "libraries": records })).map_err(|e| e.to_string())
}

/// Copies `from` to `to` as a real file: the loader refuses a link.
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map(drop)
        .map_err(|e| format!("{}: {e}", from.display()))
}

/// Builds the native mod in `project`, stages it beside the SDK's game under
/// `target/sunrise-mod/run/` (`app/` is made afresh; `game/` and `home/` keep worlds and logs
/// between runs, so nothing of a player's is touched) and starts it with `args`.
pub(crate) fn run(
    project: &Path,
    mod_package: &Package,
    with: &[&Path],
    resources: Option<&str>,
    jars: Option<&Path>,
    args: &[String],
) -> Result<(), String> {
    let resources = resources.map(PathBuf::from).ok_or_else(|| {
        "the game needs prepared resources made from your own Minecraft: pass --resources <folder>. \
         Make them with `sunrise-prepare --prepare --output <folder>` (it comes with the game's download; play \
         Minecraft Java Edition 26.3 once first; sunrise-prepare --help lists the other places it reads)"
            .to_owned()
    })?;
    if !resources.is_dir() {
        return Err(format!(
            "{} is not a folder: make the prepared resources with `sunrise-prepare --prepare --output {0}`",
            resources.display()
        ));
    }
    let stage = project.join(crate::RUN_DIR);
    let launch = crate::jar::launch_arguments(project, args)?;
    let target = if launch.installation_side() == sunrise_mod_package::manifest::Side::Server {
        contract::TargetName::Server
    } else {
        contract::TargetName::Client
    };
    // A missing or wrong original jar is known before the native compile: say so now.
    let libraries = with
        .iter()
        .filter_map(|path| Package::read(path).ok())
        .collect::<Vec<_>>();
    crate::jar::check_for_run(
        std::iter::once(&mod_package.manifest).chain(libraries.iter().map(|p| &p.manifest)),
        jars,
    )?;
    let built = build(project, mod_package, with, false, target, false)?;
    let mut staged = fs::read_dir(&built.prepared.plan.mods)
        .map_err(|e| e.to_string())?
        .map(|entry| {
            let path = entry.map_err(|e| e.to_string())?.path();
            let package = Package::read(&path)?;
            Ok::<_, String>((path, package))
        })
        .collect::<Result<Vec<_>, _>>()?;
    staged.sort_by(|(_, a), (_, b)| a.manifest.id.cmp(&b.manifest.id));
    let home = crate::jar::stage_for_run(
        staged.iter().map(|(_, package)| &package.manifest),
        project,
        jars,
        &launch,
    )?
    .unwrap_or_else(|| stage.join("home"));
    let app = stage.join("app");
    match std::fs::remove_dir_all(&app) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {e}", app.display()));
        }
        _ => {}
    }
    let package = app.join(sunrise_mod_package::mod_list::NATIVE_DIR);
    let id = &mod_package.manifest.id;
    let game = stage.join("game");
    for folder in [&app, &game, &home] {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let executable = app.join(
        built
            .binary
            .file_name()
            .ok_or("the build named no game program")?,
    );
    copy(&built.binary, &executable)?;
    for library in &built.libraries {
        copy(
            library,
            &app.join(library.file_name().ok_or("a library has no name")?),
        )?;
    }
    let record_path = app.join("native-mods.json");
    std::fs::write(&record_path, record(&built.records)?)
        .map_err(|e| format!("{}: {e}", record_path.display()))?;
    let mut content = Vec::new();
    for (path, source) in staged {
        let id = &source.manifest.id;
        if source.is_native() {
            crate::registry_source::write_files(&package.join(id), source.directory_files(&path)?)?;
        } else {
            let installed = app.join(format!("{id}.smod"));
            copy(&path, &installed)?;
            content.push(installed);
        }
    }
    let Built {
        crate_name,
        prepared,
        ..
    } = built;
    drop(prepared);
    println!("Starting {} with {id}", executable.display());
    let mut command = Command::new(&executable);
    for path in content {
        command.arg("--dev-mod").arg(path);
    }
    command
        .arg("--game-dir")
        .arg(&game)
        .arg("--resources")
        .arg(&resources)
        .args(args)
        .env("SUNRISE_HOME", &home)
        .env("SUNRISE_LOG_STDERR", &crate_name)
        // A session file handed to this process is the caller's: the game would read and delete it.
        .env_remove("SUNRISE_SESSION_FILE");
    #[cfg(target_os = "linux")]
    command.env("LD_LIBRARY_PATH", &app);
    let status = command
        .status()
        .map_err(|e| format!("{}: {e}", executable.display()))?;
    status.success().then_some(()).ok_or_else(|| {
        format!(
            "the game exited with {status}; the default log path is {}",
            home.join("logs/latest.log").display()
        )
    })
}

#[cfg(test)]
mod tests;
