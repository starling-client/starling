use std::{path::Path, process::Command};

#[test]
#[ignore = "daily"]
fn run_writes_native_registration_logs_to_latest_log() {
    use std::fs;
    use std::io::Write;

    let target = Path::new(env!("CARGO_TARGET_TMPDIR"));
    fs::create_dir_all(target).unwrap();
    let scratch = tempfile::tempdir_in(target).unwrap();
    let root = fs::canonicalize(scratch.path()).unwrap();
    let project = root.join("greeter");
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
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    tool(&root, &["new", "greeter", "--template", "native"]);
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(project.join("Cargo.toml"))
            .unwrap(),
        "log = \"0.4\""
    )
    .unwrap();
    fs::write(
        project.join("src/lib.rs"),
        r#"
#[derive(Default)]
struct Greeter;

#[mod_api::sunrise_mod]
impl mod_api::Hooks for Greeter {
    fn register(&self, _: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        log::info!(target: "greeter::registration", "ready for {}", "players");
        log::debug!(target: "greeter::registration", "not a visible diagnostic");
        log::logger().flush();
        std::process::exit(0);
    }
}
"#,
    )
    .unwrap();

    // Run the actual game through registration, then exit from the hook before a world or a
    // window opens. Registration only needs the resource marker and the registry names table.
    let resources = root.join("resources");
    fs::create_dir(&resources).unwrap();
    for name in ["sunrise-resources.json", "registries.json"] {
        fs::write(resources.join(name), "{}").unwrap();
    }
    let game = project.join("target/sunrise-mod/run/game");
    fs::create_dir_all(&game).unwrap();
    fs::write(game.join("eula.txt"), "eula=true\n").unwrap();
    tool(
        &project,
        &[
            "run",
            "--resources",
            resources.to_str().unwrap(),
            "--",
            "--dedicated",
        ],
    );
    let log = fs::read_to_string(game.join("logs/latest.log")).unwrap();
    assert!(
        log.lines()
            .any(|line| line.ends_with("/INFO]: [greeter::registration] ready for players")),
        "{log}"
    );
    assert!(!log.contains("not a visible diagnostic"), "{log}");
}

#[test]
#[ignore = "daily"]
fn run_of_a_native_mod_without_resources_says_how_to_make_them_before_building() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("native-run-resources-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // A native mod's folder, made by hand: the resources are asked for before anything is fetched or built.
    let project = dir.join("machine");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/lib.rs"), "").unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = 'machine'\nversion = '0.1.0'\n",
    )
    .unwrap();
    let manifest = include_str!("../template/native/sunrise-mod.toml")
        .replace("{id}", "machine")
        .replace("{name}", "Machine");
    std::fs::write(
        project.join("sunrise-mod.toml"),
        format!(
            "{manifest}\n[native]\ngame = \"26.3\"\nsunrise = \"{0}\"\nmixins = \"{0}\"\n",
            "a".repeat(40)
        ),
    )
    .unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .current_dir(&project)
        .arg("run")
        .output()
        .unwrap();
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(error.contains("--resources <folder>"), "{error}");
    assert!(error.contains("sunrise-prepare --prepare"), "{error}");
    // A native mod runs on its pinned SDK's game: `--game` is refused, not dropped. Without
    // resources, a run that did not refuse it would stop at them, before any SDK is looked up.
    let refused = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .current_dir(&project)
        .args(["run", "--game", "some-game"])
        .output()
        .unwrap();
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(error.contains("--game"), "{error}");
    assert!(
        !project.join("target").exists(),
        "run must stop before building"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn content_run_keeps_the_previous_package_on_jar_refusal_and_starts_with_the_staged_home() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use sunrise_mod_package::Package;

    const SHA512: &str = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
                          2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";
    let scratch = tempfile::tempdir().unwrap();
    let project = scratch.path().join("garden");
    fs::create_dir(&project).unwrap();
    let manifest = project.join("sunrise-mod.toml");
    let text = "id = 'garden'\nname = 'Garden'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n";
    fs::write(&manifest, text).unwrap();
    let previous = Package::pack_directory(&project).unwrap();
    let output = project.join("target/sunrise-mod/garden.smod");
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, &previous).unwrap();
    fs::write(
        &manifest,
        format!("{text}\n[[jar]]\nid = 'original'\nversion = '1.0'\nsha512 = '{SHA512}'\n"),
    )
    .unwrap();

    let source = scratch.path().join("originals");
    fs::create_dir(&source).unwrap();
    let game = scratch.path().join("fake-game");
    fs::write(
        &game,
        format!("#!/bin/sh\nset -eu\ntouch launched\ntest \"$1\" = --dev-mod\ntest -f \"$2\"\ntest \"$(cat \"$SUNRISE_HOME/jars/{SHA512}.jar\")\" = abc\n"),
    )
    .unwrap();
    fs::set_permissions(&game, fs::Permissions::from_mode(0o700)).unwrap();
    let run_with = |more: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
            .current_dir(&project)
            .env("SUNRISE_HOME", scratch.path().join("player-home"))
            .arg("run")
            .arg("--game")
            .arg(&game)
            .arg("--jars")
            .arg(&source)
            .args(more)
            .output()
            .unwrap()
    };
    let run = || run_with(&[]);

    let refused = run();
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(
        error.contains("original") && error.contains(SHA512),
        "{error}"
    );
    assert!(!project.join("launched").exists());
    assert_eq!(fs::read(&output).unwrap(), previous);

    fs::write(source.join(format!("{SHA512}.jar")), b"abc").unwrap();

    // The tool reads `--resources` for a native mod only: for this one it is refused, not dropped.
    let refused = run_with(&["--resources", "."]);
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(error.contains("--resources"), "{error}");
    assert!(!project.join("launched").exists());

    let ran = run();
    assert!(ran.status.success(), "{ran:?}");
    assert!(project.join("launched").is_file());
    assert_eq!(
        Package::read(&output).unwrap().manifest.jar[0].sha512(),
        SHA512
    );
}
