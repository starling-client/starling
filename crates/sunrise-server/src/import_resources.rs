//! `sunrise-server import-resources DIR`: puts a folder of prepared game files (made on the owner's own
//! computer by the launcher or `sunrise-prepare`) where the server reads them, so a headless machine
//! without Java needs no `prepare`. docs/server.md, section "Game files".
//!
//! The folder is checked at the same boundary as engine startup, copied beside its destination, and
//! moved in with a rename. The previous copy is put back if any step fails.

use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};
use sunrise_server_launch::prepared_profile::{MANIFEST, Manifest, Profile};
use sunrise_server_launch::resources::{self, Selection};
use sunrise_server_launch::{self as launch, GAME_VERSION, Install, config};

/// Use the first slot for this profile without replacing the other preparation.
fn destination_for(
    dir: &Path,
    configured: &str,
    installer: Option<&Path>,
    profile: Profile,
) -> PathBuf {
    if !configured.is_empty() {
        return dir.join(configured);
    }
    let here = dir.join("resources");
    let existing = Manifest::read(&here)
        .ok()
        .map(|manifest| manifest.profile());
    if existing == Some(profile) || existing.is_none() && (here.exists() || installer.is_none()) {
        return here;
    }
    let slot = match profile {
        Profile::Full => GAME_VERSION.to_owned(),
        Profile::Server => format!("{GAME_VERSION}-server"),
    };
    let relative = Path::new("game-files/sunrise-client").join(slot);
    let local = dir.join(&relative);
    if local.join(MANIFEST).is_file() {
        local
    } else {
        installer.unwrap_or(dir).join(relative)
    }
}

/// `import-resources DIR`.
pub fn command(dir: &Path, args: &[String], out: &mut impl Write) -> Result<(), String> {
    let [source] = args else {
        return Err(
            "import-resources needs the prepared folder to copy (one with sunrise-resources.json)"
                .into(),
        );
    };
    let config = config::Config::load(dir);
    let program = match &config {
        Ok(config) => launch::program(config, dir),
        Err(_) => launch::program(&config::Config::default(), dir),
    };
    let install = program
        .as_deref()
        .ok()
        .and_then(Install::around)
        .or_else(Install::find);
    let configured = config
        .as_ref()
        .map_or("", |cfg| cfg.engine.resources.as_str());
    let target = import(
        Path::new(source),
        |profile| {
            destination_for(
                dir,
                configured,
                install.as_ref().map(|i| i.root.as_path()),
                profile,
            )
        },
        &mut || Ok(()),
    )?;
    writeln!(out, "Imported the game files into {}", target.display())
        .map_err(|e| e.to_string())?;
    let (config, program) = match (config, program) {
        (Ok(config), Ok(program)) => (config, program),
        (Err(error), _) | (_, Err(error)) => {
            return writeln!(out, "Cannot start the server: {error}").map_err(|e| e.to_string());
        }
    };
    let selected = resources::select(
        &config,
        dir,
        Some(&program),
        install.as_ref().map(|i| i.root.as_path()),
        false,
    );
    if matches!(selected, Ok(Selection::Ready(path)) if path == target) {
        writeln!(out, "Next: `sunrise-server start` (or `sunrise-server restart` if it is running) to start on them.")
    } else {
        writeln!(out,
            "To use these files, set [engine] resources in server.toml to {}, then start or restart.",
            target.display()
        )
    }
    .map_err(|e| e.to_string())
}

