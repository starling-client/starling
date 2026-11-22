//! `.spack`, a Sunrise modpack (docs/modding/src/reference/registry-index.md): registry mods
//! played together as a profile. A zip holding `sunrise-modpack.toml`, which
//! names each mod by id, version and SHA-256 (the files themselves come from
//! the registry), and optionally `overrides/`: settings and resource packs
//! copied into the profile's game folder.
use crate::manifest::STORAGE_DIR;
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// The manifest's file name in a `.spack`.
pub const MANIFEST: &str = "sunrise-modpack.toml";
/// The manifest formats this build reads.
pub const FORMAT: u32 = 1;
const OVERRIDES: &str = "overrides/";
/// Bytes per override file and in all, unpacked.
const MAX_OVERRIDE: u64 = 64 << 20;
const MAX_OVERRIDES: u64 = 256 << 20;
/// The settings files an override may replace.
const SETTINGS: [&str; 3] = ["options.txt", "sunrise-graphics.txt", "sunrise-extras.txt"];

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    /// `[a-z0-9_]{1,64}`: its profile's folder.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The Minecraft version it is for.
    pub game: String,
    #[serde(default)]
    pub mods: Vec<PackMod>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackMod {
    pub id: String,
    pub version: String,
    pub sha256: String,
    /// Turned on in the profile (`sunrise-mods.txt`).
    #[serde(default = "on")]
    pub on: bool,
}

fn on() -> bool {
    true
}

#[cfg(feature = "index")]
impl PackMod {
    pub fn requirement(&self) -> crate::index::Requirement {
        crate::index::Requirement {
            id: self.id.clone(),
            version: self.version.clone(),
            sha256: self.sha256.clone(),
        }
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        if m.format != FORMAT {
            return Err(format!(
                "{MANIFEST}: format {}; this Sunrise reads {FORMAT}",
                m.format
            ));
        }
        let id_ok = |id: &str| {
            (1..=64).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        };
        if !id_ok(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        for (i, pm) in m.mods.iter().enumerate() {
            let hex = pm.sha256.len() == 64
                && pm
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !id_ok(&pm.id) || !hex || pm.version.is_empty() {
                return Err(format!(
                    "{MANIFEST}: mod {:?} needs an id, a version and a lowercase hex sha256",
                    pm.id
                ));
            }
            if m.mods[..i].iter().any(|other| other.id == pm.id) {
                return Err(format!("{MANIFEST}: mod {} is listed twice", pm.id));
            }
        }
        Ok(m)
    }
}

/// A `.spack` as read from disk: its manifest and its override files, by
/// path under `overrides/`.
#[derive(Debug)]
pub struct Modpack {
    pub manifest: Manifest,
    pub overrides: Vec<String>,
}

impl Modpack {
    pub fn read(path: &Path) -> Result<Modpack, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut text = String::new();
        zip.by_name(MANIFEST)
            .map_err(|_| format!("{MANIFEST}: missing"))?
            .take(1 << 20)
            .read_to_string(&mut text)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        let manifest = Manifest::parse(&text)?;
        let (mut overrides, mut total) = (Vec::new(), 0u64);
        for i in 0..zip.len() {
            let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            if name == MANIFEST || entry.is_dir() {
                continue;
            }
            let Some(rest) = name.strip_prefix(OVERRIDES) else {
                return Err(format!(
                    "{name}: a modpack holds only {MANIFEST} and overrides/"
                ));
            };
            check_override(rest, &manifest)?;
            if entry.size() > MAX_OVERRIDE {
                return Err(format!("{name} is over {} MiB", MAX_OVERRIDE >> 20));
            }
            total += entry.size();
            overrides.push(rest.to_owned());
        }
        if total > MAX_OVERRIDES {
            return Err(format!("overrides are over {} MiB", MAX_OVERRIDES >> 20));
        }
        overrides.sort();
        Ok(Modpack {
            manifest,
            overrides,
        })
    }

