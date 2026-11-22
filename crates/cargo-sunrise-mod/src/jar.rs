//! Add an exact, original Java jar as a passive asset/data source.
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use sunrise_mod_package::{
    Manifest,
    manifest::{JarRef, MANIFEST, collect_jar_requirements},
    selection::{LaunchArguments, data_dir_with},
};
use sunrise_port_pack::fetch::hash_reader;

/// The store the game would use with neither the tool nor a `--home` argument.
fn default_store() -> PathBuf {
    std::env::var_os("SUNRISE_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir_with(|key| std::env::var_os(key)).join("sunrise"))
        .join("jars")
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

/// Check each component below the project before creating or writing anything below it.
fn stage_dir(project: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut dir =
        dunce::canonicalize(project).map_err(|e| format!("{}: {e}", project.display()))?;
    for component in relative {
        dir.push(component);
        let context = |e: io::Error| format!("{}: {e}", dir.display());
        match fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
            Ok(_) => {
                return Err(format!(
                    "{}: expected a real development directory",
                    dir.display()
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&dir).map_err(context)?
            }
            Err(e) => return Err(context(e)),
        }
    }
    Ok(dir)
}

/// Stage independent snapshots of the project's exact originals. Return the home for the game.
pub(crate) fn stage_for_run(
    manifest: &Manifest,
    project: &Path,
    source: Option<&Path>,
    game_args: &[String],
) -> Result<Option<PathBuf>, String> {
    if manifest.jar.is_empty() {
        return Ok(None);
    }
    let relative_home = Path::new(crate::RUN_DIR).join("home");
    let home = project.join(&relative_home);
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let launch = LaunchArguments::parse(game_args.iter().cloned(), &cwd, &home, None)
        .map_err(|e| e.to_string())?;
    if launch.context.home != cwd.join(&home) {
        return Err(format!(
            "{} needs jars in its development home ({}); omit the game's --home and use `cargo sunrise-mod run --jars <dir>` to choose where to read originals",
            manifest.id,
            home.display()
        ));
    }
    let source = source.map_or_else(default_store, Path::to_path_buf);
    let dest = stage_dir(project, &relative_home.join("jars"))?;
    let source_in_stage = dunce::canonicalize(&source).is_ok_and(|dir| dir.starts_with(&dest));
    let requirements = collect_jar_requirements([(manifest.id.as_str(), manifest.jar.as_slice())])
        .map_err(|e| e.to_string())?;
    let wanted: HashSet<_> = requirements
        .iter()
        .map(|r| r.jar.cache_path(&dest))
        .collect();
    let dest_error = |e: io::Error| format!("{}: {e}", dest.display());
    let mut stale = Vec::new();
    for entry in fs::read_dir(&dest).map_err(dest_error)? {
        let entry = entry.map_err(dest_error)?;
        let path = entry.path();
        if entry.file_type().map_err(dest_error)?.is_dir() {
            return Err(format!(
                "{}: unexpected directory in the development jar store",
                path.display()
            ));
        }
        if !wanted.contains(&path)
            && entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".jar"))
                .is_some_and(|hash| {
                    hash.len() == 128
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
        {
            stale.push(path);
        }
    }
    let mut pending = Vec::new();
    for requirement in requirements {
        let jar = requirement.jar;
        let path = jar.cache_path(&source);
        let unavailable = |why: &str| {
            format!(
                "{} needs Java jar {} {} (SHA-512 {}): {}: {why}\n\
                 Add the exact original with `cargo sunrise-mod use-jar <path>` and run without --jars, or use `cargo sunrise-mod run --jars <dir>` with `<sha512>.jar` files",
                manifest.id,
                jar.id(),
                jar.version(),
                jar.sha512(),
                path.display()
            )
        };
        let original = dunce::canonicalize(&path).map_err(|e| unavailable(&e.to_string()))?;
        if source_in_stage || original.starts_with(&dest) {
            return Err(unavailable(
                "the development home is not an original jar store",
            ));
        }
        let metadata = fs::metadata(&original).map_err(|e| unavailable(&e.to_string()))?;
        if !metadata.is_file() {
            return Err(unavailable("not a file"));
        }
        let file = fs::File::open(&original).map_err(|e| unavailable(&e.to_string()))?;
        let staged = jar.cache_path(&dest);
        #[cfg(unix)]
        let reusable = {
            use std::os::unix::fs::MetadataExt;
            fs::symlink_metadata(&staged).is_ok_and(|metadata| {
                metadata.is_file() && !is_link(&metadata) && metadata.nlink() == 1
            }) && fs::File::open(&staged)
                .and_then(|staged| hash_reader::<sha2::Sha512>(staged, io::sink()))
                .is_ok_and(|hash| format!("{hash:x}") == jar.sha512())
        };
        // Without a link count check, an existing file could share the original's bytes.
        #[cfg(not(unix))]
        let reusable = false;
        let hash = if reusable {
            // Still verify the selected original; a previous run must not mask a bad input.
            hash_reader::<sha2::Sha512>(file, io::sink())
        } else {
            let mut copy = tempfile::NamedTempFile::new_in(&dest).map_err(dest_error)?;
            let hash = hash_reader::<sha2::Sha512>(file, &mut copy);
            pending.push((copy, staged));
            hash
        }
        .map_err(|e| unavailable(&e.to_string()))?;
        if format!("{hash:x}") != jar.sha512() {
            return Err(unavailable("the file does not match the listed SHA-512"));
        }
    }
    // All originals have passed. Publish their verified bytes without linking to the source and
    // prune old cache entries non-recursively; an input failure leaves the old snapshot set intact.
    for (copy, staged) in pending {
        copy.persist(&staged)
            .map_err(|e| format!("{}: {e}", staged.display()))?;
    }
    for old in stale {
        fs::remove_file(&old).map_err(|e| format!("{}: {e}", old.display()))?;
    }
    Ok(Some(home))
}

pub fn run(args: &[String]) -> Result<(), String> {
    let [path] = args else {
        return Err("usage: cargo sunrise-mod use-jar <path>".into());
    };
    let manifest_path = Path::new(MANIFEST);
    let text = fs::read_to_string(manifest_path).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest = Manifest::parse(&text)?;
    let path = Path::new(path);
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let store = default_store();
    fs::create_dir_all(&store).map_err(|e| format!("{}: {e}", store.display()))?;
    let mut original =
        tempfile::NamedTempFile::new_in(&store).map_err(|e| format!("{}: {e}", store.display()))?;
    let hash = hash_reader::<sha2::Sha512>(file, &mut original)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let identity = sunrise_port_pack::jar::read_identity(original.path())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let jar = JarRef::new(identity.id, identity.version, format!("{hash:x}"))?;
    let mut listed = false;
    for existing in manifest
        .jar
        .iter()
        .filter(|entry| entry.sha512() == jar.sha512())
    {
        if existing != &jar {
            return Err(format!(
                "{MANIFEST}: this hash lists {} {}, but {} identifies {} {}; correct the existing entry",
                existing.id(),
                existing.version(),
                path.display(),
                jar.id(),
                jar.version()
            ));
        }
        listed = true;
    }
    let candidate = if listed {
        None
    } else {
        let entry = toml::to_string(&jar).map_err(|e| e.to_string())?;
        let candidate = format!("{text}\n[[jar]]\n{entry}");
        Manifest::parse(&candidate).map_err(|e| format!(
            "{MANIFEST}: cannot append a jar entry; use [[jar]] tables rather than an inline jar array: {e}"
        ))?;
        Some(candidate)
    };
    // Publish the exact bytes that were hashed before naming them in the manifest. Re-listing a
    // jar can restore a deleted or damaged cache entry without editing the manifest.
    let cached = jar.cache_path(&store);
    original
        .persist(&cached)
        .map_err(|e| format!("{}: {e}", cached.display()))?;
    let Some(candidate) = candidate else {
        println!("Jar {} {} is already listed", jar.id(), jar.version());
        return Ok(());
    };
    let partial = manifest_path.with_extension(format!("toml.{}.partial", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| format!("{}: {e}", partial.display()))?;
    let result = (|| -> io::Result<()> {
        file.write_all(candidate.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, manifest_path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result.map_err(|e| format!("{MANIFEST}: {e}"))?;
    println!("Added jar {} {}", jar.id(), jar.version());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA512: &str = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
                          2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";

    fn fixture(root: &Path) -> (Manifest, PathBuf, PathBuf) {
        let source = root.join("originals");
        fs::create_dir(&source).unwrap();
        let manifest = Manifest::parse(&format!(
            "id = 'garden'\nname = 'Garden'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n\
             [[jar]]\nid = 'original'\nversion = '1.0'\nsha512 = '{SHA512}'\n"
        ))
        .unwrap();
        let original = manifest.jar[0].cache_path(&source);
        fs::write(&original, b"abc").unwrap();
        (manifest, source, original)
    }

    #[test]
    fn run_stages_only_declared_originals_and_keeps_the_previous_stage_on_refusal() {
        let root = tempfile::tempdir().unwrap();
        let (manifest, source, original) = fixture(root.path());
        let home = root.path().join(crate::RUN_DIR).join("home");
        let jars = home.join("jars");
        fs::write(source.join("unrelated.jar"), b"do not stage").unwrap();
        fs::create_dir_all(&jars).unwrap();
        let stale = jars.join(format!("{}.jar", "0".repeat(128)));
        fs::write(&stale, b"previous run").unwrap();
        let sentinel = jars.join("notes.jar");
        fs::write(&sentinel, b"keep me").unwrap();

        assert_eq!(
            stage_for_run(&manifest, root.path(), Some(&source), &[]).unwrap(),
            Some(home)
        );
        let staged = manifest.jar[0].cache_path(&jars);
        assert_eq!(fs::read(&staged).unwrap(), b"abc");
        assert!(!stale.exists());
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep me");
        assert_eq!(fs::read_dir(&jars).unwrap().count(), 2);
        assert_eq!(fs::read(&original).unwrap(), b"abc");
        assert_eq!(fs::read_dir(&source).unwrap().count(), 2);

        fs::remove_file(&original).unwrap();
        let error = stage_for_run(&manifest, root.path(), Some(&source), &[]).unwrap_err();
        for expected in [
            "garden",
            "original",
            "1.0",
            SHA512,
            &original.to_string_lossy(),
        ] {
            assert!(error.contains(expected), "{error}");
        }
        assert_eq!(fs::read(&staged).unwrap(), b"abc");
        assert_eq!(fs::read_dir(&jars).unwrap().count(), 2);
    }

    #[test]
    fn run_reuses_verified_copies_and_an_original_cannot_change_them() {
        let root = tempfile::tempdir().unwrap();
        let (manifest, source, original) = fixture(root.path());
        let stage = || stage_for_run(&manifest, root.path(), Some(&source), &[]);
        fs::write(&original, b"wrong").unwrap();
        assert!(stage().is_err());
        let jars = root.path().join(crate::RUN_DIR).join("home/jars");
        assert_eq!(fs::read_dir(&jars).unwrap().count(), 0);

        fs::write(&original, b"abc").unwrap();
        stage().unwrap();
        let staged = manifest.jar[0].cache_path(&jars);
        #[cfg(unix)]
        let stamp = {
            let handle = fs::File::options().write(true).open(&staged).unwrap();
            let stamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
            handle.set_modified(stamp).unwrap();
            stamp
        };
        stage().unwrap();
        #[cfg(unix)]
        assert_eq!(fs::metadata(&staged).unwrap().modified().unwrap(), stamp);

        fs::write(&original, b"in-place edit").unwrap();
        assert_eq!(fs::read(&staged).unwrap(), b"abc");
        fs::remove_file(&original).unwrap();
        fs::write(&original, b"replacement").unwrap();
        assert!(stage().is_err());
        assert_eq!(fs::read(&staged).unwrap(), b"abc");
    }

    #[cfg(unix)]
    #[test]
    fn run_refuses_linked_stage_directories_without_touching_outside_files() {
        for relative in [
            "target",
            "target/sunrise-mod/run/home",
            "target/sunrise-mod/run/home/jars",
        ] {
            let project = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let (manifest, source, _) = fixture(project.path());
            let redirected = project.path().join(relative);
            let expected_jars = Path::new(crate::RUN_DIR).join("home/jars");
            let outside_jars = outside
                .path()
                .join(expected_jars.strip_prefix(relative).unwrap());
            fs::create_dir_all(&outside_jars).unwrap();
            let sentinel = outside_jars.join("keep.jar");
            fs::write(&sentinel, b"player jar").unwrap();
            fs::create_dir_all(redirected.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(outside.path(), &redirected).unwrap();

            assert!(stage_for_run(&manifest, project.path(), Some(&source), &[]).is_err());
            assert_eq!(fs::read_link(&redirected).unwrap(), outside.path());
            assert_eq!(fs::read(&sentinel).unwrap(), b"player jar");
            assert_eq!(fs::read_dir(outside_jars).unwrap().count(), 1);
        }
    }
}
