//! `build` and `run` for a native mod, with no launcher: the pinned SDK's compiled slices fetched
//! and verified into a per-user cache, the Rust the SDK names, then the same compile the launcher's
//! `native-build` runs (`sunrise_native_build::contract::run`, docs/modding/src/reference/sdk/08-native-build.md),
//! and for `run` the result staged beside the SDK's game executable and started.
use crate::sdk_fetch::{self, Base, CacheStamp};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use sunrise_mod_package::sdk::{self, Index};
use sunrise_native_build::{contract, library_record::Library, prebuilt::Route, rust::Unpacked};

/// Names the cache's folder instead of the platform's.
const CACHE_ENV: &str = "SUNRISE_MOD_CACHE";

/// The per-user cache folder: `SUNRISE_MOD_CACHE`, else the platform's cache folder.
pub(crate) fn cache_dir() -> Result<PathBuf, String> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    cache_in(&env, Platform::current())
}

/// Persistent per-user data, independent of the SDK cache and its override.
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    let location = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let home = || {
        location(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or("no per-user data folder for signing keys; give a folder with --out or a key file with --key")
    };
    let base = match Platform::current() {
        Platform::Windows => match location("LOCALAPPDATA") {
            Some(path) => path,
            None => home()?.join("AppData/Local"),
        },
        Platform::Mac => home()?.join("Library/Application Support"),
        Platform::Unix => match location("XDG_DATA_HOME") {
            Some(path) => path,
            None => home()?.join(".local/share"),
        },
    };
    Ok(base.join("sunrise-mod"))
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
fn computer() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
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
    stamp: CacheStamp,
}

impl NativeSdk {
    fn slices(
        &self,
        side: sdk::Side,
        release: bool,
    ) -> Result<sunrise_native_build::prebuilt::Package, String> {
        sdk_fetch::fetch_slices(
            &self.base,
            &self.index,
            &self.rust.target,
            side,
            release,
            &self.dir,
            &self.stamp,
        )
    }
}

pub(crate) fn native_sdk(cache: &Path, build: &str) -> Result<NativeSdk, String> {
    let target = computer()?;
    let base = Base::from_env();
    let dir = cache.join("sdk").join(build);
    let keys = sdk_fetch::keys();
    sdk_fetch::fetch(&base, &keys, build, &dir, false)?;
    let index = Index::read(
        &std::fs::read(dir.join(sunrise_mod_package::sdk::INDEX))
            .map_err(|e| format!("{}: {e}", dir.display()))?,
        build,
    )
    .map_err(|e| format!("sdk-{build}: {e}"))?;
    let rust = toolchain(&index.toolchain.rust, target, cache.join("cargo-home"))?;
    Ok(NativeSdk {
        index,
        rust,
        base,
        dir,
        stamp: CacheStamp::new(build, &keys),
    })
}

/// What a native build made.
pub(crate) struct Built {
    binary: PathBuf,
    crate_name: String,
    libraries: Vec<PathBuf>,
    records: Vec<Library>,
    /// Revision for the package manifest: checked for mixins, inherited when they are unused.
    pub(crate) mixins: String,
    pub(crate) release: Option<(crate::release::BuiltLibrary, crate::release::AssetName)>,
}

