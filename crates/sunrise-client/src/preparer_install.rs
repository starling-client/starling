//! Commit prepared resources without deleting the last installation before the rename succeeds.
use std::{fs, path::Path};

/// APK updates may remove/rename original Java sources. Do not leave those in
/// the bundle key, but retain the preparer's independent `.local` work cache.
pub(super) fn reset_kit_sources(dir: &Path) -> Result<(), String> {
    for name in ["mods", "crates", "exporters"] {
        let path = dir.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let result = if metadata.is_dir() && !metadata.is_symlink() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                result.map_err(|e| format!("Cannot refresh preparer sources: {e}"))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

pub(super) fn activate(made: &Path, destination: &Path) -> Result<(), String> {
    if made == destination {
        return Ok(());
    }
    let parent = destination
        .parent()
        .ok_or("Resource folder has no parent")?;
    let name = destination
        .file_name()
        .ok_or("Resource folder has no name")?
        .to_string_lossy();
    let backup = parent.join(format!(".{name}.starling-previous"));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // Recover a stop between moving the old folder and publishing the new one.
    if backup.exists() && !destination.exists() {
        fs::rename(&backup, destination)
            .map_err(|e| format!("Cannot restore previous resources: {e}"))?;
    }
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .map_err(|e| format!("Cannot retire previous resource backup: {e}"))?;
    }
    let previous = destination.exists();
    if previous {
        fs::rename(destination, &backup)
            .map_err(|e| format!("Cannot preserve previous resources: {e}"))?;
    }
    if let Err(error) = fs::rename(made, destination) {
        if previous && let Err(restore) = fs::rename(&backup, destination) {
            return Err(format!(
                "Cannot activate resources: {error}; previous resources remain at {} (restore failed: {restore})",
                backup.display()
            ));
        }
        return Err(format!(
            "Cannot activate resources: {error}; previous resources preserved"
        ));
    }
    if previous && let Err(error) = fs::remove_dir_all(&backup) {
        log::warn!("Resources installed; old resource backup could not be removed: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "starling-resource-swap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn apk_source_refresh_removes_old_inputs_but_keeps_work_cache() {
        let dir = root();
        for name in ["mods", "crates", "exporters", ".local"] {
            fs::create_dir(dir.join(name)).unwrap();
            fs::write(dir.join(name).join("old"), name).unwrap();
        }
        reset_kit_sources(&dir).unwrap();
        assert_eq!(fs::read(dir.join(".local/old")).unwrap(), b".local");
        for name in ["mods", "crates", "exporters"] {
            assert!(!dir.join(name).exists());
        }
        reset_kit_sources(&dir).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_publish_restores_old_resources_and_a_retry_replaces_them() {
        let dir = root();
        let target = dir.join("resources");
        let made = dir.join("prepared");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("kept"), "old resources").unwrap();
        assert!(activate(&made, &target).is_err());
        assert_eq!(
            fs::read_to_string(target.join("kept")).unwrap(),
            "old resources"
        );
        fs::create_dir(&made).unwrap();
        fs::write(made.join("ready"), "new resources").unwrap();
        activate(&made, &target).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("ready")).unwrap(),
            "new resources"
        );
        assert!(!made.exists());
        assert!(!dir.join(".resources.starling-previous").exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn interrupted_swap_restores_before_a_failed_retry() {
        let dir = root();
        let target = dir.join("resources");
        let backup = dir.join(".resources.starling-previous");
        fs::create_dir(&backup).unwrap();
        fs::write(backup.join("kept"), "last working resources").unwrap();
        assert!(activate(&dir.join("missing"), &target).is_err());
        assert_eq!(
            fs::read_to_string(target.join("kept")).unwrap(),
            "last working resources"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
