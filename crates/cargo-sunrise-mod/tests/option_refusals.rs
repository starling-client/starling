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
    assert!(error.contains(option), "{error}");
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
