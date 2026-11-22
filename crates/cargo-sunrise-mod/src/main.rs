//! `cargo sunrise-mod`: make Sunrise content mods (`docs/modpacks.md`): blocks,
//! items, data and assets, no code. Code mods are native (docs/modding/src/how-to/native-mods.md).
//!
//! - `new <path> [--name "Name"]`: a content mod from the template; with
//!   `--template native [--sunrise <build>]`, a code mod pinned to the current Nightly's
//!   SDK unless `--sunrise` names another build (a release tag such as `nightly-<date>` or `26.3-0.1.0`, or a sha), with that SDK fetched
//!   into `.sunrise/sdk/` (`sdk_fetch.rs`).
//! - `update [--sunrise <build>]`: repin a native mod and diff/refresh declared internals.
//! - `internals sync [--repo <path>]`: sync declarations for native mods.
//! - `internals add <path>… [--kind <kind>]`: declare internals from the prebuilt catalog.
//! - `build [--update-lock]`: the `.smod` in `target/sunrise-mod/`, with the mod's `assets/`
//!   and `data/`; a native mod is compiled first (`dev.rs`).
//! - `publish`: build, then write the registry entry, the staged file and the pull request text (`publish.rs`).
//! - `keygen`, `sign`: make an author-held Minisign key pair or sign a release asset (`signing.rs`).
//! - `run [--game <sunrise-client>] [--resources <folder>] [--jars <dir>] [-- <game args>]`: build, then play with
//!   the mod.
//! - `check [--uses]` in a mod folder: the package rules and render pipeline shaders; for a native
//!   mod, `cargo check` and what the mod uses of its pinned build
//!   (`sdk_cmds.rs`).
//! - `check <file> [--kind <kind>] [--key <public key> --signature <file.minisig>] [--json]`: what the registry checks of a submission
//!   (`.smod`, `.spack`, resource pack `.zip`, `.sshader`).
//! - `hooks <text> [--all]`: search the pinned build's hooks (or every item) offline.
//! - `sdk diff <build> <build>`: what changed in the surface between two builds.
//! - `fits [<file.smod>] [--build <name>]…`: whether the mod fits each build; exit 0 when all
//!   do, 1 when one does not, 2 when it cannot be told.
//! - `registry build|review|check|list|release|page|releases`: the registry repository's CI (`registry.rs`).
//! - `use-jar <path>`: list an exact original Java jar as a passive asset/data source.
//! - `native-build < plan.json`: compile native mods against an unpacked SDK, as the launcher does
//!   (docs/modding/src/reference/sdk/08-native-build.md).
//! - `native-check < plan.json`: how native mods stand against one build before anything compiles (the same page).
//! - `native-discover < plan.json`, `native-lock < plan.json`: a native mod's internals or its pinned game's build data, and its registry lock (the same page).
//! - `native-layout < plan.json`: a local prebuilt game checked and laid out as an SDK's slices (the same page).
use sunrise_mod_package::{Content, Manifest, Package, manifest::MANIFEST};
mod bounded;
mod check;
mod dev;
mod help;
mod internals;
mod jar;
mod native_build;
mod options;
mod publish;
mod registry;
mod release;
mod sdk_cmds;
mod sdk_fetch;
mod signing;
mod uses_table;

use options::Options;
use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const RUN_DIR: &str = "target/sunrise-mod/run";

/// What `--version` prints: the commit this tool was built from, which tells a user whether it is as new as the
/// rolling release (`tool.json`'s `commit` in the `mod-tool` release), and the `native-*` contract number it speaks.
fn version_line() -> String {
    format!(
        "cargo-sunrise-mod commit {} native-contract {}",
        env!("SUNRISE_TOOL_BUILT_FROM"),
        sunrise_native_build::contract::CONTRACT
    )
}

