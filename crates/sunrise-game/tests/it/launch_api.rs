//! The Launch API's promises about how `sunrise-client` ends (docs/api/launch.md), checked on
//! the real program: `--launch-api` prints JSON and exits 0, a refused command line exits 2
//! without a crash report, and an error exits 1 with the report's path as the last stderr line,
//! with `--home` and `--game-dir` taken relative to where the game was started.
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A scratch folder of its own, with a space in its name as launchers' instance folders have.
fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("sunrise-launch-api {name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The game started in `cwd` with `args`, its homes kept inside `cwd`.
fn run(cwd: &Path, args: &[&str]) -> Output {
    run_command(
        cwd,
        Command::new(env!("CARGO_BIN_EXE_sunrise-client")).args(args),
    )
}

/// Run a prepared command with the same isolated homes as a direct game launch.
fn run_command(cwd: &Path, command: &mut Command) -> Output {
    command
        .current_dir(cwd)
        .env("SUNRISE_HOME", cwd.join("env home"))
        .env("SUNRISE_LAUNCHER_HOME", cwd.join("launcher"))
        .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("XDG_DATA_HOME", cwd.join("xdg data"))
        .env("XDG_CONFIG_HOME", cwd.join("xdg config"))
        .env("SUNRISE_NO_AUDIO", "1")
        .env_remove("SUNRISE_SESSION_FILE")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn last_stderr_line(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .last()
        .unwrap_or_default()
        .to_string()
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "daily"]
fn launch_help_works_with_an_eight_mib_hard_stack_limit() {
    let dir = scratch("low-stack-help");
    let output = run_command(
        &dir,
        Command::new("sh").args([
            "-c",
            r#"ulimit -S -s 8192 && ulimit -H -s 8192 && exec "$@""#,
            "sh",
            env!("CARGO_BIN_EXE_sunrise-client"),
            "--help",
        ]),
    );
    assert!(
        output.status.success(),
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--resources"), "{help}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn documented_switches_do_not_consume_the_following_option() {
    let dir = scratch("switches");
    let output = run(&dir, &["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    let switches: Vec<_> = help
        .lines()
        .filter_map(|line| {
            let (usage, _) = line.trim_start().split_once("  ")?;
            (usage.starts_with("--") && !usage.contains(char::is_whitespace)).then_some(usage)
        })
        .collect();
    assert!(!switches.is_empty(), "help must describe startup switches");
    assert!(
        switches.contains(&"--game-tests"),
        "the actual app must advertise game-test mode"
    );
    // Separate processes also isolate --pregen's global command toggle.
    for switch in switches {
        let output = run(&dir, &[switch, "--help"]);
        assert!(
            output.status.success(),
            "{switch}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if switch == "--launch-api" {
            let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(reply["launch_api"].is_string());
        } else {
            assert_eq!(String::from_utf8(output.stdout).unwrap(), help, "{switch}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn launch_api_prints_json_and_runs_nothing_else() {
    let dir = scratch("reply");
    let output = run(&dir, &["--launch-api"]);
    assert_eq!(output.status.code(), Some(0));
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(reply["launch_api"].is_string(), "{reply}");
    assert_eq!(reply["minecraft"], sunrise_core::VANILLA_VERSION);
    let names: Vec<_> = reply["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["name"].as_str().unwrap())
        .collect();
    for name in ["--resources", "--game-dir", "--home", "--session-stdin"] {
        assert!(names.contains(&name), "{name} not in {names:?}");
    }
    assert!(!dir.join("env home").exists(), "--launch-api opened a log");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_refused_command_line_exits_2_without_a_crash_report() {
    let dir = scratch("usage");
    for args in [
        &["--home", "home", "--bogus"][..],
        &["--home", "home", "--width", "wide"],
        &["--home", "home", "--game-dir"],
    ] {
        let output = run(&dir, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(
            !last_stderr_line(&output).starts_with("Sunrise: crash report saved to "),
            "{args:?}"
        );
        assert!(!dir.join("home/crash-reports").exists(), "{args:?}");
    }
    assert!(!dir.join("saves").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A control socket no system would bind: refused before the game folder, the start marker in its
/// options.txt or the resources are touched, so the next start doesn't read it as a failed one.
#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_control_socket_path_too_long_is_a_refused_command_line() {
    let dir = scratch("socket");
    let socket = dir.join(format!("{}.sock", "x".repeat(150)));
    let output = run(
        &dir,
        &[
            "--home",
            "home",
            "--game-dir",
            "game",
            "--offscreen",
            "--control-socket",
            socket.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let line = last_stderr_line(&output);
    assert!(line.contains("too long"), "last stderr line: {line}");
    assert!(!dir.join("home/crash-reports").exists());
    assert!(!dir.join("game").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "daily"]
fn an_error_exits_1_and_names_the_crash_report_under_home() {
    let dir = scratch("error");
    let start = dir.join("start");
    std::fs::create_dir_all(&start).unwrap();
    // No prepared resources: the game stops on an error once its folders are set up.
    let output = run(
        &start,
        &[
            "--home",
            "../my home",
            "--game-dir",
            "../my game",
            "--resources",
            "../no resources",
            "--offscreen",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let line = last_stderr_line(&output);
    let report = line
        .strip_prefix("Sunrise: crash report saved to ")
        .unwrap_or_else(|| panic!("last stderr line: {line}"));
    let report = Path::new(report);
    assert!(report.is_file(), "{}", report.display());
    assert_eq!(
        std::fs::canonicalize(report.parent().unwrap()).unwrap(),
        std::fs::canonicalize(dir.join("my home/crash-reports")).unwrap()
    );
    assert!(dir.join("my game").is_dir(), "--game-dir was not created");
    assert!(!start.join("saves").exists() && !start.join("options.txt").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