/// Checks `source` once, copies it next to its profile's `target`, and swaps it in.
/// `each_file` runs after every copied file (a test makes it fail partway).
fn import(
    source: &Path,
    target: impl FnOnce(Profile) -> PathBuf,
    each_file: &mut dyn FnMut() -> io::Result<()>,
) -> Result<PathBuf, String> {
    if !source.is_dir() {
        return Err(format!("{} is not a folder", source.display()));
    }
    if !source.join(MANIFEST).is_file() {
        return Err(format!(
            "{} has no {MANIFEST}: it is not a prepared folder",
            source.display()
        ));
    }
    let profile =
        resources::check_prepared(source).map_err(|e| format!("{e}\nNothing was changed."))?;
    let target = target(profile);
    if let Ok(manifest) = Manifest::read(&target)
        && manifest.profile() != profile
    {
        return Err(format!(
            "{} contains a different preparation profile; choose a different [engine] resources folder",
            target.display()
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent folder", target.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let name = target
        .file_name()
        .map_or_else(|| "resources".into(), |n| n.to_string_lossy().into_owned());
    let pid = std::process::id();
    let fresh = parent.join(format!(".{name}.importing-{pid}"));
    let old = parent.join(format!(".{name}.previous-{pid}"));
    let _ = std::fs::remove_dir_all(&fresh);
    let copied = copy_tree(source, &fresh, each_file)
        .map_err(|e| format!("copying {}: {e}", source.display()))
        .and_then(|()| {
            // Copied bytes must still pass the same check before they replace anything.
            resources::check_prepared(&fresh)
                .map(|_| ())
                .map_err(|e| format!("the copy is incomplete: {e}"))
        });
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(format!("{e}\nThe previous game files are untouched."));
    }
    let had_old = target.symlink_metadata().is_ok();
    if had_old && let Err(e) = std::fs::rename(&target, &old) {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(format!(
            "{}: {e}\nThe previous game files are untouched.",
            target.display()
        ));
    }
    if let Err(e) = std::fs::rename(&fresh, &target) {
        if had_old {
            let _ = std::fs::rename(&old, &target);
        }
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(format!(
            "{}: {e}\nThe previous game files are untouched.",
            target.display()
        ));
    }
    if had_old {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(target)
}

/// Copies the folder `from` to the new folder `to`, following links, calling `each_file` after each file.
fn copy_tree(
    from: &Path,
    to: &Path,
    each_file: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    let mut entries: Vec<_> = std::fs::read_dir(from)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if std::fs::metadata(&src)?.is_dir() {
            copy_tree(&src, &dst, each_file)?;
        } else {
            std::fs::copy(&src, &dst)?;
            each_file()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_server_launch::prepared_profile::{SCHEMA, STAMP};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sunrise-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// A prepared folder the engine's own check accepts, stamped `minecraft`; `tag` marks its content.
    fn prepared(root: &Path, minecraft: &str, tag: &str) -> PathBuf {
        let dir = root.join(format!("prepared-{tag}"));
        for file in [
            "facts.json",
            "block-tags.json",
            "registries.json",
            "commands.json",
        ] {
            put(&dir.join(file), format!("{{\"{tag}\": 1}}").as_bytes());
        }
        put(&dir.join("assets/minecraft/font/default.json"), b"{}");
        put(
            &dir.join("assets/minecraft/textures/block/stone.png"),
            tag.as_bytes(),
        );
        put(
            &dir.join("sunrise-resources.json"),
            format!("{{\"schema\": {SCHEMA}, \"minecraft\": \"{minecraft}\"}}").as_bytes(),
        );
        put(
            &dir.join(STAMP),
            format!("{{\"output_version\":{}}}", Profile::Full.output_version()).as_bytes(),
        );
        dir
    }

    fn prepared_server(root: &Path, tag: &str) -> PathBuf {
        let dir = prepared(root, GAME_VERSION, tag);
        std::fs::remove_dir_all(dir.join("assets")).unwrap();
        put(
            &dir.join(MANIFEST),
            serde_json::json!({
                "profile": "server", "schema": SCHEMA, "minecraft": GAME_VERSION,
                "server_files": ["facts.json"],
            })
            .to_string()
            .as_bytes(),
        );
        put(
            &dir.join(STAMP),
            serde_json::json!({ "output_version": Profile::Server.output_version() })
                .to_string()
                .as_bytes(),
        );
        dir
    }

    fn tag(target: &Path) -> String {
        std::fs::read_to_string(target.join("assets/minecraft/textures/block/stone.png")).unwrap()
    }

    fn leftovers(parent: &Path) -> Vec<String> {
        std::fs::read_dir(parent)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect()
    }

    /// The import goes where the server will read it: a configured folder first, then an existing
    /// `resources/` (which shadows the installer's copy), then the installer's folder.
    #[test]
    fn the_destination_is_the_folder_the_server_reads_first() {
        let root = scratch("destination");
        let installer = Some(root.join("install"));
        let installed = root
            .join("install/game-files/sunrise-client")
            .join(GAME_VERSION);
        assert_eq!(
            destination_for(&root, "", installer.as_deref(), Profile::Full),
            installed
        );
        assert_eq!(
            destination_for(&root, "", None, Profile::Full),
            root.join("resources")
        );
        assert_eq!(
            destination_for(&root, "files/mc", installer.as_deref(), Profile::Full),
            root.join("files/mc")
        );
        std::fs::create_dir_all(root.join("resources")).unwrap();
        assert_eq!(
            destination_for(&root, "", installer.as_deref(), Profile::Full),
            root.join("resources")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn installed_import_replaces_the_preferred_server_slot_without_touching_full() {
        let root = scratch("installed-profiles");
        let install = root.join("install");
        let outputs = install.join("game-files/sunrise-client");
        let full = outputs.join(GAME_VERSION);
        let server = outputs.join(format!("{GAME_VERSION}-server"));
        let existing = prepared(&root, GAME_VERSION, "full");
        std::fs::create_dir_all(&outputs).unwrap();
        std::fs::rename(existing, &full).unwrap();
        let imported = prepared_server(&root, "server");
        assert!(!server.exists());
        for tag in ["server first", "server second"] {
            put(&imported.join("facts.json"), tag.as_bytes());
            let chosen = import(
                &imported,
                |profile| destination_for(&root, "", Some(&install), profile),
                &mut || Ok(()),
            )
            .unwrap();
            assert_eq!(chosen, server);
            assert_eq!(
                std::fs::read_to_string(server.join("facts.json")).unwrap(),
                tag
            );
            assert_eq!(self::tag(&full), "full");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_guidance_names_an_override_only_when_start_would_use_another_folder() {
        let root = scratch("guidance");
        let server = root.join("server");
        std::fs::create_dir(&server).unwrap();
        let full = server.join("resources");
        std::fs::rename(prepared(&root, GAME_VERSION, "full"), &full).unwrap();
        let source = prepared_server(&root, "server");
        let install = root.join("install");
        std::fs::create_dir_all(install.join("current")).unwrap();
        let engine = install.join("releases/test-release/sunrise-engine");
        put(&engine, b"not executable");
        let mut config = config::Config::default();
        config.engine.path = engine.to_str().unwrap().into();
        put(
            &server.join(config::FILE),
            config::render(&config).as_bytes(),
        );
        let slot = install.join(format!("game-files/sunrise-client/{GAME_VERSION}-server"));
        let guide = || {
            let mut out = Vec::new();
            command(&server, &[source.to_str().unwrap().into()], &mut out).unwrap();
            String::from_utf8(out).unwrap()
        };
        let reply = guide();
        assert!(reply.contains("set [engine] resources"), "{reply}");
        assert!(reply.contains(&slot.display().to_string()), "{reply}");
        put(&full.join(STAMP), br#"{"output_version":7}"#);
        let reply = guide();
        assert!(reply.contains("sunrise-server restart"), "{reply}");
        assert!(!reply.contains("set [engine] resources"), "{reply}");
        assert_eq!(tag(&full), "full");

        config.engine.path = "missing-engine".into();
        config.engine.resources = "files/server".into();
        put(
            &server.join(config::FILE),
            config::render(&config).as_bytes(),
        );
        let reply = guide();
        assert!(reply.contains("engine.path missing-engine"), "{reply}");
        assert!(!reply.contains("sunrise-server start"), "{reply}");
        assert!(!reply.contains("set [engine] resources"), "{reply}");
        let copied = server.join("files/server");
        assert_eq!(resources::check_prepared(&copied), Ok(Profile::Server));
        assert_eq!(
            std::fs::read(copied.join("facts.json")).unwrap(),
            std::fs::read(source.join("facts.json")).unwrap()
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_good_folder_is_copied_whole_and_replaces_the_old_one() {
        let root = scratch("good");
        let source = prepared(&root, GAME_VERSION, "new");
        let target = root.join("game-files/sunrise-client").join(GAME_VERSION);
        put(&target.join("stale.txt"), b"old");
        import(&source, |_| target.clone(), &mut || Ok(())).unwrap();
        assert_eq!(tag(&target), "new");
        assert!(target.join("sunrise-resources.json").is_file());
        assert!(
            !target.join("stale.txt").exists(),
            "the old copy is replaced, not merged"
        );
        assert!(resources::check_prepared(&target).is_ok());
        assert!(leftovers(target.parent().unwrap()).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_wrong_version_is_refused_and_nothing_changes() {
        let root = scratch("version");
        let source = prepared(&root, "26.2", "new");
        let old = prepared(&root, GAME_VERSION, "old");
        let target = root.join("target");
        import(&old, |_| target.clone(), &mut || Ok(())).unwrap();
        let e = import(&source, |_| target.clone(), &mut || Ok(())).unwrap_err();
        assert!(
            e.contains("prepared for Minecraft 26.2") && e.contains(GAME_VERSION),
            "{e}"
        );
        assert_eq!(tag(&target), "old");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_file_or_manifest_is_named() {
        let root = scratch("missing");
        let source = prepared(&root, GAME_VERSION, "new");
        std::fs::remove_file(source.join("registries.json")).unwrap();
        let target = root.join("target");
        let e = import(&source, |_| target.clone(), &mut || Ok(())).unwrap_err();
        assert!(e.contains("registries.json is missing"), "{e}");
        assert!(!target.exists());
        std::fs::remove_file(source.join("sunrise-resources.json")).unwrap();
        let e = import(&source, |_| target.clone(), &mut || Ok(())).unwrap_err();
        assert!(e.contains("no sunrise-resources.json"), "{e}");
        let e = import(&root.join("nope"), |_| target.clone(), &mut || Ok(())).unwrap_err();
        assert!(e.contains("is not a folder"), "{e}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failure_partway_keeps_the_old_copy_and_leaves_no_temp_folder() {
        let root = scratch("partway");
        let old = prepared(&root, GAME_VERSION, "old");
        let source = prepared(&root, GAME_VERSION, "new");
        let target = root.join("target");
        import(&old, |_| target.clone(), &mut || Ok(())).unwrap();
        let mut copied = 0;
        let e = import(&source, |_| target.clone(), &mut || {
            copied += 1;
            if copied == 3 {
                Err(io::Error::other("disk full"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(e.contains("disk full") && e.contains("untouched"), "{e}");
        assert_eq!(copied, 3);
        assert_eq!(tag(&target), "old");
        assert!(resources::check_prepared(&target).is_ok());
        assert!(leftovers(&root).is_empty(), "{:?}", leftovers(&root));
        let _ = std::fs::remove_dir_all(&root);
    }
}
