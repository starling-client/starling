//! Native libraries built against one build of a dependency mod are refused beside another build
//! of it, before any of their code runs: real libraries, built by `cargo sunrise-mod run` against
//! this run's SDK, installed in mixed sets and started as the game starts them.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// What a library of `native` that `test_addon` links prints when it registers.
const REGISTERED: &str = "addon registered with its library";

/// One scratch folder: mods made by the tool, the stub resources a dedicated start registers
/// with, and the installations each `run` left.
struct Scratch {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        // book-quickstart assembles and signs this run's SDK, as for the add-on test.
        let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
        fs::create_dir_all(target).unwrap();
        let dir = tempfile::tempdir_in(target).unwrap();
        let root = dir.path().canonicalize().unwrap();
        let resources = root.join("resources");
        fs::create_dir(&resources).unwrap();
        for (name, json) in [
            (
                "sunrise-resources.json",
                r#"{"schema":7,"minecraft":"26.3"}"#,
            ),
            ("sunrise-prepare.json", r#"{"output_version":9}"#),
            ("registries.json", "{}"),
        ] {
            fs::write(resources.join(name), json).unwrap();
        }
        Self { _dir: dir, root }
    }

    fn tool(&self, dir: &Path, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
            .current_dir(dir)
            .env("SUNRISE_MOD_CACHE", self.root.join("cache"))
            .env("SUNRISE_HOME", self.root.join("home"))
            .env("SUNRISE_LAUNCHER_HOME", self.root.join("launcher"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .args(args)
            .output()
            .unwrap();
        passed(output)
    }

    /// A starter native mod `id` at version 1.0.0 with `source` as its library, `dependencies`
    /// as its manifest's `[dependencies]` rows and `crates` added to its Cargo dependencies.
    fn new_mod(&self, id: &str, source: &str, dependencies: &str, crates: &str) -> PathBuf {
        self.tool(&self.root, &["new", id, "--template", "native"]);
        let folder = self.root.join(id);
        fs::write(folder.join("src/lib.rs"), source).unwrap();
        let manifest = folder.join("sunrise-mod.toml");
        let text = fs::read_to_string(&manifest)
            .unwrap()
            .replace("version = \"0.1.0\"", "version = \"1.0.0\"");
        fs::write(
            &manifest,
            format!("{text}\n[dependencies]\n{dependencies}\n"),
        )
        .unwrap();
        let cargo = folder.join("Cargo.toml");
        let text = fs::read_to_string(&cargo).unwrap();
        fs::write(&cargo, format!("{text}{crates}")).unwrap();
        folder
    }

    /// `cargo sunrise-mod run` of `addon` with `libraries`, on a dedicated server whose
    /// registration exits; the installation it made, kept as `name`.
    fn run(&self, name: &str, addon: &Path, libraries: &[&Path]) -> PathBuf {
        let run = addon.join("target/sunrise-mod/run");
        fs::create_dir_all(run.join("game")).unwrap();
        fs::write(run.join("game/eula.txt"), "eula=true\n").unwrap();
        let resources = self.root.join("resources");
        let mut args = vec!["run"];
        for library in libraries {
            args.extend(["--with", library.to_str().unwrap()]);
        }
        args.extend([
            "--resources",
            resources.to_str().unwrap(),
            "--",
            "--dedicated",
        ]);
        let stdout = self.tool(addon, &args);
        assert!(stdout.contains(REGISTERED), "{stdout}");
        let executable = stdout
            .lines()
            .find_map(|line| line.strip_prefix("Starting "))
            .and_then(|line| line.split(" with ").next())
            .map(PathBuf::from)
            .expect("run names the program it starts");
        let kept = self.root.join("builds").join(name);
        copy_dir(&run.join("app"), &kept);
        fs::write(
            kept.join(".program"),
            executable.file_name().unwrap().as_encoded_bytes(),
        )
        .unwrap();
        kept
    }

    /// `base` with the libraries (file and record row) of each `(build, mod)` in `swaps` in place
    /// of its own, as an installation mixing two builds would hold them.
    fn mixed(&self, name: &str, base: &Path, swaps: &[(&Path, &str)]) -> PathBuf {
        let folder = self.root.join("mixed").join(name);
        copy_dir(base, &folder);
        let mut installed = record(&folder);
        for (build, id) in swaps {
            let row = row(&record(build), id).clone();
            let file = row["file"].as_str().unwrap();
            fs::copy(build.join(file), folder.join(file)).unwrap();
            let rows = installed["libraries"].as_array_mut().unwrap();
            let at = rows.iter().position(|old| old["mod"] == *id).unwrap();
            rows[at] = row;
        }
        fs::write(folder.join("native-mods.json"), installed.to_string()).unwrap();
        folder
    }

    /// Starts the installation's program as `run` does, on the same game folder.
    fn start(&self, installation: &Path, addon: &Path) -> Output {
        let program = fs::read_to_string(installation.join(".program")).unwrap();
        let home = tempfile::tempdir_in(&self.root).unwrap();
        let mut command = Command::new(installation.join(program));
        command
            .arg("--game-dir")
            .arg(addon.join("target/sunrise-mod/run/game"))
            .arg("--resources")
            .arg(self.root.join("resources"))
            .arg("--dedicated")
            .env("SUNRISE_HOME", home.path())
            .env("SUNRISE_NO_AUDIO", "1")
            .env_remove("SUNRISE_SESSION_FILE");
        #[cfg(target_os = "linux")]
        command.env("LD_LIBRARY_PATH", installation);
        command.output().unwrap()
    }
}

fn passed(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// The start refused the installation, naming `dependent` and the `dependency` it was built
/// against another build of, before registering anything.
fn assert_refused(start: Output, dependent: &str, dependency: &str) {
    let stdout = String::from_utf8_lossy(&start.stdout);
    let stderr = String::from_utf8_lossy(&start.stderr);
    assert!(
        !start.status.success()
            && !stdout.contains(REGISTERED)
            && stderr.contains(&format!(
                "mod {dependent}: built against another build of {dependency}"
            )),
        "{stdout}\n{stderr}"
    );
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn record(installation: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(installation.join("native-mods.json")).unwrap())
        .unwrap()
}

fn row<'a>(record: &'a Value, id: &str) -> &'a Value {
    record["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["mod"] == id)
        .unwrap_or_else(|| panic!("no library of {id} in {record}"))
}

/// The `source` field of `id`'s library identity in `installation`.
fn identity_source(installation: &Path, id: &str) -> String {
    let record = record(installation);
    let identity = row(&record, id)["identity"].as_str().unwrap().to_owned();
    identity.split(' ').nth(4).unwrap().to_owned()
}

const ADDON: &str = r#"
#[derive(Default)]
struct Addon;
#[mod_api::sunrise_mod]
impl mod_api::Hooks for Addon {
    fn register(&self, _: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        assert_eq!(native::magic(), 1729);
        println!("addon registered with its library");
        std::process::exit(0);
    }
}
"#;

/// A library mod's source: an empty mod with `functions` beside it.
fn library(functions: &str) -> String {
    format!(
        "#[derive(Default)]\nstruct Library;\n#[mod_api::sunrise_mod]\nimpl mod_api::Hooks for Library {{}}\n{functions}\n"
    )
}

/// `test_addon` links `native`, which links `base`. A library from one build and its dependent
/// from another are refused whichever of the two changed, and when only the dependency's own
/// dependency changed; the same identity with other bytes (a rebuild of the same source) is not.
#[test]
#[ignore = "daily"]
fn a_library_built_against_another_build_of_a_native_dependency_is_refused() {
    let scratch = Scratch::new();
    let base = scratch.new_mod("base", &library("pub fn value() -> u32 { 1728 }"), "", "");
    let native = scratch.new_mod(
        "native",
        &library("pub fn magic() -> u32 { base::value() + 1 }"),
        "base = '^1'",
        "",
    );
    let addon = scratch.new_mod("test_addon", ADDON, "native = '^1'", "");
    let first = scratch.run("first", &addon, &[&native, &base]);

    // The library's source changes: it and its dependent are rebuilt.
    let source = native.join("src/lib.rs");
    let text = fs::read_to_string(&source).unwrap();
    fs::write(
        &source,
        format!("{text}\npub struct Padded {{ pub pad: u64, pub value: u32 }}\n"),
    )
    .unwrap();
    let changed = scratch.run("changed", &addon, &[&native, &base]);
    fs::write(&source, text).unwrap();
    let new_library = scratch.mixed("new-library", &first, &[(&changed, "native")]);
    assert_refused(scratch.start(&new_library, &addon), "test_addon", "native");
    let new_dependent = scratch.mixed("new-dependent", &first, &[(&changed, "test_addon")]);
    assert_refused(
        scratch.start(&new_dependent, &addon),
        "test_addon",
        "native",
    );

    // Only the library's own dependency changes: the library is rebuilt from the same source.
    let source = base.join("src/lib.rs");
    let text = fs::read_to_string(&source).unwrap();
    fs::write(&source, format!("{text}\npub fn other() -> u64 {{ 7 }}\n")).unwrap();
    let deeper = scratch.run("deeper", &addon, &[&native, &base]);
    assert_eq!(
        identity_source(&first, "native"),
        identity_source(&deeper, "native")
    );
    let new_chain = scratch.mixed(
        "new-chain",
        &first,
        &[(&deeper, "base"), (&deeper, "native")],
    );
    assert_refused(scratch.start(&new_chain, &addon), "test_addon", "native");

    // Other bytes under the same identity, as a rebuild of the same source may make, still load.
    let rebuilt = scratch.mixed("rebuilt", &first, &[]);
    let mut installed = record(&rebuilt);
    let rows = installed["libraries"].as_array_mut().unwrap();
    let row = rows.iter_mut().find(|row| row["mod"] == "native").unwrap();
    let file = rebuilt.join(row["file"].as_str().unwrap());
    let mut bytes = fs::read(&file).unwrap();
    bytes.push(0);
    fs::write(&file, &bytes).unwrap();
    row["sha256"] = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(&bytes)).into()
    };
    fs::write(rebuilt.join("native-mods.json"), installed.to_string()).unwrap();
    let stdout = passed(scratch.start(&rebuilt, &addon));
    assert!(stdout.contains(REGISTERED), "{stdout}");
}