fn main() -> ExitCode {
    // A native build's registry route, and internals discovery, run this program as Cargo's rustc wrapper.
    if let Some(code) = sunrise_native_build::registry_rustc::wrapper()
        .or_else(sunrise_native_build::discover::rustc_wrapper)
    {
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    // The uses scan parses a mod's source in a child, this program started again: run its part
    // and exit.
    let entry = mod_surface::child::child_entry();
    // Cargo runs `cargo-sunrise-mod sunrise-mod <args>`.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "sunrise-mod") {
        args.remove(0);
    }
    if args.first().is_some_and(|a| a == "--version" || a == "-V") {
        println!("{}", version_line());
        return ExitCode::SUCCESS;
    }
    if let Some((command, rest)) = args.split_first()
        && help::asked(rest)
        && let Some(text) = help::of(command)
    {
        print!("{text}");
        return ExitCode::SUCCESS;
    }
    let result = match args.first().map(String::as_str) {
        Some("new") => new(&args[1..], &sdk_fetch::fetch_pinned).and_then(|code| match code {
            Some(dir) => fetched(&dir),
            None => Ok(()),
        }),
        Some("build") => build_command(entry, &args[1..]).map(drop),
        Some("publish") => publish::run(entry, &args[1..]),
        Some("release") => release::run(Path::new("."), &args[1..]),
        Some("keygen") => signing::keygen(&args[1..]),
        Some("sign") => signing::sign(&args[1..]),
        Some("run") => run(&args[1..]).map_err(|e| e.to_string()),
        Some("check") => check_args(&args[1..]).and_then(|wanted| match wanted {
            Check::Folder { uses } => sdk_cmds::check_mod(entry, uses),
            Check::File(file) => check(file),
        }),
        Some("hooks") => sdk_cmds::hooks(&args[1..]),
        Some("sdk") => sdk_cmds::sdk(&args[1..]),
        Some("fits") => return sdk_cmds::fits(entry, &args[1..]),
        Some("registry") => registry::main(&args[1..]),
        Some("use-jar") => jar::run(&args[1..]),
        Some("native-build") => return native_build::run(&args[1..]),
        Some("native-check") => return native_build::check(&args[1..]),
        Some("native-discover") => return native_build::discover(&args[1..]),
        Some("native-lock") => return native_build::lock(&args[1..]),
        Some("native-layout") => return native_build::layout(&args[1..]),
        Some("internals") => internals::run(&args[1..]),
        Some("update") => update_args(&args[1..]).and_then(|sunrise| {
            sdk_cmds::update_report(entry, Path::new("."), sunrise)
                .and_then(|()| update(Path::new("."), &sdk_fetch::fetch_build, sunrise))
                .map(|()| println!("Run `cargo sunrise-mod check`."))
        }),
        None | Some("--help" | "-h") => {
            print!("{}", help::short());
            Ok(())
        }
        Some("help") => help::run(&args[1..]).map(|text| print!("{text}")),
        Some(other) => Err(help::unknown(other)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BuildMode<'a> {
    Package,
    Archive {
        dir: &'a Path,
        tag: &'a str,
        releases: &'a Path,
        target: sunrise_native_build::contract::TargetName,
    },
    Describe {
        dir: &'a Path,
        tag: &'a str,
        releases: &'a Path,
        sdk: &'a Path,
    },
}

fn build_args(args: &[String]) -> Result<(bool, BuildMode<'_>), String> {
    let mut options = Options::new(
        "build",
        args,
        &[
            "--binaries",
            "--describe-binaries",
            "--sunrise-release",
            "--releases",
            "--sdk-data",
        ],
    );
    let update_lock = options.switch("--update-lock");
    let server = options.switch("--server");
    let archive = options.value("--binaries")?;
    let describe = options.value("--describe-binaries")?;
    let tag = options.value("--sunrise-release")?;
    let releases = options.value("--releases")?;
    let sdk = options.value("--sdk-data")?;
    options.finish()?;
    let mode = match (archive, describe, tag, releases, sdk) {
        (None, None, None, None, None) if !server => BuildMode::Package,
        (Some(dir), None, Some(tag), Some(releases), None) => BuildMode::Archive {
            dir: Path::new(dir), tag, releases: Path::new(releases),
            target: if server { sunrise_native_build::contract::TargetName::Server } else { sunrise_native_build::contract::TargetName::Client },
        },
        (None, Some(dir), Some(tag), Some(releases), Some(sdk)) if !server && !update_lock => BuildMode::Describe {
            dir: Path::new(dir), tag, releases: Path::new(releases), sdk: Path::new(sdk),
        },
        _ => return Err("use --binaries <dir> with --sunrise-release <tag> and --releases <file>, or --describe-binaries <dir> with those inputs and --sdk-data <dir>".into()),
    };
    Ok((update_lock, mode))
}

/// Compile once through native-build, then write the source package and, when requested, the
/// exact SDK host's archive. Describe mode only reads the resulting signed assets.
fn build_command(
    entry: mod_surface::child::ChildEntry,
    args: &[String],
) -> Result<PathBuf, String> {
    let (update_lock, mode) = build_args(args)?;
    let root = Path::new(".");
    if let BuildMode::Describe {
        dir,
        tag,
        releases,
        sdk,
    } = mode
    {
        let releases = release::releases(releases)?;
        let release = release::stable(&releases, tag)?;
        let dir = release::output_dir(dir)?;
        let path = release::describe(&read_manifest(root)?, &dir, release, sdk)?;
        println!(
            "Wrote {}: sign this descriptor with the project key",
            path.display()
        );
        return Ok(path);
    }
    let package = Package::read(root)?;
    if update_lock && !package.is_native() {
        return Err(format!(
            "build --update-lock rewrites a native mod's Cargo.lock; {MANIFEST} has no [native] table"
        ));
    }
    let archive = if let BuildMode::Archive {
        dir,
        tag,
        releases,
        target,
    } = mode
    {
        let native = package
            .manifest
            .native
            .as_ref()
            .ok_or("build --binaries needs a native mod")?;
        let version = release::version(&package.manifest)?;
        let releases = release::releases(releases)?;
        let release = release::stable(&releases, tag)?;
        if native.sunrise != release.commit {
            return Err(format!(
                "{MANIFEST} pins {}, but Sunrise {tag} is {}; build this release in a checkout pinned to that commit",
                native.sunrise, release.commit
            ));
        }
        release::AssetName::descriptor(&package.manifest.id, &version, tag)?;
        Some((release::output_dir(dir)?, tag, target, version))
    } else {
        None
    };
    let output = PackageOutput::in_project(root)?;
    if package.is_native() {
        fetched(root)?;
    }
    let built = package
        .is_native()
        .then(|| {
            dev::build(
                root,
                update_lock,
                archive
                    .as_ref()
                    .map(|(_, tag, target, version)| (*target, *tag, version.as_str())),
            )
        })
        .transpose()?;
    let path = build(
        entry,
        &package,
        &output,
        built.as_ref().map(|built| built.mixins.as_str()),
    )?;
    println!("Built {}", path.display());
    if let Some((dir, _, _, _)) = archive {
        let (library, name) = built
            .and_then(|built| built.release)
            .ok_or("the build returned no releasable library")?;
        let archive = library.write(&dir, name)?;
        println!(
            "Wrote {}: sign this archive with the project key",
            archive.display()
        );
    }
    Ok(path)
}

/// The manifest in `dir`, read within its size bound.
fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    Manifest::parse(&bounded::read(&dir.join(MANIFEST), bounded::MAX_MANIFEST)?)
}

/// The SDK the project at `dir` pins, fetched into its `.sunrise/sdk/` when missing or stale; a
/// project inside a game checkout builds against that tree instead.
fn fetched(dir: &Path) -> Result<(), String> {
    if let Some(tree) = sdk_fetch::game_tree(dir)? {
        println!("Building against the game checkout {}", tree.display());
        return Ok(());
    }
    let sdk = sdk_fetch::ensure_sdk(dir)?;
    println!(
        "SDK {} (build {}) in {}",
        sdk.name,
        sdk.build,
        sdk.dir.display()
    );
    Ok(())
}

/// Fetches a build's verified SDK for a project: the SDK the build's `sdk.json` describes.
type FetchSdk<'a> = &'a dyn Fn(&Path, &str) -> Result<sdk_fetch::PinnedSdk, String>;

/// Makes the mod `args` asks for; a code mod's folder is returned, with its SDK fetched into it
/// by `fetch`.
fn new(args: &[String], fetch: FetchSdk) -> Result<Option<PathBuf>, String> {
    let usage = || help::of("new").expect("new has help");
    let unexpected = |arg: &str| {
        let reason = match arg {
            "--native" | "--content" => {
                format!("unknown option {arg}; use --template {}", &arg[2..])
            }
            _ => format!("unexpected argument {arg}"),
        };
        format!("{reason}\n{}", usage().trim_end())
    };
    let (path, options) = args.split_first().ok_or_else(usage)?;
    if path.starts_with('-') {
        return Err(unexpected(path));
    }
    let (mut name, mut template, mut sunrise) = (None, None, None);
    let mut options = options.iter().map(String::as_str);
    while let Some(option) = options.next() {
        let slot = match option {
            "--name" => &mut name,
            "--template" => &mut template,
            "--sunrise" => &mut sunrise,
            _ => return Err(unexpected(option)),
        };
        let value = options
            .next()
            .ok_or_else(|| format!("missing value after {option}\n{}", usage().trim_end()))?;
        slot.get_or_insert(value);
    }
    let dir = Path::new(path);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let base = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the path needs a folder name")?;
    let id: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let name = name.map_or_else(|| base.replace(['-', '_'], " "), str::to_string);
    let fill = |t: &str| t.replace("{id}", &id).replace("{name}", &name);
    match template {
        None | Some("content") => {}
        Some("native") => {
            new_native(dir, &id, sunrise, fetch, fill)?;
            return Ok(Some(dir.to_owned()));
        }
        Some(_) => return Err("--template is content or native".into()),
    }
    // A block copying a vanilla one, with its model, name, loot, recipe and tag.
    let files = [
        (
            MANIFEST,
            include_str!("../template/content/sunrise-mod.toml"),
        ),
        (
            "data/{id}/sunrise/block/glowing_amethyst.json",
            include_str!("../template/content/block.json"),
        ),
        (
            "assets/{id}/blockstates/glowing_amethyst.json",
            include_str!("../template/content/blockstate.json"),
        ),
        (
            "assets/{id}/models/block/glowing_amethyst.json",
            include_str!("../template/content/model.json"),
        ),
        (
            "assets/{id}/items/glowing_amethyst.json",
            include_str!("../template/content/item.json"),
        ),
        (
            "assets/{id}/lang/en_us.json",
            include_str!("../template/content/lang.json"),
        ),
        (
            "data/{id}/loot_table/blocks/glowing_amethyst.json",
            include_str!("../template/content/loot.json"),
        ),
        (
            "data/{id}/recipe/glowing_amethyst.json",
            include_str!("../template/content/recipe.json"),
        ),
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            include_str!("../template/content/pickaxe.json"),
        ),
        (".gitignore", include_str!("../template/gitignore")),
    ];
    for (file, text) in files {
        let file = fill(file);
        let file = file.as_str();
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().expect("a file in the mod"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to, fill(text)).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let manifest =
        Manifest::parse(&std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| e.to_string())?)?;
    Content::read(dir, &manifest)?;
    println!(
        "Created {} (mod id {id}). Next: cd {path} && cargo sunrise-mod build, then copy \
         target/sunrise-mod/{id}.smod into your game folder's sunrise-mods/ \
         (docs/modding/src/start/first-content-mod.md)",
        dir.display()
    );
    Ok(None)
}

