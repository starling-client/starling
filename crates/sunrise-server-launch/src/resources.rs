//! Read-only discovery and validation of the game files the dedicated server will use.

use crate::{GAME_VERSION, Install, config::Config};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};
use sunrise_core::prepared_profile::{MANIFEST, Manifest, Profile};

/// The chosen folder, and whether the supervisor must prepare it before launch.
#[derive(Debug, PartialEq, Eq)]
pub enum Selection {
    Ready(PathBuf),
    NeedsPreparation(PathBuf),
}

/// Select the same folder for startup, description and import guidance without changing it.
/// `supervisor_install` provides the managed slot if the engine is not in an install.
/// An explicit jar makes setup prepare the dedicated profile even if a current folder exists.
pub fn select(
    config: &Config,
    dir: &Path,
    program: Option<&Path>,
    supervisor_install: Option<&Path>,
    supplied_jar: bool,
) -> Result<Selection, String> {
    let output = if !config.engine.resources.is_empty() {
        let path = dir.join(&config.engine.resources);
        if !supplied_jar {
            check_prepared(&path)?;
            return Ok(Selection::Ready(path));
        }
        path
    } else {
        let engine_install = program.and_then(Install::around);
        let server_version = format!("{GAME_VERSION}-server");
        let profile_root = Path::new("game-files/sunrise-client");
        let managed = engine_install
            .as_ref()
            .map(|install| install.root.as_path())
            .or(supervisor_install)
            .unwrap_or(dir)
            .join(profile_root)
            .join(&server_version);
        let mut defaults = vec![dir.join(profile_root)];
        if let Some(install) = &engine_install {
            defaults.push(install.root.join(profile_root));
        }
        if let Some(program) = program {
            for ancestor in program.ancestors().skip(1).take(4) {
                defaults.push(ancestor.join(profile_root));
                defaults.push(ancestor.join(".local/sunrise-client"));
            }
        }
        let mut candidates = vec![dir.join("resources")];
        candidates.extend(defaults.iter().map(|folder| folder.join(&server_version)));
        candidates.extend(defaults.iter().map(|folder| folder.join(GAME_VERSION)));
        if !candidates.contains(&managed) {
            candidates.push(managed.clone());
        }
        for (i, path) in candidates
            .into_iter()
            .filter(|path| path.join(MANIFEST).is_file())
            .enumerate()
        {
            let Ok(manifest) = Manifest::read(&path) else {
                continue;
            };
            let dedicated = manifest.profile() == Profile::Server;
            // A current first candidate keeps precedence. After skipping it, only a dedicated
            // profile can be selected or refreshed automatically.
            if (dedicated || i == 0)
                && !supplied_jar
                && check_prepared_manifest(&path, &manifest).is_ok()
            {
                return Ok(Selection::Ready(path));
            }
            if dedicated {
                return Ok(Selection::NeedsPreparation(path));
            }
        }
        managed
    };
    // Missing metadata allows a fresh preparation. Damaged metadata cannot establish which
    // profile owns an existing folder, so leave it alone and report where to prepare instead.
    if !output
        .join(MANIFEST)
        .symlink_metadata()
        .is_err_and(|error| error.kind() == ErrorKind::NotFound)
        && read_prepared_manifest(&output)?.profile() != Profile::Server
    {
        return Err(format!(
            "{} was prepared for the full game. {PREPARE_ELSEWHERE}",
            output.display(),
        ));
    }
    Ok(Selection::NeedsPreparation(output))
}

/// The files `sunrise-prepare` derives from the game and the engine reads at start (docs/hosting.md: a folder
/// prepared before `registries.json` and `commands.json` were kept stops with "registries.json not found").
const PREPARED_FILES: [&str; 4] = [
    "facts.json",
    "block-tags.json",
    "registries.json",
    "commands.json",
];

/// Folders of the client's own assets the engine loads fonts and textures from at start.
const PREPARED_DIRS: [&str; 2] = ["assets/minecraft/font", "assets/minecraft/textures"];

const PREPARE_ELSEWHERE: &str = "Prepare a separate folder with \
    `sunrise-prepare --server --output NEW_FOLDER` and select it with `engine.resources` in server.toml.";

fn read_prepared_manifest(resources: &Path) -> Result<Manifest, String> {
    Manifest::read(resources).map_err(|problem| {
        format!(
            "the prepared game files in {} are incomplete or damaged:\n  {problem}\n{PREPARE_ELSEWHERE}",
            resources.display(),
        )
    })
}

/// Check the same profile/version contract as `sunrise-prepare --check` before launch or import.
pub fn check_prepared(resources: &Path) -> Result<Profile, String> {
    check_prepared_manifest(resources, &read_prepared_manifest(resources)?)
}

