//! Canonical native package files shared by data preparation and engine validation.
use crate::{Manifest, Package, mod_list::NATIVE_DIR};
use std::{
    fmt,
    path::{Path, PathBuf},
};

/// A schema-validated native package, not evidence that its code is compiled into a program.
#[derive(Debug)]
pub struct AdjacentManifest {
    pub path: PathBuf,
    pub manifest: Manifest,
    /// Whether the package carries an `icon.png`.
    pub has_icon: bool,
    /// The package's runtime files, each checked against its `[runtime]` hash as the package was
    /// opened; `None` for a package with no `[runtime]`.
    pub runtime: Option<crate::manifest::RuntimeData>,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ReadError {
    Package { path: PathBuf, reason: String },
    NotNative { path: PathBuf, id: String },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package { path, reason } => write!(f, "{}: {reason}", path.display()),
            Self::NotNative { path, id } => write!(f, "{}: {id} is not a code mod", path.display()),
        }
    }
}
impl std::error::Error for ReadError {}

/// Read data candidates beneath `executable`'s adjacent `sunrise-native-mods/` directory.
/// This does not read the build record or establish compiled membership or identity. An empty
/// result is not proof of a vanilla program: the engine must validate its own embedded inventory.
pub fn adjacent_manifests(executable: &Path) -> Result<Vec<AdjacentManifest>, ReadError> {
    crate::find(&executable.with_file_name(NATIVE_DIR))
        .into_iter()
        .filter(|path| path.is_dir())
        .map(|path| read_native(&path))
        .collect()
}

/// The same package boundary for a known canonical path. The caller owns compiled-identity checks.
pub fn read_native(path: &Path) -> Result<AdjacentManifest, ReadError> {
    let package = Package::read(path).map_err(|reason| ReadError::Package {
        path: path.to_owned(),
        reason,
    })?;
    if !package.is_native() {
        return Err(ReadError::NotNative {
            path: path.to_owned(),
            id: package.manifest.id,
        });
    }
    // The one place a native package's files are hashed: a hand-edited folder is refused here.
    let runtime = package
        .read_runtime(path)
        .map_err(|reason| ReadError::Package {
            path: path.to_owned(),
            reason,
        })?;
    Ok(AdjacentManifest {
        path: path.to_owned(),
        manifest: package.manifest,
        has_icon: package.icon.is_some(),
        runtime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let name = format!(
                "adjacent-packages-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            );
            let dir = std::env::temp_dir().join(name);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn package(&self, id: &str) -> PathBuf {
            let dir = self.0.join(NATIVE_DIR).join(id);
            fs::create_dir_all(dir.join("src")).unwrap();
            fs::write(dir.join("sunrise-mod.toml"), format!("id='{id}'\nname='{id}'\nversion='1.2.3'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n", "a".repeat(40), "b".repeat(40))).unwrap();
            fs::write(
                dir.join("Cargo.toml"),
                "[package]\nname='different_crate'\nversion='9.9.9'\n",
            )
            .unwrap();
            fs::write(dir.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
            dir
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn candidates_follow_the_executable_not_the_record_or_cargo_identity() {
        let home = Scratch::new();
        let program = home.0.join("sunrise-engine");
        assert!(adjacent_manifests(&program).unwrap().is_empty());
        let first = home.package("first");
        home.package("not_in_the_record");
        fs::write(
            home.0.join("native-mods.json"),
            r#"{"mods":[{"id":"first","version":"wrong"}]}"#,
        )
        .unwrap();
        let candidates = adjacent_manifests(&program).unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|p| (p.manifest.id.as_str(), p.manifest.version.to_string()))
                .collect::<Vec<_>>(),
            [
                ("first", "1.2.3".to_owned()),
                ("not_in_the_record", "1.2.3".to_owned())
            ]
        );
        assert_eq!(candidates[0].path, first);
        fs::write(home.0.join("native-mods.json"), "not JSON").unwrap();
        assert_eq!(adjacent_manifests(&program).unwrap().len(), 2);
    }

    #[test]
    fn a_native_package_opens_with_its_runtime_files_checked_once() {
        let home = Scratch::new();
        let dir = home.package("baked");
        let hash = crate::manifest::sha512_of(b"tables");
        let manifest = fs::read_to_string(dir.join("sunrise-mod.toml")).unwrap();
        fs::write(
            dir.join("sunrise-mod.toml"),
            format!("{manifest}[runtime]\n\"nav.bin\"='{hash}'\n"),
        )
        .unwrap();
        fs::create_dir_all(dir.join("runtime")).unwrap();
        fs::write(dir.join("runtime/nav.bin"), b"tables").unwrap();
        let package = read_native(&dir).unwrap();
        let runtime = package.runtime.unwrap();
        assert_eq!(runtime.file("nav.bin").unwrap().0, b"tables");
        // A folder edited by hand is refused at open, naming the file.
        fs::write(dir.join("runtime/nav.bin"), b"edited").unwrap();
        let error = read_native(&dir).unwrap_err().to_string();
        assert!(error.contains("runtime/nav.bin"), "{error}");
    }

    #[test]
    fn corrupt_and_content_only_packages_report_their_path() {
        let home = Scratch::new();
        let dir = home.package("broken");
        fs::write(dir.join("sunrise-mod.toml"), "not TOML").unwrap();
        assert!(
            matches!(adjacent_manifests(&home.0.join("engine")), Err(ReadError::Package { path, .. }) if path == dir)
        );
        fs::write(
            dir.join("sunrise-mod.toml"),
            "id='broken'\nname='Broken'\nversion='1.2.3'\napi='0.2'\ncapabilities=['registries']\n",
        )
        .unwrap();
        fs::remove_file(dir.join("Cargo.toml")).unwrap();
        fs::remove_dir_all(dir.join("src")).unwrap();
        assert!(
            matches!(read_native(&dir), Err(ReadError::NotNative { path, id }) if path == dir && id == "broken")
        );
    }
}