/// The build a code mod pins: `--sunrise`'s (a sha or a release tag), or by default the current
/// Nightly's, the build players have.
fn pin(sunrise: &str) -> Result<String, String> {
    sdk_fetch::resolve(
        &sdk_fetch::Base::from_env(),
        &sdk_fetch::release_keys(),
        sunrise,
    )
}

/// The optional build in `update [--sunrise <build>]`.
fn update_args(args: &[String]) -> Result<Option<&str>, String> {
    let mut options = Options::new("update", args, &["--sunrise"]);
    let parsed = options.value("--sunrise")?;
    options.finish()?;
    Ok(parsed)
}

/// Repins a native mod. Declared internals are checked against the new build before writing.
/// `Cargo.toml` names the SDK by path, so the next command fetches the new one.
fn update(dir: &Path, fetch: FetchSdk, sunrise: Option<&str>) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let text = bounded::read(&path, bounded::MAX_MANIFEST)?;
    let manifest = Manifest::parse(&text)?;
    let Some(native) = manifest.native.as_ref() else {
        return Err(format!(
            "{MANIFEST}: a content mod pins no game build, so there is nothing to update"
        ));
    };
    let requested = sunrise.unwrap_or(sunrise_release_sign::NIGHTLY);
    let sunrise = pin(requested)?;
    // The new build's SDK must exist and verify before anything is written; the mixins revision
    // is the SDK's own unless the manifest pins one.
    fetch(dir, &sunrise).map_err(|e| sdk_fetch::named(requested, &sunrise, e))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !manifest.internals.is_empty() {
        // #2593: these become the pinned catalogs with the mod's own modules applied.
        let old = internals::load(dir, &native.sunrise)?;
        let new = internals::load(dir, &sunrise)?;
        print!("{}", internals::refresh(&manifest, &mut doc, &old, &new)?);
    }
    internals::set_string(
        doc["native"]["sunrise"].as_value_mut().expect("native pin"),
        &sunrise,
    );
    // Every check passes before anything is written.
    let text = doc.to_string();
    Manifest::parse(&text)?;
    internals::atomic_write_manifest(&path, text)?;
    // The old pin's SDK goes; the next command fetches this one.
    if let Err(e) = std::fs::remove_dir_all(dir.join(sdk_fetch::DIR))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(format!("{}: {e}", sdk_fetch::DIR));
    }
    println!("Pinned to Sunrise {sunrise}");
    Ok(())
}

/// A flat native mod project pins its game build in `[native] sunrise` and names the SDK in
/// `Cargo.toml` by path (`.sunrise/sdk/api/crates/sunrise-mod-api`).
fn new_native(
    dir: &Path,
    id: &str,
    sunrise: Option<&str>,
    fetch: FetchSdk,
    fill: impl Fn(&str) -> String,
) -> Result<(), String> {
    if id.len() > 64 {
        return Err(format!("mod id {id}: 64 characters at most"));
    }
    let requested = sunrise.unwrap_or(sunrise_release_sign::NIGHTLY);
    let sunrise = pin(requested)?;
    let sunrise = sunrise.as_str();
    // The folder is made exclusively, so anything already at the path (a dangling link included)
    // is refused untouched, and a failed fetch removes only what this run made.
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::create_dir(dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{} already exists", dir.display()),
        _ => format!("{}: {e}", dir.display()),
    })?;
    // The SDK is fetched now, so a failed pin leaves no folder; the mixins revision is its own.
    fetch(dir, sunrise)
        .inspect_err(|_| {
            let _ = std::fs::remove_dir_all(dir);
        })
        .map_err(|e| sdk_fetch::named(requested, sunrise, e))?;
    let manifest = format!(
        "{}\n[native]\ngame = \"26.3\"\nsunrise = \"{sunrise}\"\n",
        include_str!("../template/native/sunrise-mod.toml"),
    );
    let files = [
        ("Cargo.toml", include_str!("../template/native/api.toml")),
        ("src/lib.rs", include_str!("../template/native/lib.rs")),
        (MANIFEST, manifest.as_str()),
        (".gitignore", include_str!("../template/gitignore")),
        ("README.md", include_str!("../template/native/README.md")),
        (
            ".github/workflows/release.yml",
            include_str!("../template/native/release.yml"),
        ),
        (
            ".github/workflows/rebuild.yml",
            include_str!("../template/native/rebuild.yml"),
        ),
        (
            ".github/workflows/sunrise-release-jobs.yml",
            include_str!("../template/native/sunrise-release-jobs.yml"),
        ),
        (
            ".github/actions/sunrise-release-tool/action.yml",
            include_str!("../template/native/release-tool.yml"),
        ),
    ];
    for (file, text) in files {
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().expect("a file in the mod"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to, fill(text)).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    for folder in ["assets", "data"] {
        std::fs::create_dir_all(dir.join(folder)).map_err(|e| e.to_string())?;
    }
    println!(
        "Created {} (native mod id {id}). Your editor resolves sunrise-mod-api once the SDK is in .sunrise/sdk/: \
         `cargo sunrise-mod check` fetches it in a fresh clone. Next: cd {} && cargo sunrise-mod build",
        dir.display(),
        dir.display(),
    );
    Ok(())
}

