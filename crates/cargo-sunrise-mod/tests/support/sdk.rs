//! Minimal signed SDKs for integration tests of the debug tool. No downloads or build slices.
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use sunrise_mod_package::sdk::{self, Index};
use sunrise_release_sign::{self as sign, SigningKey};

pub struct OfflineSdk {
    base: PathBuf,
    key: SigningKey,
}

impl OfflineSdk {
    pub fn new(project: &Path) -> Self {
        Self {
            base: project.join("releases"),
            key: SigningKey::generate().unwrap().0,
        }
    }

    pub fn release(&self, commit: &str, mixins: &str) {
        let tag = format!("sdk-{commit}");
        let folder = self.base.join(&tag);
        fs::create_dir_all(&folder).unwrap();
        let surface = format!(r#"{{"schema":{},"hash":"ab"}}"#, mod_surface::SCHEMA);
        fs::write(folder.join("surface.json"), &surface).unwrap();
        // Update also fetches the project's API archive. These tests have no Rust sources to scan.
        fs::write(
            folder.join("api-source.tar.xz"),
            include_bytes!("../../../sunrise-mod-package/src/sdk/fixture.tar.xz"),
        )
        .unwrap();
        let build = sdk::Build {
            commit: commit.into(),
            game: sdk::GAME.into(),
            date: "2026-11-13".into(),
            nightly: None,
        };
        let mut index = Index::of_folder(&folder, build, "1.99.1".into()).unwrap();
        index.mixins.rev = mixins.into();
        let index = index.to_json();
        fs::write(folder.join(sdk::INDEX), &index).unwrap();
        let sums = sign::sums_file(&[(sdk::INDEX.into(), format!("{:x}", Sha256::digest(&index)))]);
        fs::write(folder.join(sign::SUMS), &sums).unwrap();
        fs::write(
            folder.join(sign::SIGNATURE),
            self.key.sign(&tag, sums.as_bytes()),
        )
        .unwrap();
    }

    pub fn command(&self, project: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-sunrise-mod"));
        command
            .arg("sunrise-mod")
            .current_dir(project)
            .env("SUNRISE_SDK_BASE", &self.base)
            .env(sign::SDK_KEY_VAR, self.key.public_base64());
        command
    }

    /// Let the tool verify and stamp both SDKs before the test seeds their unpacked build data.
    pub fn prefetch(&self, project: &Path, old: &str, new: &str) {
        let output = self
            .command(project)
            .args(["sdk", "diff", old, new])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
