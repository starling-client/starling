//! An option a mod of the wrong kind would drop is refused by name, before any SDK or package work.
use std::{path::Path, process::Output};

fn tool(dir: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .current_dir(dir)
        .args(args)
        // Nothing may be fetched: a base that does not exist fails any fetch that is tried.
        .env("SUNRISE_SDK_BASE", dir.join("no-sdk"))
        .output()
        .unwrap()
}

/// A failure that names `option` and left no SDK, cache or package behind.
fn refused_before_any_work(dir: &Path, output: &Output, option: &str) {
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(error.lines().next().unwrap().contains(option), "{error}");
    assert!(!dir.join(".sunrise").exists());
    assert!(!dir.join("target").exists());
}

#[test]
#[ignore = "daily"]
fn update_lock_is_refused_for_a_content_mod() {
    let root = tempfile::tempdir().unwrap();
    assert!(tool(root.path(), &["new", "ruby"]).status.success());
    let content = root.path().join("ruby");
    let output = tool(&content, &["build", "--update-lock"]);
    refused_before_any_work(&content, &output, "--update-lock");
}

#[test]
#[ignore = "daily"]
fn pin_in_a_content_mod_is_refused_before_any_build_is_resolved() {
    let root = tempfile::tempdir().unwrap();
    assert!(tool(root.path(), &["new", "ruby"]).status.success());
    let content = root.path().join("ruby");
    // A base with no SDK: resolving the Nightly would fail there, so only the project's own
    // manifest can answer, and it says a content mod pins no build.
    let output = tool(&content, &["pin"]);
    assert_eq!(output.status.code(), Some(1));
    refused_before_any_work(&content, &output, "pins no game build");
}

#[test]
#[ignore = "daily"]
fn check_fit_refuses_project_options_before_fetching_an_sdk() {
    let root = tempfile::tempdir().unwrap();
    for args in [&["--uses"][..], &["--with", "library"][..]] {
        let mut command = vec!["check", "--build", "nightly"];
        command.extend_from_slice(args);
        let output = tool(root.path(), &command);
        assert_eq!(output.status.code(), Some(2));
        refused_before_any_work(root.path(), &output, args[0]);
    }
}