/// Packs the mod in the current folder into `target/sunrise-mod/<id>.smod`: the manifest and
/// the files in `assets/` and `data/`, and for a code mod its source and its uses table, scanned
/// against the pinned build (none for a mod with rows no list can judge, or built against the game
/// checkout it lies in, which has no published surface to scan against).
fn build(
    entry: mod_surface::child::ChildEntry,
    package: &Package,
    output: &PackageOutput,
    mixins: Option<&str>,
) -> Result<PathBuf, String> {
    let root = Path::new(".");
    let uses = if let Some(native) = &package.manifest.native
        && sdk_fetch::game_tree(root)?.is_none()
    {
        let sdk = sdk_fetch::fetch_pinned(root, &native.sunrise)?;
        let surface = uses_table::surface_of(&sdk)?;
        let scanned = uses_table::scan(entry, root, &package.manifest, &surface)?;
        match scanned {
            uses_table::Scanned::Uses(uses) => Some(
                uses_table::table(&uses, root, &package.manifest, &sdk)?
                    .to_json()
                    .into_bytes(),
            ),
            uses_table::Scanned::Exact(_) => None,
            uses_table::Scanned::Unread(why) => {
                println!(
                    "note: no uses table packed ({why}); the package counts as made for build {} only.",
                    sdk.name
                );
                None
            }
        }
    } else {
        None
    };
    pack_into(root, output, &package.manifest, uses.as_deref(), mixins)
}

fn build_in(root: &Path, manifest: &Manifest) -> Result<PathBuf, String> {
    pack_into(
        root,
        &PackageOutput::in_project(root)?,
        manifest,
        None,
        None,
    )
}

struct PackageOutput(PathBuf);

impl PackageOutput {
    /// Check both output components under the project before writing or unlinking packages.
    fn in_project(root: &Path) -> Result<Self, String> {
        let target = root.join("target");
        internals::check_output_dir(&target)?;
        let dir = target.join("sunrise-mod");
        internals::check_output_dir(&dir)?;
        Ok(Self(dir))
    }
}

/// The manifest text a package carries: the project's with derived `api` and `capabilities`
/// defaults, and, for a native build, the mixins revision selected by that build.
fn completed_manifest(text: &str, mixins: Option<&str>) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
    let root = doc.as_table_mut();
    if !root.contains_key("api") {
        root.insert("api", toml_edit::value(sunrise_mod_package::manifest::API));
    }
    if !root.contains_key("capabilities") {
        let mut names = toml_edit::Array::new();
        for capability in sunrise_mod_package::Capability::derived() {
            names.push(capability.to_string());
        }
        root.insert("capabilities", toml_edit::value(names));
    }
    if let (Some(mixins), Some(native)) = (
        mixins,
        doc.get_mut("native").and_then(|n| n.as_table_like_mut()),
    ) {
        native.insert("mixins", toml_edit::value(mixins));
    }
    Ok(doc.to_string())
}

fn pack_into(
    root: &Path,
    output: &PackageOutput,
    manifest: &Manifest,
    uses: Option<&[u8]>,
    mixins: Option<&str>,
) -> Result<PathBuf, String> {
    let dir = &output.0;
    // A failed build leaves no earlier package that a later `check <file>` would report as good.
    let path = dir.join(format!("{}.smod", manifest.id));
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("{}: {e}", path.display()));
        }
        _ => {}
    }
    check::references(root, manifest)?;
    let text = bounded::read(&root.join(MANIFEST), bounded::MAX_MANIFEST)?;
    let bytes = Package::pack_directory_with(root, uses, Some(completed_manifest(&text, mixins)?))?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Written beside it and renamed, so a running game never reads half a file, through a
    // temporary name made new: a link planted at a guessable name is never followed.
    internals::atomic_write(&path, bytes)?;
    Ok(path)
}

/// What `check` was asked to check.
#[derive(Debug, PartialEq)]
enum Check<'a> {
    /// The mod in this folder.
    Folder {
        uses: bool,
    },
    File(CheckFile<'a>),
}

/// `check <file>`'s file and options.
#[derive(Debug, PartialEq)]
struct CheckFile<'a> {
    file: &'a str,
    kind: Option<&'a str>,
    key: Option<&'a str>,
    signature: Option<&'a str>,
    json: bool,
}

/// `check`'s two forms, told apart by whether a file is named, wherever it stands; an option of
/// the other form is refused by name.
fn check_args(args: &[String]) -> Result<Check<'_>, String> {
    let mut options = Options::new("check", args, &["--kind", "--key", "--signature"]);
    let uses = options.switch("--uses");
    let (kind, key, signature, json) = (
        options.value("--kind")?,
        options.value("--key")?,
        options.value("--signature")?,
        options.switch("--json"),
    );
    let file = options.at_most_one()?;
    options.finish()?;
    let of_file = [
        ("--kind", kind.is_some()),
        ("--key", key.is_some()),
        ("--signature", signature.is_some()),
        ("--json", json),
    ];
    let given = |options: &[(&'static str, bool)]| {
        options
            .iter()
            .find_map(|(name, given)| given.then_some(*name))
    };
    match file {
        Some(file) if uses => Err(format!(
            "check --uses checks the mod in this folder; `check {file}` does not take it"
        )),
        Some(file) => Ok(Check::File(CheckFile {
            file,
            kind,
            key,
            signature,
            json,
        })),
        None => match given(&of_file) {
            Some(name) => Err(format!("check {name} needs a file: check <file> {name}")),
            None => Ok(Check::Folder { uses }),
        },
    }
}

