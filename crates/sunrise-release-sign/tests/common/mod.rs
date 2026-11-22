//! The `sunrise-verify` the install tests run: built here with the `test-key` feature, in a folder of its own, so it
//! trusts the throwaway key of `SUNRISE_VERIFY_TEST_KEY`. The binary cargo builds for the integration tests
//! (`CARGO_BIN_EXE_sunrise-verify`) has the feature only when the whole test run is given `--features test-key`.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub fn test_key_verifier() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("verify-test-key");
        let status = Command::new(env!("CARGO"))
            .args(["build", "--quiet", "--locked", "--bin", "sunrise-verify"])
            .args(["--features", "test-key"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .arg("--target-dir")
            .arg(&target)
            .status()
            .unwrap();
        assert!(status.success(), "building sunrise-verify with test-key");
        target
            .join("debug")
            .join(format!("sunrise-verify{}", std::env::consts::EXE_SUFFIX))
    })
}
