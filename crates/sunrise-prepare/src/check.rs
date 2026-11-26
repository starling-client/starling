//! `sunrise-prepare --check`: whether prepared resources are complete and current for this
//! preparer, without Java and without the game files (docs/api/launch.md). A preparation stamps
//! its folder with the `OUTPUT_VERSION` that made it (`STAMP`); tools/prepare_output.py leaves the
//! stamp out of the output's digest, so it never asks for a new `OUTPUT_VERSION`.

use crate::pyjson::Value;
use crate::{artifacts::Lock, write_json};
use std::path::Path;
use sunrise_core::prepared_profile::{Manifest, Profile, MANIFEST};

/// The file beside `sunrise-resources.json` naming the preparer's `OUTPUT_VERSION`.
pub use sunrise_core::prepared_profile::STAMP;

pub(crate) fn write_stamp_for(resources: &Path, version: u32) -> Result<(), String> {
    write_json(
        &resources.join(STAMP),
        &Value::Object(vec![(
            "output_version".into(),
            Value::Int(version.to_string()),
        )]),
    )
}

pub(crate) fn remove_stamp(resources: &Path) -> Result<(), String> {
    let path = resources.join(STAMP);
    match std::fs::remove_file(&path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

/// What the resources at `resources` lack for `root`'s pinned version, each with what to run;
/// empty when they are complete and current. Err when `root` has no readable research-lock.json.
pub fn check(root: &Path, resources: &Path) -> Result<Vec<String>, String> {
    check_profile(root, resources, Profile::Full)
}

/// Check the dedicated profile, without requiring any client extras.
pub fn check_server(root: &Path, resources: &Path) -> Result<Vec<String>, String> {
    check_profile(root, resources, Profile::Server)
}

fn check_profile(root: &Path, resources: &Path, profile: Profile) -> Result<Vec<String>, String> {
    let lock = Lock::read(root)?;
    let command = if profile == Profile::Server {
        "sunrise-prepare --server"
    } else {
        "sunrise-prepare --prepare"
    };
    if !resources.join(MANIFEST).is_file() {
        return Ok(vec![format!(
            "{} is not prepared: {command}",
            resources.display()
        )]);
    }
    let manifest = Manifest::read(resources)?;
    if manifest.profile() != profile {
        return Ok(vec![format!(
            "{} has the wrong preparation profile: {command}",
            resources.display()
        )]);
    }
    let mut lacking: Vec<_> = manifest
        .version_problems(resources, &lock.version)
        .into_iter()
        .map(|problem| format!("{problem}: {command}"))
        .collect();
    if profile == Profile::Server {
        if manifest.server_sha1.as_deref() != Some(lock.pin("server_sha1")?) {
            lacking.push(format!("server.jar is not the pinned one: {command}"));
        }
        lacking.extend(
            manifest
                .server_inventory_problems(resources)
                .into_iter()
                .map(|problem| format!("{problem}: {command}")),
        );
    } else {
        for (command, what) in missing_extras(resources) {
            lacking.push(format!("{what} missing: sunrise-prepare {command}"));
        }
    }
    Ok(lacking)
}

/// The files the server preparation just wrote, independent of caches and client extras.
pub(crate) fn server_files(root: &Path) -> Result<Value, String> {
    fn collect(
        root: &Path,
        dir: &Path,
        recursive: bool,
        names: &mut Vec<String>,
    ) -> Result<(), String> {
        let error = |e: std::io::Error| format!("{}: {e}", dir.display());
        for entry in std::fs::read_dir(dir).map_err(error)? {
            let entry = entry.map_err(error)?;
            let path = entry.path();
            let kind = entry.file_type().map_err(error)?;
            if kind.is_dir() && recursive {
                collect(root, &path, true, names)?;
            } else if kind.is_file() {
                let name = crate::relative_display(&path, root)?.replace('\\', "/");
                if sunrise_core::prepared_files::server_resource(&name)
                    && !matches!(name.as_str(), STAMP | "sunrise-resources.json")
                {
                    names.push(name);
                }
            }
        }
        Ok(())
    }
    let mut names = Vec::new();
    collect(root, root, false, &mut names)?;
    for dir in [
        "data",
        "worldgen",
        "biome-parameters",
        "assets/minecraft/lang",
    ] {
        collect(root, &root.join(dir), true, &mut names)?;
    }
    names.sort();
    Ok(Value::Array(names.into_iter().map(Value::Str).collect()))
}

/// What the resources at `resources` lack of what a preparation may leave out when the player's
/// asset store lacks it: each as its `sunrise-prepare` command (`packs`, `panorama`, `fonts`,
/// `languages`, `sounds`) and what it is. `--check` and the launcher's `COMPLETIONS`
/// (https://github.com/sunrise-org/sunrise-launcher/blob/main/src/backend.rs) both ask here, so they never disagree.
pub fn missing_extras(resources: &Path) -> Vec<(&'static str, &'static str)> {
    EXTRAS
        .iter()
        .filter(|(_, lacks, _)| lacks(resources))
        .map(|(what, _, command)| (*command, *what))
        .collect()
}

/// What, whether it is missing, the command.
pub(crate) type Extra = (&'static str, fn(&Path) -> bool, &'static str);

pub(crate) const EXTRAS: [Extra; 5] = [
    ("the built-in packs", lacks_packs, "packs"),
    ("the title panorama", lacks_panorama, "panorama"),
    ("the Unifont glyphs", lacks_unifont, "fonts"),
    ("the languages", lacks_languages, "languages"),
    ("the sounds", lacks_sounds, "sounds"),
];

fn lacks_packs(resources: &Path) -> bool {
    crate::BUILT_IN_PACKS
        .iter()
        .any(|pack| !resources.join("assets").join(pack).is_file())
}

/// The panorama needs six equally sized, non-placeholder square faces and an overlay: prepared
/// resources may hold only some of the full-size assets after an interrupted import, and the
/// client jar's faces are 1x1 placeholders (a PNG's width and height are the big-endian words at
/// bytes 16 and 20, in the IHDR chunk).
fn lacks_panorama(resources: &Path) -> bool {
    let background = resources.join("assets/minecraft/textures/gui/title/background");
    let mut expected = None;
    for face in 0..6 {
        let Ok(png) = std::fs::read(background.join(format!("panorama_{face}.png"))) else {
            return true;
        };
        if png.get(..16) != Some(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR") {
            return true;
        }
        let Some(dimensions) = png.get(16..24) else {
            return true;
        };
        let width =
            u32::from_be_bytes([dimensions[0], dimensions[1], dimensions[2], dimensions[3]]);
        let height =
            u32::from_be_bytes([dimensions[4], dimensions[5], dimensions[6], dimensions[7]]);
        if width <= 1 || width != height || expected.is_some_and(|size| size != width) {
            return true;
        }
        expected = Some(width);
    }
    !background.join("panorama_overlay.png").is_file()
}

fn lacks_unifont(resources: &Path) -> bool {
    !resources
        .join("assets/minecraft/font/unifont.zip")
        .is_file()
}

/// The language list only the asset index holds (the client jar's `lang` has English alone).
fn lacks_languages(resources: &Path) -> bool {
    !resources.join("assets/pack.mcmeta").is_file()
}

/// What the game's sound registry reads (sunrise-client sound_events.rs).
fn lacks_sounds(resources: &Path) -> bool {
    !resources.join("assets/minecraft/sounds.json").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OUTPUT_VERSION, SCHEMA, SERVER_OUTPUT_VERSION};
    use std::fs;

    fn folder(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sunrise-check-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("research-lock.json"),
            r#"{"minecraft": {"version": "9.9", "java_major": 25}}"#,
        )
        .unwrap();
        dir
    }

    fn file(path: &Path, data: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    #[test]
    fn server_check_requires_inventory_without_client_assets() {
        let dir = folder("server-profile");
        fs::write(
            dir.join("research-lock.json"),
            r#"{"minecraft":{"version":"9.9","java_major":25,"server_sha1":"test-pin"}}"#,
        )
        .unwrap();
        let out = dir.join("output");
        let path = out.join("data/minecraft/loot_table/example.json");
        file(&path, b"{}");
        file(&out.join("sunrise-resources.json"), format!(r#"{{"schema":{SCHEMA},"profile":"server","minecraft":"9.9","server_sha1":"test-pin","server_files":["data/minecraft/loot_table/example.json"]}}"#).as_bytes());
        write_stamp_for(&out, SERVER_OUTPUT_VERSION).unwrap();
        assert!(check_server(&dir, &out).unwrap().is_empty());
        assert!(check(&dir, &out).unwrap()[0].contains("wrong preparation profile"));
        fs::remove_file(path).unwrap();
        assert!(
            check_server(&dir, &out).unwrap()[0].contains("data/minecraft/loot_table/example.json")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A PNG's signature and IHDR size, all `lacks_panorama` reads.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(width.to_be_bytes());
        png.extend(height.to_be_bytes());
        png
    }

    #[test]
    fn panorama_check_finds_missing_and_inconsistent_faces() {
        let dir = folder("panorama");
        let background = dir.join("assets/minecraft/textures/gui/title/background");
        for face in 0..6 {
            file(
                &background.join(format!("panorama_{face}.png")),
                &png_header(2, 2),
            );
        }
        assert!(lacks_panorama(&dir));
        file(&background.join("panorama_overlay.png"), b"overlay");
        assert!(!lacks_panorama(&dir));
        file(&background.join("panorama_4.png"), &png_header(1, 1));
        assert!(lacks_panorama(&dir));
        file(&background.join("panorama_4.png"), &png_header(3, 3));
        assert!(lacks_panorama(&dir));
        fs::remove_file(background.join("panorama_4.png")).unwrap();
        assert!(lacks_panorama(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unprepared_resources_need_a_preparation() {
        let root = folder("unprepared");
        let lacking = check(&root, &root.join("out")).unwrap();
        assert_eq!(lacking.len(), 1);
        assert!(
            lacking[0].ends_with("sunrise-prepare --prepare"),
            "{lacking:?}"
        );
    }

    #[test]
    fn complete_resources_pass_and_each_gap_names_its_command() {
        let root = folder("complete");
        let out = root.join("out");
        file(
            &out.join("sunrise-resources.json"),
            format!(r#"{{"schema": {SCHEMA}, "minecraft": "9.9"}}"#).as_bytes(),
        );
        write_stamp_for(&out, OUTPUT_VERSION).unwrap();
        let assets = out.join("assets");
        for pack in crate::BUILT_IN_PACKS {
            file(&assets.join(pack), b"zip");
        }
        let background = assets.join("minecraft/textures/gui/title/background");
        for face in 0..6 {
            file(
                &background.join(format!("panorama_{face}.png")),
                &png_header(1024, 1024),
            );
        }
        file(&background.join("panorama_overlay.png"), b"overlay");
        file(&assets.join("minecraft/font/unifont.zip"), b"zip");
        file(&assets.join("pack.mcmeta"), b"{}");
        file(&assets.join("minecraft/sounds.json"), b"{}");
        assert_eq!(check(&root, &out).unwrap(), Vec::<String>::new());

        fs::remove_file(assets.join("minecraft/font/unifont.zip")).unwrap();
        fs::remove_file(out.join(STAMP)).unwrap();
        let lacking = check(&root, &out).unwrap();
        assert_eq!(lacking.len(), 2, "{lacking:?}");
        assert!(lacking[0].starts_with("made by output version unknown"));
        assert!(lacking[1].ends_with("sunrise-prepare fonts"));
    }

    #[test]
    fn a_preparation_removes_the_stamp_before_writing() {
        let root = folder("restamp");
        let out = root.join("out");
        write_stamp_for(&out, OUTPUT_VERSION).unwrap();
        remove_stamp(&out).unwrap();
        assert!(!out.join(STAMP).exists());
        // Nothing to remove is not an error.
        remove_stamp(&out).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn preparation_releases_its_lock_with_a_retained_descriptor() {
        let root = folder("retained-lock");
        let first = crate::hold_lock(&root, &|_| {}).unwrap();
        // A duplicate retains the same lock reference as a descriptor inherited
        // by a forked child, without depending on subprocess scheduling.
        let retained = first.0.try_clone().unwrap();
        let second = fs::OpenOptions::new()
            .write(true)
            .open(root.join(crate::LOCK))
            .unwrap();
        assert!(matches!(
            second.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        drop(first);
        second
            .try_lock()
            .expect("the completed preparation kept its lock");
        second.unlock().unwrap();
        drop(retained);
        drop(second);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_second_preparation_of_a_folder_waits_for_the_first() {
        let root = folder("lock");
        let first = crate::hold_lock(&root, &|_| {}).unwrap();
        let waited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let second = {
            let (root, waited) = (root.clone(), waited.clone());
            std::thread::spawn(move || {
                let said = std::cell::Cell::new(false);
                let _lock = crate::hold_lock(&root, &|line| {
                    said.set(line.starts_with("Waiting for another sunrise-prepare"))
                })
                .unwrap();
                waited.store(said.get(), std::sync::atomic::Ordering::SeqCst);
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!second.is_finished(), "the second lock did not wait");
        drop(first);
        second.join().unwrap();
        assert!(waited.load(std::sync::atomic::Ordering::SeqCst));
    }
}