/// `check <file>`: the registry's checks of a submission, printed; fails
/// when one does.
fn check(wanted: CheckFile<'_>) -> Result<(), String> {
    let CheckFile {
        file,
        kind,
        key,
        signature,
        json,
    } = wanted;
    let path = Path::new(file);
    let kind = match kind {
        Some(k) => sunrise_mod_package::index::Kind::from(k.to_string()),
        None => check::kind_of(path)
            .ok_or("pass --kind mod|modpack|resourcepack: the extension doesn't say")?,
    };
    let signature = signature
        .map(|p| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")))
        .transpose()?;
    let report = check::check(path, kind, key, signature.as_deref());
    if json {
        let json = match &report {
            Ok(r) => serde_json::json!({"ok": true, "report": r.json()}),
            Err(e) => serde_json::json!({"ok": false, "error": e}),
        };
        println!("{json}");
    }
    let report = report?;
    if !json {
        print!("{}", report.text());
    }
    Ok(())
}

#[derive(Debug)]
enum RunError {
    Build(String),
    Launch { game: String, error: std::io::Error },
    Exit(std::process::ExitStatus),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Build(error) => f.write_str(error),
            Self::Launch { game, error } => write!(
                f,
                "{game}: {error} (pass --game <path to sunrise-client> or set SUNRISE_CLIENT)"
            ),
            Self::Exit(status) => write!(f, "the game exited with {status}"),
        }
    }
}

impl std::error::Error for RunError {}

/// What `run` was asked for; the game's own arguments follow a bare `--`.
#[derive(Debug, PartialEq)]
struct RunArgs<'a> {
    game: Option<&'a str>,
    resources: Option<&'a str>,
    jars: Option<&'a str>,
    extra: &'a [String],
}

fn run_args(args: &[String]) -> Result<RunArgs<'_>, String> {
    let mut options = Options::new("run", args, &["--game", "--resources", "--jars"]);
    let parsed = RunArgs {
        game: options.value("--game")?,
        resources: options.value("--resources")?,
        jars: options.value("--jars")?,
        extra: options.rest(),
    };
    options.finish()?;
    if parsed
        .jars
        .is_some_and(|a| a.is_empty() || a.starts_with("--"))
    {
        return Err("pass a folder to `cargo sunrise-mod run --jars <dir>`".into());
    }
    Ok(parsed)
}

