#![cfg(windows)]

use std::{fs, process::Command};

#[test]
#[ignore = "daily"]
fn windows_keygen_refuses_an_unencrypted_private_key_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    fs::write(
        project.join("sunrise-mod.toml"),
        "id = 'signing_test'\nname = 'Signing test'\nversion = '1.0.0'\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"))
        .current_dir(project)
        .env("LOCALAPPDATA", project.join("data"))
        .args(["keygen", "--out", ".sunrise", "--no-password"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--no-password"));
    assert!(output.stdout.is_empty());
    assert!(!project.join(".sunrise").exists());
}
