//! Validated flat packages and launcher-owned horizon-mixins input.
use super::{Bundle, MANIFEST};
use std::{fs, path::Path};
use sunrise_mod_package::mod_surface::uses::USES_FILE;
use sunrise_mod_package::{Package, manifest};

/// Return the validated native project, if this directory has a package manifest.
/// A present but invalid manifest is reported by the canonical package reader.
pub fn read(dir: &Path) -> Result<Option<Bundle>, String> {
    read_with(dir, crate::host::tr)
}

fn read_with(dir: &Path, tr: for<'a> fn(&'a str) -> &'a str) -> Result<Option<Bundle>, String> {
    let path = dir.join(manifest::MANIFEST);
    match fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    let package = read_package(dir, tr)?;
    if !package.is_native() {
        return Ok(None);
    }
    package.directory_files(dir)?;
    Ok(Some(Bundle {
        manifest: package.manifest,
        dir: dir.to_owned(),
    }))
}

fn read_package(dir: &Path, tr: for<'a> fn(&'a str) -> &'a str) -> Result<Package, String> {
    Package::read_detailed(dir).map_err(|error| match error {
        manifest::PackageReadError::NativeStateLayout { mod_id, row } => crate::host::trf_with(
            tr,
            "E_MIXIN_STATE_LAYOUT: {mod}:{state}: native state changes the prebuilt game's layout; use an attachment instead",
            &[("mod", &mod_id), ("state", &row)],
        ),
        other => other.to_string(),
    })
}

/// Generate tool input in the build's incoming copy, never in a modder's project.
pub fn tool_manifest(bundle: &Bundle) -> Result<(), String> {
    let mut manifest = toml::Table::new();
    manifest.insert(
        "mod".into(),
        toml::Value::Table(toml::Table::from_iter([
            ("id".into(), bundle.manifest.id.clone().into()),
            ("version".into(), bundle.manifest.version.to_string().into()),
        ])),
    );
    manifest.insert(
        "compatibility".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "horizon_revision".into(),
            bundle.native().sunrise.clone().into(),
        )])),
    );
    for (name, rows) in [
        ("injection", toml::Value::try_from(&bundle.native().mixin)),
        ("bridge", toml::Value::try_from(&bundle.native().bridge)),
    ] {
        let rows = rows.map_err(|error| error.to_string())?;
        if rows.as_array().is_some_and(|rows| !rows.is_empty()) {
            manifest.insert(name.into(), rows);
        }
    }
    if !bundle.native().module.is_empty() {
        manifest.insert(
            "module".into(),
            toml::Value::try_from(&bundle.native().module).map_err(|e| e.to_string())?,
        );
    }
    fs::write(
        bundle.dir.join(MANIFEST),
        toml::to_string(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn checked_files(bundle: &Bundle) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
    read_package(&bundle.dir, crate::host::tr)?.directory_files(&bundle.dir)
}

pub fn files(bundle: &Bundle) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    Ok(checked_files(bundle)?
        .into_keys()
        .map(|name| {
            let path = bundle.dir.join(&name);
            (name, path)
        })
        .collect())
}

/// The package's files and its uses table, which is no source file: a copy is judged against
/// another game build as the package was.
pub fn copy(bundle: &Bundle, to: &Path) -> Result<(), String> {
    let mut files = checked_files(bundle)?;
    if let Ok(table) = fs::read(bundle.dir.join(USES_FILE)) {
        files.insert(USES_FILE.to_owned(), table);
    }
    copy_files(to, files)
}

/// Only code enters the tool; runtime resource packs stay with the package. A root
/// `rust-toolchain` or `rust-toolchain.toml` is left out: the build runs the toolchain it pins
/// itself, and a file there would let the mod name another.
pub fn copy_tool_input(bundle: &Bundle, to: &Path) -> Result<(), String> {
    copy_files(
        to,
        checked_files(bundle)?.into_iter().filter(|(name, _)| {
            !matches!(name.as_str(), "rust-toolchain" | "rust-toolchain.toml")
                && (manifest::source_file(name)
                    || bundle.native().mixin.iter().any(|row| row.helper == *name))
        }),
    )
}

