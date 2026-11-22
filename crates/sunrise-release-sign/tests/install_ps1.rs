#![cfg(windows)]
//! The Windows leg of the install scripts' tamper test: tests/install_ps1_signature.ps1 runs the real install.ps1
//! against fake releases signed with a throwaway key, with a `sunrise-verify.exe` built with its
//! `test-key` feature, so it trusts that key (tests/common/mod.rs); a release build never has it.
use std::path::Path;

#[path = "common/mod.rs"]
mod common;
use std::process::Command;

#[test]
#[ignore = "daily"]
fn install_ps1_installs_only_what_the_verifier_checked() {
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/install_ps1_signature.ps1");
    let out = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .env("SUNRISE_VERIFY_TEST_BIN", common::test_key_verifier())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Exit 77 is the script's "skip": the runner must have OpenSSL 3 (Git for Windows ships it), so it fails here.
    assert!(out.status.success(), "{text}");
}
