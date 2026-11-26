//! Two native mods built by `cargo sunrise-mod` use mod events together in a real dedicated
//! server: `chemlib` declares `chemlib:reaction` and posts it with `host.post` from its
//! `server_started` hook; `boiler`, which depends on it, listens with `#[subscribe]` from its own
//! dylib. The listeners print markers and the test reads the game's output for them.
//!
//! Run 1 (both installed): the listeners run by descending priority (declared in the opposite
//! order), a listener returning `Flow::Cancel` keeps the normal-priority listener from running,
//! the `receive_cancelled` listener still runs, and the poster sees the cancelled result.
//! Run 2 (`chemlib` not installed, `boiler` depends on it optionally): `boiler` loads and starts
//! without its listeners.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const CHEMLIB_SOURCE: &str = r#"
#[derive(Default)]
struct ChemLib;

#[derive(mod_api::Event)]
#[event(id = "chemlib:reaction", side = server, cancellable)]
#[non_exhaustive]
pub struct Reaction {
    pub heat: u32,
    pub cancelled: bool,
}

#[mod_api::sunrise_mod]
impl mod_api::Hooks for ChemLib {
    fn server_started(
        &self,
        host: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ServerStartedEvent,
    ) {
        for heat in [10, 2000] {
            let mut event = Reaction { heat, cancelled: false };
            let cancelled = host.post(&mut event).cancelled;
            println!("m2m_mod_events posted heat={heat} cancelled={cancelled}");
        }
        std::process::exit(0);
    }
}
"#;

/// The listeners are declared from the lowest priority to the highest, so a run in priority
/// order is not a run in declaration order.
const BOILER_SOURCE: &str = r#"
#[derive(Default)]
struct Boiler;

#[mod_api::sunrise_mod]
impl mod_api::Hooks for Boiler {
    #[cfg(feature = "chemlib")]
    #[subscribe(priority = -10, receive_cancelled)]
    fn last(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut chemlib::Reaction,
    ) -> mod_api::Flow {
        println!(
            "m2m_mod_events listener last heat={} cancelled={}",
            event.heat, event.cancelled
        );
        mod_api::Flow::Continue
    }

    #[cfg(feature = "chemlib")]
    #[subscribe(priority = 0)]
    fn normal(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut chemlib::Reaction,
    ) -> mod_api::Flow {
        println!("m2m_mod_events listener normal heat={}", event.heat);
        mod_api::Flow::Continue
    }

