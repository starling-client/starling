//! Validated flat packages and launcher-owned horizon-mixins input.
use super::{Bundle, MANIFEST};
use std::{fs, path::Path};
use sunrise_mod_package::mod_surface::uses::USES_FILE;
use sunrise_mod_package::{Package, manifest};

pub fn read(dir: &Path) -> Result<Option<Bundle>, String> {
    for old in [
        "sunrise-native.json",
        "horizon-mixin.toml",
        "native",
        "rust",
    ] {
        if dir.join(old).exists() {
            return Err(format!("{}: unsupported package path {old}", dir.display()));
        }
    }
    if !dir.join(manifest::MANIFEST).is_file() {
        return Ok(None);
    }
    let package = Package::read(dir)?;
    if !package.is_native() {
        return Ok(None);
    }
    if let Some(state) = package
        .manifest
        .native
        .as_ref()
        .and_then(|native| native.state.first())
    {
        return Err(crate::host::trf(
            "E_MIXIN_STATE_LAYOUT: {mod}:{state}: native state changes the prebuilt game's layout; use an attachment instead",
            &[("mod", &package.manifest.id), ("state", &state.id)],
        ));
    }
    package.directory_files(dir)?;
    Ok(Some(Bundle {
        manifest: package.manifest,
        dir: dir.to_owned(),
    }))
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
        ("state", toml::Value::try_from(&bundle.native().state)),
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
    Package::read(&bundle.dir)?.directory_files(&bundle.dir)
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

/// Only code enters the tool; runtime resource packs stay with the package.
pub fn copy_tool_input(bundle: &Bundle, to: &Path) -> Result<(), String> {
    copy_files(
        to,
        checked_files(bundle)?.into_iter().filter(|(name, _)| {
            manifest::source_file(name)
                || bundle.native().mixin.iter().any(|row| row.helper == *name)
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
mod state_tests {
    #[test]
    fn native_state_is_refused_before_staging() {
        let dir = super::super::tests::dir("state-refusal");
        super::super::tests::write_bundle(&dir, "stateful", "");
        let path = dir.join(sunrise_mod_package::manifest::MANIFEST);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\n[[native.state]]\nid='autosave'\ntarget='game::Game'\nty='u32'\ninitial='0'\nconstructor=[]\n");
        std::fs::write(path, text).unwrap();
        let error = super::read(&dir).err().unwrap();
        assert!(error.contains("E_MIXIN_STATE_LAYOUT"), "{error}");
        assert!(error.contains("attachment"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