/// On the crates.io route one build resolves every mod's crates together: another mod in the
/// set that turns on a crate's default features changes what `native` is compiled with though
/// its files did not change, so its library from that set is refused beside a dependent built
/// in a set without it.
#[test]
#[ignore = "daily"]
fn a_library_whose_crates_another_mod_changed_is_refused_beside_its_dependent() {
    let scratch = Scratch::new();
    let native = scratch.new_mod(
        "native",
        &library("pub fn magic() -> u32 { ascii::AsciiChar::A as u32 + 1664 }"),
        "",
        "ascii = { version = \"1\", default-features = false }\n",
    );
    let pad = scratch.new_mod(
        "pad",
        &library("pub fn room() -> usize { ascii::AsciiChar::B as usize }"),
        "",
        "ascii = \"1\"\n",
    );
    // Each library's own build pins its crates in its Cargo.lock.
    for library in [&native, &pad] {
        scratch.tool(library, &["build"]);
    }
    let addon = scratch.new_mod(
        "test_addon",
        ADDON,
        "native = '^1'\npad = { version = '^1', optional = true }",
        "",
    );
    let alone = scratch.run("alone", &addon, &[&native]);
    let padded = scratch.run("padded", &addon, &[&native, &pad]);
    let (alone_source, padded_source) = (
        identity_source(&alone, "native"),
        identity_source(&padded, "native"),
    );
    let split = |source: &str| {
        let (files, graph) = source.split_once('+').expect("a crates.io graph digest");
        (files.to_owned(), graph.to_owned())
    };
    let (alone_files, alone_graph) = split(&alone_source);
    let (padded_files, padded_graph) = split(&padded_source);
    assert_eq!(alone_files, padded_files);
    assert_ne!(alone_graph, padded_graph);
    let mixed = scratch.mixed("mixed", &alone, &[(&padded, "native")]);
    assert_refused(scratch.start(&mixed, &addon), "test_addon", "native");
}