    /// Writes the overrides of the pack at `path` into the game folder
    /// `home`, each checked again. All or nothing: every file is inflated and
    /// counted in a staging folder inside `home` first, and only then moved into
    /// place, with the file it replaces kept aside. If anything fails, the files
    /// that were there are put back and what the import created is removed.
    pub fn extract_overrides(path: &Path, home: &Path) -> Result<Vec<PathBuf>, String> {
        let pack = Modpack::read(path)?;
        if pack.overrides.is_empty() {
            return Ok(Vec::new());
        }
        // Every destination first: refusing one after an earlier override was written would lose
        // the file that override replaced.
        for name in &pack.overrides {
            check_destination(home, name)?;
        }
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut created = Vec::new();
        let result = Self::import(&mut zip, &pack.overrides, home, &mut created);
        if result.is_err() {
            // Newest first; `remove_dir` leaves any folder that is not empty.
            for dir in created.iter().rev() {
                let _ = std::fs::remove_dir(dir);
            }
        }
        result
    }

    fn import(
        zip: &mut zip::ZipArchive<std::fs::File>,
        names: &[String],
        home: &Path,
        created: &mut Vec<PathBuf>,
    ) -> Result<Vec<PathBuf>, String> {
        create_dirs(home, created)?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let stage = home.join(format!(".sunrise-spack-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
        // Each target with the original it replaced, so a failure can undo it.
        let mut placed = Vec::new();
        let result = Self::stage_and_commit(zip, names, home, &stage, created, &mut placed);
        let Err(e) = result else {
            let _ = std::fs::remove_dir_all(&stage);
            return Ok(placed.into_iter().map(|(target, _)| target).collect());
        };
        let (failed, kept) = undo(&placed);
        // An original that could not be put back now lives only in its backup in the stage.
        if !kept {
            let _ = std::fs::remove_dir_all(&stage);
        }
        Err(format!("{e}{failed}"))
    }

    fn stage_and_commit(
        zip: &mut zip::ZipArchive<std::fs::File>,
        names: &[String],
        home: &Path,
        stage: &Path,
        created: &mut Vec<PathBuf>,
        placed: &mut Vec<(PathBuf, Option<PathBuf>)>,
    ) -> Result<(), String> {
        let mut total = 0u64;
        for (i, name) in names.iter().enumerate() {
            // A zip's declared sizes are the pack's own claim, so the limits
            // are counted again on the bytes actually inflated.
            Self::stage_one(zip, name, &stage.join(i.to_string()), &mut total)?;
        }
        let backups = stage.join("backup");
        std::fs::create_dir(&backups).map_err(|e| format!("{}: {e}", backups.display()))?;
        for (i, name) in names.iter().enumerate() {
            Self::place_one(
                home,
                name,
                &stage.join(i.to_string()),
                &backups.join(i.to_string()),
                created,
                placed,
            )?;
        }
        Ok(())
    }

    fn stage_one(
        zip: &mut zip::ZipArchive<std::fs::File>,
        name: &str,
        staged: &Path,
        total: &mut u64,
    ) -> Result<(), String> {
        let mut entry = zip
            .by_name(&format!("{OVERRIDES}{name}"))
            .map_err(|e| format!("{name}: {e}"))?;
        let mut out =
            std::fs::File::create(staged).map_err(|e| format!("{}: {e}", staged.display()))?;
        // Reading to the end is what checks the entry's CRC.
        let copied = std::io::copy(&mut (&mut entry).take(MAX_OVERRIDE + 1), &mut out)
            .map_err(|e| format!("{name}: {e}"))?;
        *total += copied;
        if copied > MAX_OVERRIDE {
            return Err(format!("{name} is over {} MiB", MAX_OVERRIDE >> 20));
        }
        if *total > MAX_OVERRIDES {
            return Err(format!("overrides are over {} MiB", MAX_OVERRIDES >> 20));
        }
        Ok(())
    }

    fn place_one(
        home: &Path,
        name: &str,
        staged: &Path,
        backup: &Path,
        created: &mut Vec<PathBuf>,
        placed: &mut Vec<(PathBuf, Option<PathBuf>)>,
    ) -> Result<(), String> {
        let target = home.join(name);
        if let Some(dir) = target.parent() {
            create_dirs(dir, created)?;
        }
        let kept = match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_dir() => {
                return Err(format!("{}: is a folder", target.display()));
            }
            // A link (or any other entry that is not a plain file) could not be put back as it was.
            Ok(meta) if !meta.is_file() => {
                return Err(format!("{}: is a link, not a plain file", target.display()));
            }
            Ok(_) => {
                // Copy, not move: the original stays at `target` until the rename below replaces it
                // in one step, so a crash in between never leaves the file missing.
                std::fs::copy(&target, backup).map_err(|e| format!("{}: {e}", target.display()))?;
                Some(backup.to_path_buf())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{}: {e}", target.display())),
        };
        std::fs::rename(staged, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        placed.push((target, kept));
        Ok(())
    }
}

/// Puts back what `placed` replaced, newest first. Returns a note naming each step that failed, and
/// whether an original was left in its backup.
fn undo(placed: &[(PathBuf, Option<PathBuf>)]) -> (String, bool) {
    let (mut failed, mut kept) = (String::new(), false);
    for (target, backup) in placed.iter().rev() {
        let step = match backup {
            // The original is a copy: renaming it over the target puts it back in one step.
            Some(backup) => std::fs::rename(backup, target),
            None => std::fs::remove_file(target),
        };
        if let Err(e) = step {
            failed += &format!("; could not undo {}: {e}", target.display());
            if let Some(backup) = backup {
                kept = true;
                failed += &format!(" (the original is kept at {})", backup.display());
            }
        }
    }
    (failed, kept)
}

/// Creates `dir` and the missing folders above it, noting each one made.
fn create_dirs(dir: &Path, created: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut at = dir;
    while !at.exists() {
        missing.push(at);
        match at.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => at = parent,
            _ => break,
        }
    }
    for d in missing.into_iter().rev() {
        std::fs::create_dir(d).map_err(|e| format!("{}: {e}", d.display()))?;
        created.push(d.to_path_buf());
    }
    Ok(())
}

/// Refuses an override whose destination, or a folder on the way to it under
/// `home`, is already a link (a symlink, or a junction on Windows): writing
/// through it would leave the game folder. `home` itself may be anything.
fn check_destination(home: &Path, name: &str) -> Result<(), String> {
    let parts: Vec<&str> = name.split('/').collect();
    let mut at = home.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        at.push(part);
        let meta = match std::fs::symlink_metadata(&at) {
            Ok(meta) => meta,
            // Nothing here, so nothing below it exists either.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("{}: {e}", at.display())),
        };
        if meta.file_type().is_symlink() {
            return Err(format!(
                "{}: is a link; an override is not written through links",
                at.display()
            ));
        }
        if i + 1 < parts.len() && !meta.is_dir() {
            return Err(format!("{}: is not a folder", at.display()));
        }
    }
    Ok(())
}

/// An override's path under `overrides/`: a settings file, a resource pack
/// zip, or the stored settings of one of the pack's mods. Plain parts only.
fn check_override(path: &str, manifest: &Manifest) -> Result<(), String> {
    let bad = || {
        format!(
            "overrides/{path}: only {}, resourcepacks/<name>.zip and sunrise-mods/{STORAGE_DIR}/<a listed mod>/ may be overridden",
            SETTINGS.join(", ")
        )
    };
    let parts: Vec<&str> = path.split('/').collect();
    let plain = |p: &&str| {
        !p.is_empty()
            && !p.starts_with('.')
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-+ ".contains(&b))
    };
    let ok = match parts.as_slice() {
        [file] => SETTINGS.contains(file),
        ["resourcepacks", name] => plain(name) && name.ends_with(".zip"),
        ["sunrise-mods", dir, id, rest @ ..] => {
            *dir == STORAGE_DIR
                && manifest.mods.iter().any(|m| m.id == *id)
                && !rest.is_empty()
                && rest.iter().all(plain)
        }
        _ => false,
    };
    if ok { Ok(()) } else { Err(bad()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const PACK: &str = r#"
format = 1
id = "cozy"
name = "Cozy"
version = "1.0.0"
game = "26.3"
[[mods]]
id = "coords_hud"
version = "0.1.0"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
"#;

    fn write_pack(dir: &Path, manifest: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let path = dir.join("cozy.spack");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file(MANIFEST, options).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sunrise-spack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_a_pack_and_writes_its_overrides() {
        let dir = scratch("read");
        let storage = format!("overrides/sunrise-mods/{STORAGE_DIR}/coords_hud/corner.txt");
        let path = write_pack(
            &dir,
            PACK,
            &[
                ("overrides/options.txt", b"renderDistance:8\n"),
                ("overrides/resourcepacks/Cozy Look.zip", b"zip"),
                (&storage, b"top-left"),
            ],
        );
        let pack = Modpack::read(&path).unwrap();
        assert_eq!(pack.manifest.mods[0].id, "coords_hud");
        assert!(pack.manifest.mods[0].on);
        assert_eq!(pack.overrides.len(), 3);
        let home = dir.join("home");
        Modpack::extract_overrides(&path, &home).unwrap();
        let options = std::fs::read_to_string(home.join("options.txt")).unwrap();
        assert_eq!(options, "renderDistance:8\n");
        let corner = home.join(format!("sunrise-mods/{STORAGE_DIR}/coords_hud/corner.txt"));
        assert!(corner.is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_what_a_pack_may_not_hold() {
        let dir = scratch("refuse");
        for name in [
            "mods/evil.jar",
            "overrides/../../.bashrc",
            "overrides/sunrise-mods/evil.smod",
            "overrides/resourcepacks/../options.txt",
            "overrides/servers.dat",
            "overrides/sunrise-mods/.sunrise-mod-data/other_mod/x",
            "overrides/resourcepacks/.hidden.zip",
        ] {
            let path = write_pack(&dir, PACK, &[(name, b"x")]);
            assert!(Modpack::read(&path).is_err(), "{name} was accepted");
        }
        let twice = format!("{PACK}{}", &PACK[PACK.find("[[mods]]").unwrap()..]);
        assert!(Manifest::parse(&twice).unwrap_err().contains("twice"));
        let newer = PACK.replace("format = 1", "format = 2");
        assert!(Manifest::parse(&newer).unwrap_err().contains("format 2"));
        let short = PACK.replace("0000000000000000", "00");
        assert!(Manifest::parse(&short).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failure_while_placing_restores_the_files_already_replaced() {
        let dir = scratch("place");
        let path = write_pack(
            &dir,
            PACK,
            &[
                ("overrides/options.txt", b"from the pack\n"),
                ("overrides/resourcepacks/new.zip", b"new pack"),
            ],
        );
        let home = dir.join("home");
        // A folder where the second file belongs: placing it fails after options.txt was replaced.
        std::fs::create_dir_all(home.join("resourcepacks/new.zip")).unwrap();
        std::fs::write(home.join("options.txt"), b"mine\n").unwrap();
        assert!(Modpack::extract_overrides(&path, &home).is_err());
        assert_eq!(std::fs::read(home.join("options.txt")).unwrap(), b"mine\n");
        assert!(home.join("resourcepacks/new.zip").is_dir());
        let mut left: Vec<_> = std::fs::read_dir(&home)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["options.txt", "resourcepacks"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_original_that_cannot_be_put_back_is_kept_and_reported() {
        let dir = scratch("undo");
        let (target, backup) = (dir.join("options.txt"), dir.join("backup"));
        std::fs::write(&backup, b"mine\n").unwrap();
        // A folder that is not empty where the original goes back: no rename can replace it.
        std::fs::create_dir_all(target.join("in the way")).unwrap();
        let (note, kept) = undo(&[(target.clone(), Some(backup.clone()))]);
        assert!(kept, "the backup was not kept");
        assert!(note.contains(&target.display().to_string()), "{note}");
        assert!(note.contains(&backup.display().to_string()), "{note}");
        assert_eq!(std::fs::read(&backup).unwrap(), b"mine\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_linked_destination_is_refused_and_left_a_link() {
        let dir = scratch("linked");
        let (home, outside) = (dir.join("home"), dir.join("outside.txt"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(&outside, b"shared\n").unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, home.join("options.txt"));
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&outside, home.join("options.txt"));
        if let Err(e) = linked {
            eprintln!("skipped: cannot create a symlink here ({e})");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let path = write_pack(&dir, PACK, &[("overrides/options.txt", b"from the pack\n")]);
        assert!(Modpack::extract_overrides(&path, &home).is_err());
        assert_eq!(
            std::fs::read_link(home.join("options.txt")).unwrap(),
            outside
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"shared\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_import_keeps_existing_files_and_removes_new_ones() {
        let dir = scratch("rollback");
        let path = dir.join("bad.spack");
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file(MANIFEST, options).unwrap();
        zip.write_all(PACK.as_bytes()).unwrap();
        // Name order is the import order: the entry that fails comes last.
        for (name, bytes) in [
            ("overrides/options.txt", &b"renderDistance:8\n"[..]),
            ("overrides/resourcepacks/new.zip", b"new pack"),
            ("overrides/sunrise-extras.txt", b"CORRUPT-ME-PAYLOAD"),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        // The entries are stored, so flipping a payload byte breaks its CRC.
        let mut bytes = std::fs::read(&path).unwrap();
        let at = bytes
            .windows(18)
            .position(|w| w == b"CORRUPT-ME-PAYLOAD")
            .unwrap();
        bytes[at] ^= 0xff;
        std::fs::write(&path, bytes).unwrap();
        assert!(Modpack::read(&path).is_ok());

        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("options.txt"), b"mine\n").unwrap();
        std::fs::write(home.join("sunrise-extras.txt"), b"my extras\n").unwrap();
        assert!(Modpack::extract_overrides(&path, &home).is_err());

        assert_eq!(std::fs::read(home.join("options.txt")).unwrap(), b"mine\n");
        assert_eq!(
            std::fs::read(home.join("sunrise-extras.txt")).unwrap(),
            b"my extras\n"
        );
        let mut left: Vec<_> = std::fs::read_dir(&home)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["options.txt", "sunrise-extras.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Links `home/resourcepacks` to a folder outside `home` with `link(outside, at)`, then checks
    /// that an override for `resourcepacks/pack.zip` is refused and the folder outside is left alone,
    /// and so is the player's `options.txt`, which the pack also overrides and imports first.
    fn refuses_a_link(name: &str, link: impl FnOnce(&Path, &Path) -> std::io::Result<()>) {
        let dir = scratch(name);
        let outside = dir.join("outside");
        let home = dir.join("home");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(outside.join("pack.zip"), b"not the pack's").unwrap();
        std::fs::write(home.join("options.txt"), b"mine\n").unwrap();
        if let Err(e) = link(&outside, &home.join("resourcepacks")) {
            eprintln!("skipped: cannot create a symlink here ({e})");
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let path = write_pack(
            &dir,
            PACK,
            &[
                ("overrides/options.txt", b"from the pack\n"),
                ("overrides/resourcepacks/pack.zip", b"from the pack"),
            ],
        );
        assert!(Modpack::extract_overrides(&path, &home).is_err());
        assert_eq!(std::fs::read(home.join("options.txt")).unwrap(), b"mine\n");
        assert_eq!(
            std::fs::read(outside.join("pack.zip")).unwrap(),
            b"not the pack's"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_override_is_not_written_through_a_linked_folder() {
        refuses_a_link("link", |to, at| {
            #[cfg(unix)]
            return std::os::unix::fs::symlink(to, at);
            #[cfg(windows)]
            return std::os::windows::fs::symlink_dir(to, at);
        });
    }

    /// A junction needs no privilege on Windows, so unlike the symlink above this test never skips.
    #[cfg(windows)]
    #[test]
    fn an_override_is_not_written_through_a_junction() {
        refuses_a_link("junction", |to, at| {
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(at)
                .arg(to)
                .output()
                .unwrap();
            assert!(made.status.success(), "mklink /J failed: {made:?}");
            Ok(())
        });
    }
}