/// Turns the build's event lines into the lines a developer reads, and keeps its result.
struct Events {
    partial: Vec<u8>,
    built: Option<contract::Event>,
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
                // A failure is the build's returned error; consent is never asked (the plan answers).
                _ => {}
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Compiles the native mod in `project` against its pinned SDK.
pub(crate) fn build(
    project: &Path,
    update_lock: bool,
    release_target: Option<(contract::TargetName, &str, &str)>,
) -> Result<Built, String> {
    let project =
        dunce::canonicalize(project).map_err(|e| format!("{}: {e}", project.display()))?;
    let build = sdk_fetch::pin(&project)?;
    let bundle = sunrise_native_build::bundle_in(&project)?
        .ok_or_else(|| format!("{} is not a native mod", project.display()))?;
    let id = bundle.manifest.id.clone();
    let cargo = sunrise_native_build::api::Project::read(&bundle).map_err(|e| e.to_string())?;
    let release = release_target.is_some();
    if release {
        sunrise_native_build::prebuilt::release_project(&cargo)?;
    }
    let crate_name = cargo.crate_name();
    let registry = matches!(Route::of(&[cargo]), Route::Registry);
    let target = release_target.map_or(contract::TargetName::Client, |(target, _, _)| target);
    let side = sunrise_native_build::prebuilt::side(target.into());
    let cache = cache_dir()?;
    let sdk = native_sdk(&cache, &build)?;
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
    let release_host = release_target
        .map(|(_, tag, version)| {
            let host = sdk::Asset::host_name(&sdk.rust.target, side);
            let name = crate::release::AssetName::archive(&id, version, tag, &host)?;
            Ok::<_, String>((host, name))
        })
        .transpose()?;
    let package = sdk.slices(side, release)?;
    let rust_release = sdk.index.toolchain.rust;
    let rust = sdk.rust;

    let work = match &release_host {
        Some((host, _)) => cache.join("release-work").join(&build).join(&id).join(host),
        None => cache.join("work").join(&id),
    };
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    crate::native_build::install_host(&work);
    sunrise_native_build::clear_log()?;
    let log = sunrise_native_build::log_path();
    let failed = |error: String| {
        let hint = if error.contains("Cargo.lock") {
            "\nResolve the dependencies again with: cargo sunrise-mod build --update-lock\n"
        } else {
            "\n"
        };
        format!("{error}{hint}Build log: {}", log.display())
    };

    if registry {
        println!("Compiling the crates.io dependencies of {id}, as `cargo build` would");
    }
    let lock = project.join("Cargo.lock");
    if (registry || release) && (update_lock || !lock.exists()) {
        println!("Pinning dependencies in Cargo.lock");
        sunrise_native_build::registry_generate::generate(
            || Ok(rust.clone()),
            sunrise_native_build::registry_generate::game_lock,
            &bundle,
            update_lock,
        )
        .map_err(failed)?;
    }
    if release {
        sunrise_native_build::prebuilt::check_release_lock(
            &crate::bounded::read_bytes(&lock, 16 << 20, crate::bounded::Symlinks::Refuse)
                .map_err(|e| failed(e.to_string()))?,
        )
        .map_err(failed)?;
    }

    let plan = contract::Plan {
        contract: contract::CONTRACT,
        work,
        toolchain: rust,
        package,
        mods: project,
        target,
        registry: Some(!release),
        mixins_from_source: false,
    };
    let mut events = Events {
        partial: Vec::new(),
        built: None,
    };
    contract::run(&plan, &mut std::io::empty(), &mut events).map_err(failed)?;
    let Some(contract::Event::Built {
        route,
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
        .find(|row| row.mod_id == id && row.kind == "mod")
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
    let release = release_host
        .map(|(host, name)| {
            let identity = sunrise_mod_package::index::registry::builds::library_identity(
                &build,
                &id,
                &bundle.manifest.version.to_string(),
                &rust_release,
            );
            crate::release::BuiltLibrary::from_event(&route, host, library, record, &identity)
                .map(|library| (library, name))
        })
        .transpose()?;
    println!("Built {}\nBuild log: {}", library.display(), log.display());
    Ok(Built {
        binary,
        crate_name,
        libraries,
        records,
        mixins,
        release,
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
    let mod_package = sunrise_mod_package::Package::read(project)?;
    let home = crate::jar::stage_for_run(&mod_package.manifest, project, jars, args)?
        .unwrap_or_else(|| stage.join("home"));
    let built = build(project, false, None)?;
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
    let files = mod_package.directory_files(project)?;
    for (name, bytes) in files {
        let to = package.join(id).join(&name);
        std::fs::create_dir_all(to.parent().expect("a file in the package"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    println!("Starting {} with {id}", executable.display());
    let mut command = Command::new(&executable);
    command
        .arg("--game-dir")
        .arg(&game)
        .arg("--resources")
        .arg(&resources)
        .args(args)
        .env("SUNRISE_HOME", &home)
        .env("SUNRISE_LOG_STDERR", &built.crate_name)
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
