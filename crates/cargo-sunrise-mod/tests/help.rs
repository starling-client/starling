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
        for command in ["new", "build", "run", "check", "update", "hooks"] {
            assert!(
                text.lines().any(|l| l.trim_start().starts_with(command)),
                "{args:?}: {command}"
            );
        }
        assert!(text.contains("cargo sunrise-mod help all"), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
    }
    let output = run(&["build", "--help"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with("usage: cargo sunrise-mod build [--update-lock]"),
        "{text}"
    );
    for (subcommand, usage) in [("plan", "--releases"), ("scripts", "--out")] {
        for flag in ["--help", "-h"] {
            let output = run(&["release", subcommand, flag]);
            assert!(output.status.success(), "{subcommand} {flag}");
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(
                text.starts_with(&format!(
                    "usage: cargo sunrise-mod release {subcommand} {usage}"
                )),
                "{text}"
            );
            assert!(output.stderr.is_empty());
        }
    }
}

#[test]
#[ignore = "daily"]
fn an_unknown_command_is_an_error() {
    let output = run(&["frobnicate"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a cargo sunrise-mod command"));
    let output = run(&["release", "frobnicate", "--help"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown release command \"frobnicate\"")
    );
}