    #[cfg(feature = "chemlib")]
    #[subscribe(priority = 10)]
    fn first(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut chemlib::Reaction,
    ) -> mod_api::Flow {
        println!("m2m_mod_events listener first heat={}", event.heat);
        if event.heat > 1000 {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }

    fn server_started(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::ServerStartedEvent,
    ) {
        // With chemlib installed its hook posts and exits; without it nothing else will.
        if cfg!(not(feature = "chemlib")) {
            println!("m2m_mod_events boiler_alone");
            std::process::exit(0);
        }
    }
}
"#;

fn passed(output: Output) -> String {
    assert!(
        output.status.success(),
        "command failed ({:?})\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
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

/// The prepared server resources a dedicated server starts a world with.
fn server_profile() -> PathBuf {
    if let Some(path) = std::env::var_os("SUNRISE_TEST_SERVER_PROFILE") {
        let path = PathBuf::from(path);
        assert!(
            path.join("sunrise-resources.json").is_file(),
            "SUNRISE_TEST_SERVER_PROFILE ({}) has no sunrise-resources.json",
            path.display()
        );
        return path;
    }
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client");
    for candidate in [local.join("26.3-server"), local.join("26.3")] {
        if candidate.join("sunrise-resources.json").is_file() {
            return candidate.canonicalize().unwrap();
        }
    }
    panic!("set SUNRISE_TEST_SERVER_PROFILE to prepared 26.3 server resources");
}

/// The `m2m_mod_events` lines of a game's output, in the order the game printed them.
fn markers(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|line| line.starts_with("m2m_mod_events "))
        .collect()
}

#[test]
#[ignore = "daily"]
fn two_separately_built_mods_cross_a_mod_event_in_a_real_game() {
    let profile = server_profile();
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(target).unwrap();
    let scratch = tempfile::tempdir_in(target).unwrap();
    let root = scratch.path().canonicalize().unwrap();

    // `run --initSettings` only stages the game with the mods, so it takes stub resources.
    let stub_resources = root.join("stub-resources");
    fs::create_dir(&stub_resources).unwrap();
    for (name, json) in [
        (
            "sunrise-resources.json",
            r#"{"schema":7,"minecraft":"26.3"}"#,
        ),
        ("sunrise-prepare.json", r#"{"output_version":9}"#),
        ("registries.json", "{}"),
    ] {
        fs::write(stub_resources.join(name), json).unwrap();
    }

    let tool = |dir: &Path, args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
            .current_dir(dir)
            .env("SUNRISE_MOD_CACHE", root.join("cache"))
            .env("SUNRISE_HOME", root.join("home"))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .args(args)
            .output()
            .unwrap();
        passed(output)
    };

    // chemlib: the library mod, version 1.0.0.
    tool(&root, &["new", "chemlib", "--template", "native"]);
    let chemlib = root.join("chemlib");
    fs::write(chemlib.join("src/lib.rs"), CHEMLIB_SOURCE).unwrap();
    let manifest = fs::read_to_string(chemlib.join("sunrise-mod.toml"))
        .unwrap()
        .replace("0.1.0", "1.0.0");
    fs::write(chemlib.join("sunrise-mod.toml"), manifest).unwrap();

    // boiler: depends on chemlib optionally, so `cargo sunrise-mod` turns on its `chemlib`
    // feature only when the library is supplied.
    tool(&root, &["new", "boiler", "--template", "native"]);
    let boiler = root.join("boiler");
    fs::write(boiler.join("src/lib.rs"), BOILER_SOURCE).unwrap();
    let manifest = fs::read_to_string(boiler.join("sunrise-mod.toml")).unwrap();
    fs::write(
        boiler.join("sunrise-mod.toml"),
        format!("{manifest}\n[dependencies]\nchemlib = {{ version = '^1', optional = true }}\n"),
    )
    .unwrap();

    // boiler's installation with `libraries` supplied, kept as `name`, and the program in it.
    let stage = |name: &str, libraries: &[&Path]| -> (PathBuf, String) {
        let run = boiler.join("target/sunrise-mod/run");
        let _ = fs::remove_dir_all(&run);
        fs::create_dir_all(run.join("game")).unwrap();
        fs::write(run.join("game/eula.txt"), "eula=true\n").unwrap();
        let mut args = vec!["run"];
        for library in libraries {
            args.extend(["--with", library.to_str().unwrap()]);
        }
        args.extend([
            "--resources",
            stub_resources.to_str().unwrap(),
            "--",
            "--dedicated",
            "--initSettings",
        ]);
        let stdout = tool(&boiler, &args);
        let executable = stdout
            .lines()
            .find_map(|line| line.strip_prefix("Starting "))
            .and_then(|line| line.split(" with ").next())
            .map(PathBuf::from)
            .expect("run names the program it starts");
        let kept = root.join("builds").join(name);
        copy_dir(&run.join("app"), &kept);
        (
            kept,
            executable
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        )
    };

    let play = |name: &str, app: &Path, program: &str| -> String {
        let game_dir = root.join("runs").join(name);
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(game_dir.join("eula.txt"), "eula=true\n").unwrap();
        fs::write(
            game_dir.join("server.properties"),
            "online-mode=false\nserver-ip=127.0.0.1\nserver-port=0\nspawn-protection=0\nview-distance=2\nsimulation-distance=2\n",
        )
        .unwrap();
        let mut command = Command::new(app.join(program));
        command
            .current_dir(&game_dir)
            .arg("--game-dir")
            .arg(&game_dir)
            .arg("--resources")
            .arg(&profile)
            .args(["--dedicated", "--nogui", "--port", "0"])
            .env("SUNRISE_HOME", root.join("home").join(name))
            .env("SUNRISE_LAUNCHER_HOME", root.join("launcher").join(name))
            .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("SUNRISE_NO_AUDIO", "1")
            .env("XDG_DATA_HOME", root.join("data").join(name))
            .env("XDG_CONFIG_HOME", root.join("config").join(name))
            .env_remove("SUNRISE_SESSION_FILE");
        #[cfg(target_os = "linux")]
        command.env("LD_LIBRARY_PATH", app);
        passed(command.output().unwrap())
    };

    // Run 1: chemlib and boiler, two libraries in one game. Event 1 (heat 10) is not cancelled:
    // every listener runs, highest priority first. Event 2 (heat 2000) is cancelled by `first`:
    // `normal` does not run, `last` (receive_cancelled) does, and the poster sees it cancelled.
    let (both, program) = stage("both", &[&chemlib]);
    let stdout = play("both", &both, &program);
    assert_eq!(
        markers(&stdout),
        [
            "m2m_mod_events listener first heat=10",
            "m2m_mod_events listener normal heat=10",
            "m2m_mod_events listener last heat=10 cancelled=false",
            "m2m_mod_events posted heat=10 cancelled=false",
            "m2m_mod_events listener first heat=2000",
            "m2m_mod_events listener last heat=2000 cancelled=true",
            "m2m_mod_events posted heat=2000 cancelled=true",
        ],
        "chemlib + boiler:\n{stdout}"
    );

    // Run 2: boiler alone. Its listeners are compiled out and it still loads and starts.
    let (alone, program) = stage("alone", &[]);
    let stdout = play("alone", &alone, &program);
    assert_eq!(
        markers(&stdout),
        ["m2m_mod_events boiler_alone"],
        "boiler without chemlib:\n{stdout}"
    );
}
