//! The install scripts' tamper tests (tests/install_sh_signature.sh, install_ps1_signature.ps1): each runs the
//! real script against a fake release signed with a throwaway key, with a `sunrise-verify` built with its `test-key`
//! feature, so it trusts that key (tests/common/mod.rs); a release build never has it.
use std::path::{Path, PathBuf};
use std::process::Command;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(program: &str, args: &[&str], script: &str) {
    let out = Command::new(program)
        .args(args)
        .arg(repository().join("tests").join(script))
        .env(
            "SUNRISE_VERIFY_TEST_BIN",
            crate::common::test_key_verifier(),
        )
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{script}:\n{text}");
}

#[test]
#[ignore = "daily"]
fn install_sh_installs_only_what_the_verifier_checked() {
    // install.sh is for Linux and macOS; a bare `bash` on Windows is System32's WSL one.
    if cfg!(windows) {
        return;
    }
    run("bash", &[], "install_sh_signature.sh");
}