fn check_prepared_manifest(resources: &Path, manifest: &Manifest) -> Result<Profile, String> {
    let profile = manifest.profile();
    let mut problems = manifest.version_problems(resources, GAME_VERSION);
    for name in PREPARED_FILES {
        if !resources.join(name).is_file() {
            problems.push(format!("{name} is missing"));
        }
    }
    for (name, expected) in [
        ("facts.json", manifest.facts_sha256.as_deref()),
        ("block-tags.json", manifest.block_tags_sha256.as_deref()),
    ] {
        if let Some(expected) = expected
            && let Ok(actual) = sha256_file(&resources.join(name))
            && !actual.eq_ignore_ascii_case(expected)
        {
            problems.push(format!(
                "{name} is not the file the preparation wrote (checksum differs)"
            ));
        }
    }
    if profile == Profile::Server {
        problems.extend(manifest.server_inventory_problems(resources));
    } else {
        for dir in PREPARED_DIRS {
            let empty = std::fs::read_dir(resources.join(dir))
                .map_or(true, |mut entries| entries.next().is_none());
            if empty {
                problems.push(format!("{dir} is missing or empty"));
            }
        }
    }
    if problems.is_empty() {
        return Ok(profile);
    }
    Err(format!(
        "the prepared game files in {} are incomplete, stale or damaged:\n  {}\nPrepare them again with \
         `{}`, or copy the whole prepared folder to the server again.",
        resources.display(),
        problems.join("\n  "),
        profile.repair(resources),
    ))
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder as `sunrise-prepare` leaves it, as far as `check_prepared` looks.
    fn prepared_folder(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("sunrise-server-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in PREPARED_DIRS {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            std::fs::write(root.join(dir).join("file"), "x").unwrap();
        }
        for file in PREPARED_FILES {
            std::fs::write(root.join(file), file).unwrap();
        }
        let manifest = serde_json::json!({
            "schema": crate::prepared_profile::SCHEMA,
            "minecraft": GAME_VERSION,
            "facts_sha256": sha256_file(&root.join("facts.json")).unwrap(),
            "block_tags_sha256": sha256_file(&root.join("block-tags.json")).unwrap(),
        });
        std::fs::write(root.join("sunrise-resources.json"), manifest.to_string()).unwrap();
        stamp(&root, Profile::Full.output_version());
        root
    }

    fn stamp(root: &Path, version: u32) {
        std::fs::write(
            root.join(crate::prepared_profile::STAMP),
            format!("{{\"output_version\":{version}}}"),
        )
        .unwrap();
    }

    #[test]
    fn a_stale_full_folder_is_refused_before_launch() {
        let root = prepared_folder("stale");
        stamp(&root, 7);
        let error = check_prepared(&root).unwrap_err();
        assert!(
            error.contains("output version 7") && error.contains(&Profile::Full.repair(&root)),
            "{error}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_complete_prepared_folder_passes() {
        let root = prepared_folder("complete");
        assert_eq!(check_prepared(&root), Ok(Profile::Full));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_server_profile_needs_no_client_asset_folders() {
        let root = prepared_folder("server-profile");
        std::fs::remove_dir_all(root.join("assets")).unwrap();
        assert!(
            check_prepared(&root)
                .unwrap_err()
                .contains("assets/minecraft/font")
        );
        let path = root.join("sunrise-resources.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        manifest["profile"] = "server".into();
        manifest["server_files"] = serde_json::json!(["mob-data.json"]);
        std::fs::write(&path, manifest.to_string()).unwrap();
        assert!(
            check_prepared(&root)
                .unwrap_err()
                .contains("output version")
        );
        stamp(&root, Profile::Server.output_version());
        std::fs::write(root.join("mob-data.json"), "{}").unwrap();
        assert_eq!(check_prepared(&root), Ok(Profile::Server));
        std::fs::remove_file(root.join("mob-data.json")).unwrap();
        std::fs::remove_file(root.join("commands.json")).unwrap();
        let error = check_prepared(&root).unwrap_err();
        assert!(
            error.contains("commands.json is missing")
                && error.contains("mob-data.json is missing or empty"),
            "{error}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_files_and_folders_are_named() {
        let root = prepared_folder("missing");
        std::fs::remove_file(root.join("registries.json")).unwrap();
        std::fs::remove_dir_all(root.join("assets/minecraft/font")).unwrap();
        let text = check_prepared(&root).unwrap_err();
        assert!(text.contains("registries.json is missing"), "{text}");
        assert!(
            text.contains("assets/minecraft/font is missing or empty"),
            "{text}"
        );
        assert!(!text.contains("commands.json"), "{text}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_empty_asset_folder_counts_as_missing() {
        let root = prepared_folder("empty");
        std::fs::remove_file(root.join("assets/minecraft/textures/file")).unwrap();
        let text = check_prepared(&root).unwrap_err();
        assert!(
            text.contains("assets/minecraft/textures is missing or empty"),
            "{text}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_changed_derived_file_is_reported() {
        let root = prepared_folder("changed");
        std::fs::write(root.join("facts.json"), "damaged").unwrap();
        let text = check_prepared(&root).unwrap_err();
        assert!(
            text.contains("facts.json is not the file the preparation wrote"),
            "{text}"
        );
        assert!(!text.contains("block-tags.json"), "{text}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_broken_manifest_is_reported() {
        let root = prepared_folder("manifest");
        std::fs::write(root.join("sunrise-resources.json"), "{").unwrap();
        let text = check_prepared(&root).unwrap_err();
        assert!(
            text.contains("sunrise-resources.json is not valid JSON"),
            "{text}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
