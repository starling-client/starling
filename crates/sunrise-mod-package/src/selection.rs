//! The package candidates the game loads, before content validation and conflicts.
mod context;
pub use context::{Argument, LaunchArguments, MissingValue, ModSelectionContext, data_dir_with};

use crate::{
    Package, find,
    mod_list::{DIR, NATIVE_DIR},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// A selected package, read and validated once at the filesystem boundary.
pub struct SelectedPackage {
    pub path: PathBuf,
    pub package: Package,
}

/// Selected content-only packages, including explicit development paths.
/// Native packages are not content candidates, even through a development path:
/// their canonical source and compiled identity are owned by the engine.
pub fn content_packages<'a>(
    game_dir: &Path,
    enabled: &'a BTreeSet<String>,
    dev: &'a [PathBuf],
) -> impl Iterator<Item = SelectedPackage> + 'a {
    content_packages_read(game_dir, enabled, dev, |path| Package::read(path).ok())
}

/// [`content_packages`] from packages the caller has read: `read` answers once for each candidate
/// path, with its valid package.
pub fn content_packages_read<'a>(
    game_dir: &Path,
    enabled: &'a BTreeSet<String>,
    dev: &'a [PathBuf],
    mut read: impl FnMut(&Path) -> Option<Package> + 'a,
) -> impl Iterator<Item = SelectedPackage> + 'a {
    let mut paths = find(&game_dir.join(DIR));
    paths.extend(find(&game_dir.join(NATIVE_DIR)));
    paths.retain(|path| !dev.contains(path));
    paths.extend(dev.iter().cloned());
    paths.into_iter().filter_map(move |path| {
        let package = read(&path)?;
        let selected =
            !package.is_native() && (enabled.contains(&package.manifest.id) || dev.contains(&path));
        selected.then_some(SelectedPackage { path, package })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let home = std::env::temp_dir()
                .join(format!("package-selection-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&home).unwrap();
            Self(home)
        }
        fn package(&self, folder: &str, id: &str, code: bool) -> PathBuf {
            let mut manifest = format!(
                "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n"
            );
            let source: &[(&str, &[u8])] = if code {
                manifest += &format!(
                    "[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                    "a".repeat(40),
                    "b".repeat(40)
                );
                &[
                    ("Cargo.toml", b"[package]\nname='example'\nversion='1'\n"),
                    ("src/lib.rs", b"pub fn hooks() {}"),
                ]
            } else {
                &[]
            };
            let directory = self.0.join(folder);
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join(format!("{id}.smod"));
            fs::write(&path, Package::write_files(&manifest, source).unwrap()).unwrap();
            path
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn content_toggles_apply_in_both_folders() {
        let home = Scratch::new();
        let first = home.package("sunrise-mods", "first", false);
        let second = home.package("sunrise-native-mods", "second", false);
        home.package("sunrise-mods", "disabled", false);
        home.package("sunrise-native-mods", "also_disabled", false);
        let enabled = BTreeSet::from(["first".into(), "second".into()]);
        let selected: Vec<_> = content_packages(&home.0, &enabled, &[])
            .map(|p| p.path)
            .collect();
        assert_eq!(selected, [first, second]);
    }

    #[test]
    fn content_candidates_do_not_load_native_inputs_even_when_enabled_or_developed() {
        let home = Scratch::new();
        let native = home.package("sunrise-mods", "native", true);
        home.package("sunrise-native-mods", "other_native", true);
        let plain = home.package("sunrise-native-mods", "plain", false);
        let external = home.package("workspace", "external", false);
        let enabled = BTreeSet::from(["native".into(), "other_native".into(), "plain".into()]);
        let selected: Vec<_> = content_packages(&home.0, &enabled, &[native, external.clone()])
            .map(|p| p.path)
            .collect();
        assert_eq!(selected, [plain, external]);
    }

    #[test]
    fn development_paths_are_appended_once_and_invalid_packages_are_skipped() {
        let home = Scratch::new();
        let discovered = home.package("sunrise-mods", "discovered", false);
        let external = home.package("workspace", "external", false);
        let invalid = home.0.join("sunrise-mods/invalid.smod");
        fs::write(&invalid, b"not a package").unwrap();
        let enabled = BTreeSet::from(["discovered".into(), "invalid".into()]);
        let dev = [discovered.clone(), external.clone(), invalid];
        let selected: Vec<_> = content_packages(&home.0, &enabled, &dev)
            .map(|p| p.path)
            .collect();
        assert_eq!(selected, [discovered, external]);
    }
}