fn run(args: &[String]) -> Result<(), RunError> {
    let RunArgs {
        game,
        resources,
        jars,
        extra,
    } = run_args(args).map_err(RunError::Build)?;
    let jars = jars.map(Path::new);
    let project = Path::new(".");
    let package = Package::read(project).map_err(RunError::Build)?;
    if package.is_native() {
        // `--game` and `--resources` are each read for one kind of mod: given for the other,
        // they would be dropped unsaid.
        if game.is_some() {
            return Err(RunError::Build(
                "run --game is for a content mod: a native mod runs on the game of the SDK it pins"
                    .into(),
            ));
        }
        return dev::run(Path::new("."), resources, jars, extra).map_err(RunError::Build);
    }
    if resources.is_some() {
        return Err(RunError::Build(
            "run --resources is for a native mod: for a content mod, give the game its own option after `--` (`run -- --resources <folder>`)".into(),
        ));
    }
    let home =
        jar::stage_for_run(&package.manifest, project, jars, extra).map_err(RunError::Build)?;
    // A content mod carries no uses table.
    let path = build_in(project, &package.manifest).map_err(RunError::Build)?;
    let game = game
        .map(str::to_string)
        .or_else(|| std::env::var("SUNRISE_CLIENT").ok())
        .unwrap_or_else(|| "sunrise-client".into());
    let mut command = Command::new(&game);
    if let Some(home) = home {
        command.env("SUNRISE_HOME", home);
    }
    println!("Starting {game} with {}", path.display());
    let status = command
        .arg("--dev-mod")
        .arg(&path)
        .args(extra)
        .status()
        .map_err(|error| RunError::Launch { game, error })?;
    status.success().then_some(()).ok_or(RunError::Exit(status))
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script;

#[cfg(test)]
mod tests {
    use super::*;

    const OLD_BUILD: &str = "0123456789abcdef0123456789abcdef01234567";

    /// A build's SDK as its `sdk.json` names it, without a download.
    pub(super) fn offline(project: &Path, build: &str) -> Result<sdk_fetch::PinnedSdk, String> {
        Ok(sdk_fetch::PinnedSdk {
            build: build.to_owned(),
            name: build.to_owned(),
            dir: project.join(sdk_fetch::DIR),
            surface: project.join(sdk_fetch::DIR).join("surface.json"),
        })
    }

    /// An empty list of what a mod uses, made for `made_for`.
    fn uses_list(made_for: &str) -> Vec<u8> {
        use sunrise_mod_package::mod_surface::uses::{USES_SCHEMA, UsesTable};
        UsesTable {
            schema: USES_SCHEMA,
            made_for: made_for.to_owned(),
            entries: Vec::new(),
        }
        .to_json()
        .into_bytes()
    }

    #[test]
    fn new_rejects_unknown_flags_without_creating_a_mod() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greeter");
        for arg in ["--native", "--content", "--typo", "-x", "--name"] {
            let error = new(&[path.display().to_string(), arg.into()], &offline).unwrap_err();
            assert!(error.contains(arg), "{error}");
            assert!(!path.exists());
        }
    }

    #[test]
    fn new_accepts_a_name_beginning_with_a_dash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("garden");
        new(
            &[
                path.display().to_string(),
                "--name".into(),
                "-Garden".into(),
            ],
            &offline,
        )
        .unwrap();
        assert_eq!(Package::read(&path).unwrap().manifest.name, "-Garden");
    }

    fn build_in(root: &Path) -> Result<PathBuf, String> {
        super::build_in(root, &Package::read(root)?.manifest)
    }

    fn code_args(path: &Path, template: &str) -> [String; 5] {
        [
            path.display().to_string(),
            "--template".into(),
            template.into(),
            "--sunrise".into(),
            OLD_BUILD.into(),
        ]
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_link_at_the_path_is_refused_and_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greeter");
        std::os::unix::fs::symlink(dir.path().join("nowhere"), &path).unwrap();
        let fetch = |_: &Path, _: &str| -> Result<sdk_fetch::PinnedSdk, String> {
            panic!("a refused path fetches nothing")
        };
        let error = new(&code_args(&path, "native"), &fetch).unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(
            std::fs::read_link(&path).unwrap(),
            dir.path().join("nowhere")
        );
        assert!(!dir.path().join("nowhere").exists());
    }

    #[test]
    fn a_failed_fetch_leaves_no_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greeter");
        // A fetch that got as far as its folder, as a real one does, then failed.
        let fetch = |project: &Path, _: &str| -> Result<sdk_fetch::PinnedSdk, String> {
            std::fs::create_dir_all(project.join(sdk_fetch::DIR)).unwrap();
            Err("no signed SDK".into())
        };
        assert_eq!(
            new(&code_args(&path, "native"), &fetch).unwrap_err(),
            "no signed SDK"
        );
        assert!(!path.exists());
        // A folder that was there before is refused, not removed.
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), "mine").unwrap();
        assert!(new(&code_args(&path, "native"), &fetch).is_err());
        assert_eq!(std::fs::read_to_string(path.join("keep")).unwrap(), "mine");
    }

    #[test]
    fn a_refused_update_leaves_the_manifest_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greeter");
        new(&code_args(&path, "native"), &offline).unwrap();
        let before = std::fs::read(path.join(MANIFEST)).unwrap();
        // A build whose SDK cannot be fetched: nothing is written.
        let broken = |_: &Path, _: &str| Err("no such build".to_owned());
        let newer = "89abcdef0123456789abcdef0123456789abcdef";
        assert!(update(&path, &broken, Some(newer)).is_err());
        assert_eq!(std::fs::read(path.join(MANIFEST)).unwrap(), before);
    }

    #[test]
    fn update_in_a_content_mod_says_it_pins_no_build() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gems");
        new(&[path.display().to_string()], &offline).unwrap();
        let error = update(&path, &offline, None).unwrap_err();
        assert!(error.contains("pins no game build"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_pack_leaves_packages_in_linked_output_directories_alone() {
        use std::{fs, os::unix::fs::symlink};

        let project = tempfile::tempdir().unwrap();
        let root = project.path().join("gems");
        new(&[root.display().to_string()], &offline).unwrap();
        fs::write(
            root.join("assets/gems/models/block/glowing_amethyst.json"),
            r#"{"parent":"minecraft:block/cube_all","textures":{"all":"gems:block/gone"}}"#,
        )
        .unwrap();

        let outside = tempfile::tempdir().unwrap();
        let outside_output = outside.path().join("sunrise-mod");
        fs::create_dir(&outside_output).unwrap();
        let previous = outside_output.join("gems.smod");
        fs::write(&previous, b"previous package").unwrap();

        let target = root.join("target");
        symlink(outside.path(), &target).unwrap();
        assert!(build_in(&root).is_err());
        assert_eq!(fs::read(&previous).unwrap(), b"previous package");

        fs::remove_file(&target).unwrap();
        fs::create_dir(&target).unwrap();
        symlink(&outside_output, target.join("sunrise-mod")).unwrap();
        assert!(build_in(&root).is_err());
        assert_eq!(fs::read(&previous).unwrap(), b"previous package");
    }

    #[test]
    fn the_content_template_builds_and_checks_with_no_code() {
        let dir = std::env::temp_dir().join(format!("sunrise-mod-content-{}", std::process::id()));
        let path = dir.join("gems");
        let new_args = [
            path.display().to_string(),
            "--template".into(),
            "content".into(),
        ];
        new(&new_args, &offline).unwrap();
        assert!(!path.join("Cargo.toml").exists());
        assert!(!path.join(".github").exists());
        let smod = build_in(&path).unwrap();
        assert_eq!(smod, path.join("target/sunrise-mod/gems.smod"));
        let package = Package::read(&smod).unwrap();
        let content = Content::read(&smod, &package.manifest).unwrap();
        assert!(
            content
                .files
                .contains(&"data/gems/sunrise/block/glowing_amethyst.json".into())
        );
        assert_eq!(content.files.len(), 8, "{:?}", content.files);
        let report = check::check(&smod, check::kind_of(&smod).unwrap(), None, None).unwrap();
        assert!(
            report.text().contains("no code: content only"),
            "{}",
            report.text()
        );
        // A build that fails its reference checks leaves no earlier package behind.
        let model = path.join("assets/gems/models/block/glowing_amethyst.json");
        let original = std::fs::read(&model).unwrap();
        std::fs::write(
            &model,
            r#"{"parent":"minecraft:block/cube_all","textures":{"all":"gems:block/gone"}}"#,
        )
        .unwrap();
        let error = build_in(&path).unwrap_err();
        assert!(error.contains("the texture gems:block/gone"), "{error}");
        assert!(!smod.exists());
        std::fs::write(&model, original).unwrap();
        let executable = path.join("assets/gems/mod.exe");
        std::fs::write(&executable, b"\0asm").unwrap();
        assert!(
            build_in(&path)
                .unwrap_err()
                .contains("unsupported executable file")
        );
        std::fs::remove_file(executable).unwrap();
        // Code is native now.
        std::fs::write(path.join("Cargo.toml"), "").unwrap();
        assert!(build_in(&path).unwrap_err().contains("native"));
        assert!(
            new(
                &[
                    dir.join("x").display().to_string(),
                    "--template".into(),
                    "lua".into()
                ],
                &offline
            )
            .is_err()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn native_build_and_check_share_the_content_executable_boundary() {
        use std::io::Write;
        use sunrise_mod_package::manifest::ContentFileError;

        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("native-content-contract-{}", std::process::id()));
        let path = dir.join("mixed");
        let _ = std::fs::remove_dir_all(&dir);
        new(
            &[
                path.display().to_string(),
                "--template".into(),
                "native".into(),
                "--sunrise".into(),
                "0123456789abcdef0123456789abcdef01234567".into(),
            ],
            &offline,
        )
        .unwrap();
        let asset = "assets/mixed/program.bin";
        std::fs::create_dir_all(path.join("assets/mixed")).unwrap();
        std::fs::write(path.join(asset), b"ordinary asset").unwrap();
        // Only selected content ships, not the Git project's executable hooks.
        std::fs::create_dir_all(path.join(".git/hooks")).unwrap();
        std::fs::write(path.join(".git/hooks/pre-commit"), b"#!/bin/sh\nexit 0\n").unwrap();
        let smod = pack_into(
            &path,
            &PackageOutput::in_project(&path).unwrap(),
            &Package::read(&path).unwrap().manifest,
            Some(&uses_list("0123456789abcdef0123456789abcdef01234567")),
            None,
        )
        .unwrap();
        let kind = check::kind_of(&smod).unwrap();
        assert!(check::check(&smod, kind.clone(), None, None).is_ok());

        // Model an externally supplied archive with executable bytes under an asset name.
        let original = std::fs::read(&smod).unwrap();
        let mut input = zip::ZipArchive::new(std::io::Cursor::new(original)).unwrap();
        let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        for i in 0..input.len() {
            let mut entry = input.by_index(i).unwrap();
            let name = entry.name().unwrap().into_owned();
            output.start_file(&name, options).unwrap();
            if name == asset {
                output.write_all(b"MZ-not-an-asset").unwrap();
            } else {
                std::io::copy(&mut entry, &mut output).unwrap();
            }
        }
        std::fs::write(&smod, output.finish().unwrap().into_inner()).unwrap();
        let expected = ContentFileError::Executable(asset.into()).to_string();
        assert_eq!(
            check::check(&smod, kind, None, None).err().unwrap(),
            expected
        );
        std::fs::remove_file(&smod).unwrap();
        std::fs::write(path.join(asset), b"MZ-not-an-asset").unwrap();
        assert_eq!(build_in(&path).unwrap_err(), expected);
        assert!(!smod.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_supported_explicit_mixins_pin_is_in_the_package_facts() {
        use sunrise_mod_package::{mixins_tool, sdk::Mixins};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("example");
        new(&code_args(&root, "native"), &offline).unwrap();
        let manifest = root.join(MANIFEST);
        let text = std::fs::read_to_string(&manifest).unwrap();
        let rev = mixins_tool::REV;
        std::fs::write(
            manifest,
            format!("{text}\nmixins = '{rev}'\n[[native.mixin]]\nid='tick'\nkind='HEAD'\ntarget='game::tick'\nhelper='src/tick.rs'\n"),
        )
        .unwrap();
        std::fs::write(root.join("src/tick.rs"), "pub fn tick() {}\n").unwrap();
        let bundle = sunrise_native_build::bundle_in(&root).unwrap().unwrap();
        assert!(bundle.needs_mixins());
        let sdk = Mixins {
            rev: rev.into(),
            patch: mixins_tool::patch_hash(),
        };
        let identity =
            sunrise_native_build::MixinsIdentity::resolve([&bundle], Some(&sdk)).unwrap();
        let smod = pack_into(
            &root,
            &PackageOutput::in_project(&root).unwrap(),
            &Package::read(&root).unwrap().manifest,
            None,
            Some(identity.revision()),
        )
        .unwrap();
        let packed = sunrise_mod_package::index::native::Bundle::read(&smod).unwrap();
        assert_eq!(packed.facts().api, format!("native-1/{OLD_BUILD}/{rev}"));
    }

    #[test]
    fn the_native_template_packs_into_an_smod_the_registry_accepts() {
        let dir = std::env::temp_dir().join(format!("sunrise-mod-native-{}", std::process::id()));
        let path = dir.join("machine-mod");
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let args = [
            path.display().to_string(),
            "--template".into(),
            "native".into(),
            "--sunrise".into(),
            commit.into(),
        ];
        assert_eq!(new(&args, &offline).unwrap(), Some(path.clone()));
        for file in [
            "README.md",
            ".github/workflows/release.yml",
            ".github/workflows/rebuild.yml",
            ".github/workflows/sunrise-release-jobs.yml",
            ".github/actions/sunrise-release-tool/action.yml",
        ] {
            assert!(path.join(file).is_file(), "template missing {file}");
        }
        let github_files: Vec<_> = std::fs::read_dir(path.join(".github"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(github_files.len(), 2);
        assert!(github_files.contains(&"actions".into()));
        assert!(github_files.contains(&"workflows".into()));
        // The template writes none of the fields the tool derives.
        let text = std::fs::read_to_string(path.join(MANIFEST)).unwrap();
        for key in ["api", "capabilities", "mixins"] {
            assert!(
                !text.lines().any(|l| l.starts_with(&format!("{key} "))),
                "{text}"
            );
        }
        let written = Manifest::parse(&text).unwrap();
        assert_eq!(written.native.as_ref().unwrap().mixins, None);
        let api = std::fs::read_to_string(path.join("Cargo.toml")).unwrap();
        // The SDK by path, never git: the build is the manifest's pin.
        assert!(
            api.contains(
                r#"sunrise-mod-api = { path = ".sunrise/sdk/api/crates/sunrise-mod-api" }"#
            ),
            "{api}"
        );
        assert!(
            !api.contains("git") && !api.contains("[package.metadata.sunrise]"),
            "{api}"
        );
        assert_eq!(sdk_fetch::pin(&path).unwrap(), commit);
        // No rows: the package still carries the SDK's revision for readers that require it.
        let rev = "fedcba9876543210fedcba9876543210fedcba98";
        let smod = pack_into(
            &path,
            &PackageOutput::in_project(&path).unwrap(),
            &written,
            None,
            Some(rev),
        )
        .unwrap();
        // The package carries what the tool derives, which the project did not write.
        let packed = Package::entry(&smod, MANIFEST, 1 << 20).unwrap().unwrap();
        let packed = String::from_utf8(packed).unwrap();
        assert!(packed.contains("api = \"0.2\""), "{packed}");
        assert!(
            packed.contains("capabilities = [\"registries\"]"),
            "{packed}"
        );
        let bundle = sunrise_mod_package::index::native::Bundle::read(&smod).unwrap();
        assert_eq!(bundle.manifest.id, "machine_mod");
        assert_eq!(bundle.facts().api, format!("native-1/{commit}/{rev}"));
        let names: Vec<_> = bundle.files.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            ["Cargo.toml", "README.md", "src/lib.rs", "sunrise-mod.toml"]
        );
        assert!(path.join("sunrise-mod.toml").is_file());
        for old in [
            "api",
            "mixins",
            "content",
            "native",
            "rust",
            "sunrise-native.json",
            "horizon-mixin.toml",
        ] {
            assert!(!path.join(old).exists());
        }
        assert_eq!(
            check::kind_of(&smod),
            Some(sunrise_mod_package::index::Kind::NativeMod)
        );
        assert_eq!(check::kind_of(&dir.join("machine_mod.hnmod")), None);
        // Without the list of what it uses, the mod would work on its pinned build only.
        let refused = check::check(&smod, check::kind_of(&smod).unwrap(), None, None)
            .err()
            .unwrap();
        assert!(refused.contains("uses.json"), "{refused}");
        let pack = |made_for: &str| {
            let smod = pack_into(
                &path,
                &PackageOutput::in_project(&path).unwrap(),
                &written,
                Some(&uses_list(made_for)),
                Some(rev),
            )
            .unwrap();
            check::check(&smod, check::kind_of(&smod).unwrap(), None, None)
        };
        // A list made for another build says nothing about the pinned one.
        assert!(pack(&"f".repeat(40)).is_err());
        let report = pack(commit).unwrap();
        assert_eq!(report.facts.id.as_deref(), Some("machine_mod"));
        // A row no list can judge ties the mod to its build, so its file needs no list.
        let project = path.join("sunrise-mod.toml");
        let text = std::fs::read_to_string(&project).unwrap();
        let row = "\n[[native.module]]\nid = \"m\"\nfile = \"game.rs\"\nname = \"m\"\nsource = \"src/m.rs\"\n";
        std::fs::write(&project, format!("{text}{row}")).unwrap();
        let exact = Package::read(&path).unwrap().manifest;
        std::fs::write(path.join("src/m.rs"), "").unwrap();
        let smod = pack_into(
            &path,
            &PackageOutput::in_project(&path).unwrap(),
            &exact,
            None,
            Some(rev),
        )
        .unwrap();
        check::check(&smod, check::kind_of(&smod).unwrap(), None, None).unwrap();
        std::fs::remove_file(path.join("src/m.rs")).unwrap();
        std::fs::write(&project, text).unwrap();
        assert!(
            new(
                &[
                    dir.join("bad").display().to_string(),
                    "--template".into(),
                    "native".into(),
                    "--sunrise".into(),
                    "main".into(),
                ],
                &offline
            )
            .unwrap_err()
            .contains("is not a build")
        );
        let newer = "89abcdef0123456789abcdef0123456789abcdef";
        update(&path, &offline, Some(newer)).unwrap();
        let manifest =
            Manifest::parse(&std::fs::read_to_string(path.join(MANIFEST)).unwrap()).unwrap();
        let native = manifest.native.unwrap();
        assert_eq!(native.sunrise, newer);
        // `update` moves the pin only: the SDK names the tool.
        assert_eq!(native.mixins, None);
        assert_eq!(
            std::fs::read_to_string(path.join("Cargo.toml")).unwrap(),
            api
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn argv(line: &str) -> Vec<String> {
        line.split(' ')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn build_run_update_and_check_read_their_options_and_refuse_others() {
        assert_eq!(build_args(&[]), Ok((false, BuildMode::Package)));
        assert_eq!(
            build_args(&argv("--update-lock")),
            Ok((true, BuildMode::Package))
        );
        let line = argv("--game g --resources r --jars j -- --relase");
        assert_eq!(
            run_args(&line),
            Ok(RunArgs {
                game: Some("g"),
                resources: Some("r"),
                jars: Some("j"),
                extra: &line[7..],
            })
        );
        let line = argv("--sunrise nightly");
        assert_eq!(update_args(&line), Ok(Some("nightly")));
        for (error, option) in [
            (
                build_args(&argv("--relase")).map(drop).unwrap_err(),
                "--relase",
            ),
            (
                build_args(&argv("--update-lock --typo"))
                    .map(drop)
                    .unwrap_err(),
                "--typo",
            ),
            (run_args(&argv("--gme g")).map(drop).unwrap_err(), "--gme"),
            (
                update_args(&argv("nightly-2026-11-20"))
                    .map(drop)
                    .unwrap_err(),
                "nightly-2026-11-20",
            ),
            (
                update_args(&argv("--sunrise nightly --acept a"))
                    .map(drop)
                    .unwrap_err(),
                "--acept",
            ),
            (
                check_args(&argv("mod.smod --jsno")).map(drop).unwrap_err(),
                "--jsno",
            ),
        ] {
            assert!(error.contains(option), "{error}");
        }
    }

    #[test]
    fn check_takes_its_file_wherever_it_stands_and_refuses_the_other_forms_options() {
        let file = CheckFile {
            file: "mod.smod",
            kind: None,
            key: None,
            signature: None,
            json: true,
        };
        assert_eq!(check_args(&argv("--json mod.smod")), Ok(Check::File(file)));
        // A value that reads like the other form's option is still the value.
        assert_eq!(
            check_args(&argv("mod.smod --key KEY --signature --native")),
            Ok(Check::File(CheckFile {
                file: "mod.smod",
                kind: None,
                key: Some("KEY"),
                signature: Some("--native"),
                json: false,
            }))
        );
        assert_eq!(
            check_args(&argv("--uses")),
            Ok(Check::Folder { uses: true })
        );
        for (line, named) in [
            ("--json", "--json"),
            ("mod.smod --uses", "--uses"),
            ("a.smod b.smod", "b.smod"),
            ("--use", "--use"),
        ] {
            let error = check_args(&argv(line)).map(drop).unwrap_err();
            assert!(error.contains(named), "{line}: {error}");
        }
    }

    #[test]
    fn served_by_package_with_host_crafting_remainder_packs_and_checks() {
        let dir = std::env::temp_dir().join(format!(
            "sunrise-mod-tool-served-rem-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("runes");
        std::fs::create_dir_all(root.join("data/runes/sunrise/item")).unwrap();
        std::fs::create_dir_all(root.join("data/runes/recipe")).unwrap();
        let manifest_text = "\
id = \"runes\"
name = \"Runes\"
version = \"1.0.0\"
side = \"both\"
namespaces = [\"runes_extra\"]
served_by = \"horizon_java\"
";
        std::fs::write(root.join(MANIFEST), manifest_text).unwrap();
        // Item references a host-declared crafting remainder in a served namespace, and a recipe
        // references a host-declared item in the same namespace.
        std::fs::write(
            root.join("data/runes/sunrise/item/stew.json"),
            r#"{"template":"minecraft:mushroom_stew","crafting_remainder":"runes_extra:host_bowl"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("data/runes/recipe/host_bowl.json"),
            r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:oak_planks"],"result":{"id":"runes_extra:host_bowl"}}"#,
        )
        .unwrap();
        let manifest = Package::read(&root).unwrap().manifest;
        let smod = pack_into(
            &root,
            &PackageOutput::in_project(&root).unwrap(),
            &manifest,
            None,
            None,
        )
        .unwrap();
        let report = check::check(&smod, check::kind_of(&smod).unwrap(), None, None).unwrap();
        assert_eq!(report.facts.id.as_deref(), Some("runes"));

        // Invalid syntax, minecraft:air, foreign namespace, or unserved undeclared remainder still fail.
        for (label, manifest_override, remainder) in [
            ("air", None, "minecraft:air"),
            ("bad-id", None, "runes:Bad"),
            ("foreign", None, "other:bowl"),
            (
                "unserved",
                Some(manifest_text.replace("served_by = \"horizon_java\"\n", "")),
                "runes_extra:host_bowl",
            ),
        ] {
            if let Some(text) = &manifest_override {
                std::fs::write(root.join(MANIFEST), text).unwrap();
            } else {
                std::fs::write(root.join(MANIFEST), manifest_text).unwrap();
            }
            std::fs::write(
                root.join("data/runes/sunrise/item/stew.json"),
                format!(
                    r#"{{"template":"minecraft:mushroom_stew","crafting_remainder":"{remainder}"}}"#
                ),
            )
            .unwrap();
            let manifest = Package::read(&root).unwrap().manifest;
            assert!(
                pack_into(
                    &root,
                    &PackageOutput::in_project(&root).unwrap(),
                    &manifest,
                    None,
                    None,
                )
                .is_err(),
                "{label} ({remainder}) should fail pack_into"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
