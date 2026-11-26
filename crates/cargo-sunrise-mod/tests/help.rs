//! A newcomer's first command: no command, `--help` and `-h` print the usage and succeed; an unknown command fails.
use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
#[ignore = "daily"]
fn help_lists_the_everyday_commands_and_each_command_says_its_usage() {
    for args in [&[][..], &["--help"], &["-h"], &["sunrise-mod", "--help"]] {
        let output = run(args);
        assert!(output.status.success(), "{args:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.lines().count() < 20, "{args:?}: {text}");
        for command in ["new", "build", "run", "check", "pin", "hooks", "release"] {
            assert!(
                text.lines().any(|l| l.trim_start().starts_with(command)),
                "{args:?}: {command}"
            );
        }
        assert!(text.contains("cargo sunrise-mod help all"), "{args:?}");
        assert!(!text.contains("self-update"), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
    }
    let all = run(&["help", "all"]);
    assert!(all.status.success());
    assert!(
        String::from_utf8_lossy(&all.stdout)
            .lines()
            .any(|line| line == "cargo sunrise-mod self-update")
    );
    let output = run(&["build", "--help"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with("usage: cargo sunrise-mod build [--update-lock]"),
        "{text}"
    );
    let output = run(&["help", "all"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("cargo sunrise-mod sdk diff"), "{text}");
    assert!(!text.contains("cargo sunrise-mod native-"), "{text}");
    let output = run(&["native-build", "--help"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with("usage: cargo sunrise-mod native-build < plan.json"),
        "{text}"
    );
}

#[test]
#[ignore = "daily"]
fn an_unknown_command_is_an_error() {
    // Removed and renamed verbs get the ordinary refusal, before any work, with or without
    // `--help`: the message, not just a failed exit, says the command is unknown.
    for name in ["frobnicate", "update", "fits", "publish", "registry"] {
        for args in [&[name][..], &[name, "--help"][..]] {
            let output = run(args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert!(output.stdout.is_empty(), "{args:?}");
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains(&format!("{name:?} is not a cargo sunrise-mod command")),
                "{args:?}: {error}"
            );
        }
    }
    let output = run(&["release", "frobnicate", "--help"]);
    assert!(!output.status.success());
}