fn copy_files(to: &Path, files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Result<(), String> {
    for (name, bytes) in files {
        let path = to.join(name);
        fs::create_dir_all(path.parent().ok_or("invalid destination")?)
            .map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::write_bundle;

    #[test]
    fn scan_skips_non_packages_and_reads_native_projects_with_unrelated_folders() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        fs::create_dir_all(workspace.join("rust")).unwrap();
        fs::write(workspace.join("horizon-mixin.toml"), "not a package").unwrap();
        let project = workspace.join("example");
        write_bundle(&project, "example", "");
        fs::create_dir(project.join("native")).unwrap();
        fs::write(project.join("sunrise-native.json"), "not a manifest").unwrap();

        let bundles = crate::scan_bundles(&workspace).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].manifest.id, "example");
        assert_eq!(bundles[0].dir, project);
    }

    #[test]
    fn a_present_malformed_manifest_is_not_skipped() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(manifest::MANIFEST)).unwrap();
        let error = read(root.path()).unwrap_err();
        assert_eq!(
            error,
            "sunrise-mod.toml: unsupported file type in a content mod"
        );
    }

    #[test]
    fn native_state_refusal_does_not_expand_holes_in_ids() {
        let dir = tempfile::tempdir().unwrap();
        write_bundle(dir.path(), "stateful{state}", "");
        let path = dir.path().join(manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(
            path,
            format!("{text}[[native.state]]\nid='autosave{{mod}}'\n"),
        )
        .unwrap();

        assert_eq!(
            read_with(dir.path(), |en| en).unwrap_err(),
            "E_MIXIN_STATE_LAYOUT: stateful{state}:autosave{mod}: native state changes the prebuilt game's layout; use an attachment instead"
        );
    }

    #[test]
    fn native_state_is_refused_by_the_package_reader_before_staging() {
        fn localized(en: &str) -> &str {
            match en {
                "E_MIXIN_STATE_LAYOUT: {mod}:{state}: native state changes the prebuilt game's layout; use an attachment instead" => {
                    "E_MIXIN_STATE_LAYOUT: localized {mod}/{state}"
                }
                other => other,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        write_bundle(dir.path(), "stateful", "");
        let path = dir.path().join(manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap();
        for (row, table) in [
            (
                "autosave",
                "[[native.state]]\nid='autosave'\ntarget='game::Game'\nty='u32'\ninitial='0'\nconstructor=[]\n",
            ),
            ("native.state", "state = []\n"),
        ] {
            fs::write(&path, format!("{text}{table}")).unwrap();
            let error = format!(
                "E_MIXIN_STATE_LAYOUT: stateful:{row}: native state changes the prebuilt game's layout; use an attachment instead"
            );
            assert_eq!(Package::read(dir.path()).err().unwrap(), error);
            assert_eq!(read(dir.path()).unwrap_err(), error);
            assert_eq!(
                read_with(dir.path(), localized).unwrap_err(),
                format!("E_MIXIN_STATE_LAYOUT: localized stateful/{row}")
            );
        }
    }

    #[test]
    fn the_staged_tool_input_leaves_out_a_root_toolchain_file() {
        let dir = tempfile::tempdir().unwrap();
        write_bundle(dir.path(), "example", "");
        for name in [
            "rust-toolchain",
            "rust-toolchain.toml",
            "vendor/rust-toolchain",
        ] {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "[toolchain]\nchannel = 'nightly'\n").unwrap();
        }
        let bundle = read(dir.path()).unwrap().unwrap();
        let staged = tempfile::tempdir().unwrap();
        copy_tool_input(&bundle, staged.path()).unwrap();
        assert!(staged.path().join("src/lib.rs").is_file());
        assert!(staged.path().join("vendor/rust-toolchain").is_file());
        assert!(!staged.path().join("rust-toolchain").exists());
        assert!(!staged.path().join("rust-toolchain.toml").exists());
    }

    #[test]
    fn an_extracted_package_with_root_page_entries_stages_without_them() {
        // An `.smod` packed before #5974 extracts with the registry page beside the source.
        let dir = tempfile::tempdir().unwrap();
        write_bundle(dir.path(), "example", "");
        for name in ["page.toml", "page/runes.jpg", "src/page.rs"] {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        let bundle = read(dir.path()).unwrap().unwrap();
        let staged = tempfile::tempdir().unwrap();
        copy_tool_input(&bundle, staged.path()).unwrap();
        assert!(staged.path().join("src/lib.rs").is_file());
        assert!(staged.path().join("src/page.rs").is_file());
        assert!(!staged.path().join("page.toml").exists());
        assert!(!staged.path().join("page").exists());
    }
}
